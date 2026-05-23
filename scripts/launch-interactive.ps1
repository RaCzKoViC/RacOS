# Launch RacOS in QEMU with a graphical window for interactive use,
# tee serial to a log file so we can both watch what's happening.

param([switch]$NoNet)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

$QemuExe   = "D:\qemu\qemu-system-x86_64.exe"
$OvmfCode  = "D:\qemu\share\edk2-x86_64-code.fd"
$EspDir    = Join-Path $Root "esp"
$DiskPath  = Join-Path $Root "racos-disk.img"
$SerialLog = Join-Path $Root "racos-serial.log"

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
Start-Process -FilePath $QemuExe -ArgumentList $args -PassThru | Select-Object Id
