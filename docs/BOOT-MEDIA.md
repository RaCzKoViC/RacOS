# Booting RacOS from real media (ROADMAP §3.4)

RacOS's development workflow boots QEMU from `fat:rw:esp` — a synthetic FAT
view of a host directory. Real media is different in every way that matters:
a USB stick is a block device with a partition table and an actual FAT32
filesystem, enumerated by the firmware over an actual bus. This document
covers building that image, proving it boots, and what to expect when you
write it to a physical stick.

## Building the image

```
powershell -File scripts\build-image.ps1     # stage esp/ first
python scripts\make-esp-image.py esp racos-esp.img
```

The output is an MBR-partitioned disk image: one bootable FAT32 partition
(type 0x0C) starting at 1 MiB, holding `EFI/BOOT/BOOTX64.EFI` (the UEFI
fallback path — no NVRAM boot entry needed), `racore.elf` and
`initramfs.img`. The FAT32 formatter is `make-esp-image.py` itself, written
from scratch because mtools does not exist on a stock Windows box and
`fat:rw:esp` is not a real filesystem. Long-file-name entries are written
for every file, since `initramfs.img` does not fit 8.3.

`NvVars` is deliberately excluded: it is OVMF's variable store, host-side
state that the firmware writes back into `esp/` — shipping a snapshot of it
would hand every machine this machine's boot variables.

## Proving it boots (QEMU, automated)

```
powershell -File scripts\test-usb-boot.ps1
```

Attaches the image as USB mass storage on an XHCI controller — with **no**
`fat:rw` fallback — and requires the whole chain to produce a racsh prompt:
OVMF USB enumeration → partition parse → FAT32 driver → fallback loader →
RacOS bootloader reading the kernel and initramfs over USB. Five assertions,
marker `USB-BOOT PASS`.

## Writing it to a physical USB stick

**Everything on the stick is destroyed.** Double-check the target device;
there is no undo.

Windows — use [Rufus](https://rufus.ie) in *DD image* mode with
`racos-esp.img`, or from an **administrator** Git Bash (replace `N` after
triple-checking in `Get-Disk` / Disk Management):

```
dd if=racos-esp.img of=\\\\.\\PhysicalDriveN bs=4M conv=fsync
```

Linux (replace `sdX`, verify with `lsblk` first):

```
sudo dd if=racos-esp.img of=/dev/sdX bs=4M conv=fsync status=progress
```

Then boot the target machine from USB (usually a firmware boot menu on
F8/F10/F11/F12). The machine must be x86_64 UEFI; **Secure Boot must be
disabled** — RacOS binaries are unsigned (signing is the post-v0.4 T4.1
crypto track).

## What to expect on real hardware — honest edition

This is the first-ever real-media path for a hobby OS whose only tested
target is QEMU. The kernel's driver surface is exactly: AHCI (one port),
VirtIO-net, PS/2 keyboard, GOP framebuffer, serial. That implies:

- **Keyboard**: the kernel speaks PS/2 only. On desktops whose firmware
  provides legacy PS/2 emulation for USB keyboards, typing works; on
  machines without that (most modern laptops), the console will display
  but not accept input. A USB HID driver is future work.
- **Display**: the GOP framebuffer is claimed from the bootloader, so
  video output should work on any UEFI machine.
- **The USB stick itself disappears after boot.** The kernel has no USB
  stack; the bootloader reads everything into RAM first, so RacOS runs
  entirely from the initramfs afterwards. Nothing can be written back to
  the stick.
- **Persistent storage**: if the machine has a SATA disk on AHCI, the
  kernel binds the first port as `sda`. **RacOS will not format a disk it
  does not recognise**: a valid racfs mounts, a *blank* disk (first sector
  all zeroes) is claimed and formatted, and anything else — a Windows or
  Linux disk, say — is refused with a message, leaving `/home`, `/etc`,
  `/var/log` and `/var/lib/rpkg` volatile for that session. Claiming a
  disk for RacOS is an explicit `mkfs.racfs sda` in the shell, and it
  destroys that disk's contents. This guard exists precisely because the
  auto-format policy that was safe under QEMU (every disk is a fresh
  zeroed image) would have been a data shredder on real hardware.
- **NVMe disks are invisible** — there is no NVMe driver. AHCI/SATA only.
- **Networking**: VirtIO-net is a paravirtual device; no real NIC will
  match. Expect no network on physical hardware for now.

Machines are weirder than emulators. A boot that hangs before the first
serial line is most usefully debugged with the framebuffer messages; a
photo of the screen plus the exact hardware model is the right bug report.
