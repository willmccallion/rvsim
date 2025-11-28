use std::io::{self, Read, Write};

pub struct Uart;

impl Uart {
    pub fn new() -> Self {
        Self
    }

    pub fn read_u8(&self, _addr: u64) -> u8 {
        let mut buf = [0u8; 1];
        match io::stdin().read(&mut buf) {
            Ok(1) => buf[0],
            _ => 0,
        }
    }

    pub fn write_u8(&mut self, _addr: u64, val: u8) {
        print!("{}", val as char);
        io::stdout().flush().ok();
    }
}
