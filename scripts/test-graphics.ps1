# RacOS - graphics smoke (ROADMAP 4.4, first slice of v0.4).
#
# Boots with -vga std and asserts two things:
#
#   1. The kernel CLAIMED the framebuffer - the serial log carries the gfx
#      owner's claim line with geometry and channel order (section 4.1).
#   2. Real pixels reached the screen: a QMP `screendump` is taken and the
#      PPM is required to contain >= 1000 DISTINCT non-zero pixel values.
#      A text console alone produces a handful of values; only the status
#      bar's per-pixel gradient (drawn through a gfx Surface and presented
#      by the owner - the section 6b path) yields a thousand. So this
#      number is not a vanity metric: it can only be reached if the
#      Surface/present machinery actually works.
#
# The screendump is the assertion the serial log cannot make. A kernel that
# claims the framebuffer and then draws nothing, or draws into the wrong
# place, logs exactly the same lines; the dump is ground truth from the
# emulated display itself.
#
# Usage:
#   powershell -File scripts/test-graphics.ps1 [-BootWaitMax 90]
#
# Exit: 0 = both assertions hold, 1 = not.
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
$DiskPath = Join-Path $RootQemu "racos-gfx-disk.img"
$DiskReal = Join-Path $Root "racos-gfx-disk.img"
$DumpPath = Join-Path $RootQemu "racos-gfx-dump.ppm"
$DumpReal = Join-Path $Root "racos-gfx-dump.ppm"
$QmpPort  = 4488

if (-not (Test-Path (Join-Path $Root "esp\racore.elf"))) {
    Write-Host "esp/racore.elf missing. Stage the image first:" -ForegroundColor Yellow
    Write-Host "  powershell -File scripts\build-image.ps1"
    exit 2
}

Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
if (Test-Path $DiskReal) { Remove-Item $DiskReal -Force }
if (Test-Path $DumpReal) { Remove-Item $DumpReal -Force }
$fs = [System.IO.File]::Create($DiskReal); $fs.SetLength(16MB); $fs.Close()

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
    "-vga",     "std",
    "-qmp",     "tcp:127.0.0.1:$QmpPort,server,nowait",
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
Write-Host "QEMU PID=$($p.Id), QMP on 127.0.0.1:$QmpPort"

# Pump serial until racsh (the status bar is drawn long before that).
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

# QMP: capabilities handshake, then a screendump of the primary console.
function Invoke-Qmp($commands) {
    $client = New-Object System.Net.Sockets.TcpClient
    $client.Connect("127.0.0.1", $QmpPort)
    $stream = $client.GetStream()
    $writer = New-Object System.IO.StreamWriter($stream)
    $reader = New-Object System.IO.StreamReader($stream)
    $writer.AutoFlush = $true
    Start-Sleep -Milliseconds 200
    $writer.WriteLine('{"execute":"qmp_capabilities"}')
    Start-Sleep -Milliseconds 200
    foreach ($c in $commands) {
        $writer.WriteLine($c)
        Start-Sleep -Milliseconds 400
    }
    while ($stream.DataAvailable) { $reader.ReadLine() | Out-Null }
    $client.Close()
}

$dumpJson = ($DumpPath -replace '\\', '/')
try {
    Invoke-Qmp @("{""execute"":""screendump"",""arguments"":{""filename"":""$dumpJson""}}")
} catch {
    Write-Host "QMP connection failed: $_" -ForegroundColor Red
}
Start-Sleep -Milliseconds 500

if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
Get-Process -Name qemu-system-x86_64 -ErrorAction SilentlyContinue | Stop-Process -Force

Write-Host ""
Write-Host "=== graphics assertions ==="
$fail = 0

# 1. The claim line, with geometry and channel order.
if ($text -match '\[  GFX   \] claimed (\d+)x(\d+)x32 (BGRX|RGBX) framebuffer') {
    Write-Host ("  PASS  framebuffer claimed: " + $Matches[1] + "x" + $Matches[2] + " " + $Matches[3])
} else {
    Write-Host "  FAIL  no framebuffer claim line in the serial log" -ForegroundColor Red
    $fail++
}

# 2. The status bar was presented through a Surface.
if ($text -match 'status bar presented') {
    Write-Host "  PASS  status bar surface presented"
} else {
    Write-Host "  FAIL  status bar was never presented" -ForegroundColor Red
    $fail++
}

# 3. >= 1000 distinct non-zero pixel values in the actual display output.
if (Test-Path $DumpReal) {
    $count = python -c @"
import sys
with open(r'$DumpReal','rb') as f:
    data = f.read()
# P6 header: magic, width height, maxval, then binary RGB triples. Comments
# (#...) are legal between tokens; QEMU does not emit them, but skip anyway.
tok = []
i = 2
while len(tok) < 3:
    while data[i] in b' \t\r\n': i += 1
    if data[i:i+1] == b'#':
        while data[i] not in b'\r\n': i += 1
        continue
    j = i
    while data[j] not in b' \t\r\n': j += 1
    tok.append(int(data[i:j])); i = j
i += 1  # single whitespace after maxval
px = data[i:]
seen = set()
for o in range(0, len(px) - 2, 3):
    v = (px[o] << 16) | (px[o+1] << 8) | px[o+2]
    if v: seen.add(v)
print(len(seen))
"@
    if ([int]$count -ge 1000) {
        Write-Host ("  PASS  screendump has " + $count + " distinct non-zero pixel values (>= 1000)")
    } else {
        Write-Host ("  FAIL  screendump has only " + $count + " distinct non-zero pixel values") -ForegroundColor Red
        $fail++
    }
} else {
    Write-Host "  FAIL  no screendump was produced" -ForegroundColor Red
    $fail++
}

Write-Host ""
if ($fail -eq 0) {
    Write-Host "GRAPHICS-SMOKE PASS" -ForegroundColor Green
    exit 0
} else {
    Write-Host "GRAPHICS-SMOKE FAIL ($fail)" -ForegroundColor Red
    exit 1
}
