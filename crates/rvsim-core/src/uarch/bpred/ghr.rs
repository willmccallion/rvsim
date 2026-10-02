//! Global history register.

/// Maximum number of u64 words in a GHR. 16 × 64 = 1024 bits.
/// This is a capacity bound — the effective history length comes from config.
const GHR_MAX_WORDS: usize = 16;

/// Global History Register snapshot.
///
/// A fixed-capacity shift register that stores branch outcome history.
/// The effective history length (`len`) is determined by the predictor
/// configuration (e.g., `max(hist_lengths)` for TAGE), while the storage
/// capacity is bounded at compile time at `GHR_MAX_WORDS` × 64 = 1024 bits.
///
/// Bit 0 is the most recently pushed outcome. Snapshots are captured at
/// fetch time and carried through pipeline latches so that update and
/// repair operations use the correct history state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ghr {
    /// Bit storage. `bits[0]` bit 0 = position 0 (most recent).
    /// `bits[0]` bit 63 = position 63, `bits[1]` bit 0 = position 64, etc.
    bits: [u64; GHR_MAX_WORDS],
    /// Effective history length in bits, from config. Bits beyond this are
    /// masked off on push and ignored by consumers.
    len: u16,
}

impl Default for Ghr {
    fn default() -> Self {
        Self { bits: [0; GHR_MAX_WORDS], len: 0 }
    }
}

impl Ghr {
    /// Creates a zero-initialized GHR with the given effective history length.
    ///
    /// # Panics
    ///
    /// Panics if `max_bits` exceeds the compile-time capacity (1024 bits).
    pub fn with_len(max_bits: usize) -> Self {
        assert!(
            max_bits <= GHR_MAX_WORDS * 64,
            "GHR: requested {max_bits} bits exceeds capacity of {} bits",
            GHR_MAX_WORDS * 64,
        );
        Self { bits: [0; GHR_MAX_WORDS], len: max_bits as u16 }
    }

    #[cfg(test)]
    /// Returns the low 64 bits of the GHR.
    ///
    /// Backward-compatible accessor for predictors that only use 64 bits.
    #[inline]
    pub const fn val(&self) -> u64 {
        self.bits[0]
    }

    /// Returns the raw u64 word at the given index into the backing store.
    ///
    /// Returns 0 for out-of-bounds indices. Useful for word-level algorithms
    /// that process the GHR in 64-bit chunks.
    #[inline]
    pub const fn word(&self, idx: usize) -> u64 {
        if idx < GHR_MAX_WORDS { self.bits[idx] } else { 0 }
    }

    /// Returns the bit at position `pos` (0 = most recent outcome).
    ///
    /// Returns `false` for positions beyond storage capacity.
    #[inline]
    pub const fn bit(&self, pos: usize) -> bool {
        let word_idx = pos / 64;
        let bit_idx = pos % 64;
        if word_idx >= GHR_MAX_WORDS {
            return false;
        }
        (self.bits[word_idx] >> bit_idx) & 1 != 0
    }

    /// Pushes a new branch outcome into the register (left shift by 1).
    ///
    /// All positions shift up by 1 (position K moves to K+1). The new
    /// outcome is inserted at position 0. Bits beyond `len` are masked off.
    pub fn push(&mut self, taken: bool) {
        for i in (1..GHR_MAX_WORDS).rev() {
            self.bits[i] = (self.bits[i] << 1) | (self.bits[i - 1] >> 63);
        }
        self.bits[0] = (self.bits[0] << 1) | (taken as u64);

        let len = self.len as usize;
        if len > 0 && len < GHR_MAX_WORDS * 64 {
            let top_word_idx = len / 64;
            let top_bit_count = len % 64;
            if top_word_idx < GHR_MAX_WORDS {
                if top_bit_count > 0 {
                    self.bits[top_word_idx] &= (1u64 << top_bit_count) - 1;
                } else {
                    self.bits[top_word_idx] = 0;
                }
                for w in &mut self.bits[top_word_idx + 1..] {
                    *w = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ghr_with_len() {
        let ghr = Ghr::with_len(712);
        assert_eq!(ghr.len, 712);
        assert_eq!(ghr.val(), 0);
    }

    #[test]
    fn test_ghr_push_and_bit() {
        let mut ghr = Ghr::with_len(128);
        ghr.push(true);
        ghr.push(false);
        ghr.push(true);
        assert!(ghr.bit(0)); // newest = true
        assert!(!ghr.bit(1)); // second = false
        assert!(ghr.bit(2)); // oldest = true
        assert!(!ghr.bit(3)); // never pushed = false
    }

    #[test]
    fn test_ghr_push_across_word_boundary() {
        let mut ghr = Ghr::with_len(128);
        for i in 0..65 {
            ghr.push(i == 0); // only the first push is true, now at position 64
        }
        assert!(ghr.bit(64));
        assert!(!ghr.bit(63));
        assert!(!ghr.bit(65));
    }

    #[test]
    fn test_ghr_push_masks_to_len() {
        let mut ghr = Ghr::with_len(5);
        for _ in 0..10 {
            ghr.push(true);
        }
        assert!(ghr.bit(0));
        assert!(ghr.bit(4));
        assert!(!ghr.bit(5)); // masked off
    }

    #[test]
    fn test_ghr_default() {
        let ghr = Ghr::default();
        assert_eq!(ghr.len, 0);
        assert_eq!(ghr.val(), 0);
        assert!(!ghr.bit(0));
    }

    #[test]
    fn test_ghr_copy_semantics() {
        let mut ghr = Ghr::with_len(64);
        ghr.push(true);
        let snapshot = ghr; // Copy
        ghr.push(false); // mutate original
        assert!(snapshot.bit(0)); // snapshot unchanged
        assert!(!ghr.bit(0)); // original has the new push
    }
}
