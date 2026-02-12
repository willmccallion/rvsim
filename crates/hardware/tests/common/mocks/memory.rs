use riscv_core::soc::devices::Device;
use riscv_core::soc::memory::Memory;
use riscv_core::soc::memory::controller::MemoryController;
use std::sync::{Arc, Mutex};

pub struct MockMemoryController {
    latency: u64,
}

impl MockMemoryController {
    pub fn new(latency: u64) -> Self {
        Self { latency }
    }

    pub fn set_latency(&mut self, latency: u64) {
        self.latency = latency;
    }
}

impl MemoryController for MockMemoryController {
    fn access_latency(&mut self, _addr: u64) -> u64 {
        self.latency
    }
}

pub struct MockMemory {
    data: Vec<u8>,
    base: u64,
    fault_addrs: Arc<Mutex<Vec<u64>>>,
}

impl MockMemory {
    pub fn new(size: usize, base: u64) -> Self {
        Self {
            data: vec![0; size],
            base,
            fault_addrs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn inject_fault(&self, addr: u64) {
        self.fault_addrs.lock().unwrap().push(addr);
    }

    fn check_fault(&self, offset: u64) {
        let addr = self.base + offset;
        if self.fault_addrs.lock().unwrap().contains(&addr) {
            // In a real scenario, this would trigger a bus error signal.
            // Since the Device trait doesn't support errors, we panic to simulate
            // a catastrophic failure that the test harness should catch or expect.
            panic!("Bus Error injected at address {:#x}", addr);
        }
    }
}

impl Device for MockMemory {
    fn name(&self) -> &str {
        "MockMemory"
    }
    fn address_range(&self) -> (u64, u64) {
        (self.base, self.data.len() as u64)
    }

    fn read_u8(&mut self, offset: u64) -> u8 {
        self.check_fault(offset);
        self.data.get(offset as usize).copied().unwrap_or(0)
    }

    fn read_u16(&mut self, offset: u64) -> u16 {
        self.check_fault(offset);
        let idx = offset as usize;
        if idx + 2 <= self.data.len() {
            u16::from_le_bytes(self.data[idx..idx + 2].try_into().unwrap())
        } else {
            0
        }
    }

    fn read_u32(&mut self, offset: u64) -> u32 {
        self.check_fault(offset);
        let idx = offset as usize;
        if idx + 4 <= self.data.len() {
            u32::from_le_bytes(self.data[idx..idx + 4].try_into().unwrap())
        } else {
            0
        }
    }

    fn read_u64(&mut self, offset: u64) -> u64 {
        self.check_fault(offset);
        let idx = offset as usize;
        if idx + 8 <= self.data.len() {
            u64::from_le_bytes(self.data[idx..idx + 8].try_into().unwrap())
        } else {
            0
        }
    }

    fn write_u8(&mut self, offset: u64, val: u8) {
        self.check_fault(offset);
        if let Some(elem) = self.data.get_mut(offset as usize) {
            *elem = val;
        }
    }

    fn write_u16(&mut self, offset: u64, val: u16) {
        self.check_fault(offset);
        let idx = offset as usize;
        if idx + 2 <= self.data.len() {
            self.data[idx..idx + 2].copy_from_slice(&val.to_le_bytes());
        }
    }

    fn write_u32(&mut self, offset: u64, val: u32) {
        self.check_fault(offset);
        let idx = offset as usize;
        if idx + 4 <= self.data.len() {
            self.data[idx..idx + 4].copy_from_slice(&val.to_le_bytes());
        }
    }

    fn write_u64(&mut self, offset: u64, val: u64) {
        self.check_fault(offset);
        let idx = offset as usize;
        if idx + 8 <= self.data.len() {
            self.data[idx..idx + 8].copy_from_slice(&val.to_le_bytes());
        }
    }

    fn as_memory_mut(&mut self) -> Option<&mut Memory> {
        // We cannot downcast to real Memory because we are not it.
        // Return None.
        None
    }
}
