#[derive(Clone, Default)]
struct CacheLine {
    tag: u64,
    valid: bool,
    dirty: bool,
    last_used: u64,
}

pub struct CacheSim {
    lines: Vec<CacheLine>, // index = (set * ways) + way
    num_sets: usize,
    ways: usize,
    line_bytes: usize,
    access_counter: u64,
}

impl CacheSim {
    pub fn new(size_bytes: usize, line_bytes: usize, ways: usize) -> Self {
        let num_lines = size_bytes / line_bytes;
        let num_sets = num_lines / ways;

        let lines = vec![CacheLine::default(); num_sets * ways];

        Self {
            lines,
            num_sets,
            ways,
            line_bytes,
            access_counter: 0,
        }
    }

    pub fn access(&mut self, addr: u64, is_write: bool, next_level_latency: u64) -> (bool, u64) {
        self.access_counter += 1;
        let set_index = ((addr as usize) / self.line_bytes) % self.num_sets;
        let tag = addr / (self.line_bytes * self.num_sets) as u64;

        let base_idx = set_index * self.ways;

        for i in 0..self.ways {
            let idx = base_idx + i;
            if self.lines[idx].valid && self.lines[idx].tag == tag {
                self.lines[idx].last_used = self.access_counter;
                if is_write {
                    self.lines[idx].dirty = true;
                }
                return (true, 0);
            }
        }

        let mut replace_offset = 0;
        let mut min_lru = u64::MAX;

        for i in 0..self.ways {
            let idx = base_idx + i;
            if !self.lines[idx].valid {
                replace_offset = i;
                break;
            }
            if self.lines[idx].last_used < min_lru {
                min_lru = self.lines[idx].last_used;
                replace_offset = i;
            }
        }

        let victim_idx = base_idx + replace_offset;
        let mut penalty = 0;

        if self.lines[victim_idx].valid && self.lines[victim_idx].dirty {
            penalty += next_level_latency;
        }

        self.lines[victim_idx] = CacheLine {
            tag,
            valid: true,
            dirty: is_write,
            last_used: self.access_counter,
        };

        (false, penalty)
    }
}
