use core::sync::atomic::Ordering;

use crate::{
    arch::x86_64::{apic, gdt, idt, percpu},
    serial_println,
};

use super::{
    local,
    state::{CpuState, CPU_STATES},
    topology::{CpuTopology, TOPOLOGY},
    trampoline,
};

fn delay(iterations: usize) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}

fn start_application_processors(topology: &CpuTopology) {
    if topology.cpu_count <= 1 {
        return;
    }

    for (cpu_id, &apic_id) in topology.apic_ids().iter().enumerate().skip(1) {
        let state = &CPU_STATES[cpu_id];
        if state
            .transition(CpuState::Offline, CpuState::Starting)
            .is_err()
        {
            serial_println!("[SMP] CPU {} is not in the offline state", cpu_id);
            continue;
        }

        let pc = local::prepare(cpu_id, apic_id);
        if let Err(error) = trampoline::install(cpu_id, apic_id, pc) {
            let _ = state.transition(CpuState::Starting, CpuState::Offline);
            serial_println!("[SMP] CPU {} startup preparation failed: {}", cpu_id, error);
            continue;
        }

        let vector = trampoline::startup_vector();
        if let Err(error) = apic::send_init(apic_id) {
            let _ = state.transition(CpuState::Starting, CpuState::Offline);
            serial_println!("[SMP] INIT for CPU {} failed: {}", cpu_id, error);
            continue;
        }
        delay(10_000_000);
        if let Err(error) = apic::send_init_deassert(apic_id) {
            let _ = state.transition(CpuState::Starting, CpuState::Offline);
            serial_println!("[SMP] INIT deassert for CPU {} failed: {}", cpu_id, error);
            continue;
        }
        delay(1_000_000);
        if let Err(error) = apic::send_startup(apic_id, vector) {
            serial_println!("[SMP] first SIPI for CPU {} failed: {}", cpu_id, error);
            break;
        }
        delay(200_000);

        if state.get() == CpuState::Starting {
            if let Err(error) = apic::send_startup(apic_id, vector) {
                serial_println!("[SMP] second SIPI for CPU {} failed: {}", cpu_id, error);
                break;
            }
        }

        let mut online = false;
        for _ in 0..10_000_000 {
            if state.get() == CpuState::Online {
                online = true;
                break;
            }
            core::hint::spin_loop();
        }
        if online {
            serial_println!("[SMP] CPU {} (APIC {}) online", cpu_id, apic_id);
        } else {
            serial_println!(
                "[SMP] CPU {} (APIC {}) did not report online",
                cpu_id,
                apic_id
            );
            break;
        }
    }
}

pub fn init() {
    let topology = TOPOLOGY
        .get()
        .unwrap_or_else(|| panic!("CPU topology not initialized"));
    if !apic::is_enabled() {
        serial_println!("[SMP] local APIC unavailable; keeping BSP online only");
        return;
    }
    start_application_processors(topology);
}

pub fn ap_entry(cpu_id: usize) -> ! {
    let Some(topology) = TOPOLOGY.get() else {
        panic!("AP started before CPU topology initialization");
    };
    let Some(&expected_apic_id) = topology.apic_ids().get(cpu_id) else {
        panic!("AP logical CPU id is out of range");
    };
    let Some(pc) = percpu::for_cpu(cpu_id) else {
        panic!("AP has no per-CPU storage");
    };

    let stack_top = pc.idle_stack.load(Ordering::Acquire) as usize;
    if stack_top == 0 {
        panic!("AP has no idle stack");
    }
    gdt::init_ap(cpu_id, stack_top);
    if apic::init(topology.local_apic_address).is_err() || apic::local_apic_id() != expected_apic_id
    {
        let _ = CPU_STATES[cpu_id].transition(CpuState::Starting, CpuState::Offline);
        ap_halt();
    }
    idt::init_ap();

    if CPU_STATES[cpu_id]
        .transition(CpuState::Starting, CpuState::Online)
        .is_err()
    {
        ap_halt();
    }
    apic::configure_timer(apic::APIC_TIMER_VECTOR, 1_000_000);
    x86_64::instructions::interrupts::enable();
    loop {
        x86_64::instructions::hlt();
    }
}

fn ap_halt() -> ! {
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}

pub fn on_tick() {
    if let Some(pc) = percpu::try_this_cpu() {
        pc.ticks.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn on_reschedule_ipi() {
    if let Some(pc) = percpu::try_this_cpu() {
        pc.reschedule_pending.store(true, Ordering::Release);
    }
}

pub fn send_reschedule(cpu_id: usize) -> Result<(), &'static str> {
    let topology = TOPOLOGY.get().ok_or("CPU topology is not initialized")?;
    if cpu_id >= topology.cpu_count {
        return Err("logical CPU id is out of range");
    }
    if CPU_STATES[cpu_id].get() != CpuState::Online {
        return Err("target CPU is not online");
    }
    apic::send_reschedule(topology.apic_ids()[cpu_id])
}

pub fn reschedule_other_cpus() {
    let current_cpu = percpu::this_cpu().id() as usize;
    let Some(topology) = TOPOLOGY.get() else {
        return;
    };
    for cpu_id in 0..topology.cpu_count {
        if cpu_id == current_cpu || CPU_STATES[cpu_id].get() != CpuState::Online {
            continue;
        }
        if let Err(error) = send_reschedule(cpu_id) {
            serial_println!("[SMP] reschedule IPI to CPU {} failed: {}", cpu_id, error);
        }
    }
}
