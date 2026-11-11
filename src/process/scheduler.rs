use alloc::collections::{BTreeMap, VecDeque};

use crate::{
    cpu::{
        affinity::CpuMask,
        state::{CpuState, CPU_STATES},
        topology::MAX_CPUS,
    },
    process::task::{task_debug_enabled, Task, TaskState},
};

const TASK_KERNEL_STACK_SIZE: usize = 32 * 1024;

#[repr(align(16))]
#[derive(Clone, Copy)]
struct TaskKernelStack([u8; TASK_KERNEL_STACK_SIZE]);

static mut TASK_KERNEL_STACKS: [TaskKernelStack; kelvos_shared_ring::MAX_TASKS] =
    [TaskKernelStack([0; TASK_KERNEL_STACK_SIZE]); kelvos_shared_ring::MAX_TASKS];

pub struct Scheduler {
    ready_queue: VecDeque<Task>,
    sleeping: BTreeMap<u32, Task>,
    current: [Option<Task>; MAX_CPUS],
    next_id: u32,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            ready_queue: VecDeque::new(),
            sleeping: BTreeMap::new(),
            current: [None; MAX_CPUS],
            next_id: 0,
        }
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, task: Task) {
        self.ready_queue.push_back(task);
    }

    pub fn create_task(&mut self, stack_top: usize) -> Task {
        let id = self.next_id;
        self.next_id += 1;
        let task = Task::new(id, stack_top);
        if task_debug_enabled() {
            task.print_info_banner("created");
        }
        self.enqueue(task);
        task
    }

    pub fn create_task_with_entry(
        &mut self,
        stack_top: usize,
        entry_point: usize,
        ring: u8,
    ) -> Task {
        let id = self.next_id;
        self.next_id += 1;
        let task = Task::with_entry(id, stack_top, entry_point, ring);
        if ring != 3 && task_debug_enabled() {
            task.print_info_banner("created");
        }
        self.enqueue(task);
        task
    }

    pub fn create_user_task(&mut self, stack_top: usize, entry_point: usize) -> Option<Task> {
        let id = self.next_id;
        if id == 0
            || id as usize >= kelvos_shared_ring::MAX_TASKS
            || stack_top as u64 != kelvos_shared_ring::SPAWN_STACK_TOP
        {
            return None;
        }
        let next_id = self.next_id.checked_add(1)?;
        let task_stack_top = kelvos_shared_ring::SPAWN_STACK_TOP
            + u64::from(id - 1) * kelvos_shared_ring::USER_STACK_SIZE as u64;
        let mut task = Task::with_entry(id, task_stack_top as usize, entry_point, 3);
        task.prepare_for_user_mode(entry_point, task_stack_top as usize);
        let kernel_stack_top = task_kernel_stack_top(id)?;
        task.initialize_kernel_stack(kernel_stack_top);
        if task_debug_enabled() {
            task.print_info_banner("created");
        }
        self.next_id = next_id;
        self.enqueue(task);
        Some(task)
    }

    pub fn schedule(&mut self) -> Option<Task> {
        let cpu_id = current_cpu_id();
        let task = self.pop_ready_for_cpu(cpu_id)?;
        self.current[cpu_id] = Some(task);
        self.current[cpu_id]
    }

    pub fn switch_context(&mut self, from: &mut Task, mut to: Task) {
        let cpu_id = current_cpu_id();
        from.state = TaskState::Sleeping;
        to.state = TaskState::Runnable;
        self.current[cpu_id] = Some(to);
    }

    pub fn prepare_switch(&mut self, next: Task) -> Option<Task> {
        let cpu_id = current_cpu_id();
        let current = self.current[cpu_id].take()?;
        let mut next_task = next;
        next_task.state = TaskState::Runnable;
        self.current[cpu_id] = Some(next_task);
        Some(current)
    }

    pub fn prepare_user_switch(
        &mut self,
        next: Task,
    ) -> Option<(Task, crate::arch::x86_64::gdt::UserModeFrame)> {
        let cpu_id = current_cpu_id();
        let current = self.current[cpu_id].take()?;
        let mut next_task = next;
        next_task.prepare_for_user_mode(next_task.entry_point, next_task.stack_top);
        let frame = next_task.user_frame();
        next_task.state = TaskState::Runnable;
        self.current[cpu_id] = Some(next_task);
        Some((current, frame))
    }

    pub fn bootstrap_user_task(
        &mut self,
        stack_top: usize,
        entry_point: usize,
    ) -> Option<crate::arch::x86_64::gdt::UserModeFrame> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        let kernel_stack_top = task_kernel_stack_top(id)?;
        let mut task = Task::with_entry(id, stack_top, entry_point, 3);
        task.prepare_for_user_mode(entry_point, stack_top);
        task.initialize_kernel_stack(kernel_stack_top);
        task.affinity = CpuMask::empty();
        task.affinity.set(0, true);
        let frame = task.user_frame();
        task.started = true;
        task.state = TaskState::Runnable;
        crate::arch::x86_64::gdt::set_kernel_stack_top_for_cpu(0, kernel_stack_top);
        if task_debug_enabled() {
            task.print_info_banner("created");
        }
        self.current[0] = Some(task);
        Some(frame)
    }

    pub fn sleep_task(&mut self, task_id: u32) {
        let cpu_id = current_cpu_id();
        if let Some(task) = self.current[cpu_id].take() {
            if task.id == task_id {
                self.sleeping.insert(task_id, task);
            } else {
                self.current[cpu_id] = Some(task);
            }
        }
    }

    pub fn current_task(&self) -> Option<Task> {
        self.current[current_cpu_id()]
    }

    pub fn tick(&mut self) -> Option<Task> {
        let cpu_id = current_cpu_id();
        let current = self.current[cpu_id].take()?;
        self.ready_queue.push_back(current);
        let next = self.pop_ready_for_cpu(cpu_id)?;
        self.current[cpu_id] = Some(next);
        self.current[cpu_id]
    }

    pub fn switch_from_interrupt(
        &mut self,
        cpu_id: usize,
        current_kernel_stack_pointer: usize,
    ) -> Option<(usize, usize, Option<u32>)> {
        if cpu_id >= MAX_CPUS || CPU_STATES[cpu_id].get() != CpuState::Online {
            return None;
        }

        if let Some(mut current) = self.current[cpu_id].take() {
            current.kernel_stack_pointer = current_kernel_stack_pointer;
            current.state = TaskState::Runnable;
            self.ready_queue.push_back(current);
        }

        let mut next = self.pop_ready_for_cpu(cpu_id)?;
        let first_start = if next.started {
            None
        } else {
            next.started = true;
            Some(next.id)
        };
        let target = (
            next.kernel_stack_pointer,
            next.kernel_stack_top,
            first_start,
        );
        self.current[cpu_id] = Some(next);
        if let (Some(pc), Some(task)) = (
            crate::arch::x86_64::percpu::for_cpu(cpu_id),
            self.current[cpu_id].as_mut(),
        ) {
            pc.current_task
                .store(task as *mut Task, core::sync::atomic::Ordering::Release);
        }
        Some(target)
    }

    fn pop_ready_for_cpu(&mut self, cpu_id: usize) -> Option<Task> {
        if cpu_id >= MAX_CPUS || CPU_STATES[cpu_id].get() != CpuState::Online {
            return None;
        }
        let index = self
            .ready_queue
            .iter()
            .position(|task| task.affinity.contains(cpu_id))?;
        self.ready_queue.remove(index)
    }
}

fn current_cpu_id() -> usize {
    crate::arch::x86_64::percpu::try_this_cpu().map_or(0, |pc| {
        pc.cpu_id.load(core::sync::atomic::Ordering::Relaxed) as usize
    })
}

fn task_kernel_stack_top(task_id: u32) -> Option<usize> {
    let index = task_id as usize;
    if index >= kelvos_shared_ring::MAX_TASKS {
        return None;
    }

    let stacks = core::ptr::addr_of_mut!(TASK_KERNEL_STACKS).cast::<TaskKernelStack>();
    Some(unsafe { stacks.add(index) as usize + TASK_KERNEL_STACK_SIZE })
}

pub static SCHEDULER: spin::Lazy<spin::Mutex<Scheduler>> =
    spin::Lazy::new(|| spin::Mutex::new(Scheduler::new()));

pub fn init() {
    *SCHEDULER.lock() = Scheduler::new();
}

pub fn on_timer_tick() {
    let mut scheduler = SCHEDULER.lock();
    let _ = scheduler.tick();
}
