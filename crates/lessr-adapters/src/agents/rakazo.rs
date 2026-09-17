//! Rakazo. Confidence: unverified.
//!
//! Rakazo bots inherit pi's configuration (`docs/ADAPTERS.md`), so configuring
//! pi configures them; the entry exists so that `lessr init` can say so.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Rakazo,
    probes: &[".rakazo", ".config/rakazo"],
    mechanism: "the pi config its bots inherit",
};
