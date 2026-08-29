# RacOS - the v0.3 Definition-of-Done smoke (ROADMAP section 3.5).
#
# Boot 1, on a fresh disk: type a command into racsh (it lands in
# /home/racos/.racsh_history, which racsh saves after every line), install
# the sample package shipped in the initramfs (rpkg writes to
# /var/lib/rpkg, a persistent subtree since section 3.3), sync, hard-kill.
#
# Boot 2: the GUEST verifies both survived the reboot and prints
# MILESTONE-V0.3-OK on its own console. The host only greps for it. The
# marker coming from inside the system is the point - it proves racsh,
# grep, rpkg and the persistent mounts all cooperate after a reboot, not
# merely that some bytes were on the disk.
#
# Usage:
#   powershell -File scripts/test-milestone-v03.ps1
#
# Exit: 0 = MILESTONE-V0.3-OK seen, 1 = not.
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
$DiskPath = Join-Path $RootQemu "racos-milestone-disk.img"
$DiskReal = Join-Path $Root "racos-milestone-disk.img"

if (-not (Test-Path (Join-Path $Root "esp\racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    exit 2
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $DiskReal) { Remove-Item $DiskReal -Force }
$fs = [System.IO.File]::Create($DiskReal); $fs.SetLength(16MB); $fs.Close()

function Quote-Arg($s) {
    if ($s -match '[\s"]') { return '"' + ($s -replace '"', '\"') + '"' }
    return $s
}

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
    return [System.Diagnostics.Process]::Start($psi)
}

function New-PumpState {
    return @{ Text = ""; Buf = New-Object byte[] 8192; Pending = $null }
}

# Non-blocking read; see test-crash-consistency.ps1 for why blocking here
# deadlocks the runner the moment the guest goes quiet.
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

# Character-at-a-time, or the guest's serial input path silently drops most
# of the line and the workload never runs. See the pitfall notes.
function Send-Line($proc, $line) {
    foreach ($ch in ($line + "`n").ToCharArray()) {
        $proc.StandardInput.Write($ch)
        $proc.StandardInput.Flush()
        Start-Sleep -Milliseconds 30
    }
}

$fail = 0

# ---- Boot 1: leave traces ------------------------------------------------
Write-Host ""
Write-Host "[1/2] boot 1: history line + rpkg install, then hard kill" -ForegroundColor Cyan
$p = Start-Guest
$st = New-PumpState
if (-not (Pump $p $st $BootWaitMax 'racsh 0\.1\.0')) {
    Write-Host "  FAIL  boot 1 never reached racsh" -ForegroundColor Red
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    exit 1
}
Pump $p $st 2 $null | Out-Null

# The history probe. racsh appends and saves history after every line, so
# this lands in /home/racos/.racsh_history the moment it is typed.
$st.Text = ""
Send-Line $p "echo milestone-v03-probe"
if (-not (Pump $p $st 15 'milestone-v03-probe')) {
    Write-Host "  FAIL  shell did not echo the probe; input path broken" -ForegroundColor Red
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    exit 1
}
Write-Host "  probe typed (now in history)"

$st.Text = ""
Send-Line $p "rpkg install /share/demo.rpk && echo INSTALL-DONE"
if (-not (Pump $p $st 20 'INSTALL-DONE')) {
    Write-Host "  FAIL  rpkg install did not succeed on boot 1" -ForegroundColor Red
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    exit 1
}
Write-Host "  demo.rpk installed"

$st.Text = ""
Send-Line $p "sync && echo SYNC-DONE"
Pump $p $st 15 'SYNC-DONE' | Out-Null

# Hard kill - the journal and the persistent mounts are supposed to make
# this survivable, and the milestone should hold under the unfriendly
# shutdown, not only the polite one.
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Milliseconds 400

# ---- Boot 2: the guest itself verifies and prints the marker -------------
Write-Host ""
Write-Host "[2/2] boot 2: guest verifies history + package, prints marker" -ForegroundColor Cyan
$p2 = Start-Guest
$st2 = New-PumpState
if (-not (Pump $p2 $st2 $BootWaitMax 'racsh 0\.1\.0')) {
    Write-Host "  FAIL  boot 2 never reached racsh" -ForegroundColor Red
    if (-not $p2.HasExited) { Stop-Process -Id $p2.Id -Force }
    exit 1
}
Pump $p2 $st2 2 $null | Out-Null

# Same disk, not a reformat: the boot counter says which boot this is.
$counterOk = $st2.Text -match 'boot-counter = 2 \(was 1, file survived reboot\)'
if (-not $counterOk) {
    Write-Host "  FAIL  boot 2 is not running on boot 1's filesystem" -ForegroundColor Red
    $fail++
}

# Every success token below is assembled from a variable, so the token
# never appears in the typed command itself. The console echoes keystrokes,
# and a pattern that occurs in the command matches its own echo before the
# guest has produced any output - which is exactly how the first version of
# this test reported an impossible result (both checks FAIL yet the final
# marker, whose text was in the typed line, "passed").
Send-Line $p2 'h=$(grep milestone-v03-probe /home/racos/.racsh_history)'
Pump $p2 $st2 4 $null | Out-Null
Send-Line $p2 'r=$(rpkg list)'
Pump $p2 $st2 4 $null | Out-Null
Send-Line $p2 'hm=HIST-SURVIVED; pm=PKG-SURVIVED; mm=MILESTONE-V0.3-OK'
Pump $p2 $st2 4 $null | Out-Null

$st2.Text = ""
Send-Line $p2 'test -n "$h" && echo $hm'
$histOk = Pump $p2 $st2 15 'HIST-SURVIVED'
if ($histOk) { Write-Host "  history line survived the reboot" }
else { Write-Host "  FAIL  history did not survive" -ForegroundColor Red; $fail++ }

$st2.Text = ""
Send-Line $p2 'case $r in *demo*) echo $pm;; esac'
$pkgOk = Pump $p2 $st2 15 'PKG-SURVIVED'
if ($pkgOk) { Write-Host "  installed package survived the reboot" }
else { Write-Host "  FAIL  rpkg list does not know demo any more" -ForegroundColor Red; $fail++ }

# The guest, not the host, declares the milestone.
$st2.Text = ""
Send-Line $p2 'case $r in *demo*) test -n "$h" && echo $mm;; esac'
$marker = Pump $p2 $st2 15 'MILESTONE-V0\.3-OK'
if ($marker) { Write-Host "  guest printed MILESTONE-V0.3-OK" -ForegroundColor Green }
else { Write-Host "  FAIL  guest did not print the milestone marker" -ForegroundColor Red; $fail++ }

if (-not $p2.HasExited) { Stop-Process -Id $p2.Id -Force }
Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force

Write-Host ""
if ($fail -eq 0) {
    Write-Host "MILESTONE-V0.3 SMOKE PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host "MILESTONE-V0.3 SMOKE FAIL ($fail)" -ForegroundColor Red
    exit 1
}
