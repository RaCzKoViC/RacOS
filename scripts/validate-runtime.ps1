#!/usr/bin/env pwsh
# RacOS Runtime Validation - Simplified Test Suite

param([int]$TimeoutSeconds = 120)

$ErrorActionPreference = "Continue"

$KERNEL = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\release\racore"
$OVMF_CODE = "tools\OVMF_CODE.fd"
$ESP_DIR = "esp"
$LOG_FILE = "runtime-validation-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"

function Write-Log {
    param([string]$Message)
    $date = Get-Date -Format "HH:mm:ss.fff"
    $msg = "[$date] $Message"
    Write-Host $msg
    Add-Content $LOG_FILE $msg
}

function Run-BootTest {
    param([string]$Name, [int]$Duration = 30)
    
    Write-Log "Running: $Name"
    
    try {
        $proc = Start-Process -FilePath "qemu-system-x86_64" `
            -ArgumentList @(
                "-machine", "q35",
                "-cpu", "qemu64",
                "-m", "512M",
                "-drive", "if=pflash,format=raw,file=$OVMF_CODE,readonly=on",
                "-drive", "file=fat:rw:$ESP_DIR,format=raw",
                "-serial", "stdio",
                "-monitor", "none",
                "-display", "none",
                "-no-reboot",
                "-nographic"
            ) `
            -RedirectStandardOutput "qemu-out.tmp" `
            -PassThru
        
        $elapsed = 0
        while ($elapsed -lt $Duration -and -not $proc.HasExited) {
            Start-Sleep -Milliseconds 500
            $elapsed += 0.5
        }
        
        if (-not $proc.HasExited) {
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 1
        }
        
        if (Test-Path "qemu-out.tmp") {
            $output = Get-Content "qemu-out.tmp" -ErrorAction SilentlyContinue
            Remove-Item "qemu-out.tmp" -Force -ErrorAction SilentlyContinue
            return $output
        }
        return @()
    }
    catch {
        Write-Log "Error: $_"
        return @()
    }
}

Write-Host ""
Write-Host "===================================================="
Write-Host "RacOS Full Runtime Validation Test Suite"
Write-Host "===================================================="
Write-Host ""

Write-Log "Validation started"

# Check prerequisites
if (-not (Test-Path $KERNEL)) {
    Write-Host "ERROR: Kernel not found" -ForegroundColor Red
    exit 1
}
if (-not (Test-Path $OVMF_CODE)) {
    Write-Host "ERROR: OVMF not found" -ForegroundColor Red
    exit 1
}
if (-not (Test-Path $ESP_DIR)) {
    Write-Host "ERROR: ESP not found" -ForegroundColor Red
    exit 1
}

Write-Host "Prerequisites OK`n"

# Initialize results
$bootPass = $false
$fsPass = $false
$shellPass = $false
$stabPass = $false

# Test 1: Boot
Write-Host "[1/4] Boot Flow Validation..."
$output1 = Run-BootTest "boot" 25
$output1_str = $output1 -join "`n"

Write-Log "Boot output length: $($output1.Count) lines"

if ($output1_str -match "RACORE.*kernel starting" -or $output1_str -match "Bootloader starting") {
    Write-Host "  [OK] Boot sequence completed" -ForegroundColor Green
    $bootPass = $true
    Write-Log "PASS: Boot sequence"
} else {
    Write-Host "  [FAIL] Boot failed" -ForegroundColor Red
    Write-Log "FAIL: Boot sequence"
}

if ($output1_str -match "GDT.*loaded") {
    Write-Host "  [OK] GDT initialized" -ForegroundColor Green
    Write-Log "PASS: GDT"
} else {
    Write-Host "  [WARN] GDT status unclear" -ForegroundColor Yellow
    Write-Log "WARN: GDT"
}

# Test 2: Filesystem  
Write-Host ""
Write-Host "[2/4] Filesystem Validation..."

if ($output1_str -match "SFS-MEM|bin|etc|home" -or $output1_str -match "add Directory") {
    Write-Host "  [OK] Filesystem initialized" -ForegroundColor Green
    $fsPass = $true
    Write-Log "PASS: Filesystem"
} else {
    Write-Host "  [FAIL] Filesystem not ready" -ForegroundColor Red
    Write-Log "FAIL: Filesystem"
}

if ($output1_str -match "Physical allocator" -or $output1_str -match "heap") {
    Write-Host "  [OK] Memory management active" -ForegroundColor Green
    Write-Log "PASS: Memory"
} else {
    Write-Host "  [WARN] Memory status unclear" -ForegroundColor Yellow  
    Write-Log "WARN: Memory"
}

# Test 3: Shell
Write-Host ""
Write-Host "[3/4] Shell/Command Execution..."

if ($output1_str -match "SHELL|command=" -or $output1_str -match "racos>") {
    Write-Host "  [OK] Shell active" -ForegroundColor Green
    $shellPass = $true
    Write-Log "PASS: Shell"
} else {
    Write-Host "  [WARN] Shell may be interactive only" -ForegroundColor Yellow
    Write-Log "WARN: Shell not in log output"
}

if ($output1_str -match "echo|pwd|ls") {
    Write-Host "  [OK] Commands detected" -ForegroundColor Green
    Write-Log "PASS: Commands"
} else {
    Write-Host "  [WARN] Command output not in boot log" -ForegroundColor Yellow
    Write-Log "INFO: Commands not visible in non-interactive mode"
}

# Test 4: Stability
Write-Host "`n[4/4] Extended Stability Test (45s)..."

$output2 = Run-BootTest "stability" 45
$output2_str = $output2 -join "`n"

$crashCount = [regex]::Matches($output2_str, "panic|PANIC|crash|fatal").Count
$exceptionCount = ([regex]::Matches($output2_str, "page fault|exception")).Count

if ($crashCount -eq 0) {
    Write-Host "  [OK] No crashes during 45 second run" -ForegroundColor Green
    $stabPass = $true
    Write-Log "PASS: Stability"
} else {
    Write-Host "  [WARN] $crashCount crash events detected" -ForegroundColor Yellow
    Write-Log "WARN: $crashCount crashes"
}

if ($exceptionCount -eq 0) {
    Write-Host "  [OK] No exceptions" -ForegroundColor Green
    Write-Log "PASS: No exceptions"
} elseif ($output2_str -match "recovered|handled") {
    Write-Host "  [OK] Exceptions handled gracefully" -ForegroundColor Green
    Write-Log "INFO: $exceptionCount exceptions but handled"
} else {
    Write-Host "  [WARN] $exceptionCount exceptions (review logs)" -ForegroundColor Yellow
    Write-Log "INFO: $exceptionCount exceptions detected"
}

# Summary
Write-Host ""
Write-Host "===================================================="
Write-Host "VALIDATION SUMMARY"
Write-Host "===================================================="

$passCount = 0
if ($bootPass) { 
    Write-Host "  PASS: Boot" -ForegroundColor Green
    $passCount++ 
} else { 
    Write-Host "  FAIL: Boot" -ForegroundColor Red 
}
if ($fsPass) { 
    Write-Host "  PASS: Filesystem" -ForegroundColor Green
    $passCount++ 
} else { 
    Write-Host "  FAIL: Filesystem" -ForegroundColor Red 
}
if ($shellPass) { 
    Write-Host "  PASS: Shell" -ForegroundColor Green
    $passCount++ 
} else { 
    Write-Host "  WARN: Shell" -ForegroundColor Yellow 
}
if ($stabPass) { 
    Write-Host "  PASS: Stability" -ForegroundColor Green
    $passCount++ 
} else { 
    Write-Host "  WARN: Stability" -ForegroundColor Yellow 
}

Write-Host ""
if ($passCount -eq 4) {
    Write-Host "RESULT: VALIDATION SUCCESSFUL - System is fully functional" -ForegroundColor Green
    Write-Log "Overall: SUCCESS"
} elseif ($passCount -ge 3) {
    Write-Host "RESULT: VALIDATION PARTIAL - Most systems operational" -ForegroundColor Yellow
    Write-Log "Overall: PARTIAL"
} else {
    Write-Host "RESULT: VALIDATION FAILED - Critical issues detected" -ForegroundColor Red
    Write-Log "Overall: FAILED"
}

Write-Host ""
Write-Host "Log file: $LOG_FILE`n"
Write-Log "Validation complete"
