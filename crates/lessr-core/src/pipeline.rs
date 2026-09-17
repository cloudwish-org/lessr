//! The pipeline: an ordered list of stages, each with a mode and a budget.

use std::time::Instant;

use crate::budget::{Budget, Verdict};
use crate::error::{Error, Result};
use crate::stage::{Position, Stage};
use crate::types::{Mode, Request, Saving, Tier, ToolResult, Usage};

/// One stage as the pipeline holds it.
struct Registered {
    stage: Box<dyn Stage>,
    /// Cached so we can read it without touching the trait object.
    name: &'static str,
    tier: Tier,
    mode: Mode,
    budget: Budget,
    /// Set when the stage blew the hard budget. It is skipped for the rest of
    /// the session; the budget is a contract.
    disabled: bool,
}

/// A stage run that went over budget.
#[derive(Clone, Debug)]
pub struct BudgetEvent {
    /// Which stage.
    pub stage: &'static str,
    /// How long it took.
    pub elapsed: std::time::Duration,
    /// Soft or hard.
    pub verdict: Verdict,
    /// Whether this run is the one that disabled the stage.
    pub disabled_stage: bool,
}

/// Builds a [`Pipeline`].
///
/// This is the seam Lessr Pro registers through: the Pro binary takes the
/// builder the free binary would have used, adds its own stages by
/// [`Position`], and calls the same entry point. Nothing in the free workspace
/// needs to know that happened.
#[derive(Default)]
pub struct PipelineBuilder {
    stages: Vec<Registered>,
}

impl PipelineBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a stage.
    pub fn register<S: Stage + 'static>(&mut self, stage: S, mode: Mode) -> Result<&mut Self> {
        self.register_at(Position::Last, stage, mode)
    }

    /// Insert a stage at a position relative to the stages already registered.
    pub fn register_at<S: Stage + 'static>(
        &mut self,
        position: Position,
        stage: S,
        mode: Mode,
    ) -> Result<&mut Self> {
        let name = stage.name();
        if self.contains(name) {
            return Err(Error::DuplicateStage(name.to_string()));
        }

        let index = match position {
            Position::First => 0,
            Position::Last => self.stages.len(),
            Position::Before(other) => self.index_of(other)?,
            Position::After(other) => self.index_of(other)? + 1,
        };

        let registered = Registered {
            name,
            tier: stage.tier(),
            stage: Box::new(stage),
            mode,
            budget: Budget::default(),
            disabled: false,
        };
        self.stages.insert(index, registered);
        Ok(self)
    }

    /// Give one stage a budget other than the default.
    pub fn set_budget(&mut self, name: &str, budget: Budget) -> Result<&mut Self> {
        let index = self.index_of(name)?;
        self.stages[index].budget = budget;
        Ok(self)
    }

    /// Whether a stage of this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.stages.iter().any(|s| s.name == name)
    }

    /// The stage order as it stands.
    pub fn names(&self) -> Vec<&'static str> {
        self.stages.iter().map(|s| s.name).collect()
    }

    fn index_of(&self, name: &str) -> Result<usize> {
        self.stages
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| Error::UnknownStage(name.to_string()))
    }

    /// Freeze the order and produce the pipeline.
    pub fn build(self) -> Pipeline {
        Pipeline {
            stages: self.stages,
            events: Vec::new(),
        }
    }
}

impl std::fmt::Debug for PipelineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineBuilder")
            .field("stages", &StageList(&self.stages))
            .finish()
    }
}

/// An ordered list of stages, timed against their budgets.
pub struct Pipeline {
    stages: Vec<Registered>,
    events: Vec<BudgetEvent>,
}

impl Pipeline {
    /// Run the hook path over one tool result.
    ///
    /// Stages in [`Mode::Shadow`] run against a copy: their saving is counted
    /// for the receipt, their rewrite is thrown away.
    pub fn run_tool_result(&mut self, result: &mut ToolResult) -> Vec<Saving> {
        let mut savings = Vec::new();
        for index in 0..self.stages.len() {
            let mode = self.stages[index].mode;
            if mode == Mode::Off || self.stages[index].disabled {
                continue;
            }

            let (saving, elapsed) = match mode {
                Mode::Active => {
                    let start = Instant::now();
                    let saving = self.stages[index].stage.on_tool_result(result);
                    (saving, start.elapsed())
                }
                Mode::Shadow => {
                    let mut shadow = result.clone();
                    let start = Instant::now();
                    let saving = self.stages[index].stage.on_tool_result(&mut shadow);
                    (saving, start.elapsed())
                }
                Mode::Off => unreachable!("filtered above"),
            };

            self.charge(index, elapsed);
            if let Some(saving) = saving {
                savings.push(self.stamp(index, saving));
            }
        }
        savings
    }

    /// Run the proxy path over one outgoing request.
    ///
    /// Nothing in the free engine mutates a request (invariant 4), so `Shadow`
    /// and `Active` differ only in how the saving is labelled here.
    pub fn run_request(&mut self, request: &mut Request) -> Vec<Saving> {
        let mut savings = Vec::new();
        for index in 0..self.stages.len() {
            let mode = self.stages[index].mode;
            if mode == Mode::Off || self.stages[index].disabled {
                continue;
            }

            let (saving, elapsed) = match mode {
                Mode::Active => {
                    let start = Instant::now();
                    let saving = self.stages[index].stage.on_request(request);
                    (saving, start.elapsed())
                }
                Mode::Shadow => {
                    let mut shadow = request.clone();
                    let start = Instant::now();
                    let saving = self.stages[index].stage.on_request(&mut shadow);
                    (saving, start.elapsed())
                }
                Mode::Off => unreachable!("filtered above"),
            };

            self.charge(index, elapsed);
            if let Some(saving) = saving {
                savings.push(self.stamp(index, saving));
            }
        }
        savings
    }

    /// Run the proxy path over the usage fields of a finished turn.
    ///
    /// Read-only, so shadow and active are the same thing.
    pub fn run_usage(&mut self, usage: &Usage) -> Vec<Saving> {
        let mut savings = Vec::new();
        for index in 0..self.stages.len() {
            if self.stages[index].mode == Mode::Off || self.stages[index].disabled {
                continue;
            }
            let start = Instant::now();
            let saving = self.stages[index].stage.on_usage(usage);
            self.charge(index, start.elapsed());
            if let Some(saving) = saving {
                savings.push(self.stamp(index, saving));
            }
        }
        savings
    }

    /// Time one run against the stage's budget, disabling it if it blew the
    /// hard limit.
    fn charge(&mut self, index: usize, elapsed: std::time::Duration) {
        let stage = &mut self.stages[index];
        match stage.budget.check(elapsed) {
            Verdict::Ok => {}
            Verdict::Soft => self.events.push(BudgetEvent {
                stage: stage.name,
                elapsed,
                verdict: Verdict::Soft,
                disabled_stage: false,
            }),
            Verdict::Hard => {
                stage.disabled = true;
                self.events.push(BudgetEvent {
                    stage: stage.name,
                    elapsed,
                    verdict: Verdict::Hard,
                    disabled_stage: true,
                });
            }
        }
    }

    /// Stamp the pipeline's own view of who ran and how onto a saving, so a
    /// stage cannot misreport itself on the receipt.
    fn stamp(&self, index: usize, mut saving: Saving) -> Saving {
        let stage = &self.stages[index];
        saving.stage = stage.name;
        saving.tier = stage.tier;
        saving.mode = stage.mode;
        saving
    }

    /// Budget breaches seen so far.
    pub fn events(&self) -> &[BudgetEvent] {
        &self.events
    }

    /// Take the budget breaches, leaving the pipeline empty of them. The
    /// recorder drains these off the hot path.
    pub fn take_events(&mut self) -> Vec<BudgetEvent> {
        std::mem::take(&mut self.events)
    }

    /// The stage order.
    pub fn names(&self) -> Vec<&'static str> {
        self.stages.iter().map(|s| s.name).collect()
    }

    /// The mode a stage is running in.
    pub fn mode(&self, name: &str) -> Option<Mode> {
        self.stages.iter().find(|s| s.name == name).map(|s| s.mode)
    }

    /// Change a stage's mode. This is the kill switch behind `lessr on|off`
    /// and the self-healing table.
    pub fn set_mode(&mut self, name: &str, mode: Mode) -> Result<()> {
        let stage = self
            .stages
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| Error::UnknownStage(name.to_string()))?;
        stage.mode = mode;
        Ok(())
    }

    /// Whether a stage has been disabled for the session by the hard budget.
    pub fn is_disabled(&self, name: &str) -> Option<bool> {
        self.stages
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.disabled)
    }

    /// How many stages are registered.
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// Whether no stages are registered. A pipeline like this is a pure
    /// passthrough, which is exactly what the hook must be before any
    /// mechanism ships.
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field("stages", &StageList(&self.stages))
            .field("events", &self.events.len())
            .finish()
    }
}

/// Renders the stage order as `name(mode)`, which is the only part of a
/// pipeline worth printing.
struct StageList<'a>(&'a [Registered]);

impl std::fmt::Debug for StageList<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|s| {
                format!(
                    "{}({}{})",
                    s.name,
                    s.mode.as_str(),
                    if s.disabled { ", disabled" } else { "" }
                )
            }))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Tokens, ToolKind};
    use bytes::Bytes;

    /// Truncates content to `keep` bytes and counts what it removed.
    struct Truncate {
        name: &'static str,
        keep: usize,
    }

    impl Stage for Truncate {
        fn name(&self) -> &'static str {
            self.name
        }
        fn tier(&self) -> Tier {
            Tier::Free
        }
        fn on_tool_result(&mut self, result: &mut ToolResult) -> Option<Saving> {
            let before = result.content.len() as u64;
            if result.content.len() <= self.keep {
                return None;
            }
            result.content = result.content.slice(..self.keep);
            let after = result.content.len() as u64;
            Some(Saving::bytes(before, after, Tokens::Exact(before - after)))
        }
    }

    struct Slow;
    impl Stage for Slow {
        fn name(&self) -> &'static str {
            "slow"
        }
        fn tier(&self) -> Tier {
            Tier::Free
        }
        fn on_tool_result(&mut self, _result: &mut ToolResult) -> Option<Saving> {
            std::thread::sleep(std::time::Duration::from_millis(12));
            None
        }
    }

    fn result(content: &'static str) -> ToolResult {
        ToolResult::new(
            ToolKind::Shell,
            "Bash",
            Bytes::from_static(content.as_bytes()),
        )
    }

    #[test]
    fn an_empty_pipeline_passes_content_through_untouched() {
        let mut pipeline = PipelineBuilder::new().build();
        let mut r = result("cargo test output");
        let savings = pipeline.run_tool_result(&mut r);
        assert!(savings.is_empty());
        assert_eq!(&r.content[..], b"cargo test output");
    }

    #[test]
    fn an_active_stage_rewrites_and_counts() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 5,
                },
                Mode::Active,
            )
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);

        assert_eq!(&r.content[..], b"01234");
        assert_eq!(savings.len(), 1);
        assert_eq!(savings[0].stage, "gate");
        assert_eq!(savings[0].bytes_saved(), 5);
        assert_eq!(savings[0].mode, Mode::Active);
    }

    #[test]
    fn a_shadow_stage_counts_without_changing_what_the_agent_sees() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 5,
                },
                Mode::Shadow,
            )
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);

        assert_eq!(&r.content[..], b"0123456789", "shadow must not mutate");
        assert_eq!(savings.len(), 1, "but it must still count");
        assert_eq!(savings[0].bytes_saved(), 5);
        assert_eq!(savings[0].mode, Mode::Shadow);
    }

    #[test]
    fn shadow_is_the_default_mode_for_a_new_stage() {
        assert_eq!(Mode::default(), Mode::Shadow);
    }

    #[test]
    fn an_off_stage_does_not_run() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 5,
                },
                Mode::Off,
            )
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("0123456789");
        assert!(pipeline.run_tool_result(&mut r).is_empty());
        assert_eq!(&r.content[..], b"0123456789");
    }

    #[test]
    fn stages_run_in_registration_order() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "trap",
                    keep: 8,
                },
                Mode::Active,
            )
            .unwrap();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 4,
                },
                Mode::Active,
            )
            .unwrap();
        let mut pipeline = builder.build();
        assert_eq!(pipeline.names(), ["trap", "gate"]);

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"0123");
        assert_eq!(savings.len(), 2);
        assert_eq!(savings[0].stage, "trap");
        assert_eq!(savings[1].stage, "gate");
    }

    #[test]
    fn a_pro_stage_can_insert_itself_around_a_free_one() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 9,
                },
                Mode::Active,
            )
            .unwrap();
        builder
            .register_at(
                Position::Before("gate"),
                Truncate {
                    name: "cachefix",
                    keep: 9,
                },
                Mode::Active,
            )
            .unwrap();
        builder
            .register_at(
                Position::After("gate"),
                Truncate {
                    name: "compact",
                    keep: 9,
                },
                Mode::Active,
            )
            .unwrap();

        assert_eq!(builder.names(), ["cachefix", "gate", "compact"]);
    }

    #[test]
    fn positioning_against_an_unregistered_stage_is_an_error() {
        let mut builder = PipelineBuilder::new();
        let err = builder
            .register_at(
                Position::Before("gate"),
                Truncate {
                    name: "cachefix",
                    keep: 1,
                },
                Mode::Active,
            )
            .unwrap_err();
        assert!(matches!(err, Error::UnknownStage(name) if name == "gate"));
    }

    #[test]
    fn two_stages_cannot_share_a_name() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 1,
                },
                Mode::Active,
            )
            .unwrap();
        let err = builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 2,
                },
                Mode::Active,
            )
            .unwrap_err();
        assert!(matches!(err, Error::DuplicateStage(name) if name == "gate"));
    }

    #[test]
    fn a_stage_over_the_hard_budget_is_dropped_for_the_session() {
        let mut builder = PipelineBuilder::new();
        builder.register(Slow, Mode::Active).unwrap();
        let mut pipeline = builder.build();

        let mut r = result("anything");
        pipeline.run_tool_result(&mut r);

        assert_eq!(pipeline.is_disabled("slow"), Some(true));
        let events = pipeline.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].verdict, Verdict::Hard);
        assert!(events[0].disabled_stage);

        // The second call must not pay for it again.
        let before = Instant::now();
        pipeline.run_tool_result(&mut r);
        assert!(before.elapsed() < std::time::Duration::from_millis(5));
        assert_eq!(pipeline.events().len(), 1, "no second breach to report");
    }

    #[test]
    fn the_kill_switch_turns_a_stage_off_in_place() {
        let mut builder = PipelineBuilder::new();
        builder
            .register(
                Truncate {
                    name: "gate",
                    keep: 5,
                },
                Mode::Active,
            )
            .unwrap();
        let mut pipeline = builder.build();

        pipeline.set_mode("gate", Mode::Off).unwrap();
        assert_eq!(pipeline.mode("gate"), Some(Mode::Off));

        let mut r = result("0123456789");
        assert!(pipeline.run_tool_result(&mut r).is_empty());
        assert_eq!(&r.content[..], b"0123456789");

        assert!(pipeline.set_mode("nope", Mode::Off).is_err());
    }

    #[test]
    fn the_pipeline_stamps_savings_so_a_stage_cannot_misreport() {
        /// Claims to be a different, free stage that saved nothing.
        struct Liar;
        impl Stage for Liar {
            fn name(&self) -> &'static str {
                "honest"
            }
            fn tier(&self) -> Tier {
                Tier::Pro
            }
            fn on_tool_result(&mut self, _r: &mut ToolResult) -> Option<Saving> {
                let mut saving = Saving::bytes(100, 10, Tokens::Exact(90));
                saving.stage = "someone-else";
                saving.tier = Tier::Free;
                saving.mode = Mode::Active;
                Some(saving)
            }
        }

        let mut builder = PipelineBuilder::new();
        builder.register(Liar, Mode::Shadow).unwrap();
        let mut pipeline = builder.build();

        let mut r = result("x");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(savings[0].stage, "honest");
        assert_eq!(savings[0].tier, Tier::Pro);
        assert_eq!(savings[0].mode, Mode::Shadow);
    }
}
