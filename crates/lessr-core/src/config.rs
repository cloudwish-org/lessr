//! The resolved configuration model: what mode, level and settings a stage
//! runs with, which layer decided that, and the floor no layer can cross.
//!
//! This crate owns the model and nothing else. Parsing `config.toml` belongs
//! to the CLI, which builds a [`Config`] through the constructors here and
//! compiles it into a [`crate::Snapshot`]; the hook path reads the snapshot and
//! never sees a TOML parser (technique 7 in `docs/PERFORMANCE.md`). The layers,
//! their order and the floor above them are the contract in `docs/CONFIG.md`.

use std::path::{Path, PathBuf};

use crate::settings::{Settings, Value};
use crate::types::{Level, Mode, ToolResult};

/// Where a resolved value came from.
///
/// The variant order **is** the resolution order (`docs/CONFIG.md`): the
/// highest layer that set a value wins, which is why resolution can be a
/// `max` and not a pile of `if`s. Do not reorder these, and do not add a
/// variant above [`Layer::SafetyFloor`] — nothing outranks the floor.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
pub enum Layer {
    /// The value compiled into Lessr: [`Mode::Shadow`], [`Level::Balanced`],
    /// and for settings whatever default the stage passed to the accessor.
    #[default]
    Default,
    /// `[stages.default]`.
    GlobalDefault,
    /// `[stages.<name>]`.
    GlobalStage,
    /// `[repo."<path>".stages.default]`.
    RepoDefault,
    /// `[repo."<path>".stages.<name>]`.
    RepoStage,
    /// `LESSR_STAGE_GATE_MODE=off`. This is how tests and CI pin behaviour, so
    /// it sits above every file.
    Env,
    /// The self-healing table (loop-safety rule 5). Above everything:
    /// configuration tunes a mechanism, it does not overrule the evidence that
    /// the mechanism is hurting this repo.
    SafetyFloor,
}

impl Layer {
    /// The token `lessr config` prints in the "set by" column.
    pub const fn as_str(self) -> &'static str {
        match self {
            Layer::Default => "default",
            Layer::GlobalDefault => "global-default",
            Layer::GlobalStage => "global-stage",
            Layer::RepoDefault => "repo-default",
            Layer::RepoStage => "repo-stage",
            Layer::Env => "env",
            Layer::SafetyFloor => "safety-floor",
        }
    }

    /// The byte that stands for this layer in the snapshot. Stable across
    /// releases: it is on disk.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Layer::Default => 0,
            Layer::GlobalDefault => 1,
            Layer::GlobalStage => 2,
            Layer::RepoDefault => 3,
            Layer::RepoStage => 4,
            Layer::Env => 5,
            Layer::SafetyFloor => 6,
        }
    }

    /// The layer a snapshot byte stands for, or `None` for a byte written by a
    /// version we do not know.
    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Layer::Default),
            1 => Some(Layer::GlobalDefault),
            2 => Some(Layer::GlobalStage),
            3 => Some(Layer::RepoDefault),
            4 => Some(Layer::RepoStage),
            5 => Some(Layer::Env),
            6 => Some(Layer::SafetyFloor),
            _ => None,
        }
    }
}

/// Everything one stage runs with.
///
/// The three axes of `docs/CONFIG.md`: a kill switch, an intensity and the
/// thresholds behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageConfig {
    /// Whether the stage runs, and whether its output is applied.
    pub mode: Mode,
    /// How much it cuts. Never whether the safety checks run — see [`Level`].
    pub level: Level,
    /// The stage's own tunables.
    pub settings: Settings,
}

impl StageConfig {
    /// The compiled-in defaults: [`Mode::Shadow`], [`Level::Balanced`], no
    /// settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this stage may rewrite this result.
    ///
    /// Two things, and deliberately not three: the stage is not switched off,
    /// and the agent did not ask for exactly these bytes (loop-safety 1). The
    /// level is absent from the answer on purpose — there is no level at which
    /// requested content stops being sacred, so the question a stage naturally
    /// asks cannot be the place it gets that wrong.
    ///
    /// A stage still owes the rest of the contract itself: the error guard runs
    /// after the filter, and every cut leaves a handle, at every level.
    pub fn may_rewrite(&self, result: &ToolResult) -> bool {
        self.mode != Mode::Off && !result.explicit_selection
    }
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            level: Level::default(),
            settings: Settings::new(),
        }
    }
}

/// What one layer says about one stage.
///
/// Partial on purpose: a layer that sets only `mode` must leave `level` to the
/// layer below instead of silently resetting it to the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageOverride {
    /// The mode this layer sets, if it sets one.
    pub mode: Option<Mode>,
    /// The level this layer sets, if it sets one.
    pub level: Option<Level>,
    /// The tunables this layer sets. Merged key by key, not wholesale.
    pub settings: Settings,
}

impl StageOverride {
    /// An override that says nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the mode. Chainable, for the CLI building a layer from TOML.
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Set the level.
    pub fn with_level(mut self, level: Level) -> Self {
        self.level = Some(level);
        self
    }

    /// Set one tunable.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.settings.set(key, value);
        self
    }

    /// Whether this layer says nothing at all about the stage.
    pub fn is_empty(&self) -> bool {
        self.mode.is_none() && self.level.is_none() && self.settings.is_empty()
    }
}

/// A stage the self-healing table has forced off, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorEntry {
    /// The stage that is off.
    pub stage: String,
    /// The repository it is off in, or `None` for everywhere.
    pub repo: Option<PathBuf>,
    /// What the receipt prints. Loop-safety 5 requires the reason be reported,
    /// so it travels with the entry and into the snapshot.
    pub reason: String,
}

/// Stages the evidence has switched off (loop-safety rule 5).
///
/// When handles from one filter are expanded on more than 10 % of its outputs
/// in a repo, that filter goes to [`Mode::Off`] for that repo. This is not a
/// configuration layer that happens to sit high; it is a floor. There is no
/// API here or on [`Config`] that resolves around it, and it is written into
/// the snapshot so the hook path honours it too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SafetyFloor {
    entries: Vec<FloorEntry>,
}

impl SafetyFloor {
    /// A floor that holds nothing down.
    pub fn new() -> Self {
        Self::default()
    }

    /// Force a stage off, for one repository or for every one.
    ///
    /// Forcing the same stage and repo again replaces the reason: the newest
    /// evidence is the one worth printing.
    pub fn force_off(&mut self, stage: &str, repo: Option<&Path>, reason: impl Into<String>) {
        let reason = reason.into();
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.stage == stage && e.repo.as_deref() == repo)
        {
            entry.reason = reason;
            return;
        }
        self.entries.push(FloorEntry {
            stage: stage.to_string(),
            repo: repo.map(Path::to_path_buf),
            reason,
        });
    }

    /// Let a stage run again, once the evidence against it is gone.
    pub fn clear(&mut self, stage: &str, repo: Option<&Path>) {
        self.entries
            .retain(|e| !(e.stage == stage && e.repo.as_deref() == repo));
    }

    /// Why this stage is off here, or `None` if it is not.
    ///
    /// An entry with no repository holds everywhere; an entry with one holds in
    /// that directory and below it, because a hook runs in a subdirectory of
    /// the repo as often as at its root.
    pub fn reason(&self, stage: &str, repo: Option<&Path>) -> Option<&str> {
        self.entries
            .iter()
            .filter(|e| e.stage == stage)
            .find(|e| match (&e.repo, repo) {
                (None, _) => true,
                (Some(scope), Some(path)) => path.starts_with(scope),
                (Some(_), None) => false,
            })
            .map(|e| e.reason.as_str())
    }

    /// Every entry, for `lessr config` and the receipt.
    pub fn entries(&self) -> &[FloorEntry] {
        &self.entries
    }

    /// Whether nothing is held down.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many stages are held down.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The overrides an environment holds, read once.
///
/// `LESSR_STAGE_<STAGE>_MODE` and `LESSR_STAGE_<STAGE>_LEVEL`, which is how a
/// test or a CI job pins one stage without touching anyone's config file. Read
/// once at startup and passed in, never read from inside a stage: the hook path
/// owes the agent 2 ms, and a process environment is not something to walk per
/// tool call.
///
/// A value that does not parse is dropped and reported, never fatal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvOverrides {
    stages: Vec<(String, StageOverride)>,
    rejected: Vec<RejectedEnv>,
}

/// An environment variable that named a stage but not a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedEnv {
    /// The variable, as it was spelled.
    pub var: String,
    /// What it was set to.
    pub value: String,
}

/// The prefix every stage override shares.
const ENV_PREFIX: &str = "LESSR_STAGE_";

impl EnvOverrides {
    /// No overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read `LESSR_STAGE_*` from the process environment.
    ///
    /// Call once, at startup. Variables that are not valid UTF-8 are skipped:
    /// a stage name is ASCII.
    pub fn from_env() -> Self {
        Self::from_pairs(
            std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))),
        )
    }

    /// Build from explicit pairs.
    ///
    /// What the tests use, so they never have to write to a process-wide
    /// environment that every other test in the binary shares.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut env = Self::new();
        for (key, value) in pairs {
            env.insert(key.as_ref(), value.as_ref());
        }
        env
    }

    /// Fold in one variable, ignoring anything that is not a stage override.
    fn insert(&mut self, var: &str, value: &str) {
        let Some(rest) = var.strip_prefix(ENV_PREFIX) else {
            return;
        };
        let (stage, is_mode) = match rest.rsplit_once('_') {
            Some((stage, "MODE")) => (stage, true),
            Some((stage, "LEVEL")) => (stage, false),
            _ => return,
        };
        if stage.is_empty() {
            return;
        }

        let stage = stage.to_ascii_lowercase();
        let parsed = if is_mode {
            Mode::parse(value).map(Axis::Mode)
        } else {
            Level::parse(value).map(Axis::Level)
        };
        let Some(parsed) = parsed else {
            // Loud, not fatal: `lessr config` prints these, and the stage falls
            // back to the layer below.
            self.rejected.push(RejectedEnv {
                var: var.to_string(),
                value: value.to_string(),
            });
            return;
        };

        let entry = match self.stages.iter_mut().find(|(name, _)| *name == stage) {
            Some((_, entry)) => entry,
            None => {
                self.stages.push((stage, StageOverride::new()));
                &mut self.stages.last_mut().expect("just pushed").1
            }
        };
        match parsed {
            Axis::Mode(mode) => entry.mode = Some(mode),
            Axis::Level(level) => entry.level = Some(level),
        }
    }

    /// What the environment says about one stage.
    ///
    /// Matched case-insensitively with `-` and `_` treated as the same
    /// character, so a stage named `edit-verify` answers to
    /// `LESSR_STAGE_EDIT_VERIFY_MODE`, which is the only way an environment
    /// variable can spell it.
    pub fn stage(&self, name: &str) -> Option<&StageOverride> {
        self.stages
            .iter()
            .find(|(stage, _)| name_eq(stage, name))
            .map(|(_, entry)| entry)
    }

    /// Variables that named a stage but not a value `lessr` understands.
    pub fn rejected(&self) -> &[RejectedEnv] {
        &self.rejected
    }

    /// Whether the environment says nothing.
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// How many stages the environment speaks about.
    pub fn len(&self) -> usize {
        self.stages.len()
    }
}

/// Which axis one environment variable set.
enum Axis {
    Mode(Mode),
    Level(Level),
}

/// Stage names compared the way an environment variable has to spell them.
fn name_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes().zip(b.bytes()).all(|(x, y)| {
            let x = if x == b'_' {
                b'-'
            } else {
                x.to_ascii_lowercase()
            };
            let y = if y == b'_' {
                b'-'
            } else {
                y.to_ascii_lowercase()
            };
            x == y
        })
}

/// One repository's overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RepoScope {
    path: PathBuf,
    default: StageOverride,
    stages: Vec<(String, StageOverride)>,
}

/// Every layer, and the floor above them.
///
/// Built by the CLI out of `config.toml` and the self-healing table, then
/// either asked directly ([`Config::resolve`]) or compiled into a snapshot for
/// the hook path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    global_default: StageOverride,
    global: Vec<(String, StageOverride)>,
    repos: Vec<RepoScope>,
    env: EnvOverrides,
    floor: SafetyFloor,
}

impl Config {
    /// A configuration that says nothing, so every stage resolves to the
    /// compiled-in defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// `[stages.default]`.
    pub fn global_default_mut(&mut self) -> &mut StageOverride {
        &mut self.global_default
    }

    /// `[stages.<name>]`, created empty if this is the first mention.
    pub fn global_stage_mut(&mut self, stage: &str) -> &mut StageOverride {
        if let Some(index) = self.global.iter().position(|(name, _)| name == stage) {
            return &mut self.global[index].1;
        }
        self.global.push((stage.to_string(), StageOverride::new()));
        &mut self.global.last_mut().expect("just pushed").1
    }

    /// `[repo."<path>".stages.default]`.
    pub fn repo_default_mut(&mut self, repo: &Path) -> &mut StageOverride {
        &mut self.repo_mut(repo).default
    }

    /// `[repo."<path>".stages.<name>]`.
    pub fn repo_stage_mut(&mut self, repo: &Path, stage: &str) -> &mut StageOverride {
        let scope = self.repo_mut(repo);
        if let Some(index) = scope.stages.iter().position(|(name, _)| name == stage) {
            return &mut scope.stages[index].1;
        }
        scope.stages.push((stage.to_string(), StageOverride::new()));
        &mut scope.stages.last_mut().expect("just pushed").1
    }

    /// The environment layer. Replaces whatever was there.
    pub fn set_env(&mut self, env: EnvOverrides) {
        self.env = env;
    }

    /// The environment layer as it stands.
    pub fn env(&self) -> &EnvOverrides {
        &self.env
    }

    /// The self-healing table, to read or to add to.
    pub fn floor_mut(&mut self) -> &mut SafetyFloor {
        &mut self.floor
    }

    /// The self-healing table.
    pub fn floor(&self) -> &SafetyFloor {
        &self.floor
    }

    /// Every stage named anywhere in this configuration, sorted.
    ///
    /// A stage that is named nowhere needs no entry: it resolves to the
    /// defaults, and that is what an absent entry means.
    pub fn stage_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .global
            .iter()
            .map(|(name, _)| name.as_str())
            .chain(
                self.repos
                    .iter()
                    .flat_map(|r| r.stages.iter().map(|(name, _)| name.as_str())),
            )
            .chain(self.env.stages.iter().map(|(name, _)| name.as_str()))
            .chain(self.floor.entries.iter().map(|e| e.stage.as_str()))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Every repository named anywhere in this configuration, sorted.
    pub fn repo_scopes(&self) -> Vec<&Path> {
        let mut paths: Vec<&Path> = self
            .repos
            .iter()
            .map(|r| r.path.as_path())
            .chain(self.floor.entries.iter().filter_map(|e| e.repo.as_deref()))
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths
    }

    /// Resolve one stage, in one repository, through every layer.
    ///
    /// `repo` is where the agent is working; `None` is the global view. The
    /// safety floor is applied last and cannot be reached around.
    pub fn resolve(&self, stage: &str, repo: Option<&Path>) -> Resolved {
        self.resolve_upto(stage, repo, Layer::Env)
    }

    /// Resolve using only the layers up to and including `highest`.
    ///
    /// The snapshot writer uses this to stop below [`Layer::Env`]: a snapshot
    /// is a file other processes read, and the environment of whichever shell
    /// happened to run `lessr config` has no business in it. The floor applies
    /// whatever `highest` is — it is not a layer, and capping the layers does
    /// not lift it.
    pub fn resolve_upto(&self, stage: &str, repo: Option<&Path>, highest: Layer) -> Resolved {
        let mut resolved = Resolved::defaults(stage);

        for (layer, over) in self.layers(stage, repo) {
            if layer > highest {
                break;
            }
            resolved.apply(layer, over);
        }

        if let Some(reason) = self.floor.reason(stage, repo) {
            resolved.force_off(reason);
        }
        resolved
    }

    /// Every layer with something to say about this stage, lowest first.
    fn layers(&self, stage: &str, repo: Option<&Path>) -> Vec<(Layer, &StageOverride)> {
        let mut layers = vec![(Layer::GlobalDefault, &self.global_default)];
        if let Some(over) = find(&self.global, stage) {
            layers.push((Layer::GlobalStage, over));
        }
        if let Some(scope) = repo.and_then(|path| self.deepest_scope(path)) {
            layers.push((Layer::RepoDefault, &scope.default));
            if let Some(over) = find(&scope.stages, stage) {
                layers.push((Layer::RepoStage, over));
            }
        }
        if let Some(over) = self.env.stage(stage) {
            layers.push((Layer::Env, over));
        }
        layers.retain(|(_, over)| !over.is_empty());
        layers
    }

    /// The repo scope that fits `path` best.
    ///
    /// A hook runs wherever the agent ran its tool, which is as often a
    /// subdirectory of the repository as its root, so a scope holds for
    /// everything under it and the longest match wins.
    fn deepest_scope(&self, path: &Path) -> Option<&RepoScope> {
        self.repos
            .iter()
            .filter(|scope| path.starts_with(&scope.path))
            .max_by_key(|scope| scope.path.as_os_str().len())
    }

    fn repo_mut(&mut self, repo: &Path) -> &mut RepoScope {
        if let Some(index) = self.repos.iter().position(|s| s.path == repo) {
            return &mut self.repos[index];
        }
        self.repos.push(RepoScope {
            path: repo.to_path_buf(),
            ..RepoScope::default()
        });
        self.repos.last_mut().expect("just pushed")
    }
}

fn find<'a>(entries: &'a [(String, StageOverride)], stage: &str) -> Option<&'a StageOverride> {
    entries
        .iter()
        .find(|(name, _)| name == stage)
        .map(|(_, over)| over)
}

/// One stage's configuration and the record of how it got that way.
///
/// The record is the point. `lessr config --explain <stage>` has to be able to
/// say *why* a setting is not taking effect, which means keeping every layer
/// that had an opinion, not just the one that won.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    stage: String,
    config: StageConfig,
    mode: Vec<(Layer, Mode)>,
    level: Vec<(Layer, Level)>,
    settings: Vec<SettingOrigin>,
    floor_reason: Option<String>,
}

/// Every layer that set one tunable, lowest first.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SettingOrigin {
    key: Box<str>,
    layers: Vec<(Layer, Value)>,
}

impl Resolved {
    /// The compiled-in defaults, with [`Layer::Default`] already on every
    /// trace, so a trace is never empty and every value has a source.
    fn defaults(stage: &str) -> Self {
        let config = StageConfig::default();
        Self {
            stage: stage.to_string(),
            mode: vec![(Layer::Default, config.mode)],
            level: vec![(Layer::Default, config.level)],
            config,
            settings: Vec::new(),
            floor_reason: None,
        }
    }

    /// Land one layer on top of what is resolved so far.
    fn apply(&mut self, layer: Layer, over: &StageOverride) {
        if let Some(mode) = over.mode {
            self.config.mode = mode;
            self.mode.push((layer, mode));
        }
        if let Some(level) = over.level {
            self.config.level = level;
            self.level.push((layer, level));
        }
        for (key, value) in over.settings.iter() {
            self.config.settings.set(key, value.clone());
            match self.settings.iter_mut().find(|o| &*o.key == key) {
                Some(origin) => origin.layers.push((layer, value.clone())),
                None => self.settings.push(SettingOrigin {
                    key: key.into(),
                    layers: vec![(layer, value.clone())],
                }),
            }
        }
    }

    /// Apply the safety floor. Only [`Config::resolve_upto`] calls this, and it
    /// calls it last.
    fn force_off(&mut self, reason: &str) {
        self.config.mode = Mode::Off;
        self.mode.push((Layer::SafetyFloor, Mode::Off));
        self.floor_reason = Some(reason.to_string());
    }

    /// The stage this is about.
    pub fn stage(&self) -> &str {
        &self.stage
    }

    /// The mode in force.
    pub fn mode(&self) -> Mode {
        self.config.mode
    }

    /// The level in force.
    pub fn level(&self) -> Level {
        self.config.level
    }

    /// The settings in force.
    pub fn settings(&self) -> &Settings {
        &self.config.settings
    }

    /// Everything in force, together.
    pub fn config(&self) -> &StageConfig {
        &self.config
    }

    /// Take the configuration, dropping the record of how it was reached.
    pub fn into_config(self) -> StageConfig {
        self.config
    }

    /// The layer that set the mode in force.
    pub fn mode_layer(&self) -> Layer {
        self.mode.last().map_or(Layer::Default, |(layer, _)| *layer)
    }

    /// The layer that set the level in force.
    pub fn level_layer(&self) -> Layer {
        self.level
            .last()
            .map_or(Layer::Default, |(layer, _)| *layer)
    }

    /// The layer that set one tunable, or `None` if nothing set it and the
    /// stage's own default at the call site is what will be used.
    pub fn setting_layer(&self, key: &str) -> Option<Layer> {
        self.settings
            .iter()
            .find(|o| &*o.key == key)?
            .layers
            .last()
            .map(|(layer, _)| *layer)
    }

    /// Every layer that set the mode, lowest first; the last one is in force.
    pub fn mode_trace(&self) -> &[(Layer, Mode)] {
        &self.mode
    }

    /// Every layer that set the level, lowest first; the last one is in force.
    pub fn level_trace(&self) -> &[(Layer, Level)] {
        &self.level
    }

    /// Every layer that set one tunable, lowest first. This is what tells a
    /// user their global value is being shadowed by a repo one.
    pub fn setting_trace(&self, key: &str) -> &[(Layer, Value)] {
        self.settings
            .iter()
            .find(|o| &*o.key == key)
            .map_or(&[], |o| &o.layers)
    }

    /// Why the safety floor turned this stage off, if it did.
    pub fn floor_reason(&self) -> Option<&str> {
        self.floor_reason.as_deref()
    }

    /// Whether the self-healing table is what is holding this stage off.
    pub fn forced_off(&self) -> bool {
        self.floor_reason.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::path::PathBuf;

    use crate::types::ToolKind;

    fn repo() -> PathBuf {
        PathBuf::from("/home/dev/work/monorepo")
    }

    #[test]
    fn an_unnamed_stage_resolves_to_the_compiled_in_defaults() {
        let config = Config::new();
        let gate = config.resolve("gate", None);

        assert_eq!(gate.mode(), Mode::Shadow, "every stage ships in shadow");
        assert_eq!(gate.level(), Level::Balanced);
        assert_eq!(gate.mode_layer(), Layer::Default);
        assert_eq!(gate.level_layer(), Layer::Default);
        assert_eq!(gate.settings().u64("max_repeated_lines", 3), 3);
    }

    #[test]
    fn the_global_default_beats_the_compiled_in_default() {
        let mut config = Config::new();
        *config.global_default_mut() = StageOverride::new().with_mode(Mode::Active);

        let gate = config.resolve("gate", None);
        assert_eq!(gate.mode(), Mode::Active);
        assert_eq!(gate.mode_layer(), Layer::GlobalDefault);
        assert_eq!(gate.level(), Level::Balanced, "untouched by that layer");
        assert_eq!(gate.level_layer(), Layer::Default);
    }

    #[test]
    fn a_named_stage_beats_the_global_default() {
        let mut config = Config::new();
        *config.global_default_mut() = StageOverride::new().with_mode(Mode::Shadow);
        *config.global_stage_mut("gate") = StageOverride::new()
            .with_mode(Mode::Active)
            .with_level(Level::Aggressive);

        let gate = config.resolve("gate", None);
        assert_eq!(gate.mode(), Mode::Active);
        assert_eq!(gate.mode_layer(), Layer::GlobalStage);
        assert_eq!(gate.level(), Level::Aggressive);

        let trap = config.resolve("trap", None);
        assert_eq!(trap.mode(), Mode::Shadow, "other stages keep the default");
    }

    #[test]
    fn a_repo_default_beats_a_named_global_stage() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        *config.repo_default_mut(&repo()) = StageOverride::new().with_mode(Mode::Shadow);

        let here = config.resolve("gate", Some(&repo()));
        assert_eq!(here.mode(), Mode::Shadow);
        assert_eq!(here.mode_layer(), Layer::RepoDefault);

        let elsewhere = config.resolve("gate", Some(Path::new("/home/dev/other")));
        assert_eq!(elsewhere.mode(), Mode::Active, "the repo layer is scoped");
    }

    #[test]
    fn a_named_repo_stage_beats_the_repo_default() {
        let mut config = Config::new();
        *config.repo_default_mut(&repo()) = StageOverride::new().with_mode(Mode::Active);
        *config.repo_stage_mut(&repo(), "gate") = StageOverride::new().with_mode(Mode::Off);

        let gate = config.resolve("gate", Some(&repo()));
        assert_eq!(gate.mode(), Mode::Off);
        assert_eq!(gate.mode_layer(), Layer::RepoStage);
        assert_eq!(
            config.resolve("trap", Some(&repo())).mode(),
            Mode::Active,
            "the repo default still holds for everything else"
        );
    }

    #[test]
    fn a_repo_scope_holds_for_everything_under_it() {
        let mut config = Config::new();
        *config.repo_stage_mut(&repo(), "gate") = StageOverride::new().with_mode(Mode::Off);

        let deep = repo().join("crates/lessr-core/src");
        assert_eq!(config.resolve("gate", Some(&deep)).mode(), Mode::Off);
    }

    #[test]
    fn the_deepest_repo_scope_wins() {
        let mut config = Config::new();
        *config.repo_stage_mut(Path::new("/home/dev/work"), "gate") =
            StageOverride::new().with_mode(Mode::Active);
        *config.repo_stage_mut(&repo(), "gate") = StageOverride::new().with_mode(Mode::Off);

        assert_eq!(config.resolve("gate", Some(&repo())).mode(), Mode::Off);
        assert_eq!(
            config
                .resolve("gate", Some(Path::new("/home/dev/work/other")))
                .mode(),
            Mode::Active
        );
    }

    #[test]
    fn a_sibling_path_is_not_a_repo_match() {
        let mut config = Config::new();
        *config.repo_stage_mut(Path::new("/home/dev/less"), "gate") =
            StageOverride::new().with_mode(Mode::Off);

        // `/home/dev/lessr` starts with the same bytes as `/home/dev/less`, and
        // is a different repository.
        let gate = config.resolve("gate", Some(Path::new("/home/dev/lessr")));
        assert_eq!(gate.mode(), Mode::Shadow);
    }

    #[test]
    fn the_environment_beats_every_file_layer() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new()
            .with_mode(Mode::Active)
            .with_level(Level::Aggressive);
        *config.repo_stage_mut(&repo(), "gate") = StageOverride::new().with_mode(Mode::Active);
        config.set_env(EnvOverrides::from_pairs([
            ("LESSR_STAGE_GATE_MODE", "off"),
            ("LESSR_STAGE_GATE_LEVEL", "safe"),
        ]));

        let gate = config.resolve("gate", Some(&repo()));
        assert_eq!(gate.mode(), Mode::Off);
        assert_eq!(gate.mode_layer(), Layer::Env);
        assert_eq!(gate.level(), Level::Safe);
        assert_eq!(gate.level_layer(), Layer::Env);
    }

    #[test]
    fn an_environment_value_nobody_can_spell_is_reported_not_fatal() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        let env = EnvOverrides::from_pairs([
            ("LESSR_STAGE_GATE_MODE", "banana"),
            ("LESSR_STAGE_GATE_LEVEL", "sideways"),
            ("PATH", "/usr/bin"),
        ]);

        assert_eq!(env.rejected().len(), 2);
        assert_eq!(env.rejected()[0].var, "LESSR_STAGE_GATE_MODE");
        assert!(env.is_empty(), "nothing usable, so nothing overridden");

        config.set_env(env);
        let gate = config.resolve("gate", None);
        assert_eq!(gate.mode(), Mode::Active, "the layer below still holds");
    }

    #[test]
    fn an_environment_variable_can_spell_a_hyphenated_stage() {
        let env = EnvOverrides::from_pairs([("LESSR_STAGE_EDIT_VERIFY_MODE", "off")]);
        assert_eq!(
            env.stage("edit-verify").and_then(|o| o.mode),
            Some(Mode::Off)
        );
        assert_eq!(env.stage("gate"), None);
    }

    #[test]
    fn the_safety_floor_beats_every_layer_including_the_environment() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        *config.repo_stage_mut(&repo(), "gate") = StageOverride::new().with_mode(Mode::Active);
        config.set_env(EnvOverrides::from_pairs([(
            "LESSR_STAGE_GATE_MODE",
            "active",
        )]));
        config
            .floor_mut()
            .force_off("gate", Some(&repo()), "handles expanded on 14 % of outputs");

        let gate = config.resolve("gate", Some(&repo()));
        assert_eq!(gate.mode(), Mode::Off);
        assert_eq!(gate.mode_layer(), Layer::SafetyFloor);
        assert!(gate.forced_off());
        assert_eq!(
            gate.floor_reason(),
            Some("handles expanded on 14 % of outputs")
        );
    }

    #[test]
    fn the_floor_only_holds_in_the_repo_that_earned_it() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        config
            .floor_mut()
            .force_off("gate", Some(&repo()), "expanded too often");

        assert_eq!(config.resolve("gate", Some(&repo())).mode(), Mode::Off);
        assert_eq!(
            config
                .resolve("gate", Some(Path::new("/home/dev/other")))
                .mode(),
            Mode::Active
        );
        assert_eq!(config.resolve("gate", None).mode(), Mode::Active);
        assert_eq!(
            config.resolve("trap", Some(&repo())).mode(),
            Mode::Shadow,
            "the floor is per stage"
        );
    }

    #[test]
    fn a_floor_with_no_repo_holds_everywhere_and_can_be_lifted() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        config.floor_mut().force_off("gate", None, "expanded");

        assert_eq!(config.resolve("gate", Some(&repo())).mode(), Mode::Off);
        assert_eq!(config.resolve("gate", None).mode(), Mode::Off);

        config.floor_mut().clear("gate", None);
        assert_eq!(config.resolve("gate", None).mode(), Mode::Active);
    }

    #[test]
    fn capping_the_layers_does_not_lift_the_floor() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        config.set_env(EnvOverrides::from_pairs([(
            "LESSR_STAGE_GATE_LEVEL",
            "aggressive",
        )]));
        config.floor_mut().force_off("gate", None, "expanded");

        let capped = config.resolve_upto("gate", None, Layer::RepoStage);
        assert_eq!(capped.mode(), Mode::Off, "the floor is not a layer");
        assert_eq!(
            capped.level(),
            Level::Balanced,
            "the environment was capped out"
        );
        assert_eq!(config.resolve("gate", None).level(), Level::Aggressive);
    }

    #[test]
    fn settings_layer_key_by_key() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new()
            .with("max_repeated_lines", 3u64)
            .with("summary_after_lines", 400u64);
        *config.repo_stage_mut(&repo(), "gate") =
            StageOverride::new().with("summary_after_lines", 2000u64);

        let gate = config.resolve("gate", Some(&repo()));
        assert_eq!(gate.settings().u64("max_repeated_lines", 0), 3);
        assert_eq!(gate.settings().u64("summary_after_lines", 0), 2000);
        assert_eq!(
            gate.setting_layer("max_repeated_lines"),
            Some(Layer::GlobalStage)
        );
        assert_eq!(
            gate.setting_layer("summary_after_lines"),
            Some(Layer::RepoStage)
        );
        assert_eq!(gate.setting_layer("strip_timestamps"), None);
    }

    #[test]
    fn provenance_keeps_every_layer_that_spoke_not_just_the_winner() {
        let mut config = Config::new();
        *config.global_default_mut() = StageOverride::new().with_mode(Mode::Shadow);
        *config.global_stage_mut("gate") = StageOverride::new()
            .with_mode(Mode::Active)
            .with("json_bytes", 65536u64);
        *config.repo_stage_mut(&repo(), "gate") = StageOverride::new()
            .with_mode(Mode::Off)
            .with("json_bytes", 131072u64);

        let gate = config.resolve("gate", Some(&repo()));
        assert_eq!(
            gate.mode_trace(),
            [
                (Layer::Default, Mode::Shadow),
                (Layer::GlobalDefault, Mode::Shadow),
                (Layer::GlobalStage, Mode::Active),
                (Layer::RepoStage, Mode::Off),
            ]
        );
        assert_eq!(
            gate.setting_trace("json_bytes"),
            [
                (Layer::GlobalStage, Value::Int(65536)),
                (Layer::RepoStage, Value::Int(131072)),
            ]
        );
        assert!(gate.setting_trace("nothing").is_empty());
    }

    #[test]
    fn a_bad_setting_still_resolves_and_is_reported() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new()
            .with("max_repeated_lines", "three")
            .with("stirp_timestamps", true);

        let gate = config.resolve("gate", None);
        let settings = gate.settings();
        assert_eq!(settings.u64("max_repeated_lines", 3), 3, "the default");
        assert_eq!(settings.rejected().len(), 1);
        assert_eq!(settings.unread(), ["stirp_timestamps"], "a typo, surfaced");
    }

    #[test]
    fn a_level_never_makes_requested_content_rewritable() {
        let mut asked_for = ToolResult::new(ToolKind::Read, "Read", Bytes::from_static(b"x"));
        asked_for.explicit_selection = true;
        let ordinary = ToolResult::new(ToolKind::Read, "Read", Bytes::from_static(b"x"));

        for level in Level::ALL {
            let config = StageConfig {
                mode: Mode::Active,
                level,
                settings: Settings::new(),
            };
            assert!(
                !config.may_rewrite(&asked_for),
                "loop-safety 1 at {}",
                level.as_str()
            );
            assert!(config.may_rewrite(&ordinary));
        }
    }

    #[test]
    fn a_stage_that_is_off_rewrites_nothing_at_any_level() {
        let ordinary = ToolResult::new(ToolKind::Read, "Read", Bytes::from_static(b"x"));
        for level in Level::ALL {
            let config = StageConfig {
                mode: Mode::Off,
                level,
                settings: Settings::new(),
            };
            assert!(!config.may_rewrite(&ordinary));
        }
    }

    #[test]
    fn the_names_and_scopes_a_snapshot_has_to_cover() {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        *config.repo_stage_mut(&repo(), "trap") = StageOverride::new().with_mode(Mode::Off);
        config
            .floor_mut()
            .force_off("dedup", Some(Path::new("/home/dev/other")), "expanded");
        config.set_env(EnvOverrides::from_pairs([(
            "LESSR_STAGE_DETECT_MODE",
            "off",
        )]));

        assert_eq!(config.stage_names(), ["dedup", "detect", "gate", "trap"]);
        assert_eq!(
            config.repo_scopes(),
            [Path::new("/home/dev/other"), repo().as_path()]
        );
    }
}
