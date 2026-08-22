#!/bin/zsh

#
# Historical macOS TruckLedger watcher prototype.
#
# REFERENCE ONLY.
#
# This demonstrates known-working behavior.
# It is NOT the desired production architecture.
#
# Production code should prefer filesystem notifications.
#

set -u

SAVE_ROOT="${SAVE_ROOT:-}"

if [[ -z "$SAVE_ROOT" ]]; then
    echo "SAVE_ROOT must point to an ETS2 profile save directory" >&2
    exit 2
fi

HERE="$(cd "$(dirname "$0")" && pwd)"

SYNC_SCRIPT="${SYNC_SCRIPT:-$HERE/ets2-sync.sh}"

if [[ ! -d "$SAVE_ROOT" ]]; then
    echo "Save directory does not exist: $SAVE_ROOT" >&2
    exit 2
fi

echo "ETS2 prototype watcher started"

latest_save_signature() {
    find "$SAVE_ROOT" \
        -type f \
        -name game.sii \
        -print0 \
        2>/dev/null |
    xargs -0 stat -f '%m|%N' \
        2>/dev/null |
    sort -t'|' -k1,1nr |
    head -n 1
}

last_signature="$(latest_save_signature)"

while true; do
    latest="$(latest_save_signature)"

    if [[ -n "$latest" && "$latest" != "$last_signature" ]]; then
        save="${latest#*|}"

        echo "Save changed: ${save#$SAVE_ROOT/}"

        # ETS2 may still be finishing the write.
        sleep 1

        if "$SYNC_SCRIPT" "$save"; then
            last_signature="$latest"
        else
            echo "Sync failed; waiting for another change" >&2
        fi
    fi

    sleep 2
done
