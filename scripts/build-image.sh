#!/usr/bin/env bash
# RacOS - Build kernel + userland + initramfs image
#
# Builds the kernel and all coreutils for x86_64-unknown-none, assembles the
# initramfs-root staging tree, and packs it into esp/initramfs.img.
#
# This is the Linux/macOS counterpart to scripts/build-image.ps1 — keep them
# in behavioural parity.
#
# Usage: bash scripts/build-image.sh [--release]
#
# Environment overrides:
#   RACOS_TARGET_DIR  cargo target dir  (default: cargo metadata target_directory)
#
# Prerequisites:
#   - cargo (nightly)
#   - python3 (for pack-initramfs.py)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# --- Argument parsing ---------------------------------------------------------
RELEASE=0
for arg in "$@"; do
    case "$arg" in
        --release|-Release) RELEASE=1 ;;
        *) echo "Unknown arg: $arg" >&2; exit 1 ;;
    esac
done

if [[ "$RELEASE" -eq 1 ]]; then
    PROFILE="release"
    PROFILE_DIR="release"
    CARGO_PROFILE_FLAGS=("--release")
else
    PROFILE="dev"
    PROFILE_DIR="debug"
    CARGO_PROFILE_FLAGS=()
fi

CARGO_FLAGS=("--target" "x86_64-unknown-none" "${CARGO_PROFILE_FLAGS[@]}")

# --- Discover target dir ------------------------------------------------------
if [[ -n "${RACOS_TARGET_DIR:-}" ]]; then
    TARGET_DIR="$RACOS_TARGET_DIR"
else
    TARGET_DIR="$(cargo metadata --format-version 1 --no-deps --quiet \
        | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')"
fi
# Pin cargo's output dir so subsequent `cargo build` calls write to TARGET_DIR.
export CARGO_TARGET_DIR="$TARGET_DIR"
BIN_DIR="$TARGET_DIR/x86_64-unknown-none/$PROFILE_DIR"

# Keep caller RUSTFLAGS and use kernel-specific flags only for kernel build.
OLD_RUSTFLAGS="${RUSTFLAGS:-}"

echo "=== RacOS Image Builder ==="
echo "Profile: $PROFILE"
echo "Target dir: $TARGET_DIR"

# --- Step 1: Build kernel -----------------------------------------------------
echo ""
echo "[1/4] Building kernel..."
RUSTFLAGS="-C relocation-model=static -C link-arg=-no-pie" \
    cargo build --package racore "${CARGO_FLAGS[@]}"

# --- Step 2: Build coreutils --------------------------------------------------
echo ""
echo "[2/4] Building coreutils..."
COREUTILS=(
    racos-hello racos-echo racos-cat racos-true racos-false racos-sh
    racos-init racos-test racos-ls racos-wc racos-uptime racos-mkdir
    racos-rm racos-sleep racos-head racos-tail racos-env racos-basename
    racos-dirname racos-grep racos-cp racos-mv racos-cut racos-uniq
    racos-find racos-od racos-tee racos-hexdump racterm
)
for pkg in "${COREUTILS[@]}"; do
    RUSTFLAGS="$OLD_RUSTFLAGS" \
        cargo build --package "$pkg" "${CARGO_FLAGS[@]}" \
            -Z build-std=core,alloc \
            -Z build-std-features=compiler-builtins-mem
done

# --- Step 3: Assemble initramfs root -----------------------------------------
echo ""
echo "[3/4] Assembling initramfs..."
INITRAMFS_ROOT="$ROOT_DIR/initramfs-root"

# Clean and recreate
if [[ -e "$INITRAMFS_ROOT" ]]; then
    rm -rf "$INITRAMFS_ROOT"
fi
mkdir -p "$INITRAMFS_ROOT/bin"
mkdir -p "$INITRAMFS_ROOT/sbin"
mkdir -p "$INITRAMFS_ROOT/etc/racinit"
mkdir -p "$INITRAMFS_ROOT/etc"

# Copy binaries — bin names match the [[bin]] name in Cargo.toml
BIN_LIST=(
    hello echo cat true false sh racterm racos-test ls wc uptime mkdir
    rm sleep head tail env basename dirname grep cp mv cut uniq find od
    tee hexdump
)
SBIN_LIST=(init)

for bin in "${BIN_LIST[@]}"; do
    src="$BIN_DIR/$bin"
    if [[ ! -f "$src" ]]; then
        echo "WARNING: Binary not found: $bin - skipping" >&2
        continue
    fi
    dst="$INITRAMFS_ROOT/bin/$bin"
    cp -f "$src" "$dst"
    size=$(wc -c < "$dst")
    echo "  bin/$bin [$size bytes]"
done

for bin in "${SBIN_LIST[@]}"; do
    src="$BIN_DIR/$bin"
    if [[ ! -f "$src" ]]; then
        echo "WARNING: Binary not found: $bin - skipping" >&2
        continue
    fi
    dst="$INITRAMFS_ROOT/sbin/$bin"
    cp -f "$src" "$dst"
    size=$(wc -c < "$dst")
    echo "  sbin/$bin [$size bytes]"
done

# --- Step 4: Pack initramfs image ---------------------------------------------
echo ""
echo "[4/4] Packing initramfs image..."
ESP_DIR="$ROOT_DIR/esp"
mkdir -p "$ESP_DIR"

# Mirrors build-image.ps1, which calls pack-initramfs.ps1 directly (not
# make-image.ps1). The Linux build uses pack-initramfs.py for parity with
# scripts/make-image.sh.
python3 "$ROOT_DIR/scripts/pack-initramfs.py" "$INITRAMFS_ROOT" "$ESP_DIR/initramfs.img"

echo ""
echo "=== Build complete ==="
echo "Kernel:    $BIN_DIR/racore"
echo "Initramfs: $ESP_DIR/initramfs.img"
