use core::sync::atomic::{AtomicPtr, AtomicU32};
use x86_64::{VirtAddr, registers::model_specific::GsBase};

#[repr(C, align(64))]
pub struct PerCpu {
    pub self_ptr:      *const PerCpu,
    pub cpu_id:        u32,
    pub apic_id:      u32,
    pub current_task:  AtomicPtr<crate::process::task::Task>,
    pub idle_stack:    *mut u8,
    pub preempt_count: AtomicU32,
    _pad: [u8; 24],
}

pub fn init_bsp(pc: &'static PerCpu) {
    let gs = pc as *const PerCpu as u64;
    GsBase::write(VirtAddr::new(gs));
}

#[inline(always)]
pub fn this_cpu() -> &'static PerCpu {
    let p: *const PerCpu;
    unsafe { core::arch::asm!("mov {}, gs:[0]", out(reg) p) };
    unsafe { &*p }
}