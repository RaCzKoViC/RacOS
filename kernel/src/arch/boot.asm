; RaCore — Kernel entry point (x86_64 assembly)
;
; This is the first code executed when the bootloader jumps to the kernel.
; It sets up the initial stack and calls kernel_main(boot_info).
;
; Calling convention: boot_info pointer is in RDI (System V AMD64 ABI).

section .bss
align 16
stack_bottom:
    resb 65536          ; 64 KiB initial kernel stack
stack_top:

section .text
global _start
extern kernel_main

_start:
    ; RDI already contains the BootInfo pointer from the bootloader
    ; Set up the kernel stack
    lea rsp, [rel stack_top]

    ; Clear RFLAGS
    push 0
    popf

    ; Align stack to 16 bytes (System V ABI requirement)
    and rsp, ~0xF

    ; Call Rust kernel_main(boot_info)
    ; RDI is already set by the bootloader
    call kernel_main

    ; kernel_main should never return, but if it does:
.hang:
    cli
    hlt
    jmp .hang
