#!/usr/bin/env bash
# Build an unsigned local developer package. Never signs or notarizes.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

output_dir="$(parse_output_dir "$@")"
build_package "$REPOSITORY_ROOT/target/release/truck-ledger" "$output_dir"
