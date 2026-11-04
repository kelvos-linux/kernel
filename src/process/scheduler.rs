use alloc::collections::{BTreeMap, VecDeque};

use crate::process::task::{Task, TaskState};

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

    pub fn create_task_with_entry(&mut self, stack_top: usize, entry_point: usize, ring: u8) -> Task {
        let id = self.next_id;
        self.next_id += 1;
        let task = Task::with_entry(id, stack_top, entry_point, ring);
        self.enqueue(task);
        task
    }

    pub fn create_user_task(&mut self, stack_top: usize, entry_point: usize) -> Task {
        self.create_task_with_entry(stack_top, entry_point, 3)
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

    pub fn prepare_user_switch(&mut self, next: Task) -> Option<(Task, crate::arch::x86_64::gdt::UserModeFrame)> {
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
        let mut task = self.create_user_task(stack_top, entry_point);
        task.prepare_for_user_mode(entry_point, stack_top);
        let frame = task.user_frame();
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
        let mut current = self.current.take()?;
        current.state = TaskState::Sleeping;

        if let Some(next) = self.ready_queue.pop_front() {
            let mut next_task = next;
            next_task.state = TaskState::Runnable;
            self.current = Some(next_task);
            self.ready_queue.push_back(current);
            return self.current;
        }

        self.ready_queue.push_back(current);
        self.current = self.ready_queue.pop_front();
        self.current
    }
}

pub static SCHEDULER: spin::Lazy<spin::Mutex<Scheduler>> = spin::Lazy::new(|| spin::Mutex::new(Scheduler::new()));

pub fn init() {
    let mut scheduler = SCHEDULER.lock();
    scheduler.create_task_with_entry(0x1000, 0x1000, 0);
    let _user_frame = scheduler.bootstrap_user_task(0x2000, 0x0000_0000_1000_0000);
    scheduler.current = scheduler.ready_queue.pop_front();
}

pub fn on_timer_tick() {
    let mut scheduler = SCHEDULER.lock();
    let _ = scheduler.tick();
}
