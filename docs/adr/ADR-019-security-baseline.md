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
