# Vendored source provenance

Source: `https://github.com/fangyi-zhou/sii-decode-rs`

Upstream commit: `f65cc2f68401e74cfb7c1aae497d196c79d051a7`

License: MIT. Original `LICENSE` retained unchanged.

TruckLedger delta:

- add BSII value type `0x17` (`vec4s`) parsing as four little-endian IEEE-754
  single-precision values;
- add focused parser tests for valid and truncated `0x17` data.
- remove upstream workspace declaration because TruckLedger vendors only library
  crate; root `Cargo.lock` remains dependency lock authority.

Do not merge unrelated upstream changes into this directory without recording
new source commit, license review, and TruckLedger-specific delta here.
