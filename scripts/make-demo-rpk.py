#!/usr/bin/env python3
"""Generate share/demo.rpk - a sample package shipped in the initramfs.

It exists so "install something and see it work" is possible on a fresh
image without any network: the v0.3 milestone smoke installs it on boot 1
and expects `rpkg list` to still know it on boot 2, after a reboot, off
the persistent /var/lib/rpkg.

Layout (pkg/rpkg/src/lib.rs parse_header): 56-byte header -
  0  "RPK\x01"          magic + format version
  8  manifest_offset    u64 LE
  16 manifest_size
  24 signature_offset
  32 signature_size
  40 data_offset
  48 data_size
- followed by the three sections back to back. The signature is a
placeholder byte: signing lands with T4.1 crypto (ADR-019).
"""
import struct
import sys

MANIFEST = b'[package]\nname = "demo"\nversion = "0.3.0"\narch = "x86_64"\n'
SIGNATURE = b"\x00"
DATA = b"Hello from demo.rpk - installed by rpkg, survived a reboot.\n"

def build() -> bytes:
    mo = 56
    so = mo + len(MANIFEST)
    do = so + len(SIGNATURE)
    header = b"RPK\x01" + b"\x00" * 4 + struct.pack(
        "<6Q", mo, len(MANIFEST), so, len(SIGNATURE), do, len(DATA)
    )
    assert len(header) == 56
    return header + MANIFEST + SIGNATURE + DATA

if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "initramfs-root/share/demo.rpk"
    blob = build()
    with open(out, "wb") as f:
        f.write(blob)
    print(f"[demo-rpk] {len(blob)} bytes -> {out}")
