# ADR-012: Service Unit File Format

**Status**: Accepted
**Date**: 2026-04-04

## Context

RacInit needs a file format for defining services, targets, timers, and mounts. The format must be human-readable, parseable without complex libraries, and extensible.

## Decision

INI-style format with defined sections: `[Unit]`, `[Dependencies]`, `[Service]`, `[Timer]`, `[Mount]`, `[Install]`. Files stored in `/etc/racinit/` with extensions matching type (`.service`, `.target`, `.timer`, `.mount`, `.device`).

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| TOML | More complex parser needed; INI is simpler for this use case |
| YAML | Whitespace-sensitive, complex parser, error-prone |
| JSON | Not human-friendly for config files |
| Custom DSL | Unnecessary complexity for what is essentially key-value config |

## Consequences

- Simple parser (section headers + key=value pairs)
- Well-defined set of keys per section
- Unknown keys generate warnings (forward compatibility)
- Validation at parse time with clear error messages
- See SERVICE_MODEL.md for full format specification

## Rollback

Format can be versioned. New format version can coexist with old if a version header is added.
