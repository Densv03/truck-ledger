# TruckLedger

TruckLedger is local analytics software for hired drivers in Euro Truck
Simulator 2 (ETS2). ETS2 retains only a short rolling window of hired-driver
profit history in save files; TruckLedger is intended to collect that history
locally before it disappears.

TruckLedger is pre-v0.1 and under development. Phase 1 imports one explicitly
supplied ETS2 `ScsC` save or decoded `SiiNunit` file into local SQLite:

```sh
truck-ledger ingest --profile fixture-profile --input game.sii
truck-ledger ingest --profile fixture-profile --input decoded.sii --database /tmp/ledger.sqlite3
```

Profiles and input paths are explicit. Automatic ETS2/Steam discovery and
watchers are Phase 2. Default data is stored in platform local application
data under `truck-ledger/truck-ledger.sqlite3`.

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

macOS is first tested platform. Code should avoid unnecessary platform
coupling so Windows and Linux support can follow.

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
