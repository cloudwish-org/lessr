//! Kilo Code. Confidence: unverified, and structurally so: like Cline and Roo,
//! Kilo has no hook mechanism. Configuration has moved to `kilo.jsonc` with an
//! `instructions` key; `.kilocode/rules/` is the legacy spelling of the same
//! idea. Both inject instructions, and neither can filter a tool result.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Kilo,
    probes: &["kilo.jsonc", ".kilocode/rules", ".kilocode"],
    note: "Kilo Code has no hook mechanism: `kilo.jsonc` carries an `instructions` key\n\
           (`.kilocode/rules/` is the older spelling), and instructions cannot filter a\n\
           tool result. Its base URL is set in the VS Code settings UI, not in a file\n\
           Lessr can patch, so point it at the proxy there.",
};
