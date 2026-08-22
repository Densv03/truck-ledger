# TruckLedger roadmap

This roadmap defines v0.1 scope. Phases are intentional boundaries, not claims
that later functionality already exists.

## v0.1 engineering contract

TruckLedger will be one Rust executable, `truck-ledger`, for local ETS2
hired-driver history and analytics. It will read saves without modifying them,
store local history in SQLite, discover profiles and changed saves, and serve a
browser dashboard only on loopback. macOS is first tested platform; design
should permit later Windows/Linux support without unnecessary coupling.

Out of scope: cloud services, accounts, authentication, remote sync, telemetry
plugins, save editing, native desktop/menu-bar/tray apps, maps, advanced garage
or cargo analytics, ATS, rollback timeline management, auto-update, mobile,
unjustified React/Vue/Svelte, and Windows installer/service integration.

## Reference contract

`reference/` records useful observations, not guaranteed ETS2 format
contracts. Production code must validate assumptions and fail gracefully:

- Encoded saves observed as `ScsC`; decoded documents observed as `SiiNunit`.
- Hired drivers observed as `driver_ai` blocks with non-empty `hometown`;
  recruitment candidates observed with empty `hometown`.
- Observed traversal: `driver_ai -> profit_log -> stats_data[n] ->
  profit_log_entry`.
- Profit history observed as rolling/limited.
- `_nameless.*` references are not durable cross-save identifiers.
- Content-based trip fingerprints prevented duplicate ingestion.
- Collection must not assume only `autosave/game.sii` changes.
- Original save files must remain read-only.

## Phase 0 — foundation

**Objective:** establish scope, engineering contract, repository foundation.

**Included:** repository/reference inspection; documented validated prototype
behavior; Rust binary foundation; MIT license; roadmap and contributor docs.

**Excluded:** decoding, parsing, SQLite persistence, discovery, watching, HTTP,
and UI.

**Acceptance:** `truck-ledger --help` and `--version` work; documentation does
not claim unimplemented features; Cargo build/test/fmt/clippy/release-build
checks pass; reference fixture is manually verified.

## Phase 1 — decode, parse, and preserve

**Objective:** turn supplied decoded saves into durable, profile-scoped history.

**Included:** validated SCS decoding integration; SII parsing; hired-driver and
trip extraction; canonical v1 trip fingerprinting; SQLite persistence; Rust
tests using sanitized fixtures.

**Excluded:** automatic profile/save discovery, filesystem watching, HTTP, UI.

**Acceptance:** malformed/unsupported input fails clearly without touching
original saves; fixture behavior passes production tests; SQLite deduplicates
within profile; repeated import is idempotent.

Profile scope is separate from content fingerprinting. Drivers are unique by
profile plus raw ETS2 driver ID. Trip uniqueness is
`(profile_id, fingerprint_version, fingerprint)`; v1 hash includes raw driver
ID and canonical values for `timestamp_day`, revenue, wage, maintenance, fuel,
distance, `distance_on_job`, cargo count/cargo, and source/destination city and
company. It excludes `_nameless.*` and all save-observation metadata.

Persist monetary fields and `timestamp_day` as raw `i64`/SQLite `INTEGER`
values. `net = revenue - wage - maintenance - fuel`; no currency, decimal, or
wall-clock time semantics are assumed.

## Phase 2 — discovery and collection

**Objective:** find ETS2 profiles/saves and collect changed saves automatically.

**Included:** platform-specific known-root probing; explicit custom roots;
structured profile candidates and selection; persisted local profile choice;
filesystem notifications; debounce; stable copied snapshots; bounded retry;
temporary cleanup.

**Excluded:** full-home/full-disk scanning, arbitrary profile selection,
unbounded polling/retries, HTTP, UI, rollback timeline management.

**Acceptance:** no personal paths/IDs hard-coded; one valid profile may be
selected as documented, multiples require selection; invalid configured profile
reports clearly; transient failures recover or terminate clearly without
blocking later saves; originals are never written.

Persistent data uses a platform-standard per-user application-data directory
with testable path resolution and explicit override. Tests use temporary paths.

## Phase 3 — local HTTP API

**Objective:** expose collected analytics to local clients.

**Included:** local HTTP API and explicit bind-address validation; configurable
port; loopback-only operation.

**Excluded:** remote/LAN binding, `--allow-remote`, authentication, cloud API,
and dashboard implementation.

**Acceptance:** default bind is `127.0.0.1`; `[::1]` works where practical;
non-loopback addresses are rejected; tests cover accepted/rejected addresses.

## Phase 4 — local web dashboard

**Objective:** present useful hired-driver history in a browser.

**Included:** minimal dashboard consuming local API; clear local-only UX.

**Excluded:** native desktop GUI, mobile interface, maps, advanced garage/cargo
analytics, framework adoption without demonstrated need.

**Acceptance:** browser dashboard displays collected data through loopback API;
no remote dependency/account is required.

## Phase 5 — operations and installation UX

**Objective:** make recurring local use understandable and reliable.

**Included:** background-service/installation UX and operational commands.

**Excluded:** auto-update, Windows installer/service integration, menu-bar/tray
apps, cloud operation.

**Acceptance:** macOS tested operational flow starts, reports status, and
stops cleanly; failures provide actionable local diagnostics.

## Phase 6 — release engineering

**Objective:** prepare public v0.1 distribution.

**Included:** packaged binaries, Homebrew tap, release documentation, public
v0.1 release work.

**Excluded:** unplanned product expansion or platform integrations outside
v0.1 contract.

**Acceptance:** reproducible packaged release; documented install/use/upgrade
path; quality gates and release checks pass; public v0.1 artifacts published.
