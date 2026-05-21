# RacOS — Build kernel + userland + initramfs image
#
# Builds the kernel and all coreutils for x86_64-unknown-none,
# assembles an initramfs image, and prepares the ESP directory
# for QEMU boot testing.
#
# Usage: powershell -File scripts/build-image.ps1 [-Release]

param(
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
Set-Location $Root

$Profile = if ($Release) { "release" } else { "dev" }
$ProfileDir = if ($Release) { "release" } else { "debug" }
$CargoFlags = @("--target", "x86_64-unknown-none")
if ($Release) { $CargoFlags += "--release" }

$TargetDir = & cargo metadata --format-version 1 --no-deps --quiet 2>$null |
    ConvertFrom-Json | Select-Object -ExpandProperty target_directory
$BinDir = "$TargetDir\x86_64-unknown-none\$ProfileDir"

# Keep caller RUSTFLAGS and use kernel-specific flags only for kernel build.
$OldRustFlags = $env:RUSTFLAGS

Write-Host "=== RacOS Image Builder ===" -ForegroundColor Cyan
Write-Host "Profile: $Profile"
Write-Host "Target dir: $TargetDir"

# --- Step 1: Build kernel ---
Write-Host "`n[1/4] Building kernel..." -ForegroundColor Yellow
$env:RUSTFLAGS = "-C relocation-model=static -C link-arg=-no-pie"
cargo build --package racore @CargoFlags
if ($LASTEXITCODE -ne 0) { throw "Kernel build failed" }

# Restore default flags before building userland binaries.
$env:RUSTFLAGS = $OldRustFlags

# --- Step 2: Build coreutils ---
Write-Host "`n[2/4] Building coreutils..." -ForegroundColor Yellow
$Coreutils = @("racos-hello", "racos-echo", "racos-cat", "racos-true", "racos-false", "racos-sh", "racos-init", "racos-test", "racos-ls", "racos-wc", "racos-uptime", "racos-mkdir", "racos-rm", "racos-sleep", "racos-head", "racos-tail", "racos-env", "racos-basename", "racos-dirname", "racos-grep", "racos-cp", "racos-mv", "racos-cut", "racos-uniq", "racos-find", "racos-od", "racos-tee", "racos-hexdump", "racterm", "racos-dig", "racos-wget", "racos-mount", "racos-df", "racos-umount", "racos-mkfs-racfs")
foreach ($pkg in $Coreutils) {
    cargo build --package $pkg @CargoFlags -Z build-std=core,alloc -Z build-std-features=compiler-builtins-mem
    if ($LASTEXITCODE -ne 0) { throw "Build failed for $pkg" }
}

# --- Step 3: Assemble initramfs root ---
Write-Host "`n[3/4] Assembling initramfs..." -ForegroundColor Yellow
$InitramfsRoot = "$Root\initramfs-root"

# Clean and recreate
if (Test-Path $InitramfsRoot) { Remove-Item -Recurse -Force $InitramfsRoot }
New-Item -ItemType Directory -Force "$InitramfsRoot\bin" | Out-Null
New-Item -ItemType Directory -Force "$InitramfsRoot\sbin" | Out-Null
New-Item -ItemType Directory -Force "$InitramfsRoot\etc\racinit" | Out-Null
New-Item -ItemType Directory -Force "$InitramfsRoot\etc" | Out-Null

# Copy binaries — bin names match the [[bin]] name in Cargo.toml.
# Tuple form is "<cargo-bin-name>=<initramfs-name>"; plain entries map 1:1.
# Cargo rejects '.' in crate names so mkfs_racfs is renamed to mkfs.racfs here.
$BinList = @("hello", "echo", "cat", "true", "false", "sh", "racterm", "racos-test", "ls", "wc", "uptime", "mkdir", "rm", "sleep", "head", "tail", "env", "basename", "dirname", "grep", "cp", "mv", "cut", "uniq", "find", "od", "tee", "hexdump", "dig", "wget", "mount", "df", "umount", "mkfs_racfs=mkfs.racfs")
$SbinList = @("init")

foreach ($entry in $BinList) {
    $parts = $entry.Split("=")
    $srcName = $parts[0]
    $dstName = if ($parts.Length -gt 1) { $parts[1] } else { $parts[0] }
    $src = "$BinDir\$srcName"
    if (-not (Test-Path $src)) {
        Write-Warning "Binary not found: $srcName - skipping"
        continue
    }
    $dst = "$InitramfsRoot\bin\$dstName"
    Copy-Item $src $dst
    $size = (Get-Item $dst).Length
    $msg = "  bin/" + $dstName + " [" + $size + " bytes]"
    Write-Host $msg
}

foreach ($bin in $SbinList) {
    $src = "$BinDir\$bin"
    if (-not (Test-Path $src)) {
        Write-Warning "Binary not found: $bin - skipping"
        continue
    }
    $dst = "$InitramfsRoot\sbin\$bin"
    Copy-Item $src $dst
    $size = (Get-Item $dst).Length
    $msg = "  sbin/" + $bin + " [" + $size + " bytes]"
    Write-Host $msg
}

# --- Step 4: Pack initramfs image ---
Write-Host ""
Write-Host "[4/4] Packing initramfs image..." -ForegroundColor Yellow
$EspDir = Join-Path $Root "esp"
if (-not (Test-Path $EspDir)) { New-Item -ItemType Directory -Force $EspDir | Out-Null }

& "$Root\scripts\pack-initramfs.ps1" -RootDir $InitramfsRoot -Output (Join-Path $EspDir "initramfs.img")

# Restore caller environment.
$env:RUSTFLAGS = $OldRustFlags

Write-Host ""
Write-Host "=== Build complete ===" -ForegroundColor Green
Write-Host "Kernel:    $BinDir\racore"
$imgPath = Join-Path $EspDir "initramfs.img"
Write-Host "Initramfs: $imgPath"
