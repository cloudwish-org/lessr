//! omp. Confidence: unverified.
//!
//! Runs on the pi harness (`docs/ADAPTERS.md`), so it is configured the way pi
//! is. It keeps its own entry because a user who installed omp should see omp
//! in `lessr init`, not be told to go and read about pi.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Omp,
    probes: &[".omp", ".config/omp"],
    mechanism: "the pi harness adapter it runs on",
};
