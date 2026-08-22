use rusqlite::{Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};
use tiny_http::{Header, Method, Response, Server, StatusCode};

pub const DEFAULT_API_PORT: u16 = 32947;
const SCHEMA_VERSION: i64 = 2;
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

#[derive(Debug)]
enum ApiFailure {
    NotFound,
    MethodNotAllowed,
    ProfileNotFound,
    InvalidQuery,
    InvalidPath,
    DatabaseUnavailable,
    Internal,
}

impl From<crate::Error> for ApiFailure {
    fn from(_: crate::Error) -> Self {
        Self::Internal
    }
}

fn database_failure(error: SqlError) -> ApiFailure {
    match error {
        SqlError::SqliteFailure(error, _)
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            ApiFailure::DatabaseUnavailable
        }
        _ => ApiFailure::Internal,
    }
}

fn open_read_only(path: &Path) -> Result<Connection, ApiFailure> {
    let metadata = std::fs::metadata(path).map_err(|_| ApiFailure::Internal)?;
    if !metadata.is_file() {
        return Err(ApiFailure::Internal);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(database_failure)?;
    connection
        .busy_timeout(Duration::from_millis(100))
        .map_err(database_failure)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(database_failure)?;
    if version != SCHEMA_VERSION {
        return Err(ApiFailure::Internal);
    }
    Ok(connection)
}

fn validate_database(path: &Path) -> Result<(), crate::Error> {
    open_read_only(path).map(|_| ()).map_err(|_| {
        crate::Error::Input(
            "cannot open configured database read-only with schema version 2".into(),
        )
    })
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDto<'a>,
}
#[derive(Serialize)]
struct ErrorDto<'a> {
    code: &'a str,
    message: &'a str,
}
fn error_response(error: ApiFailure) -> Response<std::io::Cursor<Vec<u8>>> {
    let (status, code, message) = match error {
        ApiFailure::NotFound => (404, "not_found", "not found"),
        ApiFailure::MethodNotAllowed => (405, "method_not_allowed", "method not allowed"),
        ApiFailure::ProfileNotFound => (404, "profile_not_found", "profile not found"),
        ApiFailure::InvalidQuery => (400, "invalid_query", "invalid query parameters"),
        ApiFailure::InvalidPath => (400, "invalid_path", "invalid path"),
        ApiFailure::DatabaseUnavailable => (
            503,
            "database_unavailable",
            "database temporarily unavailable",
        ),
        ApiFailure::Internal => (500, "internal_error", "internal server error"),
    };
    json_response(
        status,
        &ErrorBody {
            error: ErrorDto { code, message },
        },
    )
}
fn json_response<T: Serialize>(status: u16, value: &T) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| {
        b"{\"error\":{\"code\":\"internal_error\",\"message\":\"internal server error\"}}".to_vec()
    });
    Response::from_data(bytes)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
}

#[derive(Serialize)]
struct MetadataDto {
    api_version: &'static str,
    schema_version: u8,
}
#[derive(Serialize)]
struct ProfilesDto {
    profiles: Vec<ProfileDto>,
}
#[derive(Serialize)]
struct ProfileDto {
    scope_key: String,
    driver_count: u64,
    trip_count: u64,
}
#[derive(Serialize)]
struct DriversDto {
    drivers: Vec<DriverDto>,
}
#[derive(Serialize)]
struct DriverDto {
    raw_id: String,
    adr: String,
    long_dist: String,
    heavy: String,
    fragile: String,
    urgent: String,
    mechanical: String,
    hometown: String,
    current_city: String,
    experience_points: String,
}
#[derive(Serialize)]
struct TripsDto {
    trips: Vec<TripDto>,
    limit: u32,
    offset: u32,
}
#[derive(Serialize)]
struct TripDto {
    driver_raw_id: String,
    fingerprint_version: u8,
    fingerprint: String,
    timestamp_day: String,
    revenue: String,
    wage: String,
    maintenance: String,
    fuel: String,
    distance: String,
    distance_on_job: bool,
    cargo_count: String,
    cargo: String,
    source_city: String,
    source_company: String,
    destination_city: String,
    destination_company: String,
    net: String,
}
#[derive(Serialize)]
struct SummaryDto {
    hired_driver_count: u64,
    trip_count: u64,
    loaded_trip_count: u64,
    empty_trip_count: u64,
    total_distance: String,
    total_revenue: String,
    total_wage: String,
    total_maintenance: String,
    total_fuel: String,
    total_net: String,
}

fn profile_exists(connection: &Connection, scope: &str) -> Result<(), ApiFailure> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT id FROM profiles WHERE scope_key=?1",
            [scope],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_failure)?;
    exists.map(|_| ()).ok_or(ApiFailure::ProfileNotFound)
}
fn count(value: i64) -> Result<u64, ApiFailure> {
    u64::try_from(value).map_err(|_| ApiFailure::Internal)
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn profiles(connection: &Connection) -> Result<ProfilesDto, ApiFailure> {
    let mut statement = connection.prepare("SELECT p.scope_key, (SELECT count(*) FROM drivers d WHERE d.profile_id=p.id), (SELECT count(*) FROM trips t WHERE t.profile_id=p.id) FROM profiles p ORDER BY p.scope_key ASC").map_err(database_failure)?;
    let rows = statement
        .query_map([], |row| {
            Ok(ProfileDto {
                scope_key: row.get(0)?,
                driver_count: count(row.get(1)?).map_err(|_| SqlError::InvalidQuery)?,
                trip_count: count(row.get(2)?).map_err(|_| SqlError::InvalidQuery)?,
            })
        })
        .map_err(database_failure)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map(|profiles| ProfilesDto { profiles })
        .map_err(database_failure)
}
fn drivers(connection: &Connection, scope: &str) -> Result<DriversDto, ApiFailure> {
    profile_exists(connection, scope)?;
    let mut statement = connection.prepare("SELECT d.raw_driver_id,d.adr,d.long_dist,d.heavy,d.fragile,d.urgent,d.mechanical,d.hometown,d.current_city,d.experience_points FROM drivers d JOIN profiles p ON p.id=d.profile_id WHERE p.scope_key=?1 ORDER BY d.raw_driver_id ASC").map_err(database_failure)?;
    let rows = statement
        .query_map([scope], |r| {
            Ok(DriverDto {
                raw_id: r.get(0)?,
                adr: r.get::<_, i64>(1)?.to_string(),
                long_dist: r.get::<_, i64>(2)?.to_string(),
                heavy: r.get::<_, i64>(3)?.to_string(),
                fragile: r.get::<_, i64>(4)?.to_string(),
                urgent: r.get::<_, i64>(5)?.to_string(),
                mechanical: r.get::<_, i64>(6)?.to_string(),
                hometown: r.get(7)?,
                current_city: r.get(8)?,
                experience_points: r.get::<_, i64>(9)?.to_string(),
            })
        })
        .map_err(database_failure)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map(|drivers| DriversDto { drivers })
        .map_err(database_failure)
}

#[derive(Default)]
struct Pagination {
    limit: u32,
    offset: u32,
    driver: Option<String>,
}
fn percent_decode(input: &str) -> Result<String, ApiFailure> {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(ApiFailure::InvalidQuery);
            }
            let h = |b: u8| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                b'A'..=b'F' => Ok(b - b'A' + 10),
                _ => Err(ApiFailure::InvalidQuery),
            };
            output.push(h(bytes[index + 1])? * 16 + h(bytes[index + 2])?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| ApiFailure::InvalidQuery)
}
fn pagination(query: Option<&str>) -> Result<Pagination, ApiFailure> {
    let mut result = Pagination {
        limit: DEFAULT_LIMIT,
        ..Default::default()
    };
    let Some(query) = query else {
        return Ok(result);
    };
    if query.is_empty() {
        return Err(ApiFailure::InvalidQuery);
    }
    let mut seen = std::collections::BTreeSet::new();
    for item in query.split('&') {
        let (key, value) = item.split_once('=').ok_or(ApiFailure::InvalidQuery)?;
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        if value.is_empty() || !seen.insert(key.clone()) {
            return Err(ApiFailure::InvalidQuery);
        }
        match key.as_str() {
            "limit" => {
                let parsed = value.parse::<u32>().map_err(|_| ApiFailure::InvalidQuery)?;
                if !(1..=MAX_LIMIT).contains(&parsed) {
                    return Err(ApiFailure::InvalidQuery);
                }
                result.limit = parsed;
            }
            "offset" => {
                result.offset = value.parse::<u32>().map_err(|_| ApiFailure::InvalidQuery)?
            }
            "driver" => result.driver = Some(value),
            _ => return Err(ApiFailure::InvalidQuery),
        }
    }
    Ok(result)
}
fn trips(connection: &Connection, scope: &str, page: Pagination) -> Result<TripsDto, ApiFailure> {
    profile_exists(connection, scope)?;
    let mut statement = connection.prepare("SELECT d.raw_driver_id,t.fingerprint_version,t.fingerprint,t.timestamp_day,t.revenue,t.wage,t.maintenance,t.fuel,t.distance,t.distance_on_job,t.cargo_count,t.cargo,t.source_city,t.source_company,t.destination_city,t.destination_company FROM trips t JOIN profiles p ON p.id=t.profile_id JOIN drivers d ON d.id=t.driver_id WHERE p.scope_key=?1 AND (?2 IS NULL OR d.raw_driver_id=?2) ORDER BY t.timestamp_day DESC,t.fingerprint ASC LIMIT ?3 OFFSET ?4").map_err(database_failure)?;
    let rows = statement
        .query_map(
            params![
                scope,
                page.driver,
                i64::from(page.limit),
                i64::from(page.offset)
            ],
            |r| {
                let fp: Vec<u8> = r.get(2)?;
                if fp.len() != 32 {
                    return Err(SqlError::InvalidQuery);
                }
                let revenue: i64 = r.get(4)?;
                let wage: i64 = r.get(5)?;
                let maintenance: i64 = r.get(6)?;
                let fuel: i64 = r.get(7)?;
                let net = revenue
                    .checked_sub(wage)
                    .and_then(|x| x.checked_sub(maintenance))
                    .and_then(|x| x.checked_sub(fuel))
                    .ok_or(SqlError::InvalidQuery)?;
                Ok(TripDto {
                    driver_raw_id: r.get(0)?,
                    fingerprint_version: u8::try_from(r.get::<_, i64>(1)?)
                        .map_err(|_| SqlError::InvalidQuery)?,
                    fingerprint: hex(&fp),
                    timestamp_day: r.get::<_, i64>(3)?.to_string(),
                    revenue: revenue.to_string(),
                    wage: wage.to_string(),
                    maintenance: maintenance.to_string(),
                    fuel: fuel.to_string(),
                    distance: r.get::<_, i64>(8)?.to_string(),
                    distance_on_job: r.get::<_, i64>(9)? != 0,
                    cargo_count: r.get::<_, i64>(10)?.to_string(),
                    cargo: r.get(11)?,
                    source_city: r.get(12)?,
                    source_company: r.get(13)?,
                    destination_city: r.get(14)?,
                    destination_company: r.get(15)?,
                    net: net.to_string(),
                })
            },
        )
        .map_err(database_failure)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map(|trips| TripsDto {
            trips,
            limit: page.limit,
            offset: page.offset,
        })
        .map_err(database_failure)
}
fn summary(connection: &Connection, scope: &str) -> Result<SummaryDto, ApiFailure> {
    profile_exists(connection, scope)?;
    let hired_driver_count = count(connection.query_row("SELECT count(*) FROM drivers d JOIN profiles p ON p.id=d.profile_id WHERE p.scope_key=?1", [scope], |r| r.get(0)).map_err(database_failure)?)?;
    let mut statement=connection.prepare("SELECT t.revenue,t.wage,t.maintenance,t.fuel,t.distance,t.distance_on_job FROM trips t JOIN profiles p ON p.id=t.profile_id WHERE p.scope_key=?1").map_err(database_failure)?;
    let rows = statement
        .query_map([scope], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)? != 0,
            ))
        })
        .map_err(database_failure)?;
    let (mut trip_count, mut loaded, mut empty) = (0u64, 0u64, 0u64);
    let (mut distance, mut revenue, mut wage, mut maintenance, mut fuel, mut net) =
        (0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
    for row in rows {
        let (r, w, m, f, d, on_job) = row.map_err(database_failure)?;
        let trip_net = r
            .checked_sub(w)
            .and_then(|x| x.checked_sub(m))
            .and_then(|x| x.checked_sub(f))
            .ok_or(ApiFailure::Internal)?;
        trip_count = trip_count.checked_add(1).ok_or(ApiFailure::Internal)?;
        if on_job {
            loaded = loaded.checked_add(1).ok_or(ApiFailure::Internal)?;
        } else {
            empty = empty.checked_add(1).ok_or(ApiFailure::Internal)?;
        }
        distance = distance.checked_add(d).ok_or(ApiFailure::Internal)?;
        revenue = revenue.checked_add(r).ok_or(ApiFailure::Internal)?;
        wage = wage.checked_add(w).ok_or(ApiFailure::Internal)?;
        maintenance = maintenance.checked_add(m).ok_or(ApiFailure::Internal)?;
        fuel = fuel.checked_add(f).ok_or(ApiFailure::Internal)?;
        net = net.checked_add(trip_net).ok_or(ApiFailure::Internal)?;
    }
    Ok(SummaryDto {
        hired_driver_count,
        trip_count,
        loaded_trip_count: loaded,
        empty_trip_count: empty,
        total_distance: distance.to_string(),
        total_revenue: revenue.to_string(),
        total_wage: wage.to_string(),
        total_maintenance: maintenance.to_string(),
        total_fuel: fuel.to_string(),
        total_net: net.to_string(),
    })
}

fn split_url(url: &str) -> (&str, Option<&str>) {
    url.split_once('?')
        .map_or((url, None), |(path, query)| (path, Some(query)))
}
fn decode_scope(segment: &str) -> Result<String, ApiFailure> {
    if segment.is_empty() {
        Err(ApiFailure::InvalidPath)
    } else {
        percent_decode(segment).map_err(|_| ApiFailure::InvalidPath)
    }
}
fn known_route(path: &str) -> bool {
    let segments: Vec<_> = path.split('/').collect();
    matches!(
        segments.as_slice(),
        ["", "api", "v1"]
            | ["", "api", "v1", "profiles"]
            | ["", "api", "v1", "profiles", _, "drivers"]
            | ["", "api", "v1", "profiles", _, "trips"]
            | ["", "api", "v1", "profiles", _, "summary"]
    )
}
fn route(database: &Path, method: &Method, url: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let (path, query) = split_url(url);
    if !known_route(path) {
        return error_response(ApiFailure::NotFound);
    }
    if *method != Method::Get {
        return error_response(ApiFailure::MethodNotAllowed)
            .with_header(Header::from_bytes("Allow", "GET").unwrap());
    }
    let result = (|| {
        let connection = open_read_only(database)?;
        let segments: Vec<_> = path.split('/').collect();
        match segments.as_slice() {
            ["", "api", "v1"] if query.is_none() => Ok(json_response(
                200,
                &MetadataDto {
                    api_version: "v1",
                    schema_version: 2,
                },
            )),
            ["", "api", "v1", "profiles"] if query.is_none() => {
                Ok(json_response(200, &profiles(&connection)?))
            }
            ["", "api", "v1", "profiles", scope, "drivers"] if query.is_none() => Ok(
                json_response(200, &drivers(&connection, &decode_scope(scope)?)?),
            ),
            ["", "api", "v1", "profiles", scope, "trips"] => Ok(json_response(
                200,
                &trips(&connection, &decode_scope(scope)?, pagination(query)?)?,
            )),
            ["", "api", "v1", "profiles", scope, "summary"] if query.is_none() => Ok(
                json_response(200, &summary(&connection, &decode_scope(scope)?)?),
            ),
            _ => Err(ApiFailure::InvalidQuery),
        }
    })();
    match result {
        Ok(response) => response,
        Err(error) => error_response(error),
    }
}

pub struct ApiServer {
    address: SocketAddr,
    shutdown: Sender<()>,
    thread: JoinHandle<()>,
}
impl ApiServer {
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.thread.join();
    }
}
pub fn start_for_tests(database: &Path, port: u16) -> Result<ApiServer, crate::Error> {
    validate_database(database)?;
    let server = Server::http(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .map_err(|e| crate::Error::Io(format!("cannot bind local API: {e}")))?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| crate::Error::Io("local API did not bind an IP socket".into()))?;
    let database = PathBuf::from(database);
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let thread = thread::spawn(move || {
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(50)) {
                let response = route(&database, request.method(), request.url());
                let _ = request.respond(response);
            }
        }
    });
    Ok(ApiServer {
        address,
        shutdown: shutdown_tx,
        thread,
    })
}
pub fn serve(database: &Path, port: u16) -> Result<(), crate::Error> {
    let server = start_for_tests(database, port)?;
    println!(
        "TruckLedger local API listening on http://{}",
        server.address()
    );
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extract, ingest, open_database};
    use serde_json::Value;
    use std::{
        io::{Read, Write},
        net::TcpStream,
        time::Duration,
    };

    fn fixture() -> String {
        include_str!("../reference/fixtures/hired_drivers_minimal.sii").into()
    }

    fn request(port: u16, method: &str, path: &str) -> (u16, Vec<(String, String)>, Value) {
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
        (status, headers, serde_json::from_str(body).unwrap())
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
}
