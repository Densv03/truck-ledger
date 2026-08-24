# Contributing to TruckLedger

## Prerequisites

Install Rust **1.85** or newer. TruckLedger uses Rust edition 2024; `1.85` is
the declared minimum supported Rust version (MSRV). Raise this baseline only
deliberately and document why.

Python 3 is needed only for manual verification of historical reference
material. It is not a TruckLedger runtime dependency.

## Local checks

Run all checks before submitting changes:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked
cargo build --release --locked
cargo run -- --help
cargo run -- --version
git diff --check
git diff -- reference
```

`Cargo.lock` is committed because TruckLedger is an application. Update it
only through intentional dependency changes.

`vendor/sii-decode-rs` is MIT-licensed source from upstream commit
`f65cc2f68401e74cfb7c1aae497d196c79d051a7`. Its provenance file records
TruckLedger-only delta; preserve both when updating dependency source.

## macOS package and release validation

Build unsigned local developer package without Apple credentials:

```sh
cargo build --release --locked
./packaging/macos/build-pkg.sh --output-dir dist
./packaging/macos/validate-pkg.sh --pkg dist/truck-ledger-<version>-macos-<architecture>.pkg
```

`build-pkg.sh` derives Cargo package version and inspects native release binary
for one supported slice. It packages only `/usr/local/bin/truck-ledger` with
identifier `com.truckledger.cli`; validator expands without installing and
checks metadata, payload, permissions, binary version, architecture, and BOM.

See [RELEASING.md](RELEASING.md) for normal release flow. Cargo package version
declares release intent; successful CI on `main` creates tag and publishes both
unsigned native macOS packages automatically. Do not normally create a
release-preparation branch or manually create/push release tags.

## Reference fixture verification

`reference/` is historical evidence, not production code. Its sanitized
fixture can be checked manually with its historical Python prototype. Use a
temporary database; do not create one in the repository:

```sh
tmp_dir="$(mktemp -d)"
python3 reference/prototype/ets2_driver_ledger.py \
  --db "$tmp_dir/ledger.sqlite3" \
  ingest reference/fixtures/hired_drivers_minimal.sii
python3 reference/prototype/ets2_driver_ledger.py \
  --db "$tmp_dir/ledger.sqlite3" \
  ingest reference/fixtures/hired_drivers_minimal.sii
rm -rf "$tmp_dir"
```

Expected behavior: two hired drivers and two visible trips; first import adds
two trips, second import adds zero. This check does not belong in `cargo test`.
Phase 1 will reuse sanitized fixtures for production Rust tests.

Phase 1 CLI smoke test:

```sh
tmp_dir="$(mktemp -d)"
cargo run -- ingest --profile fixture-profile --input reference/fixtures/hired_drivers_minimal.sii --database "$tmp_dir/ledger.sqlite3"
cargo run -- ingest --profile fixture-profile --input reference/fixtures/hired_drivers_minimal.sii --database "$tmp_dir/ledger.sqlite3"
rm -rf "$tmp_dir"
```

## Phase 2 synthetic checks

Phase 2 tests build synthetic profile/save trees; they never require personal
Steam or ETS2 data. Run normal verification with:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked
cargo build --release --locked
cargo run -- discover --help
cargo run -- watch --help
cargo run -- collect --help
cargo run -- setup --help
cargo run -- status --help
cargo run -- uninstall --help
cargo run -- monitor --help
cargo run -- serve --help
```

Optional macOS native-notification smoke test:

```sh
cargo test macos_notify_observes_save_tree_activity -- --ignored
```

This ignored synthetic test may time out in restricted FSEvents environments.
It is supplementary, not a normal quality gate. Real macOS Steam Cloud ETS2
end-to-end watcher collection has been manually verified separately.

Do not make normal tests depend on filesystem-event timing. `watch` is a
single-profile foreground collector; `collect` is persisted multi-profile
foreground collector. Both use native notifications after catch-up. macOS
`setup` creates only a user LaunchAgent; tests must never touch real
LaunchAgents, TruckLedger data, or ETS2 saves.

## Phase 4 API checks

HTTP integration tests use temporary SQLite files and ephemeral IPv4 loopback
ports only:

```sh
cargo test api::tests
cargo run -- serve --help
```

`serve` is a manual development API, not collector setup. It must open only an
existing schema-v2 database with SQLite read-only flags; tests must prove it
does not alter schema, rows, fingerprints, or profile locators.

## Phase 5 dashboard assets

Dashboard assets live in `web/` as plain `index.html`, `app.js`, and
`styles.css`. They are compiled into binary with `include_str!`; do not add a
runtime asset lookup, Node/npm toolchain, generated bundle, or API writes.

Use DOM APIs and `textContent` for persisted API values. API `i64` strings must
remain strings: `app.js` groups decimal digits directly and must not convert
money, distance, or other domain integers through JavaScript `Number`.
`timestamp_day` displays as raw `Day <value>`, never calendar date. Formatter
self-check vectors cover signed values through both i64 bounds during module
initialization; static-route coverage runs under `cargo test api::tests`.
Driver lifetime aggregation belongs in `GET /api/v1/profiles/{scope}/driver-stats`;
do not reintroduce browser-side full-archive scans.

## Scope and safety

Keep Phase 5 bounded. Do not add HTTP writes, remote/LAN serving, root/system
services, collector polling, frontend build tooling, or release packaging
before planned phases.

Never commit personal saves, profile identifiers, Steam IDs, local databases,
or hard-coded personal paths. Original ETS2 saves must remain read-only.

TruckLedger uses MIT licensing. Preserve attribution and separate provenance
for material under `reference/`.
