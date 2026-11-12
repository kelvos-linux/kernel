use alloc::boxed::Box;
use core::sync::atomic::Ordering;
use kelvos_shared_ring::{ExitStatus, Pid, Tid, BOOT_CHILD_IMAGE, MAX_TASKS};
use x86_64::{
    registers::control::Cr3,
    structures::paging::{PhysFrame, Size4KiB},
};

use crate::{
    arch::x86_64::idt::UserInterruptFrame,
    arch::x86_64::{apic, gdt, percpu},
    cpu::{
        state::{CpuState, CPU_STATES},
        topology::{MAX_CPUS, TOPOLOGY},
    },
    process::{image::LoadedImage, thread::Thread},
    serial_println,
};

const TASK_KERNEL_STACK_SIZE: usize = 32 * 1024;

#[repr(align(16))]
struct KernelStack([u8; TASK_KERNEL_STACK_SIZE]);

static mut KERNEL_STACKS: [KernelStack; MAX_TASKS] =
    [const { KernelStack([0; TASK_KERNEL_STACK_SIZE]) }; MAX_TASKS];

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ready,
    Running,
    Waiting(Pid),
    Retiring,
}

struct Process {
    id: Pid,
    image: LoadedImage,
}

#[derive(Clone, Copy)]
struct Child {
    pid: Pid,
    parent: Pid,
    status: Option<ExitStatus>,
}

pub enum Switch {
    Preempt,
    Wait(Pid),
    Exit(ExitStatus),
}

// Each process has one schedulable thread.
struct Slot {
    process: Process,
    thread: Thread,
    state: State,
    cpu: usize,
    started: bool,
}

#[derive(Clone, Copy, Default)]
struct CpuSchedule {
    running: Option<usize>,
    idle_frame: usize,
    cursor: usize,
    retiring: Option<(usize, ExitStatus)>,
}

pub struct Scheduler {
    slots: Box<[Option<Slot>]>,
    reserved_images: [Option<LoadedImage>; MAX_TASKS],
    children: [Option<Child>; MAX_TASKS - 1],
    next_pid: u32,
    next_tid: u32,
    cpus: [CpuSchedule; MAX_CPUS],
    kernel_root: PhysFrame<Size4KiB>,
}

pub static SCHEDULER: spin::Once<spin::Mutex<Scheduler>> = spin::Once::new();

pub fn init(images: [LoadedImage; MAX_TASKS]) {
    SCHEDULER.call_once(|| {
        let bsp = TOPOLOGY.get().unwrap().bsp_index;
        let mut scheduler = Scheduler {
            // Build the thread table on the heap. Its saved contexts would
            // otherwise be copied through the limited bootstrap stack.
            slots: core::iter::repeat_with(|| None).take(MAX_TASKS).collect(),
            reserved_images: images.map(Some),
            cpus: [CpuSchedule::default(); MAX_CPUS],
            kernel_root: Cr3::read().0,
            children: [None; MAX_TASKS - 1],
            next_pid: Pid::INIT.get(),
            next_tid: 1,
        };
        assert_eq!(scheduler.start_process(0, bsp, 0), Ok(Pid::INIT));
        spin::Mutex::new(scheduler)
    });
}

impl Scheduler {
    pub fn current_ids(&self, cpu: usize) -> Option<(Pid, Tid)> {
        let index = self.cpus[cpu].running?;
        let slot = self.slots[index].as_ref()?;
        Some((slot.thread.process, slot.thread.id))
    }

    fn start_process(&mut self, index: usize, cpu: usize, argument: u64) -> Result<Pid, i64> {
        assert!(self.slots[index].is_none());
        let pid = Pid::from_raw(self.next_pid).ok_or(-11)?;
        let tid = Tid::from_raw(self.next_tid).ok_or(-11)?;
        let next_pid = self.next_pid.checked_add(1).ok_or(-11)?;
        let next_tid = self.next_tid.checked_add(1).ok_or(-11)?;
        let image = self.reserved_images[index].take().ok_or(-11)?;
        let stack = unsafe { (&raw mut KERNEL_STACKS).cast::<KernelStack>().add(index) };
        let stack_top = stack as usize + TASK_KERNEL_STACK_SIZE;
        // The reservation guarantees an inactive image and an exclusive kernel stack.
        let thread =
            unsafe { Thread::new(tid, pid, image.entry, image.stack_top, stack_top, argument) };
        self.slots[index] = Some(Slot {
            process: Process { id: pid, image },
            thread,
            state: State::Ready,
            cpu,
            started: false,
        });
        self.next_pid = next_pid;
        self.next_tid = next_tid;
        Ok(pid)
    }

    pub fn spawn(&mut self, cpu: usize, image: u64, argument: u64) -> Result<(Pid, usize), i64> {
        // PID 1 owns the boot image grant.
        let (parent, _) = self.current_ids(cpu).ok_or(-3)?;
        if parent != Pid::INIT {
            return Err(-1);
        }
        if image != BOOT_CHILD_IMAGE {
            return Err(-2);
        }
        let record = self.children.iter().position(Option::is_none).ok_or(-11)?;
        let index = self
            .reserved_images
            .iter()
            .position(Option::is_some)
            .ok_or(-11)?;
        let count = TOPOLOGY.get().unwrap().cpu_count;
        let preferred = index % count;
        let target = if CPU_STATES[preferred].get() == CpuState::Online {
            preferred
        } else {
            cpu
        };
        let pid = self.start_process(index, target, argument)?;
        self.children[record] = Some(Child {
            pid,
            parent,
            status: None,
        });
        Ok((pid, target))
    }

    pub fn wait_child(
        &mut self,
        cpu: usize,
        stack: usize,
        pid: Pid,
        no_hang: bool,
    ) -> Result<usize, i64> {
        let (caller, _) = self.current_ids(cpu).ok_or(-3)?;
        let index = self
            .children
            .iter()
            .position(|record| {
                record.is_some_and(|child| child.pid == pid && child.parent == caller)
            })
            .ok_or(-10)?;
        if let Some(status) = self.children[index].unwrap().status {
            self.children[index] = None;
            // The syscall stub owns this frame and the thread is still running.
            unsafe { (*(stack as *mut UserInterruptFrame)).registers.rax = status.encode() };
            report_wait(caller, pid, status);
            return Ok(stack);
        }
        if no_hang {
            return Err(-11);
        }

        serial_println!("[WAIT] PID {} waiting for PID {}", caller.get(), pid.get());
        // The check and transition to Waiting share the same lock as retirement.
        Ok(self.switch(cpu, stack, Switch::Wait(pid)))
    }

    fn finish_retirement(&mut self, index: usize, process: Process, status: ExitStatus) {
        let pid = process.id;
        self.reserved_images[index] = Some(process.image);
        let record = self
            .children
            .iter()
            .position(|child| child.is_some_and(|child| child.pid == pid))
            .expect("retired process missing child record");
        let child = self.children[record].as_mut().unwrap();
        let parent = child.parent;
        if let Some(waiter) = self
            .slots
            .iter_mut()
            .flatten()
            .find(|slot| slot.process.id == parent && slot.state == State::Waiting(pid))
        {
            // The blocked thread's saved frame belongs to its pinned CPU.
            // Any in-progress switch will leave the frame before it can resume.
            unsafe { waiter.thread.complete_wait(status.encode()) };
            waiter.state = State::Ready;
            self.children[record] = None;
            report_wait(parent, pid, status);
        } else {
            child.status = Some(status);
        }
    }

    fn switch(&mut self, cpu: usize, frame: usize, action: Switch) -> usize {
        let local = &mut self.cpus[cpu];
        if let Some(id) = local.running.take() {
            let slot = self.slots[id].as_mut().unwrap();
            assert!(slot.cpu == cpu && slot.state == State::Running);
            match action {
                Switch::Exit(status) => {
                    assert!(local.retiring.is_none());
                    slot.state = State::Retiring;
                    local.retiring = Some((id, status));
                }
                Switch::Preempt | Switch::Wait(_) => {
                    slot.thread.save(frame);
                    slot.state = match action {
                        Switch::Wait(child) => State::Waiting(child),
                        _ => State::Ready,
                    };
                }
            }
        } else {
            assert!(matches!(action, Switch::Preempt));
            local.idle_frame = frame;
        }

        let next = (0..MAX_TASKS)
            .map(|offset| (local.cursor + offset) % MAX_TASKS)
            .find(|&id| {
                self.slots[id]
                    .as_ref()
                    .is_some_and(|slot| slot.state == State::Ready && slot.cpu == cpu)
            });
        let Some(id) = next else {
            let pc = percpu::for_cpu(cpu).unwrap();
            activate(
                self.kernel_root,
                cpu,
                pc.idle_stack.load(Ordering::Relaxed) as usize,
            );
            return local.idle_frame;
        };

        local.cursor = (id + 1) % MAX_TASKS;
        local.running = Some(id);
        let slot = self.slots[id].as_mut().unwrap();
        slot.state = State::Running;
        if !slot.started {
            slot.started = true;
            serial_println!(
                "[SCHED] CPU {} starting PID {} TID {}",
                cpu,
                slot.process.id.get(),
                slot.thread.id.get()
            );
        }
        activate(slot.process.image.root, cpu, slot.thread.stack_top);
        slot.thread.restore()
    }
}

fn activate(root: PhysFrame<Size4KiB>, cpu: usize, kernel_stack_top: usize) {
    if Cr3::read().0 != root {
        // Every root maps the active kernel stack and kernel code identically.
        unsafe { Cr3::write(root, x86_64::registers::control::Cr3Flags::empty()) };
    }
    gdt::set_kernel_stack_top_for_cpu(cpu, kernel_stack_top);
}

fn report_wait(parent: Pid, child: Pid, status: ExitStatus) {
    serial_println!(
        "[WAIT] PID {} collected PID {}: {:?}",
        parent.get(),
        child.get(),
        status
    );
}

fn reap_previous(cpu: usize) {
    let scheduler = SCHEDULER.get().unwrap();
    let (index, status, slot) = {
        let mut state = scheduler.lock();
        let Some((index, status)) = state.cpus[cpu].retiring.take() else {
            return;
        };
        (index, status, state.slots[index].take().unwrap())
    };

    // This CPU has completed the previous switch. The thread was pinned, and
    // switching CR3 without PCID retired its non-global user translations.
    // No CPU can launch this slot until its image reservation is returned.
    unsafe {
        slot.process.image.reset();
        let stack = (slot.thread.stack_top - TASK_KERNEL_STACK_SIZE) as *mut u8;
        stack.write_bytes(0, TASK_KERNEL_STACK_SIZE);
    }
    scheduler
        .lock()
        .finish_retirement(index, slot.process, status);
}

pub fn switch_from_interrupt(frame: usize, action: Switch) -> usize {
    let cpu = percpu::this_cpu().id() as usize;
    reap_previous(cpu);
    let mut scheduler = SCHEDULER.get().unwrap().lock();
    if let Switch::Exit(status) = action {
        if scheduler
            .current_ids(cpu)
            .is_some_and(|(pid, _)| pid == Pid::INIT)
        {
            drop(scheduler);
            let topology = TOPOLOGY.get().unwrap();
            for (index, &apic_id) in topology.apic_ids().iter().enumerate() {
                if index != cpu && CPU_STATES[index].get() == CpuState::Online {
                    let _ = apic::send_halt(apic_id);
                }
            }
            panic!("PID 1 terminated: {status:?}");
        }
    }
    scheduler.switch(cpu, frame, action)
}

pub fn terminate_current(status: ExitStatus) -> ! {
    let stack = switch_from_interrupt(0, Switch::Exit(status));
    unsafe extern "C" {
        fn resume_interrupt_frame(stack: usize) -> !;
    }
    // The selected frame resides on an owned task stack or this CPU's idle stack.
    unsafe { resume_interrupt_frame(stack) }
}
