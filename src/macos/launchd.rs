//! macOS LaunchAgent lifecycle. Internal seams exist only for deterministic tests.
#[cfg(test)]
use super::{
    monitor::{monitor_new_lines, monitor_read, monitor_recent},
    service::{install_service, setup_auto_with, status_with, uninstall_with},
};
#[allow(unused_imports)]
use crate::{
    Error, ProfileLocation, associate_profile_location, configured_profile_scopes,
    default_database_path, discover_default, load_profile_location, open_database,
    validate_profile_root,
};
use directories::BaseDirs;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

pub const SERVICE_LABEL: &str = "truck-ledger.collector";
const PLIST_NAME: &str = "truck-ledger.collector.plist";
const THROTTLE_SECONDS: u32 = 60;

#[derive(Clone, Debug)]
pub(crate) struct Paths {
    pub(crate) data: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) binary: PathBuf,
    pub(crate) plist: PathBuf,
    pub(crate) stdout: PathBuf,
    pub(crate) stderr: PathBuf,
}

pub(crate) fn production_paths() -> Result<Paths, Error> {
    let database = default_database_path()?;
    let data = database
        .parent()
        .ok_or_else(|| Error::Input("database path has no parent".into()))?
        .to_path_buf();
    let home = BaseDirs::new()
        .ok_or_else(|| Error::Input("could not resolve user home directory".into()))?
        .home_dir()
        .to_path_buf();
    Ok(Paths {
        binary: data.join("bin/truck-ledger"),
        stdout: data.join("logs/collector.stdout.log"),
        stderr: data.join("logs/collector.stderr.log"),
        plist: home.join("Library/LaunchAgents").join(PLIST_NAME),
        database,
        data,
    })
}

#[derive(Clone, Debug)]
pub(crate) struct LaunchResult {
    status: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}
pub(crate) trait Launchctl {
    fn run(&self, args: &[String]) -> Result<LaunchResult, Error>;
}
pub(crate) struct RealLaunchctl;
impl Launchctl for RealLaunchctl {
    fn run(&self, args: &[String]) -> Result<LaunchResult, Error> {
        let o = Command::new("launchctl")
            .args(args)
            .output()
            .map_err(|e| Error::Io(format!("launchctl unavailable: {e}")))?;
        Ok(LaunchResult {
            status: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).trim().into(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().into(),
        })
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn getuid() -> u32;
}
#[cfg(target_os = "macos")]
pub(crate) fn real_uid() -> u32 {
    unsafe { getuid() }
}

pub(crate) struct Env<'a> {
    pub(crate) paths: Paths,
    pub(crate) source: PathBuf,
    pub(crate) uid: u32,
    pub(crate) launchctl: &'a dyn Launchctl,
}
impl<'a> Env<'a> {
    pub(crate) fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }
    pub(crate) fn target(&self) -> String {
        format!("{}/{}", self.domain(), SERVICE_LABEL)
    }
}

pub(crate) fn xml(value: &str) -> Result<String, Error> {
    if value
        .chars()
        .any(|c| matches!(c, '\0'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'))
    {
        return Err(Error::Input(
            "LaunchAgent plist text contains XML-invalid control character".into(),
        ));
    }
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}
pub(crate) fn plist(paths: &Paths) -> Result<String, Error> {
    let text = |p: &Path| xml(&p.to_string_lossy());
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>collect</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>ThrottleInterval</key><integer>{}</integer>\n<key>StandardOutPath</key><string>{}</string>\n<key>StandardErrorPath</key><string>{}</string>\n</dict></plist>\n",
        xml(SERVICE_LABEL)?,
        text(&paths.binary)?,
        THROTTLE_SECONDS,
        text(&paths.stdout)?,
        text(&paths.stderr)?
    ))
}
pub(crate) fn stage(
    destination: &Path,
    bytes: &[u8],
    executable: bool,
) -> Result<tempfile::NamedTempFile, Error> {
    let parent = destination
        .parent()
        .ok_or_else(|| Error::Input("installation path has no parent".into()))?;
    fs::create_dir_all(parent)
        .map_err(|e| Error::Io(format!("cannot create {}: {e}", parent.display())))?;
    let mut f = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| Error::Io(format!("cannot stage {}: {e}", destination.display())))?;
    f.write_all(bytes)
        .and_then(|_| f.as_file().sync_all())
        .map_err(|e| Error::Io(format!("cannot stage {}: {e}", destination.display())))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(f.path(), fs::Permissions::from_mode(0o755))
            .map_err(|e| Error::Io(format!("cannot set executable permissions: {e}")))?;
    }
    Ok(f)
}
pub(crate) fn stage_binary(env: &Env<'_>) -> Result<tempfile::NamedTempFile, Error> {
    let meta = fs::metadata(&env.source).map_err(|e| {
        Error::Io(format!(
            "cannot inspect current executable {}: {e}",
            env.source.display()
        ))
    })?;
    if !meta.is_file() {
        return Err(Error::Input(
            "current executable is not a regular file".into(),
        ));
    }
    let bytes = fs::read(&env.source)
        .map_err(|e| Error::Io(format!("cannot read current executable: {e}")))?;
    stage(&env.paths.binary, &bytes, true)
}
pub(crate) fn replace(staged: tempfile::NamedTempFile, final_path: &Path) -> Result<(), Error> {
    fs::rename(staged.path(), final_path)
        .map_err(|e| Error::Io(format!("cannot install {}: {e}", final_path.display())))
}
pub(crate) fn not_loaded(stderr: &str) -> bool {
    stderr.contains("Could not find service") || stderr.contains("No such process")
}
pub(crate) enum Inspection {
    Loaded,
    NotLoaded,
    Unavailable(String),
}
pub(crate) fn inspect(env: &Env<'_>) -> Result<Inspection, Error> {
    let r = env.launchctl.run(&["print".into(), env.target()])?;
    if r.status == 0 {
        Ok(Inspection::Loaded)
    } else if not_loaded(&r.stderr) {
        Ok(Inspection::NotLoaded)
    } else {
        Ok(Inspection::Unavailable(format!(
            "exit {}: {}",
            r.status, r.stderr
        )))
    }
}
pub(crate) fn bootout(env: &Env<'_>, required: bool) -> Result<(), Error> {
    let r = env.launchctl.run(&["bootout".into(), env.target()])?;
    if r.status == 0 || (!required && not_loaded(&r.stderr)) {
        Ok(())
    } else {
        Err(Error::Io(format!(
            "launchctl bootout failed (exit {}): {}",
            r.status, r.stderr
        )))
    }
}
pub(crate) fn stop_before_replace(env: &Env<'_>) -> Result<(), Error> {
    match inspect(env)? {
        Inspection::Loaded => bootout(env, true),
        Inspection::NotLoaded => Ok(()),
        Inspection::Unavailable(_) => bootout(env, false),
    }
}
pub(crate) fn launch_ok(env: &Env<'_>, op: &str, args: Vec<String>) -> Result<(), Error> {
    let r = env.launchctl.run(&args)?;
    if r.status == 0 {
        Ok(())
    } else {
        Err(Error::Io(format!(
            "launchctl {op} failed (exit {}): {}; run truck-ledger status or rerun setup",
            r.status, r.stderr
        )))
    }
}

pub(crate) fn choose(
    conn: &mut rusqlite::Connection,
    scope: &str,
    explicit: Option<PathBuf>,
    discovery: impl FnOnce() -> Result<Vec<crate::ProfileCandidate>, Error>,
) -> Result<ProfileLocation, Error> {
    if let Some(root) = explicit {
        let location = validate_profile_root(&root)?;
        let shared = configured_profile_scopes(conn)?.iter().any(|x| {
            x != scope
                && load_profile_location(conn, x)
                    .ok()
                    .is_some_and(|p| p.profile_root == location.profile_root)
        });
        if shared {
            eprintln!(
                "warning: this ETS2 profile root is already associated with another TruckLedger scope"
            );
        }
        associate_profile_location(conn, scope, &location)?;
        return Ok(location);
    }
    if configured_profile_scopes(conn)?.iter().any(|x| x == scope) {
        return load_profile_location(conn, scope);
    }
    let candidates = discovery()?;
    match candidates.as_slice() {
        []=>Err(Error::Input("no ETS2 profile was discovered; run truck-ledger discover or rerun setup with --profile-root".into())),
        [c]=>{
            let p=ProfileLocation{profile_root:c.profile_root.clone(),layout_kind:c.layout_kind.clone()};
            let duplicate=configured_profile_scopes(conn)?.iter().any(|x|load_profile_location(conn,x).ok().is_some_and(|y|y.profile_root==p.profile_root));
            if duplicate { Err(Error::Input("discovered ETS2 profile is already associated with another scope; rerun setup with --profile-root to confirm sharing".into())) } else { associate_profile_location(conn,scope,&p)?; Ok(p) }
        }
        _=>Err(Error::Input(format!("multiple ETS2 profiles discovered:\n{}\nrerun setup with --profile-root <chosen-profile-root>",candidates.iter().map(|c|c.profile_root.display().to_string()).collect::<Vec<_>>().join("\n")))),
    }
}
#[cfg(test)]
fn setup_with(
    env: &Env<'_>,
    scope: &str,
    explicit: Option<PathBuf>,
    discovery: impl FnOnce() -> Result<Vec<crate::ProfileCandidate>, Error>,
) -> Result<(), Error> {
    let mut db = open_database(&env.paths.database)?;
    choose(&mut db, scope, explicit, discovery)?;
    install_service(env)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};
    struct Fake {
        calls: RefCell<Vec<Vec<String>>>,
        answers: RefCell<VecDeque<LaunchResult>>,
    }
    impl Fake {
        fn new(a: Vec<LaunchResult>) -> Self {
            Self {
                calls: RefCell::new(vec![]),
                answers: RefCell::new(a.into()),
            }
        }
    }
    impl Launchctl for Fake {
        fn run(&self, a: &[String]) -> Result<LaunchResult, Error> {
            self.calls.borrow_mut().push(a.into());
            Ok(self
                .answers
                .borrow_mut()
                .pop_front()
                .unwrap_or(LaunchResult {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                }))
        }
    }
    fn ok() -> LaunchResult {
        LaunchResult {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
    fn missing() -> LaunchResult {
        LaunchResult {
            status: 3,
            stdout: String::new(),
            stderr: "Could not find service".into(),
        }
    }
    fn bad() -> LaunchResult {
        LaunchResult {
            status: 1,
            stdout: String::new(),
            stderr: "broken".into(),
        }
    }
    fn env<'a>(td: &'a tempfile::TempDir, f: &'a Fake, source: &str) -> Env<'a> {
        let data = td.path().join("data");
        Env {
            paths: Paths {
                database: data.join("truck-ledger.sqlite3"),
                binary: data.join("bin/truck-ledger"),
                plist: td.path().join("LaunchAgents").join(PLIST_NAME),
                stdout: data.join("logs/out.log"),
                stderr: data.join("logs/err.log"),
                data,
            },
            source: td.path().join(source),
            uid: 12345,
            launchctl: f,
        }
    }
    fn profile(td: &tempfile::TempDir, n: &str) -> PathBuf {
        let p = td.path().join(n);
        fs::create_dir_all(p.join("save")).unwrap();
        p
    }
    fn fixture() -> String {
        include_str!("../../reference/fixtures/hired_drivers_minimal.sii").into()
    }
    #[test]
    fn plist_complete_and_escaped() {
        let td = tempfile::tempdir().unwrap();
        let f = Fake::new(vec![]);
        let e = env(&td, &f, "x & < > \" ' €");
        let mut paths = e.paths.clone();
        paths.binary = td.path().join("x & < > \" ' €");
        paths.stdout = paths.binary.clone();
        paths.stderr = paths.binary.clone();
        let p = plist(&paths).unwrap();
        for x in [
            "&amp;",
            "&lt;",
            "&gt;",
            "&quot;",
            "&apos;",
            "€",
            "<string>collect</string>",
            "<integer>60</integer>",
            "<true/>",
        ] {
            assert!(p.contains(x));
        }
        assert!(!p.contains("cargo run"));
        assert!(!p.contains("bash -c"));
        assert!(xml("\u{1f}").is_err());
    }
    #[test]
    fn fresh_setup_stages_artifacts_and_launches() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"v1").unwrap();
        let f = Fake::new(vec![missing(), ok(), ok()]);
        let e = env(&td, &f, "source");
        let p = profile(&td, "p");
        setup_with(&e, "main", Some(p.clone()), || panic!()).unwrap();
        assert_eq!(fs::read(&e.paths.binary).unwrap(), b"v1");
        let text = fs::read_to_string(&e.paths.plist).unwrap();
        assert!(text.contains(&e.paths.binary.display().to_string()));
        assert!(!text.contains("main"));
        assert!(!text.contains(&p.display().to_string()));
        assert_eq!(
            f.calls.borrow()[1],
            vec!["bootstrap", "gui/12345", &e.paths.plist.to_string_lossy()]
        );
        assert_eq!(
            f.calls.borrow()[2],
            vec!["kickstart", "gui/12345/truck-ledger.collector"]
        );
    }
    #[test]
    fn setup_idempotence_preserves_history_and_refreshes_binary() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"one").unwrap();
        let f = Fake::new(vec![missing(), ok(), ok(), ok(), ok(), ok()]);
        let e = env(&td, &f, "source");
        let p = profile(&td, "p");
        setup_with(&e, "main", Some(p.clone()), || panic!()).unwrap();
        let mut db = open_database(&e.paths.database).unwrap();
        crate::ingest(&mut db, "main", &crate::extract(&fixture()).unwrap()).unwrap();
        let fp: Vec<u8> = db
            .query_row("SELECT fingerprint FROM trips LIMIT 1", [], |r| r.get(0))
            .unwrap();
        drop(db);
        fs::write(td.path().join("source"), b"two").unwrap();
        setup_with(&e, "main", None, || panic!()).unwrap();
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT count(*) FROM trips", [], |r| r.get(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row::<Vec<u8>, _, _>("SELECT fingerprint FROM trips LIMIT 1", [], |r| r
                .get(0))
                .unwrap(),
            fp
        );
        assert_eq!(fs::read(&e.paths.binary).unwrap(), b"two");
    }
    #[test]
    fn bootout_failure_keeps_old_artifacts() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"new").unwrap();
        let f = Fake::new(vec![ok(), bad()]);
        let e = env(&td, &f, "source");
        fs::create_dir_all(e.paths.binary.parent().unwrap()).unwrap();
        fs::write(&e.paths.binary, b"old").unwrap();
        fs::create_dir_all(e.paths.plist.parent().unwrap()).unwrap();
        fs::write(&e.paths.plist, b"oldplist").unwrap();
        assert!(setup_with(&e, "p", Some(profile(&td, "p")), || panic!()).is_err());
        assert_eq!(fs::read(&e.paths.binary).unwrap(), b"old");
        assert_eq!(fs::read_to_string(&e.paths.plist).unwrap(), "oldplist");
    }
    #[test]
    fn start_failure_keeps_new_artifacts_and_recovers() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"new").unwrap();
        let f = Fake::new(vec![missing(), bad()]);
        let e = env(&td, &f, "source");
        assert!(setup_with(&e, "p", Some(profile(&td, "p")), || panic!()).is_err());
        assert_eq!(fs::read(&e.paths.binary).unwrap(), b"new");
        assert!(e.paths.plist.exists());
        let good = Fake::new(vec![missing(), ok(), ok()]);
        let e2 = env(&td, &good, "source");
        setup_with(&e2, "p", None, || panic!()).unwrap();
    }
    #[test]
    fn selection_preserves_existing_and_requires_explicit_shared_discovery() {
        let td = tempfile::tempdir().unwrap();
        let dbpath = td.path().join("db");
        let mut db = open_database(&dbpath).unwrap();
        let a = profile(&td, "a");
        let b = profile(&td, "b");
        choose(&mut db, "a", Some(a.clone()), || panic!()).unwrap();
        let preserved = choose(&mut db, "a", None, || {
            panic!("must not discover existing locator")
        })
        .unwrap();
        assert_eq!(preserved.profile_root, a);
        let candidate = crate::ProfileCandidate {
            profile_root: a.clone(),
            save_root: a.join("save"),
            layout_kind: "macos-local".into(),
        };
        assert!(choose(&mut db, "b", None, || Ok(vec![candidate])).is_err());
        let shared = choose(&mut db, "b", Some(a.clone()), || panic!()).unwrap();
        assert_eq!(shared.profile_root, a);
        fs::remove_dir_all(a.join("save")).unwrap();
        assert!(choose(&mut db, "a", None, || Ok(vec![])).is_err());
        assert!(choose(&mut db, "new", None, || Ok(vec![])).is_err());
        let c = crate::ProfileCandidate {
            profile_root: b.clone(),
            save_root: b.join("save"),
            layout_kind: "macos-local".into(),
        };
        assert_eq!(
            choose(&mut db, "new", None, || Ok(vec![c]))
                .unwrap()
                .layout_kind,
            "macos-local"
        );
        let x = profile(&td, "x");
        let y = profile(&td, "y");
        let candidates = vec![
            crate::ProfileCandidate {
                profile_root: x.clone(),
                save_root: x.join("save"),
                layout_kind: "macos-local".into(),
            },
            crate::ProfileCandidate {
                profile_root: y.clone(),
                save_root: y.join("save"),
                layout_kind: "macos-local".into(),
            },
        ];
        assert!(choose(&mut db, "many", None, || Ok(candidates)).is_err());
    }
    #[test]
    fn status_is_read_only_and_distinguishes_states() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![missing()]);
        let e = env(&td, &f, "source");
        let text = status_with(&e).unwrap();
        assert!(text.contains("service: not installed"));
        assert!(text.contains("launchd: not loaded"));
        assert!(!e.paths.database.exists());
        let f = Fake::new(vec![
            LaunchResult {
                status: 0,
                stdout: "unrelated running text\nstate = waiting".into(),
                stderr: String::new(),
            },
            LaunchResult {
                status: 0,
                stdout: "unrelated running text\nstate = waiting".into(),
                stderr: String::new(),
            },
        ]);
        let e = env(&td, &f, "source");
        fs::create_dir_all(e.paths.plist.parent().unwrap()).unwrap();
        fs::write(&e.paths.plist, b"p").unwrap();
        fs::create_dir_all(e.paths.binary.parent().unwrap()).unwrap();
        fs::write(&e.paths.binary, b"b").unwrap();
        let text = status_with(&e).unwrap();
        assert!(text.contains("launchd: loaded\nstate: unavailable"));
    }
    #[test]
    fn uninstall_is_idempotent_and_keeps_data() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![missing(), ok(), ok(), missing(), missing()]);
        let e = env(&td, &f, "source");
        let p = profile(&td, "p");
        setup_with(&e, "p", Some(p.clone()), || panic!()).unwrap();
        let mut db = open_database(&e.paths.database).unwrap();
        crate::ingest(&mut db, "p", &crate::extract(&fixture()).unwrap()).unwrap();
        let fp: Vec<u8> = db
            .query_row("SELECT fingerprint FROM trips LIMIT 1", [], |r| r.get(0))
            .unwrap();
        drop(db);
        uninstall_with(&e).unwrap();
        uninstall_with(&e).unwrap();
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(
            db.query_row::<Vec<u8>, _, _>("SELECT fingerprint FROM trips LIMIT 1", [], |r| r
                .get(0))
                .unwrap(),
            fp
        );
        assert!(p.exists());
    }
    #[test]
    fn multi_profile_setup_keeps_one_service_and_marks_shared_status() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![
            missing(),
            ok(),
            ok(),
            ok(),
            ok(),
            ok(),
            ok(),
            missing(),
        ]);
        let e = env(&td, &f, "source");
        let a = profile(&td, "a");
        let b = profile(&td, "b");
        setup_with(&e, "a", Some(a.clone()), || panic!()).unwrap();
        setup_with(&e, "b", Some(b.clone()), || panic!()).unwrap();
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(configured_profile_scopes(&db).unwrap(), vec!["a", "b"]);
        drop(db);
        let text = fs::read_to_string(&e.paths.plist).unwrap();
        assert!(text.contains("<string>collect</string>"));
        assert!(!text.contains("<string>a</string>") && !text.contains("<string>b</string>"));
        assert_eq!(
            std::fs::read_dir(e.paths.plist.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        let db = open_database(&e.paths.database).unwrap();
        drop(db);
        let f2 = Fake::new(vec![missing()]);
        let e2 = env(&td, &f2, "source");
        let status = status_with(&e2).unwrap();
        assert!(status.contains("a\tlocator valid") && status.contains("b\tlocator valid"));
    }
    #[test]
    fn collector_rejects_zero_or_all_invalid_profiles_without_watching() {
        let td = tempfile::tempdir().unwrap();
        let mut db = open_database(&td.path().join("db")).unwrap();
        assert!(crate::collect(&mut db).is_err());
        let p = profile(&td, "p");
        associate_profile_location(&mut db, "p", &validate_profile_root(&p).unwrap()).unwrap();
        fs::remove_dir_all(p.join("save")).unwrap();
        assert!(crate::collect(&mut db).is_err());
    }
    #[test]
    fn staging_failure_does_not_replace_existing_artifact() {
        let td = tempfile::tempdir().unwrap();
        let final_path = td.path().join("final");
        fs::write(&final_path, b"old").unwrap();
        let blocker = td.path().join("blocker");
        fs::write(&blocker, b"x").unwrap();
        assert!(stage(&blocker.join("child"), b"new", false).is_err());
        assert_eq!(fs::read(final_path).unwrap(), b"old");
    }
    #[test]
    fn bare_setup_refreshes_existing_main_without_creating_default() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![missing(), ok(), ok()]);
        let e = env(&td, &f, "source");
        let main = profile(&td, "main");
        let mut db = open_database(&e.paths.database).unwrap();
        choose(&mut db, "main", Some(main), || panic!()).unwrap();
        drop(db);
        setup_auto_with(&e, None, None, || panic!("bare refresh must not discover")).unwrap();
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(configured_profile_scopes(&db).unwrap(), vec!["main"]);
    }
    #[test]
    fn bare_setup_refreshes_multiple_or_partially_invalid_profiles() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![missing(), ok(), ok()]);
        let e = env(&td, &f, "source");
        let a = profile(&td, "a");
        let b = profile(&td, "b");
        let mut db = open_database(&e.paths.database).unwrap();
        choose(&mut db, "main", Some(a), || panic!()).unwrap();
        choose(&mut db, "second", Some(b.clone()), || panic!()).unwrap();
        drop(db);
        fs::remove_dir_all(b.join("save")).unwrap();
        setup_auto_with(&e, None, None, || panic!()).unwrap();
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(
            configured_profile_scopes(&db).unwrap(),
            vec!["main", "second"]
        );
        assert!(
            !configured_profile_scopes(&db)
                .unwrap()
                .iter()
                .any(|x| x == "default")
        );
    }
    #[test]
    fn bare_setup_rejects_all_invalid_but_first_setup_discovers_default() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![]);
        let e = env(&td, &f, "source");
        let broken = profile(&td, "broken");
        let mut db = open_database(&e.paths.database).unwrap();
        choose(&mut db, "main", Some(broken.clone()), || panic!()).unwrap();
        drop(db);
        fs::remove_dir_all(broken.join("save")).unwrap();
        assert!(setup_auto_with(&e, None, None, || panic!()).is_err());
        assert!(!e.paths.binary.exists());
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![missing(), ok(), ok()]);
        let e = env(&td, &f, "source");
        let root = profile(&td, "one");
        let candidate = crate::ProfileCandidate {
            profile_root: root.clone(),
            save_root: root.join("save"),
            layout_kind: "macos-local".into(),
        };
        setup_auto_with(&e, None, None, || Ok(vec![candidate])).unwrap();
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(configured_profile_scopes(&db).unwrap(), vec!["default"]);
    }
    #[test]
    fn monitor_recent_lines_are_bounded_and_labeled() {
        let td = tempfile::tempdir().unwrap();
        let f = Fake::new(vec![]);
        let e = env(&td, &f, "source");
        fs::create_dir_all(e.paths.stdout.parent().unwrap()).unwrap();
        fs::write(&e.paths.stdout, "one\ntwo\nthree\n").unwrap();
        fs::write(&e.paths.stderr, "bad\n").unwrap();
        let output = monitor_recent(&e.paths, 2).unwrap();
        assert_eq!(
            output,
            vec!["[collector] two", "[collector] three", "[error] bad"]
        );
    }
    #[test]
    fn monitor_reads_appends_and_recovers_from_truncation() {
        let td = tempfile::tempdir().unwrap();
        let f = Fake::new(vec![]);
        let e = env(&td, &f, "source");
        fs::create_dir_all(e.paths.stdout.parent().unwrap()).unwrap();
        let mut offset = 0;
        assert!(
            monitor_read(&e.paths.stdout, "collector", &mut offset)
                .unwrap()
                .is_empty()
        );
        fs::write(&e.paths.stdout, "first\n").unwrap();
        assert_eq!(
            monitor_read(&e.paths.stdout, "collector", &mut offset).unwrap(),
            vec!["[collector] first"]
        );
        fs::write(&e.paths.stdout, "new\n").unwrap();
        assert_eq!(
            monitor_read(&e.paths.stdout, "collector", &mut offset).unwrap(),
            vec!["[collector] new"]
        );
        fs::write(&e.paths.stderr, "oops\n").unwrap();
        let mut err = 0;
        assert_eq!(
            monitor_read(&e.paths.stderr, "error", &mut err).unwrap(),
            vec!["[error] oops"]
        );
    }
    #[test]
    fn monitor_periodically_emits_append_without_notify_event() {
        let td = tempfile::tempdir().unwrap();
        let f = Fake::new(vec![]);
        let e = env(&td, &f, "source");
        fs::create_dir_all(e.paths.stdout.parent().unwrap()).unwrap();
        fs::write(&e.paths.stdout, "old\n").unwrap();
        let mut stdout = fs::metadata(&e.paths.stdout).unwrap().len();
        let mut stderr = 0;
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&e.paths.stdout)
            .unwrap();
        handle.write_all(b"new\n").unwrap();
        handle.sync_all().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let mut seen = vec![];
        while std::time::Instant::now() < deadline {
            seen = monitor_new_lines(&e.paths, &mut stdout, &mut stderr).unwrap();
            if !seen.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(seen, vec!["[collector] new"]);
        let mut handle = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&e.paths.stderr)
            .unwrap();
        handle.write_all(b"failure\n").unwrap();
        handle.sync_all().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let mut seen = vec![];
        while std::time::Instant::now() < deadline {
            seen = monitor_new_lines(&e.paths, &mut stdout, &mut stderr).unwrap();
            if !seen.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(seen, vec!["[error] failure"]);
    }
    #[test]
    fn launchctl_pre_replace_matrix_is_conservative() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let loaded = Fake::new(vec![ok(), ok()]);
        let e = env(&td, &loaded, "source");
        assert!(stop_before_replace(&e).is_ok());
        assert_eq!(loaded.calls.borrow().len(), 2);
        let failed = Fake::new(vec![ok(), bad()]);
        let e = env(&td, &failed, "source");
        assert!(stop_before_replace(&e).is_err());
        let absent = Fake::new(vec![missing()]);
        let e = env(&td, &absent, "source");
        assert!(stop_before_replace(&e).is_ok());
        assert_eq!(absent.calls.borrow().len(), 1);
        let unknown = Fake::new(vec![bad(), ok()]);
        let e = env(&td, &unknown, "source");
        assert!(stop_before_replace(&e).is_ok());
        let unknown_fail = Fake::new(vec![bad(), bad()]);
        let e = env(&td, &unknown_fail, "source");
        assert!(stop_before_replace(&e).is_err());
    }
    #[test]
    fn status_marks_explicitly_shared_locator() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![missing()]);
        let e = env(&td, &f, "source");
        let root = profile(&td, "shared");
        let mut db = open_database(&e.paths.database).unwrap();
        choose(&mut db, "a", Some(root.clone()), || panic!()).unwrap();
        choose(&mut db, "b", Some(root), || panic!()).unwrap();
        drop(db);
        let s = status_with(&e).unwrap();
        assert!(s.contains("a\tlocator valid (shared)") && s.contains("b\tlocator valid (shared)"));
    }
    #[test]
    fn status_matrix_and_uninstall_preserve_database() {
        let td = tempfile::tempdir().unwrap();
        fs::write(td.path().join("source"), b"x").unwrap();
        let f = Fake::new(vec![
            missing(),
            ok(),
            ok(),
            LaunchResult {
                status: 0,
                stdout: "state = running".into(),
                stderr: String::new(),
            },
            LaunchResult {
                status: 0,
                stdout: "state = running".into(),
                stderr: String::new(),
            },
            ok(),
        ]);
        let e = env(&td, &f, "source");
        let p = profile(&td, "p");
        setup_with(&e, "p", Some(p.clone()), || panic!()).unwrap();
        let mut db = open_database(&e.paths.database).unwrap();
        crate::ingest(&mut db, "p", &crate::extract(&fixture()).unwrap()).unwrap();
        drop(db);
        let s = status_with(&e).unwrap();
        assert!(s.contains("launchd: loaded\nstate: running"));
        uninstall_with(&e).unwrap();
        assert!(!e.paths.binary.exists() && !e.paths.plist.exists());
        let db = open_database(&e.paths.database).unwrap();
        assert_eq!(crate::read_trips(&db, "p").unwrap().len(), 2);
        assert!(p.exists());
    }
}
