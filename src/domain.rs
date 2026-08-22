use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Driver {
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
    pub(crate) trips: Vec<Trip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Trip {
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
}
impl Trip {
    pub(crate) fn net(&self) -> Result<i64, Error> {
        self.revenue
            .checked_sub(self.wage)
            .and_then(|x| x.checked_sub(self.maintenance))
            .and_then(|x| x.checked_sub(self.fuel))
            .ok_or_else(|| Error::Parse("trip net overflows i64".into()))
    }
}

#[derive(Debug)]
pub(crate) struct IngestResult {
    pub(crate) hired_drivers_scanned: usize,
    pub(crate) visible_trips_scanned: usize,
    pub(crate) newly_inserted_trips: usize,
}

#[derive(Debug)]
pub(crate) enum Error {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileCandidate {
    pub(crate) layout_kind: String,
    pub(crate) profile_root: PathBuf,
    pub(crate) save_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileLocation {
    pub(crate) profile_root: PathBuf,
    pub(crate) layout_kind: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CollectionSummary {
    pub(crate) files: usize,
    pub(crate) processed: usize,
    pub(crate) failed: usize,
    pub(crate) drivers_scanned: usize,
    pub(crate) trips_scanned: usize,
    pub(crate) inserted: usize,
}

impl CollectionSummary {
    pub(crate) fn add(&mut self, result: IngestResult) {
        self.processed += 1;
        self.drivers_scanned += result.hired_drivers_scanned;
        self.trips_scanned += result.visible_trips_scanned;
        self.inserted += result.newly_inserted_trips;
    }
}
