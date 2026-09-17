//! Windsurf. Confidence: unverified.
//!
//! Windsurf keeps hooks at `~/.codeium/windsurf/hooks.json`, but the schema of
//! an entry in it could not be confirmed, and neither could the event that can
//! replace a tool result.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Windsurf,
    probes: &[
        ".codeium/windsurf/hooks.json",
        ".codeium/windsurf",
        ".windsurf",
        "Library/Application Support/Windsurf",
    ],
    note: "Windsurf's hooks file is easy to find and its entry schema is not: neither the\n\
           shape of an entry nor which event can replace a tool result could be\n\
           confirmed. Lessr writes nothing it has not verified.",
};
