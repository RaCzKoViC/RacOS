# ADR-019: Security Baseline

**Status**: Accepted
**Date**: 2026-04-04

## Context

Security must be designed from the start, not bolted on later. RacOS needs a baseline of security mechanisms that are enabled by default.

## Decision

### Baseline principles
1. Deny by default where possible
2. Least privilege for all services
3. Signed artifacts for packages and images
4. Capability separation between processes
5. Defense in depth (multiple independent barriers)

### Mechanisms enabled by default
- User/group ownership and permissions on all files
- Capability model (processes start with no special capabilities unless granted)
- Mount flags: /tmp with noexec,nosuid,nodev; /proc with nosuid,nodev
- Package signature verification mandatory
- Stack protector enabled in all builds
- NX (no-execute) bit enforced
- Crash dump sanitization (strip environment variables)
- No services as root without explicit justification in unit file

### Post-MVP additions
- Syscall allowlist per service (seccomp-like)
- ASLR for user space
- Secure boot support

## Consequences

- Security is a compile-time and boot-time property, not an add-on
- Every new service must justify its capability requirements
- Security tests are part of CI

## Risks

- Overly restrictive defaults may break initial development (mitigate: dev channel has relaxed defaults for testing)
- Capability model complexity (mitigate: start with small set, extend as needed)

## Rollback

Individual mechanisms can be toggled via kernel config and unit file settings. Relaxing defaults is possible but requires ADR.

## Implementation status (2026-06-16)

About half of §Decision's mechanisms-enabled-by-default are shipped. Capabilities + DAC + SMEP/SMAP + NX are real and tested; signature verification, secure boot, ASLR, and seccomp are still TODO.

**Shipped:**
* **DAC** — `kernel/src/security/dac.rs::can_access` enforces owner/group/other RWX bits with the file's `uid`/`gid` against the caller's `Credentials`. Wired into every path-aware syscall (mkdir/unlink/rename/chmod/chown/open/access) via `require_dac_access`.
* **Capability model** — `kernel/src/security/capability.rs`. Bitmask of `cap_permitted` / `cap_effective` / `cap_inheritable` on every `Task`. Capabilities defined and used: `CAP_DAC_OVERRIDE`, `CAP_FOWNER`, `CAP_SETUID`, `CAP_SETGID`, `CAP_CHOWN`, `CAP_SYS_ADMIN`, `CAP_SYS_BOOT`. `racos-test::test_security_syscalls` proves the gates (CAP_SETUID denies non-cap callers, CAP_DAC_OVERRIDE lets a cap-effective process write a 0644 file it doesn't own).
* **Per-process credentials** — `Task::creds` snapshotted on fork/clone/exec; new processes inherit parent's `Credentials::root()` for now (PID 1 boots as root).
* **SMEP + SMAP** — enabled in `arch::enable_smep_smap` (`kernel/src/arch/mod.rs:128`) gated on CPUID.7:EBX. The syscall entry stub wraps the dispatcher in STAC/CLAC so handlers can touch validated user buffers; everywhere else in ring 0 is blocked from touching user memory by SMAP.
* **NX (no-execute)** — `kernel/src/mm/virt.rs` sets the NX bit on data/BSS/stack mappings (`USER_DATA & !WRITABLE` for RO data, `USER_DATA` for RW, `USER_CODE` for executable text only).
* **User-pointer validation** — `validate_user_ptr` + `validate_user_string` bound every syscall argument that crosses the ring boundary. Result is annotated in every `// SAFETY:` comment under `kernel/src/syscall/handlers.rs` (T4.2).
* **Per-process FD table** — `Task::fd_table`; close-on-exec where set; FDs don't leak across `sys_exec`.
* **Capabilities required for risky ops** — `sys_mount`/`sys_umount`/`sys_mkfs` require `CAP_SYS_ADMIN`; `sys_reboot` requires `CAP_SYS_BOOT`; `sys_chown` requires `CAP_CHOWN`.

**Still deferred:**
* **Package signature verification** — gated on crypto (see ADR-018 status). Today `rpkg install` accepts unsigned `.rpk` files.
* **Stack protector** — Rust's overflow-check on debug builds catches integer overflow into UB; the `-Z stack-protector` flag isn't passed yet because the kernel uses a custom stack-guard sentinel (per-task `KERNEL_STACK_GUARD_BYTE`, checked on every context switch — see `task/scheduler.rs:check_kernel_stack_guards`) which is stricter than the LLVM canary approach and works for kernel stacks specifically.
* **Mount flags** (noexec/nosuid/nodev) — `sys_mount` parses a `flags` argument but the kernel doesn't enforce per-mount restrictions yet. Today every mount is effectively `rw,suid,exec,dev`.
* **Crash dump sanitization** — there's no crash-dump path; on panic the kernel prints the full register state to serial and halts. PID-namespacing isn't relevant since there are no namespaces.
* **Syscall allowlist per service** (seccomp-like) — not in the engine, not in the kernel.
* **ASLR for user space** — `process::from_elf` puts the user stack at a fixed `USER_STACK_TOP` and ET_DYN binaries at a fixed `ET_DYN_LOAD_BIAS`. No randomization source.
* **Secure boot** — bootloader doesn't verify the kernel ELF signature; the UEFI image is signed by the firmware's chain but the kernel itself isn't.
* **Dev-channel relaxation** — there's only one build channel today.

The §Consequences "security is a compile-time and boot-time property" is half true: DAC + caps + SMEP/SMAP + NX + validate_user_ptr are all on by default and proven by tests. The signed-artifacts + seccomp + ASLR half is the T4.x roadmap.
