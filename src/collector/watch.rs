use super::discovery::{absolute_lexical, load_profile_location, real_directory};
use crate::db::configured_profile_scopes;
use crate::ingest::{decode_input, extract};
use crate::{
    db,
    domain::{CollectionSummary, Error, IngestResult, ProfileLocation},
};
use rusqlite::Connection;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};
use walkdir::WalkDir;

pub(crate) const DEBOUNCE: Duration = Duration::from_millis(300);
pub(crate) const RETRY_BACKOFFS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];

pub(crate) fn discover_save_files(save_root: &Path) -> Result<Vec<PathBuf>, Error> {
    real_directory(save_root, "save root")?;
    let mut files = Vec::new();
    for entry in WalkDir::new(save_root).follow_links(false) {
        let entry = entry.map_err(|e| {
            Error::Io(format!(
                "cannot scan save root {}: {e}",
                save_root.display()
            ))
        })?;
        if entry.file_type().is_symlink()
            || !entry.file_type().is_file()
            || entry.file_name() != "game.sii"
        {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| {
            Error::Io(format!(
                "cannot inspect save {}: {e}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        files.push(entry.into_path());
    }
    files.sort();
    Ok(files)
}

fn process_one_save(
    conn: &mut Connection,
    scope: &str,
    source: &Path,
) -> Result<IngestResult, Error> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|e| Error::Io(format!("cannot inspect save {}: {e}", source.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::Input(format!(
            "save is not a real regular file: {}",
            source.display()
        )));
    }
    let snapshot = tempfile::NamedTempFile::new()
        .map_err(|e| Error::Io(format!("cannot create save snapshot: {e}")))?;
    fs::copy(source, snapshot.path())
        .map_err(|e| Error::Io(format!("cannot snapshot save {}: {e}", source.display())))?;
    let raw = fs::read(snapshot.path())
        .map_err(|e| Error::Io(format!("cannot read save snapshot: {e}")))?;
    let decoded = decode_input(&raw)?;
    let drivers = extract(&decoded)?;
    db::ingest(conn, scope, &drivers)
}

pub fn collect_save_tree(
    conn: &mut Connection,
    scope: &str,
    save_root: &Path,
) -> Result<CollectionSummary, Error> {
    let files = discover_save_files(save_root)?;
    let mut summary = CollectionSummary {
        files: files.len(),
        ..Default::default()
    };
    for source in files {
        match retry_source(
            || process_one_save(conn, scope, &source),
            std::thread::sleep,
        )? {
            Ok(result) => summary.add(result),
            Err(error) => {
                summary.failed += 1;
                eprintln!(
                    "save failed: {}: retry exhausted: {error}",
                    source.display()
                );
            }
        }
    }
    Ok(summary)
}

pub(crate) fn retry_source<T, Attempt, Sleep>(
    mut attempt: Attempt,
    mut sleep: Sleep,
) -> Result<Result<T, Error>, Error>
where
    Attempt: FnMut() -> Result<T, Error>,
    Sleep: FnMut(Duration),
{
    let mut last = None;
    for backoff in RETRY_BACKOFFS
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
    {
        match attempt() {
            Ok(value) => return Ok(Ok(value)),
            Err(Error::Database(error)) => return Err(Error::Database(error)),
            Err(error) => {
                last = Some(error);
                if let Some(backoff) = backoff {
                    sleep(backoff);
                }
            }
        }
    }
    Ok(Err(last.expect("retry loop always records an error")))
}

pub fn event_is_relevant(save_root: &Path, paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return true;
    }
    paths.iter().any(|path| match absolute_lexical(path) {
        Ok(path) => path == save_root || path.starts_with(save_root),
        Err(_) => false,
    })
}

pub fn watch(conn: &mut Connection, scope: &str, location: &ProfileLocation) -> Result<(), Error> {
    let save_root = location.profile_root.join("save");
    real_directory(&save_root, "save root")?;
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|e| Error::Io(format!("watcher initialization failed: {e}")))?;
    use notify::Watcher;
    watcher
        .watch(&save_root, notify::RecursiveMode::Recursive)
        .map_err(|e| {
            Error::Io(format!(
                "watcher initialization failed for {}: {e}",
                save_root.display()
            ))
        })?;
    let initial = collect_save_tree(conn, scope, &save_root)?;
    print_summary("catch-up", &initial);
    println!("watching: {}", save_root.display());
    watch_events(conn, scope, &save_root, &receiver)
}

fn watch_events(
    conn: &mut Connection,
    scope: &str,
    save_root: &Path,
    receiver: &Receiver<notify::Result<notify::Event>>,
) -> Result<(), Error> {
    loop {
        let event = receiver
            .recv()
            .map_err(|e| Error::Io(format!("watcher event channel failed: {e}")))?
            .map_err(|e| Error::Io(format!("watcher event failed: {e}")))?;
        if !event_is_relevant(save_root, &event.paths) {
            continue;
        }
        let mut deadline = Instant::now() + DEBOUNCE;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Ok(next)) if event_is_relevant(save_root, &next.paths) => {
                    deadline = Instant::now() + DEBOUNCE
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(Error::Io(format!("watcher event failed: {error}"))),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Io("watcher event channel failed".into()));
                }
            }
        }
        let summary = collect_save_tree(conn, scope, save_root)?;
        print_summary("collection pass", &summary);
    }
}

pub fn print_summary(label: &str, summary: &CollectionSummary) {
    println!(
        "{label}: files={} processed={} failed={} drivers_scanned={} trips_scanned={} inserted={}",
        summary.files,
        summary.processed,
        summary.failed,
        summary.drivers_scanned,
        summary.trips_scanned,
        summary.inserted
    );
}

#[derive(Debug, Clone)]
pub(crate) struct CollectRoot {
    pub(crate) save_root: PathBuf,
    pub(crate) scopes: Vec<String>,
}

pub(crate) fn configured_collect_roots(conn: &Connection) -> Result<Vec<CollectRoot>, Error> {
    let mut roots: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for scope in configured_profile_scopes(conn)? {
        match load_profile_location(conn, &scope) {
            Ok(location) => {
                let save_root = location.profile_root.join("save");
                roots.entry(save_root).or_default().push(scope);
            }
            Err(error) => eprintln!("configured profile invalid: {scope}: {error}"),
        }
    }
    Ok(roots
        .into_iter()
        .map(|(save_root, scopes)| CollectRoot { save_root, scopes })
        .collect())
}

pub(crate) fn collect_root(
    conn: &mut Connection,
    root: &CollectRoot,
) -> Result<Vec<(String, CollectionSummary)>, Error> {
    let mut results = root
        .scopes
        .iter()
        .cloned()
        .map(|scope| {
            (
                scope,
                CollectionSummary {
                    files: 0,
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    let files = discover_save_files(&root.save_root)?;
    for (_, summary) in &mut results {
        summary.files = files.len();
    }
    for source in files {
        let drivers = match retry_source(
            || {
                let metadata = fs::symlink_metadata(&source).map_err(|e| {
                    Error::Io(format!("cannot inspect save {}: {e}", source.display()))
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(Error::Input(format!(
                        "save is not a real regular file: {}",
                        source.display()
                    )));
                }
                let snapshot = tempfile::NamedTempFile::new()
                    .map_err(|e| Error::Io(format!("cannot create save snapshot: {e}")))?;
                fs::copy(&source, snapshot.path()).map_err(|e| {
                    Error::Io(format!("cannot snapshot save {}: {e}", source.display()))
                })?;
                let raw = fs::read(snapshot.path())
                    .map_err(|e| Error::Io(format!("cannot read save snapshot: {e}")))?;
                extract(&decode_input(&raw)?)
            },
            std::thread::sleep,
        )? {
            Ok(drivers) => drivers,
            Err(error) => {
                eprintln!(
                    "save failed: {}: retry exhausted: {error}",
                    source.display()
                );
                for (_, summary) in &mut results {
                    summary.failed += 1;
                }
                continue;
            }
        };
        for (scope, summary) in &mut results {
            summary.add(db::ingest(conn, scope, &drivers)?);
        }
    }
    Ok(results)
}

/// Foreground multi-profile collector. It has no service-manager dependency.
pub fn collect(conn: &mut Connection) -> Result<(), Error> {
    let roots = configured_collect_roots(conn)?;
    if roots.is_empty() {
        return Err(Error::Input(
            "no valid configured profiles to monitor; run setup with --profile-root".into(),
        ));
    }
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|e| Error::Io(format!("watcher initialization failed: {e}")))?;
    use notify::Watcher;
    for root in &roots {
        watcher
            .watch(&root.save_root, notify::RecursiveMode::Recursive)
            .map_err(|e| {
                Error::Io(format!(
                    "watcher initialization failed for {}: {e}",
                    root.save_root.display()
                ))
            })?;
        for (scope, summary) in collect_root(conn, root)? {
            print_summary(&format!("catch-up [{scope}]"), &summary);
        }
        println!("watching: {}", root.save_root.display());
    }
    loop {
        let event = receiver
            .recv()
            .map_err(|_| Error::Io("watcher event channel failed".into()))?
            .map_err(|e| Error::Io(format!("watcher event failed: {e}")))?;
        let mut changed = roots
            .iter()
            .filter(|root| event_is_relevant(&root.save_root, &event.paths))
            .map(|root| root.save_root.clone())
            .collect::<Vec<_>>();
        if changed.is_empty() {
            continue;
        }
        let mut deadline = Instant::now() + DEBOUNCE;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Ok(next)) => {
                    for root in &roots {
                        if event_is_relevant(&root.save_root, &next.paths)
                            && !changed.contains(&root.save_root)
                        {
                            changed.push(root.save_root.clone());
                        }
                    }
                    deadline = Instant::now() + DEBOUNCE;
                }
                Ok(Err(error)) => return Err(Error::Io(format!("watcher event failed: {error}"))),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Io("watcher event channel failed".into()));
                }
            }
        }
        for root in &roots {
            if changed.contains(&root.save_root) {
                for (scope, summary) in collect_root(conn, root)? {
                    print_summary(&format!("collection pass [{scope}]"), &summary);
                }
            }
        }
    }
}
