//! The shared vocabulary every stage speaks.

use std::path::PathBuf;

use bytes::Bytes;

use crate::handle::HandleId;

/// Which engine a stage belongs to.
///
/// Tiering is by crate, never by feature flag (`docs/CODE_STYLE.md`). The tier
/// exists so the receipt can separate what Lessr saved from what Lessr Pro
/// would have saved.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tier {
    /// Ships in the open-source engine.
    Free,
    /// Ships in Lessr Pro.
    Pro,
}

impl Tier {
    /// The name used on the receipt and in the database.
    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::Free => "free",
            Tier::Pro => "pro",
        }
    }
}

/// How a registered stage runs.
///
/// Every new stage ships in [`Mode::Shadow`] (loop-safety rule 6), which is why
/// that is the default: registering a stage without saying otherwise cannot
/// change what the agent sees.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Mode {
    /// Not run at all. The kill switch.
    Off,
    /// Run, count the saving, throw the rewritten output away. This is what
    /// fills the "left on the table" column of the receipt.
    #[default]
    Shadow,
    /// Run and apply the rewritten output.
    Active,
}

impl Mode {
    /// The name used on the receipt and in the database.
    pub const fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Shadow => "shadow",
            Mode::Active => "active",
        }
    }
}

/// The kind of tool call a result came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ToolKind {
    /// A shell command: `Bash`, `run_terminal_cmd`, and friends.
    Shell,
    /// A file read.
    Read,
    /// A search: grep, glob, ripgrep.
    Search,
    /// A write or patch.
    Edit,
    /// Anything else. Stages should leave these alone unless they know better.
    Other,
}

/// A shell command as the agent invoked it, split for rule matching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The program, with any path stripped: `git`, `cargo`, `npm`.
    pub program: String,
    /// Arguments in order, as given.
    pub args: Vec<String>,
}

/// A tool result on its way into the agent's context.
///
/// `content` is the only field a stage may rewrite.
#[derive(Clone, Debug)]
pub struct ToolResult {
    /// What kind of tool produced this.
    pub tool: ToolKind,
    /// The agent's own name for the tool, kept for the receipt.
    pub tool_name: String,
    /// The parsed command, for [`ToolKind::Shell`].
    pub command: Option<Command>,
    /// The absolute path read or written, where there is one.
    pub path: Option<PathBuf>,
    /// The agent asked for this content by explicit range or search term.
    ///
    /// Loop-safety rule 1: requested content is sacred. The gate, the trap list
    /// and (in Pro) zoom must return such a result untouched. Dedup still
    /// applies, because it answers the same question more cheaply rather than
    /// answering a narrower one.
    pub explicit_selection: bool,
    /// The bytes themselves. `Bytes` so slicing across stages is zero-copy.
    pub content: Bytes,
}

impl ToolResult {
    /// A result carrying nothing but content, for tests and for agents that
    /// give us no structure to work with.
    pub fn new(tool: ToolKind, tool_name: impl Into<String>, content: Bytes) -> Self {
        Self {
            tool,
            tool_name: tool_name.into(),
            command: None,
            path: None,
            explicit_selection: false,
            content,
        }
    }

    /// The size of the current content, which is what every gate-side saving is
    /// counted against.
    pub fn len(&self) -> usize {
        self.content.len()
    }

    /// Whether the content is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }
}

/// Which API shape a request is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Provider {
    /// `/v1/messages`.
    Anthropic,
    /// `/v1/chat/completions`.
    OpenAi,
    /// Something we forward but do not parse.
    Other,
}

/// A provider request passing through the proxy.
///
/// Nothing in the free engine mutates one of these (invariant 4); `detect`
/// reads the prefix and reports.
#[derive(Clone, Debug)]
pub struct Request {
    /// The API shape.
    pub provider: Provider,
    /// The model named in the body, where we could read one.
    pub model: String,
    /// The raw request body, untouched.
    pub body: Bytes,
}

/// The provider's own usage fields. These are the truth; everything else on the
/// receipt is either a byte count or a marked estimate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// The model that served the turn.
    pub model: String,
    /// Input tokens billed at the normal rate.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Input tokens served from the provider's prompt cache.
    pub cache_read: u64,
    /// Input tokens written into the provider's prompt cache. A turn with a
    /// non-zero value here after a warm turn is a cache break.
    pub cache_write: u64,
}

/// A token count and whether it is arithmetic or an estimate.
///
/// The receipt never prints an estimate as exact (`docs/MECHANISMS.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tokens {
    /// Derived from the provider's usage fields or from a byte count.
    Exact(u64),
    /// Derived from bytes through the [`crate::Estimator`].
    Estimate(u64),
}

impl Tokens {
    /// The count, whichever kind it is. Callers that print it must also print
    /// [`Tokens::is_estimate`].
    pub const fn count(self) -> u64 {
        match self {
            Tokens::Exact(n) | Tokens::Estimate(n) => n,
        }
    }

    /// Whether this number must be marked as an estimate on the receipt.
    pub const fn is_estimate(self) -> bool {
        matches!(self, Tokens::Estimate(_))
    }
}

/// What one stage saved on one call.
///
/// Stages build these; the pipeline stamps the stage name, tier and mode before
/// handing them back, so the receipt cannot disagree with the pipeline about
/// who did what.
#[derive(Clone, Debug)]
pub struct Saving {
    /// The stage that produced it. Stamped by the pipeline.
    pub stage: &'static str,
    /// Free or Pro. Stamped by the pipeline.
    pub tier: Tier,
    /// The mode it ran in. Stamped by the pipeline; `Shadow` means this saving
    /// was counted but not applied.
    pub mode: Mode,
    /// Content size before the stage.
    pub bytes_before: u64,
    /// Content size after the stage.
    pub bytes_after: u64,
    /// The token saving, exact or estimated.
    pub tokens: Tokens,
    /// The handle holding whatever was removed. Every cut leaves one
    /// (invariant 3).
    pub handle: Option<HandleId>,
}

impl Saving {
    /// A saving of `before - after` bytes, with the token count supplied by the
    /// caller. The stage, tier and mode are placeholders until the pipeline
    /// stamps them.
    pub fn bytes(before: u64, after: u64, tokens: Tokens) -> Self {
        Self {
            stage: "",
            tier: Tier::Free,
            mode: Mode::Shadow,
            bytes_before: before,
            bytes_after: after,
            tokens,
            handle: None,
        }
    }

    /// Attach the handle that holds the removed bytes.
    pub fn with_handle(mut self, handle: HandleId) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Bytes removed, saturating: a stage that grew the output saved nothing.
    pub const fn bytes_saved(&self) -> u64 {
        self.bytes_before.saturating_sub(self.bytes_after)
    }
}
