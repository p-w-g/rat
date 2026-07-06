/// How many directories `fep` may run at once.
///
/// A dedicated type rather than a bare `usize` so `--sync`'s intent -
/// "exactly one directory at a time, always" - is explicit at the call
/// site instead of looking like ordinary concurrency that happens to be 1.
/// The scheduler (`run_bounded`) still only ever sees a plain limit; this
/// type exists purely to make choosing that limit unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Run directories strictly one after another.
    Sync,
    /// Run up to this many directories at once.
    Concurrent(usize),
}

impl ExecutionMode {
    /// The concurrency limit to hand to the scheduler.
    pub fn concurrency_limit(self) -> usize {
        match self {
            ExecutionMode::Sync => 1,
            ExecutionMode::Concurrent(n) => n.max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_forces_a_limit_of_one() {
        assert_eq!(ExecutionMode::Sync.concurrency_limit(), 1);
    }

    #[test]
    fn concurrent_uses_the_given_limit() {
        assert_eq!(ExecutionMode::Concurrent(4).concurrency_limit(), 4);
    }

    #[test]
    fn concurrent_limit_is_never_less_than_one() {
        assert_eq!(ExecutionMode::Concurrent(0).concurrency_limit(), 1);
    }
}
