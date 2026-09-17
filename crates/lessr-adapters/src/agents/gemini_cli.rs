//! Gemini CLI. Confidence: unverified.
//!
//! `docs/ADAPTERS.md` gives it a hook running `lessr hook gemini`, in "Gemini
//! hooks". The settings file is easy to find; the hook entry's shape is what is
//! not confirmed, and that is the half that breaks an agent when guessed.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::GeminiCli,
    probes: &[".gemini", ".config/gemini"],
    mechanism: "a pre-tool hook in its settings",
};
