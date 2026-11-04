#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Runnable,
    Sleeping,
    Stopped,
    Zombie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rflags: u64,
    pub cs: u64,
    pub ss: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: u32,
    pub state: TaskState,
    pub stack_top: usize,
    pub context: Context,
    pub priority: u8,
    pub entry_point: usize,
    pub ring: u8,
}

impl Task {
    pub const fn new(id: u32, stack_top: usize) -> Self {
        Self::with_entry(id, stack_top, 0, 0)
    }

    pub const fn with_entry(id: u32, stack_top: usize, entry_point: usize, ring: u8) -> Self {
        Self {
            id,
            state: TaskState::Runnable,
            stack_top,
            context: Context {
                rip: entry_point as u64,
                rsp: stack_top as u64,
                rbp: stack_top as u64,
                rflags: 0x202,
                cs: 0,
                ss: 0,
            },
            priority: 16,
            entry_point,
            ring,
        }
    }

    pub fn set_context(&mut self, rip: u64, rsp: u64, rbp: u64) {
        self.context.rip = rip;
        self.context.rsp = rsp;
        self.context.rbp = rbp;
        self.entry_point = rip as usize;
    }

    pub const fn is_user_task(&self) -> bool {
        self.ring == 3
    }

    pub fn prepare_for_user_mode(&mut self, entry_point: usize, stack_top: usize) {
        let selectors = crate::arch::x86_64::gdt::ring3_selectors();
        self.ring = 3;
        self.entry_point = entry_point;
        self.stack_top = stack_top;
        self.context.rip = entry_point as u64;
        self.context.rsp = stack_top as u64;
        self.context.rbp = stack_top as u64;
        self.context.rflags = 0x202;
        self.context.cs = selectors.code.0 as u64;
        self.context.ss = selectors.data.0 as u64;
    }

    pub fn user_frame(&mut self) -> crate::arch::x86_64::gdt::UserModeFrame {
        let frame = crate::arch::x86_64::gdt::build_user_mode_frame(self.entry_point, self.stack_top);
        self.context.cs = frame.cs as u64;
        self.context.ss = frame.ss as u64;
        frame
    }

    pub fn context_for_switch(&self) -> Context {
        self.context
    }
}
