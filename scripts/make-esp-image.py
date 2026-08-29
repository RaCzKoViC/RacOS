#!/usr/bin/env python3
"""Build a bootable ESP disk image from the esp/ directory (ROADMAP 3.4).

Output: an MBR-partitioned disk image with one FAT32 partition (type 0x0C,
boot flag set) holding EFI/BOOT/BOOTX64.EFI, racore.elf and initramfs.img.
OVMF boots it as a USB mass-storage device in QEMU, and the same image can
be written raw to a USB stick for real UEFI hardware - see
docs/BOOT-MEDIA.md for that procedure and for what to expect there.

Why this exists at all: CI builds its ESP image with mtools, which do not
exist on a stock Windows box, and QEMU's `fat:rw:esp` auto-mode is not a
real filesystem - its writeback path occasionally leaves the second boot
unable to load Boot0002 at all (see ci.yml). This is a from-scratch FAT32
formatter in ~250 lines of Python instead: no dependencies, same output
everywhere. It writes long-file-name entries because `initramfs.img` does
not fit 8.3, and UEFI firmware looks names up case-insensitively via LFN.

Usage:
  python scripts/make-esp-image.py [esp_dir] [output.img] [--size-mib N]
"""

import os
import struct
import sys

SECTOR = 512
SEC_PER_CLUS = 8            # 4 KiB clusters
RSVD_SECS = 32
NUM_FATS = 2
PART_START_LBA = 2048       # partition starts at 1 MiB, the alignment
                            # every partitioning tool has used for a decade


class Fat32Volume:
    """A FAT32 filesystem built in memory, then written in one pass."""

    def __init__(self, total_sectors: int):
        self.total_sectors = total_sectors
        # FAT size and cluster count depend on each other; iterate to a
        # fixed point exactly like racfs sizes its allocation bitmap.
        fat_secs = 1
        for _ in range(8):
            data_secs = total_sectors - RSVD_SECS - NUM_FATS * fat_secs
            clusters = data_secs // SEC_PER_CLUS
            needed = (clusters + 2) * 4  # 4 bytes per FAT32 entry
            needed_secs = (needed + SECTOR - 1) // SECTOR
            if needed_secs <= fat_secs:
                break
            fat_secs = needed_secs
        self.fat_secs = fat_secs
        self.data_secs = total_sectors - RSVD_SECS - NUM_FATS * fat_secs
        self.clusters = self.data_secs // SEC_PER_CLUS
        if self.clusters < 65525:
            # Below this count the volume would legally be FAT16 and
            # firmware may treat it as such; refuse rather than build an
            # image that boots on one machine and not another.
            raise SystemExit(
                f"volume too small for FAT32: {self.clusters} clusters < 65525"
            )
        self.fat = [0] * (self.clusters + 2)
        self.fat[0] = 0x0FFFFFF8
        self.fat[1] = 0x0FFFFFFF
        self.cluster_data = {}   # cluster index -> bytes (<= cluster size)
        self.next_free = 3       # cluster 2 is the root directory
        self.alloc_chain(2, 1)   # root starts as a single cluster

    def cluster_size(self) -> int:
        return SEC_PER_CLUS * SECTOR

    def alloc_chain(self, first: int, count: int) -> None:
        prev = None
        c = first
        for i in range(count):
            if prev is not None:
                self.fat[prev] = c
            self.fat[c] = 0x0FFFFFFF
            prev = c
            c = self.next_free if i + 1 < count else c
        # advance the bump allocator past everything claimed
        self.next_free = max(self.next_free, first + count)

    def alloc(self, count: int) -> int:
        first = self.next_free
        for i in range(count):
            c = first + i
            if c >= len(self.fat):
                raise SystemExit("image full - raise --size-mib")
            self.fat[c] = c + 1 if i + 1 < count else 0x0FFFFFFF
        self.next_free = first + count
        return first

    def write_chain(self, first: int, data: bytes) -> None:
        cs = self.cluster_size()
        c = first
        off = 0
        while True:
            self.cluster_data[c] = data[off : off + cs]
            off += cs
            nxt = self.fat[c]
            if nxt >= 0x0FFFFFF8:
                break
            c = nxt


def lfn_checksum(short: bytes) -> int:
    s = 0
    for b in short:
        s = (((s & 1) << 7) + (s >> 1) + b) & 0xFF
    return s


def short_name_for(name: str, taken: set) -> bytes:
    """11-byte 8.3 alias. Uses NAME~N when the base does not fit."""
    base, dot, ext = name.upper().partition(".")
    ext = (ext.replace(".", ""))[:3]
    clean = "".join(ch for ch in base if ch.isalnum())
    if len(clean) <= 8 and clean == base:
        cand = (clean.ljust(8) + ext.ljust(3)).encode("ascii")
        if cand not in taken:
            taken.add(cand)
            return cand
    for n in range(1, 100):
        tail = f"~{n}"
        cand = ((clean[: 8 - len(tail)] + tail).ljust(8) + ext.ljust(3)).encode("ascii")
        if cand not in taken:
            taken.add(cand)
            return cand
    raise SystemExit(f"cannot make a unique 8.3 alias for {name}")


def dir_entries(name: str, short: bytes, attr: int, first_cluster: int, size: int) -> bytes:
    """LFN entries (always, to preserve the exact name) + the 8.3 entry."""
    out = b""
    utf16 = name.encode("utf-16-le") + b"\x00\x00"
    utf16 += b"\xff" * ((26 - len(utf16) % 26) % 26)
    chunks = [utf16[i : i + 26] for i in range(0, len(utf16), 26)]
    csum = lfn_checksum(short)
    for i, chunk in enumerate(reversed(chunks)):
        seq = len(chunks) - i
        if i == 0:
            seq |= 0x40
        out += (
            bytes([seq])
            + chunk[0:10]
            + bytes([0x0F, 0x00, csum])
            + chunk[10:22]
            + b"\x00\x00"
            + chunk[22:26]
        )
    out += struct.pack(
        "<11sBBBHHHHHHHI",
        short, attr, 0, 0, 0, 0x21, 0x21, (first_cluster >> 16) & 0xFFFF,
        0, 0x21, first_cluster & 0xFFFF, size,
    )
    return out


def dot_entries(self_cluster: int, parent_cluster: int) -> bytes:
    def one(name: bytes, cluster: int) -> bytes:
        return struct.pack(
            "<11sBBBHHHHHHHI",
            name, 0x10, 0, 0, 0, 0x21, 0x21, (cluster >> 16) & 0xFFFF,
            0, 0x21, cluster & 0xFFFF, 0,
        )
    # ".." pointing at the root is stored as cluster 0, per the spec
    parent = 0 if parent_cluster == 2 else parent_cluster
    return one(b".          ", self_cluster) + one(b"..         ", parent)


def build_volume(esp_dir: str, total_sectors: int) -> Fat32Volume:
    vol = Fat32Volume(total_sectors)
    cs = vol.cluster_size()

    def add_tree(host_dir: str, dir_cluster: int, parent_cluster: int) -> None:
        entries = b"" if dir_cluster == 2 else dot_entries(dir_cluster, parent_cluster)
        taken: set = set()
        for entry in sorted(os.listdir(host_dir)):
            # NvVars is OVMF's variable store, written back by the firmware
            # when QEMU runs with fat:rw:esp. It is host-side state, not part
            # of RacOS, and shipping a snapshot of it on a USB stick would
            # hand every machine this machine's boot variables.
            if entry == "NvVars":
                continue
            path = os.path.join(host_dir, entry)
            short = short_name_for(entry, taken)
            if os.path.isdir(path):
                sub = vol.alloc(1)
                entries += dir_entries(entry, short, 0x10, sub, 0)
                add_tree(path, sub, dir_cluster)
            else:
                with open(path, "rb") as f:
                    data = f.read()
                count = max(1, (len(data) + cs - 1) // cs)
                first = vol.alloc(count)
                vol.write_chain(first, data)
                entries += dir_entries(entry, short, 0x20, first, len(data))
                print(f"[esp-image]   {entry} ({len(data)} bytes, {count} clusters)")
        # grow the directory chain to fit its entries
        count = max(1, (len(entries) + cs - 1) // cs)
        if dir_cluster == 2 and count > 1:
            extra = vol.alloc(count - 1)
            vol.fat[2] = extra
        vol.write_chain(2 if dir_cluster == 2 else dir_cluster, entries)

    add_tree(esp_dir, 2, 2)
    return vol


def boot_sector(vol: Fat32Volume, hidden: int) -> bytes:
    bs = bytearray(SECTOR)
    bs[0:3] = b"\xeb\x58\x90"
    bs[3:11] = b"RACOSESP"
    struct.pack_into("<H", bs, 11, SECTOR)
    bs[13] = SEC_PER_CLUS
    struct.pack_into("<H", bs, 14, RSVD_SECS)
    bs[16] = NUM_FATS
    struct.pack_into("<H", bs, 17, 0)          # RootEntCnt (FAT32: 0)
    struct.pack_into("<H", bs, 19, 0)          # TotSec16
    bs[21] = 0xF8
    struct.pack_into("<H", bs, 22, 0)          # FATSz16
    struct.pack_into("<H", bs, 24, 63)
    struct.pack_into("<H", bs, 26, 255)
    struct.pack_into("<I", bs, 28, hidden)
    struct.pack_into("<I", bs, 32, vol.total_sectors)
    struct.pack_into("<I", bs, 36, vol.fat_secs)
    struct.pack_into("<H", bs, 40, 0)          # ExtFlags: mirrored FATs
    struct.pack_into("<H", bs, 42, 0)          # FSVer
    struct.pack_into("<I", bs, 44, 2)          # RootClus
    struct.pack_into("<H", bs, 48, 1)          # FSInfo sector
    struct.pack_into("<H", bs, 50, 6)          # backup boot sector
    bs[64] = 0x80
    bs[66] = 0x29
    struct.pack_into("<I", bs, 67, 0x52AC0503) # volume id, stable on purpose
    bs[71:82] = b"RACOS-ESP  "
    bs[82:90] = b"FAT32   "
    bs[510:512] = b"\x55\xaa"
    return bytes(bs)


def fsinfo_sector(vol: Fat32Volume) -> bytes:
    fi = bytearray(SECTOR)
    struct.pack_into("<I", fi, 0, 0x41615252)
    struct.pack_into("<I", fi, 484, 0x61417272)
    free = sum(1 for c in range(2, vol.clusters + 2) if vol.fat[c] == 0)
    struct.pack_into("<I", fi, 488, free)
    struct.pack_into("<I", fi, 492, vol.next_free)
    fi[508:512] = b"\x00\x00\x55\xaa"
    return bytes(fi)


def mbr(part_start: int, part_sectors: int) -> bytes:
    m = bytearray(SECTOR)
    # One partition: bootable, type 0x0C (FAT32 LBA). CHS fields are the
    # 0xFE 0xFF 0xFF "use LBA" sentinel; nothing modern reads them.
    entry = struct.pack(
        "<B3sB3sII",
        0x80, b"\xfe\xff\xff", 0x0C, b"\xfe\xff\xff", part_start, part_sectors,
    )
    m[446 : 446 + 16] = entry
    m[510:512] = b"\x55\xaa"
    return bytes(m)


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    esp_dir = args[0] if len(args) > 0 else "esp"
    out = args[1] if len(args) > 1 else "racos-esp.img"
    size_mib = 300
    for a in sys.argv[1:]:
        if a.startswith("--size-mib"):
            size_mib = int(a.split("=", 1)[1])

    part_sectors = size_mib * 1024 * 1024 // SECTOR
    print(f"[esp-image] {esp_dir} -> {out} ({size_mib} MiB partition)")
    vol = build_volume(esp_dir, part_sectors)

    with open(out, "wb") as f:
        f.write(mbr(PART_START_LBA, part_sectors))
        f.write(b"\x00" * ((PART_START_LBA - 1) * SECTOR))

        base = f.tell()
        bs = boot_sector(vol, PART_START_LBA)
        fi = fsinfo_sector(vol)
        f.write(bs)
        f.write(fi)
        f.write(b"\x00" * (4 * SECTOR))
        f.write(bs)          # backup boot sector at +6
        f.write(fi)          # and its FSInfo at +7
        f.seek(base + RSVD_SECS * SECTOR)

        fat_blob = b"".join(struct.pack("<I", e) for e in vol.fat)
        fat_blob += b"\x00" * (vol.fat_secs * SECTOR - len(fat_blob))
        f.write(fat_blob)
        f.write(fat_blob)    # FAT copy 2

        cs = vol.cluster_size()
        data_start = f.tell()
        end_cluster = vol.next_free
        for c in range(2, end_cluster):
            blob = vol.cluster_data.get(c, b"")
            f.seek(data_start + (c - 2) * cs)
            f.write(blob + b"\x00" * (cs - len(blob)))
        # pad the image to its full advertised size
        full = (PART_START_LBA + part_sectors) * SECTOR
        f.seek(full - 1)
        f.write(b"\x00")

    print(f"[esp-image] done: {os.path.getsize(out)} bytes, "
          f"{vol.clusters} clusters, FAT {vol.fat_secs} sectors")


if __name__ == "__main__":
    main()
