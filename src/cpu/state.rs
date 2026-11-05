use core::sync::atomic::{AtomicU8, Ordering};

use super::topology::MAX_CPUS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CpuState {
    Offline = 0,
    Starting = 1,
    Online = 2,
    Halted = 3,
}

pub struct CpuStateSlot {
    state: AtomicU8,
}

impl CpuStateSlot {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(CpuState::Offline as u8),
        }
    }

    pub fn get(&self) -> CpuState {
        match self.state.load(Ordering::Acquire) {
            1 => CpuState::Starting,
            2 => CpuState::Online,
            3 => CpuState::Halted,
            _ => CpuState::Offline,
        }
    }

    pub fn transition(&self, from: CpuState, to: CpuState) -> Result<(), CpuState> {
        self.state
            .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|actual| match actual {
                1 => CpuState::Starting,
                2 => CpuState::Online,
                3 => CpuState::Halted,
                _ => CpuState::Offline,
            })
    }
}

pub static CPU_STATES: [CpuStateSlot; MAX_CPUS] = [const { CpuStateSlot::new() }; MAX_CPUS];

pub fn init_bsp(bsp_index: usize) {
    let slot = CPU_STATES
        .get(bsp_index)
        .unwrap_or_else(|| panic!("BSP logical CPU id is out of range"));
    slot.transition(CpuState::Offline, CpuState::Online)
        .unwrap_or_else(|state| panic!("invalid BSP CPU state: {state:?}"));
}
