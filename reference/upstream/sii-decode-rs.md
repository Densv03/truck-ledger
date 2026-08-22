# sii-decode-rs upstream reference

Repository:

https://github.com/fangyi-zhou/sii-decode-rs

TruckLedger prototype development used this project to decode encoded SCS save files.

The prototype successfully built it with Rust and decoded an ETS2 `ScsC` `game.sii` into textual `SiiNunit`.

During prototype work the standalone binary was built using:

    cargo build --release

and appeared as:

    target/release/sii-decode

Example prototype usage:

    sii-decode /path/to/game.sii > decoded.sii

## Production integration

TruckLedger should preferably consume the decoder as a Rust library rather than requiring users to install a separate executable.

However:

DO NOT treat this document as authoritative for the current upstream API.

Before implementing decoder integration, the agent must inspect the current upstream repository and verify:

- current crate/package name;
- current public Rust API;
- supported file formats;
- current license;
- latest suitable revision;
- whether library integration is officially supported.

If using a Git dependency, pin it to an explicit commit revision rather than a floating branch.

Do not vendor or copy upstream source code without a justified reason.

The production TruckLedger binary should ideally contain everything required for decoding so end users do not need:

- Rust;
- Cargo;
- Python;
- a separately installed `sii-decode` executable.

The prototype investigation indicated a permissive license, but the production implementation MUST independently verify the current upstream license before release.
