# RacOS - drive the in-guest `racos-test` suite (local equivalent of the CI
# `guest-suite` job).
#
# Thin wrapper: the machine definition, the readiness logic and the verdict
# all live in scripts/guest-suite.py, which is the very script the CI job
# runs. This file only resolves the Windows QEMU/OVMF paths and forwards the
# budgets, so the local gate and CI cannot drift apart again.
#
# Usage:
#   powershell -File scripts/test-racos-test.ps1 [-BootWaitMax 180] [-TestBudget 600] [-Smp 2]
#
# Exit: 0 = suite reported 0 failures, exit status 0 and every device was
# present; 1 = verdict FAIL or a required device missing; 2 = could not run.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252 and a stray non-ASCII
# character fails the file with "The string is missing the terminator".

param(
    [int]$BootWaitMax = 180,
    [int]$TestBudget  = 600,
    [int]$Smp         = 2
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")
$qemu = Find-QemuPaths

# One QEMU per disk image: a leftover instance from an earlier gate still
# holds racos-racostest-disk.img and the driver refuses to recreate it.
Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

& python (Join-Path $PSScriptRoot "guest-suite.py") `
    --qemu $qemu.Exe --ovmf $qemu.Ovmf `
    --smp $Smp --boot-timeout $BootWaitMax --suite-timeout $TestBudget
exit $LASTEXITCODE
