//! Cursor. Confidence: unverified.
//!
//! `docs/ADAPTERS.md` gives Cursor a hook running `lessr hook cursor`, and it
//! does have hooks — two vocabularies of them, `preToolUse` and
//! `beforeShellExecution`, carrying different payloads, with the
//! `postToolUse` schema the one we would actually need still unconfirmed.
//! Reading the wrong one into a config that Cursor consults on every tool call
//! is not a guess worth making.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Cursor,
    probes: &[
        ".cursor",
        ".config/Cursor",
        "Library/Application Support/Cursor",
    ],
    note: "Cursor has two hook vocabularies in circulation, `preToolUse` and\n\
           `beforeShellExecution`, with different payloads, and the `postToolUse` schema\n\
           Lessr would need could not be confirmed. Writing the wrong one would break\n\
           every tool call, so this adapter writes nothing until it is verified.",
};
