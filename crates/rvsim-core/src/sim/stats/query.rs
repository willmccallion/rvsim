//! Wildcard queries over the stats tree.
//!
//! Two wildcards:
//! - `*` matches any run of characters *within* one segment — never crossing
//!   `.`. `core*.commit.insts` matches `core0.commit.insts`, `core42.commit.insts`;
//!   `*.misses` matches `hits.misses` but not `core0.cache.misses`.
//! - `**` matches any number of full segments including zero.
//!   `**.misses` → every `.misses` leaf at any depth.
//!
//! Segments split on `.`; `*` is a per-segment glob, `**` is the cross-segment
//! escape hatch.

use std::collections::BTreeMap;

/// Result of a [`Stats::query`](super::Stats::query) call.
///
/// Owns the matched `(path, value)` pairs; supports aggregation helpers.
#[derive(Clone, Debug)]
pub struct QueryResult {
    pub(super) matches: Vec<(String, f64)>,
}

impl QueryResult {
    /// Sum of all matched values. Returns 0.0 for an empty result.
    #[must_use]
    pub fn sum(&self) -> f64 {
        self.matches.iter().map(|(_, v)| *v).sum()
    }

    /// Number of matched paths.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.matches.len()
    }

    /// True when no path matched the pattern.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Groups matches by their first path segment.
    ///
    /// Useful for `core*.commit.insts` — returns one entry per core.
    #[must_use]
    pub fn by_subject(&self) -> BTreeMap<String, f64> {
        let mut acc = BTreeMap::new();
        for (path, value) in &self.matches {
            let subject = path.split_once('.').map_or_else(|| path.clone(), |(s, _)| s.to_string());
            *acc.entry(subject).or_insert(0.0) += *value;
        }
        acc
    }

    /// Iterates matched `(path, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        self.matches.iter().map(|(p, v)| (p.as_str(), *v))
    }
}

/// Matches a path against a wildcard pattern.
///
/// `*` matches one segment, `**` matches zero or more segments. Literal
/// segments must match verbatim.
pub(super) fn matches(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('.').collect();
    let seg: Vec<&str> = path.split('.').collect();
    match_slices(&pat, &seg)
}

fn match_slices(pat: &[&str], seg: &[&str]) -> bool {
    match (pat.first(), seg.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            // `**` matches 0..=seg.len() leading segments.
            if match_slices(&pat[1..], seg) {
                return true;
            }
            if seg.is_empty() {
                return false;
            }
            match_slices(pat, &seg[1..])
        }
        (None, Some(_)) | (Some(_), None) => false,
        (Some(p), Some(s)) => segment_matches(p, s) && match_slices(&pat[1..], &seg[1..]),
    }
}

/// Matches a single segment against a pattern that may contain `*` wildcards.
///
/// `*` matches any run of characters (including empty) *within* the segment —
/// it never crosses a `.` boundary. `core*` matches `core0`, `core42`.
/// `*_hits` matches `l1_hits`, `spec_hits`. A bare `*` matches any single
/// segment. Cross-segment wildcards are the `**` case handled above.
fn segment_matches(pat: &str, seg: &str) -> bool {
    let bytes_pat = pat.as_bytes();
    let bytes_seg = seg.as_bytes();
    glob_match(bytes_pat, bytes_seg)
}

fn glob_match(pat: &[u8], seg: &[u8]) -> bool {
    match (pat.first(), seg.first()) {
        (None, None) => true,
        (Some(&b'*'), _) => {
            // `*` matches 0..=seg.len() bytes.
            if glob_match(&pat[1..], seg) {
                return true;
            }
            if seg.is_empty() {
                return false;
            }
            glob_match(pat, &seg[1..])
        }
        (None, Some(_)) | (Some(_), None) => false,
        (Some(p), Some(s)) => p == s && glob_match(&pat[1..], &seg[1..]),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn literal_matches() {
        assert!(matches("core0.commit.insts", "core0.commit.insts"));
        assert!(!matches("core0.commit.insts", "core0.commit.other"));
        assert!(!matches("core0.commit.insts", "core0.commit.insts.extra"));
    }

    #[test]
    fn single_star_matches_one_segment() {
        assert!(matches("core*.commit.insts", "core0.commit.insts"));
        assert!(matches("core*.commit.insts", "core42.commit.insts"));
        assert!(!matches("core*.commit.insts", "core0.commit.other"));
        assert!(!matches("*.insts", "core0.commit.insts"));
    }

    #[test]
    fn double_star_matches_any_depth() {
        assert!(matches("**.misses", "core0.cache.l1d.misses"));
        assert!(matches("**.misses", "core0.misses"));
        assert!(matches("core0.**.hits", "core0.cache.l1d.hits"));
        assert!(matches("core0.**.hits", "core0.hits"));
        assert!(!matches("**.hits", "core0.cache.l1d.misses"));
    }

    #[test]
    fn double_star_matches_zero_segments() {
        assert!(matches("core0.**", "core0"));
        assert!(matches("core0.**.hits", "core0.hits"));
    }

    #[test]
    fn query_result_helpers() {
        let q = QueryResult {
            matches: vec![("core0.commit.insts".into(), 10.0), ("core1.commit.insts".into(), 20.0)],
        };
        assert_eq!(q.sum(), 30.0);
        assert_eq!(q.len(), 2);
        let by = q.by_subject();
        assert_eq!(by["core0"], 10.0);
        assert_eq!(by["core1"], 20.0);
    }
}
