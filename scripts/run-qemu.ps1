# RacOS — QEMU Boot Script
# Boots the RacOS image using OVMF (UEFI) firmware.
#
# Usage:   powershell -File scripts/run-qemu.ps1
# Options: -Debug      — enable GDB stub on port 1234 (-s -S, halts at start)
#          -NoAccel    — disable WHPX hardware acceleration (force TCG)
#          -Headless   — no graphical window, serial only (useful for CI)
#          -Net        — attach virtio-net-pci on QEMU user networking (10.0.2.0/24)
#          -KernelTest — wire up isa-debug-exit device for in-kernel test harness
#          -Disk       — attach racos-disk.img to ich9-ahci (created if missing)
#          -Ram <MB>   — override guest RAM (default 512)

param(
    [switch]$Debug,
    [switch]$NoAccel,
    [switch]$Headless,
    [switch]$Net,
    [switch]$KernelTest,
    [switch]$Disk,
    [int]$Ram = 512,
    [string[]]$KernelFeatures = @()
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Definition)
Set-Location $ProjectRoot

# ── Paths ──
$QemuExe   = "D:\qemu\qemu-system-x86_64.exe"
$OvmfCode  = "D:\qemu\share\edk2-x86_64-code.fd"
$EspDir    = Join-Path $ProjectRoot "esp"
$KernelElf = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\debug\racore"

# Build kernel with static relocation model for direct physical entry jumping
$oldRustflags = $env:RUSTFLAGS
$env:RUSTFLAGS = "-C relocation-model=static -C link-arg=-no-pie"
$KernelArgs = @("--package", "racore", "--target", "x86_64-unknown-none")
if ($KernelFeatures.Count -gt 0) {
    $KernelArgs += @("--features", ($KernelFeatures -join ","))
}
cargo build @KernelArgs
if ($LASTEXITCODE -ne 0) { throw "Kernel build failed" }
$env:RUSTFLAGS = $oldRustflags

# ── Validate prerequisites ──
if (-not (Test-Path $QemuExe))  { throw "QEMU not found at $QemuExe" }
if (-not (Test-Path $OvmfCode)) { throw "OVMF firmware not found at $OvmfCode" }
if (-not (Test-Path "$EspDir\EFI\BOOT\BOOTX64.EFI")) {
    throw "Bootloader not found. Run build-image.ps1 first."
}

# Copy kernel to ESP
$KernelDest = Join-Path $EspDir "racore.elf"
if (Test-Path $KernelElf) {
    Copy-Item $KernelElf $KernelDest -Force
    Write-Host "Copied kernel to ESP"
} else {
    if (-not (Test-Path $KernelDest)) {
        throw "Kernel not found. Run build-image.ps1 first."
    }
}

# ── Detect accelerator ──
# WHPX requires split kernel-irqchip; set on -machine, not -accel.
$Accel = "tcg"
$Machine = "q35"
if (-not $NoAccel) {
    $accelHelp = & $QemuExe -accel help 2>&1
    if ($accelHelp -match "whpx") {
        $Accel = "whpx"
        $Machine = "q35,kernel-irqchip=off"
    }
}

# ── Build QEMU command ──
$QemuArgs = @(
    "-machine", $Machine
    "-accel", $Accel
    "-cpu", "qemu64"
    "-m", "${Ram}M"
    "-drive", "if=pflash,format=raw,readonly=on,file=$OvmfCode"
    "-boot", "menu=on"
    "-drive", "if=ide,format=raw,file=fat:rw:$EspDir"
    "-serial", "stdio"
    "-no-reboot"
    "-no-shutdown"
)

if ($Headless) {
    $QemuArgs += @("-display", "none", "-nographic")
} else {
    $QemuArgs += @("-vga", "std")
}

if ($Net) {
    # virtio-net-pci on QEMU user-mode networking: gateway 10.0.2.2, DNS 10.0.2.3
    # filter-dump captures every frame on net0 to a pcap for offline inspection.
    $pcapPath = "$ProjectRoot\racos-net.pcap"
    # disable-modern=on forces transitional virtio-net into legacy mode so our
    # PIO-based driver can talk to it. Without this, QEMU 4.0+ defaults q35 to
    # modern-only and our legacy I/O ops are ignored silently.
    $QemuArgs += @(
        "-netdev", "user,id=net0"
        "-device", "virtio-net-pci,netdev=net0,romfile=,disable-modern=on,disable-legacy=off"
        "-object", "filter-dump,id=f0,netdev=net0,file=$pcapPath"
    )
    Write-Host "  PCAP:  $pcapPath"
}

if ($Disk) {
    # Persistent SATA disk on ich9-ahci. The image is created lazily — first
    # run produces a 16 MiB sparse file that survives reboots and stays in the
    # project root next to the pcap.
    $diskPath = "$ProjectRoot\racos-disk.img"
    if (-not (Test-Path $diskPath)) {
        $size = 16MB
        $fs = [System.IO.File]::Create($diskPath)
        $fs.SetLength($size)
        $fs.Close()
        Write-Host "  Disk:  created $diskPath ($size bytes)"
    }
    # cache=writethrough fsyncs after every write so the on-disk image stays
    # consistent even if QEMU is killed hard (e.g. Stop-Process -Force). The
    # default writeback cache otherwise loses recent writes on abrupt exit
    # and the next boot sees a stale/corrupt superblock.
    $QemuArgs += @(
        "-drive",  "id=disk0,file=$diskPath,if=none,format=raw,cache=writethrough"
        "-device", "ich9-ahci,id=ahci"
        "-device", "ide-hd,drive=disk0,bus=ahci.0"
    )
    Write-Host "  Disk:  $diskPath -> ich9-ahci"
}

if ($KernelTest) {
    # isa-debug-exit lets the kernel signal QEMU exit code via port 0xf4
    # Convention: exit_code = (test_status << 1) | 1  →  QEMU returns 33 on success (0x10<<1|1)
    $QemuArgs += @("-device", "isa-debug-exit,iobase=0xf4,iosize=0x04")
}

if ($Debug) {
    $QemuArgs += @("-s", "-S")
    Write-Host "Debug mode: GDB stub on localhost:1234, waiting for connection..."
}

Write-Host ""
Write-Host "=== Launching RacOS in QEMU ==="
Write-Host "  QEMU:  $QemuExe"
Write-Host "  OVMF:  $OvmfCode"
Write-Host "  ESP:   $EspDir"
Write-Host "  Accel: $Accel"
Write-Host "  RAM:   ${Ram}M"
if ($Net)        { Write-Host "  Net:   virtio-net-pci (user mode 10.0.2.0/24)" }
if ($KernelTest) { Write-Host "  Test:  isa-debug-exit @ port 0xf4" }
Write-Host ""

& $QemuExe @QemuArgs
