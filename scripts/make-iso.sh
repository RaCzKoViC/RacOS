#!/usr/bin/env bash
# RacOS - Build Bootable ISO Image
#
# Creates a bootable ISO from the ESP directory using xorriso.
# Requires xorriso to be installed (or mkisofs).
#
# Mirrors scripts/make-iso.ps1 for Linux/macOS.
#
# Usage: bash scripts/make-iso.sh [--release]

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

RELEASE=0
for arg in "$@"; do
    case "$arg" in
        --release) RELEASE=1 ;;
        *) echo "Unknown arg: $arg" >&2; exit 1 ;;
    esac
done

if [[ $RELEASE -eq 1 ]]; then
    PROFILE="release"
else
    PROFILE="dev"
fi

echo "=== RacOS ISO Builder ==="
echo "Profile: $PROFILE"

ESP_DIR="$ROOT_DIR/esp"
ISO_PATH="$ROOT_DIR/racos-$PROFILE.iso"

if [[ ! -d "$ESP_DIR" ]]; then
    echo "ESP directory not found: $ESP_DIR. Run make-image.sh first." >&2
    exit 1
fi

# Use xorriso if available, else mkisofs
if command -v xorriso >/dev/null 2>&1; then
    echo "Using xorriso..."
    xorriso -as mkisofs \
        -o "$ISO_PATH" \
        -b boot/limine-cd.bin \
        -no-emul-boot \
        -boot-load-size 4 \
        -boot-info-table \
        --efi-boot boot/limine-eltorito-efi.bin \
        -efi-boot-part --efi-boot-image \
        --protective-msdos-label \
        "$ESP_DIR"
elif command -v mkisofs >/dev/null 2>&1; then
    echo "Using mkisofs..."
    mkisofs -o "$ISO_PATH" \
        -b boot/limine-cd.bin \
        -no-emul-boot \
        -boot-load-size 4 \
        -boot-info-table \
        --efi-boot boot/limine-eltorito-efi.bin \
        -efi-boot-part --efi-boot-image \
        --protective-msdos-label \
        "$ESP_DIR"
else
    echo "Neither xorriso nor mkisofs found. Install one to create ISO." >&2
    exit 1
fi

echo ""
echo "=== ISO created ==="
echo "Path: $ISO_PATH"
SIZE_BYTES=$(stat -c%s "$ISO_PATH" 2>/dev/null || stat -f%z "$ISO_PATH")
SIZE_MB=$(awk "BEGIN {printf \"%.2f\", $SIZE_BYTES / 1048576}")
echo "Size: $SIZE_MB MB"
