# RacOS — Roadmapa rozwoju

> Status: Living document
> Utworzona: 2026-06-16
> Last updated: 2026-06-16 (Tier 1 complete + Tier 2 complete)

Ten dokument jest źródłem prawdy o kierunku rozwoju RacOS. Każda większa praca powinna pasować do jednego z tierów poniżej. Zmiana priorytetów wymaga aktualizacji tego pliku w PR-ze.

---

## 1. Stan obecny (snapshot 2026-06-16)

### Działa solidnie
- **Bootloader UEFI** — production-ready (ELF64 loader, GOP, mmap, ACPI, ExitBootServices)
- **Kernel core** — MM (phys/virt allocator, page tables), task model, 15+ syscalli, VFS z 5 zamontowanymi FS (initramfs/devfs/tmpfs/procfs + racfs + FAT32 writable), round-robin scheduler
- **Stack sieciowy end-to-end** — ARP → IPv4 → UDP → DNS → TCP → HTTP/1.0 bez third-party crate'ów
- **Userland** — ~36 binarek w `/bin`, większość pełnych implementacji POSIX; `libc-lite` z ~80 syscall wrapperami inline-asm
- **Shell racsh** — 4362 linii, AST-based parser, lexer/parser/AST/expand/exec/builtin/readline jako osobne moduły
- **CI** — zielona macierz na ubuntu/macOS/windows + smoke testy UEFI/QEMU/interactive shell

### Krytyczne luki blokujące real use
1. **Init nie ma service managera w boot path** — RacInit jako PID 1 tylko spawnuje shell i reapuje zombies. Model unit files / dependency graph / restart policy jest spec-only (engine.rs istnieje, niepodłączony). RacOS nie potrafi uruchomić więcej niż jednego programu userspace.
2. **Shell scripting nie istnieje** — racsh jest interactive-only. Brak `if/while/for`, `$?`, `$0..$9`, brak ładowania skryptów z pliku, brak `source`. Bez tego żaden service unit nie ma sensu.
3. **Brak persistence na realnym dysku** — racfs żyje na ramdisku. Brak sterownika VirtIO-block → nic nie przeżywa reboota.
4. **RacTerm to stub** — wrapper PTY + przekazywanie bajtów. Brak parsera ANSI/VT, brak scrollback, brak dirty rendering. Wszystkie 7 wymagań z ARCHITECTURE.md §10 nie spełnione.
5. **Sygnały user-space nie deliver'ują się do handlerów** — `deliver_pending_signals` nie pushuje SignalFrame ani nie przekierowuje RIP do handlera; `sys_sigreturn` jest stubem. Bez tego nie ma poprawnego Ctrl+C, job control, ani restart policy w init.
6. **rpkg/rapt to skeletony** — brak parsera formatu `.rpk`, brak resolvera, brak repo. Nie da się dystrybuować ani aktualizować.
7. **Single-core** — `smp.rs` + AP trampoline istnieją ale nie są podłączone do init.

### Niespójności dokumentacji
- `ARCHITECTURE.md` §1.3 mówi "Userland phase 1: C17" — kod jest w Rust no_std. Dokument zdezaktualizowany.
- ADR-006 / ADR-007 / ADR-014 vs faktyczna implementacja — warto audyt zgodności po Phase 2/3.

---

## 2. Tier 1 — chokepointy, bez których nic się nie ruszy

> 1–2 cykle pracy. Te trzy są ze sobą sprzężone: sygnały odblokowują job control w shellu, scripting odblokowuje sensowne unit files, service manager odblokowuje wieloprocesową przestrzeń użytkownika. Po nich RacOS przestaje być "boot demo" a staje się rozwijalnym systemem.

- [x] **T1.1 — Dokończyć Phase 2 (signals + cleanup)** — zmergowane w PR #6
  - PR #5 dał: close fds on exit, SIGCHLD do parenta, ioctl routing
  - PR #6 dodał:
    - [x] User-handler delivery via `PER_CPU.syscall_frame_ptr` (gs:[0x10]) + on-stack `UserSignalFrame`
    - [x] `sys_sigreturn` przywraca orig_rip/rflags/rsp/rax z `UserSignalFrame`
    - [x] libc-lite `signal()` + `__signal_dispatcher` (naked) — bridge kernel→user handler bez modyfikacji SYSRET path
    - [x] 2 integration testy w racos-test: `PHASE21-USER-HANDLER-OK` + `PHASE21-USER-HANDLER-REENTRANT-OK`

- [x] **T1.2 — Shell scripting w racsh**
  - Parser już miał AST, exec runtime też (if/while/for/case/function); dodano brakujące kawałki + testy
  - Status sub-zadań:
    - [x] Runtime dla `if/then/elif/else/fi`, `while/do/done`, `for ... in ... do ... done`, `case/esac` (już było w `shell/src/exec.rs`)
    - [x] Parameter expansion: `$?`, `$0..$9`, `$#`, `$@`, `$*`, `${VAR}`, `${VAR:-default}`, `${VAR:+w}`, `${VAR:?e}`, `${#VAR}` (już było w `shell/src/expand.rs`)
    - [x] `source` (alias `.`) — ładowanie skryptu w bieżącym shellu (`shell/src/builtin.rs:builtin_source`, `run_source_in_env`)
    - [x] Invokacja `sh script.sh` z pliku — już była w `userland/coreutils/sh/src/main.rs`
    - [x] Field splitting — unquoted `$VAR` / `${VAR}` / `$(cmd)` split na IFS w `expand_word_list`; kwotowane `"$VAR"` nie split
    - [x] Testy: 14 host-side tests w `shell/tests/control_flow.rs` + `T12-SHELL-CONTROL-FLOW-OK` marker w racos-test (sh -c z if/for/case)
  - **Pozostałe gaps (post-MVP):** mixed words `prefix$VAR` z partial splitting, configurable IFS variable, command sub `$(cmd)` runtime (parser wspiera, exec stub)

- [x] **T1.3 — Wire RacInit service manager**
  - Status sub-zadań:
    - [x] Parser unit files (.service / .target / .timer / .mount / .device) — już był w `init/src/lib.rs:parse_unit`, dodane testy
    - [x] Dependency graph z cycle detection — fixed buggy Kahn's algorithm w `Engine::resolve_start_order`; zwraca `ResolveResult { order, cycle }`
    - [x] Restart policy (always / on-failure / on-abnormal / no) + burst limit (5 restartów w 30s → Failed)
    - [x] Podpięcie do PID 1 boot path — `userland/coreutils/init/main.rs` próbuje engine path, fallback do legacy spawn-shell loop jeśli brak unit files
    - [x] Default unit files w initramfs: `base.target` + `shell.service` (Restart=always)
    - [x] Resolved collision: usunięty `init/src/main.rs` + `[[bin]]` z init/Cargo.toml; jedyne źródło `/sbin/init` to crate `racos-init`
    - [x] Build scripts (build-image.sh + .ps1) nie wywalają już całego `/etc/` przy clean
  - **Pozostałe (post-T1.3):** `servicectl` CLI (spec w ARCHITECTURE.md §8.4), socket activation, .timer scheduler
  - **Tests:** 13 host tests w `init/tests/engine.rs` (parser, topo sort: linear/diamond/cycle/self-edge, burst tracker window decay) + racos-test smoke `T13-INIT-ENGINE-OK` w QEMU

---

## 3. Tier 2 — przejście z demo do developable OS

> Kolejne 1–2 cykle po Tier 1.

- [x] **T2.1 — Persistence wired in CI**
  - Discovery showed the heavy lifting was already done: `BlockDevice` trait, AHCI driver (`kernel/src/drivers/ahci.rs`, 522 lines), racfs on `sda` mounted at `/mnt` (`kernel/src/main.rs:322`), and `vfs::racfs::persistence_test` writing a `boot-counter` file that grows by 1 each boot. The actual gap was that CI never attached a disk, so the persistence path was dead code in CI.
  - This PR fills the gap:
    - [x] Boot-smoke now creates an empty 16 MiB `disk.img` and attaches it via `-drive file=disk.img,if=ide,format=raw` (q35's built-in ich9-ahci controller → kernel sees `sda`)
    - [x] CI runs QEMU **twice** with the same image — boot 1 formats + writes counter=1, boot 2 reads it back and bumps to 2
    - [x] New grep assertions: `created boot-counter = 1 (first boot)` on boot 1, `boot-counter = 2 (was 1, file survived reboot)` on boot 2 — failure mode is explicit ("sda missing or racfs format failed" vs "persistence broken")
    - [x] All existing kernel/init/racsh banner assertions re-applied to boot 2 (catches regressions caused by disk being present)
    - [x] Both boot1.log + boot2.log uploaded as artifacts
  - **Pozostałe (deferred):** VirtIO-block driver as alternative to AHCI (cleaner QEMU integration, mostly cosmetic); userland file-level persistence test via racos-test (kernel-level is sufficient for v0); making `/etc`, `/var`, `/home` persistent (currently initramfs/ram-based; needs init-side migration).

- [x] **T2.2 — RacTerm: ANSI emulator tested + response drain fixed**
  - Discovery: the emulator (1616 lines: buffer/cursor/escape/terminal) was already implemented and feature-complete for all DoD items — full CSI handler (CUU/CUD/CUF/CUB/CUP, ED/EL, IL/DL, SU/SD, ICH/DCH/ECH, SCP/RCP, DECSTBM, DSR, DA), SGR (16+256+truecolor + bold/italic/underline), DEC private (cursor show/hide, alternate buffer 1049), OSC 0 title, ESC (RIS/DECSC/DECRC/IND/NEL/RI), scrollback ring 10k, partial-sequence buffering. What was missing was test coverage + a bug in the PTY relay.
  - This PR fills the gap:
    - [x] Fixed: `terminal::Terminal::drain_response()` was never called in racterm's main PTY loop, so DSR (`\e[6n` cursor position query) and DA (`\e[c`) responses queued in the emulator never reached the shell. ncurses-style apps waiting for the reply would hang. main.rs now drains the response buffer back to ptmx_fd after each `term.feed(...)` cycle.
    - [x] 31 host tests in `terminal/tests/ansi.rs`: parser (Print/Execute/partial-CSI/private-CSI), cursor movement (CUP absolute, CUU/CUD/CUF/CUB relative, clamp at edges), erase (ED 0/2, EL 0/2), SGR (basic + bright + 256 + truecolor + attrs + reset), alternate buffer (1049h/l with cursor save/restore), DECTCEM cursor visibility, DECSTBM scroll region, DSR (CPR + status), DA primary, scrollback retention, OSC 0 title, CR/LF semantics.
    - [x] racterm's `[[bin]]` now gated behind `required-features = ["bin-target"]` so `cargo test -p racterm` on host doesn't pull libc-lite's `_start` / `panic_handler` (which need a bare-metal target). build-image.sh / build-image.ps1 / justfile / CI workflow updated to pass `--features racterm/bin-target` to the workspace build.
    - [x] CI host test command now includes `racterm` and `init` alongside racsh/rpkg/rapt — 75 host tests run on every push.
  - **Pozostałe (deferred):** real renderer that reads from `Terminal::buffer` and updates a framebuffer (currently the host terminal does rendering via PTY byte forwarding — sufficient for v0 since RacOS runs over serial); UTF-8 multibyte handling in the print path; mouse-tracking modes (1000/1006).

- [x] **T2.3 — Phase 1 cross-platform build**
  - Discovery: the bulk was already in place. `justfile` already has `[unix]`/`[windows]` attributes routing every build/run/image/iso recipe to the right shell. Bash counterparts existed for the heavy lifters (`build-image.sh`, `make-image.sh`, `make-iso.sh`, `boot-test.sh`); `pack-initramfs.py` covers the initramfs packing cross-platform. CI has been building from `build-image.sh` for months. `DEVELOPMENT_LINUX.md` existed too. Real gap: the local "did I just break CI?" loop (`run-ci-smoke.ps1`) was Windows-only.
  - This PR fills the gap:
    - [x] `scripts/run-ci-smoke.sh` — bash port of the PS smoke runner. Rebuilds the kernel with `--features ci-smoke` and the static-relocation RUSTFLAGS the bootloader needs, stages it into `esp/`, launches QEMU with `isa-debug-exit`, and asserts exit code 33 (PASS) / 35 (FAIL) / 124 (timeout). Supports `--disk` to attach the AHCI image that the boot-smoke two-boot test uses.
    - [x] `justfile` gets `smoke` and `smoke-disk` recipes routed to the bash/PS script via `[unix]`/`[windows]` attributes — `just smoke` works on Ubuntu.
    - [x] `DEVELOPMENT_LINUX.md` documents `just smoke` / `just smoke-disk` and the exit-code contract.
  - **Pozostałe (deferred):** CI parity check that runs `bash scripts/run-ci-smoke.sh` alongside the inline kernel-smoke job (would catch script rot but is a doubling of an already-covered code path; skipped for v0). PowerShell-only local helpers (`launch-interactive.ps1`, `runtime-validation-*.ps1`, `validate-*.ps1`) — not needed for the contributor-on-Ubuntu DoD path.

---

## 4. Tier 3 — droga do v1.0

- [~] **T3.1 — SMP** (AP bring-up exercised in CI; per-CPU run queues + IPI preemption deferred to Tier 4 as scheduler refactor)
  - Discovery: heavy lifting was already done. `arch::smp::init()` enumerates ACPI CPUs into a 32-slot CpuState table, `arch::ap::bring_up_all` lives in `kernel/src/arch/ap.rs` with the full INIT-SIPI-SIPI flow, a real-mode → protected → long-mode trampoline in `kernel/src/arch/trampoline.asm`, each AP loads the kernel GDT/IDT, enables its LAPIC, binds its GS to its PerCpu slot, starts its LAPIC timer, and flips `smp::mark_started`. `bring_up_all` is wired into `kernel_main` at line 162.
  - This PR exercises the path in CI:
    - [x] `/proc/cpuinfo` now iterates `arch::smp::for_each_cpu`, emitting one Linux-style block per **online** CPU (with `processor`, `apicid`, `role` BSP/AP, `apic_mode` xapic/x2apic). Fallback to a single hardcoded block keeps `grep ^processor` callers sane in the impossible "0 online" case.
    - [x] CI boot-smoke and kernel-smoke jobs now pass `-smp 4` to QEMU. New boot-smoke assertions: `SMP topology - 4 enabled CPU(s)` in `smp::init` output AND ≥ 3 distinct `AP apic_id=N alive` lines from `bring_up_one` — proves enumeration saw 4 CPUs AND the trampoline brought 3 APs all the way to mark_started.
    - [x] Existing smoke gated behind `--features ci-smoke` (kernel-smoke-isadbg job) now also runs against 4 CPUs, exercising `PASS smp::all_aps_started (4/4)` plus the GS-base / IDT / timer self-checks that follow it.
  - **Pozostałe (deferred to Tier 4 — scheduler refactor):** per-CPU run queues with work stealing (current scheduler is single global queue; refactor is substantial), IPI for cross-CPU preemption (LAPIC ICR send-vector path), per-CPU TSS for ring-3 interrupts on APs (today APs only handle ring-0 timer IRQs while parked).

- [x] **T3.2 — rpkg MVP**
  - Discovery: rpkg lib (246 linii) had the header parser + section extractor + manifest TOML reader + install-plan helper, plus 2 tests. The matching binary skeleton in `pkg/rpkg-bin/` was dead code (wrong libc-lite path, not in workspace, only printed a plan — never wrote anything).
  - This PR closes the gap:
    - [x] Extended rpkg lib with `serialize_files_list`/`parse_files_list` (host-testable) + 4 new tests. 6 rpkg tests now run in CI.
    - [x] Rewrote `pkg/rpkg-bin` end-to-end: install/list/remove subcommands. Install parses the .rpk, writes `manifest.toml` + `files` index + `data` payload into `/var/lib/rpkg/info/<name>/`. List does getdents on the info root. Remove reads the files index, unlinks each path, then drops the info dir.
    - [x] rpkg-bin wired into workspace, build-image.sh + .ps1 BIN_LIST and Coreutils list (cargo-bin name `rpkg-bin`, installed as `/bin/rpkg`).
    - [x] `racos-test::test_rpkg_install_list_remove` builds a minimal .rpk in memory, writes it to `/tmp/demo.rpk`, then spawns `/bin/rpkg install /tmp/demo.rpk`, `/bin/rpkg list`, `/bin/rpkg remove demo-rpkg` — asserting exit 0 for each and emitting `T32-RPKG-OK`.
  - **Pozostałe (deferred):** signature verification (no crypto yet), dependency resolution (rapt territory), repository protocol, multi-file packages (DATA is single-payload in MVP — multi-file would need a real archive format), `/bin/` deployment after T4.x makes initramfs writable or rootfs is on persistent disk.

- [~] **T3.3 — Userland: dokończyć stuby** (ps + sed shipped; env/awk pending)
  - [x] **`ps`** — real procfs reader. Walks `/proc` via getdents, reads `/proc/<pid>/status` for each numeric pid dir, prints `PID PPID STATE NAME` columns.
    - Fixed the dead-code state it was in: wrong libc-lite dep path (`../../../../` → `../../../`), missing `alloc` feature, missing from workspace `members`/`default-members`, missing from BIN_LIST in both build scripts (so the binary was never staged into initramfs). Procfs already serves `/proc/<pid>/status` in key:value form so no kernel changes were needed.
    - `racos-test::test_ps_lists_running_processes` smoke spawns `/bin/ps` and asserts exit 0; emits `T33-PS-OK` marker.
  - [ ] `env` — pełne `getenv`/`setenv`/`unsetenv` + iteracja po environ (still 22-line stub printing only PWD+PATH; needs proper envp inheritance in fork/exec first)
  - [x] **`sed`** — MVP stream editor. Single-command scripts on byte-level input (no regex, no addresses, no multi-command `;`/`-e`).
    - Same dead-code wiring fixes as the ps PR: bad libc-lite path, no `alloc` feature, not in workspace, not in BIN_LIST.
    - Supported commands: `s/X/Y/[g]` (substitute first/global), `d` (delete = skip default print), `p` (explicit print). The `-n` flag suppresses the default per-line print so `p` is the only path that emits output.
    - `racos-test::test_sed_substitute` exercises all five paths (s, s/g, s/no-g, -n p, d) via shell command substitution + case-match assertions, emits `T33-SED-OK`.
  - [ ] `awk` — minimal: pola `$1..$N`, `BEGIN/END`, basic actions (not present)

---

## 5. Tier 4 — strategiczne

- [ ] **T4.1 — TLS/HTTPS w stacku sieciowym**
  - Crypto (ed25519 dla podpisów, ChaCha20-Poly1305 dla TLS) — duża praca
  - Odkładać do momentu gdy będzie repo paczek (T3.2 done)

- [ ] **T4.2 — Audyt `unsafe` pod policy z ARCHITECTURE.md §3.3**
  - Każdy `unsafe { ... }` musi mieć WHY / INVARIANT / FAILURE / TESTED BY
  - Wymaga grep `unsafe` + uzupełnień + ewentualnie clippy lint custom

- [ ] **T4.3 — Synchronizacja ADR/spec z kodem**
  - `ARCHITECTURE.md` §1.3 (Rust vs C17) — aktualizacja
  - ADR-006 (process/thread model), ADR-007 (scheduler), ADR-014 (TTY/PTTY) — audyt po Tier 1
  - CHANGELOG.md → wprowadzić, prowadzić od następnej minor

---

## 6. Co dalej (post v1.0)

Eksplicytnie wykluczone z v1.0 wg `ARCHITECTURE.md` §13 (zostawione jako placeholder):
- GUI desktop environment
- Pełna kompatybilność glibc / Linux userspace
- Kontenery na poziomie Dockera
- Szeroki support sterowników HW
- ARM, RISC-V, inne architektury
- Real-time scheduling

---

## 7. Jak aktualizować tę roadmapę

- Każda zmiana priorytetów → PR z update tego pliku + uzasadnienie w commit message
- Ukończenie zadania → zaznacz `[x]` w tym pliku w PR-ze który zamyka pracę
- Nowa praca która nie pasuje do żadnego tieru → dyskusja w issue zanim trafi do roadmapy
- Snapshot stanu obecnego (sekcja 1) refreshować po każdym domknięciu tieru
