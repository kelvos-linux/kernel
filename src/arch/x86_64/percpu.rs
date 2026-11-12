use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64};

use crate::cpu::topology::{MAX_CPUS, TOPOLOGY};

#[repr(C, align(64))]
pub struct PerCpu {
    pub cpu_id: AtomicU32,
    pub apic_id: AtomicU32,
    pub idle_stack: AtomicPtr<u8>,
    pub preempt_count: AtomicU32,
    pub reschedule_pending: AtomicBool,
    pub ticks: AtomicU64,
}

impl PerCpu {
    pub const fn new() -> Self {
        Self {
            cpu_id: AtomicU32::new(u32::MAX),
            apic_id: AtomicU32::new(u32::MAX),
            idle_stack: AtomicPtr::new(core::ptr::null_mut()),
            preempt_count: AtomicU32::new(0),
            reschedule_pending: AtomicBool::new(false),
            ticks: AtomicU64::new(0),
        }
    }

    pub fn id(&self) -> u32 {
        self.cpu_id.load(core::sync::atomic::Ordering::Relaxed)
    }
}

static PER_CPU: [PerCpu; MAX_CPUS] = [const { PerCpu::new() }; MAX_CPUS];

pub fn for_cpu(cpu_id: usize) -> Option<&'static PerCpu> {
    PER_CPU.get(cpu_id)
}

pub fn prepare(cpu_id: usize, apic_id: u32) -> &'static PerCpu {
    let pc = for_cpu(cpu_id).expect("logical CPU id is out of range");
    pc.cpu_id
        .store(cpu_id as u32, core::sync::atomic::Ordering::Relaxed);
    pc.apic_id
        .store(apic_id, core::sync::atomic::Ordering::Relaxed);
    pc
}

pub fn try_this_cpu() -> Option<&'static PerCpu> {
    // CPUID's initial xAPIC ID is independent of user-controlled segment state.
    let apic_id = core::arch::x86_64::__cpuid(1).ebx >> 24;
    let id = TOPOLOGY.get()?.logical_id(apic_id)?;
    for_cpu(id)
}

#[inline(always)]
pub fn this_cpu() -> &'static PerCpu {
    try_this_cpu().unwrap_or_else(|| panic!("per-CPU storage has not been initialized"))
}
