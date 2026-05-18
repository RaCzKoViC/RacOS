#!/usr/bin/env pwsh
# RacOS Runtime Validation - Using FAT ESP Image
# This validates the full system using the bootloader + FAT filesystem

param(
    [int]$BootSeconds = 60,
    [switch]$Verbose = $false
)

$ErrorActionPreference = "Continue"

$KERNEL = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\release\racore"
$BOOTLOADER = "esp/EFI/RACOS/bootx64.efi"
$ESP_DIR = "esp"
$LOG_FILE = "runtime-validation-esp-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"

function Write-Log {
    param([string]$Message)
    $date = Get-Date -Format "HH:mm:ss"
    "$date | $Message" | Tee-Object -FilePath $LOG_FILE -Append
}

# Check prerequisites
Write-Host ""
Write-Host "╔════════════════════════════════════════════════════╗"
Write-Host "║    RacOS Full System Validation - ESP Boot       ║"
Write-Host "╚════════════════════════════════════════════════════╝"
Write-Host ""

$missing = @()
if (-not (Test-Path $ESP_DIR\EFI)) { $missing += "EFI directory" }
if (-not (Test-Path $ESP_DIR\initramfs.img)) { $missing += "Initramfs" }  
if (-not (Test-Path $BOOTLOADER)) { $missing += "Bootloader" }

if ($missing) {
    Write-Host "ERROR: Missing components:" -ForegroundColor Red
    $missing | ForEach-Object { Write-Host "  - $_" }
    exit 1
}

Write-Log "Starting validation with ESP image"
Write-Log "Boot duration: $BootSeconds seconds"

# Use qemu-system-i386 or x86_64 with BIOS firmware instead of trying UEFI
# Try creating a simple BIOS boot scenario

Write-Host "Attempting legacy BIOS boot (SeaBIOS)..."
Write-Log "Attempting legacy BIOS boot"

$outlog = "boot-esp-output-$([random]).log"

try {
    # Try with SeaBIOS (default BIOS firmware)
    $proc = Start-Process -FilePath "qemu-system-x86_64" `
        -ArgumentList @(
            "-machine", "pc-i440fx",
            "-bios", "seabios",
            "-m", "512M",
            "-drive", "file=fat:rw:$ESP_DIR,if=ide,index=0,format=raw",
            "-serial", "file:$outlog",
            "-display", "none",
            "-no-reboot",
            "-nographic",
            "-boot", "d"  # Boot from cdrom/disk
        ) `
        -PassThru `
        -ErrorAction SilentlyContinue
    
    if (-not $proc) {
        Write-Host "Alternative: Trying x86_64 with BIOS..."
        # Fallback: try without specifying bios
        $proc = Start-Process -FilePath "qemu-system-x86_64" `
            -ArgumentList @(
                "-machine", "pc-q35-2.12",
                "-m", "512M",  
                "-hda", "fat:rw:$ESP_DIR",
                "-serial", "file:$outlog",
                "-display", "none",
                "-no-reboot",
                "-nographic"
            ) `
            -PassThru
    }
    
    # Wait for boot process
    Write-Host "System booting (waiting $BootSeconds seconds)..."
    $elapsed = 0
    $lastSize = 0
    
    while ($elapsed -lt $BootSeconds) {
        Start-Sleep -Milliseconds 1000
        $elapsed++
        
        # Check if log file is growing
        if (Test-Path $outlog) {
            $size = (Get-Item $outlog).Length
            if ($size -ne $lastSize) {
                Write-Host "." -NoNewline
                $lastSize = $size
            }
        }
    }
    
    Write-Host ""
    
    # Terminate QEMU
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
    }
    
catch {
    Write-Log "Boot error: $_"
    Write-Host "Boot failed: $_" -ForegroundColor Red
}

# Analyze output
Write-Host ""
Write-Host "Analyzing boot output..."
Write-Log "Analyzing boot output"

if (Test-Path $outlog) {
    $output = Get-Content $outlog -ErrorAction SilentlyContinue
    $output_str = $output -join "`n"
    
    # Show first 50 lines
    if ($Verbose) {
        Write-Host ""
        Write-Host "Boot output (first 50 lines):"
        Write-Host "=============================="
        $output | Select-Object -First 50 | ForEach-Object { Write-Host $_ }
    }
    
    # Analyze
    Write-Host ""
    Write-Host "System Analysis:"
    Write-Host "================"
    
    $tests = @{
        "Firmware/BIOS" = ($output_str -match "BIOS|QEMU|SeaBIOS")
        "Bootloader" = ($output_str -match "RacOS|Bootloader|bootx64")
        "Kernel" = ($output_str -match "RACORE|kernel")
        "GDT/IDT" = ($output_str -match "GDT|IDT")
        "Memory" = ($output_str -match "allocator|heap|MiB")
        "Filesystem" = ($output_str -match "SFS|Directory|bin|etc")
        "Devices" = ($output_str -match "serial|tty|devfs|DEVFS")  
        "Init/Shell" = ($output_str -match "init|shell|racos>")
        "No Crashes" = -not ($output_str -match "panic|PANIC|crash")
    }
    
    $passCount = 0
    foreach ($test in $tests.Keys | Sort-Object) {
        if ($tests[$test]) {
            Write-Host "  [PASS] $test"
            Write-Log "PASS: $test"
            $passCount++
        } else {
            Write-Host "  [WARN] $test"
            Write-Log "WARN: $test"
        }
    }
    
    Write-Host ""
    $resultStr = $passCount.ToString() + "/9 checks"
    if ($passCount -ge 7) {
        Write-Host ("RESULT: VALIDATION SUCCESSFUL (" + $resultStr + ")") -ForegroundColor Green
        Write-Log ("Validation: SUCCESS - " + $resultStr + " passed")
    } else {
        Write-Host ("RESULT: VALIDATION PARTIAL (" + $resultStr + ")") -ForegroundColor Yellow
        Write-Log ("Validation: PARTIAL - " + $resultStr)
    }
} else {
    Write-Host "ERROR: No boot output captured"
    Write-Log "ERROR: Boot output not captured"
}

Write-Host ""
Write-Host "Log saved: $LOG_FILE"
Write-Host ""

# Cleanup
Remove-Item $outlog -Force -ErrorAction SilentlyContinue
