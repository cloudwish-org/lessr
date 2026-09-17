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

    /// Read a mode from what a config file, an environment variable or
    /// `lessr on|off` wrote. `None` for anything else: a mode nobody can spell
    /// falls back to the layer below rather than guessing (`docs/CONFIG.md`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Mode::Off),
            "shadow" => Some(Mode::Shadow),
            // `on` is what `lessr on <stage>` means, and what a user writes.
            "active" | "on" => Some(Mode::Active),
            _ => None,
        }
    }

    /// The byte that stands for this mode in the snapshot. Stable across
    /// releases: it is on disk.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Mode::Off => 0,
            Mode::Shadow => 1,
            Mode::Active => 2,
        }
    }

    /// The mode a snapshot byte stands for, or `None` for a byte written by a
    /// version we do not know.
    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Mode::Off),
            1 => Some(Mode::Shadow),
            2 => Some(Mode::Active),
            _ => None,
        }
    }
}

/// How much a stage cuts.
///
/// The universal intensity axis: every mechanism, free and Pro, has one, and
/// the three names mean the same thing everywhere (`docs/CONFIG.md`).
///
/// - [`Level::Safe`] — only changes that cannot lose anything a human would
///   want back.
/// - [`Level::Balanced`] — the default once a mechanism is proven.
/// - [`Level::Aggressive`] — maximum savings, still bound by loop safety.
///
/// # A level never turns a safety check off
///
/// This is the whole contract of the type, and it is a property every
/// implementor owes, not a suggestion. A level changes **how much** a stage
/// cuts; it never changes **whether** the checks run. At `Aggressive`, exactly
/// as at `Safe`:
///
/// - error lines pass — `guard::errors_preserved` runs after every filter
///   (invariant 2);
/// - every cut leaves a handle (invariant 3);
/// - a result with [`ToolResult::explicit_selection`] set is returned untouched
///   (loop-safety 1) — ask [`crate::StageConfig::may_rewrite`], which gives the
///   same answer at every level;
/// - no mechanism forces a second tool call to get what one call used to
///   return (loop-safety 4).
///
/// There is no variant, setting or config file that relaxes any of those. A
/// mechanism that needed one would be a mechanism that does not ship.
///
/// The variants are ordered by intensity, so a stage can ask
/// `level >= Level::Balanced` for a cut it only makes above `Safe`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
pub enum Level {
    /// Only changes that cannot lose anything a human would want back.
    Safe,
    /// The default once a mechanism is proven.
    #[default]
    Balanced,
    /// Maximum savings, still bound by loop safety.
    Aggressive,
}

impl Level {
    /// Every level, weakest first. Iterated by the tests that check a property
    /// holds at all of them.
    pub const ALL: [Level; 3] = [Level::Safe, Level::Balanced, Level::Aggressive];

    /// The name used in config, on the receipt and in the database.
    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Safe => "safe",
            Level::Balanced => "balanced",
            Level::Aggressive => "aggressive",
        }
    }

    /// Read a level from what a config file or an environment variable wrote.
    /// `None` for anything else, which falls back to the layer below.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "safe" => Some(Level::Safe),
            "balanced" => Some(Level::Balanced),
            "aggressive" => Some(Level::Aggressive),
            _ => None,
        }
    }

    /// The byte that stands for this level in the snapshot. Stable across
    /// releases: it is on disk.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Level::Safe => 0,
            Level::Balanced => 1,
            Level::Aggressive => 2,
        }
    }

    /// The level a snapshot byte stands for, or `None` for a byte written by a
    /// version we do not know.
    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Level::Safe),
            1 => Some(Level::Balanced),
            2 => Some(Level::Aggressive),
            _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proven_mechanism_runs_balanced() {
        assert_eq!(Level::default(), Level::Balanced);
        assert_eq!(Mode::default(), Mode::Shadow);
    }

    #[test]
    fn levels_are_ordered_by_intensity() {
        assert!(Level::Safe < Level::Balanced);
        assert!(Level::Balanced < Level::Aggressive);
        assert_eq!(
            Level::ALL,
            [Level::Safe, Level::Balanced, Level::Aggressive]
        );
    }

    #[test]
    fn modes_and_levels_read_back_what_they_print() {
        for mode in [Mode::Off, Mode::Shadow, Mode::Active] {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
        for level in Level::ALL {
            assert_eq!(Level::parse(level.as_str()), Some(level));
        }
        assert_eq!(
            Mode::parse("  OFF "),
            Some(Mode::Off),
            "as a shell writes it"
        );
        assert_eq!(Mode::parse("on"), Some(Mode::Active), "as `lessr on` means");
        assert_eq!(Level::parse("Aggressive"), Some(Level::Aggressive));
    }

    #[test]
    fn a_value_nobody_can_spell_is_not_guessed_at() {
        assert_eq!(Mode::parse("aktive"), None);
        assert_eq!(Mode::parse(""), None);
        assert_eq!(Level::parse("maximum"), None);
    }

    #[test]
    fn the_snapshot_tags_are_stable() {
        // These bytes are on disk. Changing one changes what an old snapshot
        // means, which is what the format version exists to prevent.
        assert_eq!(
            (Mode::Off.tag(), Mode::Shadow.tag(), Mode::Active.tag()),
            (0, 1, 2)
        );
        assert_eq!(
            (
                Level::Safe.tag(),
                Level::Balanced.tag(),
                Level::Aggressive.tag()
            ),
            (0, 1, 2)
        );
        for mode in [Mode::Off, Mode::Shadow, Mode::Active] {
            assert_eq!(Mode::from_tag(mode.tag()), Some(mode));
        }
        for level in Level::ALL {
            assert_eq!(Level::from_tag(level.tag()), Some(level));
        }
        assert_eq!(
            Mode::from_tag(9),
            None,
            "a tag from a version we do not have"
        );
        assert_eq!(Level::from_tag(9), None);
    }
}
