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
    ".global apic_timer_interrupt_entry",
    ".type apic_timer_interrupt_entry,@function",
    "apic_timer_interrupt_entry:",
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
    "call apic_timer_interrupt_switch",
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
    ".global reschedule_interrupt_entry",
    ".type reschedule_interrupt_entry,@function",
    "reschedule_interrupt_entry:",
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
    "call reschedule_interrupt_switch",
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
    fn apic_timer_interrupt_entry();
    fn reschedule_interrupt_entry();
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

extern "x86-interrupt" fn apic_error_handler(_stack_frame: InterruptStackFrame) {
    apic::end_of_interrupt();
}

extern "x86-interrupt" fn apic_spurious_handler(_stack_frame: InterruptStackFrame) {}

extern "x86-interrupt" fn tlb_shootdown_ipi_handler(_stack_frame: InterruptStackFrame) {
    unsafe {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    apic::end_of_interrupt();
}

extern "x86-interrupt" fn halt_ipi_handler(_stack_frame: InterruptStackFrame) {
    apic::end_of_interrupt();
    if let Some(pc) = crate::arch::x86_64::percpu::try_this_cpu() {
        let cpu_id = pc.cpu_id.load(Ordering::Relaxed) as usize;
        if let Some(slot) = crate::cpu::state::CPU_STATES.get(cpu_id) {
            let _ = slot.transition(
                crate::cpu::state::CpuState::Online,
                crate::cpu::state::CpuState::Halted,
            );
        }
    }
    loop {
        x86_64::instructions::interrupts::disable();
        x86_64::instructions::hlt();
    }
}

fn handle_timer_tick() {
    let ticks = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::cpu::smp::on_tick();
    let _secs = ticks * u64::from(PIT_DIVISOR) / PIT_INPUT_HZ;
}

fn switch_to_runnable_task(current_stack_pointer: usize) -> usize {
    let cpu_id = crate::arch::x86_64::percpu::this_cpu().id() as usize;
    let target = crate::process::scheduler::SCHEDULER
        .lock()
        .switch_from_interrupt(cpu_id, current_stack_pointer);
    let Some((stack_pointer, stack_top, first_start)) = target else {
        return current_stack_pointer;
    };

    gdt::set_kernel_stack_top_for_cpu(cpu_id, stack_top);
    if let Some(task_id) = first_start {
        serial_println!("[SCHED] CPU {} starting task {}", cpu_id, task_id);
    }
    stack_pointer
}

#[unsafe(no_mangle)]
extern "C" fn timer_interrupt_switch(current_stack_pointer: usize) -> usize {
    pic_eoi(0);
    handle_timer_tick();
    let stack_pointer = switch_to_runnable_task(current_stack_pointer);
    report_child_started();
    stack_pointer
}

#[unsafe(no_mangle)]
extern "C" fn apic_timer_interrupt_switch(current_stack_pointer: usize) -> usize {
    apic::LOCAL_APIC_TIMER.tick();
    crate::cpu::smp::on_tick();
    apic::end_of_interrupt();
    let stack_pointer = switch_to_runnable_task(current_stack_pointer);
    report_child_started();
    stack_pointer
}

#[unsafe(no_mangle)]
extern "C" fn reschedule_interrupt_switch(current_stack_pointer: usize) -> usize {
    crate::cpu::smp::on_reschedule_ipi();
    apic::end_of_interrupt();
    if let Some(pc) = crate::arch::x86_64::percpu::try_this_cpu() {
        pc.reschedule_pending.store(false, Ordering::Release);
    }
    let stack_pointer = switch_to_runnable_task(current_stack_pointer);
    report_child_started();
    stack_pointer
}

fn report_child_started() {
    let ring = unsafe {
        &*(kelvos_shared_ring::USER_RING_ADDRESS as *const kelvos_shared_ring::SharedRing)
    };
    if ring.child_started.load(Ordering::Acquire) != 0
        && !CHILD_STARTED_REPORTED.swap(true, Ordering::AcqRel)
    {
        serial_println!("[TASK] spawned user task executed");
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
        IDT[apic::APIC_TIMER_VECTOR]
            .set_handler_addr(VirtAddr::from_ptr(apic_timer_interrupt_entry as *const ()));
        IDT[apic::IPI_RESCHEDULE]
            .set_handler_addr(VirtAddr::from_ptr(reschedule_interrupt_entry as *const ()));
        IDT[apic::IPI_TLB_SHOOTDOWN].set_handler_fn(tlb_shootdown_ipi_handler);
        IDT[apic::IPI_HALT].set_handler_fn(halt_ipi_handler);
        IDT[apic::APIC_ERROR_VECTOR].set_handler_fn(apic_error_handler);
        IDT[apic::APIC_SPURIOUS_VECTOR].set_handler_fn(apic_spurious_handler);
        IDT[0x80]
            .set_handler_fn(sys_ring_notify_handler)
            .set_privilege_level(PrivilegeLevel::Ring3);
        (*(&raw mut IDT)).load();
    }

    init_pic();
    init_pit();
}

pub fn init_ap() {
    unsafe {
        (*(&raw mut IDT)).load();
    }
}
