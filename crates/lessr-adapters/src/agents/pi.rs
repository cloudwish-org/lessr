//! The pi harness. Confidence: unverified.
//!
//! `docs/ADAPTERS.md` puts pi, omp and Rakazo on one row: a pi harness adapter,
//! configured in pi's config, which Rakazo bots inherit.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Pi,
    probes: &[".pi", ".config/pi"],
    mechanism: "the pi harness adapter",
};
