//! Keeping `config.bin` in step with `config.toml`.
//!
//! The hook path runs on every tool call and owes the agent 2 ms, so it parses
//! no TOML: it memory-maps a compiled snapshot instead (technique 7 in
//! `docs/PERFORMANCE.md`, and the Snapshot section of `docs/CONFIG.md`). That
//! only works if the snapshot is never behind the file a human edits, so this
//! module has one job: make the two agree, on every command that reads or
//! writes the configuration.
//!
//! It compares bytes, not timestamps. Encoding a configuration is
//! deterministic and costs microseconds on a file this size, so
//! `encode(config) == what is on disk` answers the question exactly, and
//! answers it for the cases a timestamp cannot: a snapshot copied from another
//! machine, a clock that went backwards, an edit made inside the same second
//! as the last write, or a `config.toml` that was deleted and left its
//! snapshot behind still configuring the hook.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use lessr_core::{Config, Snapshot, SnapshotView};

/// The compiled snapshot, beside `config.toml` in the config directory.
///
/// Named and placed so that one directory holds the pair: the file a human
/// edits and the file the hook maps. `lessr-core`'s own example spells this
/// path (`crates/lessr-core/src/mmap.rs`), and the hook has to be able to find
/// it without being told.
pub const FILE_NAME: &str = "config.bin";

/// What happened to the snapshot on this run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// It already said exactly what the configuration says.
    Fresh,
    /// It did not, and was rebuilt. The string says what was wrong with it,
    /// because a snapshot that silently regenerates is a snapshot nobody
    /// notices was stale.
    Rebuilt(&'static str),
    /// There is nothing to compile and nothing stale to correct: no config
    /// file, no floor, no snapshot on disk.
    Nothing,
}

impl State {
    /// The line `lessr config` prints beside the path.
    pub fn note(&self) -> &'static str {
        match self {
            State::Fresh => "up to date",
            State::Rebuilt(why) => why,
            State::Nothing => "not written: nothing to compile yet",
        }
    }
}

/// Make the snapshot agree with `config`, rebuilding it if it does not.
///
/// `source_exists` is whether `config.toml` is there: with no file, no floor
/// and no snapshot, there is nothing to write and a user who has never
/// configured anything gets no files they did not ask for. A snapshot that
/// *does* exist is always brought into line, including down to "the defaults",
/// which is what a deleted `config.toml` means.
pub fn refresh(path: &Path, config: &Config, source_exists: bool) -> Result<State> {
    let wanted = Snapshot::encode(config);

    let why = match std::fs::read(path) {
        // Byte-identical to what this build would write, which is a stronger
        // statement than "the header parses": every layer, every scope and
        // every floor reason already agrees.
        Ok(bytes) if bytes == wanted => return Ok(State::Fresh),
        Ok(bytes) => match SnapshotView::parse(&bytes) {
            Ok(_) => "rebuilt: the config file changed",
            // Wrong magic, a version from another Lessr, a half-copied file:
            // the hook refuses it and falls back to the compiled-in defaults,
            // so leaving it would quietly un-configure every stage.
            Err(_) => "rebuilt: the old one was not a snapshot this build understands",
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if !source_exists && config == &Config::new() {
                return Ok(State::Nothing);
            }
            "written: there was none"
        }
        Err(err) => {
            return Err(
                anyhow::Error::from(err).context(format!("cannot read {}", path.display()))
            );
        }
    };

    Snapshot::write(path, config)
        .with_context(|| format!("cannot write the config snapshot {}", path.display()))?;
    Ok(State::Rebuilt(why))
}

/// Where the snapshot lives, given the config directory.
pub fn path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lessr_core::{Mode, StageOverride};

    fn configured() -> Config {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Active);
        config
    }

    #[test]
    fn nothing_to_compile_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        assert_eq!(refresh(&path, &Config::new(), false).unwrap(), State::Nothing);
        assert!(!path.exists(), "a user who configured nothing gets no files");
    }

    #[test]
    fn a_missing_snapshot_is_written_and_then_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());

        assert!(matches!(
            refresh(&path, &configured(), true).unwrap(),
            State::Rebuilt(_)
        ));
        assert_eq!(
            refresh(&path, &configured(), true).unwrap(),
            State::Fresh,
            "a second run must not rewrite an identical file"
        );
    }

    #[test]
    fn a_changed_configuration_rebuilds_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        refresh(&path, &configured(), true).unwrap();

        let mut changed = configured();
        *changed.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Off);
        assert!(matches!(
            refresh(&path, &changed, true).unwrap(),
            State::Rebuilt(_)
        ));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            SnapshotView::parse(&bytes).unwrap().stage("gate").mode(),
            Mode::Off,
            "the hook must see the edit, not the bytes before it"
        );
    }

    #[test]
    fn bytes_that_are_not_a_snapshot_are_replaced_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        std::fs::write(&path, b"mode = \"active\"\n").unwrap();

        let state = refresh(&path, &configured(), true).unwrap();
        assert_eq!(
            state,
            State::Rebuilt("rebuilt: the old one was not a snapshot this build understands")
        );
        assert!(SnapshotView::parse(&std::fs::read(&path).unwrap()).is_ok());
    }

    #[test]
    fn a_deleted_config_file_does_not_leave_its_snapshot_in_charge() {
        // The stale-snapshot bug in its worst shape: the file is gone, the
        // user believes Lessr is back to its defaults, and the hook goes on
        // reading yesterday's settings.
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        refresh(&path, &configured(), true).unwrap();

        assert!(matches!(
            refresh(&path, &Config::new(), false).unwrap(),
            State::Rebuilt(_)
        ));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            SnapshotView::parse(&bytes).unwrap().stage("gate").mode(),
            Mode::Shadow,
            "the compiled-in default, which is what an absent config means"
        );
    }
}
