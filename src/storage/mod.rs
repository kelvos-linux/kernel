pub mod async_io;
pub mod driver_service;
pub mod fs;
pub mod storage;

pub fn init() {
    storage::init();
}
