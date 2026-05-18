# RacOS - Build UEFI-bootable ESP directory
#
# Creates the EFI System Partition directory structure for QEMU.
# QEMU can boot directly from a directory using: -drive file=fat:rw:esp
#
# Usage: powershell -File scripts/make-image.ps1 [-Profile debug|release]
#
# Prerequisites:
#   - cargo (nightly)
#   - OVMF firmware at tools\OVMF_CODE.fd (download from https://www.tianocore.org/)
#
# After running, boot with:
#   just run-uefi   (or see justfile for the full QEMU command)

param(
    [string]$Profile   = "debug",
    [string]$TargetDir = "C:\Users\Maciej\RacOS-target",
    [string]$EspDir    = "esp"
)

$ErrorActionPreference = "Stop"

Write-Host "=== RacOS Image Builder ===" -ForegroundColor Cyan

# ── Step 1: Build kernel ──────────────────────────────────────────────────────
Write-Host "[1/5] Building kernel (x86_64-unknown-none)..."
cargo build --package racore --target x86_64-unknown-none
if ($LASTEXITCODE -ne 0) { throw "Kernel build failed" }

# ── Step 2: Build UEFI bootloader ────────────────────────────────────────────
Write-Host "[2/5] Building bootloader (x86_64-unknown-uefi)..."
cargo build --package racos-boot --target x86_64-unknown-uefi
if ($LASTEXITCODE -ne 0) { throw "Bootloader build failed" }

# ── Step 3: Create ESP directory structure ────────────────────────────────────
Write-Host "[3/5] Creating ESP directory structure..."
$EfiBootDir = Join-Path $EspDir "EFI\BOOT"
New-Item -ItemType Directory -Force $EfiBootDir | Out-Null

$BootloaderSrc = Join-Path $TargetDir "x86_64-unknown-uefi\$Profile\racos-boot.efi"
$BootloaderDst = Join-Path $EfiBootDir "BOOTX64.EFI"
if (-not (Test-Path $BootloaderSrc)) {
    throw "Bootloader not found: $BootloaderSrc"
}
Copy-Item $BootloaderSrc $BootloaderDst -Force
Write-Host "  Bootloader -> $BootloaderDst"

$KernelSrc = Join-Path $TargetDir "x86_64-unknown-none\$Profile\racore"
$KernelDst = Join-Path $EspDir "racore.elf"
if (-not (Test-Path $KernelSrc)) {
    throw "Kernel not found: $KernelSrc"
}
Copy-Item $KernelSrc $KernelDst -Force
Write-Host "  Kernel     -> $KernelDst"

# ── Step 4: Pack initramfs ────────────────────────────────────────────────────
Write-Host "[4/5] Packing initramfs..."
$InitramfsOutput = Join-Path $EspDir "initramfs.img"
& "$PSScriptRoot\pack-initramfs.ps1" -RootDir "initramfs-root" -Output $InitramfsOutput
if ($LASTEXITCODE -ne 0) { throw "Initramfs packing failed" }

# ── Step 5: Check for OVMF ───────────────────────────────────────────────────
Write-Host "[5/5] Checking OVMF firmware..."
$OvmfPath = "tools\OVMF_CODE.fd"
if (-not (Test-Path $OvmfPath)) {
    Write-Host "  WARNING: OVMF not found at '$OvmfPath'" -ForegroundColor Yellow
    Write-Host "  Download from: https://www.tianocore.org/ovmf/" -ForegroundColor Yellow
    Write-Host "  Or install QEMU for Windows (includes OVMF)" -ForegroundColor Yellow
    Write-Host "  Place OVMF_CODE.fd in the 'tools/' directory" -ForegroundColor Yellow
} else {
    Write-Host "  OVMF found: $OvmfPath"
}

Write-Host ""
Write-Host "=== Build complete ===" -ForegroundColor Green
Write-Host "ESP directory: $EspDir"
Write-Host "Run with: just run-uefi"
