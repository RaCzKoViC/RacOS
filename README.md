# RacOS

**Autorski system operacyjny z architekturą warstwową inspirowaną Ubuntu.**

RacOS to w pełni autorski system operacyjny budowany od zera — bez kopiowania kodu Linux, Ubuntu, Debian ani GNU. Architektura wzoruje się na logicznym modelu warstwowym: boot → kernel → user space → init/usługi → pakietowanie → shell → terminal → narzędzia systemowe.

## Komponenty

| Komponent | Nazwa | Opis |
|-----------|-------|------|
| Kernel | **RaCore** | Modular monolithic kernel w Rust + x86_64 ASM |
| Init/Service Manager | **RacInit** | Autorski init z dependency graph, restart policy, unit files |
| Shell | **racsh** | Powłoka systemowa z AST-based parserem, job control, scripting |
| Terminal | **RacTerm** | Emulator terminala z ANSI/VT, PTY, scrollback, dirty rendering |
| Pkg (low-level) | **rpkg** | Lokalny instalator pakietów, baza plików, hooki |
| Pkg (high-level) | **rapt** | Resolver zależności, repozytoria, kanały, bezpieczne aktualizacje |

## Platforma

- **Architektura CPU**: x86_64
- **Firmware**: UEFI
- **Środowisko**: QEMU/KVM
- **Boot**: UEFI → bootloader → kernel ELF64 + initramfs

## Stos technologiczny

- **Kernel**: Rust + x86_64 assembly
- **Userland**: C17 + libc-lite (faza początkowa)
- **Build**: clang/llvm, lld, nasm, cargo, just+ninja
- **CI**: lint → build → unit tests → kernel tests → boot tests → integration tests → image build

## Struktura repozytorium

```
/RacOS
  /boot          — bootloader i boot flow
  /kernel        — jądro RaCore
    /arch/x86_64 — kod architektoniczny
    /mm          — zarządzanie pamięcią
    /sched       — scheduler
    /task        — model procesów
    /syscall     — syscall ABI
    /ipc         — IPC, sygnały, pipe
    /vfs         — Virtual File System
    /fs          — systemy plików
    /drivers     — sterowniki
    /net         — sieć
    /security    — moduły bezpieczeństwa
    /time        — zegary i timery
    /debug       — debug infrastructure
  /init          — RacInit service manager
  /libs          — biblioteki systemowe
  /userland      — narzędzia użytkownika
  /shell         — racsh
  /terminal      — RacTerm
  /pkg           — rpkg + rapt + repo-tools
  /services      — definicje usług systemowych
  /images        — obrazy systemu
  /scripts       — skrypty pomocnicze
  /toolchain     — konfiguracja narzędzi
  /docs          — dokumentacja
  /tests         — testy
  /ci            — konfiguracja CI/CD
```

## Budowanie

```bash
just build        # pełny build
just test         # uruchom testy
just run          # uruchom w QEMU
just image        # zbuduj obraz ISO
```

## Dokumentacja

- [ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md) — architektura systemu
- [BOOT_FLOW.md](docs/specs/BOOT_FLOW.md) — flow rozruchu
- [KERNEL_ABI.md](docs/specs/KERNEL_ABI.md) — ABI jądra
- [SYSCALL_SPEC.md](docs/specs/SYSCALL_SPEC.md) — specyfikacja wywołań systemowych
- [SERVICE_MODEL.md](docs/specs/SERVICE_MODEL.md) — model usług
- [SHELL_GRAMMAR.md](docs/specs/SHELL_GRAMMAR.md) — gramatyka shella
- [TERMINAL_PROTOCOLS.md](docs/specs/TERMINAL_PROTOCOLS.md) — protokoły terminala
- [PACKAGE_FORMAT.md](docs/specs/PACKAGE_FORMAT.md) — format pakietów
- [SECURITY_MODEL.md](docs/specs/SECURITY_MODEL.md) — model bezpieczeństwa
- [RELEASE_POLICY.md](docs/specs/RELEASE_POLICY.md) — polityka wydań
- [TEST_STRATEGY.md](docs/specs/TEST_STRATEGY.md) — strategia testów
- [ADRs](docs/adr/) — Architecture Decision Records

## Licencja

Licensed under the **Apache License, Version 2.0** — see [LICENSE](LICENSE) for the
full text. Copyright © 2026 RaCzKoViC.

You may use, modify, and distribute this software under the terms of the
license. See <http://www.apache.org/licenses/LICENSE-2.0> for details.
