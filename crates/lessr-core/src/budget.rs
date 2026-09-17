//! The per-stage time budget.
//!
//! 2 ms soft, 10 ms hard, measured on the 1 MB `cargo test` fixture
//! (`docs/PERFORMANCE.md`). The budget is a contract: a stage that blows the
//! hard limit is skipped for the rest of the session and reported.

use std::time::Duration;

/// Soft and hard limits for one stage on one call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Over this, the stage is logged and left running.
    pub soft: Duration,
    /// Over this, the stage is disabled for the rest of the session.
    pub hard: Duration,
}

impl Budget {
    /// The soft limit from `docs/PERFORMANCE.md`.
    pub const DEFAULT_SOFT: Duration = Duration::from_millis(2);
    /// The hard limit from `docs/PERFORMANCE.md`, also the CI gate.
    pub const DEFAULT_HARD: Duration = Duration::from_millis(10);

    /// A budget with explicit limits.
    pub const fn new(soft: Duration, hard: Duration) -> Self {
        Self { soft, hard }
    }

    /// Classify one measured run.
    pub fn check(&self, elapsed: Duration) -> Verdict {
        if elapsed > self.hard {
            Verdict::Hard
        } else if elapsed > self.soft {
            Verdict::Soft
        } else {
            Verdict::Ok
        }
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SOFT, Self::DEFAULT_HARD)
    }
}

/// What one timed stage run was worth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Within budget.
    Ok,
    /// Over the soft limit: logged, still running.
    Soft,
    /// Over the hard limit: disabled for the session.
    Hard,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_against_the_documented_limits() {
        let b = Budget::default();
        assert_eq!(b.check(Duration::from_micros(500)), Verdict::Ok);
        assert_eq!(b.check(Duration::from_millis(5)), Verdict::Soft);
        assert_eq!(b.check(Duration::from_millis(11)), Verdict::Hard);
    }

    #[test]
    fn the_limits_are_the_ones_the_docs_promise() {
        let b = Budget::default();
        assert_eq!(b.soft, Duration::from_millis(2));
        assert_eq!(b.hard, Duration::from_millis(10));
    }
}
