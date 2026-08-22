# TruckLedger

TruckLedger is local analytics software for hired drivers in Euro Truck
Simulator 2 (ETS2). ETS2 retains only a short rolling window of hired-driver
profit history in save files; TruckLedger is intended to collect that history
locally before it disappears.

TruckLedger is pre-v0.1 and under development. Current Rust code provides only
the executable foundation and CLI metadata; it does not yet decode saves,
persist data, monitor saves, or provide a dashboard.

## Planned experience

The v0.1 product is planned as one `truck-ledger` executable. It will read
ETS2 saves without modifying them, retain hired-driver history in local SQLite
storage, discover profiles and changed saves, and serve a browser dashboard on
loopback only. Everything stays local: no cloud backend, accounts, remote sync,
or telemetry.

macOS is first tested platform. Code should avoid unnecessary platform
coupling so Windows and Linux support can follow.

See [ROADMAP.md](ROADMAP.md) for phased scope and acceptance criteria.

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
