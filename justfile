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

# Default recipe
default: build

# Full build: kernel + boot + userland
build: build-kernel build-boot build-userland
    @Write-Host "Build complete."

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
    @Write-Host "All tests passed."

# Unit tests (host-side, default members only)
test-unit:
    cargo test

# Boot tests in QEMU (direct kernel load, no UEFI)
test-boot: build-kernel
    {{qemu}} -machine q35 -cpu qemu64 -m 256M `
        -serial stdio -display none -no-reboot `
        -kernel "{{target_dir}}/{{target}}/debug/racore" `
        2>&1 | Select-Object -First 30

# Run in QEMU
run: build-kernel
    {{qemu}} -machine q35 -cpu qemu64 -m 256M `
        -serial stdio -display none -no-reboot `
        -kernel "{{target_dir}}/{{target}}/debug/racore"

# Lint
lint:
    cargo clippy --workspace -- -D warnings

# Format check
fmt:
    cargo fmt --all -- --check

# Build UEFI disk image (ESP directory)
image:
    powershell -NoProfile -File scripts/make-image.ps1

# Build full image: kernel + coreutils + initramfs
build-image:
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-image.ps1

# Build full image (release)
build-image-release:
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-image.ps1 -Release

# Build bootable ISO
iso:
    powershell -NoProfile -File scripts/make-iso.ps1

# Build bootable ISO (release)
iso-release:
    powershell -NoProfile -File scripts/make-iso.ps1 -Release

# Run in QEMU with UEFI bootloader (requires OVMF at tools\OVMF_CODE.fd)
run-uefi: image
    {{qemu}} -machine q35 -cpu qemu64 -m 512M `
        -drive if=pflash,format=raw,file=tools\OVMF_CODE.fd,readonly=on `
        -drive file=fat:rw:esp,format=raw `
        -serial stdio -display none -no-reboot

# Test UEFI boot (non-interactive, validates serial output)
test-uefi: image
    {{qemu}} -machine q35 -cpu qemu64 -m 512M `
        -drive if=pflash,format=raw,file=tools\OVMF_CODE.fd,readonly=on `
        -drive file=fat:rw:esp,format=raw `
        -serial stdio -display none -no-reboot `
        2>&1 | Select-Object -First 60

# Clean build artifacts
clean:
    cargo clean
    Remove-Item -Recurse -Force esp -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force initramfs-root -ErrorAction SilentlyContinue
