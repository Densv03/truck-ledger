use super::launchd::{Paths, production_paths};
#[cfg(not(target_os = "macos"))]
use super::service::unsupported;
use crate::Error;
use std::{fs, io::Read, path::Path};

pub(crate) fn monitor_recent(paths: &Paths, lines: usize) -> Result<Vec<String>, Error> {
    fn recent(path: &Path, label: &str, lines: usize) -> Result<Vec<String>, Error> {
        if !path.exists() {
            return Ok(vec![]);
        }
        let text = fs::read_to_string(path)
            .map_err(|e| Error::Io(format!("cannot read collector log {}: {e}", path.display())))?;
        Ok(text
            .lines()
            .rev()
            .take(lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| format!("[{label}] {line}"))
            .collect())
    }
    let mut out = recent(&paths.stdout, "collector", lines)?;
    out.extend(recent(&paths.stderr, "error", lines)?);
    Ok(out)
}
pub(crate) fn monitor_read(
    path: &Path,
    label: &str,
    offset: &mut u64,
) -> Result<Vec<String>, Error> {
    if !path.exists() {
        *offset = 0;
        return Ok(vec![]);
    }
    let len = fs::metadata(path)
        .map_err(|e| {
            Error::Io(format!(
                "cannot inspect collector log {}: {e}",
                path.display()
            ))
        })?
        .len();
    if len < *offset {
        *offset = 0
    }
    let mut file = fs::File::open(path)
        .map_err(|e| Error::Io(format!("cannot open collector log {}: {e}", path.display())))?;
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(*offset))
        .map_err(|e| Error::Io(format!("cannot seek collector log: {e}")))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| Error::Io(format!("cannot read collector log: {e}")))?;
    *offset = len;
    Ok(text
        .lines()
        .map(|line| format!("[{label}] {line}"))
        .collect())
}
pub(crate) fn monitor_new_lines(
    paths: &Paths,
    stdout: &mut u64,
    stderr: &mut u64,
) -> Result<Vec<String>, Error> {
    let mut lines = monitor_read(&paths.stdout, "collector", stdout)?;
    lines.extend(monitor_read(&paths.stderr, "error", stderr)?);
    Ok(lines)
}
pub(crate) fn monitor_with(paths: &Paths, lines: usize) -> Result<(), Error> {
    if !paths.stdout.exists() && !paths.stderr.exists() && !paths.plist.exists() {
        return Err(Error::Input(
            "collector logs are absent and no managed service is installed".into(),
        ));
    }
    println!("monitoring collector logs; Ctrl+C to stop");
    for line in monitor_recent(paths, lines)? {
        println!("{line}")
    }
    let mut out_offset = fs::metadata(&paths.stdout).map(|m| m.len()).unwrap_or(0);
    let mut err_offset = fs::metadata(&paths.stderr).map(|m| m.len()).unwrap_or(0);
    let dir = paths
        .stdout
        .parent()
        .ok_or_else(|| Error::Input("collector log path has no parent".into()))?;
    fs::create_dir_all(dir)
        .map_err(|e| Error::Io(format!("cannot inspect collector log directory: {e}")))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|e| Error::Io(format!("monitor watcher initialization failed: {e}")))?;
    use notify::Watcher;
    watcher
        .watch(dir, notify::RecursiveMode::NonRecursive)
        .map_err(|e| Error::Io(format!("monitor watcher initialization failed: {e}")))?;
    loop {
        match receiver.recv_timeout(std::time::Duration::from_millis(300)) {
            Ok(Ok(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Err(error)) => {
                return Err(Error::Io(format!("monitor watcher event failed: {error}")));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Error::Io("monitor watcher channel failed".into()));
            }
        }
        for line in monitor_new_lines(paths, &mut out_offset, &mut err_offset)? {
            println!("{line}")
        }
    }
}
pub fn monitor(lines: usize) -> Result<(), Error> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = lines;
        return Err(unsupported());
    }
    #[cfg(target_os = "macos")]
    {
        let paths = production_paths()?;
        monitor_with(&paths, lines)
    }
}
