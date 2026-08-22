# TruckLedger reference material

This directory contains historical prototype code, sanitized fixtures, and observations collected while validating the TruckLedger idea against real Euro Truck Simulator 2 saves.

## Important

Everything in `reference/` is **reference material**, not production architecture.

Production code should:

- treat the current Rust implementation and its tests as the source of truth;
- use these files to understand known-working behavior and save structures;
- verify assumptions instead of blindly reproducing prototype code;
- avoid porting Python or shell implementation details line-for-line.

The prototype proved that the core idea works, but it was intentionally built as a quick local experiment.

---

# Observed ETS2 save behavior

The following observations were verified against real ETS2 Steam Cloud saves during prototype development.

They are observations, not guaranteed or documented ETS2 format contracts.

Production code should validate them and fail gracefully when they do not hold.

## Save encoding

The tested `game.sii` files were encoded and started with:

    ScsC

After decoding with `sii-decode-rs`, the resulting document started with:

    SiiNunit

TruckLedger must never modify the original save file.

---

## Hired drivers

Hired-driver objects were represented as:

    driver_ai : driver.<id> {
        ...
    }

Example:

    driver_ai : driver.116 {
        adr: 3
        long_dist: 3
        heavy: 3
        fragile: 6
        urgent: 3
        mechanical: 3
        hometown: groningen
        current_city: groningen
        experience_points: 93113
        profit_log: _nameless....
    }

Hundreds of additional `driver_ai` objects also existed for recruitment candidates.

In the tested save:

- hired drivers had a non-empty `hometown`;
- recruitment-pool drivers had an empty `hometown`.

The prototype therefore used:

    hired_driver = hometown != ""

This is a heuristic observed in real saves, not a guaranteed format contract.

Production code should isolate this logic behind a clear function and test it.

---

## Driver names

Human-readable hired-driver names were not found inside the observed `driver_ai` blocks.

The prototype therefore used the stable-looking game driver identifier:

    driver.116
    driver.110
    ...

as the internal identifier.

Manual display-name mapping was supported separately.

Do not assume names are unavailable everywhere without checking newer saves.

---

## Profit history

A hired driver referenced a profit log:

    driver_ai
        -> profit_log

The profit log referenced individual trip entries through:

    stats_data[n]

Example:

    profit_log : _nameless.... {
        stats_data: 3
        stats_data[0]: _nameless.trip1
        stats_data[1]: _nameless.trip2
        stats_data[2]: _nameless.trip3
        history_age: 7
    }

Each referenced object was:

    profit_log_entry : _nameless.... {
        ...
    }

Observed trip fields included:

- revenue
- wage
- maintenance
- fuel
- distance
- distance_on_job
- cargo_count
- cargo
- source_city
- source_company
- destination_city
- destination_company
- timestamp_day

---

## Rolling history

The tested save contained:

    history_age: 7

and only a short recent list of `profit_log_entry` objects.

Older entries disappeared from newer saves.

This means TruckLedger cannot reconstruct unlimited historical data from a single current save.

It must continuously archive entries before ETS2 removes them.

This behavior was the primary motivation for TruckLedger.

---

## Net profit

The prototype calculated trip net income as:

    net = revenue - wage - maintenance - fuel

This matched the values observed in the ETS2 UI for tested hired-driver jobs.

---

## Empty trips

Observed empty repositioning jobs typically looked like:

    revenue: 0
    wage: 0
    maintenance: 0
    fuel: <positive>
    distance_on_job: false
    cargo_count: 0
    cargo: ""

These naturally produce negative net values because fuel is still consumed.

---

## ADR

Observed ADR values included values such as:

    3
    15
    63

Therefore ADR must NOT automatically be interpreted as a simple skill level.

The prototype stores the raw value.

Production code should verify whether it is a bitmask before exposing individual ADR categories.

---

## Internal `_nameless.*` references

Objects such as:

    _nameless.6000.0311.e920

are useful for resolving references inside one decoded save.

They must NOT be assumed to be durable identifiers across different saves.

The prototype therefore did not use `_nameless.*` as the persistent trip primary key.

---

## Trip deduplication

The working prototype generated a deterministic fingerprint from:

- driver_id
- timestamp_day
- revenue
- wage
- maintenance
- fuel
- distance
- distance_on_job
- cargo_count
- cargo
- source_city
- source_company
- destination_city
- destination_company

The fingerprint was hashed with SHA-256.

This behavior was validated experimentally:

Initial import:

    64 currently visible trip entries
    64 new trips

Second import of the exact same save:

    64 currently visible trip entries
    0 new trips

Later a newer quicksave contained only 58 currently visible entries but produced:

    12 new trips

The database then contained:

    76 trips

This demonstrated that historical trips remained stored even after ETS2 removed them from its own rolling history.

---

## Save slots

It is not sufficient to watch only:

    save/autosave/game.sii

Observed save slots included:

- quicksave
- autosave
- autosave_drive
- autosave_drive_1 ... autosave_drive_9
- autosave_job
- autosave_job_1 ... autosave_job_9
- multiplayer_backup variants

A user-bound quicksave key updated:

    save/quicksave/game.sii

Production collection should therefore monitor the save tree rather than one hard-coded slot.

---

## Save rollback caveat

Loading an older ETS2 save rewinds the game's state but does not rewind TruckLedger's external SQLite database.

The prototype intentionally did not solve branching/timeline semantics.

Short rollbacks may therefore leave historical trips in TruckLedger that were later erased from the game's timeline.

Rollback handling is explicitly out of scope for the initial v0.1 unless the roadmap says otherwise.

---

# Prototype validation

The prototype was tested with:

- 21 hired drivers;
- 64 initially visible trip records;
- subsequent save changes adding additional trips;
- deterministic duplicate rejection;
- persistent SQLite storage;
- manual and automatic quicksave ingestion.

The prototype successfully demonstrated:

    save -> decode -> parse -> deduplicate -> SQLite

The production Rust implementation should preserve these behavioral properties while replacing the prototype architecture.
