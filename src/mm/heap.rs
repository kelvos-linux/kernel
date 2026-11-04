use linked_list_allocator::LockedHeap;
use x86_64::VirtAddr;

use crate::mm::{frame::FrameAllocator, paging::PagingManager};

pub const HEAP_START: usize = 0xFFFF_FFFF_C000_0000;
pub const HEAP_SIZE: usize = 16 * 1024 * 1024;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub fn init(mapper: &mut PagingManager, alloc: &mut FrameAllocator) {
    unsafe {
        mapper
            .map_range(VirtAddr::new(HEAP_START as u64), HEAP_SIZE, alloc)
            .expect("heap mapping failed");
    }
    unsafe {
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }
}

pub fn init_default() {
    let mut frames = FrameAllocator::new_default();
    let mut paging = unsafe { crate::mm::paging::PagingManager::new() };
    init(&mut paging, &mut frames);
}
