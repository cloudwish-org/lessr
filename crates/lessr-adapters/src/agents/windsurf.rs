//! Windsurf. Confidence: unverified.
//!
//! One of the four rules-file agents in `docs/ADAPTERS.md` (with Cline, Roo and
//! Kilo): a rules file, plus a hook where the agent has one. Neither the file's
//! location nor whether the hook exists is confirmed here.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Windsurf,
    probes: &[
        ".codeium/windsurf",
        ".windsurf",
        ".config/Windsurf",
        "Library/Application Support/Windsurf",
    ],
    mechanism: "a rules file, and a hook if it has one",
};
