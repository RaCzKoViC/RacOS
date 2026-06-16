#!/usr/bin/env bash
# RacOS — advisory lint for ARCHITECTURE.md §3.3 unsafe-block policy.
#
# Walks every `unsafe {` in kernel/ and libs/ and flags blocks that
# don't have a `// SAFETY:` comment in the preceding 5 lines. Output:
#
#     <file>:<line> unsafe block missing SAFETY comment
#
# Exit code is *always* 0 — this is informational, not gating. RacOS
# is still working through a ~400-block backlog (see ROADMAP.md T4.2)
# and we don't want to block CI while we chip away at it. Use the
# `--strict` flag to flip the exit code so a future CI gate can pick
# this up after the backlog clears.
#
# Usage:
#   bash scripts/check-unsafe-safety.sh
#   bash scripts/check-unsafe-safety.sh --strict      # exit 1 if any missing

set -uo pipefail

STRICT=0
for arg in "$@"; do
    case "$arg" in
        --strict) STRICT=1 ;;
        --help|-h)
            sed -n '2,17p' "$0"
            exit 0
            ;;
        *)
            echo "unknown flag: $arg" >&2
            echo "try --help" >&2
            exit 2
            ;;
    esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Window: how many lines above an `unsafe {` to search for `// SAFETY:`.
WINDOW=5

MISSING=0
TOTAL=0

# Iterate over every unsafe-block opener under kernel/ and libs/.
while IFS= read -r hit; do
    TOTAL=$((TOTAL + 1))
    file="${hit%%:*}"
    rest="${hit#*:}"
    line="${rest%%:*}"
    # Compute the window: max(1, line-WINDOW) .. line-1.
    start=$((line - WINDOW))
    if [ "$start" -lt 1 ]; then start=1; fi
    end=$((line - 1))
    if [ "$end" -lt "$start" ]; then continue; fi

    if ! sed -n "${start},${end}p" "$file" | grep -q "// SAFETY:"; then
        echo "$file:$line unsafe block missing SAFETY comment"
        MISSING=$((MISSING + 1))
    fi
done < <(grep -rn "unsafe {" kernel/src libs/libc-lite/src)

echo ""
echo "scanned $TOTAL unsafe blocks under kernel/ + libs/; $MISSING missing SAFETY"

if [ "$STRICT" -eq 1 ] && [ "$MISSING" -gt 0 ]; then
    echo "strict mode: failing because $MISSING blocks are missing SAFETY annotations"
    exit 1
fi
exit 0
