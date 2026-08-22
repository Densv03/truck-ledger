use super::launchd::*;
use crate::{
    Error, configured_profile_scopes, default_database_path, discover_default,
    load_profile_location, open_database,
};
use std::{fs, path::PathBuf};

pub(crate) fn install_service(env: &Env<'_>) -> Result<(), Error> {
    let binary = stage_binary(env)?;
    let plist_stage = stage(&env.paths.plist, plist(&env.paths)?.as_bytes(), false)?;
    fs::create_dir_all(&env.paths.data)
        .map_err(|e| Error::Io(format!("cannot create app data: {e}")))?;
    fs::create_dir_all(env.paths.stdout.parent().unwrap())
        .map_err(|e| Error::Io(format!("cannot create log directory: {e}")))?;
    stop_before_replace(env)?;
    replace(binary, &env.paths.binary)?;
    replace(plist_stage, &env.paths.plist)?;
    launch_ok(
        env,
        "bootstrap",
        vec![
            "bootstrap".into(),
            env.domain(),
            env.paths.plist.to_string_lossy().into_owned(),
        ],
    )?;
    launch_ok(env, "kickstart", vec!["kickstart".into(), env.target()])?;
    Ok(())
}
pub(crate) fn setup_auto_with(
    env: &Env<'_>,
    scope: Option<&str>,
    explicit: Option<PathBuf>,
    discovery: impl FnOnce() -> Result<Vec<crate::ProfileCandidate>, Error>,
) -> Result<(), Error> {
    let mut db = open_database(&env.paths.database)?;
    match scope {
        Some(scope) => {
            choose(&mut db, scope, explicit, discovery)?;
        }
        None => {
            if explicit.is_some() {
                return Err(Error::Input("--profile-root requires --profile".into()));
            }
            let scopes = configured_profile_scopes(&db)?;
            if scopes.is_empty() {
                choose(&mut db, "default", None, discovery)?;
            } else {
                let mut valid = 0;
                for scope in scopes {
                    match load_profile_location(&db, &scope) {
                        Ok(_) => valid += 1,
                        Err(error) => eprintln!("configured profile invalid: {scope}: {error}"),
                    }
                }
                if valid == 0 {
                    return Err(Error::Input("no valid configured profiles; repair configuration with setup --profile <scope> --profile-root <path>".into()));
                }
            }
        }
    }
    install_service(env)
}
pub(crate) fn status_with(env: &Env<'_>) -> Result<String, Error> {
    let mut out = format!(
        "service: {}\ncollector_binary: {}\ndatabase: {}\n",
        if env.paths.plist.exists() {
            "installed"
        } else {
            "not installed"
        },
        if env.paths.binary.exists() {
            env.paths.binary.display().to_string()
        } else {
            "missing".into()
        },
        env.paths.database.display()
    );
    match inspect(env)? {
        Inspection::Loaded => {
            out.push_str("launchd: loaded\n");
            let r = env.launchctl.run(&["print".into(), env.target()])?;
            out.push_str(if r.stdout.lines().any(|l| l.trim() == "state = running") {
                "state: running\n"
            } else {
                "state: unavailable\n"
            });
        }
        Inspection::NotLoaded => out.push_str("launchd: not loaded\n"),
        Inspection::Unavailable(e) => out.push_str(&format!("launchd: unavailable ({e})\n")),
    }
    if !env.paths.database.exists() {
        out.push_str("profiles: unavailable (database missing)\n");
        return Ok(out);
    }
    let db = rusqlite::Connection::open_with_flags(
        &env.paths.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| Error::Database(e.to_string()))?;
    let scopes = configured_profile_scopes(&db)?;
    out.push_str(&format!("profiles: {}\n", scopes.len()));
    let roots = scopes
        .iter()
        .filter_map(|s| {
            load_profile_location(&db, s)
                .ok()
                .map(|x| (s, x.profile_root))
        })
        .collect::<Vec<_>>();
    for s in &scopes {
        let valid = load_profile_location(&db, s).is_ok();
        let shared = roots
            .iter()
            .find(|(x, _)| *x == s)
            .is_some_and(|(_, r)| roots.iter().filter(|(_, q)| q == r).count() > 1);
        out.push_str(&format!(
            "{s}\tlocator {}{}\n",
            if valid { "valid" } else { "invalid" },
            if shared { " (shared)" } else { "" }
        ));
    }
    Ok(out)
}
pub(crate) fn uninstall_with(env: &Env<'_>) -> Result<(), Error> {
    match inspect(env)? {
        Inspection::Loaded => bootout(env, true)?,
        Inspection::Unavailable(_) => bootout(env, false)?,
        Inspection::NotLoaded => {}
    }
    for p in [&env.paths.plist, &env.paths.binary] {
        if p.exists() {
            fs::remove_file(p)
                .map_err(|e| Error::Io(format!("cannot remove {}: {e}", p.display())))?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn unsupported() -> Error {
    Error::Input("macOS background-service lifecycle is unsupported on this platform".into())
}
pub fn setup(scope: Option<&str>, root: Option<PathBuf>) -> Result<(), Error> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (scope, root);
        return Err(unsupported());
    }
    #[cfg(target_os = "macos")]
    {
        let real = RealLaunchctl;
        let env = Env {
            paths: production_paths()?,
            source: std::env::current_exe()
                .map_err(|e| Error::Io(format!("cannot resolve current executable: {e}")))?,
            uid: real_uid(),
            launchctl: &real,
        };
        setup_auto_with(&env, scope, root, discover_default)?;
        println!("setup complete: collector monitors all configured profiles");
        Ok(())
    }
}
pub fn collect() -> Result<(), Error> {
    let mut db = open_database(&default_database_path()?)?;
    crate::collect(&mut db)
}
pub fn status() -> Result<(), Error> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err(unsupported());
    }
    #[cfg(target_os = "macos")]
    {
        let real = RealLaunchctl;
        let env = Env {
            paths: production_paths()?,
            source: PathBuf::new(),
            uid: real_uid(),
            launchctl: &real,
        };
        print!("{}", status_with(&env)?);
        Ok(())
    }
}
pub fn uninstall() -> Result<(), Error> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err(unsupported());
    }
    #[cfg(target_os = "macos")]
    {
        let real = RealLaunchctl;
        let env = Env {
            paths: production_paths()?,
            source: PathBuf::new(),
            uid: real_uid(),
            launchctl: &real,
        };
        uninstall_with(&env)?;
        println!("service artifacts removed; database and history preserved");
        Ok(())
    }
}
