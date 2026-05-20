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
