//! Memory Access Helpers.
//!
//! This module provides the interface between the CPU and the memory subsystem.
//! It performs the following:
//! 1. **Address Translation:** Interfaces with the MMU to convert virtual to physical addresses.
//! 2. **Cache Simulation:** Models the behavior of L1, L2, and L3 caches during memory access.
//! 3. **Latency Modeling:** Calculates timing penalties for cache hits, misses, and bus transit.

use super::Cpu;
use crate::common::{AccessType, PhysAddr, TranslationResult, Trap, VirtAddr};
use crate::config::InclusionPolicy;
use crate::core::units::mmu::pmp::PmpResult;

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
    pub fn translate(&mut self, vaddr: VirtAddr, access: AccessType, size: u64) -> TranslationResult {
        if self.direct_mode {
            let paddr = PhysAddr::new(vaddr.val());
            if !self.bus.bus.is_valid_address(paddr) {
                let trap = match access {
                    AccessType::Fetch => Trap::InstructionAccessFault(vaddr.val()),
                    AccessType::Read => Trap::LoadAccessFault(vaddr.val()),
                    AccessType::Write => Trap::StoreAccessFault(vaddr.val()),
                };
                return TranslationResult::fault(trap, 0);
            }
            return TranslationResult::success(paddr, 0);
        }

        // MPRV: when set and access is not Fetch, use MPP as effective privilege.
        let effective_priv = if access != AccessType::Fetch
            && (self.csrs.mstatus & crate::core::arch::csr::MSTATUS_MPRV) != 0
        {
            use crate::core::arch::csr::{MSTATUS_MPP_MASK, MSTATUS_MPP_SHIFT};
            use crate::core::arch::mode::PrivilegeMode;
            let mpp = ((self.csrs.mstatus >> MSTATUS_MPP_SHIFT) & MSTATUS_MPP_MASK) as u8;
            PrivilegeMode::from_u8(mpp)
        } else {
            self.privilege
        };

        let result = self.mmu.translate_with_pmp(
            vaddr,
            access,
            effective_priv,
            &self.csrs,
            &mut self.bus.bus,
            Some(&self.pmp),
        );

        // PMP check on the translated physical address.
        // PMP applies to all privilege modes: M-mode with no matching entry gets Allow,
        // S/U-mode with no matching entry gets NoMatch (denied).
        if result.trap.is_none() {
            let paddr = result.paddr.val();
            let is_machine = effective_priv == crate::core::arch::mode::PrivilegeMode::Machine;
            let pmp_result = self.pmp.check(
                paddr,
                size,
                matches!(access, AccessType::Read),
                matches!(access, AccessType::Write),
                matches!(access, AccessType::Fetch),
                is_machine,
            );
            if pmp_result != PmpResult::Allow {
                let trap = match access {
                    AccessType::Fetch => Trap::InstructionAccessFault(vaddr.val()),
                    AccessType::Read => Trap::LoadAccessFault(vaddr.val()),
                    AccessType::Write => Trap::StoreAccessFault(vaddr.val()),
                };
                return TranslationResult::fault(trap, result.cycles);
            }
        }

        result
    }

    /// Computes the total latency for an L1D miss, walking L2 → L3 → DRAM.
    ///
    /// Does NOT modify the L1D cache. The caller (MSHR) is responsible for
    /// installing the L1D line when the miss completes. L2/L3 accesses are
    /// synchronous (blocking) — only L1D gets MSHRs.
    pub fn simulate_l1d_miss_latency(&mut self, addr: PhysAddr, access: AccessType) -> u64 {
        let mut total_penalty = 0;
        let raw_addr = addr.val();
        let ram_latency = self.bus.mem_controller.access_latency(raw_addr, self.csrs.cycle);
        let next_lat = ram_latency;
        let is_write = matches!(access, AccessType::Write);
        let inclusion = self.inclusion_policy;

        if self.l2_cache.enabled {
            total_penalty += self.l2_cache.latency;
            let (l2_hit, l2_pen, l2_evictions, l2_prefetches) =
                self.l2_cache.access_tracked_split(raw_addr, is_write, next_lat);
            total_penalty += l2_pen;

            // Filter and install L2 prefetch candidates through the shared filter
            let filtered = self
                .prefetch_filter
                .filter_and_record(l2_prefetches, &mut self.stats.prefetch_filter_dedup);
            let pf_evictions = self.l2_cache.install_prefetches(&filtered, next_lat);

            // Inclusive policy: L2 eviction → back-invalidate matching L1D/L1I lines
            if inclusion == InclusionPolicy::Inclusive {
                for ev in l2_evictions.iter().chain(pf_evictions.iter()) {
                    if self.l1_d_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                    if self.l1_i_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                }
            }

            if l2_hit {
                self.stats.l2_hits += 1;
                return total_penalty;
            }
            self.stats.l2_misses += 1;
        }

        if self.l3_cache.enabled {
            total_penalty += self.l3_cache.latency;
            let (l3_hit, l3_pen, l3_evictions, l3_prefetches) =
                self.l3_cache.access_tracked_split(raw_addr, is_write, next_lat);
            total_penalty += l3_pen;

            // Filter and install L3 prefetch candidates
            let filtered = self
                .prefetch_filter
                .filter_and_record(l3_prefetches, &mut self.stats.prefetch_filter_dedup);
            let pf_evictions = self.l3_cache.install_prefetches(&filtered, next_lat);

            // Inclusive policy: L3 eviction → back-invalidate L2, L1D, L1I
            if inclusion == InclusionPolicy::Inclusive {
                for ev in l3_evictions.iter().chain(pf_evictions.iter()) {
                    let _ = self.l2_cache.invalidate_line(ev.addr);
                    if self.l1_d_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                    if self.l1_i_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                }
            }

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
        let ram_latency = self.bus.mem_controller.access_latency(raw_addr, self.csrs.cycle);
        let next_lat = ram_latency;
        let is_inst = matches!(access, AccessType::Fetch);
        let is_write = matches!(access, AccessType::Write);
        let inclusion = self.inclusion_policy;

        // Determine which L1 cache applies
        let l1_enabled = if is_inst { self.l1_i_cache.enabled } else { self.l1_d_cache.enabled };

        // If no cache level is enabled, every access goes directly to DRAM.
        // The access still pays bus transit + DRAM latency — disabling
        // caches should make performance worse, not give free memory.
        if !l1_enabled && !self.l2_cache.enabled && !self.l3_cache.enabled {
            let penalty = self.bus.bus.calculate_transit_time(8)
                + ram_latency
                + self.bus.bus.calculate_transit_time(64);
            return penalty;
        }

        // L1 access with eviction tracking + prefetch candidates split out
        let (l1_hit, l1_pen, l1_evictions, l1_prefetches) = if is_inst {
            if self.l1_i_cache.enabled {
                self.l1_i_cache.access_tracked_split(raw_addr, false, next_lat)
            } else {
                (false, 0, Vec::new(), Vec::new())
            }
        } else if self.l1_d_cache.enabled {
            self.l1_d_cache.access_tracked_split(raw_addr, is_write, next_lat)
        } else {
            (false, 0, Vec::new(), Vec::new())
        };

        // Filter L1 prefetch candidates through the shared filter, then install
        let filtered_l1 = self
            .prefetch_filter
            .filter_and_record(l1_prefetches, &mut self.stats.prefetch_filter_dedup);
        let l1_pf_evictions = if is_inst {
            self.l1_i_cache.install_prefetches(&filtered_l1, next_lat)
        } else {
            self.l1_d_cache.install_prefetches(&filtered_l1, next_lat)
        };

        // Combine demand evictions and prefetch evictions for policy handling
        // Exclusive policy: L1 eviction → install evicted line into L2
        if inclusion == InclusionPolicy::Exclusive && self.l2_cache.enabled {
            for ev in l1_evictions.iter().chain(l1_pf_evictions.iter()) {
                let _ = self.l2_cache.install_or_replace(ev.addr, ev.dirty, next_lat);
                self.stats.exclusive_l1_to_l2_swaps += 1;
            }
        }

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
            let (l2_hit, l2_pen, l2_evictions, l2_prefetches) =
                self.l2_cache.access_tracked_split(raw_addr, is_write, next_lat);
            total_penalty += l2_pen;

            // Filter and install L2 prefetch candidates
            let filtered_l2 = self
                .prefetch_filter
                .filter_and_record(l2_prefetches, &mut self.stats.prefetch_filter_dedup);
            let l2_pf_evictions = self.l2_cache.install_prefetches(&filtered_l2, next_lat);

            // Inclusive policy: L2 eviction → back-invalidate L1 lines
            if inclusion == InclusionPolicy::Inclusive {
                for ev in l2_evictions.iter().chain(l2_pf_evictions.iter()) {
                    if self.l1_d_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                    if self.l1_i_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                }
            }

            // Exclusive policy: on L2 hit, remove from L2 (data moves to L1 exclusively)
            if inclusion == InclusionPolicy::Exclusive && l2_hit {
                let _ = self.l2_cache.invalidate_line(raw_addr);
            }

            if l2_hit {
                self.stats.l2_hits += 1;
                return total_penalty;
            }
            self.stats.l2_misses += 1;
        }

        if self.l3_cache.enabled {
            total_penalty += self.l3_cache.latency;
            let (l3_hit, l3_pen, l3_evictions, l3_prefetches) =
                self.l3_cache.access_tracked_split(raw_addr, is_write, next_lat);
            total_penalty += l3_pen;

            // Filter and install L3 prefetch candidates
            let filtered_l3 = self
                .prefetch_filter
                .filter_and_record(l3_prefetches, &mut self.stats.prefetch_filter_dedup);
            let l3_pf_evictions = self.l3_cache.install_prefetches(&filtered_l3, next_lat);

            // Inclusive policy: L3 eviction → back-invalidate L2, L1D, L1I
            if inclusion == InclusionPolicy::Inclusive {
                for ev in l3_evictions.iter().chain(l3_pf_evictions.iter()) {
                    let _ = self.l2_cache.invalidate_line(ev.addr);
                    if self.l1_d_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                    if self.l1_i_cache.invalidate_line(ev.addr) {
                        self.stats.inclusion_back_invalidations += 1;
                    }
                }
            }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::soc::builder::System;

    #[test]
    fn test_translate_direct_mode() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        // RAM_BASE = 0x8000_0000 by default. It's a valid address.
        let result = cpu.translate(VirtAddr::new(0x8000_0000), AccessType::Read, 4);
        assert_eq!(result.paddr.val(), 0x8000_0000);
        assert!(result.trap.is_none());

        // Test invalid address translation trap in direct mode
        let result = cpu.translate(VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF), AccessType::Fetch, 4);
        assert!(result.trap.is_some());
    }

    #[test]
    fn test_simulate_memory_access_no_caches() {
        let mut config = Config::default();
        config.cache.l1_i.enabled = false;
        config.cache.l1_d.enabled = false;
        config.cache.l2.enabled = false;
        config.cache.l3.enabled = false;

        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        let penalty = cpu.simulate_memory_access(PhysAddr::new(0x8000_0000), AccessType::Read);
        assert!(penalty > 0);
    }
}
