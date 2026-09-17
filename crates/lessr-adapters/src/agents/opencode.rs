//! OpenCode. Confidence: unverified.
//!
//! The odd one out: OpenCode loads a TypeScript plugin rather than taking a
//! hook entry in a JSON config, so installing it means writing a whole file
//! (`lessr.ts`) that Lessr owns — a [`crate::Change::WriteFile`], not a
//! [`crate::Change::WriteJson`]. That file has to call a plugin API whose shape
//! is not confirmed here, and a plugin that fails to load is worse than a
//! missing hook: it can stop the agent from starting at all.
//!
//! `docs/ADAPTERS.md` writes the directory as `~/.config/opencode/plugins/`;
//! the working note that reached this crate says `plugin/`. Both are probed, so
//! detection is right either way, and the difference is settled before anything
//! is written.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::OpenCode,
    probes: &[
        ".config/opencode/plugin",
        ".config/opencode/plugins",
        ".config/opencode",
        ".opencode",
    ],
    mechanism: "a plugin file under ~/.config/opencode",
};
