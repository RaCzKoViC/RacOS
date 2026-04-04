# ADR-018: Repository Model and Package Signing

**Status**: Accepted
**Date**: 2026-04-04

## Context

Packages must be distributed through repositories with integrity verification. Users must be able to trust that packages come from authorized sources.

## Decision

### Repository Model
- Repository configuration in `/etc/rapt/sources.toml`
- Each repository has: name, URL, channel (stable/testing/dev), priority, signing key
- Repository index: `index.toml` listing all packages with versions, checksums, sizes, dependencies
- Index itself is signed

### Signing
- Algorithm: **Ed25519**
- Package signatures cover: MANIFEST hash + CHECKSUMS hash + DATA hash
- Repository index signatures cover: entire index.toml content
- Trusted keys stored in `/etc/rapt/keys/`
- Unsigned packages rejected by default (`allow_unsigned = false` in config)

### Channels
| Channel | Purpose |
|---------|---------|
| stable | Tested, production-ready releases |
| testing | Pre-release validation |
| dev | Continuous builds from main branch |

## Consequences

- All package installations verified against signing keys
- Repositories can be mirrored (future)
- Channel pinning (future): user can lock specific packages to specific channels

## Risks

- Key compromise (mitigate: key rotation procedure, documented in security policy)
- Repository availability (mitigate: local cache, offline install via rpkg)

## Rollback

Signing algorithm can be upgraded by adding new key type support alongside Ed25519.
