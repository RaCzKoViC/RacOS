---
description: "Skoncentruj się na sfinalizowaniu projektu RacOS, analizując status i planując kolejne kroki."
name: "Finalizuj RacOS"
argument-hint: "Wpisz obszar, nad którym chcesz teraz pracować (np. sieć, system plików, optymalizacja)..."
agent: "plan"
---

Twoim zadaniem jest robienie wszystkiego co w twojej mocy aby projekt RacOS w pełni został sfinalizowany.

### Kontekst Projektu
- **Status:** [racos-status.md](../../memories/repo/racos-status.md) (przeczytaj ten plik, aby poznać aktualny stan sprintów i architektury)
- **Architektura:** [ARCHITECTURE.md](../../docs/architecture/ARCHITECTURE.md)
- **Specyfikacje:** [docs/specs/](../../docs/specs/)

### Cel
Doprowadź do pełnej funkcjonalności systemu RacOS, skupiając się na:
1. Analizie brakujących elementów względem specyfikacji ADR (Architectural Decision Records).
2. Rozwiązaniu problemów z buildem (patrz: [build_error.txt](../../build_error.txt)).
3. Implementacji brakujących podsystemów (np. stos sieciowy, zaawansowane sterowniki, pełna obsługa userland).

### Instrukcje
- Jeśli użytkownik podał konkretny obszar: {{argument}}, skup się na nim w pierwszej kolejności.
- Zawsze weryfikuj zmiany względem `rust-toolchain.toml` i specyficznych wymagań targetu `x86_64`.
- Proponuj konkretne zadania do wykonania w formie listy TODO.
- Dbaj o spójność z modelem modularnego monolitu opisanym w ADR.
