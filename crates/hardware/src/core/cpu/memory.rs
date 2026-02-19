//! Memory Access Helpers.
//!
//! This module provides the interface between the CPU and the memory subsystem.
//! It performs the following:
//! 1. **Address Translation:** Interfaces with the MMU to convert virtual to physical addresses.
//! 2. **Cache Simulation:** Models the behavior of L1, L2, and L3 caches during memory access.
//! 3. **Latency Modeling:** Calculates timing penalties for cache hits, misses, and bus transit.

use super::Cpu;
use crate::common::{AccessType, PhysAddr, TranslationResult, Trap, VirtAddr};

impl Cpu {
    /// Translates a virtual address to a physical address using the MMU.
    ///
    /// # Arguments
    ///
    /// * `vaddr` - The virtual address to translate.
    /// * `access` - The type of memory access (Fetch/Read/Write).
    ///
    /// # Returns
    ///
    /// A `TranslationResult` containing the physical address or a trap if translation fails.
    pub fn translate(&mut self, vaddr: VirtAddr, access: AccessType) -> TranslationResult {
        if self.direct_mode {
            let paddr = vaddr.val();
            if !self.bus.bus.is_valid_address(paddr) {
                let trap = match access {
                    AccessType::Fetch => Trap::InstructionAccessFault(paddr),
                    AccessType::Read => Trap::LoadAccessFault(paddr),
                    AccessType::Write => Trap::StoreAccessFault(paddr),
                };
                return TranslationResult::fault(trap, 0);
            }
            return TranslationResult::success(PhysAddr::new(paddr), 0);
        }

        self.mmu
            .translate(vaddr, access, self.privilege, &self.csrs, &mut self.bus.bus)
    }

    /// Simulates a memory access through the cache hierarchy.
    ///
    /// # Arguments
    ///
    /// * `addr` - The physical address to access.
    /// * `access` - The type of memory access.
    ///
    /// # Returns
    ///
    /// The total latency penalty in cycles for the memory operation.
    pub fn simulate_memory_access(&mut self, addr: PhysAddr, access: AccessType) -> u64 {
        let mut total_penalty = 0;
        let raw_addr = addr.val();
        let ram_latency = self.bus.mem_controller.access_latency(raw_addr);
        let next_lat = ram_latency;
        let is_inst = matches!(access, AccessType::Fetch);
        let is_write = matches!(access, AccessType::Write);

        // Determine which L1 cache applies
        let l1_enabled = if is_inst {
            self.l1_i_cache.enabled
        } else {
            self.l1_d_cache.enabled
        };

        // If no cache level is enabled, there is no memory hierarchy to
        // simulate — the pipeline structural latency is the only cost.
        if !l1_enabled && !self.l2_cache.enabled && !self.l3_cache.enabled {
            return 0;
        }

        let (l1_hit, l1_pen) = if is_inst {
            if self.l1_i_cache.enabled {
                self.l1_i_cache.access(raw_addr, false, next_lat)
            } else {
                (false, 0)
            }
        } else if self.l1_d_cache.enabled {
            self.l1_d_cache.access(raw_addr, is_write, next_lat)
        } else {
            (false, 0)
        };

        total_penalty += l1_pen;
        if is_inst && self.l1_i_cache.enabled {
            if l1_hit {
                self.stats.icache_hits += 1;
                return total_penalty;
            }
            self.stats.icache_misses += 1;
        } else if !is_inst && self.l1_d_cache.enabled {
            if l1_hit {
                self.stats.dcache_hits += 1;
                return total_penalty;
            }
            self.stats.dcache_misses += 1;
        }

        if self.l2_cache.enabled {
            total_penalty += self.l2_cache.latency;
            let (l2_hit, l2_pen) = self.l2_cache.access(raw_addr, is_write, next_lat);
            total_penalty += l2_pen;
            if l2_hit {
                self.stats.l2_hits += 1;
                return total_penalty;
            }
            self.stats.l2_misses += 1;
        }

        if self.l3_cache.enabled {
            total_penalty += self.l3_cache.latency;
            let (l3_hit, l3_pen) = self.l3_cache.access(raw_addr, is_write, next_lat);
            total_penalty += l3_pen;
            if l3_hit {
                self.stats.l3_hits += 1;
                return total_penalty;
            }
            self.stats.l3_misses += 1;
        }

        total_penalty += self.bus.bus.calculate_transit_time(8);
        total_penalty += ram_latency;
        total_penalty += self.bus.bus.calculate_transit_time(64);
        total_penalty
    }
}
