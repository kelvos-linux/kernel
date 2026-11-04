pub const APIC_TIMER_VECTOR: u8 = 0xF0;
pub const APIC_SPURIOUS_VECTOR: u8 = 0xF1;
pub const APIC_ERROR_VECTOR: u8 = 0xF2;
pub const IPI_RESCHEDULE: u8 = 0xF3;
pub const IPI_TLB_SHOOTDOWN: u8 = 0xF4;
pub const IPI_HALT: u8 = 0xF5;

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_PHYS: usize = 0xFEE0_0000;
const APIC_OFFSET_SPURIOUS: u32 = 0xF0;
const APIC_OFFSET_EOI: u32 = 0x0B0;
const APIC_OFFSET_LVT_TIMER: u32 = 0x320;
const APIC_OFFSET_TIMER_DIVIDE: u32 = 0x3E0;
const APIC_OFFSET_TIMER_INITIAL: u32 = 0x380;

fn apic_read(offset: u32) -> u32 {
    unsafe {
        let ptr = (APIC_BASE_PHYS + offset as usize) as *const u32;
        core::ptr::read_volatile(ptr)
    }
}

fn apic_write(offset: u32, value: u32) {
    unsafe {
        let ptr = (APIC_BASE_PHYS + offset as usize) as *mut u32;
        core::ptr::write_volatile(ptr, value);
    }
}

#[derive(Debug)]
pub struct LocalApicTimer {
    ticks: core::sync::atomic::AtomicU64,
    vector: u8,
}

impl LocalApicTimer {
    pub const fn new(vector: u8) -> Self {
        Self {
            ticks: core::sync::atomic::AtomicU64::new(0),
            vector,
        }
    }

    pub fn init(&self) {
        let mut apic_base_msr = x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR);
        unsafe {
            let base = apic_base_msr.read();
            apic_base_msr.write(base | (1 << 11));
        }

        apic_write(APIC_OFFSET_SPURIOUS, (self.vector as u32) | (1 << 8));
        apic_write(APIC_OFFSET_LVT_TIMER, (self.vector as u32) | (1 << 17));
        apic_write(APIC_OFFSET_TIMER_DIVIDE, 0x03);
        self.set_timer_period(10000);
    }

    pub fn set_timer_period(&self, initial_count: u32) {
        apic_write(APIC_OFFSET_TIMER_INITIAL, initial_count);
    }

    pub fn current_count(&self) -> u32 {
        apic_read(APIC_OFFSET_TIMER_INITIAL)
    }

    pub fn tick(&self) {
        self.ticks
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn end_of_interrupt(&self) {
        apic_write(APIC_OFFSET_EOI, 0);
    }

    pub fn elapsed(&self) -> u64 {
        self.ticks.load(core::sync::atomic::Ordering::Relaxed)
    }
}

pub static LOCAL_APIC_TIMER: LocalApicTimer = LocalApicTimer::new(APIC_TIMER_VECTOR);
