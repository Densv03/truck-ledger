#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() { printf 'usage: %s <version|vversion>\n' "$0" >&2; exit 1; }
[[ "$#" -eq 1 ]] || usage
version="${1#v}"
[[ -n "$version" ]] || usage
version_pattern='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
[[ "$version" =~ $version_pattern ]] || { printf 'error: invalid Cargo version: %s\n' "$1" >&2; exit 1; }
cd "$REPOSITORY_ROOT"
package_version() {
  awk '
    /^\[package\][[:space:]]*$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^[[:space:]]*version[[:space:]]*=/ {
      value = $0
      sub(/^[^=]*=[[:space:]]*"/, "", value)
      sub(/"[[:space:]]*(#.*)?$/, "", value)
      print value
      exit
    }
  ' "$1"
}
current="$(package_version Cargo.toml)"
[[ -n "$current" ]] || { echo 'error: root Cargo.toml package version missing' >&2; exit 1; }
if [[ "$current" != "$version" ]]; then
  temporary="$(mktemp "${TMPDIR:-/tmp}/truck-ledger-cargo.XXXXXX")"
  trap 'rm -f "$temporary"' EXIT
  awk -v target="$version" '
    /^\[package\][[:space:]]*$/ { in_package = 1; print; next }
    /^\[/ { in_package = 0 }
    in_package && !updated && /^[[:space:]]*version[[:space:]]*=/ { sub(/"[^"]*"/, "\"" target "\""); updated = 1 }
    { print }
    END { if (!updated) exit 1 }
  ' Cargo.toml > "$temporary" || { echo 'error: cannot update root package version' >&2; exit 1; }
  mv "$temporary" Cargo.toml
  trap - EXIT
fi
cargo update -p truck-ledger --offline
toml_version="$(package_version Cargo.toml)"
lock_version="$(awk '
  /^\[\[package\]\][[:space:]]*$/ { in_package = 0 }
  /^name = "truck-ledger"$/ { in_package = 1; next }
  in_package && /^version = / { value = $0; sub(/^version = "/, "", value); sub(/"$/, "", value); print value; exit }
' Cargo.lock)"
[[ "$toml_version" == "$version" ]] || { echo 'error: Cargo.toml version verification failed' >&2; exit 1; }
[[ "$lock_version" == "$version" ]] || { echo 'error: Cargo.lock root package version verification failed' >&2; exit 1; }
printf '%s\n' "$version"
