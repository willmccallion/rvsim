#[derive(Default, Debug)]
pub struct SimStats {
    pub cycles: u64,
    pub instructions_retired: u64,
    pub branch_predictions: u64,
    pub branch_mispredictions: u64,

    pub cycles_user: u64,
    pub cycles_kernel: u64,
    pub cycles_machine: u64,

    pub stalls_mem: u64,
    pub stalls_control: u64,
    pub stalls_data: u64,

    pub inst_load: u64,
    pub inst_store: u64,
    pub inst_branch: u64,
    pub inst_alu: u64,
    pub inst_system: u64,

    pub icache_hits: u64,
    pub icache_misses: u64,
    pub dcache_hits: u64,
    pub dcache_misses: u64,

    pub l2_hits: u64,
    pub l2_misses: u64,

    pub l3_hits: u64,
    pub l3_misses: u64,
}

impl SimStats {
    pub fn print(&self) {
        println!("\n=========================================================");

        println!("\n[General]");
        println!("  Cycles:               {}", self.cycles);
        println!("  Instructions Retired: {}", self.instructions_retired);

        let ipc = if self.cycles > 0 {
            self.instructions_retired as f64 / self.cycles as f64
        } else {
            0.0
        };
        println!("  IPC:                  {:.4}", ipc);

        println!("\n[Execution Time Distribution]");
        let total_cycles = self.cycles as f64;
        if total_cycles > 0.0 {
            println!(
                "  User Mode:            {:<10} ({:.2}%)",
                self.cycles_user,
                (self.cycles_user as f64 / total_cycles) * 100.0
            );
            println!(
                "  Kernel (Supervisor):  {:<10} ({:.2}%)",
                self.cycles_kernel,
                (self.cycles_kernel as f64 / total_cycles) * 100.0
            );
            println!(
                "  Bootloader (Machine): {:<10} ({:.2}%)",
                self.cycles_machine,
                (self.cycles_machine as f64 / total_cycles) * 100.0
            );
        }

        println!("\n[Pipeline Stalls]");
        let total_stalls = self.stalls_mem + self.stalls_control + self.stalls_data;
        if total_stalls > 0 {
            println!("  Total Stalled Cycles: {}", total_stalls);
            println!(
                "    Memory Latency:     {:<10} ({:.2}%)",
                self.stalls_mem,
                (self.stalls_mem as f64 / total_stalls as f64) * 100.0
            );
            println!(
                "    Control Hazards:    {:<10} ({:.2}%)",
                self.stalls_control,
                (self.stalls_control as f64 / total_stalls as f64) * 100.0
            );
            println!(
                "    Data Hazards:       {:<10} ({:.2}%)",
                self.stalls_data,
                (self.stalls_data as f64 / total_stalls as f64) * 100.0
            );
        } else {
            println!("  Total Stalled Cycles: 0");
        }

        println!("\n[Instruction Mix]");
        let total_inst = self.instructions_retired as f64;
        if total_inst > 0.0 {
            println!(
                "  ALU Operations:       {:<10} ({:.2}%)",
                self.inst_alu,
                (self.inst_alu as f64 / total_inst) * 100.0
            );
            println!(
                "  Loads:                {:<10} ({:.2}%)",
                self.inst_load,
                (self.inst_load as f64 / total_inst) * 100.0
            );
            println!(
                "  Stores:               {:<10} ({:.2}%)",
                self.inst_store,
                (self.inst_store as f64 / total_inst) * 100.0
            );
            println!(
                "  Branches/Jumps:       {:<10} ({:.2}%)",
                self.inst_branch,
                (self.inst_branch as f64 / total_inst) * 100.0
            );
            println!(
                "  System:               {:<10} ({:.2}%)",
                self.inst_system,
                (self.inst_system as f64 / total_inst) * 100.0
            );
        }

        println!("\n[Branch Prediction]");
        let total_branches = self.branch_predictions;
        if total_branches > 0 {
            let accuracy = 1.0 - (self.branch_mispredictions as f64 / total_branches as f64);
            println!(
                "  Accuracy:             {:.2}% ({} / {})",
                accuracy * 100.0,
                total_branches - self.branch_mispredictions,
                total_branches
            );
        } else {
            println!("  No branches executed.");
        }

        println!("\n[Memory Hierarchy]");
        let print_cache = |name: &str, hits: u64, misses: u64| {
            let total = hits + misses;
            if total > 0 {
                let rate = hits as f64 / total as f64;
                println!(
                    "  {:<20} {:.2}% hit rate ({} / {})",
                    name,
                    rate * 100.0,
                    hits,
                    total
                );
            } else {
                println!("  {:<20} No Accesses", name);
            }
        };

        print_cache("L1 I-Cache:", self.icache_hits, self.icache_misses);
        print_cache("L1 D-Cache:", self.dcache_hits, self.dcache_misses);
        print_cache("L2 Cache:", self.l2_hits, self.l2_misses);
        print_cache("L3 Cache:", self.l3_hits, self.l3_misses);
        println!("  {:<20} {} accesses", "DRAM:", self.l3_misses);

        println!("=========================================================\n");
    }
}
