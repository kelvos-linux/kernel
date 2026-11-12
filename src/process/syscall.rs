use kelvos_shared_ring::{
    ExitStatus, Pid, SYS_EXIT, SYS_GET_PID, SYS_GET_TID, SYS_SPAWN, SYS_WAIT_CHILD, WAIT_NO_HANG,
};

use crate::{
    arch::x86_64::{idt::UserInterruptFrame, percpu},
    cpu::smp,
    serial_println,
};

use super::scheduler::{self, Switch, SCHEDULER};

#[unsafe(no_mangle)]
extern "C" fn dispatch_syscall(frame: *mut UserInterruptFrame) -> usize {
    // The assembly entry created this frame on the current thread's stack.
    let (number, arg, extra) = unsafe {
        let frame = &*frame;
        frame.validate_return();
        (
            frame.registers.rax,
            frame.registers.rdi,
            frame.registers.rsi,
        )
    };
    let stack = frame as usize;
    let cpu = percpu::this_cpu().id() as usize;
    let scheduler = SCHEDULER.get().unwrap();
    let result = match number {
        SYS_SPAWN => {
            let result = scheduler.lock().spawn(cpu, arg, extra);
            match result {
                Ok((pid, target)) => {
                    serial_println!("[SYSCALL] SPAWN queued PID {}", pid.get());
                    if target != cpu {
                        let _ = smp::send_reschedule(target);
                    }
                    i64::from(pid.get())
                }
                Err(error) => error,
            }
        }
        SYS_GET_PID | SYS_GET_TID => scheduler.lock().current_ids(cpu).map_or(-3, |(pid, tid)| {
            i64::from(if number == SYS_GET_PID {
                pid.get()
            } else {
                tid.get()
            })
        }),
        SYS_EXIT => {
            return scheduler::switch_from_interrupt(
                stack,
                Switch::Exit(ExitStatus::Exited(arg as u32)),
            );
        }
        SYS_WAIT_CHILD if extra & !WAIT_NO_HANG == 0 => {
            match u32::try_from(arg).ok().and_then(Pid::from_raw) {
                Some(pid) => {
                    match scheduler
                        .lock()
                        .wait_child(cpu, stack, pid, extra == WAIT_NO_HANG)
                    {
                        Ok(next) => return next,
                        Err(error) => error,
                    }
                }
                None => -22,
            }
        }
        SYS_WAIT_CHILD => -22,
        _ => -38,
    };
    // Blocking operations return above and leave completion to the scheduler.
    unsafe { (*frame).registers.rax = result as u64 };
    stack
}
