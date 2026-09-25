//! The generated device tree enumerates every hart.

use rvsim_core::config::Config;
use rvsim_core::sim::dtb::generate_dtb;
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
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> &Node {
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
        self.props[name]
            .chunks(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
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
    let intc = |hart: u32| cpus.child(&format!("cpu@{hart}")).child("interrupt-controller").u32_prop("phandle");

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
    let root = tree_for(5);
    let mut seen = Vec::new();
    fn collect(node: &Node, seen: &mut Vec<u32>) {
        if let Some(bytes) = node.props.get("phandle") {
            seen.push(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        }
        for child in &node.children {
            collect(child, seen);
        }
    }
    collect(&root, &mut seen);
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count);
}
