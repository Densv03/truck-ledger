mod fingerprint;
mod sii;

use crate::{
    db,
    domain::{Error, IngestResult},
};
use std::{fs, path::Path};

pub(crate) use fingerprint::fingerprint;
pub(crate) use sii::{decode_input, extract};
#[allow(dead_code)]
pub(crate) const FINGERPRINT_VERSION: i64 = 1;

pub(crate) fn ingest_path(
    database: &Path,
    scope: &str,
    input: &Path,
) -> Result<IngestResult, Error> {
    let raw = fs::read(input)
        .map_err(|e| Error::Io(format!("cannot read input {}: {e}", input.display())))?;
    let decoded = decode_input(&raw)?;
    let drivers = extract(&decoded)?;
    let mut db = db::open_database(database)?;
    db::ingest(&mut db, scope, &drivers)
}
