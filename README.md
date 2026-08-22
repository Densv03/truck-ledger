# TruckLedger

TruckLedger archives hired-driver history and analytics for Euro Truck Simulator 2 (ETS2), locally. ETS2 keeps only a short rolling history of driver profits in
save files; TruckLedger preserves that history over time for long-term driver performance analysis.

TruckLedger is local-only and read-only toward ETS2 saves.

## Features

- Automatic hired-driver trip-history archiving.
- Lifetime profitability per driver, including top and losing driver views.
- Loaded and empty/reposition trip counts.
- Cumulative distance, revenue, cost, and net statistics.
- Multiple ETS2 profile scopes.
- Automatic background collection on macOS.
- Driver-specific trip-history drill-down.
- Local browser dashboard and read-only HTTP API.
- Append-only historical archive with versioned content-fingerprint deduplication.
- No cloud account, remote sync, or telemetry.
- macOS arm64 and x86_64 `.pkg` releases.

## Quick start

TruckLedger supports macOS 13 Ventura or later.

1. Download matching `.pkg` from [GitHub Releases](../../releases):
   - Apple Silicon / M-series: `truck-ledger-<version>-macos-arm64.pkg`
   - Intel: `truck-ledger-<version>-macos-x86_64.pkg`
2. Install package.
3. If macOS blocks unsigned package, open **System Settings → Privacy &
   Security → Open Anyway**.
4. Run:

   ```sh
   truck-ledger setup
   ```

5. Play ETS2 normally.
6. Optional dashboard:

   ```sh
   truck-ledger serve
   ```

   Open <http://127.0.0.1:32947/>.

Current packages are unsigned and non-notarized because project does not
maintain paid Apple Developer Program membership. Approve only this installer;
do not disable Gatekeeper globally.

## Commands

| Command | Purpose |
| --- | --- |
| `truck-ledger` | Print concise first-run help. |
| `truck-ledger setup` | Install or refresh macOS background collection for configured profiles. |
| `truck-ledger status` | Show macOS collector installation, service state, database, and profile locators. |
| `truck-ledger monitor` | Follow managed collector logs without changing collection state. |
| `truck-ledger serve` | Start local dashboard and read-only HTTP API. |
| `truck-ledger discover` | List ETS2 profile candidates without changing data or configuration. |
| `truck-ledger watch` | Run foreground collection for one selected profile. |
| `truck-ledger collect` | Run foreground collection for every valid persisted profile locator. |
| `truck-ledger ingest` | Import one save file into a named profile scope; useful for manual/debug work. |
| `truck-ledger uninstall` | Remove managed macOS collector artifacts while preserving history. |

Use `truck-ledger <command> --help` for flags.

## Dashboard

`truck-ledger serve` starts driver-first local dashboard. Select profile, then view profile summary, top drivers by cumulative lifetime net, losing drivers,
and lifetime statistics for every driver. Select driver to drill into that driver's archived trips.

<img width="1728" height="994" alt="image" src="https://github.com/user-attachments/assets/c1af5356-ed67-42b0-baa8-29587c61f875" />


Dashboard is read-only. Server binds IPv4 loopback only (`127.0.0.1`), so it
is not exposed to LAN or internet.

## How it works

TruckLedger discovers or configures ETS2 profile. Background collector watches profile save tree. When save changes, TruckLedger snapshots and reads it,
extracts hired-driver trip records, deduplicates already-known trips, then appends new history to local SQLite database.

TruckLedger never modifies ETS2 saves. Deleting or rotating old ETS2 saves
does not delete previously archived TruckLedger history.

## Profiles and discovery

On macOS, automatic discovery checks local ETS2 profiles beneath:

```text
~/Library/Application Support/Euro Truck Simulator 2/profiles/*/save
```

It also checks Steam Cloud profiles beneath:

```text
~/Library/Application Support/Steam/userdata/<account>/227300/remote/profiles/*/save
```

First bare `truck-ledger setup` selects one uniquely discovered profile as
`default`. Multiple candidates cause clear error; select explicitly:

```sh
truck-ledger setup --profile personal --profile-root /path/to/profile
```

`--profile-root` must contain real `save/` directory. Selected absolute path is persisted as locator metadata for that profile scope. Later `setup` refreshes
managed service using valid persisted locators; it does not silently select new candidate. `watch --profile personal` uses persisted locator, while
`watch --profile personal --profile-root /path/to/profile` sets or replaces it.

`discover` is read-only. Use custom structural root when automatic discovery
does not apply:

```sh
truck-ledger discover --root /path/to/ets2-or-steam-data
```

## Background collection on macOS

`setup` manages one per-user LaunchAgent and one managed collector process. It
starts at login and watches all configured valid profiles. Collection uses
filesystem events, not idle polling.

Use `status` to inspect service and configured locators. Use `monitor` to
follow collector logs. Use `uninstall` to remove managed service. History and
profile locators survive service removal.

`collect` is foreground multi-profile collector. `watch` is foreground
single-profile collector for manual or debug use.

## Data and privacy

All TruckLedger data stays local. ETS2 saves are read-only. No cloud backend, account, remote sync, or telemetry exists. SQLite stores archive locally;
package upgrades preserve history. Local HTTP server accepts loopback requests only.

Default macOS database path is:

```text
~/Library/Application Support/truck-ledger/truck-ledger.sqlite3
```

## Local API

`serve` exposes read-only JSON API on loopback only:

```text
GET /api/v1
GET /api/v1/profiles
GET /api/v1/profiles/{scope}/drivers
GET /api/v1/profiles/{scope}/trips
GET /api/v1/profiles/{scope}/summary
GET /api/v1/profiles/{scope}/driver-stats
```

Domain values stored as `i64` serialize as decimal strings, preserving exact values in JSON clients.

## Advanced/manual usage

Discover from specified root:

```sh
truck-ledger discover --root /path/to/ets2-or-steam-data
```

Watch one profile in foreground:

```sh
truck-ledger watch --profile personal --profile-root /path/to/profile
truck-ledger watch --profile personal
```

Import single decoded or game save manually:

```sh
truck-ledger ingest --profile fixture-profile --input game.sii
```

Override database for `ingest`, `watch`, or `serve`:

```sh
truck-ledger ingest --profile fixture-profile --input game.sii --database /tmp/ledger.sqlite3
truck-ledger serve --database /tmp/ledger.sqlite3 --port 40000
```

`serve --port 40000` changes local dashboard/API port. `collect` needs valid
persisted locators and uses default database.

## Limitations

- macOS is primary supported automatic-discovery and service platform.
- Windows/Linux automatic default discovery and service setup are not implemented.
- Current macOS packages are unsigned and non-notarized.
- Raw ETS2 driver IDs can appear as fallback identity: current save data does not reliably expose human-readable driver names.
- Hired-driver detection uses observed non-empty hometown heuristic.
- `timestamp_day` is raw ETS2 data, not wall-clock date.
- Money values are raw game-currency integers; no real-world currency semantics assumed.
- Loading older save does not branch or roll back TruckLedger history.

## Verify downloads

Release checksum files publish beside package. Verify downloaded package:

```sh
shasum -a 256 -c truck-ledger-<version>-macos-<architecture>.pkg.sha256
```

## Uninstall

1. Remove managed collector:

   ```sh
   truck-ledger uninstall
   ```

2. Optional: remove system CLI:

   ```sh
   sudo rm /usr/local/bin/truck-ledger
   sudo pkgutil --forget com.truckledger.cli
   ```

History/database remains. `pkgutil --forget` removes package receipt only; it
does not remove files.

## Development

Rust 1.85 or newer required for source builds. Cargo install is developer path, not normal-user installation:

```sh
cargo install --path . --locked
```

Run normal checks:

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked
cargo build --release --locked
```

Build and validate unsigned local macOS package:

```sh
cargo build --release --locked
./packaging/macos/build-pkg.sh --output-dir dist
./packaging/macos/validate-pkg.sh --pkg dist/truck-ledger-<version>-macos-<architecture>.pkg
```

`reference/` holds sanitized fixtures, historical prototype code, and upstream notes. It is evidence and test material, not production architecture.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE).
