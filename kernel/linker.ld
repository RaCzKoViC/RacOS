/* RaCore — Linker script for x86_64 kernel
 *
 * Phase B: Kernel runs at physical addresses (identity-mapped by UEFI).
 * Physical/virtual load address at 1 MiB (0x100000).
 * Phase C will add higher-half remap to 0xFFFF800000100000.
 */

ENTRY(_start)

KERNEL_PHYS_BASE = 0x100000;

SECTIONS
{
    . = KERNEL_PHYS_BASE;

    .text ALIGN(4K) :
    {
        *(.text .text.*)
    }

    .rodata ALIGN(4K) :
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) :
    {
        *(.data .data.*)
    }

    .bss ALIGN(4K) :
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    /* Discard debug and comment sections in release builds */
    /DISCARD/ :
    {
        *(.comment)
        *(.note.*)
    }

    __kernel_end = .;
}
