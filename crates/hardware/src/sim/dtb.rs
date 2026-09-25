//! Device Tree Blob (DTB) generation.
//!
//! Generates a Flattened Device Tree (FDT) binary matching the `SoC` layout.
//! This allows the simulator to provide a DTB to OpenSBI/Linux without
//! requiring an external `dtc` compilation step.

use crate::config::Config;

// FDT constants
const FDT_MAGIC: u32 = 0xd00dfeed;
const FDT_VERSION: u32 = 17;
const FDT_LAST_COMP_VERSION: u32 = 16;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_END: u32 = 9;

/// Builder for constructing FDT binary blobs.
struct FdtBuilder {
    struct_buf: Vec<u8>,
    strings_buf: Vec<u8>,
    /// Map of string -> offset in `strings_buf` for dedup.
    string_offsets: Vec<(String, u32)>,
}

impl FdtBuilder {
    const fn new() -> Self {
        Self { struct_buf: Vec::new(), strings_buf: Vec::new(), string_offsets: Vec::new() }
    }

    fn push_u32(&mut self, val: u32) {
        self.struct_buf.extend_from_slice(&val.to_be_bytes());
    }

    fn begin_node(&mut self, name: &str) {
        self.push_u32(FDT_BEGIN_NODE);
        self.struct_buf.extend_from_slice(name.as_bytes());
        self.struct_buf.push(0); // null terminator
        // Align to 4 bytes
        while !self.struct_buf.len().is_multiple_of(4) {
            self.struct_buf.push(0);
        }
    }

    fn end_node(&mut self) {
        self.push_u32(FDT_END_NODE);
    }

    fn string_offset(&mut self, name: &str) -> u32 {
        for (s, off) in &self.string_offsets {
            if s == name {
                return *off;
            }
        }
        let off = self.strings_buf.len() as u32;
        self.strings_buf.extend_from_slice(name.as_bytes());
        self.strings_buf.push(0);
        self.string_offsets.push((name.to_string(), off));
        off
    }

    fn prop_u32(&mut self, name: &str, val: u32) {
        let name_off = self.string_offset(name);
        self.push_u32(FDT_PROP);
        self.push_u32(4); // length
        self.push_u32(name_off);
        self.push_u32(val);
    }

    fn prop_string(&mut self, name: &str, val: &str) {
        let name_off = self.string_offset(name);
        let data = val.as_bytes();
        let len = data.len() + 1; // include null terminator
        self.push_u32(FDT_PROP);
        self.push_u32(len as u32);
        self.push_u32(name_off);
        self.struct_buf.extend_from_slice(data);
        self.struct_buf.push(0);
        while !self.struct_buf.len().is_multiple_of(4) {
            self.struct_buf.push(0);
        }
    }

    fn prop_bytes(&mut self, name: &str, data: &[u8]) {
        let name_off = self.string_offset(name);
        self.push_u32(FDT_PROP);
        self.push_u32(data.len() as u32);
        self.push_u32(name_off);
        self.struct_buf.extend_from_slice(data);
        while !self.struct_buf.len().is_multiple_of(4) {
            self.struct_buf.push(0);
        }
    }

    fn prop_empty(&mut self, name: &str) {
        let name_off = self.string_offset(name);
        self.push_u32(FDT_PROP);
        self.push_u32(0);
        self.push_u32(name_off);
    }

    /// Encode a reg property with pairs of (`addr_hi`, `addr_lo`, `size_hi`, `size_lo`)
    /// for #address-cells=2, #size-cells=2.
    fn prop_reg_2_2(&mut self, addr: u64, size: u64) {
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&((addr >> 32) as u32).to_be_bytes());
        data.extend_from_slice(&(addr as u32).to_be_bytes());
        data.extend_from_slice(&((size >> 32) as u32).to_be_bytes());
        data.extend_from_slice(&(size as u32).to_be_bytes());
        self.prop_bytes("reg", &data);
    }

    /// Encode a reg property for #address-cells=1, #size-cells=0.
    fn prop_reg_1_0(&mut self, val: u32) {
        self.prop_bytes("reg", &val.to_be_bytes());
    }

    fn finalize(mut self) -> Vec<u8> {
        // End of structure block
        self.push_u32(FDT_END);

        let struct_size = self.struct_buf.len() as u32;
        let strings_size = self.strings_buf.len() as u32;

        // Header is 40 bytes
        let header_size = 40u32;
        // Memory reservation block: one empty entry (16 bytes of zeros)
        let memrsv_size = 16u32;

        let dt_struct_offset = header_size + memrsv_size;
        let dt_strings_offset = dt_struct_offset + struct_size;
        let total_size = dt_strings_offset + strings_size;

        let mut out = Vec::with_capacity(total_size as usize);

        // Header
        out.extend_from_slice(&FDT_MAGIC.to_be_bytes());
        out.extend_from_slice(&total_size.to_be_bytes());
        out.extend_from_slice(&dt_struct_offset.to_be_bytes());
        out.extend_from_slice(&dt_strings_offset.to_be_bytes());
        out.extend_from_slice(&(header_size).to_be_bytes()); // off_mem_rsvmap
        out.extend_from_slice(&FDT_VERSION.to_be_bytes());
        out.extend_from_slice(&FDT_LAST_COMP_VERSION.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // boot_cpuid_phys
        out.extend_from_slice(&strings_size.to_be_bytes());
        out.extend_from_slice(&struct_size.to_be_bytes());

        // Memory reservation block (empty)
        out.extend_from_slice(&[0u8; 16]);

        // Structure block
        out.extend_from_slice(&self.struct_buf);

        // Strings block
        out.extend_from_slice(&self.strings_buf);

        out
    }
}

/// Phandles for one system: interrupt controllers first so a single-hart
/// tree keeps the values it always had (cpu0 intc = 1, PLIC = 2, syscon = 3).
struct Phandles {
    hart_count: u32,
}

impl Phandles {
    const fn cpu_intc(hart: u32) -> u32 {
        1 + hart
    }

    const fn plic(&self) -> u32 {
        self.hart_count + 1
    }

    const fn syscon(&self) -> u32 {
        self.hart_count + 2
    }

    const fn cpu(&self, hart: u32) -> u32 {
        self.hart_count + 3 + hart
    }
}

/// Generates a DTB binary matching the simulator's `SoC` layout.
///
/// The generated DTB includes:
/// - Memory region at `ram_base` with `ram_size`
/// - CLINT at `clint_base` with timer and software interrupts for every hart
/// - PLIC at 0x0c000000 with M- and S-mode contexts for every hart
/// - UART at `uart_base`
/// - `VirtIO` block device at `disk_base`
/// - One `cpu@N` per hart (rv64imafdc ISA, SV39 MMU) and a `cpu-map`
pub fn generate_dtb(config: &Config) -> Vec<u8> {
    let ram_base = config.system.ram_base;
    let ram_size = config.memory.ram_size as u64;
    let uart_base = config.system.uart_base;
    let disk_base = config.system.disk_base;
    let clint_base = config.system.clint_base;
    let syscon_base = config.system.syscon_base;
    let rtc_base: u64 = 0x10_1000;
    let plic_base: u64 = 0x0c00_0000;
    let timebase_freq: u32 = 10_000_000;
    let hart_count = u32::try_from(config.system.hart_count.max(1)).unwrap_or(u32::MAX);

    let bootargs =
        format!("root=/dev/vda rw console=ttyS0 earlycon=uart8250,mmio,{uart_base:#x} rootwait");
    let stdout_path = format!("/soc/uart@{uart_base:x}");

    let phandles = Phandles { hart_count };
    let plic_phandle = phandles.plic();

    let mut b = FdtBuilder::new();

    // Root node
    b.begin_node("");
    b.prop_u32("#address-cells", 2);
    b.prop_u32("#size-cells", 2);
    b.prop_string("compatible", "riscv-virtio");
    b.prop_string("model", "riscv-virtio,qemu");

    // /chosen
    b.begin_node("chosen");
    b.prop_string("bootargs", &bootargs);
    b.prop_string("stdout-path", &stdout_path);
    b.end_node();

    // /cpus
    b.begin_node("cpus");
    b.prop_u32("#address-cells", 1);
    b.prop_u32("#size-cells", 0);
    b.prop_u32("timebase-frequency", timebase_freq);

    for hart in 0..hart_count {
        b.begin_node(&format!("cpu@{hart}"));
        b.prop_string("device_type", "cpu");
        b.prop_reg_1_0(hart);
        b.prop_string("status", "okay");
        b.prop_string("compatible", "riscv");
        b.prop_string("riscv,isa", "rv64imafdc");
        b.prop_string("mmu-type", "riscv,sv39");
        b.prop_u32("phandle", phandles.cpu(hart));

        b.begin_node("interrupt-controller");
        b.prop_u32("#interrupt-cells", 1);
        b.prop_empty("interrupt-controller");
        b.prop_string("compatible", "riscv,cpu-intc");
        b.prop_u32("phandle", Phandles::cpu_intc(hart));
        b.end_node(); // interrupt-controller

        b.end_node(); // cpu@N
    }

    // /cpus/cpu-map: one cluster, one core per hart.
    b.begin_node("cpu-map");
    b.begin_node("cluster0");
    for hart in 0..hart_count {
        b.begin_node(&format!("core{hart}"));
        b.prop_u32("cpu", phandles.cpu(hart));
        b.end_node();
    }
    b.end_node(); // cluster0
    b.end_node(); // cpu-map

    b.end_node(); // cpus

    // /memory
    {
        let node_name = format!("memory@{ram_base:x}");
        b.begin_node(&node_name);
        b.prop_string("device_type", "memory");
        b.prop_reg_2_2(ram_base, ram_size);
        b.end_node();
    }

    // /soc
    b.begin_node("soc");
    b.prop_u32("#address-cells", 2);
    b.prop_u32("#size-cells", 2);
    b.prop_string("compatible", "simple-bus");
    b.prop_empty("ranges");

    // /soc/clint
    {
        let node_name = format!("clint@{clint_base:x}");
        b.begin_node(&node_name);
        b.prop_string("compatible", "riscv,clint0");
        b.prop_reg_2_2(clint_base, 0x10000);
        // Per hart: <&intc 3> (M-mode software), <&intc 7> (M-mode timer).
        b.prop_bytes("interrupts-extended", &interrupts_extended(&phandles, &[3, 7]));
        b.end_node();
    }

    // /soc/uart
    {
        let node_name = format!("uart@{uart_base:x}");
        b.begin_node(&node_name);
        b.prop_string("compatible", "ns16550a");
        b.prop_reg_2_2(uart_base, 0x100);
        b.prop_u32("clock-frequency", 10_000_000);
        b.prop_u32("interrupt-parent", plic_phandle);
        b.prop_bytes("interrupts", &10u32.to_be_bytes());
        b.prop_string("status", "okay");
        b.end_node();
    }

    // /soc/virtio_mmio
    {
        let node_name = format!("virtio_mmio@{disk_base:x}");
        b.begin_node(&node_name);
        b.prop_string("compatible", "virtio,mmio");
        b.prop_reg_2_2(disk_base, 0x1000);
        b.prop_u32("interrupt-parent", plic_phandle);
        b.prop_bytes("interrupts", &1u32.to_be_bytes());
        b.end_node();
    }

    // /soc/plic
    {
        let node_name = format!("interrupt-controller@{plic_base:x}");
        b.begin_node(&node_name);
        b.prop_string("compatible", "riscv,plic0");
        b.prop_reg_2_2(plic_base, 0x4000000);
        b.prop_u32("#interrupt-cells", 1);
        b.prop_empty("interrupt-controller");
        // Per hart: <&intc 11> (M-mode external), <&intc 9> (S-mode external).
        b.prop_bytes("interrupts-extended", &interrupts_extended(&phandles, &[11, 9]));
        b.prop_u32("riscv,ndev", 0x35);
        b.prop_u32("phandle", plic_phandle);
        b.end_node();
    }

    // /soc/syscon (with poweroff and reboot sub-nodes)
    let syscon_phandle = phandles.syscon();
    {
        let node_name = format!("syscon@{syscon_base:x}");
        b.begin_node(&node_name);
        b.prop_string("compatible", "syscon");
        b.prop_reg_2_2(syscon_base, 0x1000);
        b.prop_u32("phandle", syscon_phandle);
        b.end_node();
    }

    // /soc/poweroff — writes 0x5555 to syscon offset 0x00
    {
        b.begin_node("poweroff");
        b.prop_string("compatible", "syscon-poweroff");
        b.prop_u32("regmap", syscon_phandle);
        b.prop_u32("offset", 0x00);
        b.prop_u32("value", 0x5555);
        b.end_node();
    }

    // /soc/reboot — writes 0x7777 to syscon offset 0x00
    {
        b.begin_node("reboot");
        b.prop_string("compatible", "syscon-reboot");
        b.prop_u32("regmap", syscon_phandle);
        b.prop_u32("offset", 0x00);
        b.prop_u32("value", 0x7777);
        b.end_node();
    }

    // /soc/rtc (Goldfish RTC)
    {
        let node_name = format!("rtc@{rtc_base:x}");
        b.begin_node(&node_name);
        b.prop_string("compatible", "google,goldfish-rtc");
        b.prop_reg_2_2(rtc_base, 0x1000);
        b.prop_u32("interrupt-parent", plic_phandle);
        b.prop_bytes("interrupts", &11u32.to_be_bytes());
        b.end_node();
    }

    b.end_node(); // soc
    b.end_node(); // root

    b.finalize()
}

/// `interrupts-extended` cells naming every hart's interrupt controller once
/// per entry in `irqs`: `<&intc0 irqs[0]> <&intc0 irqs[1]> <&intc1 irqs[0]> ...`.
fn interrupts_extended(phandles: &Phandles, irqs: &[u32]) -> Vec<u8> {
    let mut cells = Vec::with_capacity(phandles.hart_count as usize * irqs.len() * 8);
    for hart in 0..phandles.hart_count {
        for irq in irqs {
            cells.extend_from_slice(&Phandles::cpu_intc(hart).to_be_bytes());
            cells.extend_from_slice(&irq.to_be_bytes());
        }
    }
    cells
}
