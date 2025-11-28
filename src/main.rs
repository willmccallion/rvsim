use std::{env, fs, path::Path};

mod cpu;
mod devices;
mod isa;
mod memory;
mod register_file;
mod stats;

use cpu::Cpu;
use devices::{Bus, VirtualDisk};
use memory::{BASE_ADDRESS, Memory};

const BIOS_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bootloader.bin"));
const KERNEL_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kernel.bin"));

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct FileHeader {
    name: [u8; 32],
    offset: u32,
    size: u32,
}

fn main() {
    let mut trace = false;
    for arg in env::args().skip(1) {
        if arg == "--trace" {
            trace = true;
        }
    }

    let mut disk_img = Vec::new();

    let kernel_size: u32 = 16384;

    let mut padded_kernel = KERNEL_BYTES.to_vec();
    if padded_kernel.len() > kernel_size as usize {
        eprintln!(
            "WARNING: Kernel is larger than {} bytes ({}), it will be truncated!",
            kernel_size,
            padded_kernel.len()
        );
    }
    padded_kernel.resize(kernel_size as usize, 0);
    disk_img.extend_from_slice(&padded_kernel);

    let mut headers: Vec<FileHeader> = Vec::new();
    let mut file_data: Vec<u8> = Vec::new();

    let bin_dir = Path::new("bin");
    if !bin_dir.exists() {
        eprintln!("Warning: 'bin/' directory not found. Run 'make' to build user programs.");
    } else {
        let entries = fs::read_dir(bin_dir).expect("Failed to read bin directory");
        let mut entries: Vec<_> = entries.map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("bin") {
                let name_str = path.file_stem().unwrap().to_string_lossy();
                let content = fs::read(&path).expect("Failed to read user binary");

                let mut name = [0u8; 32];
                let bytes = name_str.as_bytes();
                let len = bytes.len().min(31);
                name[0..len].copy_from_slice(&bytes[0..len]);

                headers.push(FileHeader {
                    name,
                    offset: 0,
                    size: content.len() as u32,
                });

                file_data.extend_from_slice(&content);
            }
        }
    }

    let header_size = std::mem::size_of::<FileHeader>() as u32;
    let count_size = 4;

    let headers_start_offset = kernel_size + count_size;
    let data_start_offset = headers_start_offset + (headers.len() as u32 * header_size);

    let mut current_data_offset = data_start_offset;
    for header in &mut headers {
        header.offset = current_data_offset;
        current_data_offset += header.size;
    }

    let count = headers.len() as u32;
    disk_img.extend_from_slice(&count.to_le_bytes());

    for header in &headers {
        let p = header as *const FileHeader as *const u8;
        let s = unsafe { std::slice::from_raw_parts(p, header_size as usize) };
        disk_img.extend_from_slice(s);
    }

    disk_img.extend_from_slice(&file_data);

    let mut mem = Memory::new();
    mem.load(BIOS_BYTES, 0);

    let mut disk = VirtualDisk::new();
    disk.load(disk_img);

    let uart = devices::Uart::new();

    let bus = Bus::new(mem, uart, disk);

    let mut cpu = Cpu::new(BASE_ADDRESS, trace, bus);

    println!("=========================================================");

    loop {
        if let Err(e) = cpu.tick() {
            eprintln!("\nFATAL TRAP: {}", e);
            cpu.dump_state();
            cpu.print_stats();
            std::process::exit(1);
        }

        if let Some(code) = cpu.take_exit() {
            println!("\n=========================================================\n");
            println!("CPU Halted with Exit Code: {}", code);
            cpu.print_stats();
            std::process::exit(code as i32);
        }
    }
}
