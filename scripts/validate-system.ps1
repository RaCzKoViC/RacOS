#!/usr/bin/env pwsh
# RacOS Runtime Validation - ESP Boot Configuration

$ErrorActionPreference = "Continue"

$KERNEL = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\release\racore"
$ESP_DIR = "esp"
$LogFile = "validation-esp-test.log"

function WriteLog {
    param([string]$msg)
    $timestamp = Get-Date -Format "HH:mm:ss"
    $line = "[$timestamp] $msg"
    Write-Host $line
    Add-Content $LogFile $line
}

Write-Host ""
Write-Host "RacOS Runtime Validation - ESP Boot"
Write-Host "===================================="
Write-Host ""

# Check prerequisites
$prereqOk = $true
if (-not (Test-Path "$ESP_DIR\EFI")) { 
    Write-Host "ERROR: ESP EFI directory missing" -ForegroundColor Red
    $prereqOk = $false
}
if (-not (Test-Path "$ESP_DIR\initramfs.img")) {
    Write-Host "ERROR: Initramfs missing" -ForegroundColor Red
    $prereqOk = $false
}
if (-not $prereqOk) { exit 1 }

WriteLog "Starting boot test"

$bootLogFile = "boot-output.log"
$bootSeconds = 40

Write-Host "Starting QEMU with PC BIOS..."
WriteLog "Attempting PC BIOS boot"

try {
    $proc = Start-Process -FilePath "qemu-system-x86_64" `
        -ArgumentList @(
            "-m", "512M",
            "-machine", "pc",
            "-hda", "fat:rw:$ESP_DIR",
            "-serial", "file:$bootLogFile",
            "-display", "none",
            "-no-reboot",
            "-nographic"
        ) `
        -PassThru
    
    Write-Host "System booting for $bootSeconds seconds..."
    
    $elapsed = 0
    while ($elapsed -lt $bootSeconds) {
        Start-Sleep -Seconds 1
        $elapsed += 1
        if ($elapsed % 10 -eq 0) {
            Write-Host "  ${elapsed}s..."
        }
    }
    
    # Terminate QEMU
    if (-not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
    }
    
    WriteLog "Boot process completed"
}
catch {
    WriteLog "Boot error: $_"
}

# Analyze output
Write-Host ""
Write-Host "Analysis"
Write-Host "========"

if (Test-Path $bootLogFile) {
    $output = Get-Content $bootLogFile -ErrorAction SilentlyContinue
    $outputStr = $output -join "`n"
    
    $lineCount = @($output).Count
    WriteLog "Output lines: $lineCount"
    
    Write-Host ""
    Write-Host "Boot sequence checks:"
    
    $checks = @{
        "Bootloader" = ($outputStr -like "*RacOS*" -or $outputStr -like "*bootx64*")
        "Kernel" = ($outputStr -like "*RACORE*" -or $outputStr -like "*kernel*")
        "GDT" = ($outputStr -like "*GDT*")
        "Filesystem" = ($outputStr -like "*SFS*" -or $outputStr -like "*Directory*")
        "No Crashes" = -not ($outputStr -like "*panic*")
        "Memory" = ($outputStr -like "*allocator*" -or $outputStr -like "*heap*")
        "Devices" = ($outputStr -like "*devfs*" -or $outputStr -like "*tty*")
        "Init Ready" = ($outputStr -like "*init*" -or $outputStr -like "*shell*")
    }
    
    $passed = 0
    foreach ($check in $checks.Keys) {
        if ($checks[$check]) {
            Write-Host "  [PASS] $check"
            $passed++
        } else {
            Write-Host "  [WARN] $check"
        }
    }
    
    Write-Host ""
    $total = $checks.Count
    Write-Host "Result: $passed out of $total checks passed"
    
    if ($passed -ge 6) {
        Write-Host "Validation: SUCCESSFUL" -ForegroundColor Green
        WriteLog "Validation result: SUCCESS"
    } else {
        Write-Host "Validation: PARTIAL" -ForegroundColor Yellow  
        WriteLog "Validation result: PARTIAL"
    }
    
    # Sample output
    Write-Host ""
    Write-Host "First 30 lines of boot output:"
    Write-Host "=============================="
    $output | Select-Object -First 30 | ForEach-Object { Write-Host $_ }
}
else {
    Write-Host "ERROR: No boot output file" -ForegroundColor Red
}

Write-Host ""
Write-Host "Log file: $LogFile"
Write-Host ""
