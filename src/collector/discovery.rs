use crate::domain::{Error, ProfileCandidate, ProfileLocation};
use directories::BaseDirs;
use rusqlite::Connection;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub(crate) const ETS2_APP_ID: &str = "227300";

pub(crate) fn load_profile_location(
    conn: &Connection,
    scope: &str,
) -> Result<ProfileLocation, Error> {
    let row = conn.query_row(
        "SELECT l.profile_root,l.layout_kind FROM profile_locations l JOIN profiles p ON p.id=l.profile_id WHERE p.scope_key=?1",
        [scope],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    );
    let (profile_root, layout_kind) = match row {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(Error::Input(format!(
                "no saved profile location for {scope}; provide --profile-root or run discover"
            )));
        }
        Err(error) => return Err(Error::Database(error.to_string())),
    };
    let mut location = validate_profile_root(Path::new(&profile_root)).map_err(|e| {
        Error::Input(format!(
            "saved profile location for {scope} is invalid: {e}"
        ))
    })?;
    location.layout_kind = layout_kind;
    Ok(location)
}

pub(crate) fn absolute_lexical(path: &Path) -> Result<PathBuf, Error> {
    let input = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| Error::Io(format!("cannot resolve current directory: {e}")))?
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in input.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => out.push(name),
        }
    }
    Ok(out)
}

pub(crate) fn real_directory(path: &Path, context: &str) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path).map_err(|e| {
        Error::Input(format!(
            "{context} missing or inaccessible {}: {e}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Input(format!(
            "{context} must be a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn validate_profile_root(path: &Path) -> Result<ProfileLocation, Error> {
    let profile_root = absolute_lexical(path)?;
    real_directory(&profile_root, "profile root")?;
    real_directory(&profile_root.join("save"), "profile save root")?;
    profile_root.to_str().ok_or_else(|| {
        Error::Input(format!(
            "profile root is not valid UTF-8 and cannot be persisted: {}",
            profile_root.display()
        ))
    })?;
    Ok(ProfileLocation {
        profile_root,
        layout_kind: "custom".into(),
    })
}

fn candidate(profile_root: PathBuf, layout_kind: &str) -> Result<ProfileCandidate, Error> {
    real_directory(&profile_root, "candidate profile root")?;
    let save_root = profile_root.join("save");
    real_directory(&save_root, "candidate save root")?;
    Ok(ProfileCandidate {
        layout_kind: layout_kind.into(),
        profile_root,
        save_root,
    })
}

fn child_dirs(path: &Path, context: &str) -> Result<Vec<PathBuf>, Error> {
    let mut children = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|e| Error::Io(format!("cannot inspect {context} {}: {e}", path.display())))?
    {
        let entry = entry.map_err(|e| Error::Io(format!("cannot inspect {context}: {e}")))?;
        let metadata = entry
            .file_type()
            .map_err(|e| Error::Io(format!("cannot inspect {}: {e}", entry.path().display())))?;
        if metadata.is_dir() && !metadata.is_symlink() {
            children.push(entry.path());
        }
    }
    children.sort();
    Ok(children)
}

fn add_if_profile(out: &mut Vec<ProfileCandidate>, path: PathBuf, kind: &str) -> Result<(), Error> {
    let save = path.join("save");
    match fs::symlink_metadata(&save) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            out.push(candidate(path, kind)?)
        }
        Ok(_) | Err(_) => {}
    }
    Ok(())
}

fn deduplicate_candidates(mut candidates: Vec<ProfileCandidate>) -> Vec<ProfileCandidate> {
    candidates.sort_by(|a, b| a.profile_root.cmp(&b.profile_root));
    candidates.dedup_by(|a, b| a.profile_root == b.profile_root);
    candidates
}

pub(crate) fn discover_custom_roots(roots: &[PathBuf]) -> Result<Vec<ProfileCandidate>, Error> {
    let mut out = Vec::new();
    for root in roots {
        let root = absolute_lexical(root)?;
        real_directory(&root, "discovery root")?;
        add_if_profile(&mut out, root.clone(), "custom")?;
        let profiles = root.join("profiles");
        if profiles.exists() {
            real_directory(&profiles, "profiles directory")?;
            for profile in child_dirs(&profiles, "profiles directory")? {
                add_if_profile(&mut out, profile, "custom")?;
            }
        }
        let userdata = root.join("userdata");
        if userdata.exists() {
            real_directory(&userdata, "Steam userdata directory")?;
            for account in child_dirs(&userdata, "Steam userdata directory")? {
                let numeric = account
                    .file_name()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.bytes().all(|b| b.is_ascii_digit()) && !x.is_empty());
                if !numeric {
                    continue;
                }
                let profiles = account.join(ETS2_APP_ID).join("remote/profiles");
                if !profiles.exists() {
                    continue;
                }
                real_directory(&profiles, "Steam ETS2 profiles directory")?;
                for profile in child_dirs(&profiles, "Steam ETS2 profiles directory")? {
                    add_if_profile(&mut out, profile, "custom")?;
                }
            }
        }
    }
    Ok(deduplicate_candidates(out))
}

pub(crate) fn discover_macos_data_dir(data: &Path) -> Result<Vec<ProfileCandidate>, Error> {
    let mut out = Vec::new();
    let local = data.join("Euro Truck Simulator 2/profiles");
    if local.exists() {
        real_directory(&local, "macOS ETS2 profiles directory")?;
        for profile in child_dirs(&local, "macOS ETS2 profiles directory")? {
            add_if_profile(&mut out, profile, "macos-local")?;
        }
    }
    let userdata = data.join("Steam/userdata");
    if userdata.exists() {
        real_directory(&userdata, "macOS Steam userdata directory")?;
        for account in child_dirs(&userdata, "macOS Steam userdata directory")? {
            let numeric = account
                .file_name()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.bytes().all(|b| b.is_ascii_digit()) && !x.is_empty());
            if !numeric {
                continue;
            }
            let profiles = account.join(ETS2_APP_ID).join("remote/profiles");
            if !profiles.exists() {
                continue;
            }
            real_directory(&profiles, "macOS Steam ETS2 profiles directory")?;
            for profile in child_dirs(&profiles, "macOS Steam ETS2 profiles directory")? {
                add_if_profile(&mut out, profile, "macos-steam-cloud")?;
            }
        }
    }
    Ok(deduplicate_candidates(out))
}

#[cfg(target_os = "macos")]
pub fn discover_default() -> Result<Vec<ProfileCandidate>, Error> {
    let base = BaseDirs::new()
        .ok_or_else(|| Error::Input("could not resolve platform data directory".into()))?;
    discover_macos_data_dir(base.data_dir())
}

#[cfg(not(target_os = "macos"))]
pub fn discover_default() -> Result<Vec<ProfileCandidate>, Error> {
    Err(Error::Input(
        "automatic discovery not yet supported on this platform; provide --root".into(),
    ))
}

pub fn discover(roots: &[PathBuf]) -> Result<Vec<ProfileCandidate>, Error> {
    if roots.is_empty() {
        discover_default()
    } else {
        discover_custom_roots(roots)
    }
}
