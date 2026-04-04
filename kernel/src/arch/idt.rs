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
            _stack_frame: &InterruptStackFrame,
        ) {
            crate::serial::serial_println!("!!! EXCEPTION #{}: {} !!!", $vector, $msg);
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
exception_handler_with_error!(page_fault, 14, "Page Fault");
exception_handler!(x87_floating_point, 16, "x87 Floating-Point");
exception_handler_with_error!(alignment_check, 17, "Alignment Check");
exception_handler!(machine_check, 18, "Machine Check");
exception_handler!(simd_floating_point, 19, "SIMD Floating-Point");
exception_handler!(virtualization, 20, "Virtualization");

/// Default handler for unregistered interrupts.
extern "x86-interrupt" fn default_handler(_stack_frame: &InterruptStackFrame) {
    // Ignore unregistered interrupts
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

        #[allow(static_mut_refs)]
        let idt_ptr = IdtPointer {
            limit: (size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
            base: IDT.as_ptr() as u64,
        };

        core::arch::asm!("lidt [{}]", in(reg) &idt_ptr, options(nostack));
    }

    crate::serial::serial_println!("[  0.000070] RACORE: IDT loaded ({} entries)", IDT_ENTRIES);
}
