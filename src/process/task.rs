use crate::cpu::affinity::CpuMask;

// ─────────────────────────────────────────────────────────────────────────────
// Debug configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Enable task diagnostics in both debug and release builds.
pub const TASK_DEBUG: bool = true;

/// Runtime toggle — flips on by default if `TASK_DEBUG` is on.
/// Can be flipped at runtime (e.g. from a shell command `tasks debug on`).
static TASK_DEBUG_RUNTIME: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(TASK_DEBUG);

#[inline]
pub fn task_debug_enabled() -> bool {
    use core::sync::atomic::Ordering;
    TASK_DEBUG_RUNTIME.load(Ordering::Relaxed)
}

pub fn set_task_debug(on: bool) {
    use core::sync::atomic::Ordering;
    TASK_DEBUG_RUNTIME.store(on, Ordering::Relaxed);
    crate::serial_println!(
        "[task] debug output {}",
        if on { "ENABLED" } else { "DISABLED" }
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Runnable,
    Sleeping,
    Stopped,
    Zombie,
}

impl TaskState {
    pub const fn name(self) -> &'static str {
        match self {
            TaskState::Runnable => "Runnable",
            TaskState::Sleeping => "Sleeping",
            TaskState::Stopped => "Stopped",
            TaskState::Zombie => "Zombie",
        }
    }

    /// One-letter code for compact table dumps.
    pub const fn short(self) -> char {
        match self {
            TaskState::Runnable => 'R',
            TaskState::Sleeping => 'S',
            TaskState::Stopped => 'T',
            TaskState::Zombie => 'Z',
        }
    }
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
    pub kernel_stack_top: usize,
    pub kernel_stack_pointer: usize,
    pub context: Context,
    pub priority: u8,
    pub entry_point: usize,
    pub ring: u8,
    pub started: bool,
    pub affinity: CpuMask,
}

// ─────────────────────────────────────────────────────────────────────────────
// Task impl
// ─────────────────────────────────────────────────────────────────────────────

impl Task {
    pub fn new(id: u32, stack_top: usize) -> Self {
        Self::with_entry(id, stack_top, 0, 0)
    }

    pub fn with_entry(id: u32, stack_top: usize, entry_point: usize, ring: u8) -> Self {
        Self {
            id,
            state: TaskState::Runnable,
            stack_top,
            kernel_stack_top: 0,
            kernel_stack_pointer: 0,
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
            started: false,
            affinity: CpuMask::all(
                crate::cpu::topology::TOPOLOGY
                    .get()
                    .map_or(1, |topology| topology.cpu_count),
            ),
        }
    }

    pub fn initialize_kernel_stack(&mut self, kernel_stack_top: usize) {
        const SAVED_REGISTER_COUNT: usize = 15;
        const INTERRUPT_FRAME_COUNT: usize = 5;
        let frame_words = SAVED_REGISTER_COUNT + INTERRUPT_FRAME_COUNT;
        let stack_pointer =
            (kernel_stack_top - frame_words * core::mem::size_of::<u64>()) as *mut u64;

        unsafe {
            for index in 0..SAVED_REGISTER_COUNT {
                stack_pointer.add(index).write(0);
            }
            stack_pointer.add(9).write(u64::from(self.id));
            stack_pointer.add(15).write(self.context.rip);
            stack_pointer.add(16).write(self.context.cs);
            stack_pointer.add(17).write(self.context.rflags);
            stack_pointer.add(18).write(self.context.rsp);
            stack_pointer.add(19).write(self.context.ss);
        }

        self.kernel_stack_top = kernel_stack_top;
        self.kernel_stack_pointer = stack_pointer as usize;

        if task_debug_enabled() {
            crate::serial_println!(
                "[task {}] kernel stack initialized: top={:#x} sp={:#x}",
                self.id,
                self.kernel_stack_top,
                self.kernel_stack_pointer
            );
        }
    }

    pub fn set_context(&mut self, rip: u64, rsp: u64, rbp: u64) {
        if task_debug_enabled() {
            crate::serial_println!(
                "[task {}] set_context: rip {:#x} -> {:#x}, rsp {:#x} -> {:#x}, rbp {:#x} -> {:#x}",
                self.id,
                self.context.rip,
                rip,
                self.context.rsp,
                rsp,
                self.context.rbp,
                rbp
            );
        }
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

        if task_debug_enabled() {
            crate::serial_println!(
                "[task {}] prepared for user mode: entry={:#x} stack={:#x} cs={:#x} ss={:#x}",
                self.id,
                entry_point,
                stack_top,
                self.context.cs,
                self.context.ss
            );
        }
    }

    pub fn user_frame(&mut self) -> crate::arch::x86_64::gdt::UserModeFrame {
        let frame = crate::arch::x86_64::gdt::build_user_mode_frame(
            self.entry_point,
            self.stack_top,
            self.id,
        );
        self.context.cs = frame.cs as u64;
        self.context.ss = frame.ss as u64;

        if task_debug_enabled() {
            crate::serial_println!(
                "[task {}] built user-mode frame: cs={:#x} ss={:#x}",
                self.id,
                frame.cs,
                frame.ss
            );
        }
        frame
    }

    pub fn context_for_switch(&self) -> Context {
        if task_debug_enabled() {
            crate::serial_println!(
                "[task {}] switch-out: rip={:#x} rsp={:#x} rbp={:#x} rflags={:#x}",
                self.id,
                self.context.rip,
                self.context.rsp,
                self.context.rbp,
                self.context.rflags
            );
        }
        self.context
    }

    pub fn print_info_banner(&self, event: &str) {
        use core::fmt::Write;

        crate::serial::with_serial(|serial| {
            let _ = write!(serial, "{}", TaskRecord { task: self, event });
        });
    }

    /// Full detail, gated by the runtime debug flag.
    pub fn print_info(&self) {
        if task_debug_enabled() {
            self.print_info_banner("info");
        }
    }

    pub fn dump_all(tasks: &[Task]) {
        if !task_debug_enabled() {
            return;
        }
        use core::fmt::Write;

        crate::serial::with_serial(|serial| {
            let _ = write!(serial, "{}", TaskTable(tasks));
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatters
// ─────────────────────────────────────────────────────────────────────────────

use core::fmt;

/// One-line summary for `Task`.
struct TaskLine<'a>(&'a Task);

impl fmt::Display for TaskLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = self.0;
        write!(
            f,
            "task id={:<3} state={} ring={} user={} prio={:<3} started={} entry={:#010x} sp={:#010x}",
            t.id,
            t.state.short(),
            t.ring,
            t.is_user_task() as u8,
            t.priority,
            t.started as u8,
            t.entry_point,
            t.stack_top,
        )
    }
}

/// Full multi-line record for `Task`.
struct TaskRecord<'a> {
    task: &'a Task,
    event: &'a str,
}

impl fmt::Display for TaskRecord<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = self.task;
        writeln!(
            f,
            "┌─ task {} [{}] ─────────────────────────────",
            t.id, self.event
        )?;
        writeln!(f, "│ id                  : {}", t.id)?;
        writeln!(
            f,
            "│ state               : {} ({:?})",
            t.state.name(),
            t.state
        )?;
        writeln!(f, "│ ring                : {}", t.ring)?;
        writeln!(f, "│ is_user_task        : {}", t.is_user_task())?;
        writeln!(f, "│ priority            : {}", t.priority)?;
        writeln!(f, "│ started             : {}", t.started)?;
        writeln!(f, "│ entry_point         : {:#018x}", t.entry_point)?;
        writeln!(f, "│ stack_top           : {:#018x}", t.stack_top)?;
        writeln!(f, "│ kernel_stack_top    : {:#018x}", t.kernel_stack_top)?;
        writeln!(
            f,
            "│ kernel_stack_ptr    : {:#018x}",
            t.kernel_stack_pointer
        )?;
        writeln!(f, "│ context.rip         : {:#018x}", t.context.rip)?;
        writeln!(f, "│ context.rsp         : {:#018x}", t.context.rsp)?;
        writeln!(f, "│ context.rbp         : {:#018x}", t.context.rbp)?;
        writeln!(f, "│ context.rflags      : {:#018x}", t.context.rflags)?;
        writeln!(f, "│ context.cs          : {:#018x}", t.context.cs)?;
        writeln!(f, "│ context.ss          : {:#018x}", t.context.ss)?;
        writeln!(
            f,
            "│ affinity            : {:?} ({:#b})",
            t.affinity, t.affinity
        )?;
        writeln!(f, "└──────────────────────────────────────────────")
    }
}

/// Table formatter for `[Task]`.
struct TaskTable<'a>(&'a [Task]);

impl fmt::Display for TaskTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "id  st ring u prio started entry        stack        rip"
        )?;
        writeln!(
            f,
            "──  ── ──── ─ ──── ─────── ──────────── ──────────── ────────────"
        )?;
        for t in self.0 {
            writeln!(
                f,
                "{:<3} {}  {:<4} {} {:<4} {:<7} {:#012x} {:#012x} {:#012x}",
                t.id,
                t.state.short(),
                t.ring,
                t.is_user_task() as u8,
                t.priority,
                t.started as u8,
                t.entry_point,
                t.stack_top,
                t.context.rip,
            )?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Lifecycle helpers (call these from the scheduler)
// ─────────────────────────────────────────────────────────────────────────────

/// Call right before switching **into** `next`.
pub fn trace_switch(prev: Option<u32>, next: &Task) {
    if task_debug_enabled() {
        crate::serial_println!(
            "[sched] {} -> task {} (rip={:#x} rsp={:#x} ring={})",
            prev.map(|p| p).map(|p| p as i64).unwrap_or(-1),
            next.id,
            next.context.rip,
            next.context.rsp,
            next.ring,
        );
    }
}

/// Call when a task starts for the first time.
pub fn trace_started(task: &Task) {
    if task_debug_enabled() {
        task.print_info_banner("started");
    }
}

/// Call when a task exits / becomes zombie.
pub fn trace_exit(task: &Task) {
    if task_debug_enabled() {
        task.print_info_banner("exited");
    }
}
