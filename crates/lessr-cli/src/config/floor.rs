//! The self-healing table, as everything but the recorder sees it.
//!
//! Loop-safety rule 5: when handles from one filter are expanded on more than
//! 10 % of its outputs in a repository, that filter goes to `Off` there and the
//! receipt says so. `docs/CONFIG.md` puts the result above every configuration
//! layer — a floor, not a layer — because configuration tunes a mechanism and
//! does not overrule the evidence that the mechanism is hurting this repo.
//!
//! The measurement ships with the recorder. The file it writes is read here
//! already, because the floor has to hold before the evidence exists: a CLI
//! that could not see a floored stage would be a CLI that silently accepts
//! `lessr on <stage>` and leaves the user believing they turned something on.
//!
//! `<config dir>/healing.toml`, written by Lessr and edited by nobody:
//!
//! ```toml
//! # Written by the self-healing table (loop-safety 5). Not edited by hand.
//! [[off]]
//! stage = "gate"
//! repo = "/home/dev/work/monorepo"   # omitted means everywhere
//! reason = "handles expanded on 14 % of its outputs"
//! ```
//!
//! A file that cannot be read leaves the floor empty and says so. That is the
//! one direction this failure may take: a floor we cannot read must not invent
//! entries, and a stage it would have held down is one `lessr config` reports
//! on rather than one the CLI pretends to know about.

use std::path::{Path, PathBuf};

use lessr_core::SafetyFloor;
use toml_edit::{DocumentMut, Item};

use super::Problem;

/// The file the self-healing table lives in, inside the config directory.
pub const FILE_NAME: &str = "healing.toml";

/// Read the table. Never fails: an unreadable table is an empty one, reported.
pub fn load(path: &Path) -> (SafetyFloor, Vec<Problem>) {
    let mut floor = SafetyFloor::new();
    let mut problems = Vec::new();

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (floor, problems),
        Err(err) => {
            problems.push(Problem::new(FILE_NAME, format!("cannot be read: {err}")));
            return (floor, problems);
        }
    };

    let document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(err) => {
            problems.push(Problem::new(
                FILE_NAME,
                format!("is not TOML ({}); no stage is held down by it", first_line(&err)),
            ));
            return (floor, problems);
        }
    };

    let Some(entries) = document.get("off").and_then(Item::as_array_of_tables) else {
        if document.get("off").is_some() {
            problems.push(Problem::new(
                FILE_NAME,
                "`off` is not a list of entries; no stage is held down by it",
            ));
        }
        return (floor, problems);
    };

    for (index, entry) in entries.iter().enumerate() {
        let Some(stage) = entry.get("stage").and_then(Item::as_str) else {
            problems.push(Problem::new(
                format!("{FILE_NAME} off[{index}]"),
                "has no `stage`; ignored",
            ));
            continue;
        };
        // No reason is not an option: loop-safety 5 requires the reason be
        // reported, and a stage held down by nothing a user can read is
        // indistinguishable from a bug in Lessr.
        let reason = entry
            .get("reason")
            .and_then(Item::as_str)
            .unwrap_or("the self-healing table gave no reason");
        let repo = entry.get("repo").and_then(Item::as_str).map(PathBuf::from);
        floor.force_off(stage, repo.as_deref(), reason);
    }

    (floor, problems)
}

/// The first line of a parser's complaint.
///
/// `toml_edit` renders an error as a snippet with carets under it, which is
/// the right thing in a compiler and the wrong thing inside a one-line report.
fn first_line(err: &toml_edit::TomlError) -> String {
    err.message().lines().next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn an_absent_table_holds_nothing_down() {
        let dir = tempfile::tempdir().unwrap();
        let (floor, problems) = load(&dir.path().join(FILE_NAME));
        assert!(floor.is_empty());
        assert!(problems.is_empty(), "absence is not a problem to report");
    }

    #[test]
    fn an_entry_is_scoped_to_its_repository_and_carries_its_reason() {
        let (_dir, path) = write(
            r#"
[[off]]
stage = "gate"
repo = "/home/dev/work/monorepo"
reason = "handles expanded on 14 % of its outputs"
"#,
        );
        let (floor, problems) = load(&path);
        assert!(problems.is_empty());
        assert_eq!(
            floor.reason("gate", Some(Path::new("/home/dev/work/monorepo/crates"))),
            Some("handles expanded on 14 % of its outputs"),
            "a floor holds everywhere under the repository it names"
        );
        assert_eq!(floor.reason("gate", Some(Path::new("/home/dev/other"))), None);
        assert_eq!(floor.reason("gate", None), None, "it is not global");
    }

    #[test]
    fn an_entry_with_no_repo_holds_everywhere() {
        let (_dir, path) = write("[[off]]\nstage = \"trap\"\nreason = \"expanded\"\n");
        let (floor, _) = load(&path);
        assert_eq!(floor.reason("trap", None), Some("expanded"));
        assert_eq!(floor.reason("trap", Some(Path::new("/anywhere"))), Some("expanded"));
    }

    #[test]
    fn a_broken_table_holds_nothing_down_and_says_so() {
        // Never the other way round. Inventing an entry from bytes we could
        // not read would turn a corrupt file into a stage nobody can switch
        // back on.
        let (_dir, path) = write("[[off]\nstage = \"gate\"\n");
        let (floor, problems) = load(&path);
        assert!(floor.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].says.contains("no stage is held down"));
    }

    #[test]
    fn an_entry_without_a_stage_is_skipped_and_reported() {
        let (_dir, path) = write("[[off]]\nreason = \"expanded\"\n\n[[off]]\nstage = \"gate\"\n");
        let (floor, problems) = load(&path);
        assert_eq!(problems.len(), 1);
        assert_eq!(
            floor.reason("gate", None),
            Some("the self-healing table gave no reason"),
            "a reason is required, so its absence is visible rather than empty"
        );
    }
}
