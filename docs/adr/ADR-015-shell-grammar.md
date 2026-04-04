# ADR-015: Shell Grammar (racsh)

**Status**: Accepted
**Date**: 2026-04-04

## Context

racsh needs a well-defined grammar that is predictable in both interactive and script modes. The grammar determines what programs users can write and how they are interpreted.

## Decision

racsh grammar is a subset of POSIX sh with selected extensions:
- Simple commands, pipelines, and/or lists, sequences
- Redirections (>, >>, <, 2>, 2>&1)
- Subshells `(...)` and brace groups `{ ...; }`
- if/elif/else/fi, while/do/done, for/in/do/done, case/esac
- Functions
- Variable expansion, command substitution `$(...)`, globbing
- Quoting: single, double, backslash escape

Architecture: **lexer → parser → AST → semantic validation → expansion → execution** — all strictly separated modules.

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| Full bash compatibility | Enormously complex, undocumented edge cases |
| Fish-like syntax | Non-standard, would confuse users expecting POSIX-like |
| Custom non-POSIX language | Breaks user expectations, scripts not portable |
| POSIX sh strict compliance | Some POSIX behaviors are confusing; selected deviations are OK if documented |

## Consequences

- Parser is testable independently (golden tests)
- Scripts from other systems may need minor adaptation
- Grammar is formally documented (BNF in SHELL_GRAMMAR.md)
- Parser complexity is bounded by defined grammar

## Risks

- Missing constructs frustrate power users (mitigate: add features incrementally)
- Grammar ambiguities (mitigate: comprehensive test suite, BNF spec)

## Rollback

Grammar can be extended (additive); removing constructs requires deprecation period.
