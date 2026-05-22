// RaCore — Interrupt Descriptor Table (IDT) for x86_64
//
// Sets up the IDT with exception handlers for CPU exceptions (0-31).
// IRQ handlers will be added in Phase C when the timer and devices are initialized.

use core::mem::size_of;

/// Number of IDT entries (256 = full x86_64 IDT).
const IDT_ENTRIES: usize = 256;

/// IDT entry (16 bytes for x86_64).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }

    fn set_handler(&mut self, handler: u64, selector: u16, ist: u8, dpl: u8) {
        self.offset_low = handler as u16;
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.selector = selector;
        self.ist = ist;
        // Type: 0xE = 64-bit interrupt gate, Present bit set
        self.type_attr = 0x80 | ((dpl & 3) << 5) | 0x0E;
        self.zero = 0;
    }
}

/// IDT pointer for `lidt`.
#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::missing(); IDT_ENTRIES];

// Exception handler stubs — minimal handlers that print to serial and halt.
// Full handlers with proper stack frames will be implemented in Phase C.

macro_rules! exception_handler {
    ($name:ident, $vector:expr, $msg:expr) => {
        extern "x86-interrupt" fn $name(
            stack_frame: &InterruptStackFrame,
        ) {
            // Note: in older Rust nightlies, `stack_frame` was a reference to
            // the IRET frame the CPU pushed; in newer nightlies the
            // `extern "x86-interrupt"` ABI passes the same frame but the
            // `InterruptStackFrame` struct fields read back garbage for
            // some toolchains. Fall back to reading the canonical CPU-pushed
            // frame from TSS.RSP0 - 40 so the printout is always correct.
            let rsp0 = crate::arch::gdt::current_kernel_stack();
            let frame_words: [u64; 5] = unsafe {
                core::ptr::read_unaligned(rsp0.wrapping_sub(40) as *const [u64; 5])
            };
            let _ = stack_frame;
            crate::serial::serial_println!(
                "!!! EXCEPTION #{}: {} !!! rip={:#x} cs={:#x} rflags={:#x} rsp={:#x} ss={:#x} pid={}",
                $vector, $msg,
                frame_words[0], frame_words[1], frame_words[2], frame_words[3], frame_words[4],
                crate::task::scheduler::current_pid(),
            );
            loop {
                unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
            }
        }
    };
}

macro_rules! exception_handler_with_error {
    ($name:ident, $vector:expr, $msg:expr) => {
        extern "x86-interrupt" fn $name(
            _stack_frame: &InterruptStackFrame,
            error_code: u64,
        ) {
            crate::serial::serial_println!("!!! EXCEPTION #{}: {} (error: 0x{:X}) !!!", $vector, $msg, error_code);
            loop {
                unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
            }
        }
    };
}

/// Interrupt stack frame pushed by the CPU.
#[repr(C)]
pub struct InterruptStackFrame {
    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
}

// CPU exception handlers (vectors 0-21)
exception_handler!(divide_error, 0, "Division by Zero");
exception_handler!(debug, 1, "Debug");
exception_handler!(nmi, 2, "Non-Maskable Interrupt");
exception_handler!(breakpoint, 3, "Breakpoint");
exception_handler!(overflow, 4, "Overflow");
exception_handler!(bound_range, 5, "Bound Range Exceeded");
exception_handler!(invalid_opcode, 6, "Invalid Opcode");
exception_handler!(device_not_available, 7, "Device Not Available");
exception_handler_with_error!(double_fault, 8, "Double Fault");
exception_handler_with_error!(invalid_tss, 10, "Invalid TSS");
exception_handler_with_error!(segment_not_present, 11, "Segment Not Present");
exception_handler_with_error!(stack_segment_fault, 12, "Stack-Segment Fault");
exception_handler_with_error!(general_protection, 13, "General Protection Fault");

/// Page Fault handler — if user space, kill process; if kernel, halt.
extern "x86-interrupt" fn page_fault(
    stack_frame: &InterruptStackFrame,
    error_code: u64,
) {
    let rip = stack_frame.instruction_pointer;
    let cs = stack_frame.code_segment;
    let cpl = cs & 0x3;
    let current_pid = crate::task::scheduler::current_pid();
    let current_is_user_task = current_pid >= 100
        && crate::task::scheduler::current_page_table_phys() != 0;
    let fault_addr: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) fault_addr, options(nomem, nostack));
    }

    if cpl == 3 || current_is_user_task {
        // Fault in user space — kill the process
        crate::serial::serial_println!(
            "!!! PAGE FAULT in user space: RIP=0x{:X}, CR2=0x{:X}, CS=0x{:X}, error=0x{:X}, PID={}",
            rip,
            fault_addr,
            cs,
            error_code,
            current_pid,
        );
        // Signal handling is not complete yet; terminate immediately to avoid
        // fault loops on the same instruction.
        unsafe { crate::task::scheduler::exit_current(128 + 11); }
    }

    // Fault in kernel space — this is a kernel bug
    crate::serial::serial_println!(
        "!!! KERNEL PAGE FAULT: RIP=0x{:X}, CR2=0x{:X}, CS=0x{:X}, error=0x{:X}",
        rip,
        fault_addr,
        cs,
        error_code,
    );
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
    }
}
exception_handler!(x87_floating_point, 16, "x87 Floating-Point");
exception_handler_with_error!(alignment_check, 17, "Alignment Check");
exception_handler!(machine_check, 18, "Machine Check");
exception_handler!(simd_floating_point, 19, "SIMD Floating-Point");
exception_handler!(virtualization, 20, "Virtualization");

/// Poll-write a single byte to COM1 (0x3F8). Bypasses the rest of the
/// serial subsystem so it can be used safely from very early diagnostics
/// in interrupt handlers and naked asm trampolines.
#[allow(dead_code)]
#[inline(always)]
pub unsafe fn com1_poke(byte: u8) {
    loop {
        let status: u8;
        core::arch::asm!("in al, dx", in("dx") 0x3FDu16, out("al") status, options(nomem, nostack, preserves_flags));
        if status & 0x20 != 0 { break; }
    }
    core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") byte, options(nomem, nostack, preserves_flags));
}

/// Default handler for unregistered interrupts.
extern "x86-interrupt" fn default_handler(_stack_frame: &InterruptStackFrame) {
    // Ignore unregistered interrupts
}

/// Timer IRQ handler (vector 32 = IRQ0).
extern "x86-interrupt" fn timer_handler(_stack_frame: &InterruptStackFrame) {
    crate::interrupts::pit::tick();
    // NOTE: net::stack::poll() is intentionally NOT called here. It would
    // need to acquire STACK.lock(), which user-mode wait loops (resolve,
    // sys_connect, sys_recv) briefly hold. On single-core, the spin would
    // deadlock. Polling is done explicitly from idle_loop and from user
    // wait loops instead.
    crate::net::tcp::tick();
    crate::task::scheduler::timer_tick();
    crate::interrupts::pic::send_eoi(0);
}

/// Serial COM1 IRQ handler (vector 36 = IRQ4).
extern "x86-interrupt" fn serial_handler(_stack_frame: &InterruptStackFrame) {
    crate::serial::handle_irq();
    crate::interrupts::pic::send_eoi(4);
}

/// Keyboard IRQ handler (vector 33 = IRQ1).
extern "x86-interrupt" fn keyboard_handler(_stack_frame: &InterruptStackFrame) {
    crate::drivers::ps2_keyboard::handle_irq_input();
    crate::interrupts::pic::send_eoi(1);
}

/// Per-CPU LAPIC timer IRQ handler (vector 0x40, see G.4.1).
///
/// Bumps the running CPU's `tick_count` via its own GS base — single
/// memory bus operation, no scheduler call, no shared lock. Every CPU
/// runs this same handler against its own PerCpu slot, so there's no
/// cross-CPU contention even when N CPUs all fire the timer at once.
extern "x86-interrupt" fn lapic_timer_handler(_stack_frame: &InterruptStackFrame) {
    // Bump this CPU's own counter via GS base. No `lock` prefix: only the
    // owning CPU writes its tick_count, so an atomic RMW is overkill and
    // we want a single non-locked memory op in the IRQ fast path.
    unsafe {
        core::arch::asm!(
            "inc qword ptr gs:[{off}]",
            off = const crate::arch::percpu::OFFSET_TICK_COUNT,
            options(nostack, preserves_flags),
        );
    }
    crate::arch::lapic::eoi();
}

/// Load the IDT with exception handlers.
pub fn init() {
    // SAFETY: IDT is statically allocated and lives for the kernel lifetime.
    // Handler function pointers are valid for the kernel's lifetime.
    // We set handlers for known CPU exceptions (0-20).
    #[allow(function_casts_as_integer)]
    unsafe {
        IDT[0].set_handler(divide_error as u64, 0x08, 0, 0);
        IDT[1].set_handler(debug as u64, 0x08, 0, 0);
        IDT[2].set_handler(nmi as u64, 0x08, 0, 0);
        IDT[3].set_handler(breakpoint as u64, 0x08, 0, 0);
        IDT[4].set_handler(overflow as u64, 0x08, 0, 0);
        IDT[5].set_handler(bound_range as u64, 0x08, 0, 0);
        IDT[6].set_handler(invalid_opcode as u64, 0x08, 0, 0);
        IDT[7].set_handler(device_not_available as u64, 0x08, 0, 0);
        IDT[8].set_handler(double_fault as u64, 0x08, 0, 0);
        IDT[10].set_handler(invalid_tss as u64, 0x08, 0, 0);
        IDT[11].set_handler(segment_not_present as u64, 0x08, 0, 0);
        IDT[12].set_handler(stack_segment_fault as u64, 0x08, 0, 0);
        IDT[13].set_handler(general_protection as u64, 0x08, 0, 0);
        IDT[14].set_handler(page_fault as u64, 0x08, 0, 0);
        IDT[16].set_handler(x87_floating_point as u64, 0x08, 0, 0);
        IDT[17].set_handler(alignment_check as u64, 0x08, 0, 0);
        IDT[18].set_handler(machine_check as u64, 0x08, 0, 0);
        IDT[19].set_handler(simd_floating_point as u64, 0x08, 0, 0);
        IDT[20].set_handler(virtualization as u64, 0x08, 0, 0);

        // Fill remaining with default handler
        for i in 21..IDT_ENTRIES {
            if IDT[i].type_attr == 0 {
                IDT[i].set_handler(default_handler as u64, 0x08, 0, 0);
            }
        }

        // IRQ handlers (PIC remapped: IRQ0 = vector 32)
        IDT[32].set_handler(timer_handler as u64, 0x08, 0, 0);
        // IRQ1 = vector 33 (keyboard)
        IDT[33].set_handler(keyboard_handler as u64, 0x08, 0, 0);
        // IRQ4 = vector 36 (COM1 serial)
        IDT[36].set_handler(serial_handler as u64, 0x08, 0, 0);

        // G.4.1: per-CPU LAPIC timer at vector 0x40.
        IDT[crate::arch::lapic::LAPIC_TIMER_VECTOR as usize]
            .set_handler(lapic_timer_handler as u64, 0x08, 0, 0);

        #[allow(static_mut_refs)]
        let idt_ptr = IdtPointer {
            limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
            base: IDT.as_ptr() as u64,
        };

        core::arch::asm!("lidt [{}]", in(reg) &idt_ptr, options(nostack));
    }

    crate::serial::serial_println!("[  0.000070] RACORE: IDT loaded ({} entries)", IDT_ENTRIES);
}

/// Load the IDT on the running CPU. Called from `ap_entry` after the AP
/// has its per-CPU slot and LAPIC enabled. The IDT itself is shared with
/// the BSP — every CPU just needs its own `lidt` so IRQs route into our
/// handlers instead of triple-faulting against an empty IDTR.
///
/// # Safety
/// Must run with interrupts disabled (caller's responsibility) and after
/// `init()` has populated the IDT entries on the BSP.
pub unsafe fn load_on_this_cpu() {
    #[allow(static_mut_refs)]
    let idt_ptr = IdtPointer {
        limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
        base: IDT.as_ptr() as u64,
    };
    core::arch::asm!("lidt [{}]", in(reg) &idt_ptr, options(nostack));
}
