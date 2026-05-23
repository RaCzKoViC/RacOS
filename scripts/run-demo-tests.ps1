# RacOS - automated end-to-end demo / feature test driver.
#
# Launches QEMU headless with a TCP-serial chardev (same pattern as the
# CI `interactive-smoke` job), waits for the racsh prompt to appear,
# then connects via PowerShell TcpClient and streams a scripted
# sequence of shell commands. Captures the serial log, kills QEMU,
# prints a filtered transcript with the user-typed commands and their
# real responses interleaved (FLUSHD/KBD/NETSTACK noise stripped).
#
# Usage: powershell -File scripts/run-demo-tests.ps1
#
# Flags:
#   -SkipBuild      Reuse the kernel + initramfs already in esp/. Speeds
#                   up iteration when you only changed the command list.
#   -KeepDisk       Don't wipe racos-disk.img before the run. Lets you
#                   verify persistence across multiple invocations
#                   (boot-counter increments, /mnt files survive).
#   -CommandDelay N Seconds between successive commands (default 1.5).
#                   Bump if you see commands getting interleaved on
#                   slow hosts.
#   -BootWaitMax N  Hard cap (seconds) on waiting for the racsh prompt
#                   to appear in the serial log (default 30).

param(
    [switch]$SkipBuild,
    [switch]$KeepDisk,
    [double]$CommandDelay = 1.5,
    [int]$BootWaitMax = 30
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

# -- Paths --
$QemuExe   = "D:\qemu\qemu-system-x86_64.exe"
$OvmfCode  = "D:\qemu\share\edk2-x86_64-code.fd"
$EspDir    = Join-Path $Root "esp"
$DiskPath  = Join-Path $Root "racos-disk.img"
$SerialLog = Join-Path $Root "racos-demo-serial.log"
$SerialPort = 4445   # Different from CI (4444) so we don't clash if both run.

# -- Sanity checks --
foreach ($exe in @($QemuExe, $OvmfCode)) {
    if (-not (Test-Path $exe)) { throw "Not found: $exe" }
}
Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
if (Test-Path $SerialLog) { Remove-Item $SerialLog -Force }

# -- Build --
if (-not $SkipBuild) {
    Write-Host "=== [1/4] Building kernel + userland ===" -ForegroundColor Cyan
    & powershell -NoProfile -File (Join-Path $Root "scripts\build-image.ps1") | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Build failed" }
    $KernelSrc = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\debug\racore"
    if (Test-Path $KernelSrc) {
        Copy-Item $KernelSrc (Join-Path $EspDir "racore.elf") -Force
    }
}

# -- Disk --
if (-not $KeepDisk -and (Test-Path $DiskPath)) {
    Remove-Item $DiskPath -Force
}
if (-not (Test-Path $DiskPath)) {
    Write-Host "=== [2/4] Creating fresh 16 MiB persistent disk ===" -ForegroundColor Cyan
    $fs = [System.IO.File]::Create($DiskPath)
    $fs.SetLength(16MB)
    $fs.Close()
} else {
    Write-Host "=== [2/4] Reusing existing racos-disk.img (persistence test) ===" -ForegroundColor Cyan
}

# -- Launch QEMU with -serial stdio attached to a child process whose
#    stdin/stdout we own. -chardev socket... reports "running" but never
#    binds the TCP port on this Windows install (firewall/QEMU quirk), so
#    we drop to a stdio-piped child. PS 5.1's .NET Framework Process
#    doesn't have ArgumentList (.NET 6+), so we build the command line
#    manually with proper quoting around space-containing paths. --
Write-Host "=== [3/4] Booting QEMU (serial=stdio piped) ===" -ForegroundColor Cyan

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
    "-serial",  "stdio",
    "-display", "none",
    "-no-reboot",
    "-netdev",  "user,id=net0",
    "-device",  "virtio-net-pci,netdev=net0,romfile=,disable-modern=on,disable-legacy=off"
)
$cmdline = ($argList | ForEach-Object { Quote-Arg $_ }) -join ' '

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $QemuExe
$psi.Arguments = $cmdline
$psi.RedirectStandardInput  = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError  = $true
$psi.UseShellExecute = $false
$psi.CreateNoWindow  = $true

$qemu = [System.Diagnostics.Process]::Start($psi)
Write-Host "  QEMU PID=$($qemu.Id), reading serial..."

$enc    = [System.Text.Encoding]::ASCII
$logFs  = [System.IO.File]::Open($SerialLog, [System.IO.FileMode]::Create)
$outStream = $qemu.StandardOutput.BaseStream
$inWriter  = $qemu.StandardInput

function Drain-ForSeconds($seconds) {
    $end = (Get-Date).AddSeconds($seconds)
    $rx  = New-Object byte[] 4096
    while ((Get-Date) -lt $end) {
        # NetworkStream-style polling isn't available on a pipe; use
        # async BeginRead with a short timeout via WaitOne.
        $iar = $outStream.BeginRead($rx, 0, $rx.Length, $null, $null)
        if ($iar.AsyncWaitHandle.WaitOne(120)) {
            $n = $outStream.EndRead($iar)
            if ($n -gt 0) { $logFs.Write($rx, 0, $n); $logFs.Flush() }
        } else {
            # Read still pending; abandon it by closing? Easier: just
            # continue and let the next call inherit. But we can't
            # safely have two outstanding BeginReads, so we DO need to
            # wait until it completes. Block a tad longer.
            $iar.AsyncWaitHandle.WaitOne() | Out-Null
            try {
                $n = $outStream.EndRead($iar)
                if ($n -gt 0) { $logFs.Write($rx, 0, $n); $logFs.Flush() }
            } catch {}
        }
    }
}

# -- Wait for racsh banner --
Write-Host "  Waiting for racsh banner (timeout ${BootWaitMax}s)..."
$start = Get-Date
$ready = $false
$accum = ""
$rxb   = New-Object byte[] 4096
while ((Get-Date) - $start -lt [TimeSpan]::FromSeconds($BootWaitMax)) {
    $iar = $outStream.BeginRead($rxb, 0, $rxb.Length, $null, $null)
    if ($iar.AsyncWaitHandle.WaitOne(500)) {
        $n = $outStream.EndRead($iar)
        if ($n -gt 0) {
            $logFs.Write($rxb, 0, $n); $logFs.Flush()
            $accum += $enc.GetString($rxb, 0, $n)
            if ($accum -match "racsh 0\.1\.0") { $ready = $true; break }
        }
    } else {
        # Read still pending — wait it out so we don't leak handles.
        $iar.AsyncWaitHandle.WaitOne() | Out-Null
        try {
            $n = $outStream.EndRead($iar)
            if ($n -gt 0) {
                $logFs.Write($rxb, 0, $n); $logFs.Flush()
                $accum += $enc.GetString($rxb, 0, $n)
                if ($accum -match "racsh 0\.1\.0") { $ready = $true; break }
            }
        } catch {}
    }
}
if (-not $ready) {
    Write-Host "FAIL: racsh banner not seen in $BootWaitMax s." -ForegroundColor Red
    Write-Host "--- received tail ---"
    $tail = if ($accum.Length -gt 2000) { $accum.Substring($accum.Length - 2000) } else { $accum }
    Write-Output $tail
    $logFs.Close()
    if (-not $qemu.HasExited) { $qemu.Kill() }
    exit 2
}
Write-Host "  racsh up - driving tests..." -ForegroundColor Green

# -- Command sequence (with friendly section banners typed into the shell
#    via `echo` so they appear in the transcript) --
$commands = @(
    # Section 1: filesystem listing
    'echo "===== 1. Filesystem listing ====="',
    'ls /',
    'ls /bin',
    'ls /mnt',
    'ls /var',
    'ls /tmp',
    'ls /fat',
    'ls /dev',

    # Section 2: create / read / append on /mnt (persistent)
    'echo "===== 2. /mnt create+read+append ====="',
    'echo "TEST1" > /mnt/notes.txt',
    'cat /mnt/notes.txt',
    'echo "TEST2" >> /mnt/notes.txt',
    'cat /mnt/notes.txt',

    # Section 3: subdirectory
    'echo "===== 3. Subdirectory on /mnt ====="',
    'mkdir /mnt/sub',
    'echo "deep" > /mnt/sub/deeper.txt',
    'ls /mnt/sub',
    'cat /mnt/sub/deeper.txt',

    # Section 4: copy / move / remove
    'echo "===== 4. cp / mv / rm ====="',
    'cp /mnt/notes.txt /mnt/notes-copy.txt',
    'cat /mnt/notes-copy.txt',
    'mv /mnt/notes-copy.txt /mnt/renamed.txt',
    'ls /mnt',
    'rm /mnt/renamed.txt',
    'ls /mnt',

    # Section 5: cross-mount copy (FAT32 to racfs)
    'echo "===== 5. Cross-filesystem copy ====="',
    'cat /fat/TEST/BOOT.CNT',
    'cp /fat/TEST/BOOT.CNT /mnt/from-fat.txt',
    'cat /mnt/from-fat.txt',

    # Section 6: pipes / filters
    'echo "===== 6. Pipes and filters ====="',
    'echo "ala ma kota" | wc',
    'echo "ala ma kota" | grep ma',
    'ls /bin | wc',
    'ls /bin | head',
    'ls /bin | grep mkfs',
    'cat /mnt/notes.txt | tee /mnt/duplicate.txt',
    'cat /mnt/duplicate.txt',
    'basename /mnt/notes.txt',
    'dirname /mnt/notes.txt',
    'find /mnt',

    # Section 7: procfs and system info
    'echo "===== 7. procfs / system ====="',
    'cat /proc/mounts',
    'cat /proc/cachestats',
    'cat /proc/diskstats',
    'cat /proc/version',
    'mount',
    'df',
    'uptime',
    'env',

    # Section 8: network
    'echo "===== 8. Network: DNS + HTTP ====="',
    'dig example.com',
    'wget example.com',

    # Section 9: storage management
    'echo "===== 9. Storage management ====="',
    'sync',
    'cat /proc/cachestats',

    # Section 10: control flow / exit codes
    'echo "===== 10. Control flow / exit codes ====="',
    'true',
    'echo "true rc=$?"',
    'false',
    'echo "false rc=$?"',
    'test -f /mnt/notes.txt && echo "/mnt/notes.txt exists"',

    # Section 11: SMP / per-CPU timer evidence (kernel log)
    'echo "===== 11. Done. Final listing ====="',
    'ls /mnt'
)

# Drain the prompt itself + any kernel log that piled up before we started
Drain-ForSeconds 0.8

function Send-Line($line) {
    # Throttle per-byte: TCG + racsh's read→echo→process loop can't keep
    # up with bulk writes (chars get dropped mid-string), so we send one
    # character at a time with a 25ms gap. Roughly 40 chars/sec — close
    # to a fast human typist, slow enough that the shell drains each
    # byte before the next arrives. The Drain-ForSeconds between
    # commands still drains the echo + output.
    foreach ($ch in ($line + "`n").ToCharArray()) {
        $inWriter.Write($ch)
        $inWriter.Flush()
        Start-Sleep -Milliseconds 25
    }
}

foreach ($cmd in $commands) {
    Send-Line $cmd
    Drain-ForSeconds $CommandDelay
}

# Final settle so the last command's output (especially wget's HTTP body)
# fully reaches the log before we kill QEMU.
Drain-ForSeconds 4

$logFs.Close()
$inWriter.Close()

Write-Host "  All commands sent. Stopping QEMU..."
if (-not $qemu.HasExited) {
    try { $qemu.Kill() } catch {}
}
Start-Sleep -Milliseconds 500

# -- Render filtered transcript --
Write-Host "
=== [4/4] Filtered transcript ===" -ForegroundColor Cyan
if (-not (Test-Path $SerialLog)) {
    Write-Host "No serial log produced." -ForegroundColor Red
    exit 3
}

$lines = Get-Content $SerialLog
# Drop the high-noise kernel chatter that hides the command results
$filtered = $lines | Where-Object {
    $_ -notmatch '\[ FLUSHD' -and
    $_ -notmatch '\[ NETSTACK \] RX frame' -and
    $_ -notmatch '\[ KBD ' -and
    $_ -notmatch '^\[ USERPROC \]' -and
    $_ -notmatch '^\[   ELF   \]' -and
    $_ -notmatch '^\[  SCHED  \] User process' -and
    $_ -notmatch '^\[  SYSCALL\] sys_exit'
}
$filtered | ForEach-Object { Write-Output $_ }

$logBytes = (Get-Item $SerialLog).Length
Write-Host ""
$summary = "=== Full log: " + $SerialLog + " [" + $logBytes + " bytes] ==="
Write-Host $summary -ForegroundColor Cyan
Write-Host "Done." -ForegroundColor Green
