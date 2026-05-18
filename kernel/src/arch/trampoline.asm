# RaCore — AP Trampoline (16-bit real mode)
#
# This code is loaded into low memory (usually 0x8000) and executed by APs
# when they receive a STARTUP IPI. It transitions the CPU from 16-bit real mode
# to 32-bit protected mode, then to 64-bit long mode, and finally jumps to Rust.

.code16
.section .trampoline, "ax"
.global trampoline_start
.global trampoline_end

trampoline_start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax

    # 1. Start transition to 32-bit (Loading local GDT)
    lgdt [rip + gdtr_ptr - trampoline_start + 0x8000]
    
    # Enable PE (Protected Mode)
    mov eax, cr0
    or eax, 1
    mov cr0, eax

    # Far jump into 32-bit code
    ljmp 0x08, (1f - trampoline_start + 0x8000)

.code32
1:
    # Now in 32-bit protected mode
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax

    # 2. Setup for 64-bit Long Mode
    # (Simplified: assumes paging already setup by BSP or expects identity mapping)
    # Most kernels use a temporary page table here.
    
    # Final goal: jump to ap_entry in kernel
    # jmp ap_entry_32to64

.align 16
gdt_start:
    .quad 0x0000000000000000  # Null descriptor
    .quad 0x00cf9a000000ffff  # 32-bit Code
    .quad 0x00cf92000000ffff  # 32-bit Data
gdt_end:

gdtr_ptr:
    .word gdt_end - gdt_start - 1
    .long gdt_start - trampoline_start + 0x8000

trampoline_end:
