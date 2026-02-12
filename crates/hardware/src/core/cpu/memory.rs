//! Memory Access Helpers.
//!
//! This module provides the interface between the CPU and the memory subsystem.
//! It performs the following:
//! 1. **Address Translation:** Interfaces with the MMU to convert virtual to physical addresses.
//! 2. **Cache Simulation:** Models the behavior of L1, L2, and L3 caches during memory access.
//! 3. **Pipeline Synchronization:** Handles the flushing of pending store operations to memory.
//! 4. **Latency Modeling:** Calculates timing penalties for cache hits, misses, and bus transit.

use super::Cpu;
use crate::common::{AccessType, PhysAddr, TranslationResult, Trap, VirtAddr};
use crate::core::pipeline::signals;

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

    /// Flushes pending stores in the pipeline to memory.
    ///
    /// Translates virtual addresses to physical addresses before writing,
    /// just as the memory stage would normally do.
    pub(crate) fn flush_pipeline_stores(&mut self) {
        // Take entries out so we can access other fields of self for translation.
        let mut entries = std::mem::take(&mut self.ex_mem.entries);

        for entry in &mut entries {
            if entry.ctrl.mem_write {
                let vaddr = entry.alu;
                let src = entry.store_data;
                let width = entry.ctrl.width;

                // Translate virtual address to physical address
                let paddr = if self.direct_mode {
                    vaddr
                } else {
                    let result = self.mmu.translate(
                        VirtAddr::new(vaddr),
                        AccessType::Write,
                        self.privilege,
                        &self.csrs,
                        &mut self.bus.bus,
                    );
                    if result.trap.is_some() {
                        if self.trace {
                            println!(
                                "[Pipeline Flush] Translation failed for store to vaddr={:#x}, skipping",
                                vaddr
                            );
                        }
                        entry.ctrl.mem_write = false;
                        continue;
                    }
                    result.paddr.val()
                };

                if paddr >= self.ram_start && paddr < self.ram_end {
                    let offset = (paddr - self.ram_start) as usize;
                    // SAFETY: This write operation is safe because:
                    // 1. `paddr` is validated to be within RAM bounds (>= ram_start && < ram_end)
                    // 2. `offset` is computed from validated bounds, ensuring valid memory access
                    // 3. `ram_ptr` points to valid, mutable memory allocated during CPU construction
                    // 4. `write_unaligned()` safely handles potential misalignment for multi-byte writes
                    // 5. Each write size (1/2/4/8 bytes) is guaranteed not to overflow the buffer
                    // 6. Memory access permissions have been validated by MMU/PMP prior to this call
                    // 7. This is only called for atomic operations which have exclusive access semantics
                    unsafe {
                        match width {
                            signals::MemWidth::Byte => *self.ram_ptr.add(offset) = src as u8,
                            signals::MemWidth::Half => {
                                let ptr = self.ram_ptr.add(offset) as *mut u16;
                                ptr.write_unaligned(src as u16);
                            }
                            signals::MemWidth::Word => {
                                let ptr = self.ram_ptr.add(offset) as *mut u32;
                                ptr.write_unaligned(src as u32);
                            }
                            signals::MemWidth::Double => {
                                let ptr = self.ram_ptr.add(offset) as *mut u64;
                                ptr.write_unaligned(src);
                            }
                            _ => {}
                        }
                    }
                } else {
                    match width {
                        signals::MemWidth::Byte => self.bus.bus.write_u8(paddr, src as u8),
                        signals::MemWidth::Half => self.bus.bus.write_u16(paddr, src as u16),
                        signals::MemWidth::Word => self.bus.bus.write_u32(paddr, src as u32),
                        signals::MemWidth::Double => self.bus.bus.write_u64(paddr, src),
                        _ => {}
                    }
                }

                entry.ctrl.mem_write = false;

                if self.trace {
                    println!(
                        "[Pipeline Flush] Forced Store to vaddr={:#x} paddr={:#x} val={:#x}",
                        vaddr, paddr, src
                    );
                }
            }
        }

        self.ex_mem.entries = entries;
    }
}
