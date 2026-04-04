/* RaCore — Linker script for x86_64 kernel
 *
 * Lays out the kernel at higher-half virtual address.
 * Physical load address is at 1 MiB (0x100000) for simplicity.
 * Virtual address starts at 0xFFFF800000100000 (higher-half + 1 MiB offset).
 */

ENTRY(_start)

KERNEL_PHYS_BASE = 0x100000;
KERNEL_VIRT_BASE = 0xFFFF800000100000;
KERNEL_OFFSET    = KERNEL_VIRT_BASE - KERNEL_PHYS_BASE;

SECTIONS
{
    . = KERNEL_VIRT_BASE;

    .text ALIGN(4K) : AT(ADDR(.text) - KERNEL_OFFSET)
    {
        *(.text .text.*)
    }

    .rodata ALIGN(4K) : AT(ADDR(.rodata) - KERNEL_OFFSET)
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) : AT(ADDR(.data) - KERNEL_OFFSET)
    {
        *(.data .data.*)
    }

    .bss ALIGN(4K) : AT(ADDR(.bss) - KERNEL_OFFSET)
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
