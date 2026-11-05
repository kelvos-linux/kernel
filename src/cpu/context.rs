#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuContext {
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
}
