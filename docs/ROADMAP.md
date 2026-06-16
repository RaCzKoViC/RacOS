# RacOS — Roadmapa rozwoju

> Status: Living document
> Utworzona: 2026-06-16
> Last updated: 2026-06-16 (T1.1 + T1.2 done)

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

- [ ] **T1.3 — Wire RacInit service manager**
  - Engine istnieje w bibliotece `racinit`, niepodłączony do PID 1 boot path
  - Pozostaje:
    - [ ] Parser unit files (.service / .target) zgodny ze spec `docs/specs/SERVICE_MODEL.md`
    - [ ] Dependency graph z cycle detection (DAG)
    - [ ] Restart policy (always / on-failure / never) + burst limit
    - [ ] Podpięcie do PID 1 boot path zamiast hardcoded spawnu `/bin/sh`
    - [ ] `servicectl` CLI: start/stop/restart/status/list (spec w ARCHITECTURE.md §8.4)
  - **Definition of done:** boot z 3 unitami (target → 2 services), test restart-on-failure

---

## 3. Tier 2 — przejście z demo do developable OS

> Kolejne 1–2 cykle po Tier 1.

- [ ] **T2.1 — VirtIO-block + persistence**
  - Driver pod `kernel/src/drivers/virtio_blk.rs`
  - Racfs zmountowane na realnym dysku zamiast ramdiska
  - `/etc`, `/var`, `/home` przeżywające reboot
  - **Definition of done:** boot smoke test który zapisuje plik, reboot QEMU, weryfikuje że plik istnieje

- [ ] **T2.2 — RacTerm: minimal ANSI emulator**
  - CSI (kursor: CUU/CUD/CUF/CUB/CUP, erase: ED/EL, scroll regions: DECSTBM)
  - SGR (kolory 16 + 256 + truecolor)
  - Alternate screen buffer (`\e[?1049h/l`)
  - Scrollback ring 10k linii (konfigurowalne)
  - Spec referencyjny: `docs/specs/TERMINAL_PROTOCOLS.md`
  - **Definition of done:** ncurses-style "hello" działa (np. mini `vi`-clone), scroll, kolory

- [ ] **T2.3 — Phase 1 cross-platform build**
  - Port `justfile` + skryptów PowerShell na bash (utrzymać oba)
  - `docs/DEVELOPMENT_LINUX.md` z full setup
  - CI weryfikujący że bash-side i ps-side dają identyczny output
  - **Definition of done:** kontrybutor na Ubuntu może zbudować + odpalić smoke test w QEMU

---

## 4. Tier 3 — droga do v1.0

- [ ] **T3.1 — SMP**
  - Podłączyć AP trampoline do init (`kernel/src/smp.rs`)
  - Per-CPU run queues z work stealing
  - IPI dla preemption (LAPIC ICR)
  - **Definition of done:** boot QEMU `-smp 4`, parallel test który widzi 4 cpu w `/proc/cpuinfo`

- [ ] **T3.2 — rpkg MVP**
  - Bez signatures, bez resolvera — sam parser `.rpk` + install/remove/list
  - Spec referencyjny: `docs/specs/PACKAGE_FORMAT.md`
  - **Definition of done:** zbudować `.rpk` z testowego userland tool'a, zainstalować, uruchomić, usunąć

- [ ] **T3.3 — Userland: dokończyć stuby**
  - `env` — pełne `getenv`/`setenv`/`unsetenv` + iteracja po environ
  - `ps` — real reader procfs (must-have do debugowania od momentu gdy init odpala >1 procesu)
  - `sed` — minimal: `s/X/Y/g`, `d`, `p`, `-n`
  - `awk` — minimal: pola `$1..$N`, `BEGIN/END`, basic actions

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
