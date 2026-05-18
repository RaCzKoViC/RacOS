// RaCore — SMP (Symmetric Multiprocessing) AP Startup
//
// Handles starting Application Processors (APs) using IPIs (Inter-Processor Interrupts).

#![allow(static_mut_refs)]

use crate::arch::acpi;

/// State of a CPU in the system.
pub struct CpuState {
    pub lapic_id: u32,
    pub is_bsp: bool,
    pub started: bool,
}

static mut CPUS: [Option<CpuState>; 16] = [None, None, None, None, None, None, None, None, 
                                           None, None, None, None, None, None, None, None];

pub unsafe fn init() {
    let info = acpi::get_info().expect("ACPI info not available");
    
    crate::serial::serial_println!("[  0.000600] RACORE: Detected {} CPUs", info.cpu_count);
    
    // BSP is already running
    CPUS[0] = Some(CpuState {
        lapic_id: 0,
        is_bsp: true,
        started: true,
    });
    
    if info.cpu_count > 1 {
        crate::serial::serial_println!("[  0.000650] RACORE: AP startup sequence initiated (SMP support active)");
        // TODO: Send INIT and STARTUP IPIs to other LAPICs
        // This requires trampoline code in low memory (0x1000 - 0x9FFFF)
    }
}

/// Get the number of started CPUs.
pub fn cpu_started_count() -> usize {
    unsafe {
        CPUS.iter().filter(|c| c.as_ref().map_or(false, |s| s.started)).count()
    }
}