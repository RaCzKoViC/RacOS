# Phase 1 — Cross-platform build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `just build` and `just qemu` work on a fresh Ubuntu 22.04 install with no user-side patches, while preserving the existing Windows flow.

**Architecture:** Replace the PowerShell-only `justfile` with a portable one that uses `just`'s `[unix]`/`[windows]` recipe attributes. Mirror each critical PowerShell script with a bash equivalent next to it. Remove the hardcoded Windows target-dir path. Extend GitHub Actions CI to run a matrix with Linux required and Windows/macOS advisory.

**Tech Stack:** `just` (build runner), bash 5+, PowerShell 7+, GitHub Actions, Python 3 (already used by `pack-initramfs.py`), QEMU/OVMF.

**Scope of this plan:** Justfile refactor + three critical `.sh` scripts (build-image, make-image, make-iso) + Linux dev docs + CI matrix. The remaining `.ps1` scripts (`run-qemu`, `runtime-validation*`, `validate-*`, `pack-initramfs.ps1`) stay Windows-only — they are not on the `just qemu` critical path. Porting them is a follow-up.

**Spec reference:** `docs/superpowers/specs/2026-05-20-cross-platform-build-and-kernel-correctness-design.md` §4.

---

## File structure

| File | Action | Responsibility |
|---|---|---|
| `justfile` | Modify | Drop PowerShell `set shell`, replace hardcoded `target_dir`, split recipes into `[unix]`/`[windows]` variants |
| `scripts/build-image.sh` | Create | Bash port of `build-image.ps1` — assemble ESP + initramfs |
| `scripts/make-image.sh` | Create | Bash port of `make-image.ps1` — stage ESP directory |
| `scripts/make-iso.sh` | Create | Bash port of `make-iso.ps1` — produce bootable ISO via `xorriso` |
| `docs/DEVELOPMENT_LINUX.md` | Create | apt one-liner, env vars, `just build`/`just qemu` quickstart, troubleshooting |
| `docs/DEVELOPMENT_WINDOWS.md` | Create | Same content adapted for Windows + link from existing `.github/instructions/development.instructions.md` |
| `README.md` | Modify | Add Quick start block linking both DEVELOPMENT_*.md files |
| `.github/workflows/ci.yml` | Modify | Wrap existing Linux jobs in matrix; add Windows + macOS jobs with `continue-on-error: true` |

---

## Task 1: Strip hardcoded `target_dir` from justfile

**Files:**
- Modify: `justfile:4-13`

- [ ] **Step 1: Read current justfile**

Confirm lines 4 and 12 still match what this plan references. If `set shell` is on line 4 and `target_dir` on line 12, proceed.

- [ ] **Step 2: Replace the head of justfile**

Replace the first 13 lines (everything up to and including `target_dir := …`) with:

```just
# RacOS Build System
# Requires: cargo (nightly), qemu-system-x86_64
#
# Runs on Linux and Windows. Override the target dir with the env var
# RACOS_TARGET_DIR; otherwise defaults to `target/` in the repo.

project   := "RacOS"
kernel    := "RaCore"
arch      := "x86_64"
qemu      := "qemu-system-x86_64"
target    := "x86_64-unknown-none"
uefi_target := "x86_64-unknown-uefi"
target_dir := env_var_or_default("RACOS_TARGET_DIR", "target")
```

(Note the removed `set shell := ["powershell", …]` line — `just` will now use the OS default shell per recipe.)

- [ ] **Step 3: Commit**

```bash
cd /home/raczkov/Pulpit/Projekty/RacOS
git add justfile
git commit -m "build: replace hardcoded Windows target_dir with RACOS_TARGET_DIR env var"
```

---

## Task 2: Split run/test/build recipes by OS

**Files:**
- Modify: `justfile:14-102`

- [ ] **Step 1: Replace the body recipes with OS-attributed versions**

Replace everything from line 14 (after the header) to the end of the file with:

```just
# Default recipe
default: build

# Full build: kernel + boot + userland
build: build-kernel build-boot build-userland

# Build kernel only
build-kernel:
    cargo build --package racore --target {{target}}

# Build UEFI bootloader
build-boot:
    cargo build --package racos-boot --target {{uefi_target}}

# Build userland crates (default members)
build-userland:
    cargo build

# Run all tests
test: test-unit

# Unit tests (host-side, default members only)
test-unit:
    cargo test

# Boot tests in QEMU (direct kernel load, no UEFI)
[unix]
test-boot: build-kernel
    {{qemu}} -machine q35 -cpu qemu64 -m 256M \
        -serial stdio -display none -no-reboot \
        -kernel "{{target_dir}}/{{target}}/debug/racore" 2>&1 | head -n 30

[windows]
test-boot: build-kernel
    powershell -NoProfile -Command "& {{qemu}} -machine q35 -cpu qemu64 -m 256M `
        -serial stdio -display none -no-reboot `
        -kernel '{{target_dir}}/{{target}}/debug/racore' 2>&1 | Select-Object -First 30"

# Run in QEMU
[unix]
run: build-kernel
    {{qemu}} -machine q35 -cpu qemu64 -m 256M \
        -serial stdio -display none -no-reboot \
        -kernel "{{target_dir}}/{{target}}/debug/racore"

[windows]
run: build-kernel
    powershell -NoProfile -Command "& {{qemu}} -machine q35 -cpu qemu64 -m 256M `
        -serial stdio -display none -no-reboot `
        -kernel '{{target_dir}}/{{target}}/debug/racore'"

# Lint
lint:
    cargo clippy --workspace -- -D warnings

# Format check
fmt:
    cargo fmt --all -- --check

# Build UEFI disk image (ESP directory)
[unix]
image:
    bash scripts/make-image.sh

[windows]
image:
    powershell -NoProfile -File scripts/make-image.ps1

# Build full image: kernel + coreutils + initramfs
[unix]
build-image:
    bash scripts/build-image.sh

[windows]
build-image:
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-image.ps1

# Build full image (release)
[unix]
build-image-release:
    bash scripts/build-image.sh --release

[windows]
build-image-release:
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-image.ps1 -Release

# Build bootable ISO
[unix]
iso:
    bash scripts/make-iso.sh

[windows]
iso:
    powershell -NoProfile -File scripts/make-iso.ps1

# Build bootable ISO (release)
[unix]
iso-release:
    bash scripts/make-iso.sh --release

[windows]
iso-release:
    powershell -NoProfile -File scripts/make-iso.ps1 -Release

# Run in QEMU with UEFI bootloader (requires OVMF at tools/OVMF_CODE.fd)
[unix]
run-uefi: image
    {{qemu}} -machine q35 -cpu qemu64 -m 512M \
        -drive if=pflash,format=raw,file=tools/OVMF_CODE.fd,readonly=on \
        -drive file=fat:rw:esp,format=raw \
        -serial stdio -display none -no-reboot

[windows]
run-uefi: image
    powershell -NoProfile -Command "& {{qemu}} -machine q35 -cpu qemu64 -m 512M `
        -drive if=pflash,format=raw,file=tools\\OVMF_CODE.fd,readonly=on `
        -drive file=fat:rw:esp,format=raw `
        -serial stdio -display none -no-reboot"

# Test UEFI boot (non-interactive, validates serial output)
[unix]
test-uefi: image
    {{qemu}} -machine q35 -cpu qemu64 -m 512M \
        -drive if=pflash,format=raw,file=tools/OVMF_CODE.fd,readonly=on \
        -drive file=fat:rw:esp,format=raw \
        -serial stdio -display none -no-reboot 2>&1 | head -n 60

[windows]
test-uefi: image
    powershell -NoProfile -Command "& {{qemu}} -machine q35 -cpu qemu64 -m 512M `
        -drive if=pflash,format=raw,file=tools\\OVMF_CODE.fd,readonly=on `
        -drive file=fat:rw:esp,format=raw `
        -serial stdio -display none -no-reboot 2>&1 | Select-Object -First 60"

# Clean build artifacts
[unix]
clean:
    cargo clean
    rm -rf esp initramfs-root

[windows]
clean:
    cargo clean
    powershell -NoProfile -Command "Remove-Item -Recurse -Force esp -ErrorAction SilentlyContinue; Remove-Item -Recurse -Force initramfs-root -ErrorAction SilentlyContinue"
```

- [ ] **Step 2: Validate justfile syntax**

Run: `just --list`
Expected: prints recipes without parse errors.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "build: split justfile recipes into [unix]/[windows] variants"
```

---

## Task 3: Port `make-image.ps1` to `make-image.sh`

**Files:**
- Read: `scripts/make-image.ps1` (to understand current behavior)
- Create: `scripts/make-image.sh`

- [ ] **Step 1: Read the PowerShell source**

```bash
cat scripts/make-image.ps1
```

Note: this script stages the EFI System Partition layout. The bash port must produce the same `esp/` directory structure.

- [ ] **Step 2: Write `scripts/make-image.sh`**

Create the file with:

```bash
#!/usr/bin/env bash
# Stage the EFI System Partition (ESP) directory layout.
# Mirrors scripts/make-image.ps1 for Linux/macOS.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${RACOS_TARGET_DIR:-$ROOT_DIR/target}"
ESP_DIR="$ROOT_DIR/esp"

BOOT_EFI="$TARGET_DIR/x86_64-unknown-uefi/debug/bootx64.efi"
KERNEL_ELF="$TARGET_DIR/x86_64-unknown-none/debug/racore"

if [[ ! -f "$BOOT_EFI" ]]; then
    echo "ERROR: bootloader not found at $BOOT_EFI" >&2
    echo "Run: just build-boot" >&2
    exit 1
fi
if [[ ! -f "$KERNEL_ELF" ]]; then
    echo "ERROR: kernel not found at $KERNEL_ELF" >&2
    echo "Run: just build-kernel" >&2
    exit 1
fi

mkdir -p "$ESP_DIR/EFI/BOOT"
cp -f "$BOOT_EFI"   "$ESP_DIR/EFI/BOOT/BOOTX64.EFI"
cp -f "$KERNEL_ELF" "$ESP_DIR/racore.elf"

if [[ -d "$ROOT_DIR/initramfs-root" ]]; then
    python3 "$ROOT_DIR/scripts/pack-initramfs.py" \
        "$ROOT_DIR/initramfs-root" "$ESP_DIR/initramfs.img"
fi

echo "ESP staged at: $ESP_DIR"
ls -la "$ESP_DIR"
```

- [ ] **Step 3: Make executable**

```bash
chmod +x scripts/make-image.sh
```

- [ ] **Step 4: Manual smoke (only after Task 5 ensures build artifacts exist)**

Defer execution until after `build-image.sh` works end-to-end.

- [ ] **Step 5: Commit**

```bash
git add scripts/make-image.sh
git commit -m "build: add scripts/make-image.sh (bash port of make-image.ps1)"
```

---

## Task 4: Port `make-iso.ps1` to `make-iso.sh`

**Files:**
- Read: `scripts/make-iso.ps1`
- Create: `scripts/make-iso.sh`

- [ ] **Step 1: Read the PowerShell source**

```bash
cat scripts/make-iso.ps1
```

Identify: the source ESP path, the target ISO path, and the `xorriso`/`mkisofs` invocation.

- [ ] **Step 2: Write `scripts/make-iso.sh`**

Create the file with the following content. If the `.ps1` uses different paths or flags, mirror those choices here:

```bash
#!/usr/bin/env bash
# Produce a bootable ISO image from the staged ESP directory.
# Mirrors scripts/make-iso.ps1 for Linux/macOS.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ESP_DIR="$ROOT_DIR/esp"
ISO_OUT="$ROOT_DIR/racos.iso"

RELEASE=0
for arg in "$@"; do
    case "$arg" in
        --release) RELEASE=1 ;;
        *) echo "Unknown arg: $arg" >&2; exit 1 ;;
    esac
done

if [[ ! -d "$ESP_DIR" ]]; then
    echo "ERROR: ESP dir not staged at $ESP_DIR" >&2
    echo "Run: just image" >&2
    exit 1
fi

if ! command -v xorriso >/dev/null 2>&1; then
    echo "ERROR: xorriso not found. Install with: sudo apt-get install xorriso" >&2
    exit 1
fi

xorriso -as mkisofs \
    -iso-level 3 \
    -V "RACOS" \
    -e EFI/BOOT/BOOTX64.EFI -no-emul-boot \
    -isohybrid-gpt-basdat \
    -o "$ISO_OUT" \
    "$ESP_DIR"

echo "ISO produced at: $ISO_OUT"
if [[ $RELEASE -eq 1 ]]; then
    echo "(release build)"
fi
ls -la "$ISO_OUT"
```

- [ ] **Step 3: Make executable**

```bash
chmod +x scripts/make-iso.sh
```

- [ ] **Step 4: Commit**

```bash
git add scripts/make-iso.sh
git commit -m "build: add scripts/make-iso.sh (bash port of make-iso.ps1)"
```

---

## Task 5: Port `build-image.ps1` to `build-image.sh`

**Files:**
- Read: `scripts/build-image.ps1`
- Create: `scripts/build-image.sh`

- [ ] **Step 1: Read the PowerShell source**

```bash
cat scripts/build-image.ps1
```

Identify the sequence: typically (a) `cargo build` the kernel + bootloader, (b) `cargo build` the userland, (c) stage `initramfs-root/`, (d) call `make-image.ps1` (now `.sh`). If the script does anything different, adapt the bash port accordingly.

- [ ] **Step 2: Write `scripts/build-image.sh`**

```bash
#!/usr/bin/env bash
# End-to-end image build: kernel + boot + userland + initramfs + ESP staging.
# Mirrors scripts/build-image.ps1 for Linux/macOS.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${RACOS_TARGET_DIR:-$ROOT_DIR/target}"
export CARGO_TARGET_DIR="$TARGET_DIR"

PROFILE_FLAG=""
PROFILE_DIR="debug"
for arg in "$@"; do
    case "$arg" in
        --release) PROFILE_FLAG="--release"; PROFILE_DIR="release" ;;
        *) echo "Unknown arg: $arg" >&2; exit 1 ;;
    esac
done

cd "$ROOT_DIR"

echo "==> Building kernel (racore) [$PROFILE_DIR]"
cargo build $PROFILE_FLAG --package racore --target x86_64-unknown-none

echo "==> Building bootloader (racos-boot) [$PROFILE_DIR]"
cargo build $PROFILE_FLAG --package racos-boot --target x86_64-unknown-uefi

echo "==> Building userland workspace [$PROFILE_DIR]"
cargo build $PROFILE_FLAG --workspace --exclude racore --exclude racos-boot

# Stage initramfs-root if userland produces binaries that need shipping.
# The exact list of binaries copied here should match build-image.ps1; if your
# .ps1 copies a specific subset, mirror that selection. For MVP we copy every
# coreutils binary from the userland output directory.
INITRAMFS_ROOT="$ROOT_DIR/initramfs-root"
mkdir -p "$INITRAMFS_ROOT/bin" "$INITRAMFS_ROOT/sbin" "$INITRAMFS_ROOT/etc"

for bin in "$TARGET_DIR/x86_64-racos-user/$PROFILE_DIR/"* "$TARGET_DIR/$PROFILE_DIR/"*; do
    if [[ -f "$bin" && -x "$bin" && ! "$bin" =~ \.(d|rlib)$ ]]; then
        name="$(basename "$bin")"
        # Skip cargo build metadata files
        case "$name" in
            *.json|build-script-*|*-*) continue ;;
        esac
        cp -f "$bin" "$INITRAMFS_ROOT/bin/$name"
    fi
done

echo "==> Staging ESP"
bash "$ROOT_DIR/scripts/make-image.sh"

echo "==> Image ready."
```

> **Note:** The coreutils-binary selection logic above is a best-effort default. If the `.ps1` script copies a specific allow-list of binaries, replicate that allow-list verbatim here in place of the `for` loop.

- [ ] **Step 3: Make executable**

```bash
chmod +x scripts/build-image.sh
```

- [ ] **Step 4: Smoke-run the full chain on Linux**

```bash
just build-image
```

Expected: cargo build succeeds, `initramfs-root/bin/` is populated, `esp/racore.elf` and `esp/EFI/BOOT/BOOTX64.EFI` exist. Confirm:

```bash
ls -la esp/EFI/BOOT/BOOTX64.EFI esp/racore.elf
```

If either is missing, debug before continuing.

- [ ] **Step 5: Commit**

```bash
git add scripts/build-image.sh
git commit -m "build: add scripts/build-image.sh (bash port of build-image.ps1)"
```

---

## Task 6: Smoke-test `just qemu` (run-uefi) on Linux

**Files:**
- (No file changes — verification only)

- [ ] **Step 1: Ensure OVMF is available locally**

Expected at `tools/OVMF_CODE.fd` (used by `just run-uefi`). On Ubuntu it can be copied from the system package:

```bash
sudo apt-get install -y ovmf
mkdir -p tools
cp -n /usr/share/OVMF/OVMF_CODE.fd tools/OVMF_CODE.fd
```

- [ ] **Step 2: Run the UEFI flow**

```bash
just run-uefi
```

Expected: QEMU window or serial output shows `RACORE: RacOS kernel starting`, eventually `[init] spawned /bin/sh`, then the `racsh` banner. Kill with Ctrl-A X.

- [ ] **Step 3: If output stalls before the kernel banner**

Re-check that `esp/` has both `EFI/BOOT/BOOTX64.EFI` and `racore.elf`. Re-run `just build-image` if either is missing.

- [ ] **Step 4: No commit (verification step only)**

---

## Task 7: Add `docs/DEVELOPMENT_LINUX.md`

**Files:**
- Create: `docs/DEVELOPMENT_LINUX.md`

- [ ] **Step 1: Write the file**

```markdown
# Developing RacOS on Linux

This guide targets Ubuntu/Debian. Other distros work too — adapt the package
names.

## One-time setup

```bash
# Toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
rustup toolchain install nightly --component rust-src,llvm-tools-preview
rustup target add x86_64-unknown-none x86_64-unknown-uefi

# Build runner + emulator + image tools
sudo apt-get update
sudo apt-get install -y just qemu-system-x86 ovmf nasm mtools dosfstools xorriso python3

# OVMF firmware for the UEFI boot path
mkdir -p tools
cp -n /usr/share/OVMF/OVMF_CODE.fd tools/OVMF_CODE.fd
```

## Build and run

```bash
git clone https://github.com/RaCzKoViC/RacOS.git
cd RacOS
just build           # compile kernel + bootloader + userland
just build-image     # stage ESP + initramfs
just run-uefi        # boot in QEMU (Ctrl-A X to quit)
```

## Where artefacts live

By default everything is under `target/` in the repo. To put it elsewhere,
export `RACOS_TARGET_DIR`:

```bash
export RACOS_TARGET_DIR=/tmp/racos-target
just build
```

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `qemu-system-x86_64: not found` | QEMU not installed | `sudo apt-get install qemu-system-x86` |
| `xorriso: not found` | Missing for `just iso` | `sudo apt-get install xorriso` |
| Boot stuck before `RACORE:` banner | OVMF missing | Copy `OVMF_CODE.fd` into `tools/` |
| `cargo build` fails on `rust-src` | Wrong toolchain | `rustup component add rust-src --toolchain nightly` |
| `nasm: command not found` | Assembler missing | `sudo apt-get install nasm` |
```

- [ ] **Step 2: Commit**

```bash
git add docs/DEVELOPMENT_LINUX.md
git commit -m "docs: add Linux development quickstart"
```

---

## Task 8: Add `docs/DEVELOPMENT_WINDOWS.md`

**Files:**
- Create: `docs/DEVELOPMENT_WINDOWS.md`
- Reference: `.github/instructions/development.instructions.md` (existing knowledge)

- [ ] **Step 1: Read the existing instructions file**

```bash
cat .github/instructions/development.instructions.md
```

This file already contains Windows-oriented build steps. Copy the relevant prerequisites and add the `RACOS_TARGET_DIR` env-var instructions.

- [ ] **Step 2: Write `docs/DEVELOPMENT_WINDOWS.md`**

```markdown
# Developing RacOS on Windows

This guide assumes Windows 11 with PowerShell 7+.

## One-time setup

```powershell
# Toolchain
winget install Rustlang.Rustup
rustup toolchain install nightly --component rust-src,llvm-tools-preview
rustup target add x86_64-unknown-none x86_64-unknown-uefi

# Build runner + emulator + assembler
winget install Casey.Just
winget install QEMU.QEMU
winget install NASM.NASM

# OVMF firmware (download OVMF_CODE.fd manually or via your package manager,
# then place it at tools\OVMF_CODE.fd).
```

## Build and run

```powershell
git clone https://github.com/RaCzKoViC/RacOS.git
cd RacOS
just build
just build-image
just run-uefi
```

## Custom target directory

```powershell
$env:RACOS_TARGET_DIR = "C:\Users\YourName\RacOS-target"
just build
```

## Troubleshooting

See `.github/instructions/development.instructions.md` for the original
Windows-specific developer notes.
```

- [ ] **Step 3: Commit**

```bash
git add docs/DEVELOPMENT_WINDOWS.md
git commit -m "docs: add Windows development quickstart"
```

---

## Task 9: Link both dev guides from README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Locate the right insertion point**

Read `README.md`. Find the top-of-document area, after the project description but before any deep technical sections.

- [ ] **Step 2: Add a "Quick start" block**

Insert (or merge into an existing section) the following after the project tagline:

```markdown
## Quick start

- **Linux**: see [docs/DEVELOPMENT_LINUX.md](docs/DEVELOPMENT_LINUX.md)
- **Windows**: see [docs/DEVELOPMENT_WINDOWS.md](docs/DEVELOPMENT_WINDOWS.md)

Both guides walk you through toolchain install, build, and booting RacOS in
QEMU.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: link Linux/Windows quickstarts from README"
```

---

## Task 10: Add CI matrix (Linux required, Windows + macOS advisory)

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add matrix to the existing `build` job**

Replace the current `build:` block (lines 14–60) with:

```yaml
  build:
    name: Build kernel + bootloader + userland (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    continue-on-error: ${{ matrix.required == false }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-22.04
            required: true
          - os: windows-latest
            required: false
          - os: macos-latest
            required: false
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain (pinned via rust-toolchain.toml)
        uses: dtolnay/rust-toolchain@nightly
        with:
          components: rust-src, llvm-tools-preview
          targets: x86_64-unknown-none, x86_64-unknown-uefi

      - name: Install system deps (Linux)
        if: matrix.os == 'ubuntu-22.04'
        run: sudo apt-get update && sudo apt-get install -y nasm

      - name: Install system deps (Windows)
        if: matrix.os == 'windows-latest'
        run: choco install nasm -y

      - name: Install system deps (macOS)
        if: matrix.os == 'macos-latest'
        run: brew install nasm

      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: ${{ runner.os }}-cargo-

      - name: Build kernel (racore)
        run: cargo build --package racore --target x86_64-unknown-none

      - name: Build bootloader (racos-boot)
        run: cargo build --package racos-boot --target x86_64-unknown-uefi

      - name: Build userland workspace
        run: cargo build --workspace --exclude racore --exclude racos-boot

      - name: Upload kernel artifact
        if: matrix.os == 'ubuntu-22.04'
        uses: actions/upload-artifact@v4
        with:
          name: kernel-elf
          path: target/x86_64-unknown-none/debug/racore
          if-no-files-found: warn

      - name: Upload bootloader artifact
        if: matrix.os == 'ubuntu-22.04'
        uses: actions/upload-artifact@v4
        with:
          name: bootloader-efi
          path: target/x86_64-unknown-uefi/debug/bootx64.efi
          if-no-files-found: warn
```

> Note: artifact upload happens only on Linux to avoid double-uploads from the Windows/macOS legs.

- [ ] **Step 2: Validate workflow with `yamllint` or by pushing to a feature branch**

```bash
# If you have actionlint installed:
actionlint .github/workflows/ci.yml
```

If not, push to a feature branch and watch GitHub Actions report parse errors before merging.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add OS matrix to build job (Linux required, Windows/macOS advisory)"
```

---

## Task 11: Verify pre-existing Linux jobs still target Linux only

**Files:**
- Read: `.github/workflows/ci.yml:62-228`

- [ ] **Step 1: Confirm the following jobs keep `runs-on: ubuntu-latest`**

`test-unit`, `boot-smoke`, `interactive-smoke`, `lint` — these jobs depend on apt-installed packages and QEMU/OVMF and must stay Linux-only.

- [ ] **Step 2: No code change**

These jobs are deliberately kept single-OS for now. Multi-OS runtime testing is a follow-up.

- [ ] **Step 3: Skip commit (read-only verification)**

---

## Task 12: End-to-end verification on Linux

**Files:**
- (No file changes — verification only)

- [ ] **Step 1: From a freshly cloned working copy on Linux**

```bash
cd /tmp
rm -rf RacOS-fresh
git clone /home/raczkov/Pulpit/Projekty/RacOS RacOS-fresh
cd RacOS-fresh
git checkout <your phase-1 branch>
```

- [ ] **Step 2: Build**

```bash
just build
```

Expected: kernel, bootloader, userland all compile.

- [ ] **Step 3: Stage image**

```bash
just build-image
```

Expected: `esp/EFI/BOOT/BOOTX64.EFI`, `esp/racore.elf`, populated `initramfs-root/bin/`.

- [ ] **Step 4: Boot in QEMU**

```bash
just run-uefi
```

Expected: serial output reaches `racsh 0.1.0` banner.

- [ ] **Step 5: No commit (final verification)**

---

## Task 13: Final smoke on Windows (manual)

**Files:**
- (No file changes — verification only)

- [ ] **Step 1: On a Windows machine, fetch the same branch and run**

```powershell
just build
just build-image
just run-uefi
```

Expected: same outcome as Linux. The Windows recipes invoke the PowerShell scripts unchanged from before the refactor.

- [ ] **Step 2: If anything breaks**

Open an issue with the failing recipe name and full PowerShell error. Do not block PR merge — the Windows leg is advisory.

- [ ] **Step 3: No commit (manual verification)**

---

## Definition of done for Phase 1

All true:

- ✅ `just build` succeeds on Ubuntu 22.04 with no edits to repo files.
- ✅ `just build-image && just run-uefi` boots RacOS through to the racsh prompt on Linux.
- ✅ CI matrix workflow is green on `ubuntu-22.04`. Windows and macOS jobs run and report (may be red without blocking).
- ✅ Pre-existing Windows flow still works (manually verified).
- ✅ Three new bash scripts and two new doc files added; no `.ps1` deleted.

---

## Self-review notes

- **Spec coverage**: §4 of the spec is fully addressed by Tasks 1–13. The "Linux required, Windows advisory" choice from §10 is wired via `continue-on-error: ${{ matrix.required == false }}`.
- **Placeholders**: One callout in Task 5 notes that the coreutils-binary copy logic is a best-effort default; the engineer should mirror the exact allow-list from `build-image.ps1` if one exists. This is an explicit "do the right thing here" instruction, not a hidden TODO.
- **Type consistency**: Recipe names (`build`, `build-image`, `image`, `iso`, `run-uefi`, `clean`, etc.) match between justfile and dev docs. Script filenames match between justfile invocations and Task 3–5 creates.
- **Scope**: Phase 1 stays focused on the build-system port. Kernel changes are in the separate Phase 2 plan.

---

## Open follow-ups (NOT in this plan)

- Port the remaining `.ps1` scripts (`run-qemu`, `runtime-validation*`, `validate-*`, `pack-initramfs.ps1`) for Linux feature parity.
- Add a `boot-smoke-windows` CI job once Windows runners with QEMU prove stable.
- Decide whether to drop `.ps1` versions of scripts after a few sprints of dual maintenance.
