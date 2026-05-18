#!/usr/bin/env python3
"""
RacOS initramfs packer — Linux/CI compatible version.

Creates the same binary format as scripts/pack-initramfs.ps1:
  Magic:      b"RACRAMFS"  (8 bytes)
  EntryCount: u32 LE       (4 bytes)
  For each file:
    NameLen:  u16 LE       (2 bytes)
    Name:     [u8; NameLen] (UTF-8 relative path)
    DataLen:  u32 LE       (4 bytes)
    Data:     [u8; DataLen]

Usage: python3 scripts/pack-initramfs.py <root_dir> <output.img>
"""

import os
import sys
import struct

def pack(root_dir: str, output: str) -> None:
    files = []
    if os.path.isdir(root_dir):
        for dirpath, _dirnames, filenames in os.walk(root_dir):
            for fname in sorted(filenames):
                full = os.path.join(dirpath, fname)
                rel  = os.path.relpath(full, root_dir).replace(os.sep, '/')
                files.append((rel, full))

    print(f"[initramfs] Packing {len(files)} file(s) from '{root_dir}' -> '{output}'")

    buf = bytearray()
    buf += b"RACRAMFS"
    buf += struct.pack('<I', len(files))

    for rel, full in files:
        name_bytes = rel.encode('utf-8')
        with open(full, 'rb') as f:
            data = f.read()
        buf += struct.pack('<H', len(name_bytes))
        buf += name_bytes
        buf += struct.pack('<I', len(data))
        buf += data
        print(f"  + {rel} ({len(data)} bytes)")

    os.makedirs(os.path.dirname(os.path.abspath(output)) or '.', exist_ok=True)
    with open(output, 'wb') as f:
        f.write(buf)

    print(f"[initramfs] Done: {len(buf)} bytes -> {output}")

if __name__ == '__main__':
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <root_dir> <output.img>")
        sys.exit(1)
    pack(sys.argv[1], sys.argv[2])
