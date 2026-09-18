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

## Run the same smoke CI runs

Before pushing a branch you can reproduce the kernel-smoke CI job locally
in a few seconds:

```bash
just smoke           # bare ci-smoke (exit 33 = PASS)
just smoke-disk      # ci-smoke with a 16 MiB AHCI disk attached
```

These wrap `scripts/run-ci-smoke.sh`, which is the bash counterpart of
the `.ps1` helper used on Windows. Both invoke the same QEMU command,
the same kernel feature (`--features ci-smoke`), and the same
`isa-debug-exit` exit-code contract (33 = PASS, 35 = FAIL, 124 = timeout).

If `tools/OVMF_CODE.fd` is present it takes priority over the system
copy, useful when the runner ships a stripped-down OVMF that doesn't
boot UEFI apps cleanly.

## Run the in-guest suite exactly as CI does

The `guest-suite` CI job boots the canonical machine (q35, 2 vCPU, 512 MiB,
a zeroed 16 MiB AHCI disk, a legacy VirtIO-net NIC on slirp) and drives
`racos-test` through racsh until the suite reports its own verdict. The
machine, the readiness waits and the grading live in
`scripts/guest-suite.py`; the Windows gate (`scripts/test-racos-test.ps1`)
is a wrapper around the same file, so there is one definition to keep.

```bash
just build-image                                   # kernel + userland + initramfs
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/debug/bootx64.efi esp/EFI/BOOT/BOOTX64.EFI
cp target/x86_64-unknown-none/debug/racore esp/racore.elf
just guest-suite                                   # or: python3 scripts/guest-suite.py
```

Exit 0 means `=== Results: N passed, 0 failed ===`, `RACOS-TEST-EXIT=0`
echoed by the shell, no kernel panic, and every required device seen in
the boot log. Exit 1 is a verdict failure or a missing device (named in the
output); exit 2 means the harness could not run (no QEMU/OVMF, ESP not
staged). The serial log is `racos-racostest.log`.

`python3 scripts/guest-suite.py --self-test` grades fixture logs without
QEMU — including the log shape the previous marker-only CI check accepted
— and `--no-disk --no-net` reproduces the old CI machine, which the driver
must refuse.

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
