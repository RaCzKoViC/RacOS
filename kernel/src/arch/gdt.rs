// RaCore — Global Descriptor Table (GDT) for x86_64
//
// GDT layout (7 entries + 1 TSS which is 2 GDT slots = 9 slots total):
//   0: Null
//   1: Kernel code (0x08) — 64-bit, ring 0
//   2: Kernel data (0x10) — ring 0
//   3: User data  (0x20) — ring 3 (must come BEFORE user code for SYSRET)
//   4: User code  (0x28) — 64-bit, ring 3
//   5-6: TSS descriptor (16 bytes = 2 GDT slots)
//
// NOTE: For SYSCALL/SYSRET, the segment layout must be:
//   STAR.SYSRET_CS_SS = 0x18 (user data at +0, user code at +8)
//   So: GDT[3]=user_data, GDT[4]=user_code
//   SYSRET loads CS from STAR[63:48]+16, SS from STAR[63:48]+8

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

/// Task State Segment (104 bytes minimum for x86_64).
#[repr(C, packed)]
pub struct Tss {
    _reserved0: u32,
    /// RSP for privilege level 0 (ring 3 → ring 0 transition).
    pub rsp0: u64,
    pub rsp1: u64,
    pub rsp2: u64,
    _reserved1: u64,
    /// Interrupt Stack Table entries (IST1-IST7).
    pub ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    /// I/O map base address.
    pub iomap_base: u16,
}

impl Tss {
    const fn new() -> Self {
        Tss {
            _reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            _reserved1: 0,
            ist: [0; 7],
            _reserved2: 0,
            _reserved3: 0,
            iomap_base: size_of::<Tss>() as u16,
        }
    }
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

// TSS type: 0x89 = Present + 64-bit Available TSS
const TSS_PRESENT_TYPE: u8 = 0x89;

/// GDT: 5 normal entries + 2 entries for TSS (16-byte descriptor) = 7 slots
static mut GDT: [GdtEntry; 7] = [
    GdtEntry::null(),                                                              // 0x00: Null
    GdtEntry::new(PRESENT | DPL_RING0 | SEGMENT | EXECUTABLE | READ_WRITE, LONG_MODE | GRANULARITY_4K), // 0x08: Kernel code
    GdtEntry::new(PRESENT | DPL_RING0 | SEGMENT | READ_WRITE, GRANULARITY_4K),     // 0x10: Kernel data
    GdtEntry::new(PRESENT | DPL_RING3 | SEGMENT | READ_WRITE, GRANULARITY_4K),     // 0x18: User data (before user code for SYSRET)
    GdtEntry::new(PRESENT | DPL_RING3 | SEGMENT | EXECUTABLE | READ_WRITE, LONG_MODE | GRANULARITY_4K), // 0x20: User code
    GdtEntry::null(),                                                              // 0x28: TSS low (filled at runtime)
    GdtEntry::null(),                                                              // 0x30: TSS high (filled at runtime)
];

/// Global TSS instance.
pub static mut TSS: Tss = Tss::new();

/// TSS selector in the GDT.
pub const TSS_SELECTOR: u16 = 0x28;

/// Set the kernel stack pointer in the TSS (for ring 3 → ring 0 transitions).
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn set_kernel_stack(rsp0: u64) {
    let tss = &mut *core::ptr::addr_of_mut!(TSS);
    tss.rsp0 = rsp0;
}

/// Install the TSS descriptor into the GDT and load it.
unsafe fn install_tss() {
    let tss_addr = core::ptr::addr_of!(TSS) as u64;
    let tss_size = (size_of::<Tss>() - 1) as u64;
    let gdt = &mut *core::ptr::addr_of_mut!(GDT);

    // TSS descriptor is 16 bytes (occupies 2 GDT slots)
    // Low 8 bytes (slot 5):
    gdt[5] = GdtEntry {
        limit_low: (tss_size & 0xFFFF) as u16,
        base_low: (tss_addr & 0xFFFF) as u16,
        base_mid: ((tss_addr >> 16) & 0xFF) as u8,
        access: TSS_PRESENT_TYPE,
        granularity: ((tss_size >> 16) & 0x0F) as u8,
        base_high: ((tss_addr >> 24) & 0xFF) as u8,
    };

    // High 8 bytes (slot 6): upper 32 bits of base address + reserved
    let high_bytes: [u8; 8] = {
        let mut buf = [0u8; 8];
        let upper = ((tss_addr >> 32) as u32).to_le_bytes();
        buf[0] = upper[0];
        buf[1] = upper[1];
        buf[2] = upper[2];
        buf[3] = upper[3];
        // bytes 4-7 are reserved (0)
        buf
    };
    gdt[6] = unsafe { core::mem::transmute(high_bytes) };
}

/// Load the GDT, set segment registers, install and load TSS.
pub fn init() {
    unsafe {
        // Install TSS descriptor into GDT slots 5-6
        install_tss();

        #[allow(static_mut_refs)]
        let gdt_ptr = GdtPointer {
            limit: (size_of::<[GdtEntry; 7]>() - 1) as u16,
            base: (*core::ptr::addr_of!(GDT)).as_ptr() as u64,
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

        // Load TSS
        core::arch::asm!(
            "ltr ax",
            in("ax") TSS_SELECTOR,
            options(nostack),
        );
    }

    crate::serial::serial_println!("[  0.000050] RACORE: GDT loaded (7 entries + TSS)");
}

/// Kernel code segment selector.
pub const KERNEL_CS: u16 = 0x08;
/// Kernel data segment selector.
pub const KERNEL_DS: u16 = 0x10;
/// User data segment selector (with RPL=3).
pub const USER_DS: u16 = 0x18 | 3;
/// User code segment selector (with RPL=3).
pub const USER_CS: u16 = 0x20 | 3;

/// STAR MSR value for SYSCALL/SYSRET.
///
/// Bits [47:32] = SYSCALL CS/SS base. CPU loads CS = base, SS = base+8.
///   We want kernel CS = 0x08, kernel SS = 0x10, so base = 0x08.
///
/// Bits [63:48] = SYSRET CS/SS base. In 64-bit mode the CPU loads
///   SS = (base + 8) | 3 and CS = (base + 16) | 3 (RPL forced to 3).
///   We want SS = USER_DS = 0x1B and CS = USER_CS = 0x23.
///   That requires base = 0x10:
///     SS = (0x10 + 8) | 3 = 0x18 | 3 = 0x1B ✓
///     CS = (0x10 + 16) | 3 = 0x20 | 3 = 0x23 ✓
///
/// Setting base to 0x18 (the address of the user-data selector itself) is
/// the natural-looking but incorrect choice — it makes SYSRET produce
/// CS = 0x28 (TSS) and SS = 0x20 (user code), which silently corrupts
/// segment-register descriptor caches.
pub const STAR_VALUE: u64 = (0x0010_u64 << 48) | (0x0008_u64 << 32);
