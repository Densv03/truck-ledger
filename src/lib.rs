//! Internal subsystem facade. Public exports exist only for binary entry point and integration-style tests.
pub(crate) mod api;
pub mod cli;
mod collector;
mod db;
mod domain;
mod ingest;
pub(crate) mod macos;

#[allow(unused_imports)]
pub(crate) use api::DEFAULT_API_PORT;
#[allow(unused_imports)]
pub(crate) use collector::{
    CollectionSummary, DEBOUNCE, ETS2_APP_ID, ProfileCandidate, ProfileLocation,
    associate_profile_location, collect, collect_save_tree, configured_profile_scopes, discover,
    discover_custom_roots, discover_default, discover_save_files, event_is_relevant,
    load_profile_location, print_summary, validate_profile_root, watch,
};
#[allow(unused_imports)]
pub(crate) use db::{
    default_database_path, ingest, open_database, prepare_database_path, read_trips,
};
#[allow(unused_imports)]
pub(crate) use domain::{Driver, Error, IngestResult, Trip};
#[allow(unused_imports)]
pub(crate) use ingest::{FINGERPRINT_VERSION, decode_input, extract, fingerprint, ingest_path};
