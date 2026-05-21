// RaCore - Per-CPU storage via GS base (Phase G.4 foundation)
//
// Every CPU lands the address of its own `PerCpu` slot into IA32_GS_BASE.
// Code that runs on any CPU can then read `[gs:0]` to find its slot in O(1)
// without scanning the SMP table by apic_id. This is the building block
// the future per-CPU runqueue + load balancing (G.4 proper) will hang
// off — for now it just lets each CPU identify itself.
//
// IA32_GS_BASE (MSR 0xC0000101) gives the "active" GS base in kernel mode.
// SYSCALL/SYSRET swap to IA32_KERNEL_GS_BASE (0xC0000102) — that's a
// concern for ring-3 entry on APs but not for the BSP-only smoke right
// now. When APs start running user code in a later phase we'll mirror the
// base into KERNEL_GS_BASE and emit `swapgs` in the SYSCALL stub.

#![allow(static_mut_refs)]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::arch::smp::MAX_CPUS;

/// Per-CPU state. First field must be `self_ptr` so `[gs:0]` returns the
/// address of this struct — that's the standard trick that lets callers
/// then dereference at `[gs:8]`, `[gs:16]`, etc. for other fields without
/// another MSR read.
#[repr(C)]
pub struct PerCpu {
    /// Mirror of the struct's own address. Set during init; readable as
    /// `[gs:0]` from any kernel code on this CPU.
    pub self_ptr: AtomicU64,
    /// LAPIC ID of the CPU that owns this slot.
    pub apic_id: AtomicU32,
    /// Self-check field — each CPU writes its own apic_id here via its own
    /// GS base during init. BSP-side smoke reads back and compares; if the
    /// CPUs' GS bases got crossed, the values won't line up.
    pub self_check: AtomicU32,
}

impl PerCpu {
    const fn empty() -> Self {
        PerCpu {
            self_ptr:   AtomicU64::new(0),
            apic_id:    AtomicU32::new(0),
            self_check: AtomicU32::new(0),
        }
    }
}

/// One slot per supported CPU. Wrapped in UnsafeCell because each CPU
/// writes *only* its own slot, and only after init has placed the address
/// into that CPU's GS base — so there's no cross-CPU shared mutable
/// access through this array at runtime.
struct CpuSlot(UnsafeCell<PerCpu>);
unsafe impl Sync for CpuSlot {}

static AREAS: [CpuSlot; MAX_CPUS] = {
    const E: CpuSlot = CpuSlot(UnsafeCell::new(PerCpu::empty()));
    [E; MAX_CPUS]
};

// ── MSR helpers ───────────────────────────────────────────────────────────

const MSR_GS_BASE: u32 = 0xC000_0101;

#[inline]
unsafe fn wrmsr(msr: u32, val: u64) {
    let low  = val as u32;
    let high = (val >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") low,
        in("edx") high,
        options(nomem, nostack, preserves_flags),
    );
}

// ── Public API ────────────────────────────────────────────────────────────

/// Initialise this CPU's per-CPU area: find the slot for `apic_id`, write
/// its address into IA32_GS_BASE so `current()` works, populate apic_id +
/// self_check via the freshly-active GS base (proves the write went to the
/// right place).
///
/// # Safety
/// Must run on the CPU that owns `apic_id`. Each CPU calls exactly once.
pub unsafe fn init_for_this_cpu(apic_id: u32) {
    let idx = slot_index_for(apic_id);
    let slot_ptr = AREAS[idx].0.get();

    // Seed apic_id + self_ptr in the slot before flipping GS so cross-CPU
    // readers (the BSP-side smoke) see a coherent view.
    (*slot_ptr).apic_id.store(apic_id, Ordering::SeqCst);
    (*slot_ptr).self_ptr.store(slot_ptr as u64, Ordering::SeqCst);

    wrmsr(MSR_GS_BASE, slot_ptr as u64);

    // Confirm via GS base that we own the slot we think we own. If the
    // wrmsr above silently dropped (e.g. wrong privilege, MSR write
    // refused) the read here would either fault or return a stale base.
    let via_gs = current();
    via_gs.self_check.store(apic_id, Ordering::SeqCst);
}

/// Read the running CPU's PerCpu via its GS base. Cheap: one mov.
#[inline]
pub fn current() -> &'static PerCpu {
    let ptr: *const PerCpu;
    unsafe {
        core::arch::asm!(
            "mov {}, gs:[0]",
            out(reg) ptr,
            options(nomem, nostack, preserves_flags),
        );
        &*ptr
    }
}

/// Snapshot of a specific CPU's slot, looked up by apic_id. Used by BSP
/// code that wants to inspect every CPU (smoke checks, /proc, ...). Safe
/// only for read access on already-initialised slots.
pub fn peek(apic_id: u32) -> Option<&'static PerCpu> {
    let idx = slot_index_for(apic_id);
    let slot_ptr = AREAS[idx].0.get();
    // SAFETY: the array entry is alive for the kernel lifetime; we only
    // hand out a shared reference, callers must use atomics.
    Some(unsafe { &*slot_ptr })
}

/// Slot index for a given apic_id. We index by apic_id directly because
/// QEMU + Intel hand out small contiguous IDs (0..N-1). If a future host
/// hands out sparse ids this will need a real mapping table.
#[inline]
fn slot_index_for(apic_id: u32) -> usize {
    debug_assert!((apic_id as usize) < MAX_CPUS, "apic_id {} >= MAX_CPUS", apic_id);
    (apic_id as usize) & (MAX_CPUS - 1)
}
