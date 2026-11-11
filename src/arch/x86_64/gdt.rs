use crate::cpu::topology::MAX_CPUS;
use core::arch::asm;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::{PrivilegeLevel, VirtAddr};

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring3Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserModeFrame {
    pub rip: u64,
    pub cs: u16,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u16,
    pub task_id: u32,
}

static mut TSS: TaskStateSegment = TaskStateSegment::new();

const KERNEL_STACK_SIZE: usize = 4096 * 8;

#[repr(align(16))]
struct KernelStack([u8; KERNEL_STACK_SIZE]);

static mut BOOT_KERNEL_STACK: KernelStack = KernelStack([0; KERNEL_STACK_SIZE]);

// Place GDT directly in a static variable
static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();
const AP_DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;

struct ApCpuTables {
    gdt: GlobalDescriptorTable,
    tss: TaskStateSegment,
    double_fault_stack: [u8; AP_DOUBLE_FAULT_STACK_SIZE],
}

impl ApCpuTables {
    const fn new() -> Self {
        Self {
            gdt: GlobalDescriptorTable::new(),
            tss: TaskStateSegment::new(),
            double_fault_stack: [0; AP_DOUBLE_FAULT_STACK_SIZE],
        }
    }
}

static mut AP_CPU_TABLES: [ApCpuTables; MAX_CPUS] = [const { ApCpuTables::new() }; MAX_CPUS];
static mut RING3_SELECTORS: Ring3Selectors = Ring3Selectors {
    code: SegmentSelector::new(0, PrivilegeLevel::Ring0),
    data: SegmentSelector::new(0, PrivilegeLevel::Ring0),
};

pub fn ring3_selectors() -> Ring3Selectors {
    unsafe { RING3_SELECTORS }
}

pub fn kernel_stack_top() -> usize {
    unsafe { (*core::ptr::addr_of!(TSS)).privilege_stack_table[0].as_u64() as usize }
}

pub fn set_kernel_stack_top(stack_top: usize) {
    unsafe {
        (*core::ptr::addr_of_mut!(TSS)).privilege_stack_table[0] = VirtAddr::new(stack_top as u64);
    }
}

pub fn set_kernel_stack_top_for_cpu(cpu_id: usize, stack_top: usize) {
    if cpu_id == 0 {
        set_kernel_stack_top(stack_top);
        return;
    }
    if cpu_id >= MAX_CPUS {
        panic!("logical CPU id is out of range");
    }
    unsafe {
        let tables = &mut *core::ptr::addr_of_mut!(AP_CPU_TABLES)
            .cast::<ApCpuTables>()
            .add(cpu_id);
        tables.tss.privilege_stack_table[0] = VirtAddr::new(stack_top as u64);
    }
}

pub fn build_user_mode_frame(entry_point: usize, stack_top: usize, task_id: u32) -> UserModeFrame {
    let selectors = ring3_selectors();
    UserModeFrame {
        rip: entry_point as u64,
        cs: selectors.code.0,
        rflags: 0x202,
        rsp: stack_top as u64,
        ss: selectors.data.0,
        task_id,
    }
}

pub unsafe fn enter_user_mode(frame: UserModeFrame) -> ! {
    unsafe {
        asm!(
            "push {ss}",
            "push {rsp}",
            "push {rflags}",
            "push {cs}",
            "push {rip}",
            "iretq",
            in("rdi") u64::from(frame.task_id),
            ss = in(reg) u64::from(frame.ss),
            rsp = in(reg) frame.rsp,
            rflags = in(reg) frame.rflags,
            cs = in(reg) u64::from(frame.cs),
            rip = in(reg) frame.rip,
            options(noreturn)
        );
    }
}

pub fn init() {
    unsafe {
        // 1. Configure TSS with an IST stack for double faults and a ring-0 privilege stack.
        (*(&raw mut TSS)).interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            let stack_start = VirtAddr::from_ptr(&raw const STACK);
            let stack_end = stack_start + STACK_SIZE as u64;
            stack_end
        };

        let kernel_stack_top =
            VirtAddr::from_ptr(&raw const BOOT_KERNEL_STACK) + KERNEL_STACK_SIZE as u64;
        (*(&raw mut TSS)).privilege_stack_table[0] = kernel_stack_top;

        // 2. Append entries directly to the static GDT.
        let code_selector = (*(&raw mut GDT)).append(Descriptor::kernel_code_segment());
        let _data_selector = (*(&raw mut GDT)).append(Descriptor::kernel_data_segment());
        let user_code_selector = (*(&raw mut GDT)).append(Descriptor::user_code_segment());
        let user_data_selector = (*(&raw mut GDT)).append(Descriptor::user_data_segment());
        let tss_selector = (*(&raw mut GDT)).append(Descriptor::tss_segment(&*(&raw const TSS)));

        RING3_SELECTORS = Ring3Selectors {
            code: user_code_selector,
            data: user_data_selector,
        };

        // 3. Load GDT (now a static, satisfies 'static requirement)
        (*(&raw mut GDT)).load();

        // 4. Reload segment registers
        CS::set_reg(code_selector);
        DS::set_reg(SegmentSelector(0));
        ES::set_reg(SegmentSelector(0));
        SS::set_reg(SegmentSelector(0));

        // 5. Load TSS
        load_tss(tss_selector);
    }
}

pub fn init_ap(cpu_id: usize, kernel_stack_top: usize) {
    if cpu_id == 0 || cpu_id >= MAX_CPUS {
        panic!("invalid AP logical CPU id");
    }

    unsafe {
        let tables = &mut *core::ptr::addr_of_mut!(AP_CPU_TABLES)
            .cast::<ApCpuTables>()
            .add(cpu_id);
        tables.tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] =
            VirtAddr::from_ptr(tables.double_fault_stack.as_ptr())
                + AP_DOUBLE_FAULT_STACK_SIZE as u64;
        tables.tss.privilege_stack_table[0] = VirtAddr::new(kernel_stack_top as u64);

        let code_selector = tables.gdt.append(Descriptor::kernel_code_segment());
        let _data_selector = tables.gdt.append(Descriptor::kernel_data_segment());
        let _user_code_selector = tables.gdt.append(Descriptor::user_code_segment());
        let _user_data_selector = tables.gdt.append(Descriptor::user_data_segment());
        let tss_selector = tables.gdt.append(Descriptor::tss_segment(&tables.tss));
        tables.gdt.load();

        CS::set_reg(code_selector);
        DS::set_reg(SegmentSelector(0));
        ES::set_reg(SegmentSelector(0));
        SS::set_reg(SegmentSelector(0));
        load_tss(tss_selector);
    }
}
