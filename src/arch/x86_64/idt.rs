use core::{
    arch::global_asm,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use x86_64::{PrivilegeLevel, VirtAddr};

use crate::{
    arch::x86_64::{apic, gdt},
    serial_println,
};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = 40;

const PIT_COMMAND_PORT: u16 = 0x43;
const PIT_CHANNEL_0_PORT: u16 = 0x40;
const PIT_INPUT_HZ: u64 = 1_193_182;
const PIT_DIVISOR: u16 = 11_932;

static TICKS: AtomicU64 = AtomicU64::new(0);
static CHILD_STARTED_REPORTED: AtomicBool = AtomicBool::new(false);

global_asm!(
    ".global timer_interrupt_entry",
    ".type timer_interrupt_entry,@function",
    "timer_interrupt_entry:",
    "pushq %rax",
    "pushq %rbx",
    "pushq %rcx",
    "pushq %rdx",
    "pushq %rsi",
    "pushq %rdi",
    "pushq %rbp",
    "pushq %r8",
    "pushq %r9",
    "pushq %r10",
    "pushq %r11",
    "pushq %r12",
    "pushq %r13",
    "pushq %r14",
    "pushq %r15",
    "cld",
    "movq %rsp, %rdi",
    "call timer_interrupt_switch",
    "movq %rax, %rsp",
    "popq %r15",
    "popq %r14",
    "popq %r13",
    "popq %r12",
    "popq %r11",
    "popq %r10",
    "popq %r9",
    "popq %r8",
    "popq %rbp",
    "popq %rdi",
    "popq %rsi",
    "popq %rdx",
    "popq %rcx",
    "popq %rbx",
    "popq %rax",
    "iretq",
    options(att_syntax)
);

extern "C" {
    fn timer_interrupt_entry();
}

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

extern "x86-interrupt" fn apic_timer_handler(_stack_frame: InterruptStackFrame) {
    apic::LOCAL_APIC_TIMER.tick();
    apic::LOCAL_APIC_TIMER.end_of_interrupt();
}

fn handle_timer_tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::cpu::smp::on_tick();

    static LAST: AtomicU64 = AtomicU64::new(0);
    let secs = ticks * u64::from(PIT_DIVISOR) / PIT_INPUT_HZ;
    if secs != LAST.swap(secs, Ordering::Relaxed) {
        serial_println!("[TIMER] {} seconds uptime", secs);
    }
}

#[unsafe(no_mangle)]
extern "C" fn timer_interrupt_switch(current_stack_pointer: usize) -> usize {
    pic_eoi(0);
    handle_timer_tick();

    let target = crate::process::scheduler::SCHEDULER
        .lock()
        .switch_from_interrupt(current_stack_pointer);
    let Some((stack_pointer, stack_top, first_start)) = target else {
        return current_stack_pointer;
    };

    gdt::set_kernel_stack_top(stack_top);
    if let Some(task_id) = first_start {
        serial_println!("[SCHED] starting task {}", task_id);
    }

    let ring = unsafe {
        &*(kelvos_shared_ring::USER_RING_ADDRESS as *const kelvos_shared_ring::SharedRing)
    };
    if ring.child_started.load(Ordering::Acquire) != 0
        && !CHILD_STARTED_REPORTED.swap(true, Ordering::AcqRel)
    {
        serial_println!("[TASK] spawned user task executed");
    }

    stack_pointer
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

extern "x86-interrupt" fn sys_ring_notify_handler(_stack_frame: InterruptStackFrame) {
    crate::process::ring::sys_ring_notify();
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

fn init_pit() {
    unsafe {
        let mut command: Port<u8> = Port::new(PIT_COMMAND_PORT);
        let mut channel_0: Port<u8> = Port::new(PIT_CHANNEL_0_PORT);

        command.write(0x34);
        channel_0.write(PIT_DIVISOR as u8);
        channel_0.write((PIT_DIVISOR >> 8) as u8);
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
            .set_handler_addr(VirtAddr::from_ptr(timer_interrupt_entry as *const ()));
        (*(&raw mut IDT)).slice_mut(PIC_1_OFFSET + 1..PIC_1_OFFSET + 2)[0]
            .set_handler_fn(keyboard_handler);
        IDT[apic::APIC_TIMER_VECTOR].set_handler_fn(apic_timer_handler);
        IDT[0x80]
            .set_handler_fn(sys_ring_notify_handler)
            .set_privilege_level(PrivilegeLevel::Ring3);
        (*(&raw mut IDT)).load();
    }
    init_pic();
    init_pit();
}
