//! The matcher-group hook shape, shared by the agents that use it.
//!
//! Claude Code and Gemini CLI both keep hooks as
//!
//! ```json
//! {"hooks": {"<Event>": [{"matcher": "<regex>", "hooks": [<entry>, …]}]}}
//! ```
//!
//! so the patching lives here once and each adapter supplies its event name,
//! its matcher and the entry it wants added. What differs between them is the
//! entry's own fields, which is exactly the part each adapter had to verify for
//! itself.

use std::path::Path;

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// The key holding a hook list, both at the top level and inside a group.
const HOOKS: &str = "hooks";

/// The key holding a group's command.
const COMMAND: &str = "command";

/// Our own name, as it appears in a hook command.
pub(crate) const LESSR: &str = "lessr";

/// The other tool that takes this slot, and its older form: `docs/ADAPTERS.md`
/// says detect an existing `rtk` hook and chain behind it. rtk shipped as a
/// shell script before it shipped as a subcommand, and the script's path is
/// all that is left of it in a hook entry.
pub(crate) const RTK: &[&str] = &["rtk", "rtk-rewrite.sh"];

/// An empty group list, for reading a config that has none.
const NO_GROUPS: &[Value] = &[];

/// One agent's matcher-group layout.
pub(crate) struct Layout {
    /// The event key, e.g. `PreToolUse`.
    pub(crate) event: &'static str,
    /// The matcher for the group we add, when we have to add one.
    pub(crate) matcher: &'static str,
}

/// What patching did, which is also what the user is told.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Patch {
    /// A Lessr hook was already there. Nothing was changed.
    AlreadyThere,
    /// Added behind an existing `rtk` hook, in rtk's own group.
    ChainedBehindRtk,
    /// Added as a new matcher group.
    Added,
}

impl Layout {
    /// The groups for this event, for reading.
    pub(crate) fn groups<'a>(&self, root: &'a Value) -> &'a [Value] {
        root.get(HOOKS)
            .and_then(|hooks| hooks.get(self.event))
            .and_then(Value::as_array)
            .map_or(NO_GROUPS, Vec::as_slice)
    }

    /// Whether a Lessr hook, and whether an rtk hook, is already installed.
    pub(crate) fn scan(&self, root: &Value) -> (bool, bool) {
        let mut lessr = false;
        let mut rtk = false;
        for group in self.groups(root) {
            lessr |= group_runs(group, &[LESSR]);
            rtk |= group_runs(group, RTK);
        }
        (lessr, rtk)
    }

    /// Add `entry`, in place.
    ///
    /// rtk's own entry is never rewritten, only followed: rtk recognises its
    /// hook by shell-splitting the command and matching three exact tokens, so
    /// a wrapped or compound command reads to rtk as missing and its next
    /// `init` adds a second copy of itself. Appending a separate entry to the
    /// same list leaves rtk's bytes alone and still runs rtk first.
    pub(crate) fn patch(&self, path: &Path, root: &mut Value, entry: Value) -> Result<Patch> {
        let groups = self.groups_mut(path, root)?;
        if groups.iter().any(|group| group_runs(group, &[LESSR])) {
            return Ok(Patch::AlreadyThere);
        }

        if let Some(list) = groups
            .iter_mut()
            .find(|group| group_runs(group, RTK))
            .and_then(|group| group.get_mut(HOOKS))
            .and_then(Value::as_array_mut)
        {
            list.push(entry);
            return Ok(Patch::ChainedBehindRtk);
        }

        let mut group = Map::new();
        group.insert(
            "matcher".to_string(),
            Value::String(self.matcher.to_string()),
        );
        group.insert(HOOKS.to_string(), Value::Array(vec![entry]));
        groups.push(Value::Object(group));
        Ok(Patch::Added)
    }

    /// Take our entry back out, pruning whatever it leaves empty behind it.
    ///
    /// Returns whether anything was removed. Other tools' groups, and an rtk
    /// entry sharing our group, survive: we only ever remove commands that are
    /// ours.
    pub(crate) fn unpatch(&self, root: &mut Value) -> bool {
        let Some(object) = root.as_object_mut() else {
            return false;
        };
        let Some(hooks) = object.get_mut(HOOKS).and_then(Value::as_object_mut) else {
            return false;
        };
        let Some(groups) = hooks.get_mut(self.event).and_then(Value::as_array_mut) else {
            return false;
        };

        let mut removed = false;
        for group in groups.iter_mut() {
            let Some(list) = group.get_mut(HOOKS).and_then(Value::as_array_mut) else {
                continue;
            };
            let before = list.len();
            list.retain(|entry| !runs(entry, &[LESSR]));
            removed |= list.len() != before;
        }
        if !removed {
            return false;
        }

        // An empty group, an empty event and an empty `hooks` are all artefacts
        // of our own entry; leaving them behind would mean `lessr uninstall`
        // still showed up in a diff.
        groups.retain(|group| !group_is_empty(group));
        if groups.is_empty() {
            hooks.remove(self.event);
        }
        if hooks.is_empty() {
            object.remove(HOOKS);
        }
        true
    }

    /// The groups for this event, for writing, creating the keys on the way
    /// down.
    ///
    /// A key that exists but holds the wrong kind of value is an error: a
    /// settings file where `hooks` is a string is not one we understand, and
    /// rewriting what we do not understand is how an agent stops working.
    fn groups_mut<'a>(&self, path: &Path, root: &'a mut Value) -> Result<&'a mut Vec<Value>> {
        let object = root
            .as_object_mut()
            .ok_or_else(|| shape(path, "the settings file is not a JSON object".to_string()))?;
        let hooks = object
            .entry(HOOKS)
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or_else(|| shape(path, "`hooks` is not a JSON object".to_string()))?;
        hooks
            .entry(self.event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| shape(path, format!("`hooks.{}` is not a JSON array", self.event)))
    }
}

/// Whether any command in this matcher group runs one of `names`.
fn group_runs(group: &Value, names: &[&str]) -> bool {
    group
        .get(HOOKS)
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|entry| runs(entry, names)))
}

/// Whether one hook entry's command runs one of `names`.
fn runs(entry: &Value, names: &[&str]) -> bool {
    entry
        .get(COMMAND)
        .and_then(Value::as_str)
        .is_some_and(|command| names.iter().any(|name| mentions(command, name)))
}

/// Whether a group has no commands left in it.
fn group_is_empty(group: &Value) -> bool {
    group
        .get(HOOKS)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

/// Whether a command line runs a program called `name`.
///
/// Substring matching with a word boundary, rather than parsing the line: the
/// command may be `/opt/homebrew/bin/rtk hook`, `npx rtk` or
/// `sh -c "rtk | tee log"`, and all three mean rtk is already here. The
/// boundary is what keeps `/home/mrtk/bin/tool` from counting.
pub(crate) fn mentions(command: &str, name: &str) -> bool {
    let bytes = command.as_bytes();
    command.match_indices(name).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + name.len()).copied();
        !before.is_some_and(is_word_byte) && !after.is_some_and(is_word_byte)
    })
}

/// A byte that would make a match part of a longer word.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// An [`Error::Shape`] for this file.
pub(crate) fn shape(path: &Path, detail: String) -> Error {
    Error::Shape {
        path: path.to_path_buf(),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LAYOUT: Layout = Layout {
        event: "PreToolUse",
        matcher: "Bash|Read",
    };

    fn entry() -> Value {
        json!({"type": "command", "command": "lessr hook claude"})
    }

    #[test]
    fn a_command_that_merely_contains_a_name_is_not_that_command() {
        assert!(mentions("/opt/homebrew/bin/rtk hook claude", "rtk"));
        assert!(mentions("npx rtk-cli", "rtk"));
        assert!(mentions("lessr hook claude", LESSR));
        assert!(!mentions("/home/mrtk/bin/tool", "rtk"));
        assert!(!mentions("blessrunner --go", LESSR));
    }

    #[test]
    fn the_legacy_rtk_script_counts_as_rtk() {
        let root = json!({"hooks": {"PreToolUse": [
            {"matcher": "Bash", "hooks": [
                {"type": "command", "command": "/home/dev/.claude/rtk-rewrite.sh"}
            ]}
        ]}});
        assert_eq!(LAYOUT.scan(&root), (false, true));
    }

    #[test]
    fn patching_twice_is_patching_once() {
        let path = Path::new("settings.json");
        let mut root = json!({});
        assert_eq!(
            LAYOUT.patch(path, &mut root, entry()).unwrap(),
            Patch::Added
        );
        assert_eq!(
            LAYOUT.patch(path, &mut root, entry()).unwrap(),
            Patch::AlreadyThere
        );
        assert_eq!(LAYOUT.groups(&root).len(), 1);
    }

    #[test]
    fn unpatching_an_untouched_config_changes_nothing() {
        let mut root = json!({"hooks": {"PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "rtk hook claude"}]}
        ]}});
        let before = root.clone();
        assert!(!LAYOUT.unpatch(&mut root));
        assert_eq!(root, before);
    }

    #[test]
    fn patching_then_unpatching_leaves_the_document_as_it_was() {
        let mut root = json!({"model": "opus"});
        let before = root.clone();
        LAYOUT.patch(Path::new("x"), &mut root, entry()).unwrap();
        assert!(LAYOUT.unpatch(&mut root));
        assert_eq!(
            root, before,
            "the keys we created are removed with our entry"
        );
    }
}
