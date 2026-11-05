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
use kelvos_shared_ring::{
    SharedRing, SPAWN_STACK_TOP, USER_IMAGE_ADDRESS, USER_IMAGE_CAPACITY, USER_RING_ADDRESS,
    USER_STACK_TOP,
};
use x86_64::instructions::hlt;
use x86_64::structures::paging::{Page, PageTableFlags};
use x86_64::VirtAddr;

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
    if probe_frame.start_address().as_u64() < 0x10_0000 {
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

    serial_println!("[..] loading GRUB user_init module and shared ring");
    let user_image = unsafe {
        core::slice::from_raw_parts(
            boot_memory.user_module_start as *const u8,
            boot_memory.user_module_len as usize,
        )
    };
    load_user_image(&mut paging, &mut frame_allocator, user_image);
    serial_println!("[OK] user image, shared ring, and two user stacks mapped");

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
    let user_mode_frame = process::scheduler::SCHEDULER
        .lock()
        .bootstrap_user_task(USER_STACK_TOP as usize, USER_IMAGE_ADDRESS as usize)
        .expect("initial user task creation failed");
    serial_println!(
        "[..] entering ring 3: user_init will submit SPAWN (interrupts enabled by iretq)"
    );
    unsafe { arch::x86_64::gdt::enter_user_mode(user_mode_frame) }
}

fn load_user_image(
    paging: &mut mm::paging::PagingManager,
    frame_allocator: &mut mm::frame::FrameAllocator,
    image: &[u8],
) {
    const PAGE_SIZE: usize = 4096;
    const STACK_SIZE: usize = PAGE_SIZE * 2;

    if image.is_empty() || image.len() > USER_IMAGE_CAPACITY {
        panic!("embedded user image exceeds the supported image range");
    }

    let code_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    let page_count = image.len().div_ceil(PAGE_SIZE);
    for page_index in 0..page_count {
        let frame = frame_allocator
            .alloc_frame()
            .expect("user image frame allocation failed");
        let destination = VirtAddr::new(frame.start_address().as_u64()).as_mut_ptr::<u8>();
        let source_offset = page_index * PAGE_SIZE;
        let copy_length = (image.len() - source_offset).min(PAGE_SIZE);
        unsafe {
            core::ptr::write_bytes(destination, 0, PAGE_SIZE);
            core::ptr::copy_nonoverlapping(
                image.as_ptr().add(source_offset),
                destination,
                copy_length,
            );
            paging
                .map_page(
                    Page::containing_address(VirtAddr::new(
                        USER_IMAGE_ADDRESS + source_offset as u64,
                    )),
                    frame,
                    code_flags,
                    frame_allocator,
                )
                .expect("user image page mapping failed");
        }
    }

    unsafe {
        paging
            .map_user_range(VirtAddr::new(USER_RING_ADDRESS), PAGE_SIZE, frame_allocator)
            .expect("shared ring mapping failed");
        paging
            .map_user_range(
                VirtAddr::new(USER_STACK_TOP - STACK_SIZE as u64),
                STACK_SIZE,
                frame_allocator,
            )
            .expect("initial user stack mapping failed");
        paging
            .map_user_range(
                VirtAddr::new(SPAWN_STACK_TOP - STACK_SIZE as u64),
                STACK_SIZE,
                frame_allocator,
            )
            .expect("spawned user stack mapping failed");
    }

    unsafe {
        core::ptr::write(USER_RING_ADDRESS as *mut SharedRing, SharedRing::new());
        core::ptr::write_bytes(
            (USER_STACK_TOP - STACK_SIZE as u64) as *mut u8,
            0,
            STACK_SIZE,
        );
        core::ptr::write_bytes(
            (SPAWN_STACK_TOP - STACK_SIZE as u64) as *mut u8,
            0,
            STACK_SIZE,
        );
    }
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
