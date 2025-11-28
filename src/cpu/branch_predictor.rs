const HISTORY_LENGTH: usize = 32;
const TABLE_BITS: usize = 10;
const TABLE_SIZE: usize = 1 << TABLE_BITS;
const BTB_BITS: usize = 10;
const BTB_SIZE: usize = 1 << BTB_BITS;
const RAS_SIZE: usize = 32;
const THRESHOLD: i32 = 76;

#[derive(Clone, Copy, Default)]
struct BtbEntry {
    tag: u64,
    target: u64,
    valid: bool,
}

pub struct BranchPredictor {
    ghr: u64,
    table: Vec<[i8; HISTORY_LENGTH + 1]>,
    btb: Vec<BtbEntry>,
    ras: [u64; RAS_SIZE],
    ras_depth: usize,
}

impl BranchPredictor {
    pub fn new() -> Self {
        Self {
            ghr: 0,
            table: vec![[0; HISTORY_LENGTH + 1]; TABLE_SIZE],
            btb: vec![BtbEntry::default(); BTB_SIZE],
            ras: [0; RAS_SIZE],
            ras_depth: 0,
        }
    }

    fn index(&self, pc: u64) -> usize {
        let pc_idx = (pc >> 2) & ((TABLE_SIZE as u64) - 1);
        let hist_idx = self.ghr & ((TABLE_SIZE as u64) - 1);
        (pc_idx ^ hist_idx) as usize
    }

    fn output(&self, idx: usize) -> i32 {
        let w = &self.table[idx];
        let mut y = w[0] as i32;
        for i in 0..HISTORY_LENGTH {
            let bit = if (self.ghr >> i) & 1 != 0 { 1 } else { -1 };
            y += (w[i + 1] as i32) * bit;
        }
        y
    }

    fn btb_index(&self, pc: u64) -> usize {
        ((pc >> 2) as usize) & (BTB_SIZE - 1)
    }

    fn btb_lookup(&self, pc: u64) -> Option<u64> {
        let idx = self.btb_index(pc);
        let e = self.btb[idx];
        if e.valid && e.tag == pc {
            Some(e.target)
        } else {
            None
        }
    }

    fn btb_update(&mut self, pc: u64, target: u64) {
        let idx = self.btb_index(pc);
        self.btb[idx] = BtbEntry {
            tag: pc,
            target,
            valid: true,
        };
    }

    fn ras_push(&mut self, ret_addr: u64) {
        if self.ras_depth < RAS_SIZE {
            self.ras[self.ras_depth] = ret_addr;
            self.ras_depth += 1;
        } else {
            self.ras[RAS_SIZE - 1] = ret_addr;
        }
    }

    fn ras_top(&self) -> Option<u64> {
        if self.ras_depth == 0 {
            None
        } else {
            Some(self.ras[self.ras_depth - 1])
        }
    }

    fn ras_pop(&mut self) -> Option<u64> {
        if self.ras_depth == 0 {
            None
        } else {
            self.ras_depth -= 1;
            Some(self.ras[self.ras_depth])
        }
    }

    pub fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        let idx = self.index(pc);
        let y = self.output(idx);
        let taken = y >= 0;
        if taken {
            (true, self.btb_lookup(pc))
        } else {
            (false, None)
        }
    }

    pub fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>) {
        let idx = self.index(pc);
        let y = self.output(idx);
        let t = if taken { 1 } else { -1 };

        if y * t <= THRESHOLD {
            let w = &mut self.table[idx];
            let mut v = w[0] as i32 + t;
            w[0] = clamp_weight(v);
            for i in 0..HISTORY_LENGTH {
                let x = if (self.ghr >> i) & 1 != 0 { 1 } else { -1 };
                v = w[i + 1] as i32 + t * x;
                w[i + 1] = clamp_weight(v);
            }
        }

        self.ghr = ((self.ghr << 1) | if taken { 1 } else { 0 }) & ((1u64 << HISTORY_LENGTH) - 1);

        if let Some(tgt) = target {
            self.btb_update(pc, tgt);
        }
    }

    pub fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb_lookup(pc)
    }

    pub fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        self.ras_push(ret_addr);
        self.btb_update(pc, target);
    }

    pub fn predict_return(&self) -> Option<u64> {
        self.ras_top()
    }

    pub fn on_return(&mut self) {
        let _ = self.ras_pop();
    }
}

fn clamp_weight(v: i32) -> i8 {
    if v > 127 {
        127
    } else if v < -128 {
        -128
    } else {
        v as i8
    }
}
