//! The `Stage` trait: one mechanism, one crate, one implementation of this.

use crate::config::StageConfig;
use crate::types::{Request, Saving, Tier, ToolResult, Usage};

/// A mechanism.
///
/// Each hook is optional; a stage implements only the paths it runs on. The
/// pipeline decides whether a stage runs at all (its [`crate::Mode`]) and
/// enforces the [`crate::Budget`], so implementations never check either. What
/// a stage does decide — how hard it cuts, and with which thresholds — reaches
/// it through [`Stage::configure`].
///
/// Returning `Some(Saving)` claims a saving. Do not claim one without a
/// counting rule in `docs/MECHANISMS.md`: no rule, no line on the receipt.
pub trait Stage: Send + Sync {
    /// The stage's name. Unique across the pipeline, stable across releases:
    /// it keys [`Position`], the receipt and the self-healing table.
    fn name(&self) -> &'static str;

    /// Free or Pro. Used to split the receipt.
    fn tier(&self) -> Tier;

    /// Take the configuration this stage runs with.
    ///
    /// Defaulted, because a stage with nothing to tune has nothing to take:
    /// the gate reads three thresholds here, `detect` reads none, and neither
    /// has to say so.
    ///
    /// Called once at registration, and **again on a pipeline that is already
    /// running**: the kill switch, `lessr level <stage> <level>` and the
    /// self-healing table all reconfigure live stages (`docs/CONFIG.md`). So
    /// an implementation must be idempotent — derive everything it keeps from
    /// `config` alone, and never add to what the previous call left behind.
    ///
    /// `config` is borrowed for the call only, so copy out what is needed:
    /// [`crate::Level`] is `Copy` and the [`crate::Settings`] accessors take
    /// the stage's own default at the call site. Copying here is also what
    /// keeps the hook path off the settings map, which it may not search once
    /// per tool call (`docs/PERFORMANCE.md`).
    ///
    /// A level says how much to cut, never whether a check runs. Reading
    /// `config.level` to skip the error guard, a handle write or
    /// [`StageConfig::may_rewrite`] is a bug, not a level.
    fn configure(&mut self, _config: &StageConfig) {}

    /// Hook path. Rewrite a tool result before it enters the agent's context.
    ///
    /// Honour loop-safety rule 1: if `result.explicit_selection` is set, the
    /// agent asked for exactly these bytes and a filtering stage must return
    /// them untouched.
    fn on_tool_result(&mut self, _result: &mut ToolResult) -> Option<Saving> {
        None
    }

    /// Proxy path. Inspect an outgoing provider request.
    ///
    /// Nothing in the free engine mutates a request (invariant 4). Pro stages
    /// may, and only when the cache is already cold or the prefix stays
    /// byte-stable.
    fn on_request(&mut self, _request: &mut Request) -> Option<Saving> {
        None
    }

    /// Proxy path. Observe the provider's usage fields once the turn is done.
    fn on_usage(&mut self, _usage: &Usage) -> Option<Saving> {
        None
    }
}

/// Where a stage goes in the pipeline order.
///
/// The free binary registers its stages in order; Pro crates insert themselves
/// relative to stages by name, which is why names are stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    /// Run before everything already registered.
    First,
    /// Run after everything already registered.
    Last,
    /// Run immediately before the named stage.
    Before(&'static str),
    /// Run immediately after the named stage.
    After(&'static str),
}
