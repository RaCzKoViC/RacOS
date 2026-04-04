# RacOS — Release Policy

> Version: 0.1.0 | Status: Draft

## 1. Versioning

### 1.1 System Version

Semantic versioning: `MAJOR.MINOR.PATCH`

- **MAJOR**: Breaking ABI changes, architectural shifts
- **MINOR**: New features, new syscalls, non-breaking additions
- **PATCH**: Bug fixes, security fixes

### 1.2 Kernel ABI Version

Separate versioning: `ABI_MAJOR.ABI_MINOR`

- ABI_MAJOR change = breaking change (deprecated syscalls removed)
- ABI_MINOR change = new syscalls added (backward compatible)

## 2. Release Channels

| Channel | Purpose | Quality Gate |
|---------|---------|-------------|
| dev | Development builds | Compiles, unit tests pass |
| testing | Pre-release validation | All tests pass, boot test green |
| stable | Production releases | Full regression, security review, release checklist signed |

## 3. Release Types

| Type | Naming | Purpose |
|------|--------|---------|
| Snapshot | `0.1.0-dev.20260404` | Daily/CI build |
| Alpha | `0.1.0-alpha.1` | Feature-complete milestone |
| Beta | `0.1.0-beta.1` | Feature-frozen, bug-fix only |
| RC | `0.1.0-rc.1` | Release candidate, final testing |
| Release | `0.1.0` | Stable release |

## 4. Release Artifacts

| Artifact | Format | Purpose |
|----------|--------|---------|
| Kernel image | ELF64 | RaCore binary |
| initramfs | Custom archive | Initial root filesystem |
| Bootable ISO | ISO 9660 + UEFI | Installation/live medium |
| Disk image | qcow2 / raw | QEMU direct boot |
| Package repo snapshot | Directory + index | Installable packages |
| Symbol files | .debug | Debugging symbols |
| Changelog | Markdown | What changed |
| SBOM | TOML | Software bill of materials |

## 5. Release Checklist

Before any stable release:

- [ ] All unit tests pass
- [ ] All integration tests pass
- [ ] Boot test passes in QEMU (serial log validated)
- [ ] Shell regression suite passes
- [ ] Terminal escape sequence tests pass
- [ ] Package install/upgrade/rollback tests pass
- [ ] Security checklist reviewed and signed
- [ ] No known critical or high-severity bugs
- [ ] Rollback plan documented and tested
- [ ] Changelog written
- [ ] Artifacts built from clean checkout (reproducibility target)
- [ ] Artifacts signed

## 6. Rollback Plan

Each release must document:
1. How to revert to the previous version
2. ABI compatibility implications of rollback
3. Package database rollback procedure
4. Service configuration rollback

## 7. Support Policy

| Release | Support Duration |
|---------|-----------------|
| Stable | Until next stable release + 30 days |
| Testing | No support guarantee |
| Dev | No support guarantee |

## 8. Roadmap

| Milestone | Target Scope |
|-----------|-------------|
| MVP (0.1.0) | Boot + kernel + memory + scheduler + syscalls + first user process |
| Alpha (0.2.0) | VFS + init + TTY + basic shell |
| Beta (0.3.0) | Full shell + terminal + package system |
| RC (0.4.0) | Security hardening + observability + release tests |
| 1.0.0 | Full test coverage, release engineering, production-ready |

## 9. Build Types

| Type | Flags | Purpose |
|------|-------|---------|
| debug | No optimization, full debug info | Development |
| release | Optimized, stripped | Production |
| asan | AddressSanitizer (userland) | Memory bug detection |
| instrumented | Profiling hooks enabled | Performance analysis |

## 10. CI Pipeline

```
commit → lint → build (debug) → unit tests → kernel tests → boot test (QEMU)
                                                                ↓
                                              integration tests → image build
                                                                ↓
                                                   artifact upload + changelog
```

Failure at any stage blocks the pipeline.
