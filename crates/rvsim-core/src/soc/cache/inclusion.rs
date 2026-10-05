//! Inclusion: which upper-level caches hold a line, and back-invalidation.

use crate::common::LineAddr;
use crate::config::InclusionPolicy;
use crate::sim::components::ComponentId;
use crate::sim::handle::HandleCtx;
use crate::sim::packet::Packet;

use super::Cache;

impl Cache {
    pub(super) fn back_invalidate(
        &self,
        line: LineAddr,
        holders: &[ComponentId],
        ctx: &mut HandleCtx<'_>,
    ) {
        if self.upstream_inclusion != InclusionPolicy::Inclusive {
            return;
        }
        for &upstream in holders {
            ctx.scheduler.schedule(
                ctx.cycle,
                upstream,
                ctx.self_id,
                Packet::CacheInval { line_addr: line },
            );
        }
    }

    /// The next level dropped `line`; drop our copy too (writing it back
    /// first if it is dirty) and tell inclusive upper levels.
    pub(super) fn on_back_invalidate(&mut self, line: LineAddr, ctx: &mut HandleCtx<'_>) {
        if !self.enabled {
            return;
        }
        let holders = self.upper_holders(line);
        if self.invalidate_line(line.val(), ctx.stats) {
            self.write_back(line, true, ctx);
        }
        ctx.stats.counter(self.stat_paths.back_invalidations).inc();
        self.back_invalidate(line, &holders, ctx);
    }

    /// The bit `source` holds in a line's `upper` mask; zero when it is not
    /// a cache above this one.
    pub(super) fn upstream_bit(&self, source: ComponentId) -> u8 {
        self.upstream.iter().position(|&up| up == source).map_or(0, |i| 1 << i)
    }

    /// `source` was given the line in `index`.
    pub(super) fn note_upper_copy(&mut self, index: usize, source: ComponentId) {
        self.lines[index].upper |= self.upstream_bit(source);
    }

    /// `source` dropped its copy of the line holding `addr`.
    pub(super) fn forget_upper_copy(&mut self, addr: u64, source: ComponentId) {
        if let Some(way) = self.find_way(addr) {
            let index = self.set_index(addr) * self.ways + way;
            self.lines[index].upper &= !self.upstream_bit(source);
        }
    }

    /// The caches above that may hold `line`: the ones given it while it is
    /// here; every one when it is not and they need not be inclusive (or
    /// this cache is disabled and holds nothing); none when they must be.
    pub(super) fn upper_holders(&self, line: LineAddr) -> Vec<ComponentId> {
        if !self.enabled {
            return self.upstream.clone();
        }
        match self.find_way(line.val()) {
            Some(way) => {
                let bits = self.lines[self.set_index(line.val()) * self.ways + way].upper;
                self.upstream
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits & (1 << i) != 0)
                    .map(|(_, &up)| up)
                    .collect()
            }
            None if self.upstream_inclusion == InclusionPolicy::Inclusive => Vec::new(),
            None => self.upstream.clone(),
        }
    }
}
