//! RISC-V cycle-accurate simulator CLI.
//!
//! This binary provides a single entry point for all simulation modes. It performs:
//! 1. **Direct run:** Execute a bare-metal binary (default config, no kernel).
//! 2. **Kernel boot:** Load kernel image and optional disk/DTB; run in supervisor mode.
//! 3. **Script run:** Execute a Python script (gem5-style) with `riscv_emulator` injected; supports P550System, multisim, and custom sweeps.

use clap::{Parser, Subcommand};
use pyo3::prelude::*;
use pyo3::types::PyList;
use std::ffi::CString;
use std::io::Write;
use std::{fs, process};

use riscv_core::config::Config;
use riscv_core::core::Cpu;
use riscv_core::sim::loader;
use riscv_core::soc::System;

#[derive(Parser, Debug)]
#[command(
    name = "sim",
    author,
    version,
    about = "RISC-V cycle-accurate simulator",
    long_about = "Run a binary, boot a kernel, or run a Python config script (gem5-style).\n\nConfiguration is Python-first (see riscv_sim.config.SimConfig). The CLI uses built-in defaults.\n\nExamples:\n  sim run -f software/bin/benchmarks/qsort.bin\n  sim run --kernel Image --disk rootfs.img\n  sim scripts/p550/run.py\n  sim script scripts/tests/compare_p550_m1.py"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run a single binary (bare-metal) or kernel (OS boot).
    Run {
        /// Bare-metal binary to execute (direct mode).
        #[arg(short, long)]
        file: Option<String>,

        /// Kernel image for OS boot (disables direct mode).
        #[arg(long)]
        kernel: Option<String>,

        /// Disk image (e.g. rootfs) for OS boot.
        #[arg(long, default_value = "")]
        disk: String,

        /// Device tree blob for OS boot.
        #[arg(long)]
        dtb: Option<String>,
    },

    /// Run a Python script (gem5-style). Script gets argv as sys.argv. Use this for P550System, multisim, or any custom sweep.
    Script {
        /// Script path (e.g. scripts/p550/run.py).
        path: String,

        /// Arguments for the script (sys.argv[1:]).
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run {
            file,
            kernel,
            disk,
            dtb,
        }) => cmd_run(file, kernel, disk, dtb),
        Some(Commands::Script { path, args }) => run_python_script(&path, args),
        None => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let first = args.first().map(String::clone);
            if first.as_ref().map_or(false, |s| s.ends_with(".py")) {
                let script = first.unwrap();
                let script_args: Vec<String> = args.into_iter().skip(1).collect();
                run_python_script(&script, script_args);
                return;
            }
            eprintln!("RISC-V Simulator — pass a subcommand or a .py script");
            eprintln!();
            eprintln!("  sim run -f <binary>        Bare-metal run");
            eprintln!("  sim run --kernel <Image>   OS boot");
            eprintln!("  sim <script.py> [args...]  Run script (e.g. sim scripts/p550/run.py)");
            eprintln!("  sim script <script.py>     Same, explicit subcommand");
            eprintln!();
            eprintln!("  sim --help  for full options");
            process::exit(1);
        }
    }
}

/// Runs the simulator: loads kernel or bare-metal binary, then loops on `tick` until exit or trap.
///
/// Uses default config; loads kernel image and optional DTB if `kernel` is set, otherwise
/// loads the bare-metal binary at RAM base and sets PC. On trap, dumps state and exits with code 1.
fn cmd_run(file: Option<String>, kernel: Option<String>, disk: String, dtb: Option<String>) {
    let config = Config::default();

    let system = System::new(&config, &disk);
    let mut cpu = Cpu::new(system, &config);

    println!("Configuration: default (Python-first config: use riscv_sim.config.SimConfig)");
    println!(
        "  Trace: {}  Start PC: {:#x}  RAM: {} MB",
        config.general.trace_instructions,
        config.general.start_pc,
        config.memory.ram_size / 1024 / 1024
    );
    println!();

    if let Some(kernel_path) = kernel {
        println!("[*] OS Boot: kernel={}", kernel_path);
        if !disk.is_empty() {
            println!("    disk={}", disk);
        }
        if let Some(ref d) = dtb {
            println!("    dtb={}", d);
        }
        loader::setup_kernel_load(&mut cpu, &config, &disk, dtb, Some(kernel_path));
        cpu.direct_mode = false;
    } else if let Some(bin_path) = file {
        println!("[*] Direct execution: {}", bin_path);
        let bin_data = loader::load_binary(&bin_path);
        let load_addr = config.system.ram_base;
        cpu.bus.load_binary_at(&bin_data, load_addr);
        cpu.pc = load_addr;
    } else {
        eprintln!("Error: specify --file <binary> or --kernel <Image>");
        eprintln!("  sim run -f software/bin/benchmarks/qsort.bin");
        eprintln!("  sim run --kernel Image [--disk rootfs.img]");
        process::exit(1);
    }

    loop {
        if let Err(e) = cpu.tick() {
            eprintln!("\n[!] FATAL TRAP: {}", e);
            cpu.dump_state();
            cpu.stats.print();
            process::exit(1);
        }
        if let Some(code) = cpu.take_exit() {
            println!("\n[*] Exit code {}", code);
            cpu.stats.print();
            std::io::stdout().flush().ok();
            process::exit(code as i32);
        }
    }
}

/// Runs a Python script with `riscv_emulator` injected into `sys.modules` and `sys.argv` set.
///
/// The script is executed as `__main__`. Exits the process with code 1 on script error or missing file.
///
/// # Arguments
///
/// * `script_path` - Path to the `.py` file.
/// * `script_args` - Arguments passed as `sys.argv[1:]`.
fn run_python_script(script_path: &str, script_args: Vec<String>) {
    let script_content = fs::read_to_string(script_path).unwrap_or_else(|e| {
        eprintln!("Error reading script {}: {}", script_path, e);
        process::exit(1);
    });

    Python::with_gil(|py| {
        let sys = py.import("sys").expect("sys");
        let path = sys.getattr("path").expect("path");
        path.call_method1("append", (".",)).expect("path.append");
        path.call_method1("append", ("python",))
            .expect("path.append");

        let m = PyModule::new(py, "riscv_emulator").expect("module");
        riscv_emulator::register_emulator_module(&m).expect("register");
        let modules = sys.getattr("modules").expect("modules");
        modules.set_item("riscv_emulator", m).expect("inject");

        let mut full_args = vec![script_path.to_string()];
        full_args.extend(script_args);
        let py_args = PyList::new(py, &full_args).expect("argv");
        sys.setattr("argv", py_args).expect("argv");

        let code_c = CString::new(script_content).expect("code");
        let file_c = CString::new(script_path).expect("file");
        let name_c = CString::new("__main__").unwrap();

        let result = PyModule::from_code(py, &code_c, &file_c, &name_c);
        if let Err(e) = result {
            e.print(py);
            process::exit(1);
        }
    });
}
