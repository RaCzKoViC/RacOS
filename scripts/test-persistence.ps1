# RacOS - on-disk persistence test (local equivalent of the CI `boot-smoke` job)
#
# Boots RacOS twice against the same AHCI disk image and asserts that the racfs
# boot-counter written on the first boot is still there, incremented, on the
# second. CI implements this inline in .github/workflows/ci.yml; this script is
# the runnable local version.
#
# Usage:
#   powershell -File scripts/test-persistence.ps1 [-BootSeconds 45] [-Smp 4]
#
# Exit: 0 = all assertions passed, 1 = at least one failed.
#
# NOTE: ASCII only. PowerShell 5.1 reads .ps1 as Win-1252, so a single non-ASCII
# character (an em dash in a comment is the usual culprit) fails the whole file
# with "The string is missing the terminator".

param(
    [int]$BootSeconds = 45,
    [int]$Smp = 4
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")
$qemu     = Find-QemuPaths
$QemuExe  = $qemu.Exe
$OvmfCode = $qemu.Ovmf

$EspDir   = Join-Path $Root "esp"
$DiskPath = Join-Path $Root "racos-persist-disk.img"

if (-not (Test-Path (Join-Path $EspDir "racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    Write-Host "  Copy-Item target\x86_64-unknown-none\debug\racore esp\racore.elf -Force"
    exit 2
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

# Fresh disk so boot 1 is genuinely a first boot.
if (Test-Path $DiskPath) { Remove-Item $DiskPath -Force }
$fs = [System.IO.File]::Create($DiskPath)
$fs.SetLength(16MB)
$fs.Close()
Write-Host "Created fresh 16 MiB disk: $DiskPath"

function Invoke-Boot($LogName) {
    $LogPath = Join-Path $Root $LogName
    if (Test-Path $LogPath) { Remove-Item $LogPath -Force }

    $argList = @(
        "-machine", "q35",
        "-accel",   "tcg",
        "-cpu",     "qemu64,+smep,+smap",
        "-smp",     "$Smp",
        "-m",       "512M",
        "-drive",   "if=pflash,format=raw,readonly=on,file=$OvmfCode",
        "-boot",    "menu=on",
        "-drive",   "if=ide,format=raw,file=fat:rw:$EspDir",
        "-drive",   "id=disk0,file=$DiskPath,if=none,format=raw,cache=writethrough",
        "-device",  "ich9-ahci,id=ahci",
        "-device",  "ide-hd,drive=disk0,bus=ahci.0",
        "-serial",  "file:$LogPath",
        "-monitor", "null",
        "-display", "none",
        "-no-reboot"
    )

    Write-Host "Booting -> $LogName ($BootSeconds s budget)..."
    # Start-Job with the call operator: Start-Process -ArgumentList re-joins on
    # spaces and would split a project path like "RacOS - GitHub".
    $job = Start-Job -ScriptBlock {
        param($exe, $arglist)
        & $exe @arglist
    } -ArgumentList $QemuExe, $argList

    Wait-Job -Job $job -Timeout $BootSeconds | Out-Null
    Stop-Job -Job $job -ErrorAction SilentlyContinue
    Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
    Remove-Job -Job $job -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800

    if (-not (Test-Path $LogPath)) { throw "No serial log produced for $LogName" }
    Write-Host ("  " + $LogName + " captured [" + (Get-Item $LogPath).Length + " bytes]")
    return $LogPath
}

$log1 = Invoke-Boot "boot1.log"
$log2 = Invoke-Boot "boot2.log"

$b1 = Get-Content $log1 -Raw
$b2 = Get-Content $log2 -Raw

$checks = @(
    @{ Name = "boot2: kernel starts";        Log = $b2; Pattern = "RACORE: RacOS kernel starting" },
    @{ Name = "boot2: reaches idle loop";    Log = $b2; Pattern = "RACORE: Entering idle loop" },
    @{ Name = "boot2: RacInit starts";       Log = $b2; Pattern = "\[init\] RacInit starting" },
    @{ Name = "boot2: init spawns /bin/sh";  Log = $b2; Pattern = "\[init\] spawned /bin/sh" },
    @{ Name = "boot2: racsh banner";         Log = $b2; Pattern = "racsh 0\.1\.0" },
    @{ Name = "boot2: SMP $Smp CPUs enabled"; Log = $b2; Pattern = "SMP topology - $Smp enabled CPU\(s\)" },
    @{ Name = "boot1: created counter = 1";  Log = $b1; Pattern = "created boot-counter = 1 \(first boot\)" },
    @{ Name = "boot2: counter survived = 2"; Log = $b2; Pattern = "boot-counter = 2 \(was 1, file survived reboot\)" },
    # boot-counter is one byte, so it only ever proved direct[0] survives. The
    # big-probe is 8192 bytes and the assertion reads its *tail*, which lives
    # in a block reachable only through the inode's indirect pointer.
    @{ Name = "boot1: created big-probe";    Log = $b1; Pattern = "created big-probe \(8192 B, past the direct blocks\)" },
    @{ Name = "boot2: indirect blocks survived"; Log = $b2; Pattern = "big-probe tail = 1 \(expected 1, indirect blocks survived reboot\)" },
    # v0.3 section 3.3. The /etc pair is the cross-reboot assertion: boot1 copies the
    # initramfs defaults onto the disk, and boot2 must mount that /etc without
    # seeding it again. A boot2 that re-seeds would mean the persistent /etc
    # came up empty, which is also the state that leaves init with no units.
    @{ Name = "boot1: /home persistent";         Log = $b1; Pattern = "/home is persistent \(sda:/home\)" },
    @{ Name = "boot1: /var/log persistent";      Log = $b1; Pattern = "/var/log is persistent \(sda:/var/log\)" },
    @{ Name = "boot1: /var/lib/rpkg persistent"; Log = $b1; Pattern = "/var/lib/rpkg is persistent \(sda:/var/lib/rpkg\)" },
    @{ Name = "boot1: /etc seeded from initramfs"; Log = $b1; Pattern = "/etc seed: copied /etc/racinit/base.target" },
    @{ Name = "boot2: /etc persistent";          Log = $b2; Pattern = "/etc is persistent \(sda:/etc\)" },
    @{ Name = "boot2: /etc NOT re-seeded";       Log = $b2; Pattern = "/etc seed: copied"; Absent = $true }
)

Write-Host ""
Write-Host "=== Persistence assertions ==="
$fail = 0
foreach ($c in $checks) {
    # Absent = the pattern must NOT appear. Needed because some of what this
    # smoke proves is that a boot did *not* do something -- re-seeding /etc,
    # for instance, which would mean the persistent copy did not survive.
    $found = [bool]($c.Log -match $c.Pattern)
    $ok = if ($c.Absent) { -not $found } else { $found }
    if ($ok) { Write-Host ("  PASS  " + $c.Name) }
    else { Write-Host ("  FAIL  " + $c.Name); $fail++ }
}

Write-Host ""
if ($fail -eq 0) {
    Write-Host ("BOOT-SMOKE PASS (" + $checks.Count + "/" + $checks.Count + ")")
    exit 0
} else {
    # The usual cause of a wholesale failure here is the wrong kernel in the
    # ESP: run-ci-smoke.ps1 stages a --features ci-smoke build, which runs its
    # assertions and exits via isa-debug-exit instead of booting to racsh.
    if ($b2 -match "\[ SMOKE \]") {
        Write-Host "esp/racore.elf is a ci-smoke kernel, not a normal one." -ForegroundColor Yellow
        Write-Host "Re-stage the plain kernel and run this again:" -ForegroundColor Yellow
        Write-Host "  cargo build --package racore --target x86_64-unknown-none"
        Write-Host "  Copy-Item target\x86_64-unknown-none\debug\racore esp\racore.elf -Force"
    }
    Write-Host ("BOOT-SMOKE FAIL (" + $fail + " assertion(s) failed)")
    exit 1
}
