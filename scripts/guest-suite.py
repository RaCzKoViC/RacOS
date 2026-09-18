#!/usr/bin/env python3
"""RacOS - drive the in-guest `racos-test` suite on the canonical QEMU machine.

One driver for the Windows dev box and the Linux CI runner. Before this
script existed the two had drifted apart: the local gate booted with an AHCI
disk and a VirtIO-net NIC, the CI job booted with neither, slept fixed
seconds, and graded nine grepped markers - so it went red on a tree that
passes 176/0 locally, and it would have stayed green on a log that says
"1 failed" as long as those nine markers were present.

Contract:

  * The machine definition lives here and only here (see machine_args()).
    Local runs and CI boot the same devices, or the harness refuses to grade.
  * Readiness, not sleeps: we wait for the racsh prompt, for each command's
    prompt to come back, and for the suite's own final line.
  * The verdict is the suite's tally (`=== Results: N passed, 0 failed ===`)
    plus its exit status as seen by the shell (`RACOS-TEST-EXIT=0`), plus the
    absence of a kernel panic. Markers are reported, never trusted alone.
  * `--self-test` runs the grader against fixture logs, including the exact
    log shape that the old marker-only CI check accepted.

Exit status: 0 = PASS, 1 = FAIL (verdict or missing device), 2 = the harness
could not run (no QEMU, no OVMF, ESP not staged, disk image locked).

Serial input goes to the guest one character at a time with a short pause:
a whole line at once is silently dropped on the stdio path, and a suite that
never started would look like an idle, "clean" guest.

Usage:
  python3 scripts/guest-suite.py [--smp 2] [--suite-timeout 600] ...
  python3 scripts/guest-suite.py --self-test
  python3 scripts/guest-suite.py --no-disk --no-net   # must FAIL: devices missing

ASCII only, stdlib only, Python 3.8+.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import threading
import time

# --------------------------------------------------------------------------
# Machine definition
# --------------------------------------------------------------------------

DISK_SIZE_BYTES = 16 * 1024 * 1024

# What the suite needs from the machine, and the boot-log line that proves the
# kernel actually saw it. Missing any of these is a harness failure reported
# by name, not twenty scattered [FAIL] lines inside the suite.
REQUIRED_DEVICES = [
    ("sda (AHCI disk)", "DRIVERS: AHCI initialized, sda registered"),
    ("/mnt (persistent racfs on sda)", "RACORE: racfs mounted on /mnt (persistent, on sda)"),
    ("net (VirtIO-net + slirp)", "[ NETSTACK ] up: ip="),
]


def machine_args(ovmf, esp_dir, disk_path, smp, with_disk=True, with_net=True):
    """The canonical RacOS test machine. Keep in sync with nothing: this is it."""
    args = [
        "-machine", "q35",
        "-accel", "tcg",
        "-cpu", "qemu64,+smep,+smap",
        "-smp", str(smp),
        "-m", "512M",
        "-drive", "if=pflash,format=raw,readonly=on,file=" + ovmf,
        "-drive", "if=ide,format=raw,file=fat:rw:" + esp_dir,
    ]
    if with_disk:
        # Explicit ich9-ahci + ide-hd: plain `-drive if=ide` did not show up
        # in the PCI scan on q35 (kernel logged "AHCI init skipped").
        args += [
            "-drive", "id=disk0,file=" + disk_path + ",if=none,format=raw,cache=writethrough",
            "-device", "ich9-ahci,id=ahci",
            "-device", "ide-hd,drive=disk0,bus=ahci.0",
        ]
    if with_net:
        # The kernel driver speaks legacy VirtIO I/O; a modern-only
        # virtio-net-pci (QEMU's q35 default) accepts frames and drops them.
        args += [
            "-netdev", "user,id=net0",
            "-device", "virtio-net-pci,netdev=net0,romfile=,disable-modern=on,disable-legacy=off",
        ]
    args += [
        "-serial", "stdio",
        "-display", "none",
        "-no-reboot",
    ]
    return args


# --------------------------------------------------------------------------
# Grading (pure functions - covered by --self-test)
# --------------------------------------------------------------------------

BANNER_RE = re.compile(r"racsh 0\.1\.0")
PROMPT_RE = re.compile(r"racsh\$ ")
# Banner and first prompt in one pattern: they may land in the same read
# chunk, so "wait for the banner, then look for a prompt after it" races.
BANNER_THEN_PROMPT_RE = re.compile(r"racsh 0\.1\.0[\s\S]*?racsh\$ ")
RESULTS_RE = re.compile(r"=== Results: (\d+) passed, (\d+) failed ===")
FSCK_CLEAN_RE = re.compile(r"RACFS sda: fsck clean")
FSCK_DIRTY_RE = re.compile(r"RACFS sda: fsck (found|could not run).*")
EXIT_RE = re.compile(r"(?m)^RACOS-TEST-EXIT=(\d+)\s*$")
EXIT_THEN_PROMPT_RE = re.compile(r"(?m)^RACOS-TEST-EXIT=\d+\s*$[\s\S]*racsh\$ ")
MARKER_RE = re.compile(r"(?m)^([A-Z0-9][A-Z0-9-]*-(?:OK|FAIL))\b")
# Fatal kernel lines, exactly as kernel/src/panic.rs and arch/idt.rs print
# them. A user-space page fault is deliberately not here: it kills one
# process and the tally reflects it; these park the CPU.
PANIC_PATTERNS = [
    ("KERNEL PANIC", "!!! KERNEL PANIC"),
    ("KERNEL PAGE FAULT", "!!! KERNEL PAGE FAULT"),
    ("CPU EXCEPTION", "!!! EXCEPTION #"),
    ("HALTING", "!!! HALTING"),
]

# The nine checks the old CI job grepped for. Kept as data so the self-test
# can show what they accept.
LEGACY_CHECKS = [
    ("racsh prompt present", re.compile(r"racsh\$")),
    ("pwd returned /", re.compile(r"(?m)^/$")),
    ("echo roundtrip", re.compile(r"ci-smoke-ok")),
    ("/proc/version readable", re.compile(r"RacOS version")),
    ("signal default action", re.compile(r"PHASE21-SIGNAL-TERM-OK")),
    ("SIGCHLD wait", re.compile(r"PHASE21-SIGCHLD-WAIT-OK")),
    ("exec-loop cleanup", re.compile(r"PHASE21-EXEC-LOOP-OK")),
    ("poll timeout", re.compile(r"POLL-TIMEOUT-OK")),
    ("TTY ioctl state", re.compile(r"TTY-IOCTL-OK")),
]


class Verdict:
    def __init__(self):
        self.ok = False
        self.reasons = []
        self.passed = None
        self.failed = None
        self.exit_code = None
        self.markers_ok = []
        self.markers_fail = []
        self.legacy = []  # (name, ok)
        self.panic = None


def legacy_ci_greps(text):
    """True iff every check of the pre-2026-09 CI job matches. This is the
    grading the old job did, reproduced so the self-test can demonstrate the
    hole: it says nothing about the suite's tally or completion."""
    return all(rx.search(text) for _, rx in LEGACY_CHECKS)


def missing_devices(boot_text):
    return [name for name, needle in REQUIRED_DEVICES if needle not in boot_text]


def evaluate(text):
    """Grade a complete serial log. PASS requires the whole contract."""
    v = Verdict()
    m = RESULTS_RE.search(text)
    if m:
        v.passed, v.failed = int(m.group(1)), int(m.group(2))
    else:
        v.reasons.append("suite did not report a tally (=== Results: ... ===) - incomplete run")

    ex = EXIT_RE.search(text)
    if ex:
        v.exit_code = int(ex.group(1))
    else:
        v.reasons.append("shell never echoed RACOS-TEST-EXIT=<n> - the suite did not return")

    if v.failed is not None and v.failed > 0:
        v.reasons.append("suite reported %d failed assertion(s)" % v.failed)
    if v.passed is not None and v.passed == 0:
        v.reasons.append("suite reported 0 passed assertions")
    if v.exit_code is not None and v.exit_code != 0:
        v.reasons.append("racos-test exited with status %d" % v.exit_code)
    if v.failed == 0 and v.exit_code not in (None, 0):
        v.reasons.append("tally says 0 failed but the exit status disagrees")

    for name, needle in PANIC_PATTERNS:
        if needle in text:
            v.panic = name
            v.reasons.append("%s seen in the serial log ('%s')" % (name, needle))
            break

    for name, rx in LEGACY_CHECKS:
        ok = bool(rx.search(text))
        v.legacy.append((name, ok))
        if not ok:
            v.reasons.append("legacy check failed: " + name)

    markers = sorted(set(MARKER_RE.findall(text)))
    v.markers_ok = [x for x in markers if x.endswith("-OK")]
    v.markers_fail = [x for x in markers if x.endswith("-FAIL")]
    if v.markers_fail:
        v.reasons.append("FAIL markers: " + ", ".join(v.markers_fail))

    v.ok = not v.reasons
    return v


def print_summary(v, log_path=None, log_size=None):
    print("")
    if v.passed is not None:
        print("racos-test tally: %d passed, %d failed" % (v.passed, v.failed))
    else:
        print("racos-test tally: NOT REACHED (run did not complete)")
    if v.exit_code is not None:
        print("racos-test exit status (via $?): %d" % v.exit_code)
    else:
        print("racos-test exit status (via $?): NOT SEEN")
    if v.panic:
        print("!!! %s in this run !!!" % v.panic)

    print("")
    print("=== legacy CI assertions (reported, not sufficient) ===")
    for name, ok in v.legacy:
        print("  %s  %s" % ("PASS" if ok else "FAIL", name))

    print("")
    print("=== racos-test markers observed ===")
    for mk in v.markers_ok + v.markers_fail:
        print("  " + mk)
    print("")
    print("markers OK=%d  FAIL=%d" % (len(v.markers_ok), len(v.markers_fail)))
    if log_path:
        print("Full log: %s [%s bytes]" % (log_path, log_size if log_size is not None else "?"))

    print("")
    if v.ok:
        print("RACOS-TEST PASS")
    else:
        for r in v.reasons:
            print("  reason: " + r)
        print("RACOS-TEST FAIL")


# --------------------------------------------------------------------------
# Self-test fixtures
# --------------------------------------------------------------------------

BOOT_OK = """RACORE: RacOS kernel starting
[  0.001200] DRIVERS: VirtIO-Net @ PCI 00:03.0, MAC 52:54:00:12:34:56
[  0.001300] DRIVERS: AHCI initialized, sda registered
[ NETSTACK ] up: ip=10.0.2.15, gw=10.0.2.2, mac=52:54:00:12:34:56
[  0.000370] RACORE: racfs mounted on /mnt (persistent, on sda)
[init] RacInit starting (PID 1)
[init] spawned /bin/sh
racsh 0.1.0
racsh$ pwd
/
racsh$ echo ci-smoke-ok
ci-smoke-ok
racsh$ cat /proc/version
RacOS version 0.1.0
racsh$ racos-test; echo RACOS-TEST-EXIT=$?
=== RacOS System Test Suite ===
"""

MARKERS_ALL = """POLL-TIMEOUT-OK
PHASE21-SIGNAL-TERM-OK
PHASE21-SIGCHLD-WAIT-OK
PHASE21-EXEC-LOOP-OK
TTY-IOCTL-OK
"""


def fixture(tally, exit_line, extra_after=""):
    return BOOT_OK + MARKERS_ALL + tally + extra_after + exit_line + "racsh$ "


def self_test():
    failures = []

    def expect(cond, what):
        print("  %s  %s" % ("ok  " if cond else "FAIL", what))
        if not cond:
            failures.append(what)

    print("guest-suite self-test")

    # A: the complete, healthy run.
    a = fixture("\n=== Results: 176 passed, 0 failed ===\n", "RACOS-TEST-EXIT=0\n")
    va = evaluate(a)
    expect(va.ok, "A: complete 176/0 run with exit 0 -> PASS (reasons: %s)" % va.reasons)
    expect((va.passed, va.failed, va.exit_code) == (176, 0, 0), "A: tally and exit parsed")
    expect(len(va.markers_ok) == 5 and not va.markers_fail, "A: five OK markers, no FAIL markers")

    # B: the hole in the old CI job. Every grepped marker is present, the
    # tally says one assertion failed and the suite exited 1. The old greps
    # accept it; the verdict must not.
    b = fixture("  [FAIL] a 16402-byte file writes (single indirect)\n"
                "=== Results: 175 passed, 1 failed ===\n", "RACOS-TEST-EXIT=1\n")
    expect(legacy_ci_greps(b), "B: the old marker-only CI check ACCEPTS a log with 1 failed (the bug)")
    vb = evaluate(b)
    expect(not vb.ok and vb.failed == 1, "B: verdict rejects it (reasons: %s)" % vb.reasons)

    # C: the suite was cut off (what a fixed sleep budget produces on a slow
    # runner): markers so far look fine, no tally, no exit line.
    c = BOOT_OK + MARKERS_ALL + "[test] racfs past the direct blocks\n"
    vc = evaluate(c)
    expect(not vc.ok and vc.passed is None and vc.exit_code is None,
           "C: truncated run -> FAIL as incomplete (reasons: %s)" % vc.reasons)

    # D: clean tally, then the kernel panics before the prompt returns.
    d = fixture("\n=== Results: 176 passed, 0 failed ===\n", "",
                extra_after="!!! KERNEL PANIC !!!\npanicked at kernel/src/mm/virt.rs:188\n!!! HALTING !!!\n")
    vd = evaluate(d)
    expect(not vd.ok and vd.panic == "KERNEL PANIC", "D: 0 failed + KERNEL PANIC -> FAIL")

    # E: tally says clean but the exit status the shell saw is 1 - the exit
    # code is part of the contract, an inconsistent pair is a failure.
    e = fixture("\n=== Results: 176 passed, 0 failed ===\n", "RACOS-TEST-EXIT=1\n")
    ve = evaluate(e)
    expect(not ve.ok and ve.exit_code == 1, "E: 0 failed but exit status 1 -> FAIL")

    # F: device declaration. The old CI machine had no disk and no NIC.
    old_ci_boot = "RACORE: RacOS kernel starting\n[init] spawned /bin/sh\nracsh 0.1.0\nracsh$ "
    miss = missing_devices(old_ci_boot)
    expect(len(miss) == 3, "F: old CI machine (no disk, no net) is missing 3 devices: %s" % miss)
    expect(missing_devices(BOOT_OK) == [], "F: canonical boot log declares every required device")
    only_net = BOOT_OK.replace("DRIVERS: AHCI initialized, sda registered", "AHCI init skipped: NoController")
    expect(missing_devices(only_net) == ["sda (AHCI disk)"], "F: a missing disk is reported by name")

    # G: the echo of the typed command must not satisfy the exit regex.
    g = BOOT_OK  # ends with the echoed 'racos-test; echo RACOS-TEST-EXIT=$?' line
    expect(EXIT_RE.search(g) is None, "G: the echoed 'RACOS-TEST-EXIT=$?' is not mistaken for the result")

    # H: boot readiness is one pattern over the whole log, so it holds no
    # matter how the serial bytes were chunked (banner and prompt arriving
    # together once made the two-step wait miss the prompt).
    expect(BANNER_THEN_PROMPT_RE.search("boot\nracsh 0.1.0\nracsh$ [ NETSTACK ] DNS: query\n") is not None,
           "H: banner + prompt in one chunk -> ready")
    expect(BANNER_THEN_PROMPT_RE.search("boot\nracsh 0.1.0\n[ NETSTACK ] up\n") is None,
           "H: banner without a prompt -> not ready")

    # I: the mount-time fsck verdict, as kernel/src/main.rs prints it.
    expect(FSCK_CLEAN_RE.search("[  0.000368] RACFS sda: fsck clean\n") is not None,
           "I: 'fsck clean' is recognised")
    dirty = "[  0.000368] RACFS sda: fsck found leaked=3 unallocated_in_use=0 doubly_claimed=0 dangling=0 out_of_range=0 sb_drift=1\n"
    expect(FSCK_CLEAN_RE.search(dirty) is None and FSCK_DIRTY_RE.search(dirty) is not None,
           "I: a dirty fsck line is not clean and is quoted back")

    print("")
    if failures:
        print("SELF-TEST FAIL (%d)" % len(failures))
        return 1
    print("SELF-TEST PASS")
    return 0


# --------------------------------------------------------------------------
# QEMU discovery
# --------------------------------------------------------------------------

def find_qemu(explicit):
    if explicit:
        return explicit
    found = shutil.which("qemu-system-x86_64")
    if found:
        return found
    for cand in [r"D:\qemu\qemu-system-x86_64.exe",
                 r"C:\Program Files\qemu\qemu-system-x86_64.exe"]:
        if os.path.isfile(cand):
            return cand
    return None


def find_ovmf(explicit, qemu_path, root):
    if explicit:
        return explicit
    cands = [
        os.path.join(root, "tools", "OVMF_CODE.fd"),
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/OVMF/OVMF_CODE_4M.fd",
        "/usr/share/qemu/OVMF.fd",
        r"D:\qemu\share\edk2-x86_64-code.fd",
    ]
    if qemu_path:
        qdir = os.path.dirname(os.path.abspath(qemu_path))
        cands.append(os.path.join(qdir, "share", "edk2-x86_64-code.fd"))
        cands.append(os.path.join(qdir, "..", "share", "qemu", "edk2-x86_64-code.fd"))
    for c in cands:
        if os.path.isfile(c):
            return os.path.normpath(c)
    return None


# --------------------------------------------------------------------------
# Guest driver
# --------------------------------------------------------------------------

class Guest:
    def __init__(self, cmd, log_path):
        self.cmd = cmd
        self.log_path = log_path
        self.buf = bytearray()
        self.lock = threading.Lock()
        self.last_data_at = time.monotonic()
        self.max_gap = 0.0       # longest pause between two chunks, seconds
        self.max_gap_after = ""  # what the guest had just printed before it
        self.proc = None
        self.reader = None
        self.log = None

    def start(self):
        kwargs = {}
        if sys.platform == "win32":
            kwargs["creationflags"] = 0x08000000  # CREATE_NO_WINDOW
        self.log = open(self.log_path, "wb")
        self.proc = subprocess.Popen(self.cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, bufsize=0, **kwargs)
        self.reader = threading.Thread(target=self._pump, daemon=True)
        self.reader.start()

    def _pump(self):
        # stdout is an unbuffered pipe (bufsize=0), so read() returns as soon
        # as any bytes are available; a buffered read of a full block would
        # sit forever on a guest that has gone quiet.
        out = self.proc.stdout
        while True:
            try:
                chunk = out.read(8192)
            except (OSError, ValueError):
                break
            if not chunk:
                break
            self.log.write(chunk)
            self.log.flush()
            now = time.monotonic()
            with self.lock:
                gap = now - self.last_data_at
                if gap > self.max_gap:
                    self.max_gap = gap
                    self.max_gap_after = bytes(self.buf[-80:]).decode("utf-8", errors="replace")
                self.buf += chunk
                self.last_data_at = now

    def reset_gap(self):
        with self.lock:
            self.max_gap = 0.0
            self.max_gap_after = ""
            self.last_data_at = time.monotonic()

    def longest_gap(self):
        with self.lock:
            return self.max_gap, self.max_gap_after

    def text(self):
        with self.lock:
            return bytes(self.buf).decode("utf-8", errors="replace").replace("\r", "")

    def raw_len(self):
        with self.lock:
            return len(self.buf)

    def silent_for(self):
        with self.lock:
            return time.monotonic() - self.last_data_at

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def wait_for(self, regex, timeout, since=0, silence=None):
        """Poll until `regex` matches the log text produced after byte offset
        `since`. Returns the match or None on timeout / hang / QEMU exit."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.lock:
                tail = bytes(self.buf[since:])
            m = regex.search(tail.decode("utf-8", errors="replace").replace("\r", ""))
            if m:
                return m
            if not self.alive():
                # Drain whatever the reader still has, then give up.
                time.sleep(0.2)
                with self.lock:
                    tail = bytes(self.buf[since:])
                return regex.search(tail.decode("utf-8", errors="replace").replace("\r", ""))
            if silence is not None and self.silent_for() >= silence:
                return None
            time.sleep(0.1)
        return None

    def send_line(self, line):
        """One character at a time, LF-terminated. Bulk writes are dropped
        silently on this path and CR is unreliable."""
        try:
            for ch in (line + "\n").encode("ascii"):
                self.proc.stdin.write(bytes([ch]))
                self.proc.stdin.flush()
                time.sleep(0.03)
            return True
        except (OSError, ValueError):
            return False

    def stop(self):
        try:
            if self.proc and self.proc.stdin:
                self.proc.stdin.close()
        except OSError:
            pass
        if self.alive():
            try:
                self.proc.kill()
            except OSError:
                pass
        if self.proc:
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
        if self.reader:
            self.reader.join(timeout=5)
        if self.log:
            try:
                self.log.close()
            except OSError:
                pass


def tail_lines(text, n):
    lines = text.split("\n")
    return "\n".join(lines[-n:])


def run(args):
    root = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
    os.chdir(root)

    esp_dir = os.path.abspath(args.esp)
    disk_path = os.path.abspath(args.disk)
    log_path = os.path.abspath(args.log)

    qemu = find_qemu(args.qemu)
    if not qemu or not os.path.isfile(qemu):
        print("ERROR: qemu-system-x86_64 not found (pass --qemu PATH)")
        return 2
    ovmf = find_ovmf(args.ovmf, qemu, root)
    if not ovmf:
        print("ERROR: OVMF firmware not found (pass --ovmf PATH; on Debian/Ubuntu: apt-get install ovmf)")
        return 2

    missing = [p for p in ("racore.elf", "initramfs.img", os.path.join("EFI", "BOOT", "BOOTX64.EFI"))
               if not os.path.isfile(os.path.join(esp_dir, p))]
    if missing:
        print("ERROR: ESP is not staged (missing under %s: %s)" % (esp_dir, ", ".join(missing)))
        print("  Linux:   bash scripts/build-image.sh")
        print("           && cp target/x86_64-unknown-uefi/debug/bootx64.efi esp/EFI/BOOT/BOOTX64.EFI")
        print("  Windows: powershell -File scripts\\build-image.ps1 (with RUSTFLAGS empty), then copy bootx64.efi")
        return 2

    # A fresh zero-filled disk every run: the suite's racfs assertions expect
    # a first-boot format, and a locked file means another QEMU still owns it.
    # --keep-disk boots the previous run's disk instead, which is how the
    # mount-time fsck gets to judge what the suite left behind.
    if args.keep_disk:
        if not os.path.isfile(disk_path):
            print("ERROR: --keep-disk but %s does not exist" % disk_path)
            return 2
    else:
        try:
            if os.path.exists(disk_path):
                os.remove(disk_path)
            with open(disk_path, "wb") as f:
                f.truncate(DISK_SIZE_BYTES)
        except OSError as e:
            print("ERROR: cannot recreate %s (%s) - is a previous QEMU still running?" % (disk_path, e))
            return 2

    cmd = [qemu] + machine_args(ovmf, esp_dir, disk_path, args.smp,
                                with_disk=not args.no_disk, with_net=not args.no_net)
    print("QEMU: " + " ".join(cmd))
    print("serial log: " + log_path)

    guest = Guest(cmd, log_path)
    t0 = time.monotonic()
    try:
        guest.start()
    except OSError as e:
        print("ERROR: could not start QEMU: %s" % e)
        guest.stop()
        return 2

    verdict = None
    try:
        # --- boot readiness: banner, then the first prompt -----------------
        print("waiting for racsh (max %ds)..." % args.boot_timeout)
        if guest.wait_for(BANNER_THEN_PROMPT_RE, args.boot_timeout) is None:
            if BANNER_RE.search(guest.text()):
                print("FAIL: racsh banner seen but no prompt followed within %ds" % args.boot_timeout)
            else:
                print("FAIL: guest never printed the racsh banner within %ds" % args.boot_timeout)
            print(tail_lines(guest.text(), 40))
            return 1
        boot_s = time.monotonic() - t0
        print("racsh up after %.1fs" % boot_s)

        # --- device declaration --------------------------------------------
        miss = missing_devices(guest.text())
        if miss:
            for name in miss:
                print("FAIL: required device missing: " + name)
            print("the suite needs /mnt on an AHCI disk and a VirtIO-net NIC; this machine does not have them")
            return 1
        print("devices present: " + ", ".join(n for n, _ in REQUIRED_DEVICES))

        # --- fsck verdict on the disk as mounted -----------------------------
        # The kernel checks the racfs on sda at mount time and says so in one
        # of these lines. On a fresh disk "clean" is trivial; with --keep-disk
        # it is the judgement on everything the previous run did.
        boot_text = guest.text()
        if FSCK_CLEAN_RE.search(boot_text):
            print("fsck on sda: clean")
        else:
            m = FSCK_DIRTY_RE.search(boot_text)
            print("FAIL: fsck on sda did not report clean: %s" % (m.group(0) if m else "no fsck line"))
            return 1
        if args.boot_only:
            print("--boot-only: stopping after boot readiness, devices and fsck")
            print("BOOT-ONLY PASS")
            return 0

        # --- a few plain commands, each waiting for its prompt --------------
        for c in ("pwd", "echo ci-smoke-ok", "cat /proc/version"):
            mark = guest.raw_len()
            if not guest.send_line(c):
                print("FAIL: could not write '%s' to the guest" % c)
                return 1
            if guest.wait_for(PROMPT_RE, 30, since=mark) is None:
                print("FAIL: prompt did not return after '%s'" % c)
                print(tail_lines(guest.text(), 40))
                return 1

        # --- the suite; the shell reports its exit status -------------------
        print("running racos-test (budget %ds, silence limit %ds)..." % (args.suite_timeout, args.silence))
        t1 = time.monotonic()
        mark = guest.raw_len()
        guest.reset_gap()
        if not guest.send_line("racos-test; echo RACOS-TEST-EXIT=$?"):
            print("FAIL: could not start racos-test")
            return 1
        m = guest.wait_for(EXIT_RE, args.suite_timeout, since=mark, silence=args.silence)
        suite_s = time.monotonic() - t1
        gap, gap_after = guest.longest_gap()
        if m is None:
            if guest.silent_for() >= args.silence:
                print("guest silent for %ds after %.0fs - treating as hang/panic" % (args.silence, suite_s))
            elif not guest.alive():
                print("QEMU exited before the suite returned")
            else:
                print("suite did not return within %ds" % args.suite_timeout)
        else:
            print("racos-test returned after %.1fs" % suite_s)
            # Let the prompt and any trailing kernel lines land in the log.
            guest.wait_for(EXIT_THEN_PROMPT_RE, 5, since=mark)
        # Evidence for tuning --silence: the longest pause the suite made.
        print("longest silent stretch during the suite: %.1fs, after: %r"
              % (gap, " ".join(gap_after.split())[-60:]))
    finally:
        guest.stop()

    text = guest.text()
    verdict = evaluate(text)
    print_summary(verdict, log_path, os.path.getsize(log_path) if os.path.exists(log_path) else None)
    if not verdict.ok:
        print("")
        print("--- serial log tail ---")
        print(tail_lines(text, 60))
    return 0 if verdict.ok else 1


def main(argv=None):
    p = argparse.ArgumentParser(description="Drive racos-test on the canonical RacOS QEMU machine.")
    p.add_argument("--qemu", help="qemu-system-x86_64 binary (default: PATH, then D:\\qemu)")
    p.add_argument("--ovmf", help="OVMF code image (default: tools/, /usr/share/OVMF, D:\\qemu\\share)")
    p.add_argument("--esp", default="esp", help="staged ESP directory (default: esp)")
    p.add_argument("--disk", default="racos-racostest-disk.img", help="AHCI disk image, recreated each run")
    p.add_argument("--log", default="racos-racostest.log", help="serial log")
    p.add_argument("--smp", type=int, default=2)
    p.add_argument("--boot-timeout", type=int, default=180, help="seconds to reach the racsh prompt")
    p.add_argument("--suite-timeout", type=int, default=600, help="seconds for racos-test to return")
    p.add_argument("--silence", type=int, default=60, help="seconds of no serial output = hang")
    p.add_argument("--no-disk", action="store_true", help="omit the AHCI disk (negative test)")
    p.add_argument("--no-net", action="store_true", help="omit the VirtIO-net NIC (negative test)")
    p.add_argument("--keep-disk", action="store_true",
                   help="boot the existing disk image instead of a fresh one (fsck judges the last run)")
    p.add_argument("--boot-only", action="store_true",
                   help="stop after boot readiness, device check and the fsck verdict; do not run the suite")
    p.add_argument("--self-test", action="store_true", help="grade fixture logs and exit")
    args = p.parse_args(argv)

    if hasattr(sys.stdout, "reconfigure"):
        # Guest bytes are not guaranteed UTF-8 and the Windows console is not
        # guaranteed to accept them; never die while printing evidence. Line
        # buffering so progress shows up live in a CI log or a redirect, and
        # survives if the runner kills the job mid-run.
        sys.stdout.reconfigure(errors="replace", line_buffering=True)

    if args.self_test:
        return self_test()
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
