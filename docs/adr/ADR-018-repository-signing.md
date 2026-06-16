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

## Implementation status (2026-06-16)

Package format ships (T3.2); signing and the repository protocol are both still spec-only — both gated on having a crypto stack, which is being built out in T4.1.

**Shipped:**
* Package format in `pkg/rpkg/` — header + sections + manifest TOML parser + files-list serializer.
* CLI in `pkg/rpkg-bin/` (`/bin/rpkg install|list|remove`) — writes payloads to `/var/lib/rpkg/info/<name>/` and tracks installed files for clean removal. Smoke `T32-RPKG-OK` builds a `.rpk` in memory, installs/lists/removes it round-trip.
* `rapt` skeleton (host-side dependency planner) — `pkg/rapt/`.

**Still deferred (everything from §Decision and §Signing):**
* **`/etc/rapt/sources.toml`** — no on-disk repository config; `rapt` doesn't read from a network mirror yet.
* **Repository `index.toml`** — no signed index format, no fetcher.
* **Ed25519 signatures** — no crypto in tree. Once T4.1 lands ChaCha20-Poly1305 + Ed25519 (the same crypto stack TLS needs), signing can be wired here. Package manifests already have the field hooks for a `signature` block; today rpkg accepts unsigned packages because `allow_unsigned = true` is the implicit default.
* **Trusted keys at `/etc/rapt/keys/`** — directory is not created at install time.
* **Channels** (stable/testing/dev) — there's no repository to channel.
* **Mirror support / channel pinning** — both are post-§Decision-MVP from the start.

This ADR's §Risks section is essentially the post-MVP TODO list for T4.1 + a follow-up repository PR.
