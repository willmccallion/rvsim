//! Simulation statistics collection and reporting.
//!
//! This module tracks performance metrics for the RISC-V simulator. It provides:
//! 1. **Cycle and IPC:** Total cycles, retired instructions, and derived metrics (CPI, MIPS).
//! 2. **Instruction mix:** Counts by category (ALU, load, store, branch, system, FP).
//! 3. **Branch prediction:** Lookups, mispredictions, and accuracy.
//! 4. **Stalls:** Memory, control, and data hazard stall counts.
//! 5. **Cache hierarchy:** Hit/miss counts for L1-I, L1-D, L2, and L3.

use std::time::Instant;

/// Simulation statistics structure tracking all performance metrics.
///
/// Collects detailed statistics about instruction execution, cache behavior,
/// branch prediction, stalls, and execution time for performance analysis.
#[derive(Clone)]
pub struct SimStats {
    start_time: Instant,
    /// Total simulator cycles elapsed.
    pub cycles: u64,
    /// Number of instructions committed (retired).
    pub instructions_retired: u64,

    /// Count of integer load instructions retired.
    pub inst_load: u64,
    /// Count of integer store instructions retired.
    pub inst_store: u64,
    /// Count of branch/jump instructions retired.
    pub inst_branch: u64,
    /// Count of ALU (non-load/store/branch/system) instructions retired.
    pub inst_alu: u64,
    /// Count of system (CSR, ECALL, etc.) instructions retired.
    pub inst_system: u64,

    /// Count of floating-point load instructions retired.
    pub inst_fp_load: u64,
    /// Count of floating-point store instructions retired.
    pub inst_fp_store: u64,
    /// Count of FP arithmetic instructions retired.
    pub inst_fp_arith: u64,
    /// Count of FP fused multiply-add instructions retired.
    pub inst_fp_fma: u64,
    /// Count of FP divide/sqrt instructions retired.
    pub inst_fp_div_sqrt: u64,

    /// Number of branch predictions that were correct.
    pub branch_predictions: u64,
    /// Number of branch predictions that were wrong (mispredictions).
    pub branch_mispredictions: u64,

    /// Cycles spent in user (U) mode.
    pub cycles_user: u64,
    /// Cycles spent in supervisor (S) mode.
    pub cycles_kernel: u64,
    /// Cycles spent in machine (M) mode.
    pub cycles_machine: u64,

    /// Stall cycles due to memory (cache/memory not ready).
    pub stalls_mem: u64,
    /// Stall cycles due to control hazards (branch resolution, flush).
    pub stalls_control: u64,
    /// Stall cycles due to data hazards (RAW dependencies).
    pub stalls_data: u64,

    /// Number of traps (exceptions or interrupts) taken.
    pub traps_taken: u64,

    /// L1 instruction cache hit count.
    pub icache_hits: u64,
    /// L1 instruction cache miss count.
    pub icache_misses: u64,
    /// L1 data cache hit count.
    pub dcache_hits: u64,
    /// L1 data cache miss count.
    pub dcache_misses: u64,
    /// L2 cache hit count.
    pub l2_hits: u64,
    /// L2 cache miss count.
    pub l2_misses: u64,
    /// L3 cache hit count.
    pub l3_hits: u64,
    /// L3 cache miss count.
    pub l3_misses: u64,
}

impl Default for SimStats {
    /// Returns the default value.
    fn default() -> Self {
        Self {
            start_time: Instant::now(),
            cycles: 0,
            instructions_retired: 0,
            inst_load: 0,
            inst_store: 0,
            inst_branch: 0,
            inst_alu: 0,
            inst_system: 0,
            inst_fp_load: 0,
            inst_fp_store: 0,
            inst_fp_arith: 0,
            inst_fp_fma: 0,
            inst_fp_div_sqrt: 0,
            branch_predictions: 0,
            branch_mispredictions: 0,
            cycles_user: 0,
            cycles_kernel: 0,
            cycles_machine: 0,
            stalls_mem: 0,
            stalls_control: 0,
            stalls_data: 0,
            traps_taken: 0,
            icache_hits: 0,
            icache_misses: 0,
            dcache_hits: 0,
            dcache_misses: 0,
            l2_hits: 0,
            l2_misses: 0,
            l3_hits: 0,
            l3_misses: 0,
        }
    }
}

/// Section names for selective stats output.
///
/// Valid section identifiers: `"summary"`, `"core"`, `"instruction_mix"`, `"branch"`, `"memory"`.
/// Pass an empty slice to `print_sections` to print all sections.
pub const STATS_SECTIONS: &[&str] = &["summary", "core", "instruction_mix", "branch", "memory"];

impl SimStats {
    /// Prints only the requested statistics sections to stdout.
    ///
    /// Each element of `sections` should be one of `"summary"`, `"core"`, `"instruction_mix"`,
    /// `"branch"`, or `"memory"`. Pass an empty slice to print all sections (same as `print()`).
    ///
    /// # Arguments
    ///
    /// * `sections` - Slice of section names to print, or empty for all.
    ///
    /// # Panics
    ///
    /// This function will not panic. Division by zero is prevented by:
    /// - `cyc` is set to `max(cycles, 1)` before any division (line 143)
    /// - `instr` is set to `max(instructions_retired, 1)` before division (lines 144-148)
    /// - All floating-point divisions use these protected values
    pub fn print_sections(&self, sections: &[String]) {
        let want = |s: &str| sections.is_empty() || sections.iter().any(|x| x == s);
        let duration = self.start_time.elapsed();
        let seconds = duration.as_secs_f64();
        let cyc = if self.cycles == 0 { 1 } else { self.cycles };
        let instr = if self.instructions_retired == 0 {
            1
        } else {
            self.instructions_retired
        };

        if want("summary") {
            let ipc = self.instructions_retired as f64 / cyc as f64;
            let cpi = cyc as f64 / instr as f64;
            let mips = (self.instructions_retired as f64 / seconds) / 1_000_000.0;
            let khz = (self.cycles as f64 / seconds) / 1000.0;
            println!("\n==========================================================");
            println!("RISC-V SYSTEM SIMULATION STATISTICS");
            println!("==========================================================");
            println!("host_seconds             {:.4} s", seconds);
            println!("sim_cycles               {}", self.cycles);
            println!("sim_freq                 {:.2} kHz", khz);
            println!("sim_insts                {}", self.instructions_retired);
            println!("sim_ipc                  {:.4}", ipc);
            println!("sim_cpi                  {:.4}", cpi);
            println!("sim_mips                 {:.2}", mips);
            println!("----------------------------------------------------------");
        }
        if want("core") {
            println!("CORE BREAKDOWN");
            println!(
                "  cycles.user            {} ({:.2}%)",
                self.cycles_user,
                (self.cycles_user as f64 / cyc as f64) * 100.0
            );
            println!(
                "  cycles.kernel          {} ({:.2}%)",
                self.cycles_kernel,
                (self.cycles_kernel as f64 / cyc as f64) * 100.0
            );
            println!(
                "  cycles.machine         {} ({:.2}%)",
                self.cycles_machine,
                (self.cycles_machine as f64 / cyc as f64) * 100.0
            );
            println!(
                "  stalls.memory          {} ({:.2}%)",
                self.stalls_mem,
                (self.stalls_mem as f64 / cyc as f64) * 100.0
            );
            println!(
                "  stalls.control         {} ({:.2}%)",
                self.stalls_control,
                (self.stalls_control as f64 / cyc as f64) * 100.0
            );
            println!(
                "  stalls.data            {} ({:.2}%)",
                self.stalls_data,
                (self.stalls_data as f64 / cyc as f64) * 100.0
            );
            println!("----------------------------------------------------------");
        }
        if want("instruction_mix") {
            let total_inst = instr as f64;
            println!("INSTRUCTION MIX");
            println!(
                "  op.alu                 {} ({:.2}%)",
                self.inst_alu,
                (self.inst_alu as f64 / total_inst) * 100.0
            );
            println!(
                "  op.load                {} ({:.2}%)",
                self.inst_load,
                (self.inst_load as f64 / total_inst) * 100.0
            );
            println!(
                "  op.store               {} ({:.2}%)",
                self.inst_store,
                (self.inst_store as f64 / total_inst) * 100.0
            );
            println!(
                "  op.branch              {} ({:.2}%)",
                self.inst_branch,
                (self.inst_branch as f64 / total_inst) * 100.0
            );
            println!(
                "  op.system              {} ({:.2}%)",
                self.inst_system,
                (self.inst_system as f64 / total_inst) * 100.0
            );
            println!(
                "  op.fp_arith            {} ({:.2}%)",
                self.inst_fp_arith,
                (self.inst_fp_arith as f64 / total_inst) * 100.0
            );
            println!("----------------------------------------------------------");
        }
        if want("branch") {
            let bp_correct = self.branch_predictions;
            let bp_miss = self.branch_mispredictions;
            let bp_total = bp_correct + bp_miss;
            let bp_acc = if bp_total > 0 {
                100.0 * (bp_correct as f64 / bp_total as f64)
            } else {
                0.0
            };
            println!("BRANCH PREDICTION");
            println!("  bp.lookups             {}", bp_total);
            println!("  bp.mispredicts         {}", bp_miss);
            println!("  bp.accuracy            {:.2}%", bp_acc);
            println!("----------------------------------------------------------");
        }
        if want("memory") {
            let print_cache = |name: &str, hits: u64, misses: u64| {
                let total = hits + misses;
                let rate = if total > 0 {
                    (hits as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                println!(
                    "  {:<6} accesses: {:<10} | hits: {:<10} | miss_rate: {:.2}%",
                    name,
                    total,
                    hits,
                    100.0 - rate
                );
            };
            println!("MEMORY HIERARCHY");
            print_cache("L1-I", self.icache_hits, self.icache_misses);
            print_cache("L1-D", self.dcache_hits, self.dcache_misses);
            print_cache("L2", self.l2_hits, self.l2_misses);
            print_cache("L3", self.l3_hits, self.l3_misses);
        }
        println!("==========================================================");
    }

    /// Prints all statistics sections to stdout.
    ///
    /// Equivalent to `print_sections(&[])`.
    pub fn print(&self) {
        self.print_sections(&[]);
    }
}
