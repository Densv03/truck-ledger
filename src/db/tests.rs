use crate::*;
use rusqlite::OptionalExtension;
use rusqlite::{Connection, params};
use std::fmt::Write as _;
use std::fs;
fn fixture() -> String {
    include_str!("../../reference/fixtures/hired_drivers_minimal.sii").into()
}
#[test]
fn fixture_extracts_hired_drivers_and_trips() {
    let d = extract(&fixture()).unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(d[0].raw_id, "driver.10");
    assert_eq!(d[0].trips.len(), 2);
    assert_eq!(d[1].raw_id, "driver.11");
    assert!(d[1].trips.is_empty());
    assert_eq!(d[0].trips.iter().map(|t| t.distance).sum::<i64>(), 2100);
    assert_eq!(
        d[0].trips.iter().map(|t| t.net().unwrap()).sum::<i64>(),
        3500
    );
}
#[test]
fn fixture_deduplicates_per_profile() {
    let td = tempfile::tempdir().unwrap();
    let p = td.path().join("db.sqlite3");
    let d = extract(&fixture()).unwrap();
    let mut c = open_database(&p).unwrap();
    assert_eq!(
        ingest(&mut c, "fixture-profile", &d)
            .unwrap()
            .newly_inserted_trips,
        2
    );
    assert_eq!(
        ingest(&mut c, "fixture-profile", &d)
            .unwrap()
            .newly_inserted_trips,
        0
    );
    assert_eq!(
        ingest(&mut c, "another-profile", &d)
            .unwrap()
            .newly_inserted_trips,
        2
    );
}
#[test]
fn fingerprint_ignores_source_references() {
    let d = extract(&fixture()).unwrap();
    let mut x = d[0].trips[0].clone();
    assert_eq!(
        fingerprint("driver.10", &x),
        fingerprint("driver.10", &d[0].trips[0])
    );
    x.fuel += 1;
    assert_ne!(
        fingerprint("driver.10", &x),
        fingerprint("driver.10", &d[0].trips[0])
    );
}
#[test]
fn fingerprint_v1_fixture_golden_bytes() {
    let drivers = extract(&fixture()).unwrap();
    let actual = fingerprint(&drivers[0].raw_id, &drivers[0].trips[0]);
    let mut hex = String::new();
    for byte in actual {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    assert_eq!(
        hex,
        "92b030b3808f6442498e61e796e85e042a0e14b5fd4a3f236ff78f5789d488a9"
    );
}
#[test]
fn valid_zero_hired_is_noop() {
    let td = tempfile::tempdir().unwrap();
    let mut c = open_database(&td.path().join("x.db")).unwrap();
    let d = extract("SiiNunit\n{\ndriver_ai : driver.1 {\nhometown: \"\"\n}\n}\n").unwrap();
    assert!(d.is_empty());
    let r = ingest(&mut c, "zero", &d).unwrap();
    assert_eq!(r.hired_drivers_scanned, 0);
}
#[test]
fn schema_version_safety() {
    let td = tempfile::tempdir().unwrap();
    let p = td.path().join("new.db");
    let c = open_database(&p).unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(c);
    open_database(&p).unwrap();
    let p2 = td.path().join("newer.db");
    let c = Connection::open(&p2).unwrap();
    c.execute_batch("PRAGMA user_version=3;").unwrap();
    drop(c);
    assert!(open_database(&p2).is_err());
    let p3 = td.path().join("old.db");
    let c = Connection::open(&p3).unwrap();
    c.execute_batch("CREATE TABLE unknown(x); ").unwrap();
    drop(c);
    assert!(open_database(&p3).is_err());
}
#[test]
fn fingerprint_excludes_snapshot_fields() {
    let d = extract(&fixture()).unwrap();
    let h = fingerprint(&d[0].raw_id, &d[0].trips[0]);
    let mut changed = d[0].clone();
    changed.hometown = "elsewhere".into();
    changed.adr = 99;
    changed.experience_points = 999;
    assert_eq!(h, fingerprint(&changed.raw_id, &changed.trips[0]));
}
#[test]
fn sql_constraints_enforce_profiles_drivers_and_fingerprints() {
    let td = tempfile::tempdir().unwrap();
    let c = open_database(&td.path().join("db")).unwrap();
    assert_eq!(
        c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    c.execute(
        "INSERT INTO profiles(id,scope_key) VALUES(1,'a'),(2,'b')",
        [],
    )
    .unwrap();
    assert!(c.execute("INSERT INTO drivers(profile_id,raw_driver_id,adr,long_dist,heavy,fragile,urgent,mechanical,hometown,current_city,experience_points) VALUES(99,'x',0,0,0,0,0,0,'h','c',0)",[]).is_err());
    c.execute("INSERT INTO drivers(id,profile_id,raw_driver_id,adr,long_dist,heavy,fragile,urgent,mechanical,hometown,current_city,experience_points) VALUES(10,1,'x',0,0,0,0,0,0,'h','c',0)",[]).unwrap();
    assert!(c.execute("INSERT INTO drivers(profile_id,raw_driver_id,adr,long_dist,heavy,fragile,urgent,mechanical,hometown,current_city,experience_points) VALUES(1,'x',0,0,0,0,0,0,'h','c',0)",[]).is_err());
    let q = "INSERT INTO trips(profile_id,driver_id,fingerprint_version,fingerprint,timestamp_day,revenue,wage,maintenance,fuel,distance,distance_on_job,cargo_count,cargo,source_city,source_company,destination_city,destination_company) VALUES(?1,?2,1,?3,0,0,0,0,0,0,0,0,'','','','','')";
    let h = [1u8; 32];
    assert!(c.execute(q, params![99, 10, &h[..]]).is_err());
    assert!(c.execute(q, params![1, 99, &h[..]]).is_err());
    assert!(c.execute(q, params![2, 10, &h[..]]).is_err());
    assert!(c.execute(q, params![1, 10, &[1u8; 31][..]]).is_err());
    assert!(c.execute(q, params![1, 10, &[1u8; 33][..]]).is_err());
    assert!(c.execute(q, params![1, 10, "x".repeat(32)]).is_err());
    assert_eq!(c.execute(q, params![1, 10, &h[..]]).unwrap(), 1);
    assert!(c.execute(q, params![1, 10, &h[..]]).is_err());
    c.execute("INSERT INTO drivers(id,profile_id,raw_driver_id,adr,long_dist,heavy,fragile,urgent,mechanical,hometown,current_city,experience_points) VALUES(20,2,'x',0,0,0,0,0,0,'h','c',0)",[]).unwrap();
    assert_eq!(c.execute(q, params![2, 20, &h[..]]).unwrap(), 1);
    assert_eq!(
        c.query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
            .optional()
            .unwrap(),
        None
    );
}
#[test]
fn parser_handles_scalars_unknowns_and_errors() {
    let s = "SiiNunit\n{\ndriver_ai : driver.1 {\nadr: -1\nlong_dist: 1\nheavy: 1\nfragile: 1\nurgent: 1\nmechanical: 1\nhometown: \"a\\\\b\\\"c\"\ncurrent_city: city\nexperience_points: 1\nprofit_log: _nameless.log\nunknown: (1,2)\n}\nprofit_log : _nameless.log {\nstats_data[0]: _nameless.trip\n}\nprofit_log_entry : _nameless.trip {\ntimestamp_day: -2\nrevenue: 1\nwage: 0\nmaintenance: 0\nfuel: 0\ndistance: 1\ndistance_on_job: true\ncargo_count: 1\ncargo: thing\nsource_city: a\nsource_company: b\ndestination_city: c\ndestination_company: d\n}\n}";
    let d = extract(s).unwrap();
    assert_eq!(d[0].adr, -1);
    assert_eq!(d[0].hometown, "a\\b\"c");
    assert!(d[0].trips[0].distance_on_job);
    assert_eq!(d[0].trips[0].timestamp_day, -2);
    for bad in [
        "SiiNunit\n{\ndriver_ai : x",
        "SiiNunit\n{\ndriver_ai : x {\nhometown: \"bad\n}\n}",
    ] {
        assert!(extract(bad).is_err());
    }
    let missing = s.replacen("profit_log: _nameless.log\n", "", 1);
    assert!(extract(&missing).is_err());
    let wrong = s.replacen("adr: -1", "adr: no", 1);
    assert!(extract(&wrong).is_err());
    let bad_ref = s.replacen("_nameless.trip\n}", "_nameless.nope\n}", 1);
    assert!(extract(&bad_ref).is_err());
}
#[test]
fn money_paths_and_persisted_fixture_facts() {
    let d = extract(&fixture()).unwrap();
    assert_eq!(d[0].trips[0].net().unwrap(), 5000);
    assert_eq!(d[0].trips[1].net().unwrap(), -1500);
    let mut z = d[0].trips[0].clone();
    z.revenue = 0;
    z.wage = 0;
    z.maintenance = 0;
    z.fuel = 0;
    assert_eq!(z.net().unwrap(), 0);
    z.revenue = i64::MIN;
    z.wage = 1;
    assert!(z.net().is_err());
    z.revenue = i64::MAX;
    z.wage = 0;
    z.maintenance = -1;
    assert!(z.net().is_err());
    let td = tempfile::tempdir().unwrap();
    let mut c = open_database(&td.path().join("db")).unwrap();
    assert_eq!(
        ingest(&mut c, "fixture", &d).unwrap().newly_inserted_trips,
        2
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM drivers", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM trips", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row("SELECT sum(distance) FROM trips", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2100
    );
    let net: i64 = c
        .query_row(
            "SELECT sum(revenue-wage-maintenance-fuel) FROM trips",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(net, 3500);
    assert_eq!(
        ingest(&mut c, "fixture", &d).unwrap().newly_inserted_trips,
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM trips", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}
#[test]
fn paths_and_driver_upserts_are_scoped() {
    let td = tempfile::tempdir().unwrap();
    let nested = td.path().join("a/b/c/db");
    open_database(&nested).unwrap();
    assert!(nested.exists());
    let file = td.path().join("file");
    fs::write(&file, "x").unwrap();
    assert!(open_database(&file.join("db")).is_err());
    let dir = td.path().join("dir");
    fs::create_dir(&dir).unwrap();
    assert!(open_database(&dir).is_err());
    let mut c = open_database(&td.path().join("x.db")).unwrap();
    let mut d = extract(&fixture()).unwrap();
    ingest(&mut c, "a", &d).unwrap();
    ingest(&mut c, "b", &d).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM drivers WHERE raw_driver_id='driver.10'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    d[0].current_city = "updated".into();
    ingest(&mut c, "a", &d).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT current_city FROM drivers WHERE raw_driver_id='driver.10' AND profile_id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "updated"
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM trips WHERE profile_id=1", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        2
    );
}
#[test]
fn persisted_trip_reads_are_day_then_fingerprint_ordered() {
    let td = tempfile::tempdir().unwrap();
    let mut c = open_database(&td.path().join("db")).unwrap();
    let mut d = extract(&fixture()).unwrap();
    let mut a = d[0].trips[0].clone();
    a.timestamp_day = 7;
    a.cargo = "z".into();
    let mut b = d[0].trips[0].clone();
    b.timestamp_day = 7;
    b.cargo = "a".into();
    d[0].trips = vec![a.clone(), b.clone()];
    ingest(&mut c, "p", &d).unwrap();
    let first = read_trips(&c, "p").unwrap();
    let second = read_trips(&c, "p").unwrap();
    assert_eq!(first, second, "repeated reads stable");
    let mut expected = vec![a, b];
    expected.sort_by_key(|t| fingerprint("driver.10", t));
    assert_eq!(first, expected, "equal-day trips use fingerprint bytes");
}
