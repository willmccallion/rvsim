use mockall::mock;
use riscv_core::soc::devices::{Device, Plic, Uart};
use riscv_core::soc::memory::Memory;
use std::sync::{Arc, Mutex};

mock! {
    pub BusDevice {}
    impl Device for BusDevice {
        fn name(&self) -> &'static str;
        fn address_range(&self) -> (u64, u64);
        fn read_u8(&mut self, offset: u64) -> u8;
        fn read_u16(&mut self, offset: u64) -> u16;
        fn read_u32(&mut self, offset: u64) -> u32;
        fn read_u64(&mut self, offset: u64) -> u64;
        fn write_u8(&mut self, offset: u64, val: u8);
        fn write_u16(&mut self, offset: u64, val: u16);
        fn write_u32(&mut self, offset: u64, val: u32);
        fn write_u64(&mut self, offset: u64, val: u64);
        fn write_bytes(&mut self, offset: u64, data: &[u8]);
        fn tick(&mut self) -> bool;
        fn get_irq_id(&self) -> Option<u32>;
        fn as_plic_mut<'a>(&'a mut self) -> Option<&'a mut Plic>;
        fn as_uart_mut<'a>(&'a mut self) -> Option<&'a mut Uart>;
        fn as_memory_mut<'a>(&'a mut self) -> Option<&'a mut Memory>;
    }
}

/// A thread-safe wrapper around the mock device.
#[derive(Clone)]
pub struct SyncBusDevice {
    pub mock: Arc<Mutex<MockBusDevice>>,
    name: &'static str,
}

impl SyncBusDevice {
    pub fn new(mock: MockBusDevice, name: &'static str) -> Self {
        Self {
            mock: Arc::new(Mutex::new(mock)),
            name,
        }
    }
}

unsafe impl Send for SyncBusDevice {}
unsafe impl Sync for SyncBusDevice {}

impl Device for SyncBusDevice {
    fn name(&self) -> &str {
        self.name
    }

    fn address_range(&self) -> (u64, u64) {
        self.mock.lock().unwrap().address_range()
    }

    fn read_u8(&mut self, offset: u64) -> u8 {
        self.mock.lock().unwrap().read_u8(offset)
    }

    fn read_u16(&mut self, offset: u64) -> u16 {
        self.mock.lock().unwrap().read_u16(offset)
    }

    fn read_u32(&mut self, offset: u64) -> u32 {
        self.mock.lock().unwrap().read_u32(offset)
    }

    fn read_u64(&mut self, offset: u64) -> u64 {
        self.mock.lock().unwrap().read_u64(offset)
    }

    fn write_u8(&mut self, offset: u64, val: u8) {
        self.mock.lock().unwrap().write_u8(offset, val)
    }

    fn write_u16(&mut self, offset: u64, val: u16) {
        self.mock.lock().unwrap().write_u16(offset, val)
    }

    fn write_u32(&mut self, offset: u64, val: u32) {
        self.mock.lock().unwrap().write_u32(offset, val)
    }

    fn write_u64(&mut self, offset: u64, val: u64) {
        self.mock.lock().unwrap().write_u64(offset, val)
    }

    fn write_bytes(&mut self, offset: u64, data: &[u8]) {
        self.mock.lock().unwrap().write_bytes(offset, data)
    }

    fn tick(&mut self) -> bool {
        self.mock.lock().unwrap().tick()
    }

    fn get_irq_id(&self) -> Option<u32> {
        self.mock.lock().unwrap().get_irq_id()
    }

    fn as_plic_mut(&mut self) -> Option<&mut Plic> {
        None
    }
    fn as_uart_mut(&mut self) -> Option<&mut Uart> {
        None
    }
    fn as_memory_mut(&mut self) -> Option<&mut Memory> {
        None
    }
}
