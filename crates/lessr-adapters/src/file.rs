//! Reading and writing the small config files this crate touches.
//!
//! Every function here turns an I/O or parse failure into a [`crate::Error`]
//! carrying the path. A config we cannot read is a config we refuse to
//! rewrite: guessing at the contents is how an agent ends up with a settings
//! file that no longer parses.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::{Error, Result};

/// Read a file that may not exist.
///
/// `Ok(None)` means "not there", which for an agent config is a normal state:
/// `lessr init` then creates it. Any other failure — a directory, no
/// permission, invalid UTF-8 — is an error, because it means something is
/// there and we could not see it.
pub(crate) fn read(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Parse a config file's JSON, preserving key order.
///
/// An empty or whitespace-only file parses as `{}`: agents create a settings
/// file before they have anything to put in it, and refusing to patch a
/// zero-byte file would be refusing for no reason.
pub(crate) fn parse(path: &Path, text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(text).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })
}

/// Render JSON the way an agent's own settings file is written: two-space
/// indent, trailing newline. `serde_json`'s pretty printer is two-space by
/// default; the newline is ours, so the file ends the way every other line
/// does and `git diff` stays quiet about it.
pub(crate) fn pretty(value: &Value) -> String {
    let mut out = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    out.push('\n');
    out
}

/// Write a file, creating its directory if the agent has not been run yet.
pub(crate) fn write(path: &Path, contents: &str) -> Result<()> {
    create_parent(path)?;
    fs::write(path, contents).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Copy `from` to `to` byte for byte, creating `to`'s directory.
///
/// Backups and restores both go through this, which is what makes
/// `lessr uninstall` exact: nothing parses or re-serialises the file on the
/// way in or out, so whitespace, key order and the user's own comments-shaped
/// oddities survive.
pub(crate) fn copy(from: &Path, to: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(from).map_err(|source| Error::Io {
        path: from.to_path_buf(),
        source,
    })?;
    create_parent(to)?;
    fs::write(to, &bytes).map_err(|source| Error::Io {
        path: to.to_path_buf(),
        source,
    })?;
    Ok(bytes)
}

/// Delete a file that may already be gone.
///
/// A missing file is a success: uninstall is meant to be runnable twice.
pub(crate) fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// `mkdir -p` for a file's directory.
fn create_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        path: parent.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read(&tmp.path().join("nope.json")).unwrap().is_none());
    }

    #[test]
    fn an_empty_file_parses_as_an_empty_object() {
        let value = parse(Path::new("settings.json"), "  \n ").unwrap();
        assert_eq!(value, serde_json::json!({}));
    }

    #[test]
    fn malformed_json_reports_the_path() {
        let err = parse(Path::new("/x/settings.json"), "{oops").unwrap_err();
        assert!(err.to_string().contains("/x/settings.json"), "{err}");
    }

    #[test]
    fn pretty_is_two_space_indented_and_ends_in_a_newline() {
        let text = pretty(&serde_json::json!({"a": {"b": 1}}));
        assert_eq!(text, "{\n  \"a\": {\n    \"b\": 1\n  }\n}\n");
    }

    #[test]
    fn copy_creates_the_destination_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from.json");
        fs::write(&from, b"{\n\t\"a\": 1}\n").unwrap();
        let to = tmp.path().join("deep/er/still/to.json");
        let bytes = copy(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), bytes);
        assert_eq!(fs::read(&from).unwrap(), bytes);
    }
}
