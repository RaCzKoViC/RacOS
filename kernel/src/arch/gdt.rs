// RaCore — Global Descriptor Table (GDT) for x86_64
//
// Sets up a minimal GDT with:
// - Null descriptor (index 0)
// - Kernel code segment (index 1) — 64-bit, ring 0
// - Kernel data segment (index 2) — ring 0
// - User code segment (index 3) — 64-bit, ring 3 (for future user space)
// - User data segment (index 4) — ring 3 (for future user space)
//
// TSS will be added in Phase C when task switching is implemented.

use core::mem::size_of;

/// GDT entry (8 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct GdtEntry {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl GdtEntry {
    const fn null() -> Self {
        GdtEntry {
            limit_low: 0,
            base_low: 0,
            base_mid: 0,
            access: 0,
            granularity: 0,
            base_high: 0,
        }
    }

    const fn new(access: u8, granularity: u8) -> Self {
        GdtEntry {
            limit_low: 0xFFFF,
            base_low: 0,
            base_mid: 0,
            access,
            granularity,
            base_high: 0,
        }
    }
}

/// GDT pointer structure for `lgdt`.
#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

// Access byte flags
const PRESENT: u8 = 0x80;
const DPL_RING0: u8 = 0x00;
const DPL_RING3: u8 = 0x60;
const SEGMENT: u8 = 0x10;
const EXECUTABLE: u8 = 0x08;
const READ_WRITE: u8 = 0x02;

// Granularity byte flags
const LONG_MODE: u8 = 0x20;   // 64-bit code segment
const GRANULARITY_4K: u8 = 0x80;

static mut GDT: [GdtEntry; 5] = [
    GdtEntry::null(),                                                              // 0: Null
    GdtEntry::new(PRESENT | DPL_RING0 | SEGMENT | EXECUTABLE | READ_WRITE, LONG_MODE | GRANULARITY_4K), // 1: Kernel code
    GdtEntry::new(PRESENT | DPL_RING0 | SEGMENT | READ_WRITE, GRANULARITY_4K),     // 2: Kernel data
    GdtEntry::new(PRESENT | DPL_RING3 | SEGMENT | EXECUTABLE | READ_WRITE, LONG_MODE | GRANULARITY_4K), // 3: User code
    GdtEntry::new(PRESENT | DPL_RING3 | SEGMENT | READ_WRITE, GRANULARITY_4K),     // 4: User data
];

/// Load the GDT and set segment registers.
pub fn init() {
    // SAFETY: Loading GDT is required for kernel operation.
    // The GDT is statically allocated and lives for the entire kernel lifetime.
    // Segment register reload is required after lgdt to activate new descriptors.
    unsafe {
        #[allow(static_mut_refs)]
        let gdt_ptr = GdtPointer {
            limit: (size_of::<[GdtEntry; 5]>() - 1) as u16,
            base: GDT.as_ptr() as u64,
        };

        core::arch::asm!(
            "lgdt [{}]",
            // Reload CS via far return
            "push 0x08",          // Kernel code segment selector
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            // Reload data segment registers
            "mov ax, 0x10",       // Kernel data segment selector
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            in(reg) &gdt_ptr,
            out("rax") _,
            options(nostack),
        );
    }

    crate::serial::serial_println!("[  0.000050] RACORE: GDT loaded (5 entries)");
}

/// Kernel code segment selector.
pub const KERNEL_CS: u16 = 0x08;
/// Kernel data segment selector.
pub const KERNEL_DS: u16 = 0x10;
/// User code segment selector (with RPL=3).
pub const USER_CS: u16 = 0x18 | 3;
/// User data segment selector (with RPL=3).
pub const USER_DS: u16 = 0x20 | 3;
