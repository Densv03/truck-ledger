use directories::BaseDirs;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};
use walkdir::WalkDir;

pub mod service;

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

pub const ETS2_APP_ID: &str = "227300";
pub const DEBOUNCE: Duration = Duration::from_millis(300);
const RETRY_BACKOFFS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCandidate {
    pub layout_kind: String,
    pub profile_root: PathBuf,
    pub save_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLocation {
    pub profile_root: PathBuf,
    pub layout_kind: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CollectionSummary {
    pub files: usize,
    pub processed: usize,
    pub failed: usize,
    pub drivers_scanned: usize,
    pub trips_scanned: usize,
    pub inserted: usize,
}

impl CollectionSummary {
    fn add(&mut self, result: IngestResult) {
        self.processed += 1;
        self.drivers_scanned += result.hired_drivers_scanned;
        self.trips_scanned += result.visible_trips_scanned;
        self.inserted += result.newly_inserted_trips;
    }
}

fn absolute_lexical(path: &Path) -> Result<PathBuf, Error> {
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

fn real_directory(path: &Path, context: &str) -> Result<(), Error> {
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

pub fn associate_profile_location(
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

pub fn load_profile_location(conn: &Connection, scope: &str) -> Result<ProfileLocation, Error> {
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
        Err(error) => return Err(db_err(error)),
    };
    let mut location = validate_profile_root(Path::new(&profile_root)).map_err(|e| {
        Error::Input(format!(
            "saved profile location for {scope} is invalid: {e}"
        ))
    })?;
    location.layout_kind = layout_kind;
    Ok(location)
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

pub fn discover_custom_roots(roots: &[PathBuf]) -> Result<Vec<ProfileCandidate>, Error> {
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

fn discover_macos_data_dir(data: &Path) -> Result<Vec<ProfileCandidate>, Error> {
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

pub fn discover_save_files(save_root: &Path) -> Result<Vec<PathBuf>, Error> {
    real_directory(save_root, "save root")?;
    let mut files = Vec::new();
    for entry in WalkDir::new(save_root).follow_links(false) {
        let entry = entry.map_err(|e| {
            Error::Io(format!(
                "cannot scan save root {}: {e}",
                save_root.display()
            ))
        })?;
        if entry.file_type().is_symlink()
            || !entry.file_type().is_file()
            || entry.file_name() != "game.sii"
        {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| {
            Error::Io(format!(
                "cannot inspect save {}: {e}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        files.push(entry.into_path());
    }
    files.sort();
    Ok(files)
}

fn process_one_save(
    conn: &mut Connection,
    scope: &str,
    source: &Path,
) -> Result<IngestResult, Error> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|e| Error::Io(format!("cannot inspect save {}: {e}", source.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::Input(format!(
            "save is not a real regular file: {}",
            source.display()
        )));
    }
    let snapshot = tempfile::NamedTempFile::new()
        .map_err(|e| Error::Io(format!("cannot create save snapshot: {e}")))?;
    fs::copy(source, snapshot.path())
        .map_err(|e| Error::Io(format!("cannot snapshot save {}: {e}", source.display())))?;
    let raw = fs::read(snapshot.path())
        .map_err(|e| Error::Io(format!("cannot read save snapshot: {e}")))?;
    let decoded = decode_input(&raw)?;
    let drivers = extract(&decoded)?;
    ingest(conn, scope, &drivers)
}

pub fn collect_save_tree(
    conn: &mut Connection,
    scope: &str,
    save_root: &Path,
) -> Result<CollectionSummary, Error> {
    let files = discover_save_files(save_root)?;
    let mut summary = CollectionSummary {
        files: files.len(),
        ..Default::default()
    };
    for source in files {
        match retry_source(
            || process_one_save(conn, scope, &source),
            std::thread::sleep,
        )? {
            Ok(result) => summary.add(result),
            Err(error) => {
                summary.failed += 1;
                eprintln!(
                    "save failed: {}: retry exhausted: {error}",
                    source.display()
                );
            }
        }
    }
    Ok(summary)
}

fn retry_source<T, Attempt, Sleep>(
    mut attempt: Attempt,
    mut sleep: Sleep,
) -> Result<Result<T, Error>, Error>
where
    Attempt: FnMut() -> Result<T, Error>,
    Sleep: FnMut(Duration),
{
    let mut last = None;
    for backoff in RETRY_BACKOFFS
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
    {
        match attempt() {
            Ok(value) => return Ok(Ok(value)),
            Err(Error::Database(error)) => return Err(Error::Database(error)),
            Err(error) => {
                last = Some(error);
                if let Some(backoff) = backoff {
                    sleep(backoff);
                }
            }
        }
    }
    Ok(Err(last.expect("retry loop always records an error")))
}

pub fn event_is_relevant(save_root: &Path, paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return true;
    }
    paths.iter().any(|path| match absolute_lexical(path) {
        Ok(path) => path == save_root || path.starts_with(save_root),
        Err(_) => false,
    })
}

pub fn watch(conn: &mut Connection, scope: &str, location: &ProfileLocation) -> Result<(), Error> {
    let save_root = location.profile_root.join("save");
    real_directory(&save_root, "save root")?;
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|e| Error::Io(format!("watcher initialization failed: {e}")))?;
    use notify::Watcher;
    watcher
        .watch(&save_root, notify::RecursiveMode::Recursive)
        .map_err(|e| {
            Error::Io(format!(
                "watcher initialization failed for {}: {e}",
                save_root.display()
            ))
        })?;
    let initial = collect_save_tree(conn, scope, &save_root)?;
    print_summary("catch-up", &initial);
    println!("watching: {}", save_root.display());
    watch_events(conn, scope, &save_root, &receiver)
}

fn watch_events(
    conn: &mut Connection,
    scope: &str,
    save_root: &Path,
    receiver: &Receiver<notify::Result<notify::Event>>,
) -> Result<(), Error> {
    loop {
        let event = receiver
            .recv()
            .map_err(|e| Error::Io(format!("watcher event channel failed: {e}")))?
            .map_err(|e| Error::Io(format!("watcher event failed: {e}")))?;
        if !event_is_relevant(save_root, &event.paths) {
            continue;
        }
        let mut deadline = Instant::now() + DEBOUNCE;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Ok(next)) if event_is_relevant(save_root, &next.paths) => {
                    deadline = Instant::now() + DEBOUNCE
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(Error::Io(format!("watcher event failed: {error}"))),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Io("watcher event channel failed".into()));
                }
            }
        }
        let summary = collect_save_tree(conn, scope, save_root)?;
        print_summary("collection pass", &summary);
    }
}

pub fn print_summary(label: &str, summary: &CollectionSummary) {
    println!(
        "{label}: files={} processed={} failed={} drivers_scanned={} trips_scanned={} inserted={}",
        summary.files,
        summary.processed,
        summary.failed,
        summary.drivers_scanned,
        summary.trips_scanned,
        summary.inserted
    );
}

#[derive(Debug, Clone)]
struct CollectRoot {
    save_root: PathBuf,
    scopes: Vec<String>,
}

fn configured_collect_roots(conn: &Connection) -> Result<Vec<CollectRoot>, Error> {
    let mut roots: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for scope in configured_profile_scopes(conn)? {
        match load_profile_location(conn, &scope) {
            Ok(location) => {
                let save_root = location.profile_root.join("save");
                roots.entry(save_root).or_default().push(scope);
            }
            Err(error) => eprintln!("configured profile invalid: {scope}: {error}"),
        }
    }
    Ok(roots
        .into_iter()
        .map(|(save_root, scopes)| CollectRoot { save_root, scopes })
        .collect())
}

fn collect_root(
    conn: &mut Connection,
    root: &CollectRoot,
) -> Result<Vec<(String, CollectionSummary)>, Error> {
    let mut results = root
        .scopes
        .iter()
        .cloned()
        .map(|scope| {
            (
                scope,
                CollectionSummary {
                    files: 0,
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();
    let files = discover_save_files(&root.save_root)?;
    for (_, summary) in &mut results {
        summary.files = files.len();
    }
    for source in files {
        let drivers = match retry_source(
            || {
                let metadata = fs::symlink_metadata(&source).map_err(|e| {
                    Error::Io(format!("cannot inspect save {}: {e}", source.display()))
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(Error::Input(format!(
                        "save is not a real regular file: {}",
                        source.display()
                    )));
                }
                let snapshot = tempfile::NamedTempFile::new()
                    .map_err(|e| Error::Io(format!("cannot create save snapshot: {e}")))?;
                fs::copy(&source, snapshot.path()).map_err(|e| {
                    Error::Io(format!("cannot snapshot save {}: {e}", source.display()))
                })?;
                let raw = fs::read(snapshot.path())
                    .map_err(|e| Error::Io(format!("cannot read save snapshot: {e}")))?;
                extract(&decode_input(&raw)?)
            },
            std::thread::sleep,
        )? {
            Ok(drivers) => drivers,
            Err(error) => {
                eprintln!(
                    "save failed: {}: retry exhausted: {error}",
                    source.display()
                );
                for (_, summary) in &mut results {
                    summary.failed += 1;
                }
                continue;
            }
        };
        for (scope, summary) in &mut results {
            summary.add(ingest(conn, scope, &drivers)?);
        }
    }
    Ok(results)
}

/// Foreground multi-profile collector. It has no service-manager dependency.
pub fn collect(conn: &mut Connection) -> Result<(), Error> {
    let roots = configured_collect_roots(conn)?;
    if roots.is_empty() {
        return Err(Error::Input(
            "no valid configured profiles to monitor; run setup with --profile-root".into(),
        ));
    }
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|e| Error::Io(format!("watcher initialization failed: {e}")))?;
    use notify::Watcher;
    for root in &roots {
        watcher
            .watch(&root.save_root, notify::RecursiveMode::Recursive)
            .map_err(|e| {
                Error::Io(format!(
                    "watcher initialization failed for {}: {e}",
                    root.save_root.display()
                ))
            })?;
        for (scope, summary) in collect_root(conn, root)? {
            print_summary(&format!("catch-up [{scope}]"), &summary);
        }
        println!("watching: {}", root.save_root.display());
    }
    loop {
        let event = receiver
            .recv()
            .map_err(|_| Error::Io("watcher event channel failed".into()))?
            .map_err(|e| Error::Io(format!("watcher event failed: {e}")))?;
        let mut changed = roots
            .iter()
            .filter(|root| event_is_relevant(&root.save_root, &event.paths))
            .map(|root| root.save_root.clone())
            .collect::<Vec<_>>();
        if changed.is_empty() {
            continue;
        }
        let mut deadline = Instant::now() + DEBOUNCE;
        loop {
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Ok(next)) => {
                    for root in &roots {
                        if event_is_relevant(&root.save_root, &next.paths)
                            && !changed.contains(&root.save_root)
                        {
                            changed.push(root.save_root.clone());
                        }
                    }
                    deadline = Instant::now() + DEBOUNCE;
                }
                Ok(Err(error)) => return Err(Error::Io(format!("watcher event failed: {error}"))),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Io("watcher event channel failed".into()));
                }
            }
        }
        for root in &roots {
            if changed.contains(&root.save_root) {
                for (scope, summary) in collect_root(conn, root)? {
                    print_summary(&format!("collection pass [{scope}]"), &summary);
                }
            }
        }
    }
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
        associate_profile_location(&mut conn, "scope", &validate_profile_root(&root).unwrap())
            .unwrap();
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
            conn.query_row::<i64, _, _>(
                "SELECT id FROM profiles WHERE scope_key='first'",
                [],
                |r| r.get(0)
            )
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
}
