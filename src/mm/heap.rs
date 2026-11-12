use kelvos_shared_ring::layout::{HEAP_SIZE, HEAP_START};
use linked_list_allocator::LockedHeap;
use x86_64::VirtAddr;

use crate::mm::{frame::FrameAllocator, paging::PagingManager};

use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub fn init(mapper: &mut PagingManager, alloc: &mut FrameAllocator) {
    unsafe {
        mapper
            .map_range(VirtAddr::new(HEAP_START), HEAP_SIZE, alloc)
            .expect("heap mapping failed");
    }
    unsafe {
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }
}

pub fn validate_allocator() {
    {
        let boxed = Box::new(42_u64);
        let mut values = Vec::with_capacity(3);
        values.extend_from_slice(&[1_u64, 2, 3]);
        let mut index = BTreeMap::new();
        index.insert(7_u64, 49_u64);

        if *boxed != 42 || values.as_slice() != [1, 2, 3] || index.get(&7) != Some(&49) {
            panic!("kernel heap allocator validation failed");
        }

        core::hint::black_box((&boxed, &values, &index));
    }

    let reused = Box::new(84_u64);
    if *reused != 84 {
        panic!("kernel heap allocator reuse validation failed");
    }
    core::hint::black_box(&reused);
}
