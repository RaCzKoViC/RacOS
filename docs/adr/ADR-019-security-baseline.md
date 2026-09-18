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
* **DAC** — `kernel/src/security/dac.rs::can_access` enforces owner/group/other RWX bits with the file's `uid`/`gid` against the caller's `Credentials`. Wired into every path-aware syscall (mkdir/unlink/rename/chmod/chown/open/access) via `require_dac_access`, and since 2026-09 into path *resolution* itself: `mount::lookup_path_as` asks each directory for search permission, so a file's own mode is no longer the only thing protecting it. The components above a mount point are the exception — `resolve()` jumps to the deepest mount and a mount point need not exist below. `dac::can_exec` gates exec/spawn: a regular file with an execute bit, which `CAP_DAC_OVERRIDE` does not grant.
* **Capability model** — `kernel/src/security/capability.rs`. Bitmask of `cap_permitted` / `cap_effective` / `cap_inheritable` on every `Task`. Capabilities defined and used: `CAP_DAC_OVERRIDE`, `CAP_FOWNER`, `CAP_SETUID`, `CAP_SETGID`, `CAP_CHOWN`, `CAP_SYS_ADMIN`, `CAP_SYS_BOOT`. `racos-test::test_security_syscalls` proves the gates (CAP_SETUID denies non-cap callers, CAP_DAC_OVERRIDE lets a cap-effective process write a 0644 file it doesn't own).
* **Per-process credentials** — `Task::creds` snapshotted on fork/clone/exec; new processes inherit parent's `Credentials::root()` for now (PID 1 boots as root). `setuid` reduces the capability masks with the UID (`capability::drop_for_uid_change`, Linux's `cap_emulate_setxuid` without a saved UID), so dropping privileges is one-way.
* **Signals** — `security::process::can_signal` behind `sys_kill`: the sender's real or effective UID must match the target's unless it holds `CAP_KILL`; SIGCONT is allowed within a session. `kill(pid, 0)` is the permission probe (0 / ESRCH / EPERM).
* **SMEP + SMAP** — enabled in `arch::enable_smep_smap` (`kernel/src/arch/mod.rs:128`) gated on CPUID.7:EBX. The syscall entry stub wraps the dispatcher in STAC/CLAC so handlers can touch validated user buffers; everywhere else in ring 0 is blocked from touching user memory by SMAP.
* **NX (no-execute)** — `kernel/src/mm/virt.rs` sets the NX bit on data/BSS/stack mappings (`USER_DATA & !WRITABLE` for RO data, `USER_DATA` for RW, `USER_CODE` for executable text only).
* **User-pointer validation** — `kernel/src/syscall/usercopy.rs` (2026-09). Every syscall argument that crosses the ring boundary is checked against the calling process's page table - present and USER at every level, WRITABLE too for a destination - for every page of the range, and the data is moved by `copy_from_user`/`copy_to_user`/`get_user`/`put_user` with the check and the access under interrupts-off. Until then `validate_user_ptr` checked the address range only: the identity-mapped kernel at 0x100000 passed, and `write(1, 0x100000, 64)` printed kernel bytes (review 2026-09-18, item A). `racos-test::test_usercopy_rejects_bad_pointers` (`T39-USERCOPY-OK`) proves the refusals: kernel text, physical RAM, unmapped, off-the-end-of-a-mapping, read-only text as a write target, non-canonical, overflow, null.
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
