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
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
cargo build --release
cargo run -- --help
cargo run -- --version
git diff --check
git diff -- reference
```

`Cargo.lock` is committed because TruckLedger is an application. Update it
only through intentional dependency changes.

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
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
cargo build --release
cargo run -- discover --help
cargo run -- watch --help
```

Optional macOS native-notification smoke test:

```sh
cargo test macos_notify_observes_save_tree_activity -- --ignored
```

This ignored synthetic test may time out in restricted FSEvents environments.
It is supplementary, not a normal quality gate. Real macOS Steam Cloud ETS2
end-to-end watcher collection has been manually verified separately.

Do not make normal tests depend on filesystem-event timing. `watch` is a
foreground collector: it performs initial catch-up, then waits for native
filesystem notifications. It has no service installation or automatic login
startup in Phase 2.

## Scope and safety

Keep Phase 2 bounded. Do not add HTTP APIs, UI, service installation,
background startup, process polling, or permanent polling loops before their
planned phases.

Never commit personal saves, profile identifiers, Steam IDs, local databases,
or hard-coded personal paths. Original ETS2 saves must remain read-only.

TruckLedger uses MIT licensing. Preserve attribution and separate provenance
for material under `reference/`.
