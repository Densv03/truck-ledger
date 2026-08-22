#!/usr/bin/env python3

"""
Historical TruckLedger prototype.

REFERENCE ONLY.

This script expects an already decoded textual ETS2 game.sii.

It demonstrates the behavior validated during prototype development:

decoded game.sii
-> driver_ai
-> profit_log
-> profit_log_entry
-> deterministic trip fingerprint
-> persistent SQLite

Production TruckLedger should reimplement this behavior cleanly in Rust.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sqlite3
import sys

from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional


BLOCK_RE = re.compile(
    r"(?ms)^([A-Za-z0-9_]+)\s*:\s*([^\s{]+)\s*\{\s*\n(.*?)^\}"
)

FIELD_RE = re.compile(
    r"^\s*([A-Za-z0-9_\[\]]+)\s*:\s*(.*?)\s*$"
)


def parse_scalar(raw: str):
    raw = raw.strip()

    if raw.startswith('"') and raw.endswith('"'):
        return raw[1:-1]

    if raw == "true":
        return True

    if raw == "false":
        return False

    if raw in {"null", "nil"}:
        return None

    try:
        return int(raw)
    except ValueError:
        pass

    try:
        return float(raw)
    except ValueError:
        return raw


def parse_fields(body: str) -> Dict[str, object]:
    result: Dict[str, object] = {}

    for line in body.splitlines():
        match = FIELD_RE.match(line)

        if not match:
            continue

        key, raw = match.groups()
        result[key] = parse_scalar(raw)

    return result


@dataclass
class ProfitEntry:
    ref: str
    timestamp_day: int

    revenue: int
    wage: int
    maintenance: int
    fuel: int

    distance: int
    distance_on_job: bool

    cargo_count: int
    cargo: str

    source_city: str
    source_company: str

    destination_city: str
    destination_company: str

    @property
    def net(self) -> int:
        return (
            self.revenue
            - self.wage
            - self.maintenance
            - self.fuel
        )


@dataclass
class Driver:
    driver_id: str

    hometown: str
    current_city: str
    experience_points: int

    adr: object
    long_dist: object
    heavy: object
    fragile: object
    urgent: object
    mechanical: object

    entries: List[ProfitEntry]

    @property
    def is_hired(self) -> bool:
        # This heuristic was observed in real ETS2 saves.
        #
        # Hired drivers had a non-empty hometown.
        # Recruitment-pool driver_ai entries had an empty hometown.
        #
        # This is NOT claimed to be a guaranteed game-format contract.
        return bool(self.hometown)


def load_sii(path: Path) -> List[Driver]:
    text = path.read_text(
        encoding="utf-8",
        errors="replace",
    )

    drivers_raw: Dict[str, Dict[str, object]] = {}
    profit_logs: Dict[str, Dict[str, object]] = {}
    entries: Dict[str, ProfitEntry] = {}

    for block_type, block_id, body in BLOCK_RE.findall(text):
        fields = parse_fields(body)

        if block_type == "driver_ai":
            drivers_raw[block_id] = fields

        elif block_type == "profit_log":
            profit_logs[block_id] = fields

        elif block_type == "profit_log_entry":
            entries[block_id] = ProfitEntry(
                ref=block_id,
                timestamp_day=int(
                    fields.get("timestamp_day", 0) or 0
                ),
                revenue=int(
                    fields.get("revenue", 0) or 0
                ),
                wage=int(
                    fields.get("wage", 0) or 0
                ),
                maintenance=int(
                    fields.get("maintenance", 0) or 0
                ),
                fuel=int(
                    fields.get("fuel", 0) or 0
                ),
                distance=int(
                    fields.get("distance", 0) or 0
                ),
                distance_on_job=bool(
                    fields.get("distance_on_job", False)
                ),
                cargo_count=int(
                    fields.get("cargo_count", 0) or 0
                ),
                cargo=str(
                    fields.get("cargo", "") or ""
                ),
                source_city=str(
                    fields.get("source_city", "") or ""
                ),
                source_company=str(
                    fields.get("source_company", "") or ""
                ),
                destination_city=str(
                    fields.get("destination_city", "") or ""
                ),
                destination_company=str(
                    fields.get("destination_company", "") or ""
                ),
            )

    drivers: List[Driver] = []

    for driver_id, fields in drivers_raw.items():
        log_ref = fields.get("profit_log")

        log_entries: List[ProfitEntry] = []

        if (
            isinstance(log_ref, str)
            and log_ref in profit_logs
        ):
            profit_log = profit_logs[log_ref]

            refs = []

            for key, value in profit_log.items():
                match = re.fullmatch(
                    r"stats_data\[(\d+)\]",
                    key,
                )

                if match and isinstance(value, str):
                    refs.append(
                        (
                            int(match.group(1)),
                            value,
                        )
                    )

            refs.sort()

            for _, entry_ref in refs:
                if entry_ref in entries:
                    log_entries.append(
                        entries[entry_ref]
                    )

        driver = Driver(
            driver_id=driver_id,
            hometown=str(
                fields.get("hometown", "") or ""
            ),
            current_city=str(
                fields.get("current_city", "") or ""
            ),
            experience_points=int(
                fields.get("experience_points", 0) or 0
            ),
            adr=fields.get("adr"),
            long_dist=fields.get("long_dist"),
            heavy=fields.get("heavy"),
            fragile=fields.get("fragile"),
            urgent=fields.get("urgent"),
            mechanical=fields.get("mechanical"),
            entries=log_entries,
        )

        if driver.is_hired:
            drivers.append(driver)

    return drivers


def open_db(path: Path) -> sqlite3.Connection:
    db = sqlite3.connect(path)

    db.execute("PRAGMA journal_mode=WAL")

    db.execute(
        """
        CREATE TABLE IF NOT EXISTS drivers (
            driver_id TEXT PRIMARY KEY,
            name TEXT,

            hometown TEXT,
            current_city TEXT,

            experience_points INTEGER,

            adr TEXT,
            long_dist TEXT,
            heavy TEXT,
            fragile TEXT,
            urgent TEXT,
            mechanical TEXT,

            last_seen_at TEXT DEFAULT CURRENT_TIMESTAMP
        )
        """
    )

    db.execute(
        """
        CREATE TABLE IF NOT EXISTS trips (
            trip_key TEXT PRIMARY KEY,

            driver_id TEXT NOT NULL,
            timestamp_day INTEGER NOT NULL,

            revenue INTEGER NOT NULL,
            wage INTEGER NOT NULL,
            maintenance INTEGER NOT NULL,
            fuel INTEGER NOT NULL,
            net INTEGER NOT NULL,

            distance INTEGER NOT NULL,
            distance_on_job INTEGER NOT NULL,

            cargo_count INTEGER NOT NULL,
            cargo TEXT,

            source_city TEXT,
            source_company TEXT,

            destination_city TEXT,
            destination_company TEXT,

            first_seen_at TEXT DEFAULT CURRENT_TIMESTAMP,

            FOREIGN KEY(driver_id)
                REFERENCES drivers(driver_id)
        )
        """
    )

    db.commit()

    return db


def trip_key(
    driver_id: str,
    entry: ProfitEntry,
) -> str:
    """
    Build a deterministic content-based trip fingerprint.

    `_nameless.*` object references are intentionally excluded because
    prototype work did not consider them safe durable identifiers across
    different saves.
    """

    raw = "\x1f".join(
        map(
            str,
            [
                driver_id,
                entry.timestamp_day,

                entry.revenue,
                entry.wage,
                entry.maintenance,
                entry.fuel,

                entry.distance,
                int(entry.distance_on_job),

                entry.cargo_count,
                entry.cargo,

                entry.source_city,
                entry.source_company,

                entry.destination_city,
                entry.destination_company,
            ],
        )
    )

    return hashlib.sha256(
        raw.encode("utf-8")
    ).hexdigest()


def ingest(
    db: sqlite3.Connection,
    drivers: List[Driver],
) -> tuple[int, int]:
    new_trips = 0
    seen_trips = 0

    for driver in drivers:
        db.execute(
            """
            INSERT INTO drivers (
                driver_id,

                hometown,
                current_city,

                experience_points,

                adr,
                long_dist,
                heavy,
                fragile,
                urgent,
                mechanical,

                last_seen_at
            )
            VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                CURRENT_TIMESTAMP
            )

            ON CONFLICT(driver_id)
            DO UPDATE SET
                hometown = excluded.hometown,
                current_city = excluded.current_city,

                experience_points =
                    excluded.experience_points,

                adr = excluded.adr,
                long_dist = excluded.long_dist,
                heavy = excluded.heavy,
                fragile = excluded.fragile,
                urgent = excluded.urgent,
                mechanical = excluded.mechanical,

                last_seen_at = CURRENT_TIMESTAMP
            """,
            (
                driver.driver_id,

                driver.hometown,
                driver.current_city,

                driver.experience_points,

                str(driver.adr),
                str(driver.long_dist),
                str(driver.heavy),
                str(driver.fragile),
                str(driver.urgent),
                str(driver.mechanical),
            ),
        )

        for entry in driver.entries:
            seen_trips += 1

            key = trip_key(
                driver.driver_id,
                entry,
            )

            cursor = db.execute(
                """
                INSERT OR IGNORE INTO trips (
                    trip_key,

                    driver_id,
                    timestamp_day,

                    revenue,
                    wage,
                    maintenance,
                    fuel,
                    net,

                    distance,
                    distance_on_job,

                    cargo_count,
                    cargo,

                    source_city,
                    source_company,

                    destination_city,
                    destination_company
                )
                VALUES (
                    ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                    ?, ?, ?, ?, ?, ?
                )
                """,
                (
                    key,

                    driver.driver_id,
                    entry.timestamp_day,

                    entry.revenue,
                    entry.wage,
                    entry.maintenance,
                    entry.fuel,
                    entry.net,

                    entry.distance,
                    int(entry.distance_on_job),

                    entry.cargo_count,
                    entry.cargo,

                    entry.source_city,
                    entry.source_company,

                    entry.destination_city,
                    entry.destination_company,
                ),
            )

            if cursor.rowcount == 1:
                new_trips += 1

    db.commit()

    return seen_trips, new_trips


def fmt_money(value: int) -> str:
    sign = "-" if value < 0 else ""

    return (
        f"{sign}€{abs(value):,}"
        .replace(",", " ")
    )


def report(db: sqlite3.Connection):
    rows = db.execute(
        """
        SELECT
            d.driver_id,
            COALESCE(d.name, ''),
            d.hometown,

            COUNT(t.trip_key) AS trips,

            COALESCE(
                SUM(
                    CASE
                        WHEN t.distance_on_job = 1
                        THEN 1
                        ELSE 0
                    END
                ),
                0
            ) AS loaded,

            COALESCE(
                SUM(
                    CASE
                        WHEN t.distance_on_job = 0
                        THEN 1
                        ELSE 0
                    END
                ),
                0
            ) AS empty,

            COALESCE(
                SUM(t.distance),
                0
            ) AS km,

            COALESCE(
                SUM(t.net),
                0
            ) AS net

        FROM drivers d

        LEFT JOIN trips t
            ON t.driver_id = d.driver_id

        GROUP BY d.driver_id

        ORDER BY
            net DESC,
            d.driver_id
        """
    ).fetchall()

    print()

    print(
        "=== ETS2 DRIVER LEDGER — "
        f"STORED HISTORY ({len(rows)} drivers) ==="
    )

    print()

    header = (
        f"{'driver':<12} "
        f"{'name':<16} "
        f"{'home':<12} "
        f"{'trips':>5} "
        f"{'load':>5} "
        f"{'empty':>5} "
        f"{'km':>8} "
        f"{'net':>12} "
        f"{'€/km':>8}"
    )

    print(header)
    print("-" * len(header))

    for (
        driver_id,
        name,
        home,
        trips,
        loaded,
        empty,
        km,
        net,
    ) in rows:
        eur_km = (
            net / km
            if km
            else 0.0
        )

        print(
            f"{driver_id:<12} "
            f"{name[:16]:<16} "
            f"{home[:12]:<12} "
            f"{trips:>5} "
            f"{loaded:>5} "
            f"{empty:>5} "
            f"{km:>8} "
            f"{fmt_money(net):>12} "
            f"{eur_km:>8.2f}"
        )


def details(
    db: sqlite3.Connection,
    driver_id: str,
):
    driver = db.execute(
        """
        SELECT
            driver_id,
            COALESCE(name, ''),
            hometown,
            current_city,
            experience_points

        FROM drivers

        WHERE driver_id = ?
        """,
        (driver_id,),
    ).fetchone()

    if not driver:
        print(
            f"Unknown driver: {driver_id}",
            file=sys.stderr,
        )

        raise SystemExit(2)

    print()

    print(
        f"=== {driver[0]} "
        f"{('— ' + driver[1]) if driver[1] else ''} "
        "==="
    )

    print(
        f"home={driver[2]} "
        f"current_city={driver[3]} "
        f"xp={driver[4]}"
    )

    trips = db.execute(
        """
        SELECT
            timestamp_day,

            source_city,
            destination_city,

            cargo,

            distance,
            net,

            revenue,
            wage,
            maintenance,
            fuel,

            distance_on_job

        FROM trips

        WHERE driver_id = ?

        ORDER BY
            timestamp_day,
            first_seen_at
        """,
        (driver_id,),
    ).fetchall()

    for row in trips:
        (
            day,
            source,
            destination,
            cargo,
            distance,
            net,
            revenue,
            wage,
            maintenance,
            fuel,
            on_job,
        ) = row

        cargo_label = (
            cargo
            if cargo
            else "<EMPTY>"
        )

        print(
            f"day {day:>4} | "
            f"{source or '?'} -> "
            f"{destination or '?':<18} | "
            f"{cargo_label:<18} | "
            f"{distance:>5} km | "
            f"net {fmt_money(net):>10} "
            f"(rev {fmt_money(revenue)}, "
            f"wage {fmt_money(wage)}, "
            f"maint {fmt_money(maintenance)}, "
            f"fuel {fmt_money(fuel)})"
        )


def set_name(
    db: sqlite3.Connection,
    driver_id: str,
    name: str,
):
    cursor = db.execute(
        """
        UPDATE drivers
        SET name = ?
        WHERE driver_id = ?
        """,
        (
            name,
            driver_id,
        ),
    )

    db.commit()

    if cursor.rowcount == 0:
        print(
            f"Unknown driver: {driver_id}",
            file=sys.stderr,
        )

        raise SystemExit(2)

    print(
        f"{driver_id} -> {name}"
    )


def main():
    parser = argparse.ArgumentParser(
        description=(
            "Historical prototype of persistent "
            "ETS2 hired-driver history."
        )
    )

    parser.add_argument(
        "--db",
        type=Path,
        default=(
            Path.home()
            / ".ets2-driver-ledger.sqlite3"
        ),
    )

    sub = parser.add_subparsers(
        dest="command",
        required=True,
    )

    ingest_parser = sub.add_parser(
        "ingest"
    )

    ingest_parser.add_argument(
        "sii",
        type=Path,
    )

    sub.add_parser(
        "report"
    )

    driver_parser = sub.add_parser(
        "driver"
    )

    driver_parser.add_argument(
        "driver_id"
    )

    name_parser = sub.add_parser(
        "name"
    )

    name_parser.add_argument(
        "driver_id"
    )

    name_parser.add_argument(
        "name"
    )

    args = parser.parse_args()

    db = open_db(args.db)

    if args.command == "ingest":
        if not args.sii.exists():
            print(
                f"File not found: {args.sii}",
                file=sys.stderr,
            )

            raise SystemExit(1)

        drivers = load_sii(args.sii)

        seen, new = ingest(
            db,
            drivers,
        )

        print(
            f"Scanned {len(drivers)} hired drivers "
            f"and {seen} currently visible trip entries; "
            f"added {new} new trips to {args.db}"
        )

        report(db)

    elif args.command == "report":
        report(db)

    elif args.command == "driver":
        details(
            db,
            args.driver_id,
        )

    elif args.command == "name":
        set_name(
            db,
            args.driver_id,
            args.name,
        )


if __name__ == "__main__":
    main()
