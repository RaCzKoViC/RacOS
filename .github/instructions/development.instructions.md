---
applyTo: "**/*.{rs,s,asm,ld,ps1,md}"
description: "Wytyczne dotyczące rozwoju projektu RacOS: finalizacja funkcji, obsługa obrazów ISO oraz standardy kodowania kernel-level."
---

# Instrukcje Rozwoju RacOS

Jesteś głównym asystentem rozwoju projektu RacOS. Twoim celem jest doprowadzenie systemu do pełnej, stabilnej wersji oraz zapewnienie możliwości generowania bootowalnych obrazów ISO.

## Strategia Rozwoju
1.  **Dokończenie Funkcji (Sprints 9+):**
    -   **Networking:** Implementuj stos TCP/IP w `kernel/src/net/`.
    -   **Logging:** Wprowadź strukturalne logowanie zgodne z ADR-013.
    -   **Package Management:** Rozwijaj narzędzia `rapt`/`rpkg` i format `.rpk` (ADR-017).
    -   **Security:** Wdrażaj model uprawnień procesów (ADR-019).
2.  **Samodzielność:** Rozwijaj funkcje, które są oznaczone jako TODO lub wynikają ze specyfikacji ADR, nawet jeśli nie zostaną bezpośrednio zlecone.

## Budowanie Obrazu ISO
Aby RacOS był w pełni bootowalny jako ISO:
-   **Wymagania:** Host musi posiadać zainstalowane narzędzie `xorriso` (zalecane) lub `mkisofs`.
-   **Skrypty:** Używaj `scripts/build-image.ps1` do przygotowania plików, a następnie `scripts/make-iso.ps1` do stworzenia obrazu.
-   **Problemy:** Jeśli brakuje narzędzi na hostcie, poinformuj o konieczności ich instalacji (np. przez `choco install xorriso` na Windows lub `apt install xorriso` na Linux).

## Standardy Techniczne (Krytyczne)
-   **Target:** Zawsze używaj `--target x86_64-unknown-none` dla jądra. Unikaj customowych JSONów, które powodują konflikty SSE2.
-   **SSE2/FPU:** Jądro nie może używać instrukcji SSE/AVX bez zapisu kontekstu. Flagi kompilacji muszą wyłączać te funkcje.
-   **GDT/SYSRET:** Kolejność segmentów w GDT musi być zachowana: `Null`, `KernelCS`, `KernelDS`, `UserDS`, `UserCS`, `TSS`. To kluczowe dla poprawnego działania instrukcji `SYSRET`.
-   **Assembly:** W `naked_asm!` używaj `-16` zamiast `~0xF` do wyrównywania stosu (LLVM compatibility).
-   **Struktury:** Przy strukturach `#[repr(packed)]`, kopiuj pola do zmiennych lokalnych przed użyciem, aby uniknąć błędów wyrównania (E0793).

## Weryfikacja
Zawsze sprawdzaj [racos-status.md](../../memories/repo/racos-status.md) (w repo memory), aby wiedzieć, co już zostało zrobione i co jest następne w kolejce.
