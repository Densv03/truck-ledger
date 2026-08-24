use super::{
    RETRY_BACKOFFS, absolute_lexical, collect_root, configured_collect_roots,
    discover_macos_data_dir, retry_source,
};
use crate::*;
use rusqlite::Connection;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
fn fixture() -> String {
    include_str!("../../reference/fixtures/hired_drivers_minimal.sii").into()
}
fn profile(root: &Path) -> PathBuf {
    let profile = root.join("profile");
    fs::create_dir_all(profile.join("save/autosave")).unwrap();
    profile
}

#[test]
fn custom_discovery_is_structural_and_deduplicated() {
    let td = tempfile::tempdir().unwrap();
    let direct = profile(td.path());
    let local = td.path().join("profiles/local");
    fs::create_dir_all(local.join("save")).unwrap();
    let cloud = td
        .path()
        .join("userdata/12345/227300/remote/profiles/cloud/save");
    fs::create_dir_all(&cloud).unwrap();
    fs::create_dir_all(
        td.path()
            .join("userdata/not-number/227300/remote/profiles/nope/save"),
    )
    .unwrap();
    fs::create_dir_all(td.path().join("unrelated/deep/profiles/nope/save")).unwrap();
    let candidates = discover_custom_roots(&[td.path().to_path_buf(), direct]).unwrap();
    assert_eq!(candidates.len(), 3);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.layout_kind == "custom")
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.save_root.ends_with("save"))
    );
}

#[test]
fn macos_layout_probe_combines_local_and_steam_candidates() {
    let td = tempfile::tempdir().unwrap();
    fs::create_dir_all(td.path().join("Euro Truck Simulator 2/profiles/local/save")).unwrap();
    fs::create_dir_all(
        td.path()
            .join("Steam/userdata/42/227300/remote/profiles/cloud/save"),
    )
    .unwrap();
    let candidates = discover_macos_data_dir(td.path()).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].layout_kind, "macos-local");
    assert_eq!(candidates[1].layout_kind, "macos-steam-cloud");
}

#[test]
fn locator_updates_without_changing_historical_trips() {
    let td = tempfile::tempdir().unwrap();
    let first = profile(&td.path().join("one"));
    let second = profile(&td.path().join("two"));
    let mut conn = open_database(&td.path().join("ledger.db")).unwrap();
    let drivers = extract(&fixture()).unwrap();
    ingest(&mut conn, "scope", &drivers).unwrap();
    let before: Vec<u8> = conn
        .query_row("SELECT fingerprint FROM trips LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    associate_profile_location(&mut conn, "scope", &validate_profile_root(&first).unwrap())
        .unwrap();
    associate_profile_location(&mut conn, "scope", &validate_profile_root(&second).unwrap())
        .unwrap();
    assert_eq!(
        load_profile_location(&conn, "scope").unwrap().profile_root,
        second
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM profiles", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM trips", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let after: Vec<u8> = conn
        .query_row("SELECT fingerprint FROM trips LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn persisted_locator_reopens_and_missing_or_invalid_locator_fails() {
    let td = tempfile::tempdir().unwrap();
    let root = profile(td.path());
    let database = td.path().join("ledger.db");
    let mut conn = open_database(&database).unwrap();
    assert!(load_profile_location(&conn, "missing").is_err());
    associate_profile_location(&mut conn, "scope", &validate_profile_root(&root).unwrap()).unwrap();
    drop(conn);
    let conn = open_database(&database).unwrap();
    assert_eq!(
        load_profile_location(&conn, "scope").unwrap().profile_root,
        root
    );
    fs::remove_dir_all(root.join("save")).unwrap();
    assert!(load_profile_location(&conn, "scope").is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_profile_save_and_slots_are_rejected_or_ignored() {
    use std::os::unix::fs::symlink;
    let td = tempfile::tempdir().unwrap();
    let real = profile(td.path());
    let profile_link = td.path().join("profile-link");
    symlink(&real, &profile_link).unwrap();
    assert!(validate_profile_root(&profile_link).is_err());
    let save_link_root = td.path().join("save-link-root");
    fs::create_dir(&save_link_root).unwrap();
    symlink(real.join("save"), save_link_root.join("save")).unwrap();
    assert!(validate_profile_root(&save_link_root).is_err());
    let outside = td.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("game.sii"), fixture()).unwrap();
    symlink(&outside, real.join("save/linked-slot")).unwrap();
    symlink(outside.join("game.sii"), real.join("save/link-game.sii")).unwrap();
    assert!(discover_save_files(&real.join("save")).unwrap().is_empty());
}

#[test]
fn schema_v1_migrates_without_rewriting_phase_one_data() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("v1.db");
    let mut v1 = Connection::open(&path).unwrap();
    v1.execute_batch("PRAGMA foreign_keys=ON; BEGIN; CREATE TABLE profiles(id INTEGER PRIMARY KEY,scope_key TEXT NOT NULL UNIQUE CHECK(length(scope_key)>0)); CREATE TABLE drivers(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL,raw_driver_id TEXT NOT NULL,adr INTEGER NOT NULL,long_dist INTEGER NOT NULL,heavy INTEGER NOT NULL,fragile INTEGER NOT NULL,urgent INTEGER NOT NULL,mechanical INTEGER NOT NULL,hometown TEXT NOT NULL,current_city TEXT NOT NULL,experience_points INTEGER NOT NULL,FOREIGN KEY(profile_id) REFERENCES profiles(id),UNIQUE(profile_id,raw_driver_id),UNIQUE(id,profile_id)); CREATE TABLE trips(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL,driver_id INTEGER NOT NULL,fingerprint_version INTEGER NOT NULL,fingerprint BLOB NOT NULL CHECK(typeof(fingerprint)='blob' AND length(fingerprint)=32),timestamp_day INTEGER NOT NULL,revenue INTEGER NOT NULL,wage INTEGER NOT NULL,maintenance INTEGER NOT NULL,fuel INTEGER NOT NULL,distance INTEGER NOT NULL,distance_on_job INTEGER NOT NULL,cargo_count INTEGER NOT NULL,cargo TEXT NOT NULL,source_city TEXT NOT NULL,source_company TEXT NOT NULL,destination_city TEXT NOT NULL,destination_company TEXT NOT NULL,FOREIGN KEY(driver_id,profile_id) REFERENCES drivers(id,profile_id),UNIQUE(profile_id,fingerprint_version,fingerprint)); PRAGMA user_version=1; COMMIT;").unwrap();
    ingest(&mut v1, "scope", &extract(&fixture()).unwrap()).unwrap();
    let original: Vec<u8> = v1
        .query_row(
            "SELECT fingerprint FROM trips ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(v1);
    let migrated = open_database(&path).unwrap();
    assert_eq!(
        migrated
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        migrated
            .query_row("SELECT count(*) FROM profiles", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        migrated
            .query_row("SELECT count(*) FROM drivers", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        migrated
            .query_row("SELECT count(*) FROM trips", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let after: Vec<u8> = migrated
        .query_row(
            "SELECT fingerprint FROM trips ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original, after);
    assert_eq!(
        migrated
            .query_row("SELECT count(*) FROM profile_locations", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(migrated);
    open_database(&path).unwrap();
}

#[test]
fn selected_save_tree_is_recursive_and_catchup_deduplicates() {
    let td = tempfile::tempdir().unwrap();
    let root = profile(td.path());
    for slot in ["quicksave", "autosave_job_1", "future/nested"] {
        let path = root.join("save").join(slot).join("game.sii");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, fixture()).unwrap();
    }
    fs::write(root.join("save/game.sii.bak"), fixture()).unwrap();
    let files = discover_save_files(&root.join("save")).unwrap();
    assert_eq!(files.len(), 3);
    let mut conn = open_database(&td.path().join("ledger.db")).unwrap();
    let summary = collect_save_tree(&mut conn, "scope", &root.join("save")).unwrap();
    assert_eq!(summary.files, 3);
    assert_eq!(summary.processed, 3);
    assert_eq!(summary.drivers_scanned, 6);
    assert_eq!(summary.trips_scanned, 6);
    assert_eq!(summary.inserted, 2);
}

#[test]
fn multi_profile_collection_groups_shared_save_root_and_preserves_scope_identity() {
    let td = tempfile::tempdir().unwrap();
    let root = profile(td.path());
    let game = root.join("save/autosave/game.sii");
    fs::write(&game, fixture()).unwrap();
    let mut conn = open_database(&td.path().join("ledger.db")).unwrap();
    let location = validate_profile_root(&root).unwrap();
    associate_profile_location(&mut conn, "first", &location).unwrap();
    associate_profile_location(&mut conn, "second", &location).unwrap();
    let roots = configured_collect_roots(&conn).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].scopes, ["first", "second"]);
    let summaries = collect_root(&mut conn, &roots[0]).unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(read_trips(&conn, "first").unwrap().len(), 2);
    assert_eq!(read_trips(&conn, "second").unwrap().len(), 2);
    assert_ne!(
        conn.query_row::<i64, _, _>("SELECT id FROM profiles WHERE scope_key='first'", [], |r| r
            .get(0))
            .unwrap(),
        conn.query_row::<i64, _, _>(
            "SELECT id FROM profiles WHERE scope_key='second'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
    );
}

#[test]
fn configured_collection_isolates_invalid_locator_from_valid_root() {
    let td = tempfile::tempdir().unwrap();
    let valid = profile(&td.path().join("valid"));
    fs::write(valid.join("save/autosave/game.sii"), fixture()).unwrap();
    let invalid = profile(&td.path().join("invalid"));
    let mut conn = open_database(&td.path().join("ledger.db")).unwrap();
    associate_profile_location(&mut conn, "valid", &validate_profile_root(&valid).unwrap())
        .unwrap();
    associate_profile_location(
        &mut conn,
        "invalid",
        &validate_profile_root(&invalid).unwrap(),
    )
    .unwrap();
    fs::remove_dir_all(invalid.join("save")).unwrap();
    let roots = configured_collect_roots(&conn).unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].scopes, ["valid"]);
    collect_root(&mut conn, &roots[0]).unwrap();
    assert_eq!(read_trips(&conn, "valid").unwrap().len(), 2);
}

#[test]
fn retry_policy_is_bounded_and_does_not_retry_database_errors() {
    let mut attempts = 0;
    let mut waits = Vec::new();
    let result: Result<Result<(), Error>, Error> = retry_source(
        || {
            attempts += 1;
            Err(Error::Parse("partial".into()))
        },
        |duration| waits.push(duration),
    );
    assert!(result.unwrap().is_err());
    assert_eq!(attempts, 4);
    assert_eq!(waits, RETRY_BACKOFFS);
    let mut database_attempts = 0;
    let database: Result<Result<(), Error>, Error> = retry_source(
        || {
            database_attempts += 1;
            Err(Error::Database("disk full".into()))
        },
        |_| panic!("database errors must not sleep/retry"),
    );
    assert!(matches!(database, Err(Error::Database(_))));
    assert_eq!(database_attempts, 1);
}

#[test]
fn retry_can_succeed_after_transient_failure() {
    let mut attempts = 0;
    let result = retry_source(
        || {
            attempts += 1;
            if attempts < 3 {
                Err(Error::Decode("partial".into()))
            } else {
                Ok(42)
            }
        },
        |_| {},
    )
    .unwrap();
    assert_eq!(result.unwrap(), 42);
    assert_eq!(attempts, 3);
}

#[test]
fn exhausted_decoder_failures_do_not_mutate_db_and_later_save_ingests() {
    let td = tempfile::tempdir().unwrap();
    let mut conn = open_database(&td.path().join("ledger.db")).unwrap();
    let mut attempts = 0;
    let mut waits = Vec::new();

    let result: Result<Result<(), Error>, Error> = retry_source(
        || {
            attempts += 1;
            Err(Error::Decode("decoder panicked: injected".into()))
        },
        |duration| waits.push(duration),
    );

    assert!(matches!(result, Ok(Err(Error::Decode(_)))));
    assert_eq!(attempts, 4);
    assert_eq!(waits, RETRY_BACKOFFS);
    assert!(read_trips(&conn, "scope").unwrap().is_empty());

    let result = ingest(&mut conn, "scope", &extract(&fixture()).unwrap()).unwrap();
    assert_eq!(result.newly_inserted_trips, 2);
    assert_eq!(read_trips(&conn, "scope").unwrap().len(), 2);
}

#[test]
fn event_relevance_is_contained_and_pathless_is_conservative() {
    let td = tempfile::tempdir().unwrap();
    let save = absolute_lexical(&td.path().join("save")).unwrap();
    assert!(event_is_relevant(&save, &[]));
    assert!(event_is_relevant(&save, std::slice::from_ref(&save)));
    assert!(event_is_relevant(&save, &[save.join("autosave/game.sii")]));
    assert!(!event_is_relevant(
        &save,
        &[td.path().join("outside/game.sii")]
    ));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "native FSEvents smoke test; run explicitly"]
fn macos_notify_observes_save_tree_activity() {
    use notify::Watcher;
    let td = tempfile::tempdir().unwrap();
    let save = td.path().join("save");
    fs::create_dir_all(&save).unwrap();
    let (sender, receiver) = mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |event| sender.send(event).unwrap()).unwrap();
    watcher
        .watch(&save, notify::RecursiveMode::Recursive)
        .unwrap();
    // FSEvents registration is asynchronous; allow backend registration before write.
    std::thread::sleep(Duration::from_secs(1));
    let path = save.join("autosave/game.sii");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, fixture()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let timeout = deadline.saturating_duration_since(Instant::now());
        let event = receiver
            .recv_timeout(timeout)
            .expect("native watcher timed out")
            .unwrap();
        if event_is_relevant(&save, &event.paths) {
            break;
        }
    }
}
