# RacOS - Pack initramfs binary image
#
# Creates a flat binary initramfs image from a source directory.
# The kernel will parse this at boot time (kernel/src/vfs/initramfs.rs).
#
# Binary format:
#   Magic:      b"RACRAMFS"  (8 bytes)
#   EntryCount: u32 LE       (4 bytes)
#   For each file entry:
#     NameLen:  u16 LE       (2 bytes)
#     Name:     [u8; NameLen] (UTF-8 relative path, e.g. "sbin/init")
#     DataLen:  u32 LE       (4 bytes)
#     Data:     [u8; DataLen]
#
# Usage: powershell -File scripts/pack-initramfs.ps1 [-RootDir initramfs-root] [-Output esp/initramfs.img]

param(
    [string]$RootDir = "initramfs-root",
    [string]$Output  = "esp\initramfs.img"
)

$ErrorActionPreference = "Stop"

# Collect all files under RootDir
$Files = @()
if (Test-Path $RootDir) {
    $ResolvedRoot = (Resolve-Path $RootDir).Path
    $Files = Get-ChildItem -Path $RootDir -Recurse -File | ForEach-Object {
        $relPath = $_.FullName.Substring($ResolvedRoot.Length).TrimStart('\', '/').Replace('\', '/')
        [PSCustomObject]@{ FullPath = $_.FullName; RelPath = $relPath }
    }
}

Write-Host "[initramfs] Packing $($Files.Count) file(s) from '$RootDir' -> '$Output'"

$bytes = [System.Collections.Generic.List[byte]]::new()

# Magic "RACRAMFS"
[void]$bytes.AddRange([System.Text.Encoding]::ASCII.GetBytes("RACRAMFS"))

# Entry count (u32 LE)
$countBytes = [BitConverter]::GetBytes([uint32]$Files.Count)
if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($countBytes) }
[void]$bytes.AddRange($countBytes)

foreach ($file in $Files) {
    $nameBytes = [System.Text.Encoding]::UTF8.GetBytes($file.RelPath)
    $data      = [System.IO.File]::ReadAllBytes($file.FullPath)

    # Name length (u16 LE)
    $nameLenBytes = [BitConverter]::GetBytes([uint16]$nameBytes.Length)
    if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($nameLenBytes) }
    [void]$bytes.AddRange($nameLenBytes)

    # Name bytes
    [void]$bytes.AddRange($nameBytes)

    # Data length (u32 LE)
    $dataLenBytes = [BitConverter]::GetBytes([uint32]$data.Length)
    if (-not [BitConverter]::IsLittleEndian) { [Array]::Reverse($dataLenBytes) }
    [void]$bytes.AddRange($dataLenBytes)

    # Data bytes
    [void]$bytes.AddRange($data)

    Write-Host "  + $($file.RelPath) ($($data.Length) bytes)"
}

# Write output file
$outDir = Split-Path -Parent $Output
if ($outDir -and -not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Force $outDir | Out-Null
}

$outPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
[System.IO.File]::WriteAllBytes($outPath, $bytes.ToArray())

Write-Host "[initramfs] Done: $($bytes.Count) bytes -> $outPath"
