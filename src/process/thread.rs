use core::arch::asm;
use kelvos_shared_ring::{Pid, Tid};
use x86_64::{
    instructions::segmentation::{Segment, DS, ES, FS, GS},
    structures::gdt::SegmentSelector,
};

use crate::arch::x86_64::idt::UserInterruptFrame;

#[repr(C, align(16))]
struct FxState([u8; 512]);

impl FxState {
    const fn initial() -> Self {
        let mut state = [0; 512];
        state[0] = 0x7f;
        state[1] = 0x03; // x87 control word
        state[24] = 0x80;
        state[25] = 0x1f; // MXCSR
        Self(state)
    }
}

pub struct Thread {
    pub id: Tid,
    pub process: Pid,
    pub stack_top: usize,
    stack: usize,
    fp: FxState,
    segments: [SegmentSelector; 4],
}

impl Thread {
    /// # Safety
    /// The aligned kernel stack must belong exclusively to this task slot.
    pub unsafe fn new(
        id: Tid,
        process: Pid,
        entry: usize,
        user_stack_top: u64,
        stack_top: usize,
        argument: u64,
    ) -> Self {
        let stack = stack_top - size_of::<UserInterruptFrame>();
        unsafe {
            (stack as *mut UserInterruptFrame).write(UserInterruptFrame::new(
                entry,
                user_stack_top,
                argument,
            ))
        };
        Self {
            id,
            process,
            stack_top,
            stack,
            fp: FxState::initial(),
            segments: [SegmentSelector(0); 4],
        }
    }

    pub fn save(&mut self, stack: usize) {
        self.stack = stack;
        self.segments = [DS::get_reg(), ES::get_reg(), FS::get_reg(), GS::get_reg()];
        // The kernel target does not use FP/SIMD. Userspace has x87 and SSE state.
        unsafe { asm!("fxsave64 [{}]", in(reg) &mut self.fp, options(nostack, preserves_flags)) };
    }

    /// # Safety
    /// The thread must be blocked with a saved syscall frame on its stack.
    pub unsafe fn complete_wait(&self, result: u64) {
        // Write only RAX. The entry stub still owns the frame until it switches.
        unsafe {
            let frame = self.stack as *mut UserInterruptFrame;
            (&raw mut (*frame).registers.rax).write(result);
        }
    }

    pub fn restore(&self) -> usize {
        // The kernel does not use FS/GS bases. The GDT has no nonzero user
        // segment base, and userspace cannot write base MSRs or use FSGSBASE.
        let [ds, es, fs, gs] = self.segments;
        unsafe {
            DS::set_reg(ds);
            ES::set_reg(es);
            FS::set_reg(fs);
            GS::set_reg(gs);
        }
        unsafe {
            asm!("fxrstor64 [{}]", in(reg) &self.fp, options(nostack, preserves_flags, readonly))
        };
        self.stack
    }
}
