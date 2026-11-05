#[derive(Clone, Copy, Debug)]
pub struct CpuFeatures {
    pub initial_apic_id: u32,
    pub local_apic: bool,
    pub x2apic: bool,
    pub invariant_tsc: bool,
    pub logical_processor_count: u32,
}

impl CpuFeatures {
    pub fn detect() -> Self {
        let basic = core::arch::x86_64::__cpuid(1);
        let maximum_extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
        let x2apic = basic.ecx & (1 << 21) != 0;
        let invariant_tsc = maximum_extended >= 0x8000_0007
            && core::arch::x86_64::__cpuid(0x8000_0007).edx & (1 << 8) != 0;

        Self {
            initial_apic_id: basic.ebx >> 24,
            local_apic: basic.edx & (1 << 9) != 0,
            x2apic,
            invariant_tsc,
            logical_processor_count: ((basic.ebx >> 16) & 0xff).max(1),
        }
    }
}
