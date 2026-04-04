# RacOS — Test Strategy

> Version: 0.1.0 | Status: Draft

## 1. Testing Levels

| Level | Scope | Runner | Frequency |
|-------|-------|--------|-----------|
| Unit | Individual functions/modules | `cargo test` / C test harness | Every commit |
| Integration | Multi-module interaction | Custom harness | Every commit |
| Boot | Kernel boots in QEMU, serial validated | `scripts/boot-test` | Every commit |
| E2E | Full system scenario | QEMU + automation | Pre-release |
| Fuzz | Parser/protocol boundaries | `cargo-fuzz` / AFL | Nightly |
| Fault injection | Service manager, IPC | Custom injector | Weekly |
| Soak | Long-running stability | QEMU 24h run | Pre-release |
| Security | Security checklist validation | Manual + automated | Pre-release |

## 2. Per-Component Test Requirements

### 2.1 Kernel (RaCore)

| Area | Test Type | Minimum Coverage |
|------|-----------|-----------------|
| Memory allocator | Unit | Alloc, free, double-free detection, OOM |
| Page tables | Unit | Map, unmap, permission enforcement |
| Scheduler | Unit + integration | Context switch, priority, starvation prevention |
| Syscalls | Integration | Each syscall: happy path + all error codes |
| Exception handlers | Unit | Page fault, GP fault, divide-by-zero |
| Interrupt handling | Integration | Timer fires, keyboard input delivered |

### 2.2 Init (RacInit)

| Area | Test Type |
|------|-----------|
| Unit file parsing | Unit (valid and invalid inputs) |
| Dependency graph | Unit (DAG, cycle detection) |
| Service lifecycle | Integration (start, stop, restart, timeout, failure) |
| Restart policy | Integration (on-failure, always, burst limit) |
| Log routing | Integration |

### 2.3 Shell (racsh)

| Area | Test Type |
|------|-----------|
| Lexer | Golden tests (input → tokens) |
| Parser | Golden tests (tokens → AST) |
| Quoting | Edge case suite (nested, escaped, empty) |
| Expansion | Unit (variables, tilde, glob) |
| Pipelines | Integration (2, 3, N stages) |
| Redirections | Integration (>, >>, <, 2>, 2>&1) |
| Job control | Integration (bg, fg, Ctrl-C, Ctrl-Z) |
| Scripts | Regression suite (scripts with expected output) |
| Parser fuzzing | Fuzz (random/malformed input) |

### 2.4 Terminal (RacTerm)

| Area | Test Type |
|------|-----------|
| Escape sequences | Unit (each CSI, SGR, OSC) |
| Partial sequences | Unit (split at every byte) |
| Screen buffer | Unit (cursor, erase, scroll, wrap) |
| Colors | Unit (16, 256, truecolor) |
| Resize | Integration (shrink, grow, content preservation) |
| High throughput | Performance (100MB output, bounded memory) |
| Sequence fuzzing | Fuzz (random byte streams) |

### 2.5 Package System (rpkg + rapt)

| Area | Test Type |
|------|-----------|
| Package format | Unit (valid/invalid .rpk parsing) |
| Install/remove | Integration |
| Signature verification | Unit (valid, invalid, missing) |
| Dependency resolution | Unit (simple, diamond, conflict) |
| Upgrade/rollback | Integration |
| Repository index | Unit (parse, refresh) |

## 3. Boot Test Procedure

```
1. Build kernel + initramfs
2. Launch QEMU with serial on stdio
3. Capture serial output for 10 seconds
4. Validate:
   a. Boot banner present with build number
   b. Memory detection message
   c. GDT/IDT loaded messages
   d. Scheduler ready message
   e. Init process created message
   f. No panic messages
5. Exit QEMU
6. Report pass/fail
```

## 4. Regression Testing

- All tests from previous releases must pass on new code
- New bugs get a regression test before fix is merged
- Test suite is append-only (tests are never removed without ADR)

## 5. CI Integration

```yaml
# Conceptual CI stages
stages:
  - lint:
      - cargo clippy --workspace -- -D warnings
      - cargo fmt --all -- --check
  - build:
      - cargo build --workspace
  - test-unit:
      - cargo test --workspace
  - test-boot:
      - scripts/boot-test.sh
  - test-integration:
      - scripts/integration-test.sh
  - build-image:
      - scripts/build-image.sh
```

## 6. Test Pass/Fail Criteria

- **Pass**: All assertions pass, no unexpected panics, no memory leaks (where detectable)
- **Fail**: Any assertion failure, unexpected panic, timeout, memory corruption detected

## 7. Test Data Management

- Golden test data stored in `tests/fixtures/`
- Expected outputs stored alongside inputs
- Fixtures version-controlled, never auto-generated during CI
