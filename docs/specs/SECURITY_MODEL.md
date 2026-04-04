# RacOS — Security Model

> Version: 0.1.0 | Status: Draft

## 1. Principles

1. **Deny by default** — unprivileged processes have no capabilities unless granted
2. **Least privilege** — services run with minimum required permissions
3. **Signed artifacts** — packages and images are signed; unsigned artifacts rejected
4. **Defense in depth** — multiple independent barriers (capabilities, mount flags, user isolation)
5. **No security by obscurity** — all mechanisms documented and testable

## 2. Trust Boundaries

```
┌─────────────────────────────────┐
│         Kernel Space            │  Trusted
│  (RaCore, drivers)              │
├─────────────────────────────────┤  ← Syscall boundary
│  PID 1: RacInit                 │  Privileged (all caps)
├─────────────────────────────────┤
│  System services                │  Restricted capabilities
│  (logging, networking, etc.)    │
├─────────────────────────────────┤
│  User processes                 │  Unprivileged
│  (shell, user programs)         │
└─────────────────────────────────┘
```

## 3. User and Group Model

- Root user (UID 0): exists but services should not run as root without justification
- System users (UID 1-999): for services
- Regular users (UID 1000+): for interactive sessions
- Groups: standard separation (e.g., `disk`, `network`, `audio`, `wheel`)

## 4. Capability Model

Instead of all-or-nothing root privileges, RacOS uses capability bits:

| Capability | Permits |
|------------|---------|
| CAP_NET_BIND | Bind to ports < 1024 |
| CAP_SYS_ADMIN | Mount filesystems, load modules |
| CAP_SYS_KILL | Send signals to any process |
| CAP_SETUID | Change process UID |
| CAP_SETGID | Change process GID |
| CAP_DAC_OVERRIDE | Bypass file permission checks |
| CAP_SYS_TIME | Set system clock |
| CAP_SYS_BOOT | Reboot system |

Capabilities are set:
- Per-process at spawn time
- Bounded by parent's capabilities (can only drop, never gain)
- Declared in service unit files (`CapabilityBoundingSet=`)

## 5. Process Isolation

- Each process has its own address space (page tables)
- Kernel memory is not accessible from user space
- Guard pages between stack and heap
- No shared memory unless explicitly created via IPC syscalls

## 6. Filesystem Security

### 6.1 Permissions

Standard UNIX-style: owner/group/others × read/write/execute

### 6.2 Mount Flags

| Flag | Effect |
|------|--------|
| `ro` | Read-only mount |
| `noexec` | Cannot execute binaries from this mount |
| `nosuid` | Ignore setuid/setgid bits |
| `nodev` | Ignore device nodes on this mount |

Default mount flags for security-sensitive paths:
- `/tmp`: `noexec, nosuid, nodev`
- `/proc` equivalent: `nosuid, nodev`
- `/dev`: `nosuid`

## 7. Syscall Policy

- Each process has a syscall allowlist (post-MVP, similar to seccomp concept)
- Default: all stable syscalls allowed
- Restricted services can have reduced allowlists in unit files
- Violation: process killed with SIGSYS-equivalent

## 8. Package Signing

- Ed25519 signatures required for all packages
- Trusted keys in `/etc/rapt/keys/`
- Key management: `rapt key add`, `rapt key remove`, `rapt key list`
- On signature mismatch: installation aborted, security event logged

## 9. Build Hardening

Compiled artifacts should use:
- Stack protector (`-fstack-protector-strong` or Rust default)
- Position Independent Executables (PIE)
- Read-only relocations (RELRO)
- No executable stack (NX)
- ASLR (user space address randomization, post-MVP)

## 10. Crash Dump Policy

1. Kernel panic: dump registers + backtrace to serial, halt
2. User crash: generate core dump to `/var/crash/` (if enabled)
3. Sanitization: strip environment variables and potential secrets from dumps
4. Crash dumps are owned by root, mode 0600

## 11. Secure Defaults

- No services run as root unless explicitly justified
- No network listeners by default
- Package signature verification enabled
- Core dumps disabled by default (opt-in)
- Login requires authentication (when user model is active)

## 12. Threat Model (high-level)

| Threat | Mitigation |
|--------|------------|
| Buffer overflow in userland | Stack protector, NX, guard pages |
| Malicious package | Signature verification, checksum validation |
| Privilege escalation via syscall | Capability model, syscall allowlist |
| Information leak via crash dump | Dump sanitization |
| Unauthorized service access | Capability bounding, user isolation |
| Kernel memory disclosure | Strict user/kernel space separation |
| Tampered boot image | Signed images (future: secure boot) |

## 13. Required Documents

- [ ] Detailed threat model per component
- [ ] Trust boundary diagram
- [ ] Package signing key rotation policy
- [ ] Incident response runbook
- [ ] Security testing checklist for CI
