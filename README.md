# TruckLedger

TruckLedger is local analytics software for hired drivers in Euro Truck
Simulator 2 (ETS2). ETS2 retains only a short rolling window of hired-driver
profit history in save files; TruckLedger is intended to collect that history
locally before it disappears.

TruckLedger is pre-v0.1 and under development. Phase 2 imports explicit saves
and can collect a selected ETS2 profile in foreground:

```sh
truck-ledger ingest --profile fixture-profile --input game.sii
truck-ledger ingest --profile fixture-profile --input decoded.sii --database /tmp/ledger.sqlite3
```

Default data is stored in platform local application data under
`truck-ledger/truck-ledger.sqlite3`.

## Discovery and collection

Automatic default discovery currently supports macOS only. It probes only:

- `~/Library/Application Support/Euro Truck Simulator 2/profiles/*/save`
- `~/Library/Application Support/Steam/userdata/<account>/227300/remote/profiles/*/save`

Use `discover` to list candidates. It never writes the database or selects a
profile:

```sh
truck-ledger discover
truck-ledger discover --root /explicit/ets2-or-steam-root
```

Custom roots work on supported Rust platforms, but are structurally bounded:
the supplied directory itself may be a profile; otherwise only `profiles/*` and
`userdata/<numeric-account>/227300/remote/profiles/*` are inspected. TruckLedger
does not recursively scan a home directory, disk, or unrelated parent path.
Windows and Linux automatic default discovery are not implemented; use
`discover --root` there.

Start manual single-profile foreground collection by explicitly selecting a profile root:

```sh
truck-ledger watch --profile my-history --profile-root /path/to/profile
truck-ledger watch --profile my-history
```

`--profile-root` must be a real directory with a real `save/` directory. Its
absolute UTF-8 lexical path is stored as locator metadata for that TruckLedger
profile scope; changing it updates metadata without changing stored trip or
driver identity. The second command uses that persisted locator. Missing or
invalid locators fail clearly; TruckLedger never silently chooses another
candidate. Non-UTF-8 selected paths cannot currently be persisted.

On startup, `watch` catches up every real regular `game.sii` found recursively
under selected `save/`; slot names are not hard-coded. It then uses native
filesystem notifications, not permanent polling. Relevant event bursts use a
300 ms quiet debounce then bounded tree rescan. Each source attempt copies the
live save to a fresh system temporary snapshot before decoding. Source/decode/
parse failures retry at most four times with 100/200/400 ms backoff; failures
for one save are reported and do not stop later events. Live saves are never
opened writable. Collection is append-only: loading an older ETS2 save does not
remove historical TruckLedger trips or infer timeline branches.

The macOS Steam Cloud collector has been manually verified against real ETS2
save writes: native events triggered debounced rescans, unchanged save content
deduplicated to zero inserts, and later hired-driver history changes inserted
new trips.

## macOS background collection

Phase 3 adds one-time macOS user LaunchAgent setup. It installs a managed copy
of current TruckLedger executable under normal TruckLedger data directory,
creates `~/Library/LaunchAgents/truck-ledger.collector.plist`, then starts one
event-driven collector. It starts again at macOS login; it does not poll ETS2
or periodically scan saves while idle.

```sh
truck-ledger setup
truck-ledger status
truck-ledger monitor
```

One LaunchAgent runs one `collect` process watching every configured valid
profile locator. First bare setup auto-configures one uniquely discovered ETS2
profile as `default`; ambiguous discovery requires explicit selection. Later
bare setup refreshes service using every persisted scope without discovery or
creating a new `default` scope. Use `--profile` and `--profile-root` only to
configure a specific scope, for example when adding another ETS2 profile.
Multiple logical scopes may intentionally share a locator when explicitly
supplied. The managed executable is stored at normal data
path `bin/truck-ledger`; logs are `logs/collector.stdout.log` and
`logs/collector.stderr.log` beside normal database.

`truck-ledger collect` runs same multi-profile collector in foreground, on
platforms where persisted locators and native watcher support exist.
`watch --profile` remains available for manual/debug use. Remove only service
artifacts with `truck-ledger uninstall`; SQLite history and profile locators
remain. Automatic setup/status/uninstall are macOS-only. Windows/Linux service
installation is not implemented.

`monitor` is optional: it shows recent collector stdout/stderr and follows new
activity without starting collection or changing service state. Closing it with
Ctrl+C never stops the background collector.

## Development local API

Phase 4 adds a manual, foreground, read-only HTTP API for dashboard
development:

```sh
truck-ledger serve
truck-ledger serve --port 40000
truck-ledger serve --database /tmp/ledger.sqlite3 --port 40000
```

Default origin is `http://127.0.0.1:32947`. `serve` binds only IPv4 loopback,
opens existing TruckLedger SQLite data read-only, and exposes versioned routes
under `/api/v1`. It never collects saves, starts a second collector, changes
LaunchAgent state, or modifies archived data. No dashboard exists yet; normal
Phase 3 background collection remains independent. API `i64` domain values are
decimal JSON strings so browser clients do not lose precision.

Money and `timestamp_day` are raw ETS2 integers. Net is derived as `revenue -
wage - maintenance - fuel`; no currency or wall-clock semantics are assumed.
Hired drivers currently use observed `hometown != ""` heuristic, not a game
format guarantee. Fingerprint v1 is SHA-256 over exactly: raw driver ID,
`timestamp_day`, revenue, wage, maintenance, fuel, distance,
`distance_on_job`, cargo count, cargo, source city/company, destination
city/company. Encoding is `truck-ledger.trip.v1\0`, then strings as unsigned
64-bit big-endian byte length plus UTF-8 bytes, signed integers as big-endian
two's-complement `i64`, and boolean as one byte (`0`/`1`), in that field order.
Profile, driver snapshot fields, `_nameless.*`, paths, and observation metadata
are excluded. SQLite deduplicates `(profile, fingerprint version, hash)`.

## Planned experience

The v0.1 product is planned as one `truck-ledger` executable. It will read
ETS2 saves without modifying them, retain hired-driver history in local SQLite
storage, discover profiles and changed saves, and serve a browser dashboard on
loopback only. Everything stays local: no cloud backend, accounts, remote sync,
or telemetry.

macOS is first tested platform. Dashboard remains Phase 5 work.

See [ROADMAP.md](ROADMAP.md) for phased scope and acceptance criteria.
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for decoder attribution.

## Development status

```sh
cargo run -- --help
cargo run -- --version
```

`reference/` contains sanitized fixtures, prototype code, and upstream notes
from early validation. It is evidence and test material, not production
architecture. Do not port its Python or shell implementation line-for-line;
production behavior must be validated by Rust code and tests as phases are
implemented.

## License

TruckLedger is licensed under [MIT](LICENSE). Files under `reference/` retain
their existing provenance and attribution; this license does not change any
separate upstream terms.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for local development and reference
fixture verification.
