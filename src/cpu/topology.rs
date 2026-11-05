pub struct CpuTopology {
    pub cpu_count: u32,
    pub bsp_apic_id: u32,
    pub apic_ids: &'static [u32],
}

pub static TOPOLOGY: spin::Once<CpuTopology> = spin::Once::new();

impl CpuTopology {
    pub fn detect() -> Self {
        static APIC_IDS: [u32; 1] = [0];

        let cpuid = core::arch::x86_64::__cpuid(1);
        let cpu_count = ((cpuid.ebx >> 16) & 0xff).max(1) as u32;
        let bsp_apic_id = (cpuid.ebx >> 24) as u32;

        Self {
            cpu_count,
            bsp_apic_id,
            apic_ids: &APIC_IDS,
        }
    }
}

pub fn init() {
    TOPOLOGY.call_once(CpuTopology::detect);
}
