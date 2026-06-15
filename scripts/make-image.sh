#!/usr/bin/env bash
# RacOS - Build UEFI-bootable ESP directory
#
# Creates the EFI System Partition directory structure for QEMU.
# QEMU can boot directly from a directory using: -drive file=fat:rw:esp
#
# This is the Linux/macOS counterpart to scripts/make-image.ps1 — keep them
# in behavioural parity.
#
# Usage: bash scripts/make-image.sh
#
# Environment overrides:
#   RACOS_PROFILE     debug | release   (default: debug)
#   RACOS_TARGET_DIR  cargo target dir  (default: <repo>/target)
#   RACOS_ESP_DIR     ESP staging dir   (default: <repo>/esp)
#
# Prerequisites:
#   - cargo (nightly)
#   - python3 (for pack-initramfs.py)
#   - OVMF firmware at tools/OVMF_CODE.fd (warning only; not fatal)
#
# After running, boot with:
#   just run-uefi   (or see justfile for the full QEMU command)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PROFILE="${RACOS_PROFILE:-debug}"
TARGET_DIR="${RACOS_TARGET_DIR:-$ROOT_DIR/target}"
ESP_DIR="${RACOS_ESP_DIR:-$ROOT_DIR/esp}"

CARGO_PROFILE_FLAGS=()
if [[ "$PROFILE" == "release" ]]; then
    CARGO_PROFILE_FLAGS=("--release")
elif [[ "$PROFILE" != "debug" ]]; then
    echo "ERROR: RACOS_PROFILE must be 'debug' or 'release' (got '$PROFILE')" >&2
    exit 1
fi

echo "=== RacOS Image Builder ==="

# Pin cargo's output dir to TARGET_DIR so RACOS_TARGET_DIR overrides flow all
# the way through to the actual build, not just the post-build lookup.
export CARGO_TARGET_DIR="$TARGET_DIR"

# ── Step 1: Build kernel ──────────────────────────────────────────────────────
echo "[1/5] Building kernel (x86_64-unknown-none)..."
cargo build --package racore --target x86_64-unknown-none "${CARGO_PROFILE_FLAGS[@]}"

# ── Step 2: Build UEFI bootloader ────────────────────────────────────────────
echo "[2/5] Building bootloader (x86_64-unknown-uefi)..."
cargo build --package racos-boot --target x86_64-unknown-uefi "${CARGO_PROFILE_FLAGS[@]}"

# ── Step 3: Create ESP directory structure ────────────────────────────────────
echo "[3/5] Creating ESP directory structure..."
EFI_BOOT_DIR="$ESP_DIR/EFI/BOOT"
mkdir -p "$EFI_BOOT_DIR"

BOOTLOADER_SRC="$TARGET_DIR/x86_64-unknown-uefi/$PROFILE/bootx64.efi"
BOOTLOADER_DST="$EFI_BOOT_DIR/BOOTX64.EFI"
if [[ ! -f "$BOOTLOADER_SRC" ]]; then
    echo "ERROR: Bootloader not found: $BOOTLOADER_SRC" >&2
    exit 1
fi
cp -f "$BOOTLOADER_SRC" "$BOOTLOADER_DST"
echo "  Bootloader -> $BOOTLOADER_DST"

KERNEL_SRC="$TARGET_DIR/x86_64-unknown-none/$PROFILE/racore"
KERNEL_DST="$ESP_DIR/racore.elf"
if [[ ! -f "$KERNEL_SRC" ]]; then
    echo "ERROR: Kernel not found: $KERNEL_SRC" >&2
    exit 1
fi
cp -f "$KERNEL_SRC" "$KERNEL_DST"
echo "  Kernel     -> $KERNEL_DST"

# ── Step 4: Pack initramfs ────────────────────────────────────────────────────
echo "[4/5] Packing initramfs..."
INITRAMFS_OUTPUT="$ESP_DIR/initramfs.img"
INITRAMFS_ROOT="$ROOT_DIR/initramfs-root"
python3 "$ROOT_DIR/scripts/pack-initramfs.py" "$INITRAMFS_ROOT" "$INITRAMFS_OUTPUT"

# ── Step 5: Check for OVMF ───────────────────────────────────────────────────
echo "[5/5] Checking OVMF firmware..."
OVMF_PATH="$ROOT_DIR/tools/OVMF_CODE.fd"
if [[ ! -f "$OVMF_PATH" ]]; then
    echo "  WARNING: OVMF not found at '$OVMF_PATH'"
    echo "  Install via your package manager (e.g. 'sudo apt install ovmf')"
    echo "  or download from: https://www.tianocore.org/ovmf/"
    echo "  Place OVMF_CODE.fd in the 'tools/' directory."
else
    echo "  OVMF found: $OVMF_PATH"
fi

echo ""
echo "=== Build complete ==="
echo "ESP directory: $ESP_DIR"
echo "Run with: just run-uefi"
