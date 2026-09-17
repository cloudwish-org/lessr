//! The extension point of Lessr.
//!
//! Stage: none — this crate defines the trait the mechanisms implement.
//! Path: both (hook and proxy).
//! Counting rule: none of its own; it carries [`Saving`] values the stages
//! produce and stamps them with the stage name, tier, mode and level so a
//! stage cannot misreport its own accounting.
//!
//! It also owns the configuration model every mechanism is tuned through —
//! [`Mode`], [`Level`] and [`Settings`], layered by [`Config`], compiled into
//! the binary [`Snapshot`] the hook path maps, and delivered to a stage as a
//! [`StageConfig`] through [`Stage::configure`]. Reading `config.toml` is the
//! CLI's job; this crate never sees a TOML parser, which is what keeps one off
//! the path that runs per tool call.
//!
//! This crate depends on nothing else in the workspace and performs no I/O on
//! the hot path: the snapshot is mapped once at startup and read from there
//! without parsing or allocating. See `docs/ARCHITECTURE.md` for the pipeline
//! contract, `docs/CONFIG.md` for the configuration contract and
//! `docs/MECHANISMS.md` for what each stage must count.

mod budget;
mod config;
mod error;
mod estimator;
mod handle;
pub mod mmap;
mod pipeline;
mod settings;
mod snapshot;
mod stage;
mod types;

pub use budget::{Budget, Verdict};
pub use config::{
    Config, EnvOverrides, FloorEntry, Layer, RejectedEnv, Resolved, SafetyFloor, StageConfig,
    StageOverride,
};
pub use error::{Error, Result};
pub use estimator::Estimator;
pub use handle::{HandleId, HandleStore};
pub use pipeline::{BudgetEvent, Pipeline, PipelineBuilder};
pub use settings::{Rejected, Settings, Value, ValueKind};
pub use snapshot::{SettingView, Snapshot, SnapshotView, StageView};
pub use stage::{Position, Stage};
pub use types::{
    Command, Level, Mode, Provider, Request, Saving, Tier, Tokens, ToolKind, ToolResult, Usage,
};
