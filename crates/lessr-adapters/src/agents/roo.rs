//! Roo Code. Confidence: unverified, and structurally so: like Cline, Roo has
//! no hook mechanism. Its `.roo/rules/` directory injects instructions into the
//! prompt, which cannot filter a tool result.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Roo,
    probes: &[".roo/rules", ".roo", ".roorules"],
    note: "Roo Code has no hook mechanism: `.roo/rules/` injects instructions into the\n\
           prompt, which cannot filter a tool result. Its base URL is set in the VS Code\n\
           settings UI, not in a file Lessr can patch, so point it at the proxy there.",
};
