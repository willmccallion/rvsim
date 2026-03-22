//! Simulation statistics collection and reporting.
//!
//! This module tracks performance metrics for the RISC-V simulator. It provides:
//! 1. **Cycle and IPC:** Total cycles, retired instructions, and derived metrics (CPI, MIPS).
//! 2. **Instruction mix:** Counts by category (ALU, load, store, branch, system, FP).
//! 3. **Branch prediction:** Lookups, mispredictions, and accuracy.
//! 4. **Stalls:** Memory, control, and data hazard stall counts.
//! 5. **Cache hierarchy:** Hit/miss counts for L1-I, L1-D, L2, and L3.

use crate::core::pipeline::backend::o3::fu_pool::FU_TYPE_COUNT;
use std::io::IsTerminal;
use std::time::Instant;

/// Simulation statistics structure tracking all performance metrics.
///
/// Collects detailed statistics about instruction execution, cache behavior,
/// branch prediction, stalls, and execution time for performance analysis.
#[derive(Clone, Debug)]
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

    /// Number of committed branch predictions that were correct.
    pub committed_branch_predictions: u64,
    /// Number of committed branch predictions that were wrong (mispredictions).
    pub committed_branch_mispredictions: u64,

    /// Number of speculative branch predictions (including wrong-path) that were correct.
    pub speculative_branch_predictions: u64,
    /// Number of speculative branch predictions (including wrong-path) that were wrong.
    pub speculative_branch_mispredictions: u64,

    /// Cycles spent in user (U) mode.
    pub cycles_user: u64,
    /// Cycles spent in supervisor (S) mode.
    pub cycles_kernel: u64,
    /// Cycles spent in machine (M) mode.
    pub cycles_machine: u64,
    /// Cycles spent in WFI (wait-for-interrupt) state.
    pub cycles_wfi: u64,

    /// Cycles where the ROB was empty at commit (pipeline draining/refilling after flush).
    pub cycles_rob_empty: u64,

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

    /// FU utilization: count of cycles each `FuType` was executing.
    /// Indexed by `FuType as usize` (see `fu_pool::FU_TYPE_COUNT`).
    pub fu_utilization: [u64; FU_TYPE_COUNT],

    /// Stall cycles where a ready IQ entry could not issue (no free FU).
    pub stalls_fu_structural: u64,

    /// Total ROB entries squashed due to branch mispredictions / ordering violations.
    pub misprediction_penalty: u64,

    /// Cycles where execute-to-memory pipeline is backpressured (`execute_mem1` non-empty).
    pub stalls_backpressure: u64,

    /// Number of memory ordering violations detected (load queue).
    pub mem_ordering_violations: u64,

    /// Number of pipeline flushes (mispredictions + violations + CSR/FENCE redirects).
    pub pipeline_flushes: u64,

    /// MSHR allocations (new L1D cache misses with available MSHR).
    pub mshr_allocations: u64,
    /// MSHR coalesces (miss to same line as an outstanding miss).
    pub mshr_coalesces: u64,
    /// Stalls due to all MSHRs being full.
    pub stalls_mshr_full: u64,
    /// Load replays due to speculative wakeup on L1D miss.
    pub load_replays: u64,

    /// Inclusive policy: L1 lines back-invalidated due to L2/L3 eviction.
    pub inclusion_back_invalidations: u64,
    /// Exclusive policy: L1 evictees installed into L2 (swap).
    pub exclusive_l1_to_l2_swaps: u64,

    /// Write Combining Buffer: stores coalesced into existing WCB entries.
    pub wcb_coalesces: u64,
    /// Write Combining Buffer: entries drained to L1D.
    pub wcb_drains: u64,

    /// Prefetch filter: redundant prefetch requests suppressed (total across all levels).
    pub prefetch_filter_dedup: u64,
    /// Prefetch filter dedup: L1 level.
    pub pf_dedup_l1: u64,
    /// Prefetch filter dedup: L2 level.
    pub pf_dedup_l2: u64,
    /// Prefetch filter dedup: L3 level.
    pub pf_dedup_l3: u64,

    /// Cycles where the frontend had instructions but the backend could not accept them
    /// (ROB/SB/LQ/IQ/PRF full).
    pub stalls_dispatch: u64,

    /// Stall cycles where a branch/jump could not dispatch because the checkpoint table was full.
    pub stalls_checkpoint: u64,

    /// Stall cycles where rename could not dispatch because the rename map was being rebuilt
    /// by walking the ROB (no checkpoint available for this flush).
    pub stalls_rename_rebuild: u64,

    /// Total stall cycles where dispatch is blocked during squash recovery.
    /// Includes both the ROB squash walk (always) and rename rebuild (without checkpoint).
    /// This is the physical cost of rate-limited ROB entry reclamation.
    pub stalls_squash: u64,

    /// Pipeline flushes caused by branch/jump mispredictions.
    pub flushes_branch: u64,
    /// Pipeline flushes caused by serializing instructions (CSR, FENCE.I, MRET/SRET, etc.).
    pub flushes_system: u64,

    /// MDP: predictions that returned Bypass.
    pub mdp_predictions_bypass: u64,
    /// MDP: predictions that returned `WaitAll`.
    pub mdp_predictions_wait_all: u64,
    /// MDP: predictions that returned `WaitFor`.
    pub mdp_predictions_wait_for: u64,
    /// MDP: violations (calls to update).
    pub mdp_violations: u64,

    /// Retirement histogram: how many instructions were retired per cycle.
    /// Index 0 = cycles with 0 retires, 1 = 1 retire, 2 = 2 retires, 3 = 3+ retires.
    pub retire_histogram: [u64; 4],
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
            committed_branch_predictions: 0,
            committed_branch_mispredictions: 0,
            speculative_branch_predictions: 0,
            speculative_branch_mispredictions: 0,
            cycles_user: 0,
            cycles_kernel: 0,
            cycles_machine: 0,
            cycles_wfi: 0,
            cycles_rob_empty: 0,
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
            fu_utilization: [0; FU_TYPE_COUNT],
            stalls_fu_structural: 0,
            misprediction_penalty: 0,
            stalls_backpressure: 0,
            mem_ordering_violations: 0,
            pipeline_flushes: 0,
            mshr_allocations: 0,
            mshr_coalesces: 0,
            stalls_mshr_full: 0,
            load_replays: 0,
            inclusion_back_invalidations: 0,
            exclusive_l1_to_l2_swaps: 0,
            wcb_coalesces: 0,
            wcb_drains: 0,
            prefetch_filter_dedup: 0,
            pf_dedup_l1: 0,
            pf_dedup_l2: 0,
            pf_dedup_l3: 0,
            stalls_dispatch: 0,
            stalls_checkpoint: 0,
            stalls_rename_rebuild: 0,
            stalls_squash: 0,
            flushes_branch: 0,
            flushes_system: 0,
            mdp_predictions_bypass: 0,
            mdp_predictions_wait_all: 0,
            mdp_predictions_wait_for: 0,
            mdp_violations: 0,
            retire_histogram: [0; 4],
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
        let color = std::io::stdout().is_terminal();
        let bold = if color { "\x1b[1m" } else { "" };
        let teal = if color { "\x1b[36m" } else { "" };
        let dim = if color { "\x1b[2m" } else { "" };
        let rst = if color { "\x1b[0m" } else { "" };

        let want = |s: &str| sections.is_empty() || sections.iter().any(|x| x == s);
        let duration = self.start_time.elapsed();
        let seconds = duration.as_secs_f64();
        let cyc = if self.cycles == 0 { 1 } else { self.cycles };
        let instr = if self.instructions_retired == 0 { 1 } else { self.instructions_retired };

        let rule =
            format!("{bold}{teal}=========================================================={rst}");
        let sep = format!("{dim}----------------------------------------------------------{rst}");

        if want("summary") {
            let ipc = self.instructions_retired as f64 / cyc as f64;
            let cpi = cyc as f64 / instr as f64;
            let mips = (self.instructions_retired as f64 / seconds) / 1_000_000.0;
            let khz = (self.cycles as f64 / seconds) / 1000.0;
            let active_cycles = cyc.saturating_sub(self.cycles_wfi);
            let active_cyc = if active_cycles == 0 { 1 } else { active_cycles };
            let active_ipc = self.instructions_retired as f64 / active_cyc as f64;
            println!("\n{rule}");
            println!("{bold}RISC-V SYSTEM SIMULATION STATISTICS{rst}");
            println!("{rule}");
            println!("host_seconds             {seconds:.4} s");
            println!("sim_cycles               {}", self.cycles);
            println!("sim_freq                 {khz:.2} kHz");
            println!("sim_insts                {}", self.instructions_retired);
            println!("sim_ipc                  {ipc:.4}");
            if self.cycles_wfi > 0 {
                println!("sim_ipc_active           {active_ipc:.4}");
            }
            println!("sim_cpi                  {cpi:.4}");
            println!("sim_mips                 {mips:.2}");
            println!("{sep}");
        }
        if want("core") {
            // Cycle accounting: commit-side view (sums to ~sim_cycles).
            let rh = &self.retire_histogram;
            let rh_total = rh[0] + rh[1] + rh[2] + rh[3];
            let cycles_retiring = rh[1] + rh[2] + rh[3];

            println!("{bold}CYCLE ACCOUNTING{rst}");
            println!(
                "  cycles.retiring        {} ({:.2}%)",
                cycles_retiring,
                (cycles_retiring as f64 / cyc as f64) * 100.0
            );
            println!(
                "  cycles.rob_empty       {} ({:.2}%)",
                self.cycles_rob_empty,
                (self.cycles_rob_empty as f64 / cyc as f64) * 100.0
            );
            // Cycles where ROB had entries but head was not ready (long-latency
            // ops in flight, data/memory stalls upstream of commit).
            let cycles_rob_stall =
                rh[0].saturating_sub(self.cycles_rob_empty).saturating_sub(self.cycles_wfi);
            println!(
                "  cycles.rob_stall       {} ({:.2}%)",
                cycles_rob_stall,
                (cycles_rob_stall as f64 / cyc as f64) * 100.0
            );
            if self.cycles_wfi > 0 {
                println!(
                    "  cycles.wfi             {} ({:.2}%)",
                    self.cycles_wfi,
                    (self.cycles_wfi as f64 / cyc as f64) * 100.0
                );
            }
            if rh_total > 0 {
                let pct = |v: u64| (v as f64 / rh_total as f64) * 100.0;
                println!(
                    "  retire.per_cycle       0:{:.1}%  1:{:.1}%  2:{:.1}%  3+:{:.1}%",
                    pct(rh[0]),
                    pct(rh[1]),
                    pct(rh[2]),
                    pct(rh[3]),
                );
                // Show active retire distribution (excluding WFI idle cycles)
                if self.cycles_wfi > 0 {
                    let active_total = rh_total.saturating_sub(self.cycles_wfi);
                    if active_total > 0 {
                        let active_zero = rh[0].saturating_sub(self.cycles_wfi);
                        let apct = |v: u64| (v as f64 / active_total as f64) * 100.0;
                        println!(
                            "  retire.active          0:{:.1}%  1:{:.1}%  2:{:.1}%  3+:{:.1}%",
                            apct(active_zero),
                            apct(rh[1]),
                            apct(rh[2]),
                            apct(rh[3]),
                        );
                    }
                }
            }
            println!("{sep}");

            println!("{bold}PRIVILEGE BREAKDOWN{rst}");
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
            println!("{sep}");

            println!("{bold}PIPELINE STALLS{rst}");
            // Memory stalls: blocking-mode stalls + MSHR-full stalls (non-blocking mode).
            // With MSHRs enabled, cache miss latency is hidden by the non-blocking cache;
            // remaining memory stalls come from MSHR exhaustion.
            let total_mem_stalls = self.stalls_mem + self.stalls_mshr_full;
            println!(
                "  stalls.memory          {} ({:.2}%)",
                total_mem_stalls,
                (total_mem_stalls as f64 / cyc as f64) * 100.0
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
            println!(
                "  stalls.fu_structural   {} ({:.2}%)",
                self.stalls_fu_structural,
                (self.stalls_fu_structural as f64 / cyc as f64) * 100.0
            );
            println!(
                "  stalls.backpressure    {} ({:.2}%)",
                self.stalls_backpressure,
                (self.stalls_backpressure as f64 / cyc as f64) * 100.0
            );
            if self.stalls_dispatch > 0 {
                println!(
                    "  stalls.dispatch        {} ({:.2}%)",
                    self.stalls_dispatch,
                    (self.stalls_dispatch as f64 / cyc as f64) * 100.0
                );
            }
            if self.stalls_checkpoint > 0 {
                println!(
                    "  stalls.checkpoint      {} ({:.2}%)",
                    self.stalls_checkpoint,
                    (self.stalls_checkpoint as f64 / cyc as f64) * 100.0
                );
            }
            if self.stalls_squash > 0 {
                println!(
                    "  stalls.squash          {} ({:.2}%)",
                    self.stalls_squash,
                    (self.stalls_squash as f64 / cyc as f64) * 100.0
                );
            }
            if self.stalls_rename_rebuild > 0 {
                println!(
                    "  stalls.rename_rebuild  {} ({:.2}%)",
                    self.stalls_rename_rebuild,
                    (self.stalls_rename_rebuild as f64 / cyc as f64) * 100.0
                );
            }
            println!("{sep}");
        }
        if want("instruction_mix") {
            let total_inst = instr as f64;
            let fp_total = self.inst_fp_load
                + self.inst_fp_store
                + self.inst_fp_arith
                + self.inst_fp_fma
                + self.inst_fp_div_sqrt;
            println!("{bold}INSTRUCTION MIX{rst}");
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
            if fp_total > 0 {
                println!(
                    "  op.fp                  {} ({:.2}%)",
                    fp_total,
                    (fp_total as f64 / total_inst) * 100.0
                );
                if self.inst_fp_load > 0 {
                    println!("    fp.load              {}", self.inst_fp_load);
                }
                if self.inst_fp_store > 0 {
                    println!("    fp.store             {}", self.inst_fp_store);
                }
                if self.inst_fp_arith > 0 {
                    println!("    fp.arith             {}", self.inst_fp_arith);
                }
                if self.inst_fp_fma > 0 {
                    println!("    fp.fma               {}", self.inst_fp_fma);
                }
                if self.inst_fp_div_sqrt > 0 {
                    println!("    fp.div_sqrt          {}", self.inst_fp_div_sqrt);
                }
            }
            println!("{sep}");
        }
        if want("branch") {
            let bp_correct = self.committed_branch_predictions;
            let bp_miss = self.committed_branch_mispredictions;
            let bp_total = bp_correct + bp_miss;
            let bp_acc =
                if bp_total > 0 { 100.0 * (bp_correct as f64 / bp_total as f64) } else { 0.0 };

            let spec_total =
                self.speculative_branch_predictions + self.speculative_branch_mispredictions;
            let spec_miss = self.speculative_branch_mispredictions;
            let spec_acc = if spec_total > 0 {
                100.0 * (self.speculative_branch_predictions as f64 / spec_total as f64)
            } else {
                0.0
            };

            println!("{bold}BRANCH PREDICTION (COMMITTED){rst}");
            println!("  bp.committed_lookups   {bp_total}");
            println!("  bp.committed_mispreds  {bp_miss}");
            println!("  bp.committed_accuracy  {bp_acc:.2}%");
            println!("{sep}");
            println!("{bold}BRANCH PREDICTION (SPECULATIVE){rst}");
            println!("  bp.spec_lookups        {spec_total}");
            println!("  bp.spec_mispredicts    {spec_miss}");
            println!("  bp.spec_accuracy       {spec_acc:.2}%");
            println!("{sep}");
            println!("{bold}PIPELINE FLUSHES & PENALTIES{rst}");
            println!("  flush.total            {}", self.pipeline_flushes);
            if self.flushes_branch > 0 || self.flushes_system > 0 {
                println!("  flush.branch           {}", self.flushes_branch);
                println!("  flush.system           {}", self.flushes_system);
            }
            println!("  flush.mem_violations   {}", self.mem_ordering_violations);
            println!("  flush.squashed_insns   {}", self.misprediction_penalty);
            let mdp_total = self.mdp_predictions_bypass
                + self.mdp_predictions_wait_all
                + self.mdp_predictions_wait_for;
            if mdp_total > 0 {
                println!("{sep}");
                println!("{bold}MEMORY DEPENDENCE PREDICTION{rst}");
                println!("  mdp.predictions        {mdp_total}");
                println!("  mdp.bypass             {}", self.mdp_predictions_bypass);
                println!("  mdp.wait_all           {}", self.mdp_predictions_wait_all);
                println!("  mdp.wait_for           {}", self.mdp_predictions_wait_for);
                println!("  mdp.violations         {}", self.mdp_violations);
            }
            println!("{sep}");
        }
        if want("memory") {
            let print_cache = |name: &str, hits: u64, misses: u64| {
                let total = hits + misses;
                let rate = if total > 0 { (hits as f64 / total as f64) * 100.0 } else { 0.0 };
                println!(
                    "  {:<6} accesses: {:<10} | hits: {:<10} | miss_rate: {:.2}%",
                    name,
                    total,
                    hits,
                    100.0 - rate
                );
            };
            println!("{bold}MEMORY HIERARCHY{rst}");
            print_cache("L1-I", self.icache_hits, self.icache_misses);
            print_cache("L1-D", self.dcache_hits, self.dcache_misses);
            print_cache("L2", self.l2_hits, self.l2_misses);
            print_cache("L3", self.l3_hits, self.l3_misses);
            if self.mshr_allocations > 0 || self.mshr_coalesces > 0 {
                println!(
                    "  mshr.allocs            {} | coalesces: {} | full_stalls: {}",
                    self.mshr_allocations, self.mshr_coalesces, self.stalls_mshr_full
                );
                println!("  load.replays           {}", self.load_replays);
            }
            if self.inclusion_back_invalidations > 0 {
                println!("  incl.back_invalidate   {}", self.inclusion_back_invalidations);
            }
            if self.exclusive_l1_to_l2_swaps > 0 {
                println!("  excl.l1_to_l2_swaps    {}", self.exclusive_l1_to_l2_swaps);
            }
            if self.wcb_coalesces > 0 || self.wcb_drains > 0 {
                println!(
                    "  wcb.coalesces          {} | drains: {}",
                    self.wcb_coalesces, self.wcb_drains
                );
            }
            let pf_total = self.pf_dedup_l1 + self.pf_dedup_l2 + self.pf_dedup_l3;
            // Fall back to the legacy total counter if per-level aren't populated
            let pf_display = if pf_total > 0 { pf_total } else { self.prefetch_filter_dedup };
            if pf_display > 0 {
                println!("  pf_filter.dedup        {pf_display}");
                if pf_total > 0 {
                    println!(
                        "    L1: {}  L2: {}  L3: {}",
                        self.pf_dedup_l1, self.pf_dedup_l2, self.pf_dedup_l3
                    );
                }
            }
        }
        println!("{rule}");
    }

    /// Prints all statistics sections to stdout.
    ///
    /// Equivalent to `print_sections(&[])`.
    pub fn print(&self) {
        self.print_sections(&[]);
    }
}
