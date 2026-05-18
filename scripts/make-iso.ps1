# RacOS — Build Bootable ISO Image
#
# Creates a bootable ISO from the ESP directory using xorriso.
# Requires xorriso to be installed (or mkisofs).
#
# Usage: powershell -File scripts/make-iso.ps1 [-Release]

param(
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

$Profile = if ($Release) { "release" } else { "dev" }
$ProfileDir = if ($Release) { "release" } else { "debug" }

Write-Host "=== RacOS ISO Builder ===" -ForegroundColor Cyan
Write-Host "Profile: $Profile"

$EspDir = "$Root\esp"
$IsoPath = "$Root\racos-$Profile.iso"

if (-not (Test-Path $EspDir)) {
    throw "ESP directory not found: $EspDir. Run build-image.ps1 first."
}

# Use xorriso if available, else mkisofs
$xorriso = Get-Command xorriso -ErrorAction SilentlyContinue
if ($xorriso) {
    Write-Host "Using xorriso..."
    & xorriso -as mkisofs `
        -o $IsoPath `
        -b boot/limine-cd.bin `
        -no-emul-boot `
        -boot-load-size 4 `
        -boot-info-table `
        --efi-boot boot/limine-eltorito-efi.bin `
        -efi-boot-part --efi-boot-image `
        --protective-msdos-label `
        $EspDir
} else {
    $mkisofs = Get-Command mkisofs -ErrorAction SilentlyContinue
    if ($mkisofs) {
        Write-Host "Using mkisofs..."
        & mkisofs -o $IsoPath `
            -b boot/limine-cd.bin `
            -no-emul-boot `
            -boot-load-size 4 `
            -boot-info-table `
            --efi-boot boot/limine-eltorito-efi.bin `
            -efi-boot-part --efi-boot-image `
            --protective-msdos-label `
            $EspDir
    } else {
        throw "Neither xorriso nor mkisofs found. Install one to create ISO."
    }
}

if ($LASTEXITCODE -ne 0) { throw "ISO creation failed" }

Write-Host ""
Write-Host "=== ISO created ===" -ForegroundColor Green
Write-Host "Path: $IsoPath"
$size = (Get-Item $IsoPath).Length
Write-Host "Size: $([math]::Round($size / 1MB, 2)) MB"