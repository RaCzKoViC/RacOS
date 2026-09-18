# RacOS - kill the guest mid-write and check the filesystem survived it.
#
# This is the test the metadata journal exists for. Everything else about
# journaling can pass while the guarantee it sells is absent: the ordinary
# smokes shut the guest down cleanly, so they exercise the commit path and
# never the recovery path.
#
# Each iteration boots RacOS, starts a shell loop that does nothing but churn
# metadata (mkdir / create / extend / truncate / link / unlink / rmdir), lets
# it run, then kills QEMU outright - no sync, no shutdown, the power-cord
# case. The next boot has to come up with a filesystem that fsck calls
# clean, replaying a transaction if one was in flight.
#
# The disk is deliberately NOT recreated between iterations: damage that a
# single crash leaves behind is easy to miss, and damage that accumulates over
# several is not.
#
# Usage:
#   powershell -File scripts/test-crash-consistency.ps1 [-Iterations 4]
#
# Exit: 0 = every boot after a crash was consistent, 1 = at least one was not.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252 and a stray
# non-ASCII character fails the file with "The string is missing the
# terminator".

param(
    [int]$Iterations = 4,
    [int]$BootWaitMax = 90
)

$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")
$qemu     = Find-QemuPaths
$QemuExe  = $qemu.Exe
$OvmfCode = $qemu.Ovmf

$RootQemu  = Resolve-SpacelessPath $Root
$EspDir    = Join-Path $RootQemu "esp"
$DiskPath  = Join-Path $RootQemu "racos-crash-disk.img"

if (-not (Test-Path (Join-Path $Root "esp\racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    exit 2
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $DiskPath) { Remove-Item $DiskPath -Force }
$fs = [System.IO.File]::Create($DiskPath); $fs.SetLength(16MB); $fs.Close()

function Quote-Arg($s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

# One boot. Returns a hashtable with the serial text and the process, leaving
# the process running so the caller can kill it at a moment of its choosing.
function Start-Guest {
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
    return $p
}

# Read whatever the guest has produced, without ever blocking longer than
# $seconds. A blocking read deadlocks the whole runner the moment the guest
# panics and goes quiet, and it holds the stream so the evidence cannot be
# recovered either.
function Pump($proc, $state, $seconds, $stopPattern) {
    $end = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $end) {
        if ($null -eq $state.Pending) {
            $state.Pending = $proc.StandardOutput.BaseStream.BeginRead(
                $state.Buf, 0, $state.Buf.Length, $null, $null)
        }
        if ($state.Pending.AsyncWaitHandle.WaitOne(250)) {
            try { $n = $proc.StandardOutput.BaseStream.EndRead($state.Pending) }
            catch { $state.Pending = $null; return $false }
            $state.Pending = $null
            if ($n -gt 0) {
                $state.Text += [System.Text.Encoding]::ASCII.GetString($state.Buf, 0, $n)
            }
        }
        if ($stopPattern -and $state.Text -match $stopPattern) { return $true }
        if ($proc.HasExited) { return $false }
    }
    return $false
}

function New-PumpState {
    return @{ Text = ""; Buf = New-Object byte[] 8192; Pending = $null }
}

# Type a line into the guest one character at a time.
#
# Writing the whole line at once looks like it works and does not: the guest's
# serial input path drops most of it, the command never runs, and a crash test
# whose workload never started reports a beautifully clean filesystem. This
# mirrors Send-Line in test-racos-test.ps1, which is character-at-a-time for
# the same reason.
function Send-Line($proc, $line) {
    foreach ($ch in ($line + "`n").ToCharArray()) {
        $proc.StandardInput.Write($ch)
        $proc.StandardInput.Flush()
        Start-Sleep -Milliseconds 30
    }
}

$fail = 0
$results = @()

for ($i = 1; $i -le $Iterations; $i++) {
    # Vary how long the churn runs so the kill lands at a different point in
    # the write sequence each time. A fixed delay tests one instant over and
    # over and calls it four tests.
    $churnSeconds = 3 + ($i * 2)

    Write-Host ""
    Write-Host "--- iteration $i/$Iterations (churn ${churnSeconds}s, then hard kill) ---" -ForegroundColor Cyan

    $p = Start-Guest
    $st = New-PumpState
    $up = Pump $p $st $BootWaitMax 'racsh 0\.1\.0'
    if (-not $up) {
        Write-Host "  FAIL  guest did not reach racsh" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "no racsh banner" }
        $fail++
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        continue
    }

    # Metadata only: create, link, unlink, rmdir. No file data, because file
    # data is deliberately not journalled and losing the tail of a file that
    # was mid-write is the expected outcome, not a defect.
    # Prove the shell actually took input before trusting anything the crash
    # boot reports. A churn loop that never started would produce a perfectly
    # clean second boot and look like a pass.
    # Let the shell settle before typing at it.
    Pump $p $st 2 $null | Out-Null
    $st.Text = ""
    Send-Line $p "mkdir /mnt/churn-ran; echo CHURN-ARMED"
    # (?m)^ so the marker is matched as output at the start of a line, not
    # in the guest's echo of the command being typed: matching the echo
    # returned before the command ran, and the next line was then typed
    # into a shell still busy - its characters were dropped.
    $armed = Pump $p $st 20 '(?m)^CHURN-ARMED'
    if (-not $armed) {
        Write-Host "  FAIL  shell did not accept input; the churn never ran" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "churn never started" }
        $fail++
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        continue
    }

    # The seed is three blocks long so that `echo x > f` over it is a
    # truncate that releases blocks - the transaction type added with
    # O_TRUNC support - not just a size change inside one block. Built once
    # per boot, outside the loop, so the loop body stays metadata-heavy.
    Send-Line $p "echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef > /mnt/seed1; cat /mnt/seed1 /mnt/seed1 /mnt/seed1 /mnt/seed1 > /mnt/seed2; cat /mnt/seed2 /mnt/seed2 /mnt/seed2 /mnt/seed2 > /mnt/seed1; cat /mnt/seed1 /mnt/seed2 > /mnt/seed; echo SEED-READY"
    # Same rule as CHURN-ARMED: a seed that never got built would turn the
    # truncating step into a plain create and the run would still look green.
    $seeded = Pump $p $st 40 '(?m)^SEED-READY'
    if (-not $seeded) {
        Write-Host "  FAIL  the seed file was not built; the truncating churn never ran" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "seed never built" }
        $fail++
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        continue
    }
    # A file that is only ever renamed, back and forth, so a kill can land
    # inside a rename transaction. After the crash it must exist under
    # exactly one of its two names with all 1300 bytes - the property a
    # copy-then-unlink "rename" cannot give. The disk persists across
    # iterations, so first put it back under its home name.
    Send-Line $p "test -e /mnt/keep2 && mv /mnt/keep2 /mnt/keep; test -e /mnt/keep || cat /mnt/seed > /mnt/keep; echo KEEP-READY"
    $kept = Pump $p $st 40 '(?m)^KEEP-READY'
    if (-not $kept) {
        Write-Host "  FAIL  the rename fixture was not prepared" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "rename fixture not prepared" }
        $fail++
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        continue
    }
    $churn = 'while true; do mkdir /mnt/cd; cat /mnt/seed > /mnt/cd/f; echo x > /mnt/cd/f; ln /mnt/cd/f /mnt/cd/g; rm /mnt/cd/g; rm /mnt/cd/f; rmdir /mnt/cd; mv /mnt/keep /mnt/keep2; mv /mnt/keep2 /mnt/keep; done'
    Send-Line $p $churn

    Pump $p $st $churnSeconds $null | Out-Null

    # The power cord. No sync, no unmount, no chance to finish.
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    Start-Sleep -Milliseconds 400

    # Boot again and see what the filesystem looks like.
    $p2 = Start-Guest
    $st2 = New-PumpState
    $up2 = Pump $p2 $st2 $BootWaitMax 'racsh 0\.1\.0'
    $renameState = ""
    if ($up2) {
        # Which name survived, and is the content whole? "10" or "01" with
        # 1300 bytes (the seed: 1040 + 260) is the only acceptable answer.
        Pump $p2 $st2 3 $null | Out-Null
        Send-Line $p2 "a=0; test -e /mnt/keep && a=1; b=0; test -e /mnt/keep2 && b=1; if test -e /mnt/keep; then n=`$(wc -c < /mnt/keep); else n=`$(wc -c < /mnt/keep2); fi; echo RENAME-STATE=`$a`$b:`$n"
        Pump $p2 $st2 40 'RENAME-STATE=\d\d:\d+' | Out-Null
        # -match inside Pump sets $Matches in Pump's scope, not here.
        if ($st2.Text -match 'RENAME-STATE=(\d)(\d):(\d+)') {
            $renameState = $Matches[1] + $Matches[2] + ":" + $Matches[3]
        }
    }
    $log = $st2.Text
    if (-not $p2.HasExited) { Stop-Process -Id $p2.Id -Force }
    $renameOk = ($renameState -eq "10:1300") -or ($renameState -eq "01:1300")

    $panic     = $log -match 'KERNEL PANIC|HALTING'
    $clean     = $log -match 'RACFS sda: fsck clean'
    $dangerous = $log -match 'WARNING - blocks are shared'
    $replayed  = $log -match 'journal replayed transaction (\d+) \((\d+) sectors restored\)'
    $replayNote = if ($replayed) { "replayed txn $($Matches[1]), $($Matches[2]) sectors" } else { "nothing to replay" }

    # The verbatim fsck verdict, not just whether it matched a pattern. A test
    # that reports "not clean" without saying what was wrong cannot tell a
    # journal that works from one that does not.
    $fsckLine = ""
    if ($log -match '(?m)^.*RACFS sda: fsck (?:clean|found[^\r\n]*)') { $fsckLine = $Matches[0].Trim() }
    if ($fsckLine) { Write-Host ("        " + $fsckLine) -ForegroundColor DarkGray }

    if (-not $up2) {
        Write-Host "  FAIL  second boot did not reach racsh ($replayNote)" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "second boot hung" }
        $fail++
    } elseif ($panic) {
        Write-Host "  FAIL  kernel panic on the boot after the crash" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "panic" }
        $fail++
    } elseif ($dangerous) {
        Write-Host "  FAIL  fsck found shared or unallocated-but-used blocks ($replayNote)" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "dangerous fsck findings" }
        $fail++
    } elseif (-not $renameOk) {
        Write-Host "  FAIL  the renamed file is not under exactly one name with 1300 bytes (state '$renameState')" -ForegroundColor Red
        $results += @{ Iter = $i; Ok = $false; Detail = "rename state '$renameState'" }
        $fail++
    } elseif (-not $clean) {
        # Leaked blocks are the survivable class: nothing will be overwritten.
        # Still reported, because a journal that leaks on every crash is a
        # journal with a bug in its rollback.
        Write-Host "  WARN  fsck not clean but not dangerous ($replayNote)" -ForegroundColor Yellow
        $results += @{ Iter = $i; Ok = $true; Detail = "$fsckLine; $replayNote" }
    } else {
        Write-Host "  PASS  fsck clean after a hard kill ($replayNote); renamed file under one name, whole" -ForegroundColor Green
        $results += @{ Iter = $i; Ok = $true; Detail = "clean; $replayNote; rename $renameState" }
    }
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force

Write-Host ""
Write-Host "=== crash-consistency summary ==="
foreach ($r in $results) {
    $tag = if ($r.Ok) { "OK  " } else { "FAIL" }
    Write-Host ("  [$tag] iteration " + $r.Iter + " - " + $r.Detail)
}
Write-Host ""
if ($fail -eq 0) {
    Write-Host "CRASH-CONSISTENCY PASS ($Iterations/$Iterations)" -ForegroundColor Green
    exit 0
} else {
    Write-Host "CRASH-CONSISTENCY FAIL ($fail of $Iterations)" -ForegroundColor Red
    exit 1
}
