use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::cpu::topology::TOPOLOGY;

#[derive(Debug)]
pub struct CpuLocalState {
    pub cpu_id: u32,
    pub current_task_id: AtomicU32,
    pub ticks: AtomicU64,
}

impl CpuLocalState {
    pub const fn new(cpu_id: u32) -> Self {
        Self {
            cpu_id,
            current_task_id: AtomicU32::new(0),
            ticks: AtomicU64::new(0),
        }
    }

    pub fn set_current_task(&self, task_id: u32) {
        self.current_task_id.store(task_id, Ordering::Relaxed);
    }

    pub fn tick(&self) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }
}

pub static BSP_CPU_STATE: spin::Lazy<CpuLocalState> = spin::Lazy::new(|| CpuLocalState::new(0));

pub fn init() {
    let topo = TOPOLOGY
        .get()
        .unwrap_or_else(|| panic!("CPU topology not initialized"));
    let bsp = &*BSP_CPU_STATE;
    bsp.set_current_task(0);
    bsp.tick();

    if topo.cpu_count > 1 {
        // The SMP bootstrap is intentionally minimal at this stage; the BSP owns the initial
        // per-CPU state while the legacy timer drives scheduler ticks.
    }
}

pub fn on_tick() {
    BSP_CPU_STATE.tick();
}
