# Launch RacOS in QEMU as a single, self-contained interactive window.
#
# The graphical (VGA) window is a full console: the kernel mirrors /dev/console
# output to the framebuffer and feeds PS/2 keystrokes back into the console input
# stream, so you type directly in the QEMU window. We use the windowed QEMU build
# (qemu-system-x86_64w.exe) so no separate, empty terminal window is opened.
# Serial is still teed to a log file for inspection.

param([switch]$NoNet)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

. (Join-Path $PSScriptRoot "_qemu-common.ps1")

$qemu      = Find-QemuPaths
$QemuExe   = $qemu.ExeGui     # GUI build: no extra console window
$OvmfCode  = $qemu.Ovmf

# QEMU mishandles spaces in -drive/-serial paths (e.g. a project under
# "D:\OS project"). Hand it a space-free alias of the project root; the files
# still live in the real tree.
$RootQemu  = Resolve-SpacelessPath $Root
$EspDir    = Join-Path $RootQemu "esp"
$DiskPath  = Join-Path $RootQemu "racos-disk.img"
$SerialLog = Join-Path $RootQemu "racos-serial.log"

if (-not (Test-Path $DiskPath)) {
    Write-Host "Creating 16 MiB sparse disk: $DiskPath"
    $fs = [System.IO.File]::Create($DiskPath)
    $fs.SetLength(16MB)
    $fs.Close()
}
if (Test-Path $SerialLog) { Remove-Item $SerialLog -Force }

$args = @(
    "-machine", "q35",
    "-accel",   "tcg",
    "-cpu",     "qemu64,+smep,+smap",
    "-m",       "512M",
    "-drive",   "if=pflash,format=raw,readonly=on,file=$OvmfCode",
    "-boot",    "menu=on",
    "-drive",   "if=ide,format=raw,file=fat:rw:$EspDir",
    "-drive",   "id=disk0,file=$DiskPath,if=none,format=raw,cache=writethrough",
    "-device",  "ich9-ahci,id=ahci",
    "-device",  "ide-hd,drive=disk0,bus=ahci.0",
    "-serial",  "file:$SerialLog",
    "-vga",     "std",
    "-no-reboot"
)

if (-not $NoNet) {
    $args += @(
        "-netdev", "user,id=net0",
        "-device", "virtio-net-pci,netdev=net0,romfile=,disable-modern=on,disable-legacy=off"
    )
}

Write-Host "Launching RacOS QEMU window. Serial log: $SerialLog"
Write-Host "Click into the QEMU window and type - it is a full interactive console."
Write-Host "Alt+F1..F6 switch virtual terminals."
Start-Process -FilePath $QemuExe -ArgumentList $args -PassThru | Select-Object Id
