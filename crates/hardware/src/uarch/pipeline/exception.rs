//! Where in the pipeline an exception was first detected.

/// Pipeline stage where an exception was first detected.
///
/// Used to track exception origin through the pipeline for accurate
/// trap handling and diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExceptionStage {
    /// Exception detected during instruction fetch.
    #[default]
    Fetch,
    /// Exception detected during instruction decode.
    Decode,
    /// Exception detected during execution.
    Execute,
    /// Exception detected during memory access.
    Memory,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exception_stage_default() {
        assert_eq!(ExceptionStage::default(), ExceptionStage::Fetch);
    }
}
