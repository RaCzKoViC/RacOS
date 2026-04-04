# ADR-017: Package Format (.rpk)

**Status**: Accepted
**Date**: 2026-04-04

## Context

RacOS needs its own package format for distributing and installing software. The format must be simple to parse, verifiable, and support rollback.

## Decision

Custom archive format `.rpk` with:
- Magic header `RPK\x01` + format version
- Sections: MANIFEST (TOML), CHECKSUMS (sha256 per file), SIGNATURE (ed25519), HOOKS (tar), DATA (tar)
- Manifest contains: name, version, arch, dependencies, conflicts, provides, service declarations, config file list

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| .deb format | Tied to dpkg internals, Debian-specific |
| .rpm format | Complex spec, Red Hat-specific |
| Tar + metadata | No integrity verification built in |
| Flatpak/Snap | Runtime dependencies too heavy for early OS |
| Plain tar.gz | No manifest, no signatures, no hooks |

## Consequences

- Full control over format evolution
- Tools (rpkg, rapt) parse only one format
- Clear extension point (new sections can be added with backward compatibility)
- Signature verification is mandatory by default

## Risks

- Custom format means custom tooling (mitigate: simple design, well-tested parser)
- No ecosystem compatibility (acceptable: RacOS packages are RacOS-only)

## Rollback

Format version field allows introducing v2 format while v1 parser remains.
