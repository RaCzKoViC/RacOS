#!/usr/bin/env bash
# RacOS — CI smoke runner (bash counterpart of run-ci-smoke.ps1).
#
# Boots a ci-smoke-enabled kernel under QEMU with isa-debug-exit wired up
# and reports the QEMU exit code. Success is exit 33 (kernel writes 0x10
# to port 0xf4, QEMU maps that to (0x10 << 1) | 1). Failure is 35; 124
# means the kernel didn't reach the exit gate before the timeout.
#
# This script wraps the same path that .github/workflows/ci.yml's
# kernel-smoke-isadbg job runs inline. Use it locally on Linux/macOS to
# reproduce a CI smoke result without pushing to a branch.
#
# Usage:
#   bash scripts/run-ci-smoke.sh                 # bare ci-smoke
#   bash scripts/run-ci-smoke.sh --disk          # attach a 16M AHCI disk
#   TIMEOUT_SEC=120 bash scripts/run-ci-smoke.sh # extend the QEMU budget
#
# Requires: cargo (nightly via rust-toolchain.toml), qemu-system-x86_64,
# OVMF firmware (any of OVMF_CODE.fd, OVMF_CODE_4M.fd, qemu/OVMF.fd).

set -euo pipefail

TIMEOUT_SEC="${TIMEOUT_SEC:-60}"
SMP="${SMP:-1}"
DISK=0
for arg in "$@"; do
    case "$arg" in
        --disk) DISK=1 ;;
        --help|-h)
            sed -n '2,17p' "$0"
            exit 0
            ;;
        *)
            echo "unknown arg: $arg" >&2
            echo "try --help" >&2
            exit 2
            ;;
    esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Pick whichever OVMF the runner ships. Mirrors the search order used by
# the CI boot-smoke job so behaviour matches across Ubuntu releases.
OVMF=""
for cand in \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/qemu/OVMF.fd \
    tools/OVMF_CODE.fd; do
    if [ -f "$cand" ]; then OVMF="$cand"; break; fi
done
if [ -z "$OVMF" ]; then
    echo "FAIL: no OVMF firmware found. apt-get install ovmf, or drop OVMF_CODE.fd into tools/." >&2
    exit 1
fi
echo "Using OVMF=$OVMF"

LOG_PATH="$ROOT_DIR/smoke-stdout.log"
rm -f "$LOG_PATH"

# Rebuild the kernel with ci-smoke + static relocation. Without
# -C relocation-model=static the kernel ELF is PIE with dynamic relocations
# the bootloader doesn't apply, vtable calls go to garbage low memory, and
# the guest #UDs before kernel_main ever prints a line.
echo "Building kernel (ci-smoke + static relocation)..."
RUSTFLAGS="-C relocation-model=static -C link-arg=-no-pie" \
    cargo build --package racore --target x86_64-unknown-none --features ci-smoke

# Stage the freshly-built kernel into esp/ so the bootloader picks it up.
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-none/debug/racore esp/racore.elf
echo "Staged kernel: target/x86_64-unknown-none/debug/racore -> esp/racore.elf"

QEMU_ARGS=(
    -machine q35
    -accel tcg
    -cpu qemu64,+smep,+smap
    -smp "$SMP"
    -m 512M
    -drive "if=pflash,format=raw,readonly=on,file=$OVMF"
    -boot menu=on
    -drive file=fat:rw:esp,format=raw
    -serial "file:$LOG_PATH"
    -monitor null
    -display none
    -no-reboot
    -device isa-debug-exit,iobase=0xf4,iosize=0x04
)

if [ "$DISK" -eq 1 ]; then
    DISK_PATH="$ROOT_DIR/racos-smoke-disk.img"
    rm -f "$DISK_PATH"
    truncate -s 16M "$DISK_PATH"
    QEMU_ARGS+=(
        -drive "id=disk0,file=$DISK_PATH,if=none,format=raw,cache=writethrough"
        -device "ich9-ahci,id=ahci"
        -device "ide-hd,drive=disk0,bus=ahci.0"
    )
    echo "Attached smoke disk: $DISK_PATH"
fi

echo "Launching QEMU (ci-smoke, ${TIMEOUT_SEC}s budget)..."

# isa-debug-exit returns the encoded exit code via QEMU's own exit status.
# `timeout` returns 124 on its own kill. Disable -e around the call so we
# can capture both.
set +e
timeout "$TIMEOUT_SEC" qemu-system-x86_64 "${QEMU_ARGS[@]}"
EXIT_CODE=$?
set -e

if [ "$EXIT_CODE" -eq 124 ]; then
    echo "TIMEOUT after ${TIMEOUT_SEC}s"
fi

if [ -f "$LOG_PATH" ]; then
    echo "--- serial log (tail 60) ---"
    tail -n 60 "$LOG_PATH" || true
fi

# isa-debug-exit codes:
#   success = kernel writes 0x10 → (0x10 << 1) | 1 = 33
#   failure = kernel writes 0x11 → (0x11 << 1) | 1 = 35
echo "QEMU exited with code $EXIT_CODE (expect 33 for success)"
if [ "$EXIT_CODE" -eq 33 ]; then
    echo "SMOKE PASS"
    exit 0
else
    echo "SMOKE FAIL (exit $EXIT_CODE)"
    exit 1
fi
