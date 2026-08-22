use directories::BaseDirs;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const FINGERPRINT_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driver {
    pub raw_id: String,
    pub adr: i64,
    pub long_dist: i64,
    pub heavy: i64,
    pub fragile: i64,
    pub urgent: i64,
    pub mechanical: i64,
    pub hometown: String,
    pub current_city: String,
    pub experience_points: i64,
    pub trips: Vec<Trip>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trip {
    pub timestamp_day: i64,
    pub revenue: i64,
    pub wage: i64,
    pub maintenance: i64,
    pub fuel: i64,
    pub distance: i64,
    pub distance_on_job: bool,
    pub cargo_count: i64,
    pub cargo: String,
    pub source_city: String,
    pub source_company: String,
    pub destination_city: String,
    pub destination_company: String,
}
impl Trip {
    pub fn net(&self) -> Result<i64, Error> {
        self.revenue
            .checked_sub(self.wage)
            .and_then(|x| x.checked_sub(self.maintenance))
            .and_then(|x| x.checked_sub(self.fuel))
            .ok_or_else(|| Error::Parse("trip net overflows i64".into()))
    }
}
#[derive(Debug)]
pub struct IngestResult {
    pub hired_drivers_scanned: usize,
    pub visible_trips_scanned: usize,
    pub newly_inserted_trips: usize,
}
#[derive(Debug)]
pub enum Error {
    Io(String),
    Input(String),
    Decode(String),
    Parse(String),
    Database(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(x) | Self::Input(x) | Self::Decode(x) | Self::Parse(x) | Self::Database(x) => {
                f.write_str(x)
            }
        }
    }
}
impl std::error::Error for Error {}
fn db_err(e: rusqlite::Error) -> Error {
    Error::Database(e.to_string())
}

pub fn default_database_path() -> Result<PathBuf, Error> {
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

pub fn decode_input(bytes: &[u8]) -> Result<String, Error> {
    if bytes.starts_with(b"BSII") {
        return Err(Error::Input(
            "unsupported input format BSII; Phase 1 accepts ScsC or SiiNunit".into(),
        ));
    }
    let output = if bytes.starts_with(b"ScsC") {
        sii_decode::file_type::decode_until_siin(bytes)
            .map_err(|e| Error::Decode(format!("ScsC decode failed: {e}")))?
    } else if bytes.starts_with(b"SiiNunit") {
        bytes.to_vec()
    } else {
        return Err(Error::Input(
            "unsupported input format; expected ScsC or SiiNunit header".into(),
        ));
    };
    String::from_utf8(output).map_err(|e| Error::Parse(format!("decoded SII is not UTF-8: {e}")))
}

#[derive(Default)]
struct Block {
    kind: String,
    id: String,
    fields: BTreeMap<String, String>,
}
fn unquote(v: &str) -> Result<String, Error> {
    let v = v.trim();
    if !v.starts_with('"') {
        return Ok(v.to_string());
    }
    if !v.ends_with('"') || v.len() < 2 {
        return Err(Error::Parse("malformed quoted string".into()));
    }
    let mut out = String::new();
    let mut esc = false;
    for c in v[1..v.len() - 1].chars() {
        if esc {
            match c {
                '"' | '\\' => out.push(c),
                _ => return Err(Error::Parse("unsupported string escape".into())),
            };
            esc = false
        } else if c == '\\' {
            esc = true
        } else {
            out.push(c)
        }
    }
    if esc {
        return Err(Error::Parse("unterminated string escape".into()));
    }
    Ok(out)
}
fn parse_blocks(s: &str) -> Result<Vec<Block>, Error> {
    if !s.starts_with("SiiNunit") {
        return Err(Error::Parse("missing SiiNunit header".into()));
    }
    let mut blocks = Vec::new();
    let mut current: Option<Block> = None;
    for (n, line) in s.lines().enumerate().skip(1) {
        let t = line.trim();
        if t.is_empty() || t == "{" {
            continue;
        }
        if t == "}" {
            if let Some(b) = current.take() {
                blocks.push(b)
            }
            continue;
        }
        if current.is_none() {
            let (head, _) = t
                .split_once('{')
                .ok_or_else(|| Error::Parse(format!("line {}: malformed block", n + 1)))?;
            let (kind, id) = head
                .split_once(':')
                .ok_or_else(|| Error::Parse(format!("line {}: malformed block", n + 1)))?;
            current = Some(Block {
                kind: kind.trim().into(),
                id: id.trim().into(),
                ..Default::default()
            });
            continue;
        }
        let (k, v) = t
            .split_once(':')
            .ok_or_else(|| Error::Parse(format!("line {}: malformed field", n + 1)))?;
        current
            .as_mut()
            .unwrap()
            .fields
            .insert(k.trim().into(), v.trim().into());
    }
    if current.is_some() {
        return Err(Error::Parse("unterminated block".into()));
    }
    Ok(blocks)
}
fn reqs(b: &Block, k: &str, ctx: &str) -> Result<String, Error> {
    b.fields
        .get(k)
        .ok_or_else(|| Error::Parse(format!("{ctx} {} {} missing field {k}", b.kind, b.id)))
        .and_then(|v| unquote(v))
}
fn reqi(b: &Block, k: &str, ctx: &str) -> Result<i64, Error> {
    reqs(b, k, ctx)?
        .parse()
        .map_err(|_| Error::Parse(format!("{ctx} {} {} field {k} must be i64", b.kind, b.id)))
}
fn reqb(b: &Block, k: &str, ctx: &str) -> Result<bool, Error> {
    match reqs(b, k, ctx)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(Error::Parse(format!(
            "{ctx} {} {} field {k} must be boolean",
            b.kind, b.id
        ))),
    }
}
pub fn extract(s: &str) -> Result<Vec<Driver>, Error> {
    let blocks = parse_blocks(s)?;
    let mut map = BTreeMap::new();
    for b in blocks {
        map.insert(b.id.clone(), b);
    }
    let mut result = Vec::new();
    for b in map.values().filter(|b| b.kind == "driver_ai") {
        let hometown = reqs(b, "hometown", "driver")?;
        if hometown.is_empty() {
            continue;
        }
        let raw_id = b.id.clone();
        let log = reqs(b, "profit_log", &format!("driver {raw_id}"))?;
        let logb = map
            .get(&log)
            .filter(|x| x.kind == "profit_log")
            .ok_or_else(|| {
                Error::Parse(format!("driver {raw_id}: profit_log target {log} missing"))
            })?;
        let mut refs: Vec<_> = logb
            .fields
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("stats_data[")
                    .and_then(|x| x.strip_suffix(']'))
                    .and_then(|x| x.parse::<usize>().ok())
                    .map(|i| (i, v))
            })
            .collect();
        refs.sort_by_key(|x| x.0);
        let mut trips = Vec::new();
        for (_, r) in refs {
            let id = unquote(r)?;
            let t = map
                .get(&id)
                .filter(|x| x.kind == "profit_log_entry")
                .ok_or_else(|| {
                    Error::Parse(format!("driver {raw_id}: trip target {id} missing"))
                })?;
            let trip = Trip {
                timestamp_day: reqi(t, "timestamp_day", &raw_id)?,
                revenue: reqi(t, "revenue", &raw_id)?,
                wage: reqi(t, "wage", &raw_id)?,
                maintenance: reqi(t, "maintenance", &raw_id)?,
                fuel: reqi(t, "fuel", &raw_id)?,
                distance: reqi(t, "distance", &raw_id)?,
                distance_on_job: reqb(t, "distance_on_job", &raw_id)?,
                cargo_count: reqi(t, "cargo_count", &raw_id)?,
                cargo: reqs(t, "cargo", &raw_id)?,
                source_city: reqs(t, "source_city", &raw_id)?,
                source_company: reqs(t, "source_company", &raw_id)?,
                destination_city: reqs(t, "destination_city", &raw_id)?,
                destination_company: reqs(t, "destination_company", &raw_id)?,
            };
            trip.net()?;
            trips.push(trip)
        }
        result.push(Driver {
            raw_id,
            adr: reqi(b, "adr", "driver")?,
            long_dist: reqi(b, "long_dist", "driver")?,
            heavy: reqi(b, "heavy", "driver")?,
            fragile: reqi(b, "fragile", "driver")?,
            urgent: reqi(b, "urgent", "driver")?,
            mechanical: reqi(b, "mechanical", "driver")?,
            hometown,
            current_city: reqs(b, "current_city", "driver")?,
            experience_points: reqi(b, "experience_points", "driver")?,
            trips,
        })
    }
    Ok(result)
}
pub fn fingerprint(driver: &str, t: &Trip) -> [u8; 32] {
    let mut b = Vec::new();
    b.extend_from_slice(b"truck-ledger.trip.v1\0");
    fn i(b: &mut Vec<u8>, x: i64) {
        b.extend_from_slice(&x.to_be_bytes())
    }
    fn st(b: &mut Vec<u8>, s: &str) {
        b.extend_from_slice(&(s.len() as u64).to_be_bytes());
        b.extend_from_slice(s.as_bytes())
    }
    st(&mut b, driver);
    for x in [
        t.timestamp_day,
        t.revenue,
        t.wage,
        t.maintenance,
        t.fuel,
        t.distance,
    ] {
        i(&mut b, x)
    }
    b.push(u8::from(t.distance_on_job));
    i(&mut b, t.cargo_count);
    for x in [
        &t.cargo,
        &t.source_city,
        &t.source_company,
        &t.destination_city,
        &t.destination_company,
    ] {
        st(&mut b, x)
    }
    Sha256::digest(b).into()
}

pub fn open_database(path: &Path) -> Result<Connection, Error> {
    prepare_database_path(path)?;
    let c = Connection::open(path).map_err(db_err)?;
    c.execute_batch("PRAGMA foreign_keys=ON;").map_err(db_err)?;
    let initial_version: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_err)?;
    if initial_version != 0 && initial_version != 1 {
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
    c.execute_batch("PRAGMA foreign_keys=ON; BEGIN; CREATE TABLE IF NOT EXISTS profiles(id INTEGER PRIMARY KEY,scope_key TEXT NOT NULL UNIQUE CHECK(length(scope_key)>0)); CREATE TABLE IF NOT EXISTS drivers(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL,raw_driver_id TEXT NOT NULL,adr INTEGER NOT NULL,long_dist INTEGER NOT NULL,heavy INTEGER NOT NULL,fragile INTEGER NOT NULL,urgent INTEGER NOT NULL,mechanical INTEGER NOT NULL,hometown TEXT NOT NULL,current_city TEXT NOT NULL,experience_points INTEGER NOT NULL,FOREIGN KEY(profile_id) REFERENCES profiles(id),UNIQUE(profile_id,raw_driver_id),UNIQUE(id,profile_id)); CREATE TABLE IF NOT EXISTS trips(id INTEGER PRIMARY KEY,profile_id INTEGER NOT NULL,driver_id INTEGER NOT NULL,fingerprint_version INTEGER NOT NULL,fingerprint BLOB NOT NULL CHECK(typeof(fingerprint)='blob' AND length(fingerprint)=32),timestamp_day INTEGER NOT NULL,revenue INTEGER NOT NULL,wage INTEGER NOT NULL,maintenance INTEGER NOT NULL,fuel INTEGER NOT NULL,distance INTEGER NOT NULL,distance_on_job INTEGER NOT NULL,cargo_count INTEGER NOT NULL,cargo TEXT NOT NULL,source_city TEXT NOT NULL,source_company TEXT NOT NULL,destination_city TEXT NOT NULL,destination_company TEXT NOT NULL,FOREIGN KEY(driver_id,profile_id) REFERENCES drivers(id,profile_id),UNIQUE(profile_id,fingerprint_version,fingerprint)); COMMIT;").map_err(db_err)?;
    let v: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_err)?;
    if v == 0 {
        c.execute_batch("BEGIN; PRAGMA user_version=1; COMMIT;")
            .map_err(db_err)?
    } else if v != 1 {
        return Err(Error::Database(format!("unsupported schema version {v}")));
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
pub fn ingest_path(database: &Path, scope: &str, input: &Path) -> Result<IngestResult, Error> {
    let raw = fs::read(input)
        .map_err(|e| Error::Io(format!("cannot read input {}: {e}", input.display())))?;
    let decoded = decode_input(&raw)?;
    let drivers = extract(&decoded)?;
    let mut db = open_database(database)?;
    ingest(&mut db, scope, &drivers)
}

/// Returns a profile's trips by raw ETS2 day, then fingerprint bytes only as a stable tie-breaker.
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    fn fixture() -> String {
        include_str!("../reference/fixtures/hired_drivers_minimal.sii").into()
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
            1
        );
        drop(c);
        open_database(&p).unwrap();
        let p2 = td.path().join("newer.db");
        let c = Connection::open(&p2).unwrap();
        c.execute_batch("PRAGMA user_version=2;").unwrap();
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
}
