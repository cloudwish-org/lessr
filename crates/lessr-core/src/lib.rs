//! The extension point of Lessr.
//!
//! Stage: none — this crate defines the trait the mechanisms implement.
//! Path: both (hook and proxy).
//! Counting rule: none of its own; it carries [`Saving`] values the stages
//! produce and stamps them with the stage name, tier and mode so a stage
//! cannot misreport its own accounting.
//!
//! This crate depends on nothing else in the workspace and performs no I/O on
//! the hot path. See `docs/ARCHITECTURE.md` for the pipeline contract and
//! `docs/MECHANISMS.md` for what each stage must count.

mod budget;
mod error;
mod estimator;
mod handle;
mod pipeline;
mod stage;
mod types;

pub use budget::{Budget, Verdict};
pub use error::{Error, Result};
pub use estimator::Estimator;
pub use handle::{HandleId, HandleStore};
pub use pipeline::{BudgetEvent, Pipeline, PipelineBuilder};
pub use stage::{Position, Stage};
pub use types::{
    Command, Mode, Provider, Request, Saving, Tier, Tokens, ToolKind, ToolResult, Usage,
};
