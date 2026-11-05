use core::sync::atomic::{AtomicU32, Ordering};
use kelvos_shared_ring::{Completion, SharedRing, OP_SPAWN, RING_CAPACITY};

use crate::{process::scheduler::SCHEDULER, serial_println};

static SPAWNED_CHILDREN: AtomicU32 = AtomicU32::new(0);

pub fn sys_ring_notify() {
    let ring = unsafe { &*(kelvos_shared_ring::USER_RING_ADDRESS as *const SharedRing) };
    let mut sq_head = ring.sq_head.load(Ordering::Relaxed);
    let sq_tail = ring.sq_tail.load(Ordering::Acquire);
    let pending = sq_tail.wrapping_sub(sq_head);

    if pending > RING_CAPACITY {
        serial_println!("[RING] rejected invalid submission queue indices");
        return;
    }

    while sq_head != sq_tail {
        let cq_head = ring.cq_head.load(Ordering::Acquire);
        let cq_tail = ring.cq_tail.load(Ordering::Relaxed);
        if cq_tail.wrapping_sub(cq_head) >= RING_CAPACITY {
            break;
        }

        let submission = unsafe { ring.read_submission(sq_head) };
        let result = if submission.opcode != OP_SPAWN
            || submission.flags != 0
            || submission.entry_point != kelvos_shared_ring::USER_IMAGE_ADDRESS
            || submission.stack_top != kelvos_shared_ring::SPAWN_STACK_TOP
        {
            -22
        } else if SPAWNED_CHILDREN
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            -11
        } else {
            let child = SCHEDULER.lock().create_user_task(
                submission.stack_top as usize,
                submission.entry_point as usize,
            );
            if let Some(child) = child {
                serial_println!("[RING] SPAWN queued task {}", child.id);
                i64::from(child.id)
            } else {
                -11
            }
        };

        unsafe {
            ring.write_completion(
                cq_tail,
                Completion {
                    user_data: u64::from(sq_head),
                    result,
                },
            );
        }
        ring.cq_tail
            .store(cq_tail.wrapping_add(1), Ordering::Release);

        sq_head = sq_head.wrapping_add(1);
        ring.sq_head.store(sq_head, Ordering::Release);
    }
}
