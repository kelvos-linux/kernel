use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

use crate::{
    arch::x86_64::{apic, gdt},
    serial_println,
};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = 40;

static TICKS: AtomicU64 = AtomicU64::new(0);

fn pic_eoi(irq: u8) {
    if irq < 8 {
        unsafe {
            Port::<u8>::new(0x20).write(0x20);
        }
        return;
    }

    unsafe {
        Port::<u8>::new(0xA0).write(0x20);
        Port::<u8>::new(0x20).write(0x20);
    }
}

extern "x86-interrupt" fn breakpoint_handler(_stack_frame: InterruptStackFrame) {
    serial_println!("[EXC] #BP breakpoint exception");
}

extern "x86-interrupt" fn double_fault_handler(
    _stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    serial_println!(
        "[PANIC] double fault (error code {:#x}) — halting",
        error_code
    );
    loop {
        x86_64::instructions::hlt();
    }
}

extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    handle_timer_tick();
    pic_eoi(0);
}

extern "x86-interrupt" fn apic_timer_handler(_stack_frame: InterruptStackFrame) {
    handle_timer_tick();
    apic::LOCAL_APIC_TIMER.end_of_interrupt();
}

fn handle_timer_tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::arch::x86_64::apic::LOCAL_APIC_TIMER.tick();
    crate::cpu::smp::on_tick();
    crate::process::scheduler::on_timer_tick();

    if ticks % 100 == 0 {
        serial_println!("[TIMER] ~{} seconds uptime", ticks / 18);
    }
}

extern "x86-interrupt" fn keyboard_handler(_stack_frame: InterruptStackFrame) {
    let scancode = unsafe { Port::<u8>::new(0x60).read() };
    if scancode == 0x00 || scancode == 0xE0 {
        serial_println!("[KBD] ignored non-actionable scancode {:#04x}", scancode);
    } else {
        serial_println!("[KBD] scancode {:#04x}", scancode);
    }
    pic_eoi(1);
    let _ = _stack_frame;
}

extern "x86-interrupt" fn syscall_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let frame = crate::security::security::SyscallFrame {
        pid: 1,
        syscall_no: 2,
        arg1: 0x1000,
        arg2: 0x1000,
        arg3: 0x100,
        arg4: 0,
        arg5: 0,
    };

    let outcome = crate::security::security::SECURITY.handle_syscall(frame);
    match outcome {
        Ok(_) => serial_println!("[SYSCALL] ring-3 syscall accepted"),
        Err(err) => serial_println!("[SYSCALL] denied: {:?}", err),
    }

    let _ = _stack_frame;
}

static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();

fn init_pic() {
    unsafe {
        let mut cmd1: Port<u8> = Port::new(0x20);
        let mut data1: Port<u8> = Port::new(0x21);
        let mut cmd2: Port<u8> = Port::new(0xA0);
        let mut data2: Port<u8> = Port::new(0xA1);

        // ICW1: begin initialization, expect ICW4
        cmd1.write(0x11);
        cmd2.write(0x11);
        // ICW2: remap IRQs to 32..47
        data1.write(PIC_1_OFFSET);
        data2.write(PIC_2_OFFSET);
        // ICW3: cascade identity
        data1.write(4);
        data2.write(2);
        // ICW4: 8086 mode
        data1.write(0x01);
        data2.write(0x01);

        // Unmask IRQ0 (timer) and IRQ1 (keyboard); mask everything else.
        data1.write(0xFC);
        data2.write(0xFF);
    }
}

pub fn init() {
    unsafe {
        (*(&raw mut IDT))
            .breakpoint
            .set_handler_fn(breakpoint_handler);
        (*(&raw mut IDT))
            .double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        (*(&raw mut IDT)).slice_mut(PIC_1_OFFSET..PIC_1_OFFSET + 1)[0]
            .set_handler_fn(timer_handler);
        (*(&raw mut IDT)).slice_mut(PIC_1_OFFSET + 1..PIC_1_OFFSET + 2)[0]
            .set_handler_fn(keyboard_handler);
        IDT[apic::APIC_TIMER_VECTOR].set_handler_fn(apic_timer_handler);
        IDT[0x80].set_handler_fn(syscall_interrupt_handler);
        (*(&raw mut IDT)).load();
    }
    init_pic();
}
