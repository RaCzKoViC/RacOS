# RacOS - graphics smoke (ROADMAP 4.4).
#
# Boots with -vga std and asserts, from the serial log and from a QMP
# `screendump` of the emulated display:
#
#   1. The kernel CLAIMED the framebuffer - the gfx owner's claim line with
#      geometry and channel order (section 4.1).
#   2. The VT layer took the console region over (rendered from RacTerm
#      buffers through gfx Surfaces - the section 6b path).
#   3. The console it renders is on the screen: the dump holds a text-sized
#      amount of lit pixels (not none, not a flood), and lines start at the
#      left edge - glyph pixels in the leftmost 8 px on several distinct
#      16 px text rows. A staircase console (LF without CR) fails this, a
#      blank one fails this, a Surface presented in the wrong place fails
#      this.
#   4. Nothing but the console: the bottom 24 rows are black. The owner
#      used to reserve them for a rainbow status bar whose real job was to
#      be this smoke's evidence (>= 1000 distinct pixel values); the smoke
#      now checks the console instead and the console has the whole screen.
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

# Pump serial until racsh: by then the VT holds init's lines and the prompt.
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

# 2. The VT layer owns the console region (rows go through gfx Surfaces).
if ($text -match '\[  VT  \] \d+ terminals, rendered from RacTerm buffers') {
    Write-Host "  PASS  VT layer rendering the console"
} else {
    Write-Host "  FAIL  VT layer never took the console over" -ForegroundColor Red
    $fail++
}

# 3 + 4. What the display actually shows. The Python prints four numbers:
#   lit       - non-black pixels in the whole dump
#   total     - pixels in the dump
#   leftrows  - distinct 16 px text rows with a lit pixel in the leftmost 8 px
#   bottomlit - non-black pixels in the bottom 24 rows
if (Test-Path $DumpReal) {
    $stats = python -c @"
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
w, h = tok[0], tok[1]
px = data[i:]
lit = 0
leftrows = set()
bottomlit = 0
for y in range(h):
    row = px[y * w * 3:(y + 1) * w * 3]
    for x in range(w):
        o = x * 3
        if row[o] or row[o + 1] or row[o + 2]:
            lit += 1
            if x < 8:
                leftrows.add(y // 16)
            if y >= h - 24:
                bottomlit += 1
print(lit, w * h, len(leftrows), bottomlit)
"@
    $parts = "$stats".Trim() -split '\s+'
    $lit = [int]$parts[0]; $total = [int]$parts[1]; $leftrows = [int]$parts[2]; $bottomlit = [int]$parts[3]
    if ($lit -ge 1000 -and $lit -le ($total / 4)) {
        Write-Host ("  PASS  console text on screen: " + $lit + " lit pixels of " + $total)
    } else {
        Write-Host ("  FAIL  lit pixels = " + $lit + " of " + $total + " (expected a text-sized amount: 1000 .. 25%)") -ForegroundColor Red
        $fail++
    }
    if ($leftrows -ge 3) {
        Write-Host ("  PASS  lines start at the left edge: " + $leftrows + " text rows lit in the leftmost 8 px")
    } else {
        Write-Host ("  FAIL  only " + $leftrows + " text rows lit in the leftmost 8 px (staircase or blank console)") -ForegroundColor Red
        $fail++
    }
    if ($bottomlit -eq 0) {
        Write-Host "  PASS  bottom 24 rows black: nothing but the console on screen"
    } else {
        Write-Host ("  FAIL  bottom 24 rows have " + $bottomlit + " lit pixels (status bar?)") -ForegroundColor Red
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
