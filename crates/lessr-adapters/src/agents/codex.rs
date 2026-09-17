//! Codex, OpenAI's CLI. Confidence: verified format, observe-only mechanism.
//!
//! Config: `~/.codex/config.toml`. Codex grew real hooks after
//! `docs/ADAPTERS.md` was written, and they are TOML array-of-tables:
//!
//! ```toml
//! [[hooks.PreToolUse]]
//! matcher = "shell"
//!
//! [[hooks.PreToolUse.hooks]]
//! type = "command"
//! command = "lessr hook codex"
//! timeout = 5
//! ```
//!
//! `timeout` is in **seconds** here, unlike Gemini CLI's milliseconds.
//!
//! Two things make this adapter quieter than the others:
//!
//! * `PostToolUse` cannot replace a tool's output — `updatedMCPToolOutput` is
//!   explicitly unsupported — so the gate has nothing to rewrite on this path.
//!   `lessr hook codex` therefore reads its payload and prints nothing. The
//!   plan says so in as many words, because a hook that cannot save anything
//!   yet is a spawn per tool call, and the user is the one who should decide
//!   whether to take that trade.
//! * A `permissionDecision` of `allow` without an `updatedInput` is rejected by
//!   Codex. Empty stdout is the only safe way to say "unchanged", so that is
//!   the only thing this hook ever prints.
//!
//! The file is edited as text between two marker comments rather than parsed
//! and re-serialised: TOML round-trips lose comments and formatting, and a
//! config.toml is a file people keep comments in. Everything outside the block
//! keeps its bytes, and the result is parsed before it is ever proposed.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent::AgentId;
use crate::agents::matcher_groups::{self, LESSR, RTK};
use crate::agents::{Adapter, Confidence};
use crate::backup;
use crate::detect::Detected;
use crate::error::Result;
use crate::file;
use crate::hook::Parsed;
use crate::paths::Paths;
use crate::plan::{Change, Plan};

/// See the module docs.
pub(crate) static ADAPTER: Codex = Codex;

/// The Codex adapter.
pub(crate) struct Codex;

/// `~/.codex/config.toml`, home-relative.
///
/// `$CODEX_HOME` can move this, and we do not read it: an adapter that
/// consulted the environment could not be tested hermetically.
/// `lessr init --show` prints the path it will use.
const CONFIG: &str = ".codex/config.toml";

/// The first line of the block we add, and the last. Everything between them
/// is ours; everything outside them is the user's and is never reparsed.
const BEGIN: &str = "# >>> lessr: added by `lessr init` (lessr.dev)";
/// See [`BEGIN`].
const END: &str = "# <<< lessr";

/// Which tools we ask to be called for. Codex's tool names are the least
/// verified part of this block, and a name that is wrong simply never matches.
const MATCHER: &str = "shell";

/// How long Codex waits for us, in seconds.
const TIMEOUT_SECS: u32 = 5;

impl Adapter for Codex {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn confidence(&self) -> Confidence {
        Confidence::Verified
    }

    fn detect(&self, paths: &Paths) -> Detected {
        let path = config_path(paths);
        let mut found = Detected {
            installed: path.exists() || path.parent().is_some_and(Path::exists),
            ..Detected::absent(AgentId::Codex)
        };
        let Ok(Some(text)) = file::read(&path) else {
            return found;
        };
        found.config_path = Some(path);
        found.already_patched = text.contains(BEGIN) || runs(&text, &[LESSR]);
        found.rtk_present = runs(&text, RTK);
        found
    }

    fn plan_init(&self, paths: &Paths, hook_command: &str) -> Result<Plan> {
        let path = config_path(paths);
        let before = file::read(&path)?;
        let text = before.as_deref().unwrap_or("");
        if text.contains(BEGIN) || runs(text, &[LESSR]) {
            return Ok(Plan::nothing(
                AgentId::Codex,
                format!("a Lessr hook is already in {}", path.display()),
            ));
        }

        let after = append_block(text, hook_command);
        // Refuse rather than hand Codex a file it cannot read: a config that
        // already defines `hooks.PreToolUse` as something other than an array
        // of tables would make our block a duplicate key.
        if let Err(err) = after.parse::<toml::Table>() {
            return Err(matcher_groups::shape(
                &path,
                format!("adding the hook block would not parse as TOML: {err}"),
            ));
        }

        let mut changes = Vec::with_capacity(2);
        if before.is_some() {
            changes.push(Change::Backup {
                from: path.clone(),
                to: backup::destination(paths, AgentId::Codex, &path),
            });
        }
        changes.push(Change::WriteFile {
            after,
            summary: format!(
                "call `{hook_command}` before {MATCHER} tools (Codex cannot replace tool \
                 output yet, so this hook observes and changes nothing)"
            ),
            path,
            before,
        });
        Ok(Plan {
            agent: AgentId::Codex,
            changes,
        })
    }

    fn plan_uninstall(&self, paths: &Paths) -> Result<Plan> {
        let records = backup::read_index(&backup::index_path(paths))?;
        if let Some(record) = backup::newest_for(&records, AgentId::Codex) {
            return Ok(Plan {
                agent: AgentId::Codex,
                changes: vec![Change::Restore {
                    from: record.stored_as.clone(),
                    to: record.original_path.clone(),
                }],
            });
        }

        let path = config_path(paths);
        let Some(text) = file::read(&path)? else {
            return Ok(Plan::nothing(
                AgentId::Codex,
                format!("no backup to restore and no {}", path.display()),
            ));
        };
        let Some(after) = remove_block(&text) else {
            return Ok(Plan::nothing(
                AgentId::Codex,
                format!(
                    "no backup to restore and no Lessr block in {}",
                    path.display()
                ),
            ));
        };
        // The file existed only because we created it; leaving an empty one
        // behind would be litter.
        if after.trim().is_empty() {
            return Ok(Plan {
                agent: AgentId::Codex,
                changes: vec![Change::Delete { path }],
            });
        }
        Ok(Plan {
            agent: AgentId::Codex,
            changes: vec![Change::WriteFile {
                after,
                summary: "remove the Lessr hook block".to_string(),
                path,
                before: Some(text),
            }],
        })
    }

    /// Codex's `PostToolUse` cannot replace a tool result, so there is nothing
    /// here for a filtering stage to work on. `Ok(None)` rather than an error:
    /// the payload is valid, it just has nothing we may change.
    fn parse_hook(&self, _root: &Value) -> Result<Option<Parsed>> {
        Ok(None)
    }

    /// Empty stdout is the only way to say "unchanged" that Codex accepts.
    fn render_unchanged(&self, _raw: &[u8]) -> Vec<u8> {
        Vec::new()
    }
}

/// `~/.codex/config.toml`.
fn config_path(paths: &Paths) -> PathBuf {
    super::home_join(paths, CONFIG)
}

/// `text` with our block added at the end, separated by one blank line.
fn append_block(text: &str, hook_command: &str) -> String {
    let command = toml::Value::String(hook_command.to_string()).to_string();
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str(&format!(
        "[[hooks.PreToolUse]]\nmatcher = \"{MATCHER}\"\n\n\
         [[hooks.PreToolUse.hooks]]\ntype = \"command\"\ncommand = {command}\n\
         timeout = {TIMEOUT_SECS}\n"
    ));
    out.push_str(END);
    out.push('\n');
    out
}

/// `text` with our block taken back out, exactly inverting [`append_block`].
///
/// `None` when there is no block. The blank line we added before it goes with
/// it, so a file we patched and then unpatched is the file we were given.
fn remove_block(text: &str) -> Option<String> {
    let start = text.find(BEGIN)?;
    let end = text[start..].find(END).map(|at| start + at + END.len())?;
    let tail = text[end..].strip_prefix('\n').unwrap_or(&text[end..]);

    let mut head = text[..start].to_string();
    if head.ends_with("\n\n") {
        head.pop();
    }
    head.push_str(tail);
    Some(head)
}

/// Whether any hook command in this config runs one of `names`.
///
/// Parsed rather than grepped, so that a name inside a comment or a prompt
/// string is not mistaken for an installed hook.
fn runs(text: &str, names: &[&str]) -> bool {
    let Ok(table) = text.parse::<toml::Table>() else {
        return false;
    };
    let Some(groups) = table
        .get("hooks")
        .and_then(|hooks| hooks.get("PreToolUse"))
        .and_then(toml::Value::as_array)
    else {
        return false;
    };
    groups
        .iter()
        .filter_map(|group| group.get("hooks"))
        .filter_map(toml::Value::as_array)
        .flatten()
        .filter_map(|entry| entry.get("command"))
        .filter_map(toml::Value::as_str)
        .any(|command| {
            names
                .iter()
                .any(|name| matcher_groups::mentions(command, name))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::parse_hook_input;
    use crate::plan::apply;

    const HOOK: &str = "lessr hook codex";

    fn setup() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("home"), tmp.path().join("config"));
        (tmp, paths)
    }

    fn write_config(paths: &Paths, text: &str) {
        let path = config_path(paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn config(paths: &Paths) -> String {
        std::fs::read_to_string(config_path(paths)).unwrap()
    }

    #[test]
    fn the_block_is_valid_toml_with_a_timeout_in_seconds() {
        let (_tmp, paths) = setup();
        apply(&crate::plan_init(&paths, AgentId::Codex, HOOK).unwrap()).unwrap();

        let table: toml::Table = config(&paths).parse().unwrap();
        let group = &table["hooks"]["PreToolUse"].as_array().unwrap()[0];
        assert_eq!(group["matcher"].as_str(), Some(MATCHER));
        let entry = &group["hooks"].as_array().unwrap()[0];
        assert_eq!(entry["type"].as_str(), Some("command"));
        assert_eq!(entry["command"].as_str(), Some(HOOK));
        assert_eq!(
            entry["timeout"].as_integer(),
            Some(5),
            "seconds, not milliseconds"
        );
    }

    #[test]
    fn the_user_keeps_every_byte_outside_our_block() {
        let (_tmp, paths) = setup();
        let original = "# my notes\nmodel = \"o3\"\n\n[tui]\ntheme   =   \"dark\"\n";
        write_config(&paths, original);

        apply(&crate::plan_init(&paths, AgentId::Codex, HOOK).unwrap()).unwrap();
        let patched = config(&paths);
        assert!(
            patched.starts_with(original),
            "the block is appended, not merged"
        );
        assert!(patched.contains(BEGIN) && patched.contains(END));

        // And uninstall puts it back exactly, from the backup taken first.
        apply(&crate::plan_uninstall(&paths, AgentId::Codex).unwrap()).unwrap();
        assert_eq!(
            std::fs::read(config_path(&paths)).unwrap(),
            original.as_bytes()
        );
    }

    #[test]
    fn removing_the_block_inverts_adding_it() {
        let original = "model = \"o3\"\n";
        let patched = append_block(original, HOOK);
        assert_eq!(remove_block(&patched).as_deref(), Some(original));
        assert_eq!(remove_block(original), None);
    }

    #[test]
    fn a_config_we_created_is_deleted_rather_than_left_empty() {
        let (_tmp, paths) = setup();
        apply(&crate::plan_init(&paths, AgentId::Codex, HOOK).unwrap()).unwrap();
        apply(&crate::plan_uninstall(&paths, AgentId::Codex).unwrap()).unwrap();
        assert!(!config_path(&paths).exists());
    }

    #[test]
    fn a_config_whose_hooks_are_not_ours_to_extend_is_refused() {
        let (_tmp, paths) = setup();
        write_config(&paths, "[hooks]\nPreToolUse = \"handled elsewhere\"\n");
        let err = crate::plan_init(&paths, AgentId::Codex, HOOK).unwrap_err();
        assert!(err.to_string().contains("TOML"), "{err}");
        assert_eq!(
            config(&paths),
            "[hooks]\nPreToolUse = \"handled elsewhere\"\n"
        );
    }

    #[test]
    fn an_existing_rtk_hook_is_reported_and_left_alone() {
        let (_tmp, paths) = setup();
        write_config(
            &paths,
            "[[hooks.PreToolUse]]\nmatcher = \"shell\"\n\n[[hooks.PreToolUse.hooks]]\n\
             type = \"command\"\ncommand = \"/usr/local/bin/rtk hook codex\"\n",
        );
        let found = ADAPTER.detect(&paths);
        assert!(found.rtk_present);
        assert!(!found.already_patched);

        apply(&crate::plan_init(&paths, AgentId::Codex, HOOK).unwrap()).unwrap();
        let table: toml::Table = config(&paths).parse().unwrap();
        let groups = table["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "our own group, beside rtk's");
        assert_eq!(
            groups[0]["hooks"].as_array().unwrap()[0]["command"].as_str(),
            Some("/usr/local/bin/rtk hook codex"),
            "rtk's entry keeps its bytes"
        );
    }

    #[test]
    fn a_name_in_a_comment_is_not_an_installed_hook() {
        assert!(!runs(
            "# lessr hook codex would go here\nmodel = \"o3\"\n",
            &[LESSR]
        ));
    }

    #[test]
    fn the_hook_reads_its_payload_and_says_nothing() {
        // Codex cannot replace a tool result, so the payload parses and the
        // hook prints nothing at all rather than a decision it would reject.
        let payload = parse_hook_input(
            AgentId::Codex,
            br#"{"hook_event_name": "PreToolUse", "tool_name": "shell"}"#,
        )
        .unwrap();
        assert!(payload.result().is_none());
        assert!(
            crate::render_hook_output(AgentId::Codex, &payload)
                .unwrap()
                .is_empty(),
            "never a bare allow, which Codex rejects"
        );
    }
}
