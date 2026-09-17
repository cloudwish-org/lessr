//! The pipeline: an ordered list of stages, each with a configuration and a
//! budget.

use std::time::Instant;

use crate::budget::{Budget, Verdict};
use crate::config::StageConfig;
use crate::error::{Error, Result};
use crate::stage::{Position, Stage};
use crate::types::{Mode, Request, Saving, Tier, ToolResult, Usage};

/// One stage as the pipeline holds it.
struct Registered {
    stage: Box<dyn Stage>,
    /// Cached so we can read it without touching the trait object.
    name: &'static str,
    tier: Tier,
    /// The whole configuration, not just the mode: the level is stamped onto
    /// every [`Saving`] the stage produces, and `lessr config --explain` reads
    /// the settings back out of the pipeline that is actually running.
    config: StageConfig,
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

    /// Append a stage with only its mode set: the compiled-in level, and no
    /// settings.
    pub fn register<S: Stage + 'static>(&mut self, stage: S, mode: Mode) -> Result<&mut Self> {
        self.register_at(Position::Last, stage, mode)
    }

    /// Insert a stage at a position relative to the stages already registered,
    /// with only its mode set.
    pub fn register_at<S: Stage + 'static>(
        &mut self,
        position: Position,
        stage: S,
        mode: Mode,
    ) -> Result<&mut Self> {
        self.register_with(
            position,
            stage,
            StageConfig {
                mode,
                ..StageConfig::new()
            },
        )
    }

    /// Insert a stage with the configuration it is to run with.
    ///
    /// The full form; the other two are shorthand for it, so there is one path
    /// into the pipeline and not three. The caller resolves a [`StageConfig`]
    /// per stage — in the binary, out of the mapped snapshot — and the stage is
    /// handed it here, once, before it can see a tool result.
    pub fn register_with<S: Stage + 'static>(
        &mut self,
        position: Position,
        mut stage: S,
        config: StageConfig,
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

        // After the position is known to be valid, so a stage rejected by this
        // call is never left configured for a pipeline it did not join.
        stage.configure(&config);

        let registered = Registered {
            name,
            tier: stage.tier(),
            stage: Box::new(stage),
            config,
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
            let mode = self.stages[index].config.mode;
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
            let mode = self.stages[index].config.mode;
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
            if self.stages[index].config.mode == Mode::Off || self.stages[index].disabled {
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
        saving.mode = stage.config.mode;
        saving.level = stage.config.level;
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
        self.stages
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.config.mode)
    }

    /// Everything a stage is running with: mode, level and settings.
    ///
    /// What `lessr config --explain <stage>` prints, and the reason it can be
    /// trusted: these are the values the live pipeline holds, not the ones a
    /// file says it should.
    pub fn config(&self, name: &str) -> Option<&StageConfig> {
        self.stages
            .iter()
            .find(|s| s.name == name)
            .map(|s| &s.config)
    }

    /// Change a stage's mode. This is the kill switch behind `lessr on|off`
    /// and the self-healing table.
    ///
    /// The mode and nothing else: a stage switched off and on again comes back
    /// at the level and settings it had, rather than quietly at the defaults.
    pub fn set_mode(&mut self, name: &str, mode: Mode) -> Result<()> {
        let index = self.index_of(name)?;
        self.stages[index].config.mode = mode;
        self.reconfigure(index);
        Ok(())
    }

    /// Replace a stage's whole configuration and hand it to the stage.
    ///
    /// `lessr level <stage> <level>` and a reload of the snapshot arrive here.
    /// The pipeline is live and the stage has probably already run, which is
    /// exactly why [`Stage::configure`] has to be idempotent.
    pub fn set_config(&mut self, name: &str, config: StageConfig) -> Result<()> {
        let index = self.index_of(name)?;
        self.stages[index].config = config;
        self.reconfigure(index);
        Ok(())
    }

    /// Hand one stage the row it now sits in. The stage and its config are
    /// separate fields, so one can be borrowed mutably while the other is read.
    fn reconfigure(&mut self, index: usize) {
        let registered = &mut self.stages[index];
        registered.stage.configure(&registered.config);
    }

    fn index_of(&self, name: &str) -> Result<usize> {
        self.stages
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| Error::UnknownStage(name.to_string()))
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

/// Renders the stage order as `name(mode/level)`, which is the only part of a
/// pipeline worth printing.
struct StageList<'a>(&'a [Registered]);

impl std::fmt::Debug for StageList<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|s| {
                format!(
                    "{}({}/{}{})",
                    s.name,
                    s.config.mode.as_str(),
                    s.config.level.as_str(),
                    if s.disabled { ", disabled" } else { "" }
                )
            }))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::types::{Level, Tokens, ToolKind};
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

    /// Every [`Stage::configure`] a stage was handed, in order. Shared with the
    /// test because the pipeline owns the stage once it is registered.
    #[derive(Clone, Default)]
    struct ConfigLog(Arc<Mutex<Vec<StageConfig>>>);

    impl ConfigLog {
        fn calls(&self) -> Vec<StageConfig> {
            self.0
                .lock()
                .expect("no test panics while holding it")
                .clone()
        }
    }

    /// Cuts to whatever its settings say, and only above `Safe` — a stage that
    /// actually uses its configuration, so a test can see the config arrive by
    /// what the content looks like afterwards.
    struct Tunable {
        log: ConfigLog,
        /// Copied out at configure time, not read per call: the hook path may
        /// not search the settings map once per tool call.
        keep: usize,
        level: Level,
    }

    impl Tunable {
        fn new(log: &ConfigLog) -> Self {
            Self {
                log: log.clone(),
                keep: usize::MAX,
                level: Level::default(),
            }
        }
    }

    impl Stage for Tunable {
        fn name(&self) -> &'static str {
            "tunable"
        }
        fn tier(&self) -> Tier {
            Tier::Free
        }
        fn configure(&mut self, config: &StageConfig) {
            self.keep = config.settings.u64("keep", 8) as usize;
            self.level = config.level;
            self.log
                .0
                .lock()
                .expect("no test panics while holding it")
                .push(config.clone());
        }
        fn on_tool_result(&mut self, result: &mut ToolResult) -> Option<Saving> {
            if self.level == Level::Safe || result.content.len() <= self.keep {
                return None;
            }
            let before = result.content.len() as u64;
            result.content = result.content.slice(..self.keep);
            let after = result.content.len() as u64;
            Some(Saving::bytes(before, after, Tokens::Exact(before - after)))
        }
    }

    /// Blows the hard budget on every run, and counts how often it ran.
    ///
    /// The count is the point: "the disabled stage did not run again" is a
    /// statement about invocations, and asserting it with a stopwatch instead
    /// would fail on a loaded machine that simply descheduled us.
    struct Slow(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl Stage for Slow {
        fn name(&self) -> &'static str {
            "slow"
        }
        fn tier(&self) -> Tier {
            Tier::Free
        }
        fn on_tool_result(&mut self, _result: &mut ToolResult) -> Option<Saving> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(12));
            None
        }
    }

    /// A [`StageConfig`] in one line, so a test reads as the config file does.
    fn config(mode: Mode, level: Level, settings: &[(&str, u64)]) -> StageConfig {
        let mut config = StageConfig {
            mode,
            level,
            ..StageConfig::new()
        };
        for (key, value) in settings {
            config.settings.set(key, *value);
        }
        config
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
        let runs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut builder = PipelineBuilder::new();
        builder
            .register(Slow(std::sync::Arc::clone(&runs)), Mode::Active)
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("anything");
        pipeline.run_tool_result(&mut r);

        assert_eq!(pipeline.is_disabled("slow"), Some(true));
        let events = pipeline.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].verdict, Verdict::Hard);
        assert!(events[0].disabled_stage);
        assert_eq!(runs.load(std::sync::atomic::Ordering::Relaxed), 1);

        // The second call must not pay for it again.
        pipeline.run_tool_result(&mut r);
        assert_eq!(
            runs.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a stage over the hard budget ran again"
        );
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
                saving.level = Level::Aggressive;
                Some(saving)
            }
        }

        let mut builder = PipelineBuilder::new();
        builder
            .register_with(Position::Last, Liar, config(Mode::Shadow, Level::Safe, &[]))
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("x");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(savings[0].stage, "honest");
        assert_eq!(savings[0].tier, Tier::Pro);
        assert_eq!(savings[0].mode, Mode::Shadow);
        assert_eq!(savings[0].level, Level::Safe, "not the one it claimed");
    }

    #[test]
    fn register_sets_the_mode_and_leaves_the_rest_compiled_in() {
        let log = ConfigLog::default();
        let mut builder = PipelineBuilder::new();
        builder.register(Tunable::new(&log), Mode::Active).unwrap();
        let mut pipeline = builder.build();

        assert_eq!(
            pipeline.config("tunable"),
            Some(&config(Mode::Active, Level::Balanced, &[])),
            "the old two-argument form still means mode and defaults"
        );
        assert_eq!(log.calls().len(), 1, "configured even without settings");

        // And it runs: `keep` fell back to the stage's own default of 8.
        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"01234567");
        assert_eq!(savings[0].level, Level::Balanced);
    }

    #[test]
    fn register_with_hands_the_stage_the_configuration_it_will_run_with() {
        let log = ConfigLog::default();
        let wanted = config(Mode::Active, Level::Aggressive, &[("keep", 4)]);

        let mut builder = PipelineBuilder::new();
        builder
            .register_with(Position::Last, Tunable::new(&log), wanted.clone())
            .unwrap();
        let mut pipeline = builder.build();

        assert_eq!(log.calls(), vec![wanted.clone()], "once, at registration");
        assert_eq!(pipeline.config("tunable"), Some(&wanted));
        assert_eq!(pipeline.config("nope"), None);

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"0123", "the setting reached the stage");
        assert_eq!(savings[0].level, Level::Aggressive);
    }

    #[test]
    fn register_with_positions_a_stage_like_the_shorter_forms() {
        let log = ConfigLog::default();
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
            .register_with(
                Position::Before("gate"),
                Tunable::new(&log),
                config(Mode::Active, Level::Safe, &[]),
            )
            .unwrap();
        assert_eq!(builder.names(), ["tunable", "gate"]);

        // A duplicate is still refused, and the rejected stage is not left
        // configured for a pipeline it never joined.
        let err = builder
            .register_with(
                Position::Last,
                Tunable::new(&log),
                config(Mode::Active, Level::Safe, &[]),
            )
            .unwrap_err();
        assert!(matches!(err, Error::DuplicateStage(name) if name == "tunable"));
        assert_eq!(log.calls().len(), 1, "only the one that was accepted");
    }

    #[test]
    fn set_config_reconfigures_a_stage_that_is_already_running() {
        let log = ConfigLog::default();
        let first = config(Mode::Active, Level::Aggressive, &[("keep", 4)]);
        let mut builder = PipelineBuilder::new();
        builder
            .register_with(Position::Last, Tunable::new(&log), first.clone())
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("0123456789");
        pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"0123");

        // `lessr level tunable balanced`, with a threshold to match.
        let second = config(Mode::Active, Level::Balanced, &[("keep", 2)]);
        pipeline.set_config("tunable", second.clone()).unwrap();
        assert_eq!(log.calls(), vec![first, second.clone()], "again, in place");
        assert_eq!(pipeline.config("tunable"), Some(&second));

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"01", "the new threshold is in force");
        assert_eq!(savings[0].level, Level::Balanced, "so is the new level");

        assert!(pipeline.set_config("nope", StageConfig::new()).is_err());
    }

    #[test]
    fn the_kill_switch_does_not_drop_the_level_and_settings() {
        let log = ConfigLog::default();
        let registered = config(Mode::Active, Level::Aggressive, &[("keep", 3)]);
        let mut builder = PipelineBuilder::new();
        builder
            .register_with(Position::Last, Tunable::new(&log), registered.clone())
            .unwrap();
        let mut pipeline = builder.build();

        pipeline.set_mode("tunable", Mode::Off).unwrap();
        pipeline.set_mode("tunable", Mode::Shadow).unwrap();

        assert_eq!(
            pipeline.config("tunable"),
            Some(&config(Mode::Shadow, Level::Aggressive, &[("keep", 3)])),
            "off and back on is not a reset to the defaults"
        );
        assert_eq!(
            log.calls().last().map(|c| c.level),
            Some(Level::Aggressive),
            "and the stage was told, so its own copy did not drift"
        );

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"0123456789", "shadow must not mutate");
        assert_eq!(savings[0].bytes_saved(), 7, "keep = 3 survived the switch");
        assert_eq!(savings[0].level, Level::Aggressive);
    }

    #[test]
    fn each_saving_carries_the_level_of_the_stage_that_made_it() {
        // What lets the receipt say *the gate saved this much at balanced*, and
        // the self-healing table tell one level's expand rate from another's.
        let mut builder = PipelineBuilder::new();
        builder
            .register_with(
                Position::Last,
                Truncate {
                    name: "trap",
                    keep: 8,
                },
                config(Mode::Active, Level::Safe, &[]),
            )
            .unwrap();
        builder
            .register_with(
                Position::Last,
                Truncate {
                    name: "gate",
                    keep: 4,
                },
                config(Mode::Active, Level::Aggressive, &[]),
            )
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(savings.len(), 2);
        assert_eq!((savings[0].stage, savings[0].level), ("trap", Level::Safe));
        assert_eq!(
            (savings[1].stage, savings[1].level),
            ("gate", Level::Aggressive)
        );
    }

    #[test]
    fn a_stage_that_ignores_configure_is_configured_anyway() {
        // `Truncate` never implements `configure`. The defaulted method takes
        // the config and drops it; the pipeline still holds it, which is what
        // `lessr config --explain` prints and how an unread key gets reported.
        let given = config(Mode::Active, Level::Safe, &[("nobody_reads_this", 1)]);
        let mut builder = PipelineBuilder::new();
        builder
            .register_with(
                Position::Last,
                Truncate {
                    name: "gate",
                    keep: 5,
                },
                given.clone(),
            )
            .unwrap();
        let mut pipeline = builder.build();

        let mut r = result("0123456789");
        let savings = pipeline.run_tool_result(&mut r);
        assert_eq!(&r.content[..], b"01234");
        assert_eq!(savings[0].bytes_saved(), 5);
        assert_eq!(savings[0].level, Level::Safe);
        assert_eq!(pipeline.config("gate"), Some(&given));
        assert_eq!(
            pipeline.config("gate").map(|c| c.settings.unread()),
            Some(vec!["nobody_reads_this"])
        );
    }
}
