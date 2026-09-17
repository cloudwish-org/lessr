//! Cursor. Confidence: unverified.
//!
//! `docs/ADAPTERS.md` gives Cursor a hook running `lessr hook cursor`, in
//! "Cursor hooks config". Which file that is, and the shape of an entry inside
//! it, is not confirmed here, so this adapter looks and explains.

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
    mechanism: "a pre-tool hook in its hooks config",
};
