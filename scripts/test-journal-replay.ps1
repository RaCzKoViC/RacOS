# RacOS - prove the journal's replay path actually restores a filesystem.
#
# The crash test (test-crash-consistency.ps1) kills the guest mid-write and
# checks the next boot is consistent, but it almost never lands in the window
# between a commit record being written and the log being retired -- that
# window is a handful of sector writes wide. So it reports "nothing to
# replay" and the recovery path stays untested, which is the half of
# journaling that only ever runs when something has already gone wrong.
#
# This test forges that state instead of waiting for it. It corrupts a live
# metadata sector on the image and, in the treatment case, leaves a committed
# journal describing how to restore it. Two phases:
#
#   CONTROL   - corrupt the inode table, leave the journal empty.
#               The next boot MUST report damage. This is what proves the
#               test can fail; a corruption the filesystem shrugs off would
#               make the treatment phase meaningless.
#   TREATMENT - the same corruption, plus a committed journal holding the
#               original sector. The next boot MUST replay it and come up
#               clean.
#
# Usage:
#   powershell -File scripts/test-journal-replay.ps1
#
# Exit: 0 = control failed as it should and treatment recovered, 1 = otherwise.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252 and a stray
# non-ASCII character fails the file with "The string is missing the
# terminator".

param([int]$BootWaitMax = 90)

$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")
$qemu     = Find-QemuPaths
$QemuExe  = $qemu.Exe
$OvmfCode = $qemu.Ovmf

$RootQemu = Resolve-SpacelessPath $Root
$EspDir   = Join-Path $RootQemu "esp"
$DiskPath = Join-Path $RootQemu "racos-journal-disk.img"
$DiskReal = Join-Path $Root "racos-journal-disk.img"

if (-not (Test-Path (Join-Path $Root "esp\racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    exit 2
}

$SECTOR = 512
$JOURNAL_MAGIC = 0x524A4C31   # "RJL1"
$JOURNAL_COMMITTED = 1
$REPLAY_SEQ = 4242            # distinctive, so the log line is unmistakable

function Quote-Arg($s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

function Read-Sector($path, $lba) {
    $fs = [System.IO.File]::Open($path, 'Open', 'Read')
    try {
        $fs.Seek([int64]$lba * $SECTOR, 'Begin') | Out-Null
        $buf = New-Object byte[] $SECTOR
        $read = 0
        while ($read -lt $SECTOR) {
            $n = $fs.Read($buf, $read, $SECTOR - $read)
            if ($n -le 0) { break }
            $read += $n
        }
        return $buf
    } finally { $fs.Close() }
}

function Write-Sector($path, $lba, $bytes) {
    $fs = [System.IO.File]::Open($path, 'Open', 'Write')
    try {
        $fs.Seek([int64]$lba * $SECTOR, 'Begin') | Out-Null
        $fs.Write($bytes, 0, $SECTOR)
        $fs.Flush()
    } finally { $fs.Close() }
}

# Boot once and return the serial text. Killed as soon as racsh appears (or
# the budget runs out), because all this test reads is the mount-time log.
function Boot-AndCapture($seconds) {
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

    $text = ""
    $buf = New-Object byte[] 8192
    $pending = $null
    $end = (Get-Date).AddSeconds($seconds)
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
    Start-Sleep -Milliseconds 400
    return $text
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $DiskReal) { Remove-Item $DiskReal -Force }
$fs = [System.IO.File]::Create($DiskReal); $fs.SetLength(16MB); $fs.Close()

# --- Boot once so the disk is formatted and carries real metadata ---------
Write-Host ""
Write-Host "[1/3] seeding: first boot formats the disk and populates it" -ForegroundColor Cyan
$seed = Boot-AndCapture $BootWaitMax
if ($seed -notmatch 'racsh 0\.1\.0') {
    Write-Host "  FAIL  seeding boot never reached racsh" -ForegroundColor Red
    exit 1
}
if ($seed -notmatch 'journal sectors') {
    Write-Host "  FAIL  this image has no journal; nothing to test" -ForegroundColor Red
    exit 1
}
Write-Host "  seeded."

# Superblock: bitmap_start at offset 20, inode_start at 24. The inode table's
# first sector holds the root directory inode, so losing it loses the
# filesystem -- which is what makes it a good thing to destroy on purpose.
$sb = Read-Sector $DiskReal 0
$inodeStart = [BitConverter]::ToUInt32($sb, 24)
Write-Host ("  inode table starts at LBA " + $inodeStart)

$original = Read-Sector $DiskReal $inodeStart
$zeroed = New-Object byte[] $SECTOR

# Snapshot the whole seeded image. Both phases are restored from it, so they
# start from byte-identical state and their results are actually comparable.
# Without this the control boot's own writes carry into the treatment phase,
# and the replayed sector ends up paired with a superblock from a later
# generation -- an inconsistency the test manufactured rather than found.
$SeedCopy = Join-Path $Root "racos-journal-seed.img"
Copy-Item $DiskReal $SeedCopy -Force

# --- CONTROL: corrupt, no journal ----------------------------------------
Write-Host ""
Write-Host "[2/3] control: corrupt the inode table, leave the journal empty" -ForegroundColor Cyan
Copy-Item $SeedCopy $DiskReal -Force
Write-Sector $DiskReal $inodeStart $zeroed
$control = Boot-AndCapture $BootWaitMax

$controlClean = $control -match 'RACFS sda: fsck clean'
$controlLine = ""
if ($control -match '(?m)^.*RACFS sda: fsck (?:clean|found[^\r\n]*)') { $controlLine = $Matches[0].Trim() }
Write-Host ("        " + $controlLine) -ForegroundColor DarkGray

if ($controlClean) {
    # The corruption did not register, so the treatment phase would prove
    # nothing: a clean result there could just mean nothing was ever broken.
    Write-Host "  FAIL  destroying the inode table left fsck reporting clean;" -ForegroundColor Red
    Write-Host "        this test cannot distinguish a working replay from no replay" -ForegroundColor Red
    exit 1
}
Write-Host "  control behaved: the corruption is visible without a journal." -ForegroundColor Green

# --- TREATMENT: same corruption, plus a committed journal -----------------
Write-Host ""
Write-Host "[3/3] treatment: same corruption, with a committed journal entry" -ForegroundColor Cyan
Copy-Item $SeedCopy $DiskReal -Force
Write-Sector $DiskReal $inodeStart $zeroed

# Slot 0 (LBA 2) carries the sector's original contents.
Write-Sector $DiskReal 2 $original

# Header (LBA 1): magic, state=COMMITTED, seq, count=1, then the target LBA.
$header = New-Object byte[] $SECTOR
[Array]::Copy([BitConverter]::GetBytes([uint32]$JOURNAL_MAGIC), 0, $header, 0, 4)
[Array]::Copy([BitConverter]::GetBytes([uint32]$JOURNAL_COMMITTED), 0, $header, 4, 4)
[Array]::Copy([BitConverter]::GetBytes([uint64]$REPLAY_SEQ), 0, $header, 8, 8)
[Array]::Copy([BitConverter]::GetBytes([uint32]1), 0, $header, 16, 4)
[Array]::Copy([BitConverter]::GetBytes([uint64]$inodeStart), 0, $header, 24, 8)
Write-Sector $DiskReal 1 $header

$treatment = Boot-AndCapture $BootWaitMax

$replayed = $treatment -match "journal replayed transaction $REPLAY_SEQ \((\d+) sectors restored\)"
$restored = if ($replayed) { $Matches[1] } else { "0" }
$treatClean = $treatment -match 'RACFS sda: fsck clean'
$treatLine = ""
if ($treatment -match '(?m)^.*RACFS sda: fsck (?:clean|found[^\r\n]*)') { $treatLine = $Matches[0].Trim() }
Write-Host ("        " + $treatLine) -ForegroundColor DarkGray

$fail = 0
if ($replayed) {
    Write-Host ("  PASS  replay ran: transaction $REPLAY_SEQ, $restored sector(s) restored") -ForegroundColor Green
} else {
    Write-Host "  FAIL  no replay happened" -ForegroundColor Red
    $fail++
}
if ($treatClean) {
    Write-Host "  PASS  filesystem is clean again after the replay" -ForegroundColor Green
} else {
    Write-Host "  FAIL  still damaged after the replay" -ForegroundColor Red
    $fail++
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
if (Test-Path $SeedCopy) { Remove-Item $SeedCopy -Force }

Write-Host ""
if ($fail -eq 0) {
    Write-Host "JOURNAL-REPLAY PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host "JOURNAL-REPLAY FAIL" -ForegroundColor Red
    exit 1
}
