# Releasing TruckLedger

Root `Cargo.toml` package version is release intent. New version merged to `main` publishes once; unchanged version produces no release.

## Normal flow

One normal feature or fix PR may contain implementation and version bump:

```sh
git checkout -b fix/example
# implement and test
./scripts/set-version.sh 0.1.0-alpha.2
git add ...
git commit ...
git push
```

Open and merge PR. Successful `CI` push run on `main` starts release workflow. Workflow builds native macOS packages, validates them, creates annotated `v<version>` tag, then publishes GitHub Release with `.pkg` and `.sha256` assets. No `release/*` branch, manual `git tag`, or manual tag push required.

Do not bump version when change should not release. Batch later only by merging small version-bump PR.

## Immutable release state

Published tags and releases never move or get reused. Existing GitHub Release for Cargo version is clean no-op. Existing tag without release is recoverable: workflow verifies tagged commit declares exact version and is reachable from `main`, then rebuilds/publishes from tagged immutable commit. Mismatch fails; tag is never moved or recreated.

Manual `workflow_dispatch` recovery is main-only and uses dispatch SHA. It follows same state rules.

## macOS policy

Packages target macOS 13 Ventura+, native arm64 and x86_64. They remain unsigned and non-notarized intentionally. No Apple credentials are used.
