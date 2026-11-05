use crate::arch::x86_64::percpu::{self, PerCpu};

const IDLE_STACK_SIZE: usize = 16 * 1024;

#[repr(align(16))]
struct IdleStack([u8; IDLE_STACK_SIZE]);

static mut IDLE_STACKS: [IdleStack; super::topology::MAX_CPUS] =
    [const { IdleStack([0; IDLE_STACK_SIZE]) }; super::topology::MAX_CPUS];

pub fn init(cpu_id: usize, apic_id: u32) -> &'static PerCpu {
    let pc = prepare(cpu_id, apic_id);
    percpu::activate(pc);
    pc
}

pub fn prepare(cpu_id: usize, apic_id: u32) -> &'static PerCpu {
    let pc = unsafe { percpu::prepare(cpu_id, apic_id) };
    let stack = unsafe {
        core::ptr::addr_of_mut!(IDLE_STACKS)
            .cast::<IdleStack>()
            .add(cpu_id)
    };
    let top = unsafe { (*stack).0.as_mut_ptr().add(IDLE_STACK_SIZE) };
    pc.idle_stack
        .store(top, core::sync::atomic::Ordering::Release);
    pc
}
