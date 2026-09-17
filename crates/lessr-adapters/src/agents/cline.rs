//! Cline. Confidence: unverified.
//!
//! A rules-file agent in `docs/ADAPTERS.md`. Cline's rules live with the
//! project as often as with the user, and Lessr does not guess at a project's
//! files.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Cline,
    probes: &[".cline", ".clinerules", ".config/cline"],
    mechanism: "a rules file, and a hook if it has one",
};
