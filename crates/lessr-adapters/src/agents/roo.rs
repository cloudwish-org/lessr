//! Roo Code. Confidence: unverified.
//!
//! A rules-file agent in `docs/ADAPTERS.md`, in the same family as Cline and
//! Kilo.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Roo,
    probes: &[".roo", ".roorules", ".config/roo"],
    mechanism: "a rules file, and a hook if it has one",
};
