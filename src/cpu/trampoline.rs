use core::arch::global_asm;

use crate::arch::x86_64::percpu::PerCpu;

global_asm!(include_str!("trampoline.S"), options(att_syntax));

const TRAMPOLINE_ADDRESS: usize = 0x8000;
const STARTUP_VECTOR: u8 = (TRAMPOLINE_ADDRESS >> 12) as u8;

#[repr(C)]
struct StartupData {
    cr3: u32,
    cpu_id: u32,
    apic_id: u32,
    _reserved: u32,
    stack_top: u64,
    entry: u64,
}

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_end: u8;
    static ap_trampoline_data: StartupData;
    static ap_trampoline_gdtr: u8;
    static ap_trampoline_gdt: u8;
}

pub fn install(cpu_id: usize, apic_id: u32, pc: &'static PerCpu) -> Result<(), &'static str> {
    let start = core::ptr::addr_of!(ap_trampoline_start);
    let end = core::ptr::addr_of!(ap_trampoline_end);
    let length = end as usize - start as usize;
    if length > 0x0f00 {
        return Err("AP startup trampoline leaves no room for its real-mode stack");
    }

    let boot_data = unsafe { core::ptr::read_volatile(0x40e as *const u16) };
    let ebda = usize::from(boot_data) << 4;
    if ebda != 0 && TRAMPOLINE_ADDRESS + 4096 >= ebda {
        return Err("AP startup trampoline would overlap the EBDA");
    }

    let destination = TRAMPOLINE_ADDRESS as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(start, destination, length);
    }

    let source_data = core::ptr::addr_of!(ap_trampoline_data) as usize;
    let data_offset = source_data - start as usize;
    let data = unsafe { &mut *(destination.add(data_offset).cast::<StartupData>()) };
    let (cr3, _) = x86_64::registers::control::Cr3::read();
    data.cr3 = cr3.start_address().as_u64() as u32;
    data.cpu_id = cpu_id as u32;
    data.apic_id = apic_id;
    data.stack_top = pc.idle_stack.load(core::sync::atomic::Ordering::Acquire) as u64;
    data.entry = ap_entry as *const () as usize as u64;

    let gdtr_offset = core::ptr::addr_of!(ap_trampoline_gdtr) as usize - start as usize;
    let gdt_offset = core::ptr::addr_of!(ap_trampoline_gdt) as usize - start as usize;
    unsafe {
        core::ptr::write_unaligned(
            destination.add(gdtr_offset + 2).cast::<u32>(),
            (TRAMPOLINE_ADDRESS + gdt_offset) as u32,
        );
    }
    Ok(())
}

pub fn startup_vector() -> u8 {
    STARTUP_VECTOR
}

extern "C" fn ap_entry(cpu_id: u32) -> ! {
    crate::cpu::smp::ap_entry(cpu_id as usize)
}
