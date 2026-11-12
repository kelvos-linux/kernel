use crate::arch::x86_64::apic;

use super::{local, state, topology::TOPOLOGY};

pub fn init() {
    let topology = TOPOLOGY
        .get()
        .unwrap_or_else(|| panic!("CPU topology not initialized"));
    let apic_id = if super::detection::CpuFeatures::detect().local_apic {
        match apic::init(topology.local_apic_address) {
            Ok(apic_id) => apic_id,
            Err(error) => {
                crate::serial_println!(
                    "[WARN] local APIC initialization failed ({error}); continuing on BSP only"
                );
                topology.bsp_apic_id
            }
        }
    } else {
        topology.bsp_apic_id
    };
    if apic::is_enabled() && apic_id != topology.bsp_apic_id {
        panic!("CPUID and local APIC BSP IDs do not match");
    }

    local::prepare(topology.bsp_index, apic_id);
    state::init_bsp(topology.bsp_index);
}
