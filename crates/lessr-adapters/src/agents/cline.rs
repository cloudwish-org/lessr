//! Cline. Confidence: unverified, and not for want of looking — Cline has no
//! hook mechanism at all.
//!
//! What it has is rules-file injection: `.clinerules`, either a file or a
//! directory, whose contents are prepended to the model's instructions. That
//! can ask a model to be brief; it cannot filter a tool result, which is what
//! Lessr does. Its API base URL is real, but it lives in the VS Code settings
//! UI and extension global state rather than in a file worth patching.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Cline,
    probes: &[".clinerules", ".cline", ".config/cline"],
    note: "Cline has no hook mechanism: it injects rules files (`.clinerules`, a file or a\n\
           directory) into the prompt, which cannot filter a tool result. Its base URL is\n\
           set in the VS Code settings UI, not in a file Lessr can patch, so point it at\n\
           the proxy there.",
};
