//! Codex, OpenAI's CLI. Confidence: unverified.
//!
//! `docs/ADAPTERS.md` gives it a hook running `lessr hook codex`, in "Codex
//! config". Codex keeps a config directory of its own; the hook key inside it
//! is not confirmed here.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Codex,
    probes: &[".codex", ".config/codex"],
    mechanism: "a pre-tool hook in its config",
};
