//! Backups, and the index `lessr uninstall` restores from.
//!
//! Layout, per `docs/ADAPTERS.md` ("back up before patching; `lessr uninstall`
//! restores byte-for-byte"):
//!
//! ```text
//! <config>/backups/index.json
//! <config>/backups/<agent>/<filename>.<unix-timestamp>.bak
//! ```
//!
//! The copy is the truth and the index is the map to it. The index records the
//! blake3 of what was copied so a restore can say whether the file it is about
//! to put back is the file it took.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::AgentId;
use crate::error::{Error, Result};
use crate::file;
use crate::paths::Paths;

/// The index file name, under `<config>/backups/`.
const INDEX: &str = "index.json";

/// How many same-second backups of one file we are willing to number before
/// giving up and reusing the name. Two inits in the same second is already a
/// script; a thousand is a loop, and a loop should not fill a disk.
const MAX_SAME_SECOND: u32 = 1000;

/// One line of the backup index.
///
/// Serialised with the field names `docs/ADAPTERS.md` names, so the file is
/// readable by a human with `cat` when a restore goes wrong.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Record {
    /// [`AgentId::as_str`] of the agent whose config this was.
    pub(crate) agent: String,
    /// Where the file came from, and where a restore puts it back.
    pub(crate) original_path: PathBuf,
    /// The copy, under `<config>/backups/<agent>/`.
    pub(crate) stored_as: PathBuf,
    /// Unix seconds. The newest record for an agent is the one
    /// [`crate::plan_uninstall`] restores.
    pub(crate) taken_at: u64,
    /// blake3 of the bytes we copied, hex.
    pub(crate) blake3: String,
}

/// The index path for a set of [`Paths`].
pub(crate) fn index_path(paths: &Paths) -> PathBuf {
    paths.backups_dir().join(INDEX)
}

/// The index path implied by a backup destination.
///
/// [`crate::apply`] only has the `to` of a [`crate::Change::Backup`] to work
/// from, and the layout above puts the index one level above the per-agent
/// directory. A path not in that shape — a hand-built plan — gets its index
/// beside the copy, so the record is still written somewhere findable.
pub(crate) fn index_beside(stored_as: &Path) -> PathBuf {
    stored_as
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from(INDEX), |dir| dir.join(INDEX))
}

/// Where a backup of `original` taken now should go.
///
/// The timestamp is in the name so the directory reads as a history without
/// opening the index. If that name is taken — two inits inside one second — a
/// counter is appended rather than overwriting the older copy, which might be
/// the only pristine one left.
pub(crate) fn destination(paths: &Paths, agent: AgentId, original: &Path) -> PathBuf {
    let dir = paths.backups_dir().join(agent.as_str());
    let name = original.file_name().map_or_else(
        || String::from("config"),
        |n| n.to_string_lossy().into_owned(),
    );
    let stamp = now();

    let first = dir.join(format!("{name}.{stamp}.bak"));
    if !first.exists() {
        return first;
    }
    for n in 1..MAX_SAME_SECOND {
        let candidate = dir.join(format!("{name}.{stamp}-{n}.bak"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// Copy `from` to `to` and append the record that makes it restorable.
pub(crate) fn take(agent: AgentId, from: &Path, to: &Path) -> Result<()> {
    let bytes = file::copy(from, to)?;
    let record = Record {
        agent: agent.as_str().to_string(),
        original_path: from.to_path_buf(),
        stored_as: to.to_path_buf(),
        taken_at: stamp_in(to).unwrap_or_else(now),
        blake3: blake3::hash(&bytes).to_hex().to_string(),
    };
    append(&index_beside(to), record)
}

/// Every record, oldest first. A missing index is an empty history; a
/// *corrupt* index is an error, because rewriting it would throw away the only
/// record of where a user's real config went.
pub(crate) fn read_index(path: &Path) -> Result<Vec<Record>> {
    let Some(text) = file::read(path)? else {
        return Ok(Vec::new());
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&text).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })
}

/// The newest backup for `agent` whose copy is still on disk.
///
/// Later records win ties, so a same-second pair restores in the order it was
/// written. A record whose copy has been deleted is skipped rather than
/// reported: the user tidying `~/.config/lessr/backups` should not make
/// `lessr uninstall` fail, it should make it fall back.
pub(crate) fn newest_for(records: &[Record], agent: AgentId) -> Option<&Record> {
    records
        .iter()
        .filter(|record| record.agent == agent.as_str() && record.stored_as.exists())
        // `max_by_key` keeps the last of equal keys, which is the later write.
        .max_by_key(|record| record.taken_at)
}

/// Append one record, keeping the file readable.
fn append(path: &Path, record: Record) -> Result<()> {
    let mut records = read_index(path)?;
    records.push(record);
    let value = serde_json::to_value(&records).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })?;
    file::write(path, &file::pretty(&value))
}

/// Unix seconds, or 0 on a machine whose clock is before 1970. The timestamp
/// orders backups; it is not worth a panic (`docs/ADAPTERS.md`: never break
/// the agent).
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Read the timestamp back out of `<name>.<stamp>.bak`, so the record agrees
/// with the file name even if planning and applying straddle a second.
fn stamp_in(stored_as: &Path) -> Option<u64> {
    let name = stored_as.file_name()?.to_str()?;
    let stem = name.strip_suffix(".bak")?;
    let stamp = stem.rsplit('.').next()?;
    let digits = stamp.split('-').next()?;
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(tmp: &Path) -> Paths {
        Paths::with_roots(tmp.join("home"), tmp.join("config"))
    }

    #[test]
    fn a_backup_lands_under_its_agent_and_carries_a_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        let original = paths.home.join(".claude/settings.json");
        let to = destination(&paths, AgentId::ClaudeCode, &original);

        assert!(to.starts_with(paths.backups_dir().join("claude")));
        let name = to.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("settings.json."), "{name}");
        assert!(name.ends_with(".bak"), "{name}");
        assert!(stamp_in(&to).is_some(), "{name}");
    }

    #[test]
    fn a_second_backup_in_the_same_second_does_not_overwrite_the_first() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        let original = paths.home.join(".claude/settings.json");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, "{}\n").unwrap();

        let first = destination(&paths, AgentId::ClaudeCode, &original);
        take(AgentId::ClaudeCode, &original, &first).unwrap();
        let second = destination(&paths, AgentId::ClaudeCode, &original);
        assert_ne!(first, second);
    }

    #[test]
    fn take_records_what_it_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        let original = paths.home.join(".claude/settings.json");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, "{\"model\": \"opus\"}\n").unwrap();

        let to = destination(&paths, AgentId::ClaudeCode, &original);
        take(AgentId::ClaudeCode, &original, &to).unwrap();

        let records = read_index(&index_path(&paths)).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.agent, "claude");
        assert_eq!(record.original_path, original);
        assert_eq!(record.stored_as, to);
        assert_eq!(
            record.blake3,
            blake3::hash(b"{\"model\": \"opus\"}\n")
                .to_hex()
                .to_string()
        );
        assert_eq!(newest_for(&records, AgentId::ClaudeCode), Some(record));
        assert_eq!(newest_for(&records, AgentId::Cursor), None);
    }

    #[test]
    fn a_missing_index_is_an_empty_history_and_a_corrupt_one_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("index.json");
        assert!(read_index(&path).unwrap().is_empty());

        std::fs::write(&path, "not json").unwrap();
        assert!(read_index(&path).is_err());
    }

    #[test]
    fn a_backup_whose_copy_is_gone_is_skipped() {
        let records = vec![Record {
            agent: "claude".to_string(),
            original_path: PathBuf::from("/home/dev/.claude/settings.json"),
            stored_as: PathBuf::from("/nowhere/settings.json.1.bak"),
            taken_at: 1,
            blake3: String::new(),
        }];
        assert_eq!(newest_for(&records, AgentId::ClaudeCode), None);
    }
}
