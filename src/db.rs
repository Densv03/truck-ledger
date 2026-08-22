use crate::{
    domain::{Driver, Error, IngestResult, ProfileLocation, Trip},
    ingest::fingerprint,
};
use directories::BaseDirs;
use rusqlite::{Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension, params};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn db_err(e: rusqlite::Error) -> Error {
    Error::Database(e.to_string())
}

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug)]
pub(crate) enum ReadError {
    ProfileNotFound,
    DatabaseUnavailable,
    Internal,
}

fn read_error(error: SqlError) -> ReadError {
    match error {
        SqlError::SqliteFailure(error, _)
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            ReadError::DatabaseUnavailable
        }
        _ => ReadError::Internal,
    }
}

#[derive(Debug)]
pub(crate) struct ProfileRead {
    pub(crate) scope_key: String,
    pub(crate) driver_count: i64,
    pub(crate) trip_count: i64,
}

#[derive(Debug)]
pub(crate) struct DriverRead {
    pub(crate) raw_id: String,
    pub(crate) adr: i64,
    pub(crate) long_dist: i64,
    pub(crate) heavy: i64,
    pub(crate) fragile: i64,
    pub(crate) urgent: i64,
    pub(crate) mechanical: i64,
    pub(crate) hometown: String,
    pub(crate) current_city: String,
    pub(crate) experience_points: i64,
}

#[derive(Debug)]
pub(crate) struct TripRead {
    pub(crate) driver_raw_id: String,
    pub(crate) fingerprint_version: i64,
    pub(crate) fingerprint: [u8; 32],
    pub(crate) timestamp_day: i64,
    pub(crate) revenue: i64,
    pub(crate) wage: i64,
    pub(crate) maintenance: i64,
    pub(crate) fuel: i64,
    pub(crate) distance: i64,
    pub(crate) distance_on_job: bool,
    pub(crate) cargo_count: i64,
    pub(crate) cargo: String,
    pub(crate) source_city: String,
    pub(crate) source_company: String,
    pub(crate) destination_city: String,
    pub(crate) destination_company: String,
    pub(crate) net: i64,
}

#[derive(Debug)]
pub(crate) struct ProfileSummaryRead {
    pub(crate) hired_driver_count: i64,
    pub(crate) trip_count: i64,
    pub(crate) loaded_trip_count: i64,
    pub(crate) empty_trip_count: i64,
    pub(crate) total_distance: i64,
    pub(crate) total_revenue: i64,
    pub(crate) total_wage: i64,
    pub(crate) total_maintenance: i64,
    pub(crate) total_fuel: i64,
    pub(crate) total_net: i64,
}

#[derive(Debug)]
pub(crate) struct DriverStatsRead {
    pub(crate) drivers: Vec<DriverStatRead>,
}

#[derive(Debug)]
pub(crate) struct DriverStatRead {
    pub(crate) raw_id: String,
    pub(crate) trip_count: i64,
    pub(crate) loaded_trip_count: i64,
    pub(crate) empty_trip_count: i64,
    pub(crate) total_distance: i64,
    pub(crate) total_revenue: i64,
    pub(crate) total_wage: i64,
    pub(crate) total_maintenance: i64,
    pub(crate) total_fuel: i64,
    pub(crate) total_net: i64,
}

fn open_read_only(path: &Path) -> Result<Connection, ReadError> {
    let metadata = fs::metadata(path).map_err(|_| ReadError::Internal)?;
    if !metadata.is_file() {
        return Err(ReadError::Internal);
    }
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(read_error)?;
    connection
        .busy_timeout(std::time::Duration::from_millis(100))
        .map_err(read_error)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(read_error)?;
    if version != SCHEMA_VERSION {
        return Err(ReadError::Internal);
    }
    Ok(connection)
}

pub(crate) fn validate_read_database(path: &Path) -> Result<(), ReadError> {
    open_read_only(path).map(|_| ())
}

fn require_profile(connection: &Connection, scope: &str) -> Result<(), ReadError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT id FROM profiles WHERE scope_key=?1",
            [scope],
            |row| row.get(0),
        )
        .optional()
        .map_err(read_error)?;
    exists.map(|_| ()).ok_or(ReadError::ProfileNotFound)
}

fn checked_net(revenue: i64, wage: i64, maintenance: i64, fuel: i64) -> Result<i64, ReadError> {
    revenue
        .checked_sub(wage)
        .and_then(|value| value.checked_sub(maintenance))
        .and_then(|value| value.checked_sub(fuel))
        .ok_or(ReadError::Internal)
}

pub(crate) fn read_profiles(path: &Path) -> Result<Vec<ProfileRead>, ReadError> {
    let connection = open_read_only(path)?;
    let mut statement = connection.prepare("SELECT p.scope_key, (SELECT count(*) FROM drivers d WHERE d.profile_id=p.id), (SELECT count(*) FROM trips t WHERE t.profile_id=p.id) FROM profiles p ORDER BY p.scope_key ASC").map_err(read_error)?;
    statement
        .query_map([], |row| {
            Ok(ProfileRead {
                scope_key: row.get(0)?,
                driver_count: row.get(1)?,
                trip_count: row.get(2)?,
            })
        })
        .map_err(read_error)?
        .collect::<Result<_, _>>()
        .map_err(read_error)
}

pub(crate) fn read_drivers(path: &Path, scope: &str) -> Result<Vec<DriverRead>, ReadError> {
    let connection = open_read_only(path)?;
    require_profile(&connection, scope)?;
    let mut statement = connection.prepare("SELECT d.raw_driver_id,d.adr,d.long_dist,d.heavy,d.fragile,d.urgent,d.mechanical,d.hometown,d.current_city,d.experience_points FROM drivers d JOIN profiles p ON p.id=d.profile_id WHERE p.scope_key=?1 ORDER BY d.raw_driver_id ASC").map_err(read_error)?;
    statement
        .query_map([scope], |r| {
            Ok(DriverRead {
                raw_id: r.get(0)?,
                adr: r.get(1)?,
                long_dist: r.get(2)?,
                heavy: r.get(3)?,
                fragile: r.get(4)?,
                urgent: r.get(5)?,
                mechanical: r.get(6)?,
                hometown: r.get(7)?,
                current_city: r.get(8)?,
                experience_points: r.get(9)?,
            })
        })
        .map_err(read_error)?
        .collect::<Result<_, _>>()
        .map_err(read_error)
}

pub(crate) fn read_trips_page(
    path: &Path,
    scope: &str,
    driver: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<TripRead>, ReadError> {
    let connection = open_read_only(path)?;
    require_profile(&connection, scope)?;
    let mut statement = connection.prepare("SELECT d.raw_driver_id,t.fingerprint_version,t.fingerprint,t.timestamp_day,t.revenue,t.wage,t.maintenance,t.fuel,t.distance,t.distance_on_job,t.cargo_count,t.cargo,t.source_city,t.source_company,t.destination_city,t.destination_company FROM trips t JOIN profiles p ON p.id=t.profile_id JOIN drivers d ON d.id=t.driver_id WHERE p.scope_key=?1 AND (?2 IS NULL OR d.raw_driver_id=?2) ORDER BY t.timestamp_day DESC,t.fingerprint ASC LIMIT ?3 OFFSET ?4").map_err(read_error)?;
    let rows = statement
        .query_map(
            params![scope, driver, i64::from(limit), i64::from(offset)],
            |r| {
                let fingerprint: Vec<u8> = r.get(2)?;
                let fingerprint = fingerprint.try_into().map_err(|_| SqlError::InvalidQuery)?;
                let revenue: i64 = r.get(4)?;
                let wage: i64 = r.get(5)?;
                let maintenance: i64 = r.get(6)?;
                let fuel: i64 = r.get(7)?;
                let net = revenue
                    .checked_sub(wage)
                    .and_then(|value| value.checked_sub(maintenance))
                    .and_then(|value| value.checked_sub(fuel))
                    .ok_or(SqlError::InvalidQuery)?;
                Ok(TripRead {
                    driver_raw_id: r.get(0)?,
                    fingerprint_version: r.get(1)?,
                    fingerprint,
                    timestamp_day: r.get(3)?,
                    revenue,
                    wage,
                    maintenance,
                    fuel,
                    distance: r.get(8)?,
                    distance_on_job: r.get::<_, i64>(9)? != 0,
                    cargo_count: r.get(10)?,
                    cargo: r.get(11)?,
                    source_city: r.get(12)?,
                    source_company: r.get(13)?,
                    destination_city: r.get(14)?,
                    destination_company: r.get(15)?,
                    net,
                })
            },
        )
        .map_err(read_error)?;
    rows.collect::<Result<_, _>>().map_err(read_error)
}

pub(crate) fn read_profile_summary(
    path: &Path,
    scope: &str,
) -> Result<ProfileSummaryRead, ReadError> {
    let connection = open_read_only(path)?;
    require_profile(&connection, scope)?;
    let hired_driver_count = connection.query_row("SELECT count(*) FROM drivers d JOIN profiles p ON p.id=d.profile_id WHERE p.scope_key=?1", [scope], |r| r.get(0)).map_err(read_error)?;
    let mut statement = connection.prepare("SELECT t.revenue,t.wage,t.maintenance,t.fuel,t.distance,t.distance_on_job FROM trips t JOIN profiles p ON p.id=t.profile_id WHERE p.scope_key=?1").map_err(read_error)?;
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
        .map_err(read_error)?;
    let mut summary = ProfileSummaryRead {
        hired_driver_count,
        trip_count: 0,
        loaded_trip_count: 0,
        empty_trip_count: 0,
        total_distance: 0,
        total_revenue: 0,
        total_wage: 0,
        total_maintenance: 0,
        total_fuel: 0,
        total_net: 0,
    };
    for row in rows {
        let (revenue, wage, maintenance, fuel, distance, loaded) = row.map_err(read_error)?;
        summary.trip_count = summary
            .trip_count
            .checked_add(1)
            .ok_or(ReadError::Internal)?;
        if loaded {
            summary.loaded_trip_count = summary
                .loaded_trip_count
                .checked_add(1)
                .ok_or(ReadError::Internal)?;
        } else {
            summary.empty_trip_count = summary
                .empty_trip_count
                .checked_add(1)
                .ok_or(ReadError::Internal)?;
        }
        summary.total_distance = summary
            .total_distance
            .checked_add(distance)
            .ok_or(ReadError::Internal)?;
        summary.total_revenue = summary
            .total_revenue
            .checked_add(revenue)
            .ok_or(ReadError::Internal)?;
        summary.total_wage = summary
            .total_wage
            .checked_add(wage)
            .ok_or(ReadError::Internal)?;
        summary.total_maintenance = summary
            .total_maintenance
            .checked_add(maintenance)
            .ok_or(ReadError::Internal)?;
        summary.total_fuel = summary
            .total_fuel
            .checked_add(fuel)
            .ok_or(ReadError::Internal)?;
        summary.total_net = summary
            .total_net
            .checked_add(checked_net(revenue, wage, maintenance, fuel)?)
            .ok_or(ReadError::Internal)?;
    }
    Ok(summary)
}

pub(crate) fn read_driver_stats(path: &Path, scope: &str) -> Result<DriverStatsRead, ReadError> {
    let connection = open_read_only(path)?;
    require_profile(&connection, scope)?;
    let mut statement = connection.prepare("SELECT d.raw_driver_id,t.revenue,t.wage,t.maintenance,t.fuel,t.distance,t.distance_on_job FROM drivers d JOIN profiles p ON p.id=d.profile_id LEFT JOIN trips t ON t.driver_id=d.id AND t.profile_id=p.id WHERE p.scope_key=?1 ORDER BY d.raw_driver_id ASC,t.id ASC").map_err(read_error)?;
    let mut rows = statement.query([scope]).map_err(read_error)?;
    let mut drivers: Vec<DriverStatRead> = Vec::new();
    while let Some(row) = rows.next().map_err(read_error)? {
        let raw_id: String = row.get(0).map_err(read_error)?;
        if drivers.last().is_none_or(|driver| driver.raw_id != raw_id) {
            drivers.push(DriverStatRead {
                raw_id,
                trip_count: 0,
                loaded_trip_count: 0,
                empty_trip_count: 0,
                total_distance: 0,
                total_revenue: 0,
                total_wage: 0,
                total_maintenance: 0,
                total_fuel: 0,
                total_net: 0,
            });
        }
        let Some(revenue) = row.get::<_, Option<i64>>(1).map_err(read_error)? else {
            continue;
        };
        let wage: i64 = row.get(2).map_err(read_error)?;
        let maintenance: i64 = row.get(3).map_err(read_error)?;
        let fuel: i64 = row.get(4).map_err(read_error)?;
        let distance: i64 = row.get(5).map_err(read_error)?;
        let loaded = row.get::<_, i64>(6).map_err(read_error)? != 0;
        let stat = drivers.last_mut().ok_or(ReadError::Internal)?;
        stat.trip_count = stat.trip_count.checked_add(1).ok_or(ReadError::Internal)?;
        if loaded {
            stat.loaded_trip_count = stat
                .loaded_trip_count
                .checked_add(1)
                .ok_or(ReadError::Internal)?;
        } else {
            stat.empty_trip_count = stat
                .empty_trip_count
                .checked_add(1)
                .ok_or(ReadError::Internal)?;
        }
        stat.total_distance = stat
            .total_distance
            .checked_add(distance)
            .ok_or(ReadError::Internal)?;
        stat.total_revenue = stat
            .total_revenue
            .checked_add(revenue)
            .ok_or(ReadError::Internal)?;
        stat.total_wage = stat
            .total_wage
            .checked_add(wage)
            .ok_or(ReadError::Internal)?;
        stat.total_maintenance = stat
            .total_maintenance
            .checked_add(maintenance)
            .ok_or(ReadError::Internal)?;
        stat.total_fuel = stat
            .total_fuel
            .checked_add(fuel)
            .ok_or(ReadError::Internal)?;
        stat.total_net = stat
            .total_net
            .checked_add(checked_net(revenue, wage, maintenance, fuel)?)
            .ok_or(ReadError::Internal)?;
    }
    Ok(DriverStatsRead { drivers })
}

pub(crate) fn default_database_path() -> Result<PathBuf, Error> {
    BaseDirs::new()
        .map(|b| {
            b.data_local_dir()
                .join("truck-ledger")
                .join("truck-ledger.sqlite3")
        })
        .ok_or_else(|| Error::Input("could not resolve platform local data directory".into()))
}
pub fn prepare_database_path(path: &Path) -> Result<(), Error> {
    if path.is_dir() {
        return Err(Error::Input(format!(
            "database path is a directory: {}",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        if parent.exists() && !parent.is_dir() {
            return Err(Error::Input(format!(
                "database parent is not a directory: {}",
                parent.display()
            )));
        }
        fs::create_dir_all(parent).map_err(|e| {
            Error::Io(format!(
                "cannot create database directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    Ok(())
}

pub(crate) fn open_database(path: &Path) -> Result<Connection, Error> {
    prepare_database_path(path)?;
    let c = Connection::open(path).map_err(db_err)?;
    c.execute_batch("PRAGMA foreign_keys=ON;").map_err(db_err)?;
    let initial_version: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_err)?;
    if initial_version > 2 {
        return Err(Error::Database(format!(
            "unsupported schema version {initial_version}"
        )));
    }
    if initial_version == 0 {
        let objects: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE type IN ('table','index','trigger','view') AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0)).map_err(db_err)?;
        if objects != 0 {
            return Err(Error::Database(
                "unversioned database is not pristine".into(),
            ));
        }
    }
    if initial_version == 0 {
        c.execute_batch("BEGIN; CREATE TABLE profiles(id INTEGER PRIMARY KEY,scope_key TEXT NOT NULL UNIQUE CHECK(length(scope_key)>0)); CREATE TABLE drivers(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL,raw_driver_id TEXT NOT NULL,adr INTEGER NOT NULL,long_dist INTEGER NOT NULL,heavy INTEGER NOT NULL,fragile INTEGER NOT NULL,urgent INTEGER NOT NULL,mechanical INTEGER NOT NULL,hometown TEXT NOT NULL,current_city TEXT NOT NULL,experience_points INTEGER NOT NULL,FOREIGN KEY(profile_id) REFERENCES profiles(id),UNIQUE(profile_id,raw_driver_id),UNIQUE(id,profile_id)); CREATE TABLE trips(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL,driver_id INTEGER NOT NULL,fingerprint_version INTEGER NOT NULL,fingerprint BLOB NOT NULL CHECK(typeof(fingerprint)='blob' AND length(fingerprint)=32),timestamp_day INTEGER NOT NULL,revenue INTEGER NOT NULL,wage INTEGER NOT NULL,maintenance INTEGER NOT NULL,fuel INTEGER NOT NULL,distance INTEGER NOT NULL,distance_on_job INTEGER NOT NULL,cargo_count INTEGER NOT NULL,cargo TEXT NOT NULL,source_city TEXT NOT NULL,source_company TEXT NOT NULL,destination_city TEXT NOT NULL,destination_company TEXT NOT NULL,FOREIGN KEY(driver_id,profile_id) REFERENCES drivers(id,profile_id),UNIQUE(profile_id,fingerprint_version,fingerprint)); CREATE TABLE profile_locations(profile_id INTEGER PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,profile_root TEXT NOT NULL,layout_kind TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(db_err)?;
    } else if initial_version == 1 {
        c.execute_batch("BEGIN; CREATE TABLE profile_locations(profile_id INTEGER PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,profile_root TEXT NOT NULL,layout_kind TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(db_err)?;
    }
    Ok(c)
}
pub fn ingest(
    conn: &mut Connection,
    scope: &str,
    drivers: &[Driver],
) -> Result<IngestResult, Error> {
    if scope.is_empty() {
        return Err(Error::Input("profile scope key must be non-empty".into()));
    }
    let tx = conn.transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO profiles(scope_key) VALUES(?1) ON CONFLICT(scope_key) DO NOTHING",
        [scope],
    )
    .map_err(db_err)?;
    let pid: i64 = tx
        .query_row("SELECT id FROM profiles WHERE scope_key=?1", [scope], |r| {
            r.get(0)
        })
        .map_err(db_err)?;
    let (mut scanned, mut added) = (0, 0);
    for d in drivers {
        tx.execute("INSERT INTO drivers(profile_id,raw_driver_id,adr,long_dist,heavy,fragile,urgent,mechanical,hometown,current_city,experience_points) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(profile_id,raw_driver_id) DO UPDATE SET adr=excluded.adr,long_dist=excluded.long_dist,heavy=excluded.heavy,fragile=excluded.fragile,urgent=excluded.urgent,mechanical=excluded.mechanical,hometown=excluded.hometown,current_city=excluded.current_city,experience_points=excluded.experience_points",params![pid,d.raw_id,d.adr,d.long_dist,d.heavy,d.fragile,d.urgent,d.mechanical,d.hometown,d.current_city,d.experience_points]).map_err(db_err)?;
        let did: i64 = tx
            .query_row(
                "SELECT id FROM drivers WHERE profile_id=?1 AND raw_driver_id=?2",
                params![pid, d.raw_id],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        for t in &d.trips {
            scanned += 1;
            let h = fingerprint(&d.raw_id, t);
            added+=tx.execute("INSERT INTO trips(profile_id,driver_id,fingerprint_version,fingerprint,timestamp_day,revenue,wage,maintenance,fuel,distance,distance_on_job,cargo_count,cargo,source_city,source_company,destination_city,destination_company) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(profile_id,fingerprint_version,fingerprint) DO NOTHING",params![pid,did,h.as_slice(),t.timestamp_day,t.revenue,t.wage,t.maintenance,t.fuel,t.distance,i64::from(t.distance_on_job),t.cargo_count,t.cargo,t.source_city,t.source_company,t.destination_city,t.destination_company]).map_err(db_err)?
        }
    }
    tx.commit().map_err(db_err)?;
    Ok(IngestResult {
        hired_drivers_scanned: drivers.len(),
        visible_trips_scanned: scanned,
        newly_inserted_trips: added,
    })
}
/// Returns a profile's trips by raw ETS2 day, then fingerprint bytes only as a stable tie-breaker.
#[allow(dead_code)]
pub fn read_trips(conn: &Connection, scope: &str) -> Result<Vec<Trip>, Error> {
    let mut stmt = conn.prepare("SELECT t.timestamp_day,t.revenue,t.wage,t.maintenance,t.fuel,t.distance,t.distance_on_job,t.cargo_count,t.cargo,t.source_city,t.source_company,t.destination_city,t.destination_company FROM trips t JOIN profiles p ON p.id=t.profile_id WHERE p.scope_key=?1 ORDER BY t.timestamp_day ASC,t.fingerprint ASC").map_err(db_err)?;
    let rows = stmt
        .query_map([scope], |r| {
            Ok(Trip {
                timestamp_day: r.get(0)?,
                revenue: r.get(1)?,
                wage: r.get(2)?,
                maintenance: r.get(3)?,
                fuel: r.get(4)?,
                distance: r.get(5)?,
                distance_on_job: r.get::<_, i64>(6)? != 0,
                cargo_count: r.get(7)?,
                cargo: r.get(8)?,
                source_city: r.get(9)?,
                source_company: r.get(10)?,
                destination_city: r.get(11)?,
                destination_company: r.get(12)?,
            })
        })
        .map_err(db_err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
}

pub(crate) fn associate_profile_location(
    conn: &mut Connection,
    scope: &str,
    location: &ProfileLocation,
) -> Result<(), Error> {
    if scope.is_empty() {
        return Err(Error::Input("profile scope key must be non-empty".into()));
    }
    let root = location.profile_root.to_str().ok_or_else(|| {
        Error::Input(format!(
            "profile root is not valid UTF-8 and cannot be persisted: {}",
            location.profile_root.display()
        ))
    })?;
    let tx = conn.transaction().map_err(db_err)?;
    tx.execute(
        "INSERT INTO profiles(scope_key) VALUES(?1) ON CONFLICT(scope_key) DO NOTHING",
        [scope],
    )
    .map_err(db_err)?;
    let id: i64 = tx
        .query_row("SELECT id FROM profiles WHERE scope_key=?1", [scope], |r| {
            r.get(0)
        })
        .map_err(db_err)?;
    tx.execute(
        "INSERT INTO profile_locations(profile_id,profile_root,layout_kind) VALUES(?1,?2,?3) ON CONFLICT(profile_id) DO UPDATE SET profile_root=excluded.profile_root,layout_kind=excluded.layout_kind",
        params![id, root, location.layout_kind],
    )
    .map_err(db_err)?;
    tx.commit().map_err(db_err)
}

/// Lists every configured scope without validating its saved locator.  Service
/// status needs this to describe broken configuration instead of hiding it.
pub fn configured_profile_scopes(conn: &Connection) -> Result<Vec<String>, Error> {
    let mut statement = conn
        .prepare(
            "SELECT p.scope_key FROM profiles p JOIN profile_locations l ON l.profile_id=p.id ORDER BY p.scope_key",
        )
        .map_err(db_err)?;
    statement
        .query_map([], |row| row.get(0))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)
}

#[cfg(test)]
#[path = "db/tests.rs"]
mod tests;
