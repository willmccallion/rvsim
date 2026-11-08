//! Device Tree Blob (DTB) generation.
//!
//! Generates a Flattened Device Tree (FDT) binary matching the system-on-chip layout.
//! This allows the simulator to provide a DTB to OpenSBI/Linux without
//! requiring an external `dtc` compilation step.

#![expect(
    clippy::cast_possible_truncation,
    reason = "the flattened device tree stores 32-bit big-endian cells"
)]

use crate::config::{
    CLINT_REGION_BYTES, Config, PLIC_BASE, PLIC_REGION_BYTES, RTC_BASE, RTC_REGION_BYTES,
    SYSCON_REGION_BYTES, UART_REGION_BYTES, VIRTIO_REGION_BYTES,
};
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::extension::Extension;
use crate::isa::privileged::PagingMode;
use crate::isa::privileged::cause::{CAUSE_INTERRUPT_BIT, interrupt};
use crate::soc::devices::{GoldfishRtc, SysCon, Uart, VirtioBlock};

const FDT_MAGIC: u32 = 0xd00dfeed;

/// Bytes in the flattened device tree header.
const FDT_HEADER_BYTES: u32 = 40;
/// Bytes in a memory reservation block holding only its terminating entry.
const FDT_EMPTY_MEMRSV_BYTES: u32 = 16;
const FDT_VERSION: u32 = 17;
const FDT_LAST_COMP_VERSION: u32 = 16;
const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_END: u32 = 9;

/// The PLIC sources the device tree advertises, as QEMU's virt machine
/// does; every device's source is below it.
const PLIC_ADVERTISED_SOURCES: u32 = 0x35;

/// The UART's input clock the device tree reports.
const UART_CLOCK_HZ: u32 = 10_000_000;

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

    /// Writes a node named `name` whose properties and children `body` writes.
    fn node(&mut self, name: &str, body: impl FnOnce(&mut Self)) {
        self.begin_node(name);
        body(self);
        self.end_node();
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

    /// A string-list property: every string NUL-terminated, one after
    /// the other.
    fn prop_string_list(&mut self, name: &str, values: &[String]) {
        let mut data = Vec::new();
        for value in values {
            data.extend_from_slice(value.as_bytes());
            data.push(0);
        }
        self.prop_bytes(name, &data);
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

        let dt_struct_offset = FDT_HEADER_BYTES + FDT_EMPTY_MEMRSV_BYTES;
        let dt_strings_offset = dt_struct_offset + struct_size;
        let total_size = dt_strings_offset + strings_size;

        let mut out = Vec::with_capacity(total_size as usize);

        // Header
        out.extend_from_slice(&FDT_MAGIC.to_be_bytes());
        out.extend_from_slice(&total_size.to_be_bytes());
        out.extend_from_slice(&dt_struct_offset.to_be_bytes());
        out.extend_from_slice(&dt_strings_offset.to_be_bytes());
        out.extend_from_slice(&FDT_HEADER_BYTES.to_be_bytes()); // off_mem_rsvmap
        out.extend_from_slice(&FDT_VERSION.to_be_bytes());
        out.extend_from_slice(&FDT_LAST_COMP_VERSION.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // boot_cpuid_phys
        out.extend_from_slice(&strings_size.to_be_bytes());
        out.extend_from_slice(&struct_size.to_be_bytes());

        out.extend_from_slice(&[0u8; FDT_EMPTY_MEMRSV_BYTES as usize]);

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

/// The `riscv,isa-extensions` list: every single-letter extension of
/// `misa`, then every multi-letter extension the hart implements, each
/// as Linux names it.
fn isa_extensions(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = config.misa().letters().map(String::from).collect();
    names.extend(config.extension_names());
    names
}

/// The `mmu-type` naming the strongest paging mode `satp` accepts.
const fn mmu_type(paging: PagingMode) -> &'static str {
    match paging {
        PagingMode::Bare => "riscv,none",
        PagingMode::Sv39 => "riscv,sv39",
        PagingMode::Sv48 => "riscv,sv48",
        PagingMode::Sv57 => "riscv,sv57",
    }
}

/// Generates a DTB binary matching the simulator's `SoC` layout.
///
/// The generated DTB includes:
/// - Memory region at `ram_base` with `ram_size`
/// - CLINT at `clint_base` with timer and software interrupts for every hart
/// - PLIC at 0x0c000000 with M- and S-mode contexts for every hart
/// - UART at `uart_base`
/// - virtio block device at `disk_base`
/// - One `cpu@N` per hart (its ISA string, as `riscv,isa` and as the
///   `riscv,isa-base` and `riscv,isa-extensions` pair, the strongest paging
///   mode as `mmu-type`, and the block size Zicbom and Zicboz work on)
///   and a `cpu-map`
pub fn generate_dtb(config: &Config) -> Vec<u8> {
    let hart_count = u32::try_from(config.system.hart_count.get()).unwrap_or(u32::MAX);
    let phandles = Phandles { hart_count };
    let mut fdt = FdtBuilder::new();
    fdt.node("", |fdt| {
        fdt.prop_u32("#address-cells", 2);
        fdt.prop_u32("#size-cells", 2);
        fdt.prop_string("compatible", "riscv-virtio");
        fdt.prop_string("model", "riscv-virtio,qemu");
        add_chosen(fdt, config);
        add_cpus(fdt, config, &phandles);
        add_memory(fdt, config);
        add_soc(fdt, config, &phandles);
    });
    fdt.finalize()
}

/// `/chosen`: the kernel command line and the console.
fn add_chosen(fdt: &mut FdtBuilder, config: &Config) {
    let uart_base = config.system.uart_base.val();
    fdt.node("chosen", |fdt| {
        fdt.prop_string(
            "bootargs",
            &format!(
                "root=/dev/vda rw console=ttyS0 earlycon=uart8250,mmio,{uart_base:#x} rootwait"
            ),
        );
        fdt.prop_string("stdout-path", &format!("/soc/uart@{uart_base:x}"));
    });
}

/// `/cpus`: a `cpu@N` with its interrupt controller per hart, and a
/// `cpu-map` of one cluster with a core per hart.
fn add_cpus(fdt: &mut FdtBuilder, config: &Config, phandles: &Phandles) {
    fdt.node("cpus", |fdt| {
        fdt.prop_u32("#address-cells", 1);
        fdt.prop_u32("#size-cells", 0);
        fdt.prop_u32("timebase-frequency", config.system.timebase_hz.get());
        for hart in 0..phandles.hart_count {
            add_cpu(fdt, config, phandles, hart);
        }
        fdt.node("cpu-map", |fdt| {
            fdt.node("cluster0", |fdt| {
                for hart in 0..phandles.hart_count {
                    fdt.node(&format!("core{hart}"), |fdt| fdt.prop_u32("cpu", phandles.cpu(hart)));
                }
            });
        });
    });
}

/// `cpu@hart`: its ISA, paging mode and cache-block sizes.
fn add_cpu(fdt: &mut FdtBuilder, config: &Config, phandles: &Phandles, hart: u32) {
    let cache_block = u32::try_from(CBOZ_BLOCK_SIZE).unwrap_or(u32::MAX);
    fdt.node(&format!("cpu@{hart}"), |fdt| {
        fdt.prop_string("device_type", "cpu");
        fdt.prop_reg_1_0(hart);
        fdt.prop_string("status", "okay");
        fdt.prop_string("compatible", "riscv");
        fdt.prop_string("riscv,isa", &config.isa_string());
        fdt.prop_string("riscv,isa-base", "rv64i");
        fdt.prop_string_list("riscv,isa-extensions", &isa_extensions(config));
        fdt.prop_string("mmu-type", mmu_type(config.memory.paging_mode_max));
        if config.extensions().contains(Extension::Zicbom) {
            fdt.prop_u32("riscv,cbom-block-size", cache_block);
        }
        if config.extensions().contains(Extension::Zicboz) {
            fdt.prop_u32("riscv,cboz-block-size", cache_block);
        }
        fdt.prop_u32("phandle", phandles.cpu(hart));
        fdt.node("interrupt-controller", |fdt| {
            fdt.prop_u32("#interrupt-cells", 1);
            fdt.prop_empty("interrupt-controller");
            fdt.prop_string("compatible", "riscv,cpu-intc");
            fdt.prop_u32("phandle", Phandles::cpu_intc(hart));
        });
    });
}

/// `/memory`: the RAM.
fn add_memory(fdt: &mut FdtBuilder, config: &Config) {
    let ram_base = config.system.ram_base.val();
    fdt.node(&format!("memory@{ram_base:x}"), |fdt| {
        fdt.prop_string("device_type", "memory");
        fdt.prop_reg_2_2(ram_base, config.memory.ram_size);
    });
}

/// `/soc`: the devices on the bus.
fn add_soc(fdt: &mut FdtBuilder, config: &Config, phandles: &Phandles) {
    let system = &config.system;
    fdt.node("soc", |fdt| {
        fdt.prop_u32("#address-cells", 2);
        fdt.prop_u32("#size-cells", 2);
        fdt.prop_string("compatible", "simple-bus");
        fdt.prop_empty("ranges");
        fdt.node(&format!("clint@{:x}", system.clint_base.val()), |fdt| {
            fdt.prop_string("compatible", "riscv,clint0");
            fdt.prop_reg_2_2(system.clint_base.val(), CLINT_REGION_BYTES);
            let causes = [interrupt::MACHINE_SOFTWARE, interrupt::MACHINE_TIMER];
            fdt.prop_bytes("interrupts-extended", &interrupts_extended(phandles, &causes));
        });
        fdt.node(&format!("uart@{:x}", system.uart_base.val()), |fdt| {
            fdt.prop_string("compatible", "ns16550a");
            fdt.prop_reg_2_2(system.uart_base.val(), UART_REGION_BYTES);
            fdt.prop_u32("clock-frequency", UART_CLOCK_HZ);
            fdt.prop_u32("interrupt-parent", phandles.plic());
            fdt.prop_bytes("interrupts", &Uart::IRQ.val().to_be_bytes());
            fdt.prop_string("status", "okay");
        });
        fdt.node(&format!("virtio_mmio@{:x}", system.disk_base.val()), |fdt| {
            fdt.prop_string("compatible", "virtio,mmio");
            fdt.prop_reg_2_2(system.disk_base.val(), VIRTIO_REGION_BYTES);
            fdt.prop_u32("interrupt-parent", phandles.plic());
            fdt.prop_bytes("interrupts", &VirtioBlock::IRQ.val().to_be_bytes());
        });
        add_plic(fdt, phandles);
        add_syscon(fdt, system.syscon_base.val(), phandles);
        fdt.node(&format!("rtc@{:x}", RTC_BASE.val()), |fdt| {
            fdt.prop_string("compatible", "google,goldfish-rtc");
            fdt.prop_reg_2_2(RTC_BASE.val(), RTC_REGION_BYTES);
            fdt.prop_u32("interrupt-parent", phandles.plic());
            fdt.prop_bytes("interrupts", &GoldfishRtc::IRQ.val().to_be_bytes());
        });
    });
}

/// The PLIC, with machine and supervisor external interrupts per hart.
fn add_plic(fdt: &mut FdtBuilder, phandles: &Phandles) {
    fdt.node(&format!("interrupt-controller@{:x}", PLIC_BASE.val()), |fdt| {
        fdt.prop_string("compatible", "riscv,plic0");
        fdt.prop_reg_2_2(PLIC_BASE.val(), PLIC_REGION_BYTES);
        fdt.prop_u32("#interrupt-cells", 1);
        fdt.prop_empty("interrupt-controller");
        let causes = [interrupt::MACHINE_EXTERNAL, interrupt::SUPERVISOR_EXTERNAL];
        fdt.prop_bytes("interrupts-extended", &interrupts_extended(phandles, &causes));
        fdt.prop_u32("riscv,ndev", PLIC_ADVERTISED_SOURCES);
        fdt.prop_u32("phandle", phandles.plic());
    });
}

/// `SysCon` at `base`, and the poweroff and reboot nodes that write its
/// commands to offset 0.
fn add_syscon(fdt: &mut FdtBuilder, base: u64, phandles: &Phandles) {
    fdt.node(&format!("syscon@{base:x}"), |fdt| {
        fdt.prop_string("compatible", "syscon");
        fdt.prop_reg_2_2(base, SYSCON_REGION_BYTES);
        fdt.prop_u32("phandle", phandles.syscon());
    });
    for (name, compatible, command) in [
        ("poweroff", "syscon-poweroff", SysCon::POWER_OFF),
        ("reboot", "syscon-reboot", SysCon::RESET),
    ] {
        fdt.node(name, |fdt| {
            fdt.prop_string("compatible", compatible);
            fdt.prop_u32("regmap", phandles.syscon());
            fdt.prop_u32("offset", 0);
            fdt.prop_u32("value", command);
        });
    }
}

/// The interrupt number of `cause`, an interrupt's `mcause`.
const fn interrupt_code(cause: u64) -> u32 {
    (cause & !CAUSE_INTERRUPT_BIT) as u32
}

/// `interrupts-extended` cells naming every hart's interrupt controller once
/// per interrupt in `causes`: `<&intc0 c0> <&intc0 c1> <&intc1 c0> ...`.
fn interrupts_extended(phandles: &Phandles, causes: &[u64]) -> Vec<u8> {
    let mut cells = Vec::with_capacity(phandles.hart_count as usize * causes.len() * 8);
    for hart in 0..phandles.hart_count {
        for &cause in causes {
            cells.extend_from_slice(&Phandles::cpu_intc(hart).to_be_bytes());
            cells.extend_from_slice(&interrupt_code(cause).to_be_bytes());
        }
    }
    cells
}
