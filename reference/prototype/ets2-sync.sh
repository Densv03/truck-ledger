#!/bin/zsh

#
# Historical TruckLedger prototype.
#
# REFERENCE ONLY.
#
# Usage:
#
#   SII_DECODE_BIN=/path/to/sii-decode \
#   LEDGER_SCRIPT=/path/to/ets2_driver_ledger.py \
#   ./ets2-sync.sh /path/to/game.sii
#

set -e

SAVE="${1:-}"

if [[ -z "$SAVE" ]]; then
    echo "usage: $0 /path/to/game.sii" >&2
    exit 2
fi

if [[ ! -f "$SAVE" ]]; then
    echo "save does not exist: $SAVE" >&2
    exit 2
fi

SII_DECODE_BIN="${SII_DECODE_BIN:-sii-decode}"

HERE="$(cd "$(dirname "$0")" && pwd)"

LEDGER_SCRIPT="${LEDGER_SCRIPT:-$HERE/ets2_driver_ledger.py}"

TMP="$(mktemp /tmp/truck-ledger-decoded.XXXXXX.sii)"

cleanup() {
    rm -f "$TMP"
}

trap cleanup EXIT

echo "Syncing: $SAVE"

"$SII_DECODE_BIN" "$SAVE" > "$TMP"

python3 "$LEDGER_SCRIPT" ingest "$TMP"
