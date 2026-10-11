//! The generated device tree enumerates every hart.

use crate::config::Config;
use crate::system::dtb::generate_dtb;
use std::collections::BTreeMap;

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

/// One node of a parsed flattened device tree.
#[derive(Debug, Default)]
struct Node {
    name: String,
    props: BTreeMap<String, Vec<u8>>,
    children: Vec<Self>,
}

impl Node {
    fn child(&self, name: &str) -> &Self {
        self.children
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("node {:?} has no child {name:?}", self.name))
    }

    fn u32_prop(&self, name: &str) -> u32 {
        let bytes = &self.props[name];
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    fn u32_cells(&self, name: &str) -> Vec<u32> {
        self.props[name].chunks(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect()
    }
}

fn be32(bytes: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn parse(dtb: &[u8]) -> Node {
    assert_eq!(be32(dtb, 0), 0xd00d_feed, "FDT magic");
    let struct_off = be32(dtb, 8) as usize;
    let strings_off = be32(dtb, 12) as usize;
    let mut pos = struct_off;
    let mut stack: Vec<Node> = Vec::new();
    loop {
        let token = be32(dtb, pos);
        pos += 4;
        match token {
            FDT_BEGIN_NODE => {
                let end = dtb[pos..].iter().position(|b| *b == 0).unwrap() + pos;
                let name = String::from_utf8(dtb[pos..end].to_vec()).unwrap();
                pos = (end + 1 + 3) & !3;
                stack.push(Node { name, ..Node::default() });
            }
            FDT_END_NODE => {
                let node = stack.pop().unwrap();
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => return node,
                }
            }
            FDT_PROP => {
                let len = be32(dtb, pos) as usize;
                let name_off = be32(dtb, pos + 4) as usize;
                pos += 8;
                let name_start = strings_off + name_off;
                let name_end = dtb[name_start..].iter().position(|b| *b == 0).unwrap() + name_start;
                let name = String::from_utf8(dtb[name_start..name_end].to_vec()).unwrap();
                let value = dtb[pos..pos + len].to_vec();
                pos = (pos + len + 3) & !3;
                stack.last_mut().unwrap().props.insert(name, value);
            }
            FDT_NOP => {}
            FDT_END => panic!("FDT_END before the root node closed"),
            other => panic!("unknown FDT token {other}"),
        }
    }
}

fn tree_for(hart_count: usize) -> Node {
    let mut config = Config::default();
    config.system.hart_count = hart_count;
    parse(&generate_dtb(&config))
}

fn soc_child<'a>(root: &'a Node, prefix: &str) -> &'a Node {
    root.child("soc")
        .children
        .iter()
        .find(|c| c.name.starts_with(prefix))
        .unwrap_or_else(|| panic!("no /soc node starting with {prefix:?}"))
}

#[test]
fn single_hart_tree_keeps_the_original_phandles() {
    let root = tree_for(1);
    let cpus = root.child("cpus");
    let cpu0 = cpus.child("cpu@0");
    assert_eq!(cpu0.child("interrupt-controller").u32_prop("phandle"), 1);
    assert_eq!(soc_child(&root, "interrupt-controller@").u32_prop("phandle"), 2);
    assert_eq!(soc_child(&root, "syscon@").u32_prop("phandle"), 3);
    assert_eq!(cpus.children.iter().filter(|c| c.name.starts_with("cpu@")).count(), 1);
}

#[test]
fn every_hart_gets_a_cpu_node_with_its_own_interrupt_controller() {
    let root = tree_for(4);
    let cpus = root.child("cpus");
    let mut intc_phandles = Vec::new();
    for hart in 0..4u32 {
        let cpu = cpus.child(&format!("cpu@{hart}"));
        assert_eq!(cpu.u32_prop("reg"), hart);
        assert_eq!(cpu.props["status"], b"okay\0");
        intc_phandles.push(cpu.child("interrupt-controller").u32_prop("phandle"));
    }
    intc_phandles.sort_unstable();
    intc_phandles.dedup();
    assert_eq!(intc_phandles.len(), 4, "interrupt-controller phandles are unique");

    let cluster = cpus.child("cpu-map").child("cluster0");
    for hart in 0..4u32 {
        let core = cluster.child(&format!("core{hart}"));
        assert_eq!(core.u32_prop("cpu"), cpus.child(&format!("cpu@{hart}")).u32_prop("phandle"));
    }
}

#[test]
fn clint_and_plic_interrupts_extended_name_every_hart() {
    let root = tree_for(3);
    let cpus = root.child("cpus");
    let intc = |hart: u32| {
        cpus.child(&format!("cpu@{hart}")).child("interrupt-controller").u32_prop("phandle")
    };

    let clint = soc_child(&root, "clint@").u32_cells("interrupts-extended");
    let plic = soc_child(&root, "interrupt-controller@").u32_cells("interrupts-extended");
    assert_eq!(clint.len(), 3 * 4);
    assert_eq!(plic.len(), 3 * 4);
    for hart in 0..3u32 {
        let base = hart as usize * 4;
        assert_eq!(&clint[base..base + 4], &[intc(hart), 3, intc(hart), 7]);
        assert_eq!(&plic[base..base + 4], &[intc(hart), 11, intc(hart), 9]);
    }
}

#[test]
fn phandles_never_collide() {
    fn collect(node: &Node, seen: &mut Vec<u32>) {
        if let Some(bytes) = node.props.get("phandle") {
            seen.push(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        }
        for child in &node.children {
            collect(child, seen);
        }
    }
    let root = tree_for(5);
    let mut seen = Vec::new();
    collect(&root, &mut seen);
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count);
}

const DEFAULT_ISA: &str = "rv64imafdcbv_zicbom_zicboz_zicntr_zicsr_zifencei_zihintpause_zihpm_zfh_zba_zbb_zbc_zbs_zbkb_zbkc_zbkx_zvbb_zvbc_zvfh_zvkb_zvkg_zvkned_zvknha_zvknhb_zvksed_zvksh_zvkt_zvl128b_sstc";

fn cpu0(config: &Config) -> Node {
    let tree = parse(&generate_dtb(config));
    let cpu = tree.child("cpus").child("cpu@0");
    Node { name: cpu.name.clone(), props: cpu.props.clone(), children: Vec::new() }
}

fn string_prop(node: &Node, name: &str) -> String {
    String::from_utf8(node.props[name].clone()).unwrap().trim_end_matches('\0').to_owned()
}

fn string_list_prop(node: &Node, name: &str) -> Vec<String> {
    string_prop(node, name).split('\0').map(str::to_owned).collect()
}

/// The ISA string names `misa`'s letters and every extension the hart
/// implements; what the letters leave out takes its extensions with it.
#[test]
fn the_isa_string_names_every_implemented_extension() {
    let mut config = Config::default();
    config.pipeline.misa_override = Some("RV64IMAC".parse().expect("valid ISA string"));

    let (default_isa, without_fdv) = (
        string_prop(&cpu0(&Config::default()), "riscv,isa"),
        string_prop(&cpu0(&config), "riscv,isa"),
    );

    assert_eq!(default_isa, DEFAULT_ISA);
    assert_eq!(
        without_fdv,
        "rv64imacb_zicbom_zicboz_zicntr_zicsr_zifencei_zihintpause_zihpm_zba_zbb_zbc_zbs_zbkb_zbkc_zbkx_sstc"
    );
}

#[test]
fn a_svadu_hart_advertises_svadu() {
    let mut config = Config::default();
    config.isa.svadu = true;

    let isa = string_prop(&cpu0(&config), "riscv,isa");

    assert_eq!(isa, format!("{DEFAULT_ISA}_svadu"));
}

#[test]
fn a_hart_without_zvfh_leaves_it_out() {
    let mut config = Config::default();
    config.isa.vector.zvfh = false;

    let isa = string_prop(&cpu0(&config), "riscv,isa");

    assert_eq!(isa, DEFAULT_ISA.replace("_zvfh", ""));
}

/// The newer property pair says the same as the string, one extension per
/// entry, for kernels that prefer it.
#[test]
fn the_isa_extensions_list_matches_the_isa_string() {
    let cpu = cpu0(&Config::default());

    let listed = string_list_prop(&cpu, "riscv,isa-extensions");

    assert_eq!(string_prop(&cpu, "riscv,isa-base"), "rv64i");
    let from_string: Vec<String> = ["i", "m", "a", "f", "d", "c", "b", "v"]
        .into_iter()
        .map(str::to_owned)
        .chain(DEFAULT_ISA.split('_').skip(1).map(str::to_owned))
        .collect();
    assert_eq!(listed, from_string);
}

/// Linux uses Zicbom and Zicboz only when told their block size.
#[test]
fn the_cache_block_sizes_are_the_block_the_operations_act_on() {
    let cpu = cpu0(&Config::default());

    assert_eq!(cpu.u32_prop("riscv,cbom-block-size"), 64);
    assert_eq!(cpu.u32_prop("riscv,cboz-block-size"), 64);
}

#[test]
fn the_mmu_type_is_the_strongest_paging_mode() {
    let mut config = Config::default();
    config.memory.paging_mode_max = crate::isa::privileged::PagingMode::Sv39;

    let (default_mmu, sv39) = (
        string_prop(&cpu0(&Config::default()), "mmu-type"),
        string_prop(&cpu0(&config), "mmu-type"),
    );

    assert_eq!((default_mmu.as_str(), sv39.as_str()), ("riscv,sv57", "riscv,sv39"));
}
