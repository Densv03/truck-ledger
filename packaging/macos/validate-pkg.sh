#!/usr/bin/env bash
# Inspect a flat package without installing it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

usage() {
  printf 'usage: %s --pkg <path> [--min-macos <version>]\n' "$0" >&2
  exit 1
}

pkg=""
minimum_macos=""
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --pkg) [[ -n "${2:-}" ]] || usage; pkg="$2"; shift 2 ;;
    --min-macos) [[ -n "${2:-}" ]] || usage; minimum_macos="$2"; shift 2 ;;
    *) usage ;;
  esac
done
[[ -n "$pkg" && -f "$pkg" ]] || fail "package must exist as a regular file: $pkg"
require_macos
require_tool pkgutil
require_tool bsdtar
require_tool lsbom
require_tool file
require_tool lipo

version="$(cargo_version)"
expanded="$(mktemp -d "${TMPDIR:-/tmp}/truck-ledger-validate.XXXXXX")"
rm -rf "$expanded"
payload_root="$(mktemp -d "${TMPDIR:-/tmp}/truck-ledger-payload.XXXXXX")"
cleanup() {
  rm -rf "$expanded" "$payload_root"
}
trap cleanup EXIT

pkgutil --expand-full "$pkg" "$expanded"
components=()
while IFS= read -r component_info; do
  components+=("$component_info")
done < <(find "$expanded" -name PackageInfo -type f -print)
[[ "${#components[@]}" -eq 1 ]] || fail "package must contain exactly one component"
component="$(dirname "${components[0]}")"
metadata="$(cat "$component/PackageInfo")"
printf '%s\n' "$metadata" | grep -Fq "identifier=\"$PACKAGE_IDENTIFIER\"" || fail "package identifier is not $PACKAGE_IDENTIFIER"
printf '%s\n' "$metadata" | grep -Fq "version=\"$version\"" || fail "package version does not match Cargo version $version"
[[ ! -e "$component/Scripts" ]] || fail "package must not contain installer scripts"

[[ -e "$component/Payload" ]] || fail "package payload missing"
if [[ -d "$component/Payload" ]]; then
  payload_root="$component/Payload"
else
  bsdtar -xf "$component/Payload" -C "$payload_root"
fi
payload_files=()
while IFS= read -r payload_file; do
  payload_files+=("$payload_file")
done < <(cd "$payload_root" && find . \( -type f -o -type l \) -print | sort)
[[ "${#payload_files[@]}" -eq 1 && "${payload_files[0]}" == "./usr/local/bin/truck-ledger" ]] || fail "package payload must contain only /usr/local/bin/truck-ledger"
binary="$payload_root/usr/local/bin/truck-ledger"
[[ -x "$binary" ]] || fail "packaged executable lacks execute permission"
architecture="$(binary_architecture "$binary")"
binary_version="$($binary --version)"
[[ "$binary_version" == "truck-ledger $version" ]] || fail "packaged binary version mismatch: $binary_version"

bom="$(lsbom "$component/Bom")"
printf '%s\n' "$bom" | grep -Fq './usr/local/bin/truck-ledger' || fail "package BOM lacks /usr/local/bin/truck-ledger"
if printf '%s\n' "$bom" | grep -Eq '(Library/LaunchAgents|\.zshrc|\.bashrc|truck-ledger\.sqlite|\.sqlite3|collector\.(stdout|stderr))'; then
  fail "package BOM contains prohibited application data or shell/service path"
fi

signature_output="$(pkgutil --check-signature "$pkg" 2>&1 || true)"
if [[ "$signature_output" == *"Status: signed"* ]]; then
  pkgutil --check-signature "$pkg" >/dev/null
elif [[ "$signature_output" == *"Status: no signature"* || "$signature_output" == *"Status: unsigned"* ]]; then
  :
else
  printf '%s\n' "$signature_output" >&2
  fail "package signature inspection failed"
fi

if [[ -n "$minimum_macos" ]]; then
  require_tool vtool
  built_minimum="$(vtool -show-build -arch "$architecture" "$binary" | awk '/minos/ { print $2; exit }')"
  [[ -n "$built_minimum" ]] || fail "cannot inspect Mach-O minimum macOS version"
  awk -v built="$built_minimum" -v required="$minimum_macos" 'BEGIN { exit !(built <= required) }' || fail "binary requires macOS $built_minimum, newer than supported $minimum_macos"
fi

printf 'validated: %s (%s, version %s)\n' "$pkg" "$architecture" "$version"
