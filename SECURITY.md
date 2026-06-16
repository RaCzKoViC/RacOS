# Security Policy

This document covers how to report vulnerabilities in RacOS, which
versions receive security fixes, and what the project's security
posture currently is.

For the design baseline (which mechanisms are on by default, which are
still being built), see [`docs/adr/ADR-019-security-baseline.md`](docs/adr/ADR-019-security-baseline.md).

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.**

Use either of:

- GitHub's [private vulnerability reporting](https://github.com/RaCzKoViC/RacOS/security/advisories/new)
  (preferred — keeps the discussion in-repo and creates an audit trail).
- Email the maintainer directly via the address on the [@RaCzKoViC](https://github.com/RaCzKoViC)
  GitHub profile.

Include in the report:

1. A description of the issue and the affected component(s).
2. Steps to reproduce, ideally with a minimal test case (a userland
   program, a script, a QEMU command line, or a kernel diff that
   reliably triggers the bug).
3. Your assessment of impact (information leak / privilege escalation /
   denial of service / arbitrary code execution / etc.) and which
   process boundary it crosses (user → kernel? user → user? bootloader
   → kernel?).
4. Any suggested fix or mitigation, if you have one.

You should expect:

- An acknowledgement within **72 hours** that your report was received
  and is being looked at.
- A status update within **14 days** with either a fix, a mitigation,
  or a concrete plan + ETA.
- A coordinated disclosure date once a fix is ready, normally **30
  days** after the initial report (sooner if the fix is trivial and
  already merged, longer for changes that need cross-subsystem work).
- Credit in the release notes and `CHANGELOG.md` under the `Security`
  section unless you ask to be omitted.

## Supported versions

RacOS is pre-1.0. Only `main` is supported for security fixes; there
are no tagged release branches yet.

Once `v1.0.0` ships, this section will list the supported semantic
version ranges and the policy for backporting fixes.

| Version | Supported |
|---------|-----------|
| `main`  | ✅ |
| `< 1.0` snapshots | ❌ (use `main`) |

## Scope

The following are in-scope for the security policy:

- Any path that lets a user-mode process gain kernel-mode execution.
- Any path that lets a user-mode process bypass DAC, capability checks,
  or the validate_user_ptr/`validate_user_string` boundary.
- Any path that lets the bootloader hand off an unverified or malformed
  kernel image without surfacing the failure on serial.
- Information leaks across process boundaries (FDs, environment
  variables, signal state, syscall return values).
- Denial-of-service vulnerabilities that crash the kernel from a
  user-mode caller.
- Logic bugs in the package format (`rpkg`) that allow a malicious
  `.rpk` to write outside `/var/lib/rpkg/info/<name>/` or to leave
  partial files on disk after a failed install.
- Bugs in the unit-file parser (`init/src/lib.rs`) that let a malicious
  unit file crash PID 1.

The following are **out-of-scope** for v0.x:

- Side-channel attacks (Spectre, Meltdown, timing oracles, Rowhammer).
  The kernel does not yet do any speculative-execution mitigations.
- Attacks that require physical access (cold boot, DMA from a PCIe
  card, firmware downgrade).
- Attacks against the host system running QEMU.
- Bugs that only manifest on hardware RacOS doesn't claim to support
  (non-x86_64, non-UEFI).
- Missing features documented as deferred in
  [ADR-019 §Still deferred](docs/adr/ADR-019-security-baseline.md#implementation-status-2026-06-16):
  ASLR, seccomp-like syscall allowlist, secure boot, signed `.rpk`
  packages, mount-flag enforcement. These will be classified as
  vulnerabilities once they're shipped and bypassable; today they're
  open TODOs and don't qualify as security regressions.

## Current security posture

A snapshot. For the detailed list of what's enabled vs deferred, read
[ADR-019's implementation-status section](docs/adr/ADR-019-security-baseline.md#implementation-status-2026-06-16).

**Shipped and enforced:**

- DAC (owner/group/other RWX) on every path-aware syscall.
- Capability model: `CAP_DAC_OVERRIDE`, `CAP_FOWNER`, `CAP_SETUID`,
  `CAP_SETGID`, `CAP_CHOWN`, `CAP_SYS_ADMIN`, `CAP_SYS_BOOT`. Risky
  syscalls (`mount`/`umount`/`mkfs`/`reboot`/`chown`/`setuid`/`setgid`)
  are gated.
- CPU mitigations enabled on every boot that supports them:
  - **SMEP** — ring-0 can't execute user-mapped pages.
  - **SMAP** — ring-0 can't read/write user-mapped pages without an
    explicit `stac`/`clac` bracket (only the syscall entry stub takes
    that bracket).
  - **NX** — data, BSS, stack, and read-only data are mapped with the
    no-execute bit.
- Per-process FD table; FDs don't leak across `sys_exec`.
- Per-task kernel-stack guard page with a sentinel byte checked on
  every context switch — kernel-stack overflows are detected and
  panic with the offending PID instead of corrupting another task.
- `validate_user_ptr` / `validate_user_string` on every syscall
  argument that crosses the ring boundary; the bounds check is the
  documented invariant in every `// SAFETY:` annotation in
  `kernel/src/syscall/handlers.rs` (every block annotated as part of
  T4.2 — see `bash scripts/check-unsafe-safety.sh --strict`).
- **Mandatory CI gate**: `Unsafe-safety annotation lint (--strict)`
  refuses to merge PRs that add an `unsafe {` block without a
  `// SAFETY:` comment in the preceding 5 lines.

**Not yet enforced (won't be treated as vulnerabilities until shipped):**

- Package signature verification (Ed25519). Gated on T4.1 crypto.
- Mount flags (`noexec`, `nosuid`, `nodev`). The flag word is parsed
  but the kernel doesn't enforce per-mount restrictions yet.
- ASLR for user space. `process::from_elf` uses fixed
  `USER_STACK_TOP` / `ET_DYN_LOAD_BIAS`.
- Syscall allowlist per service (seccomp-like). No mechanism in the
  engine or kernel.
- Secure boot. The bootloader doesn't verify the kernel ELF
  signature; the UEFI image is signed by the firmware's chain but the
  kernel itself isn't.
- Crash-dump sanitization. On kernel panic the full register state
  goes straight to serial (there is no userland crash-dump path).

## Public disclosure history

No public vulnerabilities reported to date.

Once reports are processed they will be listed here with:

- CVE ID (if assigned)
- Affected versions
- Brief description
- Link to the fix commit / PR
- Reporter credit (if requested)
