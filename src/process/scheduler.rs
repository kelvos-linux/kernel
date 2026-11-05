use alloc::collections::{BTreeMap, VecDeque};

use crate::process::task::{Task, TaskState};

const TASK_KERNEL_STACK_SIZE: usize = 32 * 1024;

#[repr(align(16))]
struct TaskKernelStack([u8; TASK_KERNEL_STACK_SIZE]);

static mut CHILD_KERNEL_STACK: TaskKernelStack = TaskKernelStack([0; TASK_KERNEL_STACK_SIZE]);

#[derive(Default)]
pub struct Scheduler {
    ready_queue: VecDeque<Task>,
    sleeping: BTreeMap<u32, Task>,
    current: Option<Task>,
    next_id: u32,
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
        self.enqueue(task);
        task
    }

    pub fn create_user_task(&mut self, stack_top: usize, entry_point: usize) -> Option<Task> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        let mut task = Task::with_entry(id, stack_top, entry_point, 3);
        task.prepare_for_user_mode(entry_point, stack_top);
        let kernel_stack_top =
            core::ptr::addr_of_mut!(CHILD_KERNEL_STACK) as usize + TASK_KERNEL_STACK_SIZE;
        task.initialize_kernel_stack(kernel_stack_top);
        self.enqueue(task);
        Some(task)
    }

    pub fn schedule(&mut self) -> Option<Task> {
        let task = self.ready_queue.pop_front()?;
        let mut next = task;
        next.state = TaskState::Runnable;
        self.current = Some(next);
        self.current
    }

    pub fn switch_context(&mut self, from: &mut Task, mut to: Task) {
        from.state = TaskState::Sleeping;
        to.state = TaskState::Runnable;
        self.current = Some(to);
    }

    pub fn prepare_switch(&mut self, next: Task) -> Option<Task> {
        let current = self.current.take()?;
        let mut next_task = next;
        next_task.state = TaskState::Runnable;
        self.current = Some(next_task);
        Some(current)
    }

    pub fn prepare_user_switch(
        &mut self,
        next: Task,
    ) -> Option<(Task, crate::arch::x86_64::gdt::UserModeFrame)> {
        let current = self.current.take()?;
        let mut next_task = next;
        next_task.prepare_for_user_mode(next_task.entry_point, next_task.stack_top);
        let frame = next_task.user_frame();
        next_task.state = TaskState::Runnable;
        self.current = Some(next_task);
        Some((current, frame))
    }

    pub fn bootstrap_user_task(
        &mut self,
        stack_top: usize,
        entry_point: usize,
    ) -> Option<crate::arch::x86_64::gdt::UserModeFrame> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1)?;
        let mut task = Task::with_entry(id, stack_top, entry_point, 3);
        task.prepare_for_user_mode(entry_point, stack_top);
        let frame = task.user_frame();
        task.kernel_stack_top = crate::arch::x86_64::gdt::kernel_stack_top();
        task.started = true;
        task.state = TaskState::Runnable;
        self.current = Some(task);
        Some(frame)
    }

    pub fn sleep_task(&mut self, task_id: u32) {
        if let Some(task) = self.current.take() {
            if task.id == task_id {
                self.sleeping.insert(task_id, task);
            } else {
                self.current = Some(task);
            }
        }
    }

    pub fn current_task(&self) -> Option<Task> {
        self.current
    }

    pub fn tick(&mut self) -> Option<Task> {
        let current = self.current.take()?;
        self.ready_queue.push_back(current);
        let mut next = self.ready_queue.pop_front()?;
        next.state = TaskState::Runnable;
        self.current = Some(next);
        self.current
    }

    pub fn switch_from_interrupt(
        &mut self,
        current_kernel_stack_pointer: usize,
    ) -> Option<(usize, usize, Option<u32>)> {
        let mut current = self.current.take()?;
        current.kernel_stack_pointer = current_kernel_stack_pointer;
        current.state = TaskState::Runnable;
        self.ready_queue.push_back(current);

        let mut next = self.ready_queue.pop_front()?;
        next.state = TaskState::Runnable;
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
        self.current = Some(next);
        Some(target)
    }
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
