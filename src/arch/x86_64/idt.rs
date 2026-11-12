use core::{
    arch::global_asm,
    sync::atomic::{AtomicU64, Ordering},
};
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
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
// Shared frame layout for timer interrupts, syscall entry, and new tasks.
// The CPU aligns RSP before pushing five words. Fifteen GPR pushes restore
// the alignment required before calling an extern "C" Rust function.
global_asm!(
    r#"
    .macro entry name, handler
    .global \name
    .type \name,@function

\name:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    cld
    mov rdi, rsp
    call \handler
    mov rsp, rax
    jmp restore_interrupt_frame
    .endm

    entry timer_interrupt_entry, timer_interrupt_switch
    entry apic_timer_interrupt_entry, apic_timer_interrupt_switch
    entry reschedule_interrupt_entry, reschedule_interrupt_switch
    entry syscall_interrupt_entry, dispatch_syscall

    .global resume_interrupt_frame
    .type resume_interrupt_frame,@function
resume_interrupt_frame:
    mov rsp, rdi
restore_interrupt_frame:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    iretq
    "#
);

#[repr(C)]
pub(crate) struct RegisterFrame {
    r15_through_rbp: [u64; 9],
    pub rdi: u64,
    pub rsi: u64,
    rdx_through_rbx: [u64; 3],
    pub rax: u64,
}

#[repr(C)]
pub(crate) struct UserInterruptFrame {
    pub registers: RegisterFrame,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl UserInterruptFrame {
    pub fn new(entry: usize, stack: u64, argument: u64) -> Self {
        let selectors = gdt::ring3_selectors();
        Self {
            registers: RegisterFrame {
                r15_through_rbp: [0; 9],
                rdi: argument,
                rsi: 0,
                rdx_through_rbx: [0; 3],
                rax: 0,
            },
            rip: entry as u64,
            cs: u64::from(selectors.code.0),
            rflags: 0x202,
            rsp: stack,
            ss: u64::from(selectors.data.0),
        }
    }

    pub fn validate_return(&self) {
        validate_return(self.cs, self.ss, self.rip, self.rsp);
    }
}

fn validate_interrupt_return(frame: &InterruptStackFrame) {
    validate_return(
        u64::from(frame.code_segment.0),
        u64::from(frame.stack_segment.0),
        frame.instruction_pointer.as_u64(),
        frame.stack_pointer.as_u64(),
    );
}

fn validate_return(cs: u64, ss: u64, rip: u64, rsp: u64) {
    if cs & 3 == 0 {
        return;
    }

    // IRET can fault in ring 0 if the user left a noncanonical RSP.
    let selectors = gdt::ring3_selectors();
    use kelvos_shared_ring::layout::USER_LIMIT;
    if cs != u64::from(selectors.code.0)
        || ss != u64::from(selectors.data.0)
        || rip >= USER_LIMIT
        || rsp >= USER_LIMIT
    {
        serial_println!(
            "[FAULT] invalid ring-3 return frame rip={:#x} rsp={:#x}",
            rip,
            rsp
        );
        crate::process::scheduler::terminate_current(kelvos_shared_ring::ExitStatus::Faulted(
            kelvos_shared_ring::INVALID_USER_RETURN,
        ));
    }
}

const _: () = assert!(size_of::<UserInterruptFrame>() == 20 * 8);

extern "C" {
    fn timer_interrupt_entry();
    fn apic_timer_interrupt_entry();
    fn reschedule_interrupt_entry();
    fn syscall_interrupt_entry();
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

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    serial_println!("[EXC] #BP breakpoint exception");
    validate_interrupt_return(&stack_frame);
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

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    serial_println!(
        "[FAULT] #UD invalid opcode at rip={:#x}",
        stack_frame.instruction_pointer.as_u64()
    );
    handle_fault(&stack_frame, 6)
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    serial_println!(
        "[FAULT] #GP error={:#x} rip={:#x}",
        error_code,
        stack_frame.instruction_pointer.as_u64()
    );
    handle_fault(&stack_frame, 13)
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let fault_address: u64;
    unsafe {
        core::arch::asm!(
            "mov {}, cr2",
            out(reg) fault_address,
            options(nomem, nostack, preserves_flags)
        );
    }
    serial_println!(
        "[FAULT] #PF address={:#x} error={:?} rip={:#x}",
        fault_address,
        error_code,
        stack_frame.instruction_pointer.as_u64()
    );
    handle_fault(&stack_frame, 14)
}

fn handle_fault(frame: &InterruptStackFrame, vector: u16) -> ! {
    if frame.code_segment.rpl() == PrivilegeLevel::Ring3 {
        crate::process::scheduler::terminate_current(kelvos_shared_ring::ExitStatus::Faulted(
            vector,
        ));
    }
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}

macro_rules! fault_handler {
    ($handler:ident, $vector:literal, $number:literal) => {
        extern "x86-interrupt" fn $handler(frame: InterruptStackFrame) {
            serial_println!(
                "[FAULT] {} rip={:#x}",
                $vector,
                frame.instruction_pointer.as_u64()
            );
            handle_fault(&frame, $number);
        }
    };
    ($handler:ident, $vector:literal, $number:literal, error_code) => {
        extern "x86-interrupt" fn $handler(frame: InterruptStackFrame, error: u64) {
            serial_println!(
                "[FAULT] {} error={:#x} rip={:#x}",
                $vector,
                error,
                frame.instruction_pointer.as_u64()
            );
            handle_fault(&frame, $number);
        }
    };
}

fault_handler!(divide_error_handler, "#DE", 0);
fault_handler!(debug_handler, "#DB", 1);
fault_handler!(overflow_handler, "#OF", 4);
fault_handler!(bounds_handler, "#BR", 5);
fault_handler!(device_not_available_handler, "#NM", 7);
fault_handler!(invalid_tss_handler, "#TS", 10, error_code);
fault_handler!(segment_not_present_handler, "#NP", 11, error_code);
fault_handler!(stack_segment_handler, "#SS", 12, error_code);
fault_handler!(x87_handler, "#MF", 16);
fault_handler!(alignment_handler, "#AC", 17, error_code);
fault_handler!(simd_handler, "#XM", 19);

extern "x86-interrupt" fn apic_error_handler(stack_frame: InterruptStackFrame) {
    apic::clear_error_status();
    apic::end_of_interrupt();
    validate_interrupt_return(&stack_frame);
}

extern "x86-interrupt" fn apic_spurious_handler(stack_frame: InterruptStackFrame) {
    validate_interrupt_return(&stack_frame);
}

extern "x86-interrupt" fn tlb_shootdown_ipi_handler(stack_frame: InterruptStackFrame) {
    unsafe {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    apic::end_of_interrupt();
    validate_interrupt_return(&stack_frame);
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

#[unsafe(no_mangle)]
extern "C" fn timer_interrupt_switch(current_stack_pointer: usize) -> usize {
    pic_eoi(0);
    handle_timer_tick();
    schedule_interrupt(current_stack_pointer)
}

#[unsafe(no_mangle)]
extern "C" fn apic_timer_interrupt_switch(current_stack_pointer: usize) -> usize {
    apic::LOCAL_APIC_TIMER.tick();
    crate::cpu::smp::on_tick();
    apic::end_of_interrupt();
    schedule_interrupt(current_stack_pointer)
}

#[unsafe(no_mangle)]
extern "C" fn reschedule_interrupt_switch(current_stack_pointer: usize) -> usize {
    crate::cpu::smp::on_reschedule_ipi();
    apic::end_of_interrupt();
    if let Some(pc) = crate::arch::x86_64::percpu::try_this_cpu() {
        pc.reschedule_pending.store(false, Ordering::Release);
    }
    schedule_interrupt(current_stack_pointer)
}

fn schedule_interrupt(stack: usize) -> usize {
    // The assembly stubs build the same frame on a trusted kernel stack.
    unsafe { &*(stack as *const UserInterruptFrame) }.validate_return();
    crate::process::scheduler::switch_from_interrupt(
        stack,
        crate::process::scheduler::Switch::Preempt,
    )
}

extern "x86-interrupt" fn keyboard_handler(stack_frame: InterruptStackFrame) {
    let scancode = unsafe { Port::<u8>::new(0x60).read() };
    if scancode == 0x00 || scancode == 0xE0 {
        serial_println!("[KBD] ignored non-actionable scancode {:#04x}", scancode);
    } else {
        serial_println!("[KBD] scancode {:#04x}", scancode);
    }
    pic_eoi(1);
    validate_interrupt_return(&stack_frame);
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
        let idt = &mut *(&raw mut IDT);
        idt.divide_error.set_handler_fn(divide_error_handler);
        idt.debug.set_handler_fn(debug_handler);
        idt.overflow.set_handler_fn(overflow_handler);
        idt.bound_range_exceeded.set_handler_fn(bounds_handler);
        idt.device_not_available
            .set_handler_fn(device_not_available_handler);
        idt.invalid_tss.set_handler_fn(invalid_tss_handler);
        idt.segment_not_present
            .set_handler_fn(segment_not_present_handler);
        idt.stack_segment_fault
            .set_handler_fn(stack_segment_handler);
        idt.x87_floating_point.set_handler_fn(x87_handler);
        idt.alignment_check.set_handler_fn(alignment_handler);
        idt.simd_floating_point.set_handler_fn(simd_handler);
        (*(&raw mut IDT))
            .breakpoint
            .set_handler_fn(breakpoint_handler);
        (*(&raw mut IDT))
            .double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        (*(&raw mut IDT))
            .invalid_opcode
            .set_handler_fn(invalid_opcode_handler);
        (*(&raw mut IDT))
            .general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        (*(&raw mut IDT))
            .page_fault
            .set_handler_fn(page_fault_handler);
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
            .set_handler_addr(VirtAddr::from_ptr(syscall_interrupt_entry as *const ()))
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
