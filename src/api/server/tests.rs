use super::*;
use crate::{Driver, Trip, extract, ingest, open_database};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

fn fixture() -> String {
    include_str!("../../../reference/fixtures/hired_drivers_minimal.sii").into()
}

fn driver(raw_id: &str, trips: Vec<Trip>) -> Driver {
    Driver {
        raw_id: raw_id.into(),
        adr: 0,
        long_dist: 0,
        heavy: 0,
        fragile: 0,
        urgent: 0,
        mechanical: 0,
        hometown: "home".into(),
        current_city: "city".into(),
        experience_points: 0,
        trips,
    }
}

fn trip(
    timestamp_day: i64,
    revenue: i64,
    wage: i64,
    maintenance: i64,
    fuel: i64,
    distance: i64,
    distance_on_job: bool,
) -> Trip {
    Trip {
        timestamp_day,
        revenue,
        wage,
        maintenance,
        fuel,
        distance,
        distance_on_job,
        cargo_count: 0,
        cargo: String::new(),
        source_city: String::new(),
        source_company: String::new(),
        destination_city: String::new(),
        destination_company: String::new(),
    }
}

fn request_raw(port: u16, method: &str, path: &str) -> (u16, Vec<(String, String)>, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    (status, headers, body.to_owned())
}

fn request(port: u16, method: &str, path: &str) -> (u16, Vec<(String, String)>, Value) {
    let (status, headers, body) = request_raw(port, method, path);
    (status, headers, serde_json::from_str(&body).unwrap())
}

#[test]
fn http_api_driver_stats_reconcile_summary_and_preserve_read_only_state() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("ledger.sqlite3");
    let mut db = open_database(&path).unwrap();
    ingest(
        &mut db,
        "main",
        &[
            driver(
                "driver.10",
                vec![
                    trip(1, 30_000, 20_000, 3_000, 2_000, 1_200, true),
                    trip(2, 0, 0, 0, 1_500, 900, false),
                ],
            ),
            driver("driver.11", vec![]),
            driver(
                "driver.12",
                vec![
                    trip(3, 0, 0, 0, 1_000, 100, true),
                    trip(4, 0, 0, 0, 2_000, 200, false),
                ],
            ),
            driver("driver.13", vec![trip(5, 100, 100, 0, 0, 10, true)]),
        ],
    )
    .unwrap();
    ingest(
        &mut db,
        "other",
        &[driver(
            "driver.10",
            vec![trip(6, 9_007_199_254_740_993, 0, 0, 0, 9, true)],
        )],
    )
    .unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let counts: (i64, i64, i64) = db.query_row("SELECT (SELECT count(*) FROM profiles),(SELECT count(*) FROM drivers),(SELECT count(*) FROM trips)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
    let fingerprints: Vec<Vec<u8>> = {
        let mut statement = db
            .prepare("SELECT fingerprint FROM trips ORDER BY id")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    drop(db);

    let server = start_for_tests(&path, 0).unwrap();
    let port = server.address().port();
    let (status, headers, body) = request(port, "GET", "/api/v1/profiles/main/driver-stats");
    assert_eq!(status, 200);
    assert!(
        headers
            .iter()
            .any(|x| x == &("content-type".into(), "application/json".into()))
    );
    assert!(
        headers
            .iter()
            .any(|x| x == &("cache-control".into(), "no-store".into()))
    );
    let stats = body["drivers"].as_array().unwrap();
    assert_eq!(
        stats
            .iter()
            .map(|driver| driver["raw_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["driver.10", "driver.11", "driver.12", "driver.13"]
    );
    assert_eq!(
        stats[0],
        serde_json::json!({"raw_id":"driver.10","trip_count":2,"loaded_trip_count":1,"empty_trip_count":1,"total_distance":"2100","total_revenue":"30000","total_wage":"20000","total_maintenance":"3000","total_fuel":"3500","total_costs":"26500","total_net":"3500"})
    );
    assert_eq!(
        stats[1],
        serde_json::json!({"raw_id":"driver.11","trip_count":0,"loaded_trip_count":0,"empty_trip_count":0,"total_distance":"0","total_revenue":"0","total_wage":"0","total_maintenance":"0","total_fuel":"0","total_costs":"0","total_net":"0"})
    );
    assert_eq!(stats[2]["total_net"], "-3000");
    assert_eq!(stats[3]["total_net"], "0");
    assert_eq!(stats[0]["total_revenue"], "30000");
    assert_eq!(stats[0]["total_costs"], "26500");
    assert_eq!(stats[0]["total_net"], "3500");
    assert!(
        stats
            .iter()
            .all(|driver| driver["loaded_trip_count"].as_u64().unwrap()
                + driver["empty_trip_count"].as_u64().unwrap()
                == driver["trip_count"].as_u64().unwrap())
    );
    let (_, _, other) = request(port, "GET", "/api/v1/profiles/other/driver-stats");
    assert_eq!(other["drivers"][0]["total_revenue"], "9007199254740993");
    let (status, _, missing) = request(port, "GET", "/api/v1/profiles/missing/driver-stats");
    assert_eq!(status, 404);
    assert_eq!(missing["error"]["code"], "profile_not_found");
    let (_, _, summary) = request(port, "GET", "/api/v1/profiles/main/summary");
    let count_fields = [
        ("trip_count", "trip_count"),
        ("loaded_trip_count", "loaded_trip_count"),
        ("empty_trip_count", "empty_trip_count"),
    ];
    for (driver_field, summary_field) in count_fields {
        let total = stats
            .iter()
            .map(|driver| driver[driver_field].as_u64().unwrap())
            .try_fold(0_u64, |total, value| total.checked_add(value))
            .unwrap();
        assert_eq!(
            total,
            summary[summary_field].as_u64().unwrap(),
            "{summary_field}"
        );
    }
    for (driver_field, summary_field) in [
        ("total_distance", "total_distance"),
        ("total_revenue", "total_revenue"),
        ("total_wage", "total_wage"),
        ("total_maintenance", "total_maintenance"),
        ("total_fuel", "total_fuel"),
        ("total_net", "total_net"),
    ] {
        let total = stats
            .iter()
            .map(|driver| {
                driver[driver_field]
                    .as_str()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap()
            })
            .try_fold(0_i64, |total, value| total.checked_add(value))
            .unwrap();
        assert_eq!(
            total.to_string(),
            summary[summary_field].as_str().unwrap(),
            "{summary_field}"
        );
    }
    server.shutdown();

    let db = open_database(&path).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("PRAGMA user_version", [], |row| row.get(0))
            .unwrap(),
        version
    );
    assert_eq!(db.query_row("SELECT (SELECT count(*) FROM profiles),(SELECT count(*) FROM drivers),(SELECT count(*) FROM trips)", [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))).unwrap(), counts);
    let mut statement = db
        .prepare("SELECT fingerprint FROM trips ORDER BY id")
        .unwrap();
    let after: Vec<Vec<u8>> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(after, fingerprints);
}

#[test]
fn http_api_driver_stats_overflows_are_controlled() {
    for trips in [
        vec![trip(1, i64::MIN, 1, 0, 0, 0, true)],
        vec![
            trip(1, 0, 0, 0, 0, i64::MAX, true),
            trip(2, 0, 0, 0, 0, 1, true),
        ],
        vec![
            trip(1, i64::MAX, 0, 0, 0, 0, true),
            trip(2, 1, 0, 0, 0, 0, true),
        ],
        vec![
            trip(1, i64::MAX, i64::MAX, 0, 0, 0, true),
            trip(2, 0, 1, 0, 0, 0, true),
        ],
        vec![
            trip(1, 0, i64::MIN, 0, 0, 0, true),
            trip(2, 0, -1, 0, 0, 0, true),
        ],
    ] {
        let td = tempfile::tempdir().unwrap();
        let path = td.path().join("ledger.sqlite3");
        let mut db = open_database(&path).unwrap();
        ingest(&mut db, "main", &[driver("driver.overflow", trips)]).unwrap();
        drop(db);
        let server = start_for_tests(&path, 0).unwrap();
        let (status, _, body) = request(
            server.address().port(),
            "GET",
            "/api/v1/profiles/main/driver-stats",
        );
        assert_eq!(status, 500);
        assert_eq!(body["error"]["code"], "internal_error");
        server.shutdown();
    }
}

#[test]
fn http_server_serves_embedded_dashboard_assets() {
    let td = tempfile::tempdir().unwrap();
    let db_path = td.path().join("ledger.sqlite3");
    let db = open_database(&db_path).unwrap();
    drop(db);
    let server = start_for_tests(&db_path, 0).unwrap();
    let port = server.address().port();

    for (path, content_type, marker) in [
        (
            "/",
            "text/html; charset=utf-8",
            "<title>TruckLedger</title>",
        ),
        ("/app.js", "text/javascript; charset=utf-8", "driver-stats"),
        ("/styles.css", "text/css; charset=utf-8", "--bg"),
    ] {
        let (status, headers, body) = request_raw(port, "GET", path);
        assert_eq!(status, 200, "{path}");
        assert!(
            headers
                .iter()
                .any(|x| x == &("content-type".into(), content_type.into()))
        );
        assert!(
            headers
                .iter()
                .any(|x| x == &("cache-control".into(), "no-store".into()))
        );
        assert!(body.contains(marker), "{path}");
        if path == "/app.js" {
            assert!(!body.contains("loadAllTrips"));
            assert!(!body.contains("ARCHIVE_PAGE_SIZE"));
            assert!(!body.contains("limit: String(200)"));
        }
    }
    server.shutdown();
}

#[test]
fn http_server_rejects_unknown_and_traversal_static_paths() {
    let td = tempfile::tempdir().unwrap();
    let db_path = td.path().join("ledger.sqlite3");
    let db = open_database(&db_path).unwrap();
    drop(db);
    let server = start_for_tests(&db_path, 0).unwrap();
    for path in [
        "/missing",
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/web/app.js",
    ] {
        let (status, _, body) = request_raw(server.address().port(), "GET", path);
        assert_eq!(status, 404, "{path}");
        assert!(!body.contains("[package]"), "{path}");
    }
    server.shutdown();
}

#[test]
fn http_api_serves_read_only_fixture_data() {
    let td = tempfile::tempdir().unwrap();
    let db_path = td.path().join("ledger.sqlite3");
    let mut db = open_database(&db_path).unwrap();
    ingest(&mut db, "zeta", &extract(&fixture()).unwrap()).unwrap();
    ingest(&mut db, "alpha", &[]).unwrap();
    db.execute("INSERT INTO profile_locations(profile_id,profile_root,layout_kind) SELECT id,'private-path','test' FROM profiles WHERE scope_key='zeta'", []).unwrap();
    let fingerprint: Vec<u8> = db
        .query_row("SELECT fingerprint FROM trips LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let locator: String = db
        .query_row(
            "SELECT profile_root FROM profile_locations LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let row_counts: (i64, i64, i64) = db
            .query_row(
                "SELECT (SELECT count(*) FROM profiles),(SELECT count(*) FROM drivers),(SELECT count(*) FROM trips)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    drop(db);

    let server = start_for_tests(&db_path, 0).unwrap();
    assert_eq!(server.address().ip().to_string(), "127.0.0.1");
    let port = server.address().port();
    let (status, headers, body) = request(port, "GET", "/api/v1");
    assert_eq!(status, 200);
    assert_eq!(
        body,
        serde_json::json!({"api_version":"v1","schema_version":2})
    );
    assert!(
        headers
            .iter()
            .any(|x| x == &("content-type".into(), "application/json".into()))
    );
    assert!(
        headers
            .iter()
            .any(|x| x == &("cache-control".into(), "no-store".into()))
    );
    assert!(
        !headers
            .iter()
            .any(|(name, _)| name == "access-control-allow-origin")
    );

    let (_, _, profiles) = request(port, "GET", "/api/v1/profiles");
    assert_eq!(profiles["profiles"][0]["scope_key"], "alpha");
    assert_eq!(profiles["profiles"][1]["scope_key"], "zeta");
    assert_eq!(profiles["profiles"][0]["trip_count"], 0);
    assert!(!profiles.to_string().contains("profile_root"));

    let (_, _, drivers) = request(port, "GET", "/api/v1/profiles/zeta/drivers");
    assert_eq!(drivers["drivers"].as_array().unwrap().len(), 2);
    assert_eq!(drivers["drivers"][1]["raw_id"], "driver.11");
    assert!(drivers["drivers"][0]["adr"].is_string());

    let (_, _, trips) = request(port, "GET", "/api/v1/profiles/zeta/trips");
    assert_eq!(trips["limit"], 50);
    assert_eq!(trips["offset"], 0);
    assert_eq!(trips["trips"][0]["timestamp_day"], "102");
    assert!(trips["trips"][0]["net"].is_string());
    let fingerprint_text = trips["trips"][0]["fingerprint"].as_str().unwrap();
    assert_eq!(fingerprint_text.len(), 64);
    assert!(
        fingerprint_text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );

    let (_, _, summary) = request(port, "GET", "/api/v1/profiles/zeta/summary");
    assert_eq!(summary["hired_driver_count"], 2);
    assert_eq!(summary["trip_count"], 2);
    assert_eq!(summary["loaded_trip_count"], 1);
    assert_eq!(summary["empty_trip_count"], 1);
    assert_eq!(summary["total_distance"], "2100");
    assert_eq!(summary["total_net"], "3500");

    let (status, _, error) = request(port, "GET", "/api/v1/profiles/missing/drivers");
    assert_eq!(status, 404);
    assert_eq!(error["error"]["code"], "profile_not_found");
    let (status, headers, error) = request(port, "POST", "/api/v1/profiles");
    assert_eq!(status, 405);
    assert!(headers.iter().any(|x| x == &("allow".into(), "GET".into())));
    assert_eq!(error["error"]["code"], "method_not_allowed");
    let (status, _, error) = request(port, "POST", "/api/v1/nope");
    assert_eq!(status, 404);
    assert_eq!(error["error"]["code"], "not_found");
    for path in [
        "/api/v1/profiles/zeta/trips?limit=0",
        "/api/v1/profiles/zeta/trips?limit=201",
        "/api/v1/profiles/zeta/trips?limit=1&limit=2",
        "/api/v1/profiles/zeta/trips?limt=5",
        "/api/v1/profiles/zeta/trips?offset=4294967296",
    ] {
        let (status, _, error) = request(port, "GET", path);
        assert_eq!(status, 400, "{path}");
        assert_eq!(error["error"]["code"], "invalid_query");
    }
    server.shutdown();

    let db = open_database(&db_path).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("PRAGMA user_version", [], |row| row.get(0))
            .unwrap(),
        version
    );
    assert_eq!(
        db.query_row::<Vec<u8>, _, _>("SELECT fingerprint FROM trips LIMIT 1", [], |row| row
            .get(0))
            .unwrap(),
        fingerprint
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT profile_root FROM profile_locations LIMIT 1",
            [],
            |row| row.get(0)
        )
        .unwrap(),
        locator
    );
    assert_eq!(
            db.query_row(
                "SELECT (SELECT count(*) FROM profiles),(SELECT count(*) FROM drivers),(SELECT count(*) FROM trips)",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )
            .unwrap(),
            row_counts
        );
}

#[test]
fn http_api_preserves_i64_precision_and_paginates() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("db.sqlite3");
    let mut db = open_database(&path).unwrap();
    ingest(&mut db, "main", &extract(&fixture()).unwrap()).unwrap();
    db.execute("UPDATE trips SET timestamp_day=?1, revenue=?2, wage=?3, maintenance=?4, fuel=?5 WHERE id=1", rusqlite::params![9_007_199_254_740_993_i64, -9_007_199_254_740_993_i64, 0_i64, 0_i64, -1_i64]).unwrap();
    drop(db);
    let server = start_for_tests(&path, 0).unwrap();
    let port = server.address().port();
    let (_, _, one) = request(
        port,
        "GET",
        "/api/v1/profiles/main/trips?limit=1&offset=0&driver=driver.10",
    );
    assert_eq!(one["trips"].as_array().unwrap().len(), 1);
    assert_eq!(one["trips"][0]["timestamp_day"], "9007199254740993");
    assert_eq!(one["trips"][0]["revenue"], "-9007199254740993");
    assert_eq!(one["trips"][0]["net"], "-9007199254740992");
    assert!(one["limit"].is_u64() && one["offset"].is_u64());
    let (_, _, empty) = request(
        port,
        "GET",
        "/api/v1/profiles/main/trips?limit=200&offset=4294967295",
    );
    assert!(empty["trips"].as_array().unwrap().is_empty());
    server.shutdown();
}

#[test]
fn http_api_rejects_absent_and_non_v2_databases_before_binding() {
    let td = tempfile::tempdir().unwrap();
    let missing = td.path().join("missing.sqlite3");
    assert!(start_for_tests(&missing, 0).is_err());
    assert!(!missing.exists());
    for version in [0_i64, 1, 3] {
        let path = td.path().join(format!("{version}.sqlite3"));
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(&format!("PRAGMA user_version={version}"))
            .unwrap();
        drop(db);
        assert!(start_for_tests(&path, 0).is_err(), "version {version}");
    }
}

#[test]
fn http_api_rejects_occupied_port_and_allows_collector_style_write() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("db.sqlite3");
    let mut writer = open_database(&path).unwrap();
    ingest(&mut writer, "main", &extract(&fixture()).unwrap()).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let occupied = listener.local_addr().unwrap().port();
    assert!(start_for_tests(&path, occupied).is_err());
    let server = start_for_tests(&path, 0).unwrap();
    let (_, _, before) = request(
        server.address().port(),
        "GET",
        "/api/v1/profiles/main/summary",
    );
    assert_eq!(before["trip_count"], 2);
    let mut drivers = extract(&fixture()).unwrap();
    drivers[0].trips[0].timestamp_day = 999;
    ingest(&mut writer, "main", &drivers).unwrap();
    let (_, _, after) = request(
        server.address().port(),
        "GET",
        "/api/v1/profiles/main/summary",
    );
    assert_eq!(after["trip_count"], 3);
    server.shutdown();
}

#[test]
fn http_api_maps_busy_and_later_schema_change_to_controlled_errors() {
    let td = tempfile::tempdir().unwrap();
    let path = td.path().join("db.sqlite3");
    let mut writer = open_database(&path).unwrap();
    ingest(&mut writer, "main", &extract(&fixture()).unwrap()).unwrap();
    let server = start_for_tests(&path, 0).unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (status, _, body) = request(server.address().port(), "GET", "/api/v1/profiles");
    assert_eq!(status, 503);
    assert_eq!(body["error"]["code"], "database_unavailable");
    writer.execute_batch("COMMIT").unwrap();
    writer.execute_batch("PRAGMA user_version=3").unwrap();
    let (status, _, body) = request(server.address().port(), "GET", "/api/v1/profiles");
    assert_eq!(status, 500);
    assert_eq!(body["error"]["code"], "internal_error");
    server.shutdown();
}

#[test]
fn api_default_port_is_stable() {
    assert_eq!(DEFAULT_API_PORT, 32947);
}
