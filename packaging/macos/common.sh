#!/usr/bin/env bash
# Shared macOS package helpers. Source from packaging scripts; do not run directly.

set -euo pipefail

PACKAGE_IDENTIFIER="com.truckledger.cli"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || fail "required macOS tool unavailable: $1"
}

require_macos() {
  [[ "$(uname -s)" == "Darwin" ]] || fail "macOS packaging requires macOS"
}

cargo_version() {
  require_tool cargo
  local package_id
  package_id="$(cargo pkgid --manifest-path "$REPOSITORY_ROOT/Cargo.toml")" || fail "cannot determine Cargo package version"
  [[ "$package_id" == *[@#]* ]] || fail "cannot determine Cargo package version"
  printf '%s\n' "$(printf '%s\n' "$package_id" | sed 's/.*[#@]//')"
}

binary_architecture() {
  local binary="$1"
  [[ -f "$binary" ]] || fail "release binary missing: $binary"
  require_tool lipo
  local architectures
  architectures="$(lipo -archs "$binary" 2>/dev/null)" || fail "cannot determine binary architecture: $binary"
  local -a slices
  read -r -a slices <<<"$architectures"
  [[ "${#slices[@]}" -eq 1 ]] || fail "binary must contain exactly one architecture slice; found: $architectures"
  case "${slices[0]}" in
    arm64) printf 'arm64\n' ;;
    x86_64) printf 'x86_64\n' ;;
    *) fail "unsupported binary architecture: ${slices[0]}" ;;
  esac
}

artifact_name() {
  local version="$1"
  local architecture="$2"
  printf 'truck-ledger-%s-macos-%s.pkg\n' "$version" "$architecture"
}

parse_output_dir() {
  [[ "${1:-}" == "--output-dir" && -n "${2:-}" && "$#" -eq 2 ]] || fail "usage: $0 --output-dir <directory>"
  printf '%s\n' "$2"
}

build_package() {
  local binary="$1"
  local output_dir="$2"
  local signing_identity="${3:-}"
  require_macos
  require_tool pkgbuild
  require_tool xattr
  local version architecture stage artifact
  version="$(cargo_version)"
  architecture="$(binary_architecture "$binary")"
  mkdir -p "$output_dir"
  stage="$(mktemp -d "${TMPDIR:-/tmp}/truck-ledger-pkg.XXXXXX")"
  trap 'rm -rf "$stage"' RETURN
  install -d -m 0755 "$stage/usr/local/bin"
  install -m 0755 "$binary" "$stage/usr/local/bin/truck-ledger"
  xattr -c "$stage/usr/local/bin/truck-ledger"
  artifact="$output_dir/$(artifact_name "$version" "$architecture")"
  local -a args=(--root "$stage" --identifier "$PACKAGE_IDENTIFIER" --version "$version" --install-location /)
  if [[ -n "$signing_identity" ]]; then
    args+=(--sign "$signing_identity")
  fi
  pkgbuild "${args[@]}" "$artifact"
  printf '%s\n' "$artifact"
}
