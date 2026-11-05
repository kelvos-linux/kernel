use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64};
use x86_64::{registers::model_specific::GsBase, VirtAddr};

use crate::cpu::topology::MAX_CPUS;

#[repr(C, align(64))]
pub struct PerCpu {
    self_ptr: *const PerCpu,
    pub cpu_id: AtomicU32,
    pub apic_id: AtomicU32,
    pub current_task: AtomicPtr<crate::process::task::Task>,
    pub idle_stack: AtomicPtr<u8>,
    pub preempt_count: AtomicU32,
    pub reschedule_pending: AtomicBool,
    pub ticks: AtomicU64,
}

unsafe impl Sync for PerCpu {}

impl PerCpu {
    pub const fn new() -> Self {
        Self {
            self_ptr: core::ptr::null(),
            cpu_id: AtomicU32::new(u32::MAX),
            apic_id: AtomicU32::new(u32::MAX),
            current_task: AtomicPtr::new(core::ptr::null_mut()),
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

static mut PER_CPU: [PerCpu; MAX_CPUS] = [const { PerCpu::new() }; MAX_CPUS];

pub fn for_cpu(cpu_id: usize) -> Option<&'static PerCpu> {
    if cpu_id >= MAX_CPUS {
        return None;
    }
    Some(unsafe { &*core::ptr::addr_of!(PER_CPU).cast::<PerCpu>().add(cpu_id) })
}

pub unsafe fn prepare(cpu_id: usize, apic_id: u32) -> &'static PerCpu {
    let pc = for_cpu(cpu_id).unwrap_or_else(|| panic!("logical CPU id is out of range"));
    let mutable = pc as *const PerCpu as *mut PerCpu;
    unsafe {
        (*mutable).self_ptr = pc;
        pc.cpu_id
            .store(cpu_id as u32, core::sync::atomic::Ordering::Relaxed);
        pc.apic_id
            .store(apic_id, core::sync::atomic::Ordering::Relaxed);
    }
    pc
}

pub fn activate(pc: &'static PerCpu) {
    GsBase::write(VirtAddr::from_ptr(pc));
}

pub unsafe fn init(cpu_id: usize, apic_id: u32) -> &'static PerCpu {
    let pc = unsafe { prepare(cpu_id, apic_id) };
    activate(pc);
    pc
}

#[inline(always)]
pub fn try_this_cpu() -> Option<&'static PerCpu> {
    let base = GsBase::read().as_u64() as *const PerCpu;
    if base.is_null() {
        return None;
    }
    let pc = unsafe { &*base };
    if pc.self_ptr != base {
        return None;
    }
    Some(pc)
}

#[inline(always)]
pub fn this_cpu() -> &'static PerCpu {
    try_this_cpu().unwrap_or_else(|| panic!("per-CPU storage has not been initialized"))
}
