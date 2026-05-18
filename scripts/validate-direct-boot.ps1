#!/usr/bin/env pwsh
# RacOS Runtime Validation - Direct Boot Test Suite

param([int]$Duration = 120)

$ErrorActionPreference = "Continue"

$KERNEL = "C:\Users\Maciej\RacOS-target\x86_64-unknown-none\release\racore"
$LOG_FILE = "runtime-validation-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"

function Write-Log {
    param([string]$Message)
    $date = Get-Date -Format "HH:mm:ss.fff"
    $msg = "[$date] $Message"
    $msg
    Add-Content $LOG_FILE $msg
}

function Run-BootTest {
    param([string]$Name, [int]$Seconds = 30)
    
    Write-Host "[*] Running: $Name ($Seconds seconds)..."
    
    try {
        $outfile = "boot-$Name.log"
        $proc = Start-Process -FilePath "qemu-system-x86_64" `
            -ArgumentList @(
                "-machine", "q35",
                "-cpu", "qemu64",
                "-m", "512M",
                "-serial", "file:$outfile",
                "-display", "none",
                "-no-reboot",
                "-nographic",
                "-kernel", $KERNEL
            ) `
            -PassThru
        
        $elapsed = 0
        while ($elapsed -lt $Seconds -and -not $proc.HasExited) {
            Start-Sleep -Milliseconds 500
            $elapsed += 0.5
        }
        
        if (-not $proc.HasExited) {
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 500
        }
        
        $output = Get-Content $outfile -ErrorAction SilentlyContinue
        Write-Log "Boot output ($Name): $($output.Count) lines"
        return $output
    }
    catch {
        Write-Log "ERROR: $_"
        return @()
    }
}

Write-Host ""
Write-Host "===================================================="
Write-Host "RacOS Runtime Validation - Direct Boot"
Write-Host "===================================================="
Write-Host ""

Write-Log "Validation started - Direct kernel boot"

if (-not (Test-Path $KERNEL)) {
    Write-Host "ERROR: Kernel not found at $KERNEL" -ForegroundColor Red
    exit 1
}

Write-Host "Kernel:  $KERNEL"
Write-Host "Log:     $LOG_FILE"
Write-Host ""

# Test 1: Quick Boot
Write-Host "[TEST 1/5] Quick Boot (20 seconds)..."
$boot1 = Run-BootTest "quick" 20
$boot1_str = $boot1 -join "`n"

$tests = @{
    "Bootloader" = ($boot1_str -match "Bootloader|Kernel loaded|kernel starting")
    "GDT" = ($boot1_str -match "GDT")
    "Memory" = ($boot1_str -match "allocator|heap|MiB")
    "Filesystem" = ($boot1_str -match "SFS|Directory")
}

$pass = 0
foreach ($test in $tests.Keys) {
    if ($tests[$test]) {
        Write-Host "  [PASS] $test"
        $pass++
    } else {
        Write-Host "  [WARN] $test"
    }
}

Write-Log "Test 1 results: $pass/4 passed"

# Test 2: Extended Boot
Write-Host ""
Write-Host "[TEST 2/5] Extended Boot (30 seconds)...  "
$boot2 = Run-BootTest "extended" 30
$boot2_str = $boot2 -join "`n"

$crashes = ([regex]::Matches($boot2_str, "panic|crash")).Count
Write-Host "  [INFO] Crashes/Panics: $crashes"
Write-Log "Crashes detected: $crashes"

# Test 3: Command Execution Traces
Write-Host ""
Write-Host "[TEST 3/5] Command Execution Check..."
$cmdTests = @{
    "Shell loaded" = ($boot2_str -match "shell|SHELL")
    "Commands" = ($boot2_str -match "echo|pwd|ls")
    "Input handling" = ($boot2_str -match "SHELL.*buffer|input")
}

$pass2 = 0
foreach ($test in $cmdTests.Keys) {
    if ($cmdTests[$test]) {
        Write-Host "  [PASS] $test"
        $pass2++
    } else {
        Write-Host "  [WARN] $test"
    }
}

# Test 4: System Infrastructure
Write-Host ""
Write-Host "[TEST 4/5] System Infrastructure..."
$infraTests = @{
    "IDT configured" = ($boot1_str -match "IDT")
    "Interrupts" = ($boot1_str -match "interrupt|IRQ|PIC")
    "Scheduler" = ($boot1_str -match "scheduler|task|process")
    "TTY/PTY ready" = ($boot1_str -match "TTY|pty|terminal")
}

$pass3 = 0
foreach ($test in $infraTests.Keys) {
    if ($infraTests[$test]) {
        Write-Host "  [PASS] $test"
        $pass3++
    } else {
        Write-Host "  [INFO] $test (may not be in boot log)"
    }
}

# Test 5: Long Stability
Write-Host ""
Write-Host "[TEST 5/5] Stability Test (45 seconds)..."
$boot3 = Run-BootTest "stability" 45
$boot3_str = $boot3 -join "`n"

$warns = [regex]::Matches($boot3_str, "WARN|warn").Count
$errors = [regex]::Matches($boot3_str, "ERROR|error|panic").Count

Write-Host "  [INFO] Warnings: $warns"
Write-Host "  [INFO] Errors: $errors"

if ($errors -eq 0) {
    Write-Host "  [PASS] No fatal errors during extended run"
    $stabPass = $true
} else {
    Write-Host "  [WARN] Errors detected"
    $stabPass = $false
}

# Summary
Write-Host ""
Write-Host "===================================================="
Write-Host "VALIDATION SUMMARY"
Write-Host "===================================================="

$totalPass = 0
if ($tests["Bootloader"]) { Write-Host "  Boot sequence: PASS"; $totalPass++ } else { Write-Host "  Boot sequence: FAIL" }
if ($tests["Memory"]) { Write-Host "  Memory mgmt: PASS"; $totalPass++ } else { Write-Host "  Memory mgmt: WARN" }
if ($tests["Filesystem"]) { Write-Host "  Filesystem: PASS"; $totalPass++ } else { Write-Host "  Filesystem: WARN" }
if ($cmdTests["Shell loaded"]) { Write-Host "  Shell: PASS"; $totalPass++ } else { Write-Host "  Shell: WARN" }
if ($stabPass) { Write-Host "  Stability: PASS"; $totalPass++ } else { Write-Host "  Stability: WARN" }

Write-Host ""
if ($totalPass -ge 4) {
    Write-Host "RESULT: VALIDATION SUCCESSFUL" -ForegroundColor Green
    Write-Log "Validation: SUCCESS - $totalPass/5 core systems passed"
} elseif ($totalPass -ge 3) {
    Write-Host "RESULT: VALIDATION PARTIAL - Most systems functional" -ForegroundColor Yellow
    Write-Log "Validation: PARTIAL - $totalPass/5 systems passing"
} else {
    Write-Host "RESULT: VALIDATION FAILED" -ForegroundColor Red
    Write-Log "Validation: FAILED - Only $totalPass/5 systems passing"
}

Write-Host ""
Write-Host "Saved to: $LOG_FILE"
Write-Host ""
