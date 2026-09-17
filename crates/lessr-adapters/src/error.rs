//! Errors raised while detecting, planning, patching or parsing.
//!
//! Libraries use `thiserror`; only the binary uses `anyhow` (see
//! `docs/CODE_STYLE.md`).
//!
//! Every variant here exists so that a caller can fall back rather than crash.
//! `lessr hook <agent>` turns any of these into "write the original JSON back
//! and exit 0" (`docs/ADAPTERS.md`); `lessr init` turns them into a printed
//! refusal to touch a file it does not understand. Nothing in this crate
//! panics on input it did not write.

use std::path::PathBuf;

/// The result type used across this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong in `lessr-adapters`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Neither the platform's home variable nor `LESSR_HOME` told us where the
    /// user lives, so we cannot name a single config path.
    #[error("cannot locate a home directory; set LESSR_HOME to the config directory")]
    NoHome,

    /// A file could not be read, written or copied.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file we were working on.
        path: PathBuf,
        /// What the filesystem said.
        #[source]
        source: std::io::Error,
    },

    /// An on-disk config or the backup index is not valid JSON. We report it
    /// instead of replacing it: an agent config we cannot parse is one we
    /// cannot safely rewrite.
    #[error("{}: not valid JSON: {source}", path.display())]
    Json {
        /// The file that failed to parse.
        path: PathBuf,
        /// The parse failure, with its line and column.
        #[source]
        source: serde_json::Error,
    },

    /// The file parsed but is not shaped like the config we know how to patch,
    /// e.g. `hooks` holding a string. Same reasoning as [`Error::Json`]: we
    /// stop rather than guess.
    #[error("{}: {detail}", path.display())]
    Shape {
        /// The file that surprised us.
        path: PathBuf,
        /// What we expected to find and did not.
        detail: String,
    },

    /// The agent sent us something that is not JSON on stdin.
    #[error("hook input is not valid JSON: {0}")]
    HookJson(#[source] serde_json::Error),

    /// The hook input parsed but is not a JSON object, so none of the fields
    /// the hook contract names can be there.
    #[error("hook input is not a JSON object")]
    HookShape,

    /// An adapter whose config format is not verified produced a change that
    /// would write. A bug in this crate, caught before it reaches a user's
    /// config: an unverified adapter prints instructions, it does not edit
    /// files. Named after the agent so the bug report writes itself.
    #[error("the {0} adapter is not verified and must not write to a config")]
    UnverifiedWrite(&'static str),

    /// Someone asked for the hook of an agent that is configured another way.
    /// The generic adapter is a `base_url`, not a hook.
    #[error("{0} has no hook; it is configured with a base_url")]
    NoHook(&'static str),

    /// A payload parsed as one agent was rendered as another. A caller bug,
    /// caught rather than papered over: substituting into the wrong shape
    /// would hand the agent its own text back with the saving silently lost.
    #[error("hook payload was parsed as {parsed} and rendered as {rendered}")]
    AgentMismatch {
        /// The agent [`crate::parse_hook_input`] was given.
        parsed: &'static str,
        /// The agent [`crate::render_hook_output`] was given.
        rendered: &'static str,
    },

    /// A stage cut the content mid-character. The caller falls back to the
    /// original bytes rather than handing the agent broken text.
    #[error("the rewritten tool output is not valid UTF-8")]
    NonUtf8,
}
