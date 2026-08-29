# RacOS - run every CI gate locally, in CI's dependency order.
#
# Mirrors .github/workflows/ci.yml so a full check can be run before pushing:
#
#   1 build            kernel + bootloader + userland
#   2 host-tests       cargo test (racsh, rpkg, rapt, init, racterm)
#   3 fmt              cargo fmt --check
#   4 clippy           kernel lints                       (advisory)
#   5 unsafe-safety    every unsafe block has a SAFETY note
#   6 kernel-smoke     in-kernel assertions via isa-debug-exit, AHCI + SMP
#   7 boot-smoke       two boots, on-disk counter survives the reboot
#   8 racos-test       130 assertions driven through racsh in a live guest
#   9 usb-boot         boot from a real MBR+FAT32 image over USB (3.4)
#  10 milestone-v03    two boots: history + installed rpkg survive (3.5)
#  11 graphics         framebuffer claimed + screendump pixel diversity (4.4)
#
# Usage:
#   powershell -File scripts/run-all-gates.ps1 [-SkipQemu] [-Smp 4]
#
# Exit: 0 = every gate passed, 1 = at least one failed.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252 and a stray non-ASCII
# character fails the file with "The string is missing the terminator".

param(
    [switch]$SkipQemu,   # gates 1-5 only (no emulator needed)
    [int]$Smp = 4
)

$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

# Kernel and userland are freestanding targets: without a static relocation
# model cargo emits a PIE whose relative relocations the bootloader never
# applies, and the kernel triple-faults before its first serial write.
$KernelFlags = "-C relocation-model=static -C link-arg=-no-pie"

# Gate 5 runs a shell script, and a bare `bash` is the wrong one twice over on
# a stock Windows box:
#
#   - PATH resolves it to WSL's C:\Windows\System32\bash.exe, which refuses a
#     CRLF script outright ("$'\r': command not found") and the gate hard-fails
#     for a reason that has nothing to do with the code.
#   - Git's inner usr\bin\bash.exe runs the script but without coreutils on
#     PATH, so `grep` is not found, the scan matches nothing, and the lint
#     reports "scanned 0 unsafe blocks; 0 missing" and exits 0. A gate that
#     enforces SAFETY annotations passing without reading a line is worse than
#     one that fails.
#
# Git's bin\bash.exe is the wrapper that sets up its own environment, so name
# it explicitly rather than trusting PATH. The script itself now also refuses
# to report success when it scanned nothing, so this is belt and braces.
function Find-Bash {
    $gitCmd = Get-Command git -ErrorAction SilentlyContinue
    if ($gitCmd) {
        $gitRoot = Split-Path -Parent (Split-Path -Parent $gitCmd.Source)
        $candidate = Join-Path $gitRoot "bin\bash.exe"
        if (Test-Path $candidate) { return $candidate }
    }
    foreach ($p in @("C:\Program Files\Git\bin\bash.exe", "C:\Git\bin\bash.exe")) {
        if (Test-Path $p) { return $p }
    }
    return "bash"
}
$BashExe = Find-Bash

$results = [ordered]@{}
function Record($name, $ok, $detail) {
    $results[$name] = @{ Ok = $ok; Detail = $detail }
    $tag = if ($ok) { "PASS" } else { "FAIL" }
    $color = if ($ok) { "Green" } else { "Red" }
    Write-Host ("  " + $tag + "  " + $name + $(if ($detail) { " - $detail" } else { "" })) -ForegroundColor $color
}

$started = Get-Date

# ---- 1. build -----------------------------------------------------------
Write-Host ""
Write-Host "[1/11] build" -ForegroundColor Cyan
$env:RUSTFLAGS = $KernelFlags
cargo build --package racore --target x86_64-unknown-none 2>&1 | Out-Null
$kernelOk = ($LASTEXITCODE -eq 0)
cargo build --package racos-boot --target x86_64-unknown-uefi 2>&1 | Out-Null
$bootOk = ($LASTEXITCODE -eq 0)
Record "build" ($kernelOk -and $bootOk) "kernel + bootloader"

# ---- 2. host tests ------------------------------------------------------
Write-Host ""
Write-Host "[2/11] host tests" -ForegroundColor Cyan
$env:RUSTFLAGS = ""   # these are host binaries: the kernel flags break them
$out = cargo test -p racsh -p rpkg -p rapt -p init -p racterm 2>&1 | Out-String
$passed = 0; $failed = 0
foreach ($m in [regex]::Matches($out, 'test result: (\w+)\. (\d+) passed; (\d+) failed')) {
    $passed += [int]$m.Groups[2].Value
    $failed += [int]$m.Groups[3].Value
}
Record "host-tests" (($failed -eq 0) -and ($passed -gt 0)) "$passed passed, $failed failed"

# ---- 3. rustfmt ---------------------------------------------------------
Write-Host ""
Write-Host "[3/11] rustfmt" -ForegroundColor Cyan
cargo fmt --all -- --check 2>&1 | Out-Null
Record "fmt" ($LASTEXITCODE -eq 0) "no drift"

# ---- 4. clippy (advisory) -----------------------------------------------
Write-Host ""
Write-Host "[4/11] clippy (advisory)" -ForegroundColor Cyan
$env:RUSTFLAGS = $KernelFlags
$c = cargo clippy --package racore --target x86_64-unknown-none -- -W clippy::all 2>&1 | Out-String
$errs  = ([regex]::Matches($c, '(?m)^error')).Count
$warns = ([regex]::Matches($c, '(?m)^warning: ')).Count
Record "clippy" ($errs -eq 0) "$errs errors, $warns warnings"

# ---- 5. unsafe-safety lint ----------------------------------------------
Write-Host ""
Write-Host "[5/11] unsafe-safety lint" -ForegroundColor Cyan
$u = & $BashExe scripts/check-unsafe-safety.sh --strict 2>&1 | Out-String
$uOk = ($LASTEXITCODE -eq 0)
$uSummary = (($u -split "`n") | Where-Object { $_ -match 'scanned' } | Select-Object -First 1)
Record "unsafe-safety" $uOk ($uSummary -replace '\s+$', '')

if ($SkipQemu) {
    Write-Host ""
    Write-Host "-SkipQemu set: stopping after the host-side gates." -ForegroundColor Yellow
} else {
    # Gates 6-8 boot a real image, so stage the ESP. build-image.ps1 packs the
    # initramfs only; the kernel ELF has to be copied in separately.
    #
    # This has to be repeatable: run-ci-smoke.ps1 rebuilds the kernel with
    # --features ci-smoke and leaves THAT binary in esp/racore.elf. A ci-smoke
    # kernel runs its assertions and exits through isa-debug-exit instead of
    # booting to racsh, so gates 7 and 8 must re-stage the normal kernel first
    # or they boot the wrong one and fail for the wrong reason.
    function Stage-PlainKernel {
        $env:RUSTFLAGS = $KernelFlags
        cargo build --package racore --target x86_64-unknown-none 2>&1 | Out-Null
        Copy-Item (Join-Path $Root "target\x86_64-unknown-none\debug\racore") `
                  (Join-Path $Root "esp\racore.elf") -Force
        $env:RUSTFLAGS = ""
    }

    Write-Host ""
    Write-Host "staging ESP (initramfs + kernel)..." -ForegroundColor Cyan
    # RUSTFLAGS must be EMPTY here. build-image.ps1 sets the static/no-pie flags
    # itself for the kernel, then builds userland with "$OldRustFlags
    # -C debug-assertions=off" -- so anything left in RUSTFLAGS is inherited by
    # the userland build. Building userland with the kernel's
    # -C relocation-model=static -C link-arg=-no-pie strips the RELATIVE
    # relocations the kernel ELF loader applies: /sbin/init then loads, spawns
    # as PID 100, and dies before its first write, so the guest halts after
    # flushd with no init and no shell.
    $env:RUSTFLAGS = ""
    powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "build-image.ps1") 2>&1 |
        Select-String -Pattern "Done:|Build complete|^error" | ForEach-Object { Write-Host ("  " + $_) }
    Stage-PlainKernel

    # ---- 6. kernel smoke ------------------------------------------------
    Write-Host ""
    Write-Host "[6/11] kernel smoke (QEMU, isa-debug-exit)" -ForegroundColor Cyan
    $s = powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "run-ci-smoke.ps1") `
            -TimeoutSec 150 -Disk -Smp $Smp 2>&1 | Out-String
    Record "kernel-smoke" ($s -match "SMOKE PASS") "exit 33, AHCI + SMP $Smp"

    # ---- 7. persistence -------------------------------------------------
    Write-Host ""
    Write-Host "[7/11] persistence (two boots)" -ForegroundColor Cyan
    Stage-PlainKernel   # undo the ci-smoke kernel gate 6 left in the ESP
    $b = powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "test-persistence.ps1") `
            -BootSeconds 45 -Smp $Smp 2>&1 | Out-String
    $bDetail = ([regex]::Match($b, 'BOOT-SMOKE PASS \((\d+/\d+)\)')).Groups[1].Value
    Record "boot-smoke" ($b -match "BOOT-SMOKE PASS") $(if ($bDetail) { "$bDetail, counter survived reboot" } else { "" })

    # ---- 8. racos-test --------------------------------------------------
    Write-Host ""
    Write-Host "[8/11] racos-test (in-guest suite)" -ForegroundColor Cyan
    Stage-PlainKernel
    $t = powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "test-racos-test.ps1") `
            -BootWaitMax 90 -TestBudget 200 2>&1 | Out-String
    $tally = [regex]::Match($t, 'racos-test tally: (\d+) passed, (\d+) failed')
    $mk    = [regex]::Match($t, 'markers OK=(\d+)\s+FAIL=(\d+)')
    if ($t -match "KERNEL PANIC") { Write-Host "  (kernel panic observed in this run)" -ForegroundColor Red }
    $tDetail = if ($tally.Success) {
        $tally.Groups[1].Value + " passed, " + $tally.Groups[2].Value + " failed; markers " + $mk.Groups[1].Value
    } else { "suite did not complete" }
    Record "racos-test" ($t -match "RACOS-TEST PASS") $tDetail

    # ---- 9. USB boot (ROADMAP 3.4) --------------------------------------
    Write-Host ""
    Write-Host "[9/11] usb-boot (real ESP image over USB mass storage)" -ForegroundColor Cyan
    $u9 = powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "test-usb-boot.ps1") `
            -BootWaitMax 120 2>&1 | Out-String
    Record "usb-boot" ($u9 -match "USB-BOOT PASS") "MBR+FAT32 image, XHCI"

    # ---- 10. v0.3 milestone (ROADMAP 3.5) -------------------------------
    Write-Host ""
    Write-Host "[10/11] milestone-v03 (history + rpkg survive a reboot)" -ForegroundColor Cyan
    $m = powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "test-milestone-v03.ps1") `
            -BootWaitMax 90 2>&1 | Out-String
    $mDetail = if ($m -match "MILESTONE-V0\.3-OK") { "guest printed MILESTONE-V0.3-OK" } else { "marker missing" }
    Record "milestone-v03" ($m -match "MILESTONE-V0\.3 SMOKE PASS") $mDetail

    # ---- 11. graphics smoke (ROADMAP 4.4) -------------------------------
    Write-Host ""
    Write-Host "[11/11] graphics (claim line + QMP screendump)" -ForegroundColor Cyan
    $g = powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "test-graphics.ps1") `
            -BootWaitMax 90 2>&1 | Out-String
    $gDetail = if ($g -match "screendump has (\d+) distinct") { $Matches[1] + " distinct pixel values" } else { "no dump" }
    Record "graphics" ($g -match "GRAPHICS-SMOKE PASS") $gDetail
}

# ---- summary ------------------------------------------------------------
$elapsed = [int]((Get-Date) - $started).TotalSeconds
$failedGates = @($results.Keys | Where-Object { -not $results[$_].Ok })

Write-Host ""
Write-Host "=========================================="
Write-Host ("Gates run: " + $results.Count + "   Elapsed: ${elapsed}s")
if ($failedGates.Count -eq 0) {
    Write-Host "ALL GATES PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host ("FAILED: " + ($failedGates -join ", ")) -ForegroundColor Red
    exit 1
}
