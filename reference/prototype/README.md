# Historical TruckLedger prototype

These files are the cleaned reference version of the prototype used to validate TruckLedger.

They are NOT production code.

The production Rust implementation should use them to understand expected behavior, not copy their architecture blindly.

## Components

### ets2_driver_ledger.py

Reads an already-decoded textual `game.sii`.

It:

- parses `driver_ai`;
- filters hired drivers using non-empty `hometown`;
- resolves `profit_log`;
- resolves `profit_log_entry`;
- calculates net income;
- stores drivers and trips in SQLite;
- generates content-based SHA-256 trip fingerprints;
- rejects duplicate trip imports;
- provides basic cumulative reports.

### ets2-sync.sh

Prototype glue:

    encoded game.sii
        -> sii-decode
        -> temporary decoded SII
        -> ets2_driver_ledger.py ingest

The temporary decoded file is deleted afterward.

### ets2-watch.sh

macOS-oriented prototype watcher.

It:

- recursively looks for `game.sii`;
- determines the newest save;
- notices when it changes;
- invokes `ets2-sync.sh`.

The historical prototype also checked the `eurotrucks2` process.

This is intentionally NOT the desired production architecture.

Production should preferably use filesystem notifications rather than repeated shell polling.

## Important

Do not copy:

- shell polling loops;
- macOS `stat` syntax;
- process polling;
- Python regex architecture;

unless there is a concrete reason.

Preserve the proven behavioral properties instead:

- read-only saves;
- recursive save-slot support;
- content-based trip deduplication;
- persistent SQLite history;
- graceful repeated imports.
