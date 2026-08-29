#!/usr/bin/env bash
# RacOS — advisory lint for ARCHITECTURE.md §3.3 unsafe-block policy.
#
# Walks every `unsafe {` in kernel/ and libs/ and flags blocks that
# don't have a `// SAFETY:` comment in the preceding 5 lines. Output:
#
#     <file>:<line> unsafe block missing SAFETY comment
#
# Without `--strict` the exit code is 0 for missing annotations: that was
# the mode used while RacOS worked through its ~400-block backlog (ROADMAP
# T4.2). The backlog cleared, so CI now runs this `--strict` as a required
# gate and a missing annotation exits 1.
#
# Two conditions exit 2 whatever the mode, because both mean the scan did
# not happen and reporting a clean tree would be a lie: a missing tool, and
# a scan that matched no unsafe blocks at all.
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

# The tools this needs. Checked up front because the failure mode otherwise is
# silent and backwards: run under a bash whose PATH lacks coreutils (Git for
# Windows' bare bash.exe does exactly this) and `grep` is not found, the scan
# finds nothing, and the script reports "scanned 0 blocks; 0 missing" and exits
# 0 -- a gate enforcing SAFETY annotations passing without reading a line.
for tool in dirname grep sed; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "check-unsafe-safety: required tool '$tool' not found in PATH" >&2
        echo "  (on Windows use Git Bash's own environment, e.g. C:\\Git\\bin\\bash.exe)" >&2
        exit 2
    fi
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

# Finding nothing is not a pass. The kernel has hundreds of unsafe blocks, so
# zero means the scan did not happen -- wrong directory, a moved tree, a grep
# that silently produced nothing. Reporting that as clean is the one outcome
# this lint must never produce.
if [ "$TOTAL" -eq 0 ]; then
    echo "no unsafe blocks found at all: the scan did not run, not a clean tree" >&2
    exit 2
fi

if [ "$STRICT" -eq 1 ] && [ "$MISSING" -gt 0 ]; then
    echo "strict mode: failing because $MISSING blocks are missing SAFETY annotations"
    exit 1
fi
exit 0
