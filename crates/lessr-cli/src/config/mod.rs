//! `lessr on | off | shadow | level | config`: the front end to the
//! configuration model.
//!
//! The model lives in `lessr-core`: the six layers, the safety floor above
//! them, and the binary snapshot the hook path memory-maps. It has no TOML
//! parser and must not gain one — the hook runs on every tool call and owes
//! the agent 2 ms (`docs/PERFORMANCE.md`, technique 7). This module is the
//! other half of that arrangement, and the only place in Lessr where a config
//! file is read or written:
//!
//! - [`file`] reads `config.toml` into a `Config` and edits it in place;
//! - [`floor`] reads the self-healing table that sits above every layer;
//! - [`snapshot`] keeps `config.bin` in step with both;
//! - [`show`] prints what is in force and which layer decided it.
//!
//! Every command here ends the same way: write the file, re-read it, rebuild
//! the snapshot from what was re-read. Re-reading is not caution for its own
//! sake — it is what makes the printed result the file's answer rather than
//! our own, and it catches a write that landed somewhere unexpected.

mod file;
mod floor;
mod known;
mod repo;
mod show;
mod snapshot;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use lessr_adapters::Paths;
use lessr_core::{Config, EnvOverrides, Layer, Level, Mode, Resolved, Value, ValueKind};

use crate::cli::ConfigAction;

/// Something wrong with the configuration that has to be said out loud and
/// must not be fatal.
///
/// `docs/CONFIG.md`: a value that does not parse falls back to its default and
/// is reported; a key nothing reads is reported too, because a typo that
/// silently does nothing is worse than a typo that says so. Neither may break
/// someone's agent, so nothing in this crate turns one of these into an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// Where it is: `stages.gate.mode`, or the name of an environment
    /// variable.
    pub at: String,
    /// What is wrong, and what happens instead.
    pub says: String,
}

impl Problem {
    /// One problem, at one place.
    pub fn new(at: impl Into<String>, says: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            says: says.into(),
        }
    }
}

/// `lessr on|off|shadow <stage> [--repo]`.
pub fn set_mode(stage: &str, mode: Mode, repo: bool) -> Result<ExitCode> {
    let mut session = Session::open(repo)?;
    session.set_axis(stage, Axis::Mode(mode))
}

/// `lessr level <stage> <level> [--repo]`.
pub fn set_level(stage: &str, level: &str, repo: bool) -> Result<ExitCode> {
    let Some(level) = Level::parse(level) else {
        // Named, not hinted at: a user who typed `medium` needs the three
        // words, not a pointer to the documentation.
        eprintln!("lessr: `{level}` is not a level. The three are: safe, balanced, aggressive.");
        return Ok(ExitCode::FAILURE);
    };

    let mut session = Session::open(repo)?;
    session.set_axis(stage, Axis::Level(level))
}

/// `lessr config [--repo] [--explain <stage>] [set <key> <value>]`.
pub fn config(action: Option<ConfigAction>, explain: Option<&str>, repo: bool) -> Result<ExitCode> {
    if action.is_some() && explain.is_some() {
        bail!("`--explain` asks a question and `set` answers a different one; run them separately");
    }

    let mut session = Session::open(repo)?;
    match action {
        Some(ConfigAction::Set { key, value }) => session.set(&key, &value),
        None => {
            match explain {
                Some(stage) => print!("{}", show::explain(&session, stage)),
                None => print!("{}", show::table(&session)),
            }
            // A file that does not parse at all is the one thing here that is
            // worth a non-zero exit: the layers being printed are not the
            // user's, the hook is running on the snapshot from before the
            // edit, and a script has no other way to be told. Values that
            // merely fall back are reported and exit 0 — they are what the
            // command is for.
            match session.unparseable {
                Some(_) => Ok(ExitCode::FAILURE),
                None => Ok(ExitCode::SUCCESS),
            }
        }
    }
}

/// Which axis a write is about. The two are handled together because
/// everything around them — the floor, the scope, the report — is identical.
#[derive(Clone, Copy, Debug)]
enum Axis {
    Mode(Mode),
    Level(Level),
}

impl Axis {
    /// The key in the file.
    fn key(self) -> &'static str {
        match self {
            Axis::Mode(_) => "mode",
            Axis::Level(_) => "level",
        }
    }

    /// The value, as the file spells it.
    fn text(self) -> &'static str {
        match self {
            Axis::Mode(mode) => mode.as_str(),
            Axis::Level(level) => level.as_str(),
        }
    }

    /// What this axis resolves to for a stage, so a write can check whether it
    /// actually decided anything.
    fn resolved(self, resolved: &Resolved) -> (String, Layer) {
        match self {
            Axis::Mode(_) => (resolved.mode().as_str().to_string(), resolved.mode_layer()),
            Axis::Level(_) => (
                resolved.level().as_str().to_string(),
                resolved.level_layer(),
            ),
        }
    }
}

/// One run of one config command: where the files are, what they say, and what
/// is wrong with them.
struct Session {
    /// Lessr's config directory.
    dir: PathBuf,
    /// `<dir>/config.toml`.
    file: PathBuf,
    /// `<dir>/config.bin`, the snapshot the hook maps.
    snapshot: PathBuf,
    /// The repository this command is scoped to, or `None` for global.
    scope: Option<PathBuf>,
    /// The repository the user is standing in, whatever the scope. A global
    /// change still has to say when it will not take effect here.
    here: Option<PathBuf>,
    /// Every layer, the floor above them and the environment on top.
    config: Config,
    /// `[proxy] port`, if the file sets a usable one.
    port: Option<u16>,
    /// Reported by `lessr config`; never fatal.
    problems: Vec<Problem>,
    /// Whether `config.toml` is there at all.
    exists: bool,
    /// Why the file could not be read, if it could not be.
    unparseable: Option<String>,
    /// What happened to the snapshot on this run.
    snapshot_state: snapshot::State,
}

impl Session {
    /// Find the files and read them.
    fn open(repo: bool) -> Result<Session> {
        let paths = Paths::detect().context("cannot work out where your config lives")?;
        let here = repo::here();

        // `--repo` with no repository is refused rather than quietly widened:
        // writing a global setting because we could not find a `.git` would
        // change every repository the user has, on their behalf, in silence.
        let scope = if repo {
            let root = here.clone().ok_or_else(|| {
                anyhow!(
                    "--repo needs a repository: there is no `.git` here or in any directory above. \
                     Run it inside your repository, or drop --repo to set this everywhere."
                )
            })?;
            Some(root)
        } else {
            None
        };

        let mut session = Session {
            file: file::path_in(&paths.config),
            snapshot: snapshot::path_in(&paths.config),
            dir: paths.config,
            scope,
            here,
            config: Config::new(),
            port: None,
            problems: Vec::new(),
            exists: false,
            unparseable: None,
            snapshot_state: snapshot::State::Fresh,
        };
        session.reload()?;
        Ok(session)
    }

    /// Read `config.toml`, the self-healing table and the environment, and
    /// bring the snapshot into line with the first two.
    ///
    /// Called again after every write, so what a command prints is what the
    /// file now says rather than what we meant to put in it.
    fn reload(&mut self) -> Result<()> {
        let loaded = file::load(&self.file);
        let (floor, floor_problems) = floor::load(&self.dir.join(floor::FILE_NAME));

        let mut config = loaded.config;
        for entry in floor.entries() {
            config
                .floor_mut()
                .force_off(&entry.stage, entry.repo.as_deref(), &entry.reason);
        }

        // The snapshot is compiled before the environment is applied, and
        // deliberately: a snapshot is written once and read by every later
        // hook, so an override from whichever shell ran `lessr config` would
        // outlive that shell (`docs/CONFIG.md`). The hook applies its own.
        self.snapshot_state = if loaded.unparseable.is_some() {
            // Compiling the defaults over a snapshot built from a file that
            // used to parse would answer a syntax error by silently
            // un-configuring every stage.
            snapshot::State::Held
        } else {
            snapshot::refresh(&self.snapshot, &config, loaded.exists)?
        };

        let env = EnvOverrides::from_env();
        let mut problems = loaded.problems;
        problems.extend(floor_problems);
        for rejected in env.rejected() {
            problems.push(Problem::new(
                &rejected.var,
                format!(
                    "`{}` is not a value lessr understands; the file layers still decide",
                    rejected.value
                ),
            ));
        }
        config.set_env(env);

        self.config = config;
        self.port = loaded.port;
        self.problems = problems;
        self.exists = loaded.exists;
        self.unparseable = loaded.unparseable;
        Ok(())
    }

    /// `lessr on|off|shadow|level`, and `config set` for the same two keys.
    fn set_axis(&mut self, stage: &str, axis: Axis) -> Result<ExitCode> {
        named(stage)?;

        // The floor is not a layer, so there is no file that would carry this
        // out; only `off` agrees with it and can be written.
        if let Axis::Mode(mode) = axis {
            let floored = match mode {
                Mode::Off => None,
                _ => self.config.floor().reason(stage, self.scope.as_deref()),
            };
            if let Some(reason) = floored {
                return Ok(self.refuse_floor(stage, mode, reason));
            }
        }

        let slot = file::Slot::stage(self.scope.as_deref(), stage, axis.key());
        let written = file::write(&self.file, &slot, axis.text().into())?;
        self.reload()?;
        self.report(stage, &slot, axis.text(), &written, Some(axis));
        Ok(ExitCode::SUCCESS)
    }

    /// `lessr config set <key> <value>`.
    fn set(&mut self, key: &str, value: &str) -> Result<ExitCode> {
        let parts: Vec<&str> = key.split('.').collect();
        match parts.as_slice() {
            ["stages", stage, "mode"] => {
                let Some(mode) = Mode::parse(value) else {
                    eprintln!(
                        "lessr: `{value}` is not a mode. The three are: off, shadow, active."
                    );
                    return Ok(ExitCode::FAILURE);
                };
                self.set_axis(stage, Axis::Mode(mode))
            }
            ["stages", stage, "level"] => {
                let Some(level) = Level::parse(value) else {
                    eprintln!(
                        "lessr: `{value}` is not a level. The three are: safe, balanced, aggressive."
                    );
                    return Ok(ExitCode::FAILURE);
                };
                self.set_axis(stage, Axis::Level(level))
            }
            ["stages", stage, setting] => self.set_setting(stage, setting, value),
            ["proxy", "port"] => self.set_port(value),
            ["repo", ..] => bail!(
                "a repository scope is spelled with `--repo`, from inside the repository, \
                 not in the key: `lessr config --repo set stages.gate.mode off`"
            ),
            _ => bail!(
                "`{key}` is not a key lessr sets. Keys look like `stages.gate.level`, \
                 `stages.default.mode` or `proxy.port`."
            ),
        }
    }

    /// One tunable of one stage.
    fn set_setting(&mut self, stage: &str, key: &str, text: &str) -> Result<ExitCode> {
        named(stage)?;
        if key.trim().is_empty() {
            bail!("a setting needs a name: `lessr config set stages.gate.max_repeated_lines 3`");
        }
        let slot = file::Slot::stage(self.scope.as_deref(), stage, key);
        let parsed = Value::from(text);

        let value: toml_edit::Value = match known::tunable(stage, key) {
            // Validated before anything is written, against the same coercion
            // rules the stage will read it through: `"5"` is a number there,
            // so it is a number here.
            Some(tunable) => match tunable.kind {
                ValueKind::Int => match parsed.as_u64().and_then(|n| i64::try_from(n).ok()) {
                    Some(number) => number.into(),
                    None => bail!("`{}` takes a whole number, not `{text}`", slot.dotted()),
                },
                ValueKind::Bool => match parsed.as_bool() {
                    Some(flag) => flag.into(),
                    None => bail!("`{}` takes true or false, not `{text}`", slot.dotted()),
                },
                ValueKind::Str => text.into(),
            },
            // Reported, not rejected: the Pro binary is this same CLI with
            // more stages registered, so a key this build cannot name may
            // still be one something reads. Refusing it would make the free
            // binary unable to configure the Pro one. A key under a mechanism
            // this build does have, though, is a typo worth spelling out —
            // and one under a mechanism it does not is covered by the note.
            None => {
                if known::stage(stage).is_some() {
                    eprintln!(
                        "lessr: nothing in this build reads `{}`. Writing it anyway; \
                         {stage} takes {}.",
                        slot.dotted(),
                        known::keys(stage).join(", ")
                    );
                }
                infer(text)
            }
        };

        let written = file::write(&self.file, &slot, value)?;
        self.reload()?;
        self.report(stage, &slot, text, &written, None);
        Ok(ExitCode::SUCCESS)
    }

    /// `[proxy] port`.
    fn set_port(&mut self, text: &str) -> Result<ExitCode> {
        let port = text
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| anyhow!("`{text}` is not a port; pick one between 1 and 65535"))?;

        let slot = file::Slot::proxy("port");
        let written = file::write(&self.file, &slot, i64::from(port).into())?;
        self.reload()?;
        println!("proxy port {port}{}", was(&written));
        self.report_files(written.created);
        Ok(ExitCode::SUCCESS)
    }

    /// Say what a write did, and what still outranks it.
    fn report(
        &self,
        stage: &str,
        slot: &file::Slot,
        value: &str,
        written: &file::Written,
        axis: Option<Axis>,
    ) {
        let scope = match &self.scope {
            Some(repo) => format!(" in {}", repo.display()),
            None => String::new(),
        };
        println!("{stage} {} = {value}{scope}{}", slot.key(), was(written));
        self.report_files(written.created);

        for note in self.notes(stage, axis) {
            println!("  note      {note}");
        }
    }

    /// The two paths every write touches.
    ///
    /// A file that had to be created is worth naming as such: it is the moment
    /// a user learns where their configuration lives.
    fn report_files(&self, created: bool) {
        let verb = if created { "created  " } else { "wrote    " };
        println!("  {verb} {}", self.file.display());
        println!(
            "  snapshot  {} — {}",
            self.snapshot.display(),
            self.snapshot_state.note()
        );
    }

    /// Everything true about a write that the write itself does not say.
    ///
    /// A command that reports success while a higher layer decides the value
    /// is a command that teaches people their config file does not work.
    fn notes(&self, stage: &str, axis: Option<Axis>) -> Vec<String> {
        let mut notes = Vec::new();

        if stage != known::DEFAULT && known::stage(stage).is_none() {
            notes.push(format!(
                "no mechanism named `{stage}` is registered in this build; \
                 the value is written and will apply if one is"
            ));
        }

        if let Some(axis) = axis {
            let (value, layer) = axis.resolved(&self.config.resolve(stage, self.scope.as_deref()));
            if value != axis.text() {
                notes.push(format!(
                    "{} is {value} here all the same: {} sits above the layer you just wrote",
                    axis.key(),
                    layer.as_str()
                ));
            }

            // A global change that a repository layer overrides is the most
            // common way a config command looks like it did nothing.
            let here = self.here.as_ref().filter(|_| self.scope.is_none());
            if let Some(here) = here {
                let (local, layer) = axis.resolved(&self.config.resolve(stage, Some(here)));
                if local != value {
                    notes.push(format!(
                        "in {} it stays {local}, set by {}",
                        here.display(),
                        layer.as_str()
                    ));
                }
            }
        }

        match self.problems.len() {
            0 => {}
            1 => notes.push(
                "one other thing in your configuration needs attention; `lessr config` says what"
                    .to_string(),
            ),
            count => notes.push(format!(
                "{count} other things in your configuration need attention; `lessr config` says what"
            )),
        }

        notes
    }

    /// Refuse to promote a stage the self-healing table has turned off.
    ///
    /// Loop-safety rule 5 put it there and `docs/CONFIG.md` puts the floor
    /// above every layer, so there is no file we could write that would carry
    /// this out. Writing one anyway and reporting success would leave the user
    /// believing a mechanism is running that is not.
    fn refuse_floor(&self, stage: &str, mode: Mode, reason: &str) -> ExitCode {
        let where_ = match &self.scope {
            Some(repo) => format!(" in {}", repo.display()),
            None => String::new(),
        };
        eprintln!("lessr: {stage} is off{where_} and configuration cannot turn it back on.");
        eprintln!("       Lessr turned it off itself: {reason}.");
        eprintln!(
            "       That is the safety floor (loop-safety rule 5). It sits above every config"
        );
        eprintln!(
            "       layer, so `{}` has nothing to write. It lifts when the evidence does.",
            match mode {
                Mode::Active => format!("lessr on {stage}"),
                _ => format!("lessr {} {stage}", mode.as_str()),
            }
        );
        eprintln!("       `lessr config --explain {stage}` shows the entry. Nothing was written.");
        ExitCode::FAILURE
    }
}

/// `(was shadow)`, when there was something there before.
fn was(written: &file::Written) -> String {
    match &written.before {
        Some(before) => format!("  (was {before})"),
        None => String::new(),
    }
}

/// Refuse a mechanism name the file cannot hold.
///
/// An empty one is the case that matters: `[stages.""]` is what the snapshot
/// uses for the nameless `[stages.default]` record, so writing one would put
/// two different things under the same name on disk.
fn named(stage: &str) -> Result<()> {
    if stage.trim().is_empty() {
        bail!("a mechanism needs a name: `lessr off gate`, or `stages.gate.mode`");
    }
    Ok(())
}

/// The first line of a parser's complaint.
///
/// `toml_edit` renders an error as a snippet with carets under it, which is
/// the right thing in a compiler and the wrong thing in a one-line report.
fn first_line(err: &toml_edit::TomlError) -> String {
    err.message().lines().next().unwrap_or("").to_string()
}

/// The TOML type for a value nothing in this build can type-check.
///
/// Deliberately narrow. `true` and `false` are booleans and a whole number is
/// a number; `on` and `off` stay strings, because for a key we cannot name we
/// cannot know whether the user meant a flag or the word.
fn infer(text: &str) -> toml_edit::Value {
    if let Ok(number) = text.trim().parse::<i64>() {
        return number.into();
    }
    match text.trim() {
        "true" => true.into(),
        "false" => false.into(),
        _ => text.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nameless_mechanism_is_refused_before_anything_is_written() {
        assert!(named("").is_err());
        assert!(named("  ").is_err());
        assert!(named("gate").is_ok());
    }

    #[test]
    fn inference_only_claims_what_is_unambiguous() {
        // Rendered, because that is what lands in the file: a quoted `"off"`
        // is a string, and an unquoted `400` is a number.
        assert_eq!(infer("400").to_string(), "400");
        assert_eq!(infer("true").to_string(), "true");
        assert_eq!(infer("off").to_string(), "\"off\"");
        assert_eq!(infer("tail").to_string(), "\"tail\"");
    }
}
