use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use x86_64::registers::model_specific::Msr;

pub const APIC_TIMER_VECTOR: u8 = 0xF0;
pub const APIC_SPURIOUS_VECTOR: u8 = 0xF1;
pub const APIC_ERROR_VECTOR: u8 = 0xF2;
pub const IPI_RESCHEDULE: u8 = 0xF3;
pub const IPI_TLB_SHOOTDOWN: u8 = 0xF4;
pub const IPI_HALT: u8 = 0xF5;

const IA32_APIC_BASE_MSR: u32 = 0x1B;
const APIC_BASE_MASK: u64 = 0x000f_ffff_ffff_f000;
const APIC_OFFSET_ID: u32 = 0x020;
const APIC_OFFSET_EOI: u32 = 0x0B0;
const APIC_OFFSET_ERROR_STATUS: u32 = 0x280;
const APIC_OFFSET_SPURIOUS: u32 = 0x0F0;
const APIC_OFFSET_ICR_LOW: u32 = 0x300;
const APIC_OFFSET_ICR_HIGH: u32 = 0x310;
const APIC_OFFSET_LVT_TIMER: u32 = 0x320;
const APIC_OFFSET_LVT_ERROR: u32 = 0x370;
const APIC_OFFSET_TIMER_DIVIDE: u32 = 0x3E0;
const APIC_OFFSET_TIMER_INITIAL: u32 = 0x380;
const APIC_OFFSET_TIMER_CURRENT: u32 = 0x390;
const APIC_SVR_ENABLE: u32 = 1 << 8;
const ICR_DELIVERY_PENDING: u32 = 1 << 12;

static APIC_BASE: AtomicU64 = AtomicU64::new(kelvos_shared_ring::layout::LAPIC_DEFAULT);
static APIC_READY: AtomicBool = AtomicBool::new(false);

fn apic_read(offset: u32) -> u32 {
    let address = APIC_BASE.load(Ordering::Acquire) as usize + offset as usize;
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

fn apic_write(offset: u32, value: u32) {
    let address = APIC_BASE.load(Ordering::Acquire) as usize + offset as usize;
    unsafe { core::ptr::write_volatile(address as *mut u32, value) };
}

pub fn init(local_apic_address: u64) -> Result<u32, &'static str> {
    if local_apic_address == 0
        || local_apic_address & 0xfff != 0
        || local_apic_address >= kelvos_shared_ring::layout::BOOT_IDENTITY_END
    {
        return Err("invalid local APIC physical address");
    }

    let mut apic_base_msr = Msr::new(IA32_APIC_BASE_MSR);
    let base = unsafe { apic_base_msr.read() };
    if base & (1 << 10) != 0 {
        return Err("x2APIC mode is active but this kernel only supports xAPIC");
    }
    let base = (base & !APIC_BASE_MASK) | (local_apic_address & APIC_BASE_MASK) | (1 << 11);
    unsafe { apic_base_msr.write(base) };
    APIC_BASE.store(local_apic_address, Ordering::Release);

    apic_write(
        APIC_OFFSET_SPURIOUS,
        APIC_SPURIOUS_VECTOR as u32 | APIC_SVR_ENABLE,
    );
    apic_write(APIC_OFFSET_LVT_ERROR, APIC_ERROR_VECTOR as u32);
    APIC_READY.store(true, Ordering::Release);
    Ok(apic_read(APIC_OFFSET_ID) >> 24)
}

pub fn is_enabled() -> bool {
    APIC_READY.load(Ordering::Acquire)
}

pub fn local_apic_id() -> u32 {
    apic_read(APIC_OFFSET_ID) >> 24
}

fn wait_for_delivery() -> Result<(), &'static str> {
    for _ in 0..1_000_000 {
        if apic_read(APIC_OFFSET_ICR_LOW) & ICR_DELIVERY_PENDING == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err("local APIC IPI delivery timed out")
}

fn send_ipi(apic_id: u32, command: u32) -> Result<(), &'static str> {
    if !is_enabled() {
        return Err("local APIC is not initialized");
    }
    if apic_id > u8::MAX.into() {
        return Err("xAPIC cannot address APIC IDs above 255");
    }
    wait_for_delivery()?;
    apic_write(APIC_OFFSET_ICR_HIGH, apic_id << 24);
    apic_write(APIC_OFFSET_ICR_LOW, command);
    wait_for_delivery()
}

pub fn send_reschedule(apic_id: u32) -> Result<(), &'static str> {
    send_ipi(apic_id, u32::from(IPI_RESCHEDULE))
}

pub fn send_tlb_shootdown(apic_id: u32) -> Result<(), &'static str> {
    send_ipi(apic_id, u32::from(IPI_TLB_SHOOTDOWN))
}

pub fn send_halt(apic_id: u32) -> Result<(), &'static str> {
    send_ipi(apic_id, u32::from(IPI_HALT))
}

pub fn send_init(apic_id: u32) -> Result<(), &'static str> {
    send_ipi(apic_id, 0x0000_4500)
}

pub fn send_init_deassert(apic_id: u32) -> Result<(), &'static str> {
    send_ipi(apic_id, 0x0000_8500)
}

pub fn send_startup(apic_id: u32, vector: u8) -> Result<(), &'static str> {
    send_ipi(apic_id, 0x0000_0600 | u32::from(vector))
}

pub fn end_of_interrupt() {
    apic_write(APIC_OFFSET_EOI, 0);
}

pub fn clear_error_status() {
    apic_write(APIC_OFFSET_ERROR_STATUS, 0);
    let _ = apic_read(APIC_OFFSET_ERROR_STATUS);
}

pub fn configure_timer(vector: u8, initial_count: u32) {
    apic_write(APIC_OFFSET_TIMER_DIVIDE, 0x03);
    apic_write(APIC_OFFSET_LVT_TIMER, u32::from(vector) | (1 << 17));
    apic_write(APIC_OFFSET_TIMER_INITIAL, initial_count);
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
        configure_timer(self.vector, 10_000);
    }

    pub fn set_timer_period(&self, initial_count: u32) {
        apic_write(APIC_OFFSET_TIMER_INITIAL, initial_count);
    }

    pub fn current_count(&self) -> u32 {
        apic_read(APIC_OFFSET_TIMER_CURRENT)
    }

    pub fn tick(&self) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn end_of_interrupt(&self) {
        end_of_interrupt();
    }

    pub fn elapsed(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
}

pub static LOCAL_APIC_TIMER: LocalApicTimer = LocalApicTimer::new(APIC_TIMER_VECTOR);
