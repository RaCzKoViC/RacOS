# RacOS Build System
# Requires: cargo, nasm, clang/lld, qemu-system-x86_64

set shell := ["powershell", "-NoProfile", "-Command"]

project   := "RacOS"
kernel    := "RaCore"
arch      := "x86_64"
qemu      := "qemu-system-x86_64"
target    := "x86_64-unknown-none"
build_dir := "build"
image_dir := "images"

# Default recipe
default: build

# Full build: kernel + userland
build: build-kernel build-userland
    @Write-Host "Build complete."

# Build kernel only
build-kernel:
    cargo build --package racore --target {{target}}

# Build userland (placeholder for C17 build)
build-userland:
    @Write-Host "Userland build: not yet implemented (Phase D+)"

# Run all tests
test: test-unit test-kernel
    @Write-Host "All tests passed."

# Unit tests (host-side)
test-unit:
    cargo test --workspace

# Kernel-specific tests
test-kernel:
    @Write-Host "Kernel tests: not yet implemented (Phase B+)"

# Boot tests in QEMU
test-boot:
    @Write-Host "Boot tests: not yet implemented (Phase B+)"

# Run in QEMU
run: build
    {{qemu}} -machine q35 -cpu qemu64 -m 256M \
        -drive if=pflash,format=raw,file=toolchain/OVMF_CODE.fd,readonly=on \
        -drive if=pflash,format=raw,file=toolchain/OVMF_VARS.fd \
        -kernel {{build_dir}}/racore.elf \
        -serial stdio -display none -no-reboot

# Build bootable ISO
image: build
    @Write-Host "ISO image build: not yet implemented (Phase B)"

# Lint
lint:
    cargo clippy --workspace -- -D warnings

# Format check
fmt:
    cargo fmt --all -- --check

# Clean build artifacts
clean:
    cargo clean
    @if (Test-Path {{build_dir}}) { Remove-Item -Recurse -Force {{build_dir}} }
