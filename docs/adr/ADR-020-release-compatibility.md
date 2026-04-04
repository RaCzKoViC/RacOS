# ADR-020: Release Policy and Compatibility

**Status**: Accepted
**Date**: 2026-04-04

## Context

Users and developers need to understand what stability guarantees RacOS provides, how versions are numbered, and what to expect from upgrades.

## Decision

### Versioning
- System version: semantic versioning (MAJOR.MINOR.PATCH)
- Kernel ABI version: separate ABI_MAJOR.ABI_MINOR
- Both are independent (system 1.2.0 may have ABI 1.5)

### Compatibility guarantees
- Stable syscalls persist across minor versions; removal requires major bump + 2-version deprecation
- Package format versioned independently; rpkg supports latest 2 format versions
- Service unit file format changes are additive (unknown keys = warning)

### Release channels
- **stable**: full testing, security review, release checklist
- **testing**: all automated tests pass, community testing
- **dev**: compiles, unit tests pass, no further guarantees

### Release process
1. Code freeze on branch
2. Full test suite execution
3. Security checklist review
4. Changelog and SBOM generation
5. Artifact build from clean checkout
6. Artifact signing
7. Publication to appropriate channel
8. Rollback plan verified

### Support
- Stable releases supported until next stable + 30 days
- No support guarantees for testing/dev

## Consequences

- Users can rely on stable syscall numbers across minor versions
- Upgrades within a major version are safe
- Major version bumps signal potential breakage
- Release process is documented and repeatable

## Risks

- Premature stability commitment locks in suboptimal design (mitigate: use Unstable/Experimental labels aggressively)
- Release process overhead (mitigate: automate as much as possible in CI)

## Rollback

Release policy itself can be updated via new ADR. Existing releases are immutable once published.
