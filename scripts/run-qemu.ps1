# RacOS — QEMU Boot Script
# Boots the RacOS image using OVMF (UEFI) firmware.
#
# Usage:   powershell -File scripts/run-qemu.ps1
# Options: -Debug    — enable serial output to stdio and GDB stub on port 1234

param(
    [switch]$Debug
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Definition)
Set-Location $ProjectRoot

# ── Paths ──
$QemuExe   = "D:\qemu\qemu-system-x86_64.exe"
$OvmfCode  = "D:\qemu\share\edk2-x86_64-code.fd"
$EspDir    = Join-Path $ProjectRoot "esp"
$KernelElf = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\debug\racore"

# Build kernel with static relocation model for direct physical entry jumping
$oldRustflags = $env:RUSTFLAGS
$env:RUSTFLAGS = "-C relocation-model=static -C link-arg=-no-pie"
cargo build --package racore --target x86_64-unknown-none
if ($LASTEXITCODE -ne 0) { throw "Kernel build failed" }
$env:RUSTFLAGS = $oldRustflags

# ── Validate prerequisites ──
if (-not (Test-Path $QemuExe))  { throw "QEMU not found at $QemuExe" }
if (-not (Test-Path $OvmfCode)) { throw "OVMF firmware not found at $OvmfCode" }
if (-not (Test-Path "$EspDir\EFI\BOOT\BOOTX64.EFI")) {
    throw "Bootloader not found. Run build-image.ps1 first."
}

# Copy kernel to ESP
$KernelDest = Join-Path $EspDir "racore.elf"
if (Test-Path $KernelElf) {
    Copy-Item $KernelElf $KernelDest -Force
    Write-Host "Copied kernel to ESP"
} else {
    if (-not (Test-Path $KernelDest)) {
        throw "Kernel not found. Run build-image.ps1 first."
    }
}

# ── Build QEMU command ──
$QemuArgs = @(
    "-machine", "q35"
    "-cpu", "qemu64"
    "-m", "256M"
    "-drive", "if=pflash,format=raw,readonly=on,file=$OvmfCode"
    "-boot", "menu=on"
    "-drive", "if=ide,format=raw,file=fat:rw:$EspDir"
    "-serial", "stdio"
    "-vga", "std"
    "-no-reboot"
    "-no-shutdown"
)

if ($Debug) {
    $QemuArgs += @("-s", "-S")
    Write-Host "Debug mode: GDB stub on localhost:1234, waiting for connection..."
}

Write-Host ""
Write-Host "=== Launching RacOS in QEMU ==="
Write-Host "  OVMF:  $OvmfCode"
Write-Host "  ESP:   $EspDir"
Write-Host "  RAM:   256M"
Write-Host ""

& $QemuExe @QemuArgs
