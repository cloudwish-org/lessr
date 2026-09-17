//! Rakazo. Confidence: not applicable — Rakazo is not a coding-agent harness.
//!
//! `docs/ADAPTERS.md` files it beside pi, on the strength of Rakazo bots
//! inheriting pi's config. It is really a server deployment, configured with a
//! `.env`, and the way to put Lessr in front of it is to point its model URL at
//! the proxy.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Rakazo,
    probes: &[".rakazo", ".config/rakazo"],
    note: "Rakazo is a server deployment rather than a coding agent: it has no hook to\n\
           install. Point it at the proxy in the deployment's .env:\n\n    \
           RAKAZO_LOCAL_MODELS_URL=http://127.0.0.1:7433/v1",
};
