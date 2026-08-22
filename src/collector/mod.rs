mod discovery;
mod watch;

pub(crate) use crate::db::{associate_profile_location, configured_profile_scopes};
pub(crate) use crate::domain::{CollectionSummary, ProfileCandidate, ProfileLocation};
#[allow(unused_imports)]
pub(crate) use discovery::{
    ETS2_APP_ID, absolute_lexical, discover, discover_custom_roots, discover_default,
    discover_macos_data_dir, load_profile_location, validate_profile_root,
};
#[allow(unused_imports)]
pub(crate) use watch::{
    CollectRoot, DEBOUNCE, RETRY_BACKOFFS, collect, collect_root, collect_save_tree,
    configured_collect_roots, discover_save_files, event_is_relevant, print_summary, retry_source,
    watch,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
