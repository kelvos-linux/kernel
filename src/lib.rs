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
use kelvos_shared_ring::MAX_TASKS;
use x86_64::instructions::hlt;

// Export absolute link-time symbols and assemble the bootstrap with the same
// constants the Rust paging and frame allocator code uses.
core::arch::global_asm!(
    ".global KERNEL_BASE, PAGE_SIZE, LAPIC_DEFAULT",
    ".equ KERNEL_BASE, {kernel_base}",
    ".equ PAGE_SIZE, {page_size}",
    ".equ LAPIC_DEFAULT, {lapic_default}",
    ".equ BOOT_DIRECTORY_COUNT, {directories}",
    ".equ BOOT_LARGE_PAGE_COUNT, {large_pages}",
    include_str!("boot.S"),
    kernel_base = const kelvos_shared_ring::layout::KERNEL_BASE,
    page_size = const kelvos_shared_ring::layout::PAGE_SIZE,
    lapic_default = const kelvos_shared_ring::layout::LAPIC_DEFAULT,
    directories = const kelvos_shared_ring::layout::BOOT_DIRECTORY_COUNT,
    large_pages = const kelvos_shared_ring::layout::BOOT_LARGE_PAGE_COUNT,
    options(att_syntax)
);

const MULTIBOOT2_MAGIC: u32 = 0x36D76289;

unsafe extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

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
    let boot_memory = memory::init(boot_info_address).expect("physical memory map parsing failed");

    serial_println!("[..] paging initialization");
    let mut paging = unsafe { mm::paging::PagingManager::new() };
    let kernel_start = core::ptr::addr_of!(__kernel_start) as u64;
    let kernel_end = core::ptr::addr_of!(__kernel_end) as u64;
    let reserved_ranges = [
        (kernel_start, kernel_end.saturating_sub(kernel_start)),
        (boot_memory.boot_info_start, boot_memory.boot_info_len),
        (boot_memory.user_module_start, boot_memory.user_module_len),
        (boot_memory.child_module_start, boot_memory.child_module_len),
    ];
    let mut frame_allocator = mm::frame::FrameAllocator::new(boot_memory.regions, &reserved_ranges)
        .expect("physical frame allocator initialization failed");
    serial_println!(
        "[OK] physical frame allocator manages {} frames ({} free)",
        frame_allocator.total_frames(),
        frame_allocator.free_frames()
    );
    let free_frames_before_probe = frame_allocator.free_frames();
    let probe_frame = frame_allocator
        .alloc_frame()
        .expect("physical frame allocator validation failed");
    if probe_frame.start_address().as_u64() < kelvos_shared_ring::layout::LOW_RESERVED_END {
        panic!("physical frame allocator returned reserved low memory");
    }
    frame_allocator.free_frame(probe_frame);
    frame_allocator.free_frame(probe_frame);
    if frame_allocator.free_frames() != free_frames_before_probe {
        panic!("physical frame allocator reclamation validation failed");
    }
    serial_println!("[OK] physical frame allocation and reclamation validated");

    mm::heap::init(&mut paging, &mut frame_allocator);
    serial_println!("[OK] page tables initialized and the kernel heap is mapped in the identity-mapped boot layout");
    mm::heap::validate_allocator();
    serial_println!("[OK] kernel heap allocation, collection growth, and deallocation validated");

    let image_bytes = unsafe {
        core::slice::from_raw_parts(
            boot_memory.user_module_start as *const u8,
            boot_memory.user_module_len as usize,
        )
    };
    let image = process::image::UserImage::parse(image_bytes).expect("invalid user_init ELF");
    let child_bytes = unsafe {
        core::slice::from_raw_parts(
            boot_memory.child_module_start as *const u8,
            boot_memory.child_module_len as usize,
        )
    };
    let child = process::image::UserImage::parse(child_bytes).expect("invalid user_child ELF");
    let user_images: [_; MAX_TASKS] = core::array::from_fn(|index| {
        let image = if index == 0 { &image } else { &child };
        unsafe { image.instantiate(&mut paging, &mut frame_allocator) }
            .expect("user image mapping failed")
    });
    serial_println!("[OK] init ELF and {} child ELF slots loaded", MAX_TASKS - 1);

    serial_println!("[..] CPU topology detection");
    cpu::topology::init(boot_info_address);
    let topo = cpu::topology::TOPOLOGY.get().unwrap();
    serial_println!(
        "[OK] detected {} logical CPU(s){}; BSP APIC id = {}",
        topo.cpu_count,
        if topo.from_acpi {
            " from ACPI MADT"
        } else {
            " (BSP fallback)"
        },
        topo.bsp_apic_id
    );
    if topo.unsupported_cpu_count != 0 {
        serial_println!(
            "[WARN] {} processor(s) exceed xAPIC/MAX_CPUS support and will not be started",
            topo.unsupported_cpu_count
        );
    }

    serial_println!("[..] local APIC and BSP CPU initialization");
    cpu::init::init();
    serial_println!("[OK] BSP local APIC and per-CPU storage initialized");

    serial_println!("[..] scheduler and SMP bootstrap");
    process::scheduler::init(user_images);
    cpu::smp::init();
    serial_println!("[OK] SMP CPU bootstrap initialized");

    serial_println!("[..] scheduling init as PID 1");
    x86_64::instructions::interrupts::enable();
    halt_loop()
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
