use alloc::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoPriority {
    Low,
    Normal,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoOperation {
    Read,
    Write,
    Flush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsyncRequest {
    pub request_id: u64,
    pub device_id: u32,
    pub operation: IoOperation,
    pub sector: u64,
    pub length: usize,
    pub priority: IoPriority,
}

#[derive(Clone, Debug, Default)]
pub struct AsyncIoQueue {
    queue: VecDeque<AsyncRequest>,
    next_id: u64,
}

impl AsyncIoQueue {
    pub fn submit(
        &mut self,
        device_id: u32,
        operation: IoOperation,
        sector: u64,
        length: usize,
        priority: IoPriority,
    ) {
        self.queue.push_back(AsyncRequest {
            request_id: self.next_id,
            device_id,
            operation,
            sector,
            length,
            priority,
        });
        self.next_id += 1;
    }

    pub fn poll(&mut self) -> Option<AsyncRequest> {
        self.queue.pop_front()
    }
}

pub static ASYNC_IO: spin::Lazy<spin::Mutex<AsyncIoQueue>> =
    spin::Lazy::new(|| spin::Mutex::new(AsyncIoQueue::default()));

pub fn init() {
    let mut io = ASYNC_IO.lock();
    io.submit(0, IoOperation::Read, 0, 4096, IoPriority::Normal);
    io.submit(0, IoOperation::Flush, 1, 0, IoPriority::High);
}
