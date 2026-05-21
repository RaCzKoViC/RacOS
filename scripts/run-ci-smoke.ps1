# RacOS - CI smoke runner
#
# Boots a ci-smoke-enabled kernel under QEMU with isa-debug-exit wired up
# and reports the QEMU exit code. Success is exit 33 (kernel writes 0x10
# to port 0xf4, QEMU maps that to (0x10 << 1) | 1). Failure or timeout
# returns non-zero.
#
# Usage: powershell -File scripts/run-ci-smoke.ps1 [-Linux]
#   -Linux: use Linux/CI QEMU + OVMF paths instead of the local D:\qemu install

param(
    [switch]$Linux,
    [int]$TimeoutSec = 60,
    [int]$Smp = 1
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

if ($Linux) {
    $QemuExe  = "qemu-system-x86_64"
    $OvmfCode = "/usr/share/OVMF/OVMF_CODE.fd"
    $EspArg   = "file=fat:rw:esp,format=raw"
} else {
    $QemuExe  = "D:\qemu\qemu-system-x86_64.exe"
    $OvmfCode = "D:\qemu\share\edk2-x86_64-code.fd"
    $EspArg   = "if=ide,format=raw,file=fat:rw:$Root\esp"
}

$LogPath = Join-Path $Root "smoke-stdout.log"
if (Test-Path $LogPath) { Remove-Item $LogPath -Force }

# Rebuild the kernel with the ci-smoke feature AND the static relocation
# model the bootloader requires. Without -C relocation-model=static the
# kernel ELF is PIE with dynamic relocations the bootloader doesn't apply,
# vtable calls go to garbage low memory, and the guest #UDs before
# kernel_main ever prints a line. Setting RUSTFLAGS in this script keeps
# the smoke self-contained.
$oldRustflags = $env:RUSTFLAGS
$env:RUSTFLAGS = "-C relocation-model=static -C link-arg=-no-pie"
Write-Host "Building kernel (ci-smoke + static relocation)..."
cargo build --package racore --target x86_64-unknown-none --features ci-smoke
$buildExit = $LASTEXITCODE
$env:RUSTFLAGS = $oldRustflags
if ($buildExit -ne 0) {
    Write-Host ("Kernel build failed with exit " + $buildExit)
    exit $buildExit
}

# Stage the freshly-built kernel into esp/ so the bootloader picks it up.
$TargetDir = & cargo metadata --format-version 1 --no-deps --quiet 2>$null |
    ConvertFrom-Json | Select-Object -ExpandProperty target_directory
$KernelSrc = Join-Path $TargetDir "x86_64-unknown-none\debug\racore"
$KernelDst = Join-Path $Root "esp\racore.elf"
if (Test-Path $KernelSrc) {
    Copy-Item $KernelSrc $KernelDst -Force
    Write-Host ("Staged kernel: " + $KernelSrc + " -> " + $KernelDst)
}

$QemuArgs = @(
    "-machine", "q35",
    "-accel",   "tcg",
    "-cpu",     "qemu64",
    "-smp",     "$Smp",
    "-m",       "512M",
    "-drive",   "if=pflash,format=raw,readonly=on,file=$OvmfCode",
    "-boot",    "menu=on",
    "-drive",   $EspArg,
    "-serial",  "file:$LogPath",
    "-monitor", "null",
    "-display", "none",
    "-no-reboot",
    "-device",  "isa-debug-exit,iobase=0xf4,iosize=0x04"
)

Write-Host ("Launching QEMU (ci-smoke, " + $TimeoutSec + "s budget)...")

# Use the call operator (&) so QEMU receives each argument as a separate
# argv entry. Start-Process -ArgumentList silently re-joins on spaces and
# breaks any path containing a space (e.g. "D:\OS project\esp").
# Run QEMU as a background job so we can enforce the timeout in PowerShell.
$job = Start-Job -ScriptBlock {
    param($exe, $arglist)
    & $exe @arglist
    $LASTEXITCODE
} -ArgumentList $QemuExe,$QemuArgs

$finished = Wait-Job -Job $job -Timeout $TimeoutSec
if (-not $finished) {
    Write-Host ("TIMEOUT after " + $TimeoutSec + "s, killing QEMU")
    Stop-Job -Job $job
    Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
    Remove-Job -Job $job -Force
    if (Test-Path $LogPath) {
        Write-Host "--- serial log (tail 60) ---"
        Get-Content $LogPath -Tail 60
    }
    exit 124
}

$jobOutput = Receive-Job -Job $job
Remove-Job -Job $job
$exitCode = ($jobOutput | Select-Object -Last 1)
if ($null -eq $exitCode) { $exitCode = -1 }
Write-Host ("QEMU exited with code " + $exitCode + " (expect 33 for success)")

if (Test-Path $LogPath) {
    Write-Host "--- serial log (tail 60) ---"
    Get-Content $LogPath -Tail 60
}

# isa-debug-exit: success = write 0x10 -> (0x10 << 1) | 1 = 33
#                 failure = write 0x11 -> (0x11 << 1) | 1 = 35
if ($exitCode -eq 33) {
    Write-Host "SMOKE PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host ("SMOKE FAIL (exit " + $exitCode + ")") -ForegroundColor Red
    exit 1
}
