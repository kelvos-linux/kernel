use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
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
}

static mut TSS: TaskStateSegment = TaskStateSegment::new();

// Place GDT directly in a static variable
static mut GDT: GlobalDescriptorTable = GlobalDescriptorTable::new();
static mut RING3_SELECTORS: Ring3Selectors = Ring3Selectors {
    code: SegmentSelector::new(0, PrivilegeLevel::Ring0),
    data: SegmentSelector::new(0, PrivilegeLevel::Ring0),
};

pub fn ring3_selectors() -> Ring3Selectors {
    unsafe { RING3_SELECTORS }
}

pub fn build_user_mode_frame(entry_point: usize, stack_top: usize) -> UserModeFrame {
    let selectors = ring3_selectors();
    UserModeFrame {
        rip: entry_point as u64,
        cs: selectors.code.0,
        rflags: 0x202,
        rsp: stack_top as u64,
        ss: selectors.data.0,
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

        const KERNEL_STACK_SIZE: usize = 4096 * 8;
        static mut KERNEL_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];
        let kernel_stack_top = VirtAddr::from_ptr(&raw const KERNEL_STACK) + KERNEL_STACK_SIZE as u64;
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
