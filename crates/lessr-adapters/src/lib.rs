//! Per-agent config knowledge: where an agent keeps its settings, how a Lessr
//! hook gets into them, and how it comes back out.
//!
//! Stage: none. This crate registers no [`lessr_core::Stage`]; it is what
//! `lessr init`, `lessr uninstall` and `lessr hook <agent>` are built from.
//! Path: both — it writes the hook entry that feeds the hook path, and prints
//! the `base_url` that feeds the proxy path.
//! Counting rule: none. Nothing here reaches the receipt.
//!
//! Three rules shape everything in it:
//!
//! * **Never break the agent.** Nothing here panics on a config or a payload it
//!   did not write. Every failure is an [`Error`], so `lessr hook` can fall
//!   back to the original JSON and exit 0 (`docs/ADAPTERS.md`), and `lessr init`
//!   can refuse a file rather than guess at it.
//! * **Never guess a format.** An adapter either knows an agent's real on-disk
//!   shape and writes it, or it prints instructions and writes nothing. A wrong
//!   hook entry does not fail once; it fails on every tool call afterwards, and
//!   the user has no reason to suspect the thing they installed to save tokens.
//!   Claude Code, Codex, Gemini CLI, OpenCode and pi are written; Cursor and
//!   Windsurf are not, because their entry schemas could not be confirmed; and
//!   Cline, Roo and Kilo have no hook to write at all.
//! * **Nothing leaves the machine** (invariant 6). This crate reads and writes
//!   local files and opens no sockets.
//!
//! `lessr init` is [`plan_init`] then [`apply`]. A [`Plan`] is inert: it prints
//! itself with [`Plan::render`] and is only turned into writes once the user has
//! agreed. Every write to a file that already existed is preceded by a backup
//! that [`plan_uninstall`] can restore byte for byte.

mod agent;
mod agents;
mod backup;
mod detect;
mod diff;
mod error;
mod file;
mod hook;
mod paths;
mod plan;

pub use agent::AgentId;
pub use detect::{Detected, detect};
pub use error::{Error, Result};
pub use hook::{HookPayload, parse_hook_input, render_hook_output, render_hook_unchanged};
pub use paths::Paths;
pub use plan::{Applied, Change, Plan, apply, plan_init, plan_uninstall};
