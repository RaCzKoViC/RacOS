#!/usr/bin/env bash
# RacOS Boot Test Script
#
# Builds the kernel, boots in QEMU, captures serial output,
# and validates that key boot messages appear.
#
# Usage: ./scripts/boot-test.sh
# Exit: 0 = pass, 1 = fail

set -euo pipefail

TARGET="x86_64-unknown-none"
TIMEOUT=10
LOG_FILE="boot-test.log"

echo "=== RacOS Boot Test ==="

# Build kernel
echo "[1/3] Building kernel..."
cargo build --package racore --target "$TARGET" 2>&1

KERNEL="target/$TARGET/debug/racore"
if [ ! -f "$KERNEL" ]; then
    echo "FAIL: Kernel binary not found at $KERNEL"
    exit 1
fi

# Boot in QEMU with serial output
echo "[2/3] Booting in QEMU (timeout: ${TIMEOUT}s)..."
timeout "$TIMEOUT" qemu-system-x86_64 \
    -machine q35 \
    -cpu qemu64 \
    -m 256M \
    -serial stdio \
    -display none \
    -no-reboot \
    -kernel "$KERNEL" \
    2>&1 | tee "$LOG_FILE" || true

# Validate output
echo "[3/3] Validating boot output..."
PASS=true

check_line() {
    if grep -q "$1" "$LOG_FILE"; then
        echo "  OK: Found '$1'"
    else
        echo "  FAIL: Missing '$1'"
        PASS=false
    fi
}

check_line "RACORE: RacOS kernel starting"
check_line "RACORE: Build"
check_line "RACORE: Boot info validated"
check_line "RACORE: Memory detected"
check_line "RACORE: GDT loaded"
check_line "RACORE: IDT loaded"
check_line "RACORE: Entering idle loop"

# Check for panics
if grep -q "KERNEL PANIC" "$LOG_FILE"; then
    echo "  FAIL: Kernel panic detected!"
    PASS=false
fi

echo ""
if $PASS; then
    echo "=== BOOT TEST PASSED ==="
    exit 0
else
    echo "=== BOOT TEST FAILED ==="
    echo "Full log:"
    cat "$LOG_FILE"
    exit 1
fi
