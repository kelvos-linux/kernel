#![no_std]
#![feature(abi_x86_interrupt, alloc_error_handler)]
#![allow(dead_code)]

extern crate alloc;

mod arch;
mod cpu;
mod memory;
mod mm;
mod process;
mod security;
mod serial;
mod storage;

use core::panic::PanicInfo;
use x86_64::instructions::{hlt, interrupts};

const MULTIBOOT2_MAGIC: u32 = 0x36D76289;

#[alloc_error_handler]
fn oom(layout: core::alloc::Layout) -> ! {
    panic!("heap OOM: {layout:?}")
}

/// Kernel entry called from `boot.S` with the Multiboot2 magic in `edi`
/// and the physical address of the boot information structure in `esi`.
#[unsafe(no_mangle)]
pub extern "C" fn rust_entry(magic: u32, boot_info_address: u32) -> ! {
    serial::init();

    serial_println!("");
    serial_println!("======================================");
    serial_println!("          KELV-OS KERNEL v0.1");
    serial_println!("======================================");

    if magic != MULTIBOOT2_MAGIC {
        serial_println!("[ERROR] invalid Multiboot2 magic: {:#010x}", magic);
        halt_loop();
    }
    serial_println!("[OK] multiboot2 magic verified");
    serial_println!("[OK] boot info @ {:#010x}", boot_info_address);

    serial_println!("[..] GDT/TSS initialization");
    arch::x86_64::gdt::init();
    serial_println!("[OK] GDT loaded, TSS installed (IST for double faults)");

    serial_println!("[..] IDT + PIC initialization");
    arch::x86_64::idt::init();
    serial_println!("[OK] IDT loaded, PIC remapped to 32/40");

    serial_println!("[..] physical memory map detection");
    memory::init(boot_info_address);

    serial_println!("[..] paging initialization");
    let mut paging = unsafe { mm::paging::PagingManager::new() };
    let mut frame_allocator = mm::frame::FrameAllocator::new_default();
    mm::heap::init(&mut paging, &mut frame_allocator);
    serial_println!("[OK] page tables initialized and the kernel heap is mapped in the identity-mapped boot layout");

    serial_println!("[..] user memory mapping bootstrap");
    let user_start = x86_64::VirtAddr::new(0x0000_0001_0000_0000);
    unsafe {
        paging
            .map_user_range(user_start, 4096 * 8, &mut frame_allocator)
            .expect("user memory mapping failed");
    }
    serial_println!("[OK] user memory range mapped and isolated with user-accessible page flags");

    serial_println!("[..] CPU topology detection");
    cpu::topology::init();
    let topo = cpu::topology::TOPOLOGY.get().unwrap();
    serial_println!(
        "[OK] detected {} logical CPU(s); BSP APIC id = {}",
        topo.cpu_count,
        topo.bsp_apic_id
    );

    serial_println!("[..] scheduler and SMP bootstrap");
    process::scheduler::init();
    cpu::smp::init();
    serial_println!("[OK] multi-core scheduler bootstrap initialized");

    serial_println!("[..] security and isolation bootstrap");
    security::init();
    serial_println!("[OK] process isolation, capability checks, syscall filtering, and IPC isolation initialized");

    serial_println!("[..] storage, filesystem, and service-domain bootstrap");
    storage::init();
    storage::fs::init();
    storage::async_io::init();
    storage::driver_service::init();
    serial_println!("[OK] NVMe, filesystem, async I/O, and driver service domains initialized");

    serial_println!("[..] hardening and experimental release validation");
    let release_ready = security::hardening::init();
    if release_ready {
        serial_println!("[OK] kernel readiness gate passed: audit, fuzz, verifier, compatibility, and scheduler baseline are all satisfied");
    } else {
        serial_println!("[WARN] kernel readiness gate not yet satisfied: one or more validation checks remain pending");
    }

    serial_println!("--------------------------------------");
    serial_println!("enabling interrupts...");
    interrupts::enable();

    serial_println!("KELV-OS v0.1 initialization complete.");
    serial_println!("Press keys to test IRQ1, watch the timer via IRQ0.");
    halt_loop();
}

fn halt_loop() -> ! {
    loop {
        hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[KERNEL PANIC] {}", info);
    halt_loop();
}
