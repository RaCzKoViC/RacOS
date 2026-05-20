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
