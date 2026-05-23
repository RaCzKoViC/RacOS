// RaCore - SMP (Symmetric Multiprocessing)
//
// Application Processor enumeration + (eventually) bring-up. G.1 lands the
// enumeration half: pull parsed CpuInfo out of ACPI, set up per-CPU state
// slots, log the topology. Actual INIT-SIPI-SIPI lives in G.2/G.3.

#![allow(static_mut_refs)]

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::acpi;

/// Hard cap so per-CPU arrays stay statically sized.
pub const MAX_CPUS: usize = 32;

/// Per-CPU state visible to the rest of the kernel.
pub struct CpuState {
    pub apic_id: u32,
    pub acpi_uid: u32,
    pub is_bsp: bool,
    pub is_x2apic: bool,
    /// True once the AP has signalled "I'm alive" from its Rust entry.
    /// Atomic so the AP can flip it without the BSP holding a lock.
    pub started: AtomicBool,
}

impl CpuState {
    const fn empty() -> Self {
        CpuState {
            apic_id: 0,
            acpi_uid: 0,
            is_bsp: false,
            is_x2apic: false,
            started: AtomicBool::new(false),
        }
    }
}

static mut CPUS: [CpuState; MAX_CPUS] = [
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
    CpuState::empty(),
];

static CPU_COUNT: AtomicU32 = AtomicU32::new(0);
static BSP_APIC_ID: AtomicU32 = AtomicU32::new(0);

/// Snapshot ACPI's CPU list into the SMP table. The first enabled entry is
/// treated as the BSP. No IPIs sent here — that's G.2/G.3.
///
/// # Safety
/// Must run on the BSP, after `acpi::init()` populated topology.
pub unsafe fn init() {
    let Some(info) = acpi::get_info() else {
        crate::serial::serial_println!(
            "[  0.000600] RACORE: SMP - no ACPI info, assuming single CPU"
        );
        CPUS[0] = CpuState {
            apic_id: 0,
            acpi_uid: 0,
            is_bsp: true,
            is_x2apic: false,
            started: AtomicBool::new(true),
        };
        CPU_COUNT.store(1, Ordering::SeqCst);
        return;
    };

    let mut idx = 0usize;
    let mut bsp_assigned = false;
    for cpu in &info.cpus {
        if idx >= MAX_CPUS {
            break;
        }
        if !cpu.enabled {
            continue;
        }
        let is_bsp = !bsp_assigned;
        if is_bsp {
            bsp_assigned = true;
            BSP_APIC_ID.store(cpu.apic_id, Ordering::SeqCst);
        }
        CPUS[idx] = CpuState {
            apic_id: cpu.apic_id,
            acpi_uid: cpu.acpi_uid,
            is_bsp,
            is_x2apic: cpu.is_x2apic,
            started: AtomicBool::new(is_bsp),
        };
        idx += 1;
    }
    CPU_COUNT.store(idx as u32, Ordering::SeqCst);

    let started = started_count();
    let total = cpu_count();
    crate::serial::serial_println!(
        "[  0.000600] RACORE: SMP topology - {} enabled CPU(s), BSP apic_id={}, {} online",
        total,
        BSP_APIC_ID.load(Ordering::SeqCst),
        started,
    );

    if total > 1 {
        crate::serial::serial_println!(
            "[  0.000610] RACORE: SMP - {} AP(s) enumerated, waiting for arch::ap::bring_up_all",
            total - 1,
        );
    }
}

pub fn cpu_count() -> usize {
    CPU_COUNT.load(Ordering::SeqCst) as usize
}

pub fn started_count() -> usize {
    let n = cpu_count();
    let mut c = 0usize;
    for i in 0..n {
        unsafe {
            if CPUS[i].started.load(Ordering::SeqCst) {
                c += 1;
            }
        }
    }
    c
}

pub fn bsp_apic_id() -> u32 {
    BSP_APIC_ID.load(Ordering::SeqCst)
}

/// Mark the given LAPIC ID as alive. Called by APs from their Rust entry
/// (G.3). Safe to call from any CPU because each slot's `started` is its
/// own atomic.
pub fn mark_started(apic_id: u32) {
    let n = cpu_count();
    for i in 0..n {
        unsafe {
            if CPUS[i].apic_id == apic_id {
                CPUS[i].started.store(true, Ordering::SeqCst);
                return;
            }
        }
    }
}

/// Iterate parsed CPUs (BSP first). Callback returns `Some(R)` to stop early.
pub fn for_each_cpu<R, F: FnMut(&CpuState) -> Option<R>>(mut f: F) -> Option<R> {
    let n = cpu_count();
    for i in 0..n {
        unsafe {
            if let Some(r) = f(&CPUS[i]) {
                return Some(r);
            }
        }
    }
    None
}
