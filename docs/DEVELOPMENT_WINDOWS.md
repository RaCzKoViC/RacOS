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
