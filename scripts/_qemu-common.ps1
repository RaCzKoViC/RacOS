# Shared helpers for the RacOS QEMU launch scripts.
#
# QEMU's `-drive file=fat:rw:<path>`, `-drive file=<path>` and `-serial file:<path>`
# options break when <path> contains a space: the option string is tokenised on
# whitespace and QEMU only sees the part before the space (e.g. it reports
# "Could not read directory D:\OS" for an ESP under "D:\OS project\esp").
# These helpers hand QEMU a space-free path that still resolves to the real files.

function Resolve-SpacelessPath {
    <#
    .SYNOPSIS
        Return a space-free path that resolves to exactly $Path.
    .DESCRIPTION
        Strategy, in order:
          1. $Path unchanged if it already has no space.
          2. Its Windows 8.3 short name, when the volume generates them.
          3. A directory junction on the same volume pointing at $Path.
        Files written through the returned path land in the real target, so
        callers can keep using the project's actual esp/disk/serial files.
    #>
    param([Parameter(Mandatory = $true)][string]$Path)

    if ($Path -notmatch ' ') { return $Path }

    # Try the 8.3 short name first — cheapest, no side effects.
    $fso = New-Object -ComObject Scripting.FileSystemObject
    if (Test-Path -LiteralPath $Path -PathType Container) {
        $short = $fso.GetFolder($Path).ShortPath
    } elseif (Test-Path -LiteralPath $Path) {
        $short = $fso.GetFile($Path).ShortPath
    } else {
        $short = $null
    }
    if ($short -and $short -notmatch ' ') { return $short }

    # 8.3 generation is disabled on this volume — fall back to a junction.
    # Only directories can be junction targets; a spaced *file* path is not
    # supported here (callers pass directories, e.g. the project root).
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "Resolve-SpacelessPath: '$Path' has a space, no 8.3 name, and is not a directory."
    }

    # Deterministic per-target name so reruns reuse the same junction and
    # different projects never collide.
    $sha = New-Object System.Security.Cryptography.SHA1Managed
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Path.ToLowerInvariant())
    $hash = ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').Substring(0, 8)
    $volume = [System.IO.Path]::GetPathRoot($Path).TrimEnd('\')   # e.g. "D:"

    # Candidate junction locations, in preference order. The volume root keeps
    # the path shortest, but a drive root is frequently not user-writable (a
    # default Windows ACL grants plain users read+execute only), so fall back
    # to TEMP, which always is. Without the fallback every QEMU script dies
    # with "Odmowa dostepu / Access denied" on such a machine.
    $candidates = @("$volume\racos_qemu_$hash")
    if ($env:TEMP) { $candidates += (Join-Path $env:TEMP "racos_qemu_$hash") }

    $lastError = $null
    foreach ($link in $candidates) {
        if (Test-Path -LiteralPath $link) {
            $item = Get-Item -LiteralPath $link -Force
            $isReparse = ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
            if (-not $isReparse) {
                throw "Resolve-SpacelessPath: '$link' exists but is not a junction; refusing to touch it."
            }
            # Same hash => same target, safe to reuse as-is.
            return $link
        }
        try {
            New-Item -ItemType Junction -Path $link -Target $Path -ErrorAction Stop | Out-Null
            return $link
        } catch {
            $lastError = $_
        }
    }

    throw ("Resolve-SpacelessPath: could not create a space-free junction for '$Path'. " +
           "Tried: " + ($candidates -join ', ') + ". Last error: $lastError")
}

function Find-QemuPaths {
    <#
    .SYNOPSIS
        Locate qemu-system-x86_64.exe and the OVMF code firmware.
    .OUTPUTS
        Hashtable @{ Exe = <path>; ExeGui = <path>; Ovmf = <path> }.
        - Exe    : the console build of qemu-system-x86_64; use it when serial is
                   wired to stdio so the launching terminal stays interactive.
        - ExeGui : the windowed build (qemu-system-x86_64w.exe) which does NOT
                   spawn an extra console window; use it for the graphical,
                   self-contained interactive console. Falls back to Exe when the
                   windowed build is not present.
        The OVMF path is required to be space-free (it is fed into a QEMU
        `-drive file=` option); installs under "C:\Program Files" are therefore
        only used for the executable, not the firmware.
    #>
    $exeCandidates = @(
        'D:\qemu\qemu-system-x86_64.exe',
        'C:\Program Files\qemu\qemu-system-x86_64.exe'
    )
    $exe = $exeCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $exe) {
        throw "qemu-system-x86_64.exe not found. Looked in: $($exeCandidates -join '; ')"
    }

    $qemuDir = Split-Path -Parent $exe
    # The 'w' suffix is QEMU's GUI-subsystem build: it shows the display window
    # without opening a console window alongside it.
    $exeGuiCandidate = Join-Path $qemuDir 'qemu-system-x86_64w.exe'
    $exeGui = if (Test-Path -LiteralPath $exeGuiCandidate) { $exeGuiCandidate } else { $exe }
    $ovmfCandidates = @(
        'D:\qemu\share\edk2-x86_64-code.fd',
        (Join-Path $qemuDir 'share\edk2-x86_64-code.fd')
    ) | Where-Object { $_ -notmatch ' ' }
    $ovmf = $ovmfCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $ovmf) {
        throw "OVMF firmware (edk2-x86_64-code.fd) not found at a space-free path. Looked in: $($ovmfCandidates -join '; ')"
    }

    return @{ Exe = $exe; ExeGui = $exeGui; Ovmf = $ovmf }
}
