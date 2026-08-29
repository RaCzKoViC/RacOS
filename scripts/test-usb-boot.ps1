# RacOS - boot from a real ESP disk image over USB mass storage (ROADMAP 3.4).
#
# Everything else in this repo boots via QEMU's `fat:rw:esp` auto-mode, which
# synthesises a FAT view of a host directory in RAM. Real media does not work
# that way: a USB stick is a block device with a partition table and a real
# FAT32 filesystem, discovered by the firmware over a real bus. This smoke
# builds that image with scripts/make-esp-image.py, attaches it as a USB
# mass-storage device on an XHCI controller, and requires the whole path -
# OVMF USB enumeration, partition parse, FAT32 driver, \EFI\BOOT\BOOTX64.EFI
# fallback, our bootloader reading racore.elf + initramfs.img over USB - to
# produce a racsh prompt.
#
# The same image written raw to a stick is the physical-hardware boot path;
# docs/BOOT-MEDIA.md documents that procedure and its caveats.
#
# Usage:
#   powershell -File scripts/test-usb-boot.ps1 [-BootWaitMax 120]
#
# Exit: 0 = racsh came up from USB, 1 = it did not.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252 and a stray
# non-ASCII character fails the file with "The string is missing the
# terminator".

param([int]$BootWaitMax = 120)

$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")
$qemu     = Find-QemuPaths
$QemuExe  = $qemu.Exe
$OvmfCode = $qemu.Ovmf

$RootQemu = Resolve-SpacelessPath $Root
$ImgPath  = Join-Path $RootQemu "racos-esp.img"
$ImgReal  = Join-Path $Root "racos-esp.img"
$DiskPath = Join-Path $RootQemu "racos-usb-sata.img"
$DiskReal = Join-Path $Root "racos-usb-sata.img"

if (-not (Test-Path (Join-Path $Root "esp\racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    exit 2
}

Write-Host "[1/2] building the ESP disk image..." -ForegroundColor Cyan
python (Join-Path $Root "scripts\make-esp-image.py") (Join-Path $Root "esp") $ImgReal
if ($LASTEXITCODE -ne 0) {
    Write-Host "FAIL: make-esp-image.py failed" -ForegroundColor Red
    exit 1
}

# A SATA disk rides along so the persistent layout has somewhere to live -
# same as every other smoke. The USB stick is the *boot* medium; the kernel
# has no USB stack and never sees it again after the bootloader hands over.
Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $DiskReal) { Remove-Item $DiskReal -Force }
$fs = [System.IO.File]::Create($DiskReal); $fs.SetLength(16MB); $fs.Close()

Write-Host ""
Write-Host "[2/2] booting from USB mass storage (no fat:rw fallback attached)" -ForegroundColor Cyan

function Quote-Arg($s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

$argList = @(
    "-machine", "q35",
    "-accel",   "tcg",
    "-cpu",     "qemu64,+smep,+smap",
    "-smp",     "2",
    "-m",       "512M",
    "-drive",   "if=pflash,format=raw,readonly=on,file=$OvmfCode",
    "-device",  "qemu-xhci,id=xhci",
    "-drive",   "id=usbstick,if=none,format=raw,file=$ImgPath",
    "-device",  "usb-storage,bus=xhci.0,drive=usbstick",
    "-drive",   "id=disk0,file=$DiskPath,if=none,format=raw,cache=writethrough",
    "-device",  "ich9-ahci,id=ahci",
    "-device",  "ide-hd,drive=disk0,bus=ahci.0",
    "-serial",  "stdio",
    "-display", "none",
    "-no-reboot"
)

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName  = $QemuExe
$psi.Arguments = ($argList | ForEach-Object { Quote-Arg $_ }) -join ' '
$psi.RedirectStandardInput  = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError  = $true
$psi.UseShellExecute = $false
$psi.CreateNoWindow  = $true
$p = [System.Diagnostics.Process]::Start($psi)
Write-Host "QEMU PID=$($p.Id)"

$text = ""
$buf = New-Object byte[] 8192
$pending = $null
$end = (Get-Date).AddSeconds($BootWaitMax)
while ((Get-Date) -lt $end) {
    if ($null -eq $pending) {
        $pending = $p.StandardOutput.BaseStream.BeginRead($buf, 0, $buf.Length, $null, $null)
    }
    if ($pending.AsyncWaitHandle.WaitOne(250)) {
        try { $n = $p.StandardOutput.BaseStream.EndRead($pending) } catch { break }
        $pending = $null
        if ($n -gt 0) { $text += [System.Text.Encoding]::ASCII.GetString($buf, 0, $n) }
    }
    if ($text -match 'racsh 0\.1\.0') { break }
    if ($p.HasExited) { break }
}
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force

$checks = @(
    @{ Name = "bootloader loaded the kernel";      Pattern = "Loading kernel" },
    @{ Name = "initramfs found on the USB volume"; Pattern = "initramfs" },
    @{ Name = "kernel started";                    Pattern = "RACORE: RacOS kernel starting" },
    @{ Name = "persistent layout mounted";         Pattern = "/home is persistent" },
    @{ Name = "racsh prompt reached";              Pattern = "racsh 0\.1\.0" }
)

Write-Host ""
Write-Host "=== USB boot assertions ==="
$fail = 0
foreach ($c in $checks) {
    if ($text -match $c.Pattern) { Write-Host ("  PASS  " + $c.Name) }
    else { Write-Host ("  FAIL  " + $c.Name) -ForegroundColor Red; $fail++ }
}

Write-Host ""
if ($fail -eq 0) {
    Write-Host "USB-BOOT PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host "USB-BOOT FAIL ($fail)" -ForegroundColor Red
    exit 1
}
