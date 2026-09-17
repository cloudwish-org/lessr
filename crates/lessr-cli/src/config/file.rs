//! `config.toml`: reading it into the model, and editing it in place.
//!
//! This is the boundary `lessr-core` is built around. The model — the layers,
//! the floor above them, the snapshot the hook maps — lives there and knows
//! nothing about TOML; the parser stops here, in a binary a human runs, so the
//! path that runs per tool call never pays for one (`docs/PERFORMANCE.md`,
//! technique 7).
//!
//! Two rules shape everything below.
//!
//! **Nothing in a config file is fatal.** A value that does not parse falls
//! back to its default and is reported; a key nothing reads is reported too.
//! `docs/CONFIG.md` is explicit about why: a bad config line must not break
//! someone's agent. So [`load`] returns a [`Loaded`] with a list of problems,
//! never an error.
//!
//! **The file belongs to the user.** It is edited in place, through
//! `toml_edit`, so comments, key order and formatting survive a
//! `lessr config set`. Parsing and re-serialising would reformat the whole
//! file to make one change, which is how a tool teaches people to stop letting
//! it near their files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use lessr_core::{Config, Level, Mode, StageOverride, Value, ValueKind};
use toml_edit::{DocumentMut, Item, Table, TableLike};

use super::{Problem, first_line, known};

/// The file, inside the config directory.
pub const FILE_NAME: &str = "config.toml";

/// What `config.toml` says, and everything wrong with it.
pub struct Loaded {
    /// The layers the file describes. The environment and the safety floor are
    /// added by the caller; this is the file's own contribution.
    pub config: Config,
    /// `[proxy] port`, when the file sets one that can be one.
    pub port: Option<u16>,
    /// Malformed values and keys nothing reads. Printed by `lessr config`;
    /// never fatal.
    pub problems: Vec<Problem>,
    /// Set when the document is not TOML at all, in which case nothing in
    /// `config` came from it. The one failure a write has to refuse: we will
    /// not rewrite a file we could not read.
    pub unparseable: Option<String>,
    /// Whether the file is there at all.
    pub exists: bool,
}

/// Read `config.toml`. Never fails; everything wrong with the file is
/// collected rather than raised.
pub fn load(path: &Path) -> Loaded {
    let mut loaded = Loaded {
        config: Config::new(),
        port: None,
        problems: Vec::new(),
        unparseable: None,
        exists: true,
    };

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            loaded.exists = false;
            return loaded;
        }
        Err(err) => {
            loaded.unparseable = Some(format!("cannot be read: {err}"));
            return loaded;
        }
    };

    let document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(err) => {
            loaded.unparseable = Some(format!("is not TOML: {}", first_line(&err)));
            return loaded;
        }
    };

    for (key, item) in document.iter() {
        match key {
            "stages" => read_stages(item, None, &mut loaded),
            "repo" => read_repos(item, &mut loaded),
            "proxy" => read_proxy(item, &mut loaded),
            other => loaded.problems.push(Problem::new(
                other,
                "nothing in this build reads this section",
            )),
        }
    }

    loaded
}

/// `[stages.*]`, either at the top level or inside a `[repo."<path>"]`.
fn read_stages(item: &Item, repo: Option<&Path>, loaded: &mut Loaded) {
    // Spelled the way the file spells it, so a problem can be found by
    // searching for what it prints.
    let prefix = match repo {
        None => "stages".to_string(),
        Some(path) => format!("repo.\"{}\".stages", path.display()),
    };
    let Some(table) = item.as_table_like() else {
        loaded
            .problems
            .push(Problem::new(prefix, "is not a table of stages; ignored"));
        return;
    };

    for (name, entry) in table.iter() {
        let at = format!("{prefix}.{name}");
        let Some(stage) = entry.as_table_like() else {
            loaded.problems.push(Problem::new(
                at,
                "is not a table; a stage is configured with `[stages.<name>]`",
            ));
            continue;
        };

        // A name this build does not register is reported once, here, and then
        // left alone: the Pro binary is this same CLI with more stages, so its
        // keys are not typos and complaining about each of them would make the
        // free binary unusable for reading a Pro config.
        let registered = name == known::DEFAULT || known::stage(name).is_some();
        if !registered {
            loaded.problems.push(Problem::new(
                at.clone(),
                "no mechanism of that name is registered in this build",
            ));
        }

        let over = read_stage(stage, name, &at, registered, loaded);
        if over.is_empty() {
            continue;
        }
        let target = match (repo, name == known::DEFAULT) {
            (None, true) => loaded.config.global_default_mut(),
            (None, false) => loaded.config.global_stage_mut(name),
            (Some(path), true) => loaded.config.repo_default_mut(path),
            (Some(path), false) => loaded.config.repo_stage_mut(path, name),
        };
        merge(target, over);
    }
}

/// One stage table: the two axes, and everything else as a tunable.
fn read_stage(
    table: &dyn TableLike,
    stage: &str,
    prefix: &str,
    registered: bool,
    loaded: &mut Loaded,
) -> StageOverride {
    let mut over = StageOverride::new();

    for (key, item) in table.iter() {
        let at = format!("{prefix}.{key}");
        match key {
            "mode" => match item.as_str().and_then(Mode::parse) {
                Some(mode) => over.mode = Some(mode),
                None => loaded.problems.push(Problem::new(
                    at,
                    format!(
                        "{} is not a mode (off, shadow, active); the layer below it is in force",
                        rendered(item)
                    ),
                )),
            },
            "level" => match item.as_str().and_then(Level::parse) {
                Some(level) => over.level = Some(level),
                None => loaded.problems.push(Problem::new(
                    at,
                    format!(
                        "{} is not a level (safe, balanced, aggressive); the layer below it is in force",
                        rendered(item)
                    ),
                )),
            },
            _ => {
                let Some(value) = scalar(item) else {
                    loaded.problems.push(Problem::new(
                        at,
                        "only a number, a string or true/false can tune a mechanism; ignored",
                    ));
                    continue;
                };
                match known::tunable(stage, key) {
                    // Dropped, not carried: `lessr config` promises the value
                    // in force, and the value in force is the stage's own
                    // default the moment the stage cannot use this one.
                    Some(tunable) if !fits(&value, tunable.kind) => {
                        loaded.problems.push(Problem::new(
                            at,
                            format!(
                                "{} is not {}; {} stays in force",
                                rendered(item),
                                tunable.kind.as_str(),
                                tunable.default
                            ),
                        ));
                    }
                    Some(_) => over.settings.set(key, value),
                    None => {
                        // Kept: some other build's stage may read it, and the
                        // snapshot is where that build would look for it.
                        over.settings.set(key, value);
                        if registered {
                            loaded.problems.push(Problem::new(
                                at,
                                "nothing in this build reads this key",
                            ));
                        }
                    }
                }
            }
        }
    }

    over
}

/// `[repo."<path>".…]`.
fn read_repos(item: &Item, loaded: &mut Loaded) {
    let Some(table) = item.as_table_like() else {
        loaded.problems.push(Problem::new(
            "repo",
            "is not a table of repositories; ignored",
        ));
        return;
    };

    for (path, entry) in table.iter() {
        let at = format!("repo.{path:?}");
        let Some(scope) = entry.as_table_like() else {
            loaded
                .problems
                .push(Problem::new(at, "is not a table; ignored"));
            continue;
        };
        // A relative path can never be a prefix of the absolute directory the
        // hook runs in, so the scope would match nothing, for ever, in silence.
        if !Path::new(path).is_absolute() {
            loaded.problems.push(Problem::new(
                at.clone(),
                "is not an absolute path, so it can never match a repository",
            ));
        }

        for (key, inner) in scope.iter() {
            match key {
                "stages" => read_stages(inner, Some(Path::new(path)), loaded),
                other => loaded.problems.push(Problem::new(
                    format!("{at}.{other}"),
                    "nothing in this build reads this key",
                )),
            }
        }
    }
}

/// `[proxy]`.
fn read_proxy(item: &Item, loaded: &mut Loaded) {
    let Some(table) = item.as_table_like() else {
        loaded
            .problems
            .push(Problem::new("proxy", "is not a table; ignored"));
        return;
    };

    for (key, value) in table.iter() {
        match key {
            "port" => match value.as_integer().and_then(|n| u16::try_from(n).ok()) {
                Some(0) | None => loaded.problems.push(Problem::new(
                    "proxy.port",
                    format!(
                        "{} is not a port (1 to 65535); the default port stays in force",
                        rendered(value)
                    ),
                )),
                Some(port) => loaded.port = Some(port),
            },
            other => loaded.problems.push(Problem::new(
                format!("proxy.{other}"),
                "nothing in this build reads this key",
            )),
        }
    }
}

/// Land one layer's reading on whatever is already there.
///
/// Per field and per key, never wholesale: one file can name a stage twice
/// (`[stages.gate]` and a dotted `stages.gate.level`), and the second mention
/// must not erase the first.
fn merge(target: &mut StageOverride, over: StageOverride) {
    if over.mode.is_some() {
        target.mode = over.mode;
    }
    if over.level.is_some() {
        target.level = over.level;
    }
    target.settings.merge(&over.settings);
}

/// A TOML scalar as the model holds it, or `None` for a value that is not a
/// scalar at all.
fn scalar(item: &Item) -> Option<Value> {
    if let Some(text) = item.as_str() {
        return Some(Value::from(text));
    }
    if let Some(number) = item.as_integer() {
        return Some(Value::Int(number));
    }
    item.as_bool().map(Value::Bool)
}

/// Whether a stage asking for `kind` could use this value.
///
/// Asked through the model's own accessors, so the answer here is the answer
/// the stage will get: `max_lines = "200"` is a number, and `-3` is not one.
fn fits(value: &Value, kind: ValueKind) -> bool {
    match kind {
        ValueKind::Int => value.as_u64().is_some(),
        ValueKind::Bool => value.as_bool().is_some(),
        ValueKind::Str => value.as_str().is_some(),
    }
}

/// A value as the file spells it, for a message that quotes it back.
fn rendered(item: &Item) -> String {
    let text = item
        .as_value()
        .map_or_else(|| item.type_name().to_string(), |value| value.to_string());
    format!("`{}`", text.trim())
}

/// Where a value goes in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The tables to walk down, outermost first.
    path: Vec<String>,
    /// The key inside the innermost one.
    key: String,
}

impl Slot {
    /// One field of one stage, globally or inside a repository scope.
    pub fn stage(repo: Option<&Path>, stage: &str, key: &str) -> Slot {
        let mut path = Vec::new();
        if let Some(repo) = repo {
            path.push("repo".to_string());
            path.push(repo.display().to_string());
        }
        path.push("stages".to_string());
        path.push(stage.to_string());
        Slot {
            path,
            key: key.to_string(),
        }
    }

    /// One `[proxy]` key.
    pub fn proxy(key: &str) -> Slot {
        Slot {
            path: vec!["proxy".to_string()],
            key: key.to_string(),
        }
    }

    /// The key itself, for a message that has already said where it is.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// How a message spells it: `stages.gate.mode`.
    pub fn dotted(&self) -> String {
        let mut out = String::new();
        for name in &self.path {
            // Quoted where TOML would quote it, so what is printed is what the
            // file contains.
            if bare(name) {
                out.push_str(name);
            } else {
                out.push_str(&format!("{name:?}"));
            }
            out.push('.');
        }
        out.push_str(&self.key);
        out
    }
}

/// Whether a key can be written without quotes.
fn bare(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// What one write changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    /// What the key said before, if it said anything.
    pub before: Option<String>,
    /// Whether the file had to be created.
    pub created: bool,
}

/// Write one value into `config.toml`, leaving every other byte of it alone.
///
/// The document is edited, not rebuilt: an existing key keeps its position and
/// the comments around it, and a new one is appended under its own header. A
/// file that does not parse is never written to — we would be overwriting
/// something we could not read.
pub fn write(path: &Path, slot: &Slot, value: toml_edit::Value) -> Result<Written> {
    let (text, created) = match std::fs::read_to_string(path) {
        Ok(text) => (text, false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (skeleton(), true),
        Err(err) => {
            return Err(anyhow::Error::from(err).context(format!("cannot read {}", path.display())));
        }
    };

    let mut document = text.parse::<DocumentMut>().map_err(|err| {
        anyhow!(
            "{} does not parse ({}). Nothing was written: fix the file first.",
            path.display(),
            first_line(&err)
        )
    })?;

    let table = table_at(&mut document, &slot.path, &slot.dotted())?;
    let before = table.get(&slot.key).map(unquoted);

    match table.get_mut(&slot.key) {
        // Replace the value and nothing else. `insert` would reset the key's
        // formatting, and with it any comment the user wrote above the line.
        Some(existing) => {
            let decor = existing.as_value().map(|old| old.decor().clone());
            let mut fresh = toml_edit::value(value);
            if let (Some(decor), Some(new)) = (decor, fresh.as_value_mut()) {
                *new.decor_mut() = decor;
            }
            *existing = fresh;
        }
        None => {
            table.insert(&slot.key, toml_edit::value(value));
        }
    }

    atomic_write(path, document.to_string().as_bytes())?;
    Ok(Written { before, created })
}

/// Walk down to the table a slot lives in, creating what is missing.
fn table_at<'d>(
    document: &'d mut DocumentMut,
    path: &[String],
    slot: &str,
) -> Result<&'d mut dyn TableLike> {
    let mut current: &mut dyn TableLike = document.as_table_mut();

    for (depth, name) in path.iter().enumerate() {
        let leaf = depth + 1 == path.len();
        let item = current.entry(name).or_insert_with(|| {
            let mut fresh = Table::new();
            // A table created only to hold another one prints no header of its
            // own: `[stages]` above `[stages.gate]` is a line the user did not
            // write and does not need.
            fresh.set_implicit(!leaf);
            Item::Table(fresh)
        });
        current = item.as_table_like_mut().ok_or_else(|| {
            anyhow!(
                "cannot set `{slot}`: `{name}` is already a value in this file, not a table. \
                 Fix it by hand."
            )
        })?;
    }

    Ok(current)
}

/// A value as a human would say it: `active`, not `"active"`.
fn unquoted(item: &Item) -> String {
    match item.as_str() {
        Some(text) => text.to_string(),
        None => item
            .as_value()
            .map_or_else(|| item.type_name().to_string(), |value| value.to_string())
            .trim()
            .to_string(),
    }
}

/// Replace a file's contents through a temporary file and a rename.
///
/// The same technique as the snapshot, for the same reason: a crash or a full
/// disk halfway through leaves the old file whole. This one is a file a human
/// wrote, which makes losing it worse than losing a snapshot we can rebuild.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let name = path
        .file_name()
        .ok_or_else(|| anyhow!("{} has no file name", path.display()))?;
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);

    let written = std::fs::write(&temp, bytes);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(anyhow::Error::from(err).context(format!("cannot write {}", temp.display())));
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(anyhow::Error::from(err).context(format!("cannot write {}", path.display())));
    }
    Ok(())
}

/// The file `lessr` creates when there is none: the contract, in comments.
///
/// It ends with `[stages.default]` rather than a bare comment block on
/// purpose. Comments with no item under them belong to nothing, and a table
/// added later would be written above them; anchored to a header, they stay at
/// the top where they were meant to be read.
fn skeleton() -> String {
    r#"# Lessr configuration.
#
# Every mechanism has a kill switch, an intensity and its own settings.
# Nothing is all-or-nothing, and nothing is hidden: `lessr config` prints the
# value in force and which layer set it.
#
#   mode  = "off" | "shadow" | "active"
#           `shadow` runs the mechanism, counts what it would have saved and
#           throws the rewrite away. Every new mechanism ships in it.
#   level = "safe" | "balanced" | "aggressive"
#           How much is cut. Never whether the safety checks run: errors pass
#           at every level, and every cut leaves a handle at every level.
#
# Layers, lowest to highest. The highest layer that sets a value wins, per
# field, so a layer that sets only `mode` leaves `level` to the one below it.
#
#   [stages.default]                 every mechanism
#   [stages.<name>]                  one of them
#   [repo."<path>".stages.default]   every mechanism, in one repository
#   [repo."<path>".stages.<name>]    one of them, in one repository
#   LESSR_STAGE_GATE_MODE=off        the environment, above every file
#
# Above all of them is the safety floor: a mechanism the self-healing table
# turned off for a repository stays off, and nothing here can promote it.
#
# `lessr config set stages.gate.level safe` edits this file without opening
# it, and keeps what you wrote in it. Full contract: docs/CONFIG.md.

[stages.default]
"#
    .to_string()
}

/// Where the file lives, given the config directory.
pub fn path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lessr_core::Layer;

    fn load_text(text: &str) -> (tempfile::TempDir, Loaded) {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        std::fs::write(&path, text).unwrap();
        let loaded = load(&path);
        (dir, loaded)
    }

    /// The example from `docs/CONFIG.md`, which is the contract this parses.
    const DOCUMENTED: &str = r#"
[stages.default]
mode = "shadow"

[stages.gate]
mode = "active"
level = "balanced"
max_repeated_lines = 3

[stages.trap]
mode = "active"
level = "safe"
json_bytes = 131072

# This repo generates enormous fixtures the agent genuinely needs to read.
[repo."/home/dev/work/monorepo".stages.gate]
mode = "off"
"#;

    #[test]
    fn the_documented_example_reads_as_the_documented_layers() {
        let (_dir, loaded) = load_text(DOCUMENTED);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);

        let gate = loaded.config.resolve("gate", None);
        assert_eq!(gate.mode(), Mode::Active);
        assert_eq!(gate.mode_layer(), Layer::GlobalStage);
        assert_eq!(gate.settings().u64("max_repeated_lines", 0), 3);

        let dedup = loaded.config.resolve("dedup", None);
        assert_eq!(dedup.mode(), Mode::Shadow, "from [stages.default]");
        assert_eq!(dedup.mode_layer(), Layer::GlobalDefault);

        let repo = Path::new("/home/dev/work/monorepo/crates");
        let scoped = loaded.config.resolve("gate", Some(repo));
        assert_eq!(scoped.mode(), Mode::Off);
        assert_eq!(scoped.mode_layer(), Layer::RepoStage);
        assert_eq!(
            scoped.level(),
            Level::Balanced,
            "the repo layer set only the mode"
        );
    }

    #[test]
    fn a_value_nobody_can_spell_falls_back_and_is_reported() {
        let (_dir, loaded) = load_text(
            "[stages.gate]\nmode = \"banana\"\nlevel = 3\nmax_repeated_lines = \"many\"\n",
        );

        let gate = loaded.config.resolve("gate", None);
        assert_eq!(gate.mode(), Mode::Shadow, "the compiled-in default");
        assert_eq!(gate.level(), Level::Balanced);
        assert_eq!(gate.settings().get("max_repeated_lines"), None);

        let at: Vec<&str> = loaded.problems.iter().map(|p| p.at.as_str()).collect();
        assert_eq!(
            at,
            [
                "stages.gate.mode",
                "stages.gate.level",
                "stages.gate.max_repeated_lines"
            ]
        );
        assert!(loaded.problems[0].says.contains("off, shadow, active"));
    }

    #[test]
    fn a_number_written_as_text_is_still_a_number() {
        // The model accepts it, so the front end must not be stricter than the
        // stage that reads it.
        let (_dir, loaded) = load_text("[stages.gate]\nmax_repeated_lines = \"5\"\n");
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert_eq!(
            loaded
                .config
                .resolve("gate", None)
                .settings()
                .u64("max_repeated_lines", 0),
            5
        );
    }

    #[test]
    fn an_unknown_key_is_kept_and_reported() {
        let (_dir, loaded) = load_text("[stages.gate]\nmax_repated_lines = 3\n");
        assert_eq!(loaded.problems.len(), 1);
        assert_eq!(loaded.problems[0].at, "stages.gate.max_repated_lines");
        assert!(loaded.problems[0].says.contains("nothing in this build"));
        assert!(
            loaded
                .config
                .resolve("gate", None)
                .settings()
                .get("max_repated_lines")
                .is_some(),
            "kept: another build's stage may be the one that reads it"
        );
    }

    #[test]
    fn a_stage_this_build_does_not_have_is_reported_once_not_per_key() {
        // A Pro config read by the free binary. One line about the stage, and
        // silence about the keys underneath it.
        let (_dir, loaded) =
            load_text("[stages.cachefix]\nmode = \"active\"\nprefix_bytes = 2048\nwindow = 12\n");
        assert_eq!(loaded.problems.len(), 1);
        assert_eq!(loaded.problems[0].at, "stages.cachefix");
        assert_eq!(
            loaded.config.resolve("cachefix", None).mode(),
            Mode::Active,
            "its configuration still resolves, and still reaches the snapshot"
        );
    }

    #[test]
    fn a_section_nobody_reads_is_reported() {
        let (_dir, loaded) = load_text("[stagse]\ngate = \"on\"\n");
        assert_eq!(loaded.problems.len(), 1);
        assert_eq!(loaded.problems[0].at, "stagse");
    }

    #[test]
    fn a_relative_repo_scope_can_never_match_and_says_so() {
        let (_dir, loaded) = load_text("[repo.\"work/monorepo\".stages.gate]\nmode = \"off\"\n");
        assert_eq!(loaded.problems.len(), 1);
        assert!(loaded.problems[0].says.contains("absolute"));
    }

    #[test]
    fn a_file_that_is_not_toml_is_refused_rather_than_half_read() {
        let (_dir, loaded) = load_text("[stages.gate\nmode = \"off\"\n");
        assert!(loaded.unparseable.is_some());
        assert_eq!(loaded.config, Config::new());
    }

    #[test]
    fn an_absent_file_says_nothing_and_is_not_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&path_in(dir.path()));
        assert!(!loaded.exists);
        assert!(loaded.problems.is_empty());
        assert_eq!(loaded.config, Config::new());
    }

    #[test]
    fn the_proxy_port_is_read_and_a_bad_one_is_reported() {
        let (_dir, loaded) = load_text("[proxy]\nport = 7500\n");
        assert_eq!(loaded.port, Some(7500));

        let (_dir, loaded) = load_text("[proxy]\nport = 99999\nhost = \"::1\"\n");
        assert_eq!(loaded.port, None);
        assert_eq!(loaded.problems.len(), 2);
    }

    #[test]
    fn a_write_keeps_the_comments_and_the_order_around_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        let original = "# my notes\n[stages.gate]\n# keep this low, the agent re-reads a lot\nmax_repeated_lines = 3  # ← here\nmode = \"shadow\"\n";
        std::fs::write(&path, original).unwrap();

        let slot = Slot::stage(None, "gate", "mode");
        let written = write(&path, &slot, "active".into()).unwrap();
        assert_eq!(written.before.as_deref(), Some("shadow"));
        assert!(!written.created);

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            "# my notes\n[stages.gate]\n# keep this low, the agent re-reads a lot\nmax_repeated_lines = 3  # ← here\nmode = \"active\"\n",
            "everything but the one value must survive, formatting included"
        );
    }

    #[test]
    fn a_new_file_gets_the_skeleton_and_the_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());

        let written = write(&path, &Slot::stage(None, "gate", "mode"), "off".into()).unwrap();
        assert!(written.created);
        assert_eq!(written.before, None);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Lessr configuration."), "{text}");
        assert!(text.contains("[stages.gate]\nmode = \"off\""), "{text}");
        // And it has to read back: a skeleton that does not parse would break
        // the next command rather than the current one.
        let loaded = load(&path);
        assert!(loaded.unparseable.is_none());
        assert_eq!(loaded.config.resolve("gate", None).mode(), Mode::Off);
    }

    #[test]
    fn a_repo_scope_is_written_as_a_quoted_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        let repo = Path::new("/home/dev/work/monorepo");

        write(
            &path,
            &Slot::stage(Some(repo), "gate", "mode"),
            "off".into(),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("[repo.\"/home/dev/work/monorepo\".stages.gate]"),
            "{text}"
        );

        let loaded = load(&path);
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        assert_eq!(loaded.config.resolve("gate", Some(repo)).mode(), Mode::Off);
    }

    #[test]
    fn writing_twice_replaces_rather_than_stacks() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        let slot = Slot::stage(None, "gate", "level");

        write(&path, &slot, "safe".into()).unwrap();
        let second = write(&path, &slot, "aggressive".into()).unwrap();

        assert_eq!(second.before.as_deref(), Some("safe"));
        let text = std::fs::read_to_string(&path).unwrap();
        // Set lines only; the skeleton's comments explain the key as well.
        let set = text
            .lines()
            .filter(|line| line.trim_start().starts_with("level ="))
            .count();
        assert_eq!(set, 1, "{text}");
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        let broken = "[stages.gate\nmode = \"off\"\n";
        std::fs::write(&path, broken).unwrap();

        let err = write(&path, &Slot::stage(None, "gate", "mode"), "on".into()).unwrap_err();
        assert!(err.to_string().contains("Nothing was written"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn a_slot_spells_itself_the_way_the_file_does() {
        assert_eq!(
            Slot::stage(None, "gate", "mode").dotted(),
            "stages.gate.mode"
        );
        assert_eq!(
            Slot::stage(Some(Path::new("/home/dev/work")), "gate", "level").dotted(),
            "repo.\"/home/dev/work\".stages.gate.level"
        );
        assert_eq!(Slot::proxy("port").dotted(), "proxy.port");
    }
}
