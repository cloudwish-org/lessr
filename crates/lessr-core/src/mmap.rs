//! Memory-mapping the config snapshot.
//!
//! One of the two modules where `unsafe` is allowed (`docs/CODE_STYLE.md`); the
//! other is `lessr-packs::mmap`. It is here because the hook path spawns a
//! process per tool call and has 2 ms to be useful in: mapping the snapshot
//! costs one `open` and one `mmap`, and reading it costs neither a parse nor an
//! allocation.
//!
//! ```no_run
//! # use std::path::Path;
//! # fn main() -> lessr_core::Result<()> {
//! use lessr_core::{EnvOverrides, mmap::MappedSnapshot};
//!
//! let snapshot = MappedSnapshot::open(Path::new("/home/dev/.config/lessr/config.bin"))?;
//! let env = EnvOverrides::from_env();
//! let gate = snapshot.view().stage_in(Path::new("/home/dev/work"), "gate").with_env("gate", &env);
//! println!("{} at {}", gate.mode().as_str(), gate.level().as_str());
//! # Ok(())
//! # }
//! ```

use std::path::Path;

use memmap2::Mmap;

use crate::error::{Error, Result};
use crate::snapshot::SnapshotView;

/// A snapshot file mapped into memory.
///
/// Hold it for as long as the views taken from it: every string a view hands
/// back borrows from the mapping.
#[derive(Debug)]
pub struct MappedSnapshot {
    /// `None` for an empty file. A zero-length mapping is not portable, and an
    /// empty snapshot means the same thing as a missing one: the defaults.
    map: Option<Mmap>,
}

impl MappedSnapshot {
    /// Map a snapshot file.
    ///
    /// Fails only if the file cannot be opened or mapped. A file that is
    /// mapped but is not a snapshot this build understands is not an error
    /// here: [`MappedSnapshot::view`] refuses it and answers with the defaults.
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).map_err(Error::Snapshot)?;
        if file.metadata().map_err(Error::Snapshot)?.len() == 0 {
            return Ok(Self { map: None });
        }

        // SAFETY: mapping a file is unsafe because another process may write to
        // it or truncate it while it is mapped, which would tear the bytes we
        // read or fault on access. Two things answer that here. `Snapshot::write`
        // installs a new snapshot by writing a temporary file and renaming it,
        // so a regeneration replaces the directory entry and leaves this
        // mapping on the old inode, whole, until it is dropped — there is no
        // in-place truncation to fault on. And the bytes are treated as
        // untrusted whatever their provenance: `SnapshotView` checks the magic,
        // the version, the length and a digest of the body before it believes
        // any of them, and every field it then reads is bounds-checked against
        // the slice, so a file some other process corrupted is refused and the
        // caller falls back to the defaults rather than misreading it.
        let map = unsafe { Mmap::map(&file) }.map_err(Error::Snapshot)?;
        Ok(Self { map: Some(map) })
    }

    /// The mapped bytes.
    pub fn bytes(&self) -> &[u8] {
        self.map.as_deref().unwrap_or(&[])
    }

    /// A view of the mapping, or of nothing if these are not bytes this build
    /// understands.
    ///
    /// This is what the hook path calls. Falling back to the defaults means
    /// [`crate::Mode::Shadow`] everywhere, so an unreadable snapshot leaves a
    /// Lessr that counts and changes nothing.
    pub fn view(&self) -> SnapshotView<'_> {
        SnapshotView::read_or_defaults(self.bytes())
    }

    /// A view of the mapping, with the reason it is not one if it is not.
    /// `lessr config` uses this to say what is wrong with the file.
    pub fn parse(&self) -> Result<SnapshotView<'_>> {
        SnapshotView::parse(self.bytes())
    }

    /// How many bytes are mapped.
    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    /// Whether nothing is mapped.
    pub fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, StageOverride};
    use crate::snapshot::Snapshot;
    use crate::types::{Level, Mode};

    fn config() -> Config {
        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new()
            .with_mode(Mode::Active)
            .with_level(Level::Aggressive)
            .with("summary_after_lines", 400u64);
        config
    }

    #[test]
    fn a_mapped_snapshot_answers_what_the_bytes_say() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.bin");
        Snapshot::write(&path, &config()).unwrap();

        let mapped = MappedSnapshot::open(&path).unwrap();
        assert_eq!(mapped.len(), Snapshot::encode(&config()).len());

        let gate = mapped.view().stage("gate");
        assert_eq!(gate.mode(), Mode::Active);
        assert_eq!(gate.level(), Level::Aggressive);
        assert_eq!(gate.u64("summary_after_lines", 0), 400);
    }

    #[test]
    fn a_file_that_is_not_a_snapshot_reads_as_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.bin");
        std::fs::write(&path, b"# mode = \"active\"\n").unwrap();

        let mapped = MappedSnapshot::open(&path).unwrap();
        assert!(mapped.parse().is_err(), "and it says why");
        assert_eq!(mapped.view().stage("gate").mode(), Mode::Shadow);
    }

    #[test]
    fn an_empty_file_maps_to_nothing_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.bin");
        std::fs::write(&path, b"").unwrap();

        let mapped = MappedSnapshot::open(&path).unwrap();
        assert!(mapped.is_empty());
        assert_eq!(mapped.view().stage("gate").mode(), Mode::Shadow);
    }

    #[test]
    fn a_missing_file_is_an_error_the_caller_can_shrug_off() {
        let dir = tempfile::tempdir().unwrap();
        let err = MappedSnapshot::open(&dir.path().join("nothing.bin")).unwrap_err();
        assert!(matches!(err, Error::Snapshot(_)));
    }

    #[test]
    fn regenerating_a_snapshot_leaves_an_open_mapping_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.bin");
        Snapshot::write(&path, &config()).unwrap();
        let mapped = MappedSnapshot::open(&path).unwrap();

        // What `lessr config set` does while a hook is running.
        let mut changed = Config::new();
        *changed.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Off);
        Snapshot::write(&path, &changed).unwrap();

        assert_eq!(
            mapped.view().stage("gate").mode(),
            Mode::Active,
            "the mapping holds the snapshot it opened, whole"
        );
        assert_eq!(
            MappedSnapshot::open(&path)
                .unwrap()
                .view()
                .stage("gate")
                .mode(),
            Mode::Off,
            "and the next hook sees the new one"
        );
    }
}
