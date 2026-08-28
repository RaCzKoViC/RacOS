# RacOS - drive the in-guest `racos-test` suite (local equivalent of the CI
# `interactive-smoke` job).
#
# Boots RacOS headless, waits for the racsh prompt, types `racos-test`, and
# reports both the suite's own tally and the named smoke markers.
#
# CI drives racsh over a TCP-serial chardev, but that chardev does not bind on
# the Windows QEMU build (see scripts/run-demo-tests.ps1), so this uses the same
# stdio-piped-child approach the project's demo driver uses.
#
# Usage:
#   powershell -File scripts/test-racos-test.ps1 [-BootWaitMax 90] [-TestBudget 200]
#
# Exit: 0 = suite reported 0 failures and every CI assertion held, 1 otherwise.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252 and a stray non-ASCII
# character fails the file with "The string is missing the terminator".

param(
    [int]$BootWaitMax = 90,
    [int]$TestBudget  = 200
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")
$qemu     = Find-QemuPaths
$QemuExe  = $qemu.Exe
$OvmfCode = $qemu.Ovmf

$EspDir    = Join-Path $Root "esp"
$DiskPath  = Join-Path $Root "racos-racostest-disk.img"
$SerialLog = Join-Path $Root "racos-racostest.log"

if (-not (Test-Path (Join-Path $EspDir "racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    Write-Host "  Copy-Item target\x86_64-unknown-none\debug\racore esp\racore.elf -Force"
    exit 2
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $SerialLog) { Remove-Item $SerialLog -Force }
if (Test-Path $DiskPath)  { Remove-Item $DiskPath -Force }
$fs = [System.IO.File]::Create($DiskPath); $fs.SetLength(16MB); $fs.Close()

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
    "-boot",    "menu=on",
    "-drive",   "if=ide,format=raw,file=fat:rw:$EspDir",
    "-drive",   "id=disk0,file=$DiskPath,if=none,format=raw,cache=writethrough",
    "-device",  "ich9-ahci,id=ahci",
    "-device",  "ide-hd,drive=disk0,bus=ahci.0",
    "-netdev",  "user,id=net0",
    "-device",  "virtio-net-pci,netdev=net0,romfile=,disable-modern=on,disable-legacy=off",
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

$qemuProc = [System.Diagnostics.Process]::Start($psi)
Write-Host "QEMU PID=$($qemuProc.Id)"

$enc       = [System.Text.Encoding]::ASCII
$logFs     = [System.IO.File]::Open($SerialLog, [System.IO.FileMode]::Create)
$outStream = $qemuProc.StandardOutput.BaseStream
$inWriter  = $qemuProc.StandardInput

$script:accum       = ""
$script:rx          = New-Object byte[] 8192
$script:pendingRead = $null
$script:lastDataAt  = Get-Date

# Never block longer than the caller's budget. An outstanding BeginRead is
# carried across iterations rather than waited out unbounded: a blocking
# WaitOne() deadlocks the whole runner the moment the guest panics and goes
# silent, and it holds an exclusive lock on the serial log so the evidence
# cannot even be read back.
function Pump($seconds, $stopPattern) {
    $end = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $end) {
        if ($null -eq $script:pendingRead) {
            $script:pendingRead = $outStream.BeginRead($script:rx, 0, $script:rx.Length, $null, $null)
        }
        if ($script:pendingRead.AsyncWaitHandle.WaitOne(300)) {
            try { $n = $outStream.EndRead($script:pendingRead) }
            catch { $script:pendingRead = $null; return $false }
            $script:pendingRead = $null
            if ($n -gt 0) {
                $logFs.Write($script:rx, 0, $n); $logFs.Flush()
                $script:accum += $enc.GetString($script:rx, 0, $n)
                $script:lastDataAt = Get-Date
                if ($stopPattern -and $script:accum -match $stopPattern) { return $true }
            }
        }
    }
    return $false
}

function Test-GuestSilent($quietSec) {
    return ((Get-Date) - $script:lastDataAt).TotalSeconds -ge $quietSec
}

function Send-Line($line) {
    foreach ($ch in ($line + "`n").ToCharArray()) {
        $inWriter.Write($ch); $inWriter.Flush(); Start-Sleep -Milliseconds 30
    }
}

Write-Host "Waiting for racsh banner (max ${BootWaitMax}s)..."
if (-not (Pump $BootWaitMax "racsh 0\.1\.0")) {
    Write-Host "FAIL: guest never reached racsh" -ForegroundColor Red
    $logFs.Close(); try { $qemuProc.Kill() } catch {}
    exit 2
}
Write-Host "racsh up."

# A couple of basic shell interactions first, mirroring the CI assertions.
Pump 2 $null | Out-Null
foreach ($c in @('pwd', 'echo ci-smoke-ok', 'cat /proc/version')) {
    Send-Line $c
    Pump 2.0 $null | Out-Null
}

Write-Host "Running racos-test (budget ${TestBudget}s)..."
Send-Line 'racos-test'
$deadline = (Get-Date).AddSeconds($TestBudget)
while ((Get-Date) -lt $deadline) {
    if (Pump 5 "=== Results:") { break }
    if (Test-GuestSilent 25) {
        Write-Host "  guest silent for 25s - treating as hang/panic" -ForegroundColor Yellow
        break
    }
}
Pump 5 $null | Out-Null

$logFs.Close(); $inWriter.Close()
if (-not $qemuProc.HasExited) { try { $qemuProc.Kill() } catch {} }
Start-Sleep -Milliseconds 500

$log = (Get-Content $SerialLog -Raw) -replace "`r", ""

if ($log -match "KERNEL PANIC") {
    Write-Host ""
    Write-Host "!!! KERNEL PANIC in this run !!!" -ForegroundColor Red
    ($log -split "`n" | Select-String -Pattern "KERNEL PANIC" -Context 0,4) | ForEach-Object { $_.ToString() }
}

Write-Host ""
if ($log -match '=== Results: (\d+) passed, (\d+) failed ===') {
    Write-Host ("racos-test tally: " + $Matches[1] + " passed, " + $Matches[2] + " failed")
    $suiteFailed = [int]$Matches[2]
    $suiteRan = $true
} else {
    Write-Host "racos-test tally: NOT REACHED (run did not complete)" -ForegroundColor Yellow
    $suiteFailed = 1
    $suiteRan = $false
}

# The assertions the CI job greps for.
$checks = @(
    @{ Name = "racsh prompt present";   Pattern = 'racsh\$' },
    @{ Name = "pwd returned /";         Pattern = '(?m)^/$' },
    @{ Name = "echo roundtrip";         Pattern = 'ci-smoke-ok' },
    @{ Name = "/proc/version readable"; Pattern = 'RacOS version' },
    @{ Name = "signal default action";  Pattern = 'PHASE21-SIGNAL-TERM-OK' },
    @{ Name = "SIGCHLD wait";           Pattern = 'PHASE21-SIGCHLD-WAIT-OK' },
    @{ Name = "exec-loop cleanup";      Pattern = 'PHASE21-EXEC-LOOP-OK' },
    @{ Name = "poll timeout";           Pattern = 'POLL-TIMEOUT-OK' },
    @{ Name = "TTY ioctl state";        Pattern = 'TTY-IOCTL-OK' }
)

Write-Host ""
Write-Host "=== CI interactive-smoke assertions ==="
$fail = 0
foreach ($c in $checks) {
    if ($log -match $c.Pattern) { Write-Host ("  PASS  " + $c.Name) }
    else { Write-Host ("  FAIL  " + $c.Name); $fail++ }
}

$markers = [regex]::Matches($log, '(?m)^([A-Z0-9][A-Z0-9-]*-(?:OK|FAIL))\b') |
           ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
Write-Host ""
Write-Host "=== racos-test markers observed ==="
foreach ($m in $markers) { Write-Host ("  " + $m) }
$okCount   = ($markers | Where-Object { $_ -like '*-OK' }).Count
$failCount = ($markers | Where-Object { $_ -like '*-FAIL' }).Count
Write-Host ""
Write-Host ("markers OK=$okCount  FAIL=$failCount")
Write-Host ("Full log: " + $SerialLog + " [" + (Get-Item $SerialLog).Length + " bytes]")

Write-Host ""
# The tally is the real verdict: the marker list alone would stay green while
# individual assertions inside a group fail.
if ($fail -eq 0 -and $suiteRan -and $suiteFailed -eq 0 -and $log -notmatch "KERNEL PANIC") {
    Write-Host "RACOS-TEST PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host "RACOS-TEST FAIL" -ForegroundColor Red
    exit 1
}
