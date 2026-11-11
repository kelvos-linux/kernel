use core::fmt::Write;
use spin::{Mutex, Once};
use uart_16550::SerialPort;

static SERIAL1: Once<Mutex<SerialPort>> = Once::new();

pub fn init() {
    SERIAL1.call_once(|| {
        let mut serial = unsafe { SerialPort::new(0x3F8) };
        serial.init();
        Mutex::new(serial)
    });
}

pub fn _print(args: core::fmt::Arguments) {
    if let Some(serial) = SERIAL1.get() {
        let _ = serial.lock().write_fmt(args);
    }
}


pub fn with_serial<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SerialPort) -> R,
{
    SERIAL1.get().map(|serial| {
        let mut guard = serial.lock();
        f(&mut guard)
    })
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! serial_println {
    () => {
        $crate::serial_print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::serial_print!("{}\n", format_args!($($arg)*))
    };
}