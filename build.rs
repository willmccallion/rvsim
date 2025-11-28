use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Define the new directory structure
    let software_dir = manifest.join("software");
    let bootloader_dir = software_dir.join("bootloader");
    let kernel_dir = software_dir.join("kernel");

    // Watch for changes in the new locations
    println!("cargo:rerun-if-changed=software/bootloader/boot.s");
    println!("cargo:rerun-if-changed=software/kernel/entry.s");
    println!("cargo:rerun-if-changed=software/kernel/main.c");
    println!("cargo:rerun-if-changed=build.rs");

    assemble_link_bin(
        &bootloader_dir.join("boot.s"),
        &out_dir.join("bootloader.bin"),
        "_start",
    )
    .expect("bootloader build");

    build_kernel_c(&kernel_dir, &out_dir.join("kernel.bin")).expect("kernel build");
}

fn build_kernel_c(kernel_dir: &Path, dst_bin: &Path) -> Result<(), String> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let (cc, ld, objcopy) = find_tools().ok_or("No RISC-V toolchain found")?;

    let entry_src = kernel_dir.join("entry.s");
    let c_src = kernel_dir.join("main.c");

    let entry_obj = out_dir.join("kernel_entry.o");
    let c_obj = out_dir.join("kernel.o");
    let elf = out_dir.join("kernel.elf");
    let lds = out_dir.join("kernel.ld");

    // Linker Script for Kernel (Base 0x80100000)
    fs::File::create(&lds)
        .and_then(|mut f| {
            write!(
                f,
                r#"
ENTRY(_start)
SECTIONS {{
    . = 0x80100000;
    .text : {{ *(.text.entry) *(.text .text.*) }}
    .rodata : {{ *(.rodata .rodata.*) }}
    .data : {{ *(.data .data.*) }}
    .bss : {{ *(.bss .bss.*) }}
}}
"#
            )
        })
        .map_err(|e| e.to_string())?;

    run(
        Command::new(&cc)
            .arg("-march=rv64g")
            .arg("-mabi=lp64")
            .arg("-c")
            .arg(&entry_src)
            .arg("-o")
            .arg(&entry_obj),
        "as (entry)",
    )?;

    run(
        Command::new(&cc)
            .arg("-march=rv64g")
            .arg("-mabi=lp64")
            .arg("-ffreestanding")
            .arg("-nostdlib")
            .arg("-O2")
            .arg("-c")
            .arg(&c_src)
            .arg("-o")
            .arg(&c_obj),
        "cc (kernel)",
    )?;

    run(
        Command::new(&ld)
            .arg("-nostdlib")
            .arg("-T")
            .arg(&lds)
            .arg("-o")
            .arg(&elf)
            .arg(&entry_obj)
            .arg(&c_obj),
        "ld",
    )?;

    run(
        Command::new(&objcopy)
            .arg("-O")
            .arg("binary")
            .arg(&elf)
            .arg(dst_bin),
        "objcopy",
    )?;

    Ok(())
}

fn assemble_link_bin(src: &Path, dst_bin: &Path, entry: &str) -> Result<(), String> {
    let (cc, ld, objcopy) = find_tools().ok_or("No tools")?;
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let stem = src.file_stem().unwrap().to_string_lossy();
    let obj = out_dir.join(format!("{stem}.o"));
    let elf = out_dir.join(format!("{stem}.elf"));
    let lds = out_dir.join(format!("{stem}.ld"));

    fs::File::create(&lds).and_then(|mut f| write!(f, r#"
ENTRY({entry})
SECTIONS {{ . = 0x0; .text : {{ *(.text .text.*) }} .rodata : {{ *(.rodata .rodata.*) }} .data : {{ *(.data .data.*) }} .bss : {{ *(.bss .bss.*) }} }}
"#)).map_err(|e| e.to_string())?;

    run(
        Command::new(&cc)
            .arg("-march=rv64g")
            .arg("-mabi=lp64")
            .arg("-ffreestanding")
            .arg("-c")
            .arg(src)
            .arg("-o")
            .arg(&obj),
        "as",
    )?;

    run(
        Command::new(&ld)
            .arg("-nostdlib")
            .arg("-T")
            .arg(&lds)
            .arg("-o")
            .arg(&elf)
            .arg(&obj),
        "ld",
    )?;

    // Binary
    run(
        Command::new(&objcopy)
            .arg("-O")
            .arg("binary")
            .arg(&elf)
            .arg(dst_bin),
        "objcopy",
    )?;

    Ok(())
}

fn find_tools() -> Option<(String, String, String)> {
    let prefixes = ["riscv64-unknown-elf-", "riscv64-linux-gnu-", "riscv64-elf-"];
    for p in prefixes {
        if which::which(format!("{}gcc", p)).is_ok() {
            return Some((
                format!("{}gcc", p),
                format!("{}ld", p),
                format!("{}objcopy", p),
            ));
        }
    }
    None
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "{} failed:\n{}\n{}",
            what,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}
