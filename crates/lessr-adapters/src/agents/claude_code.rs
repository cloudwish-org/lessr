//! Claude Code. Confidence: verified.
//!
//! Config: `~/.claude/settings.json`. Mechanism: a `PostToolUse` hook —
//! `PreToolUse` runs before the tool does and its payload carries no output, so
//! there is nothing to filter there:
//!
//! ```json
//! {"hooks": {"PostToolUse": [{"matcher": "Bash",
//!  "hooks": [{"type": "command", "command": "lessr hook claude"}]}]}}
//! ```
//!
//! The file is the user's, not ours: it is parsed with key order preserved, the
//! one entry is added, and everything else is written back exactly as it came
//! in. An existing `rtk` hook is reported and left alone — rtk installs on
//! `PreToolUse`, so it is not even in the same list.
//!
//! **The output contract is not an echo.** Claude Code replaces a tool result
//! only when stdout is
//!
//! ```json
//! {"hookSpecificOutput": {"hookEventName": "PostToolUse",
//!  "updatedToolOutput": {"stdout": "…", "stderr": "", "interrupted": false,
//!                        "isImage": false}}}
//! ```
//!
//! (`updatedToolOutput` covers every tool, from Claude Code 2.1.121; the older
//! `updatedMCPToolOutput` is MCP-only and is not used here.) Anything else —
//! the payload echoed back, a `decision`/`reason` pair, a non-zero exit — is
//! read as "no replacement" and the original output is used. The failure is
//! silent, which is why two things here are deliberately narrow:
//!
//! * The whole `tool_response` object is round-tripped with only its text
//!   replaced, never rebuilt from scratch: "a value that doesn't match the
//!   tool's output schema is ignored and the original output is used".
//! * The matcher is `Bash` alone, whose shape is the one we have a real payload
//!   for (`fixtures/hook/claude_post_tool_use.json`). Trap and dedup want
//!   `Read` as well; widening the matcher is blocked on capturing a real `Read`
//!   payload to learn its output shape, not on writing more code here.
//!
//! With nothing to say, this hook prints nothing at all and exits 0.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use lessr_core::{Command, ToolKind, ToolResult};
use serde_json::{Value, json};

use crate::agent::AgentId;
use crate::agents::matcher_groups::{Layout, Patch};
use crate::agents::{Adapter, Confidence};
use crate::backup;
use crate::detect::Detected;
use crate::error::{Error, Result};
use crate::file;
use crate::hook::{Parsed, Slot};
use crate::paths::Paths;
use crate::plan::{Change, Plan};

/// See the module docs.
pub(crate) static ADAPTER: ClaudeCode = ClaudeCode;

/// The Claude Code adapter.
pub(crate) struct ClaudeCode;

/// `~/.claude/settings.json`, home-relative.
const SETTINGS: &str = ".claude/settings.json";

/// The event, and the matcher for the group we add.
///
/// `Bash` only, for now: see the module docs. rtk lives on `PreToolUse`, a
/// different list entirely, so there is nothing here to chain behind.
const LAYOUT: Layout = Layout {
    event: "PostToolUse",
    matcher: "Bash",
    chain_behind_rtk: false,
};

/// Where the filterable text lives in a `PostToolUse` payload.
///
/// One path and no fallbacks. A `tool_response` that is a plain string is
/// readable, but we could not put it back: `updatedToolOutput` has to match the
/// tool's own output schema or it is ignored without a word. Anything we cannot
/// write back is something we do not claim to have filtered.
const CONTENT: Slot = Slot::at(&["tool_response", "stdout"]);

impl Adapter for ClaudeCode {
    fn id(&self) -> AgentId {
        AgentId::ClaudeCode
    }

    fn confidence(&self) -> Confidence {
        Confidence::Verified
    }

    fn detect(&self, paths: &Paths) -> Detected {
        let path = settings_path(paths);
        let mut found = Detected {
            // The directory is the agent; the file appears the first time
            // something is configured, so a user who has never opened settings
            // still counts as having Claude Code installed.
            installed: path.exists() || path.parent().is_some_and(Path::exists),
            ..Detected::absent(AgentId::ClaudeCode)
        };

        // Anything unreadable or unparseable leaves the flags false: we are not
        // patched, and we are not going to touch it either.
        let Ok(Some(text)) = file::read(&path) else {
            return found;
        };
        found.config_path = Some(path.clone());
        let Ok(root) = file::parse(&path, &text) else {
            return found;
        };
        (found.already_patched, found.rtk_present) = LAYOUT.scan(&root);
        found
    }

    fn plan_init(&self, paths: &Paths, hook_command: &str) -> Result<Plan> {
        let path = settings_path(paths);
        let before = file::read(&path)?;
        let mut root = match &before {
            Some(text) => file::parse(&path, text)?,
            None => json!({}),
        };

        let entry = json!({"type": "command", "command": hook_command});
        let outcome = LAYOUT.patch(&path, &mut root, entry)?;
        if outcome == Patch::AlreadyThere {
            return Ok(Plan::nothing(
                AgentId::ClaudeCode,
                format!("a Lessr hook is already in {}", path.display()),
            ));
        }

        let mut changes = Vec::with_capacity(2);
        if before.is_some() {
            changes.push(Change::Backup {
                from: path.clone(),
                to: backup::destination(paths, AgentId::ClaudeCode, &path),
            });
        }
        changes.push(Change::WriteJson {
            after: file::pretty(&root),
            summary: summary(outcome, hook_command),
            path,
            before,
        });
        Ok(Plan {
            agent: AgentId::ClaudeCode,
            changes,
        })
    }

    fn plan_uninstall(&self, paths: &Paths) -> Result<Plan> {
        let records = backup::read_index(&backup::index_path(paths))?;
        if let Some(record) = backup::newest_for(&records, AgentId::ClaudeCode) {
            return Ok(Plan {
                agent: AgentId::ClaudeCode,
                changes: vec![Change::Restore {
                    from: record.stored_as.clone(),
                    to: record.original_path.clone(),
                }],
            });
        }

        // No backup means `lessr init` created the file from nothing, so there
        // is no earlier version to put back: take our own entry out instead.
        // Deliberately without taking a backup of its own — the index must only
        // ever hold configs from before Lessr touched them, or a second
        // uninstall would restore the patched file it just cleaned.
        let path = settings_path(paths);
        let Some(text) = file::read(&path)? else {
            return Ok(Plan::nothing(
                AgentId::ClaudeCode,
                format!("no backup to restore and no {}", path.display()),
            ));
        };
        let mut root = file::parse(&path, &text)?;
        if !LAYOUT.unpatch(&mut root) {
            return Ok(Plan::nothing(
                AgentId::ClaudeCode,
                format!(
                    "no backup to restore and no Lessr hook in {}",
                    path.display()
                ),
            ));
        }
        Ok(Plan {
            agent: AgentId::ClaudeCode,
            changes: vec![Change::WriteJson {
                after: file::pretty(&root),
                summary: "remove the Lessr hook entry".to_string(),
                path,
                before: Some(text),
            }],
        })
    }

    fn parse_hook(&self, root: &Value) -> Result<Option<Parsed>> {
        Ok(read_payload(root))
    }

    /// The whole `Output` object goes back with only its text replaced.
    ///
    /// Claude Code ignores an `updatedToolOutput` that does not match the
    /// tool's own output schema, and ignores it silently — so a minimal object
    /// we had synthesised would look exactly like success while changing
    /// nothing. Round-tripping keeps every key it had, and its type.
    fn render_hook(&self, root: &Value, slot: Slot, content: &str) -> Result<Vec<u8>> {
        let mut patched = root.clone();
        slot.put(&mut patched, content);

        let output = patched
            .get("tool_response")
            .filter(|value| value.is_object());
        let Some(output) = output else {
            // A shape we cannot vouch for. Saying nothing leaves the agent
            // with the tool's own output, which is the safe answer.
            return Ok(Vec::new());
        };
        let envelope = json!({
            "hookSpecificOutput": {
                "hookEventName": LAYOUT.event,
                "updatedToolOutput": output.clone(),
            }
        });
        serde_json::to_vec(&envelope).map_err(Error::HookJson)
    }

    /// Nothing to replace, so nothing is printed. An echoed payload would be
    /// parsed, found to carry no field Claude Code recognises, and silently
    /// ignored — the same outcome, said at more length.
    fn render_unchanged(&self, _raw: &[u8]) -> Vec<u8> {
        Vec::new()
    }
}

/// `~/.claude/settings.json`.
fn settings_path(paths: &Paths) -> PathBuf {
    super::home_join(paths, SETTINGS)
}

/// The one-line summary for `lessr init --show`.
fn summary(outcome: Patch, hook_command: &str) -> String {
    match outcome {
        Patch::AlreadyThere => String::from("nothing to add"),
        Patch::ChainedBehindRtk => format!("chain `{hook_command}` behind the existing rtk hook"),
        Patch::Added => format!("run `{hook_command}` after {} tools", LAYOUT.matcher),
    }
}

/// Read a hook payload into something a stage can filter.
///
/// Claude Code sends `hook_event_name`, `tool_name`, `tool_input`, and on
/// `PostToolUse` also `tool_response`. Both have to line up: an event other
/// than ours cannot take an `updatedToolOutput`, and no `tool_response` means
/// nothing has run yet. A payload that fails either test is not an error, it
/// is a call we leave alone.
fn read_payload(root: &Value) -> Option<Parsed> {
    let event = root.get("hook_event_name").and_then(Value::as_str);
    if event.is_some_and(|name| name != LAYOUT.event) {
        return None;
    }
    let text = CONTENT.get(root)?;

    let tool_name = root.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let tool = tool_kind(tool_name);
    let input = root.get("tool_input");

    Some((
        ToolResult {
            tool,
            tool_name: tool_name.to_string(),
            command: if tool == ToolKind::Shell {
                input
                    .and_then(|input| input.get("command"))
                    .and_then(Value::as_str)
                    .and_then(parse_command)
            } else {
                None
            },
            path: input
                .and_then(|input| input.get("file_path").or_else(|| input.get("path")))
                .and_then(Value::as_str)
                .map(PathBuf::from),
            explicit_selection: explicit_selection(tool, input),
            content: Bytes::copy_from_slice(text.as_bytes()),
        },
        CONTENT,
    ))
}

/// Claude Code's tool names, mapped to the kinds stages match on.
///
/// The matcher only asks to be called for `Bash` today (see the module docs),
/// so the other arms are unexercised in production. They stay because widening
/// the matcher is a one-line change once a real `Read` payload tells us the
/// shape of its output, and because a payload arriving from a hand-edited
/// settings file should still be read correctly rather than guessed at.
fn tool_kind(tool_name: &str) -> ToolKind {
    match tool_name {
        "Bash" => ToolKind::Shell,
        "Read" => ToolKind::Read,
        "Grep" | "Glob" => ToolKind::Search,
        "Write" | "Edit" => ToolKind::Edit,
        _ => ToolKind::Other,
    }
}

/// Split a shell command into program and arguments.
///
/// `shlex` so that `git commit -m "two words"` is three arguments and not four;
/// a line it cannot split (an unbalanced quote) falls back to whitespace rather
/// than being dropped, because a command we cannot parse is still a command a
/// rule may want to match on.
pub(crate) fn parse_command(line: &str) -> Option<Command> {
    let mut words = shlex::split(line).unwrap_or_else(|| {
        line.split_whitespace()
            .map(std::string::ToString::to_string)
            .collect()
    });
    if words.is_empty() {
        return None;
    }
    let program = strip_directory(&words.remove(0));
    if program.is_empty() {
        return None;
    }
    Some(Command {
        program,
        args: words,
    })
}

/// `/usr/bin/git` becomes `git`.
///
/// Both separators, whatever we are running on: the payload comes from the
/// agent, and the agent may be on Windows when we are reading its fixture on
/// Linux.
pub(crate) fn strip_directory(program: &str) -> String {
    program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_string()
}

/// Whether the agent asked for exactly these bytes.
///
/// Loop-safety rule 1: requested content is sacred. A `Read` with an offset or
/// a limit, and a `Grep` or `Glob` with a pattern, are the agent naming what it
/// wants; a filtering stage has to hand that back untouched.
fn explicit_selection(tool: ToolKind, input: Option<&Value>) -> bool {
    let Some(input) = input else {
        return false;
    };
    match tool {
        ToolKind::Read => is_set(input.get("offset")) || is_set(input.get("limit")),
        ToolKind::Search => input
            .get("pattern")
            .and_then(Value::as_str)
            .is_some_and(|pattern| !pattern.is_empty()),
        ToolKind::Shell | ToolKind::Edit | ToolKind::Other => false,
    }
}

/// Present and not `null`.
fn is_set(value: Option<&Value>) -> bool {
    matches!(value, Some(value) if !value.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{parse_hook_input, render_hook_output};
    use crate::plan::apply;

    /// What the CLI passes; the binary's own name is the caller's business.
    const HOOK: &str = "lessr hook claude";

    /// A settings file written the way a human writes one: tabs, no space
    /// after a colon, no trailing newline. Restoring it byte for byte is the
    /// whole promise of `lessr uninstall`.
    const MESSY: &str = "{\n\t\"model\":\"opus\",\n  \"permissions\": {\"allow\": [\"Bash(git status)\"]},\n  \"env\": {\"FOO\": \"bar\"}\n}";

    fn setup() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("home"), tmp.path().join("config"));
        (tmp, paths)
    }

    fn write_settings(paths: &Paths, text: &str) {
        let path = settings_path(paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn settings(paths: &Paths) -> String {
        std::fs::read_to_string(settings_path(paths)).unwrap()
    }

    fn init(paths: &Paths) -> Plan {
        crate::plan_init(paths, AgentId::ClaudeCode, HOOK).unwrap()
    }

    fn uninstall(paths: &Paths) -> Plan {
        crate::plan_uninstall(paths, AgentId::ClaudeCode).unwrap()
    }

    /// The shared fixtures `CLAUDE.md` points at for hook testing.
    fn fixture(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/hook")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }

    #[test]
    fn a_fresh_install_creates_the_settings_file() {
        let (_tmp, paths) = setup();
        let plan = init(&paths);

        // Nothing to back up: there was no file.
        assert!(
            plan.changes
                .iter()
                .all(|change| matches!(change, Change::WriteJson { before: None, .. })),
            "{:?}",
            plan.changes
        );
        apply(&plan).unwrap();

        let written: Value = serde_json::from_str(&settings(&paths)).unwrap();
        let entry = &written["hooks"]["PostToolUse"][0];
        assert_eq!(entry["matcher"], LAYOUT.matcher);
        assert_eq!(entry["hooks"][0]["type"], "command");
        assert_eq!(entry["hooks"][0]["command"], HOOK);
        assert!(
            settings(&paths).ends_with("}\n"),
            "two-space pretty, one newline"
        );
    }

    #[test]
    fn an_existing_file_keeps_its_own_keys_and_their_order() {
        let (_tmp, paths) = setup();
        write_settings(&paths, MESSY);
        apply(&init(&paths)).unwrap();

        let text = settings(&paths);
        let at = |key: &str| text.find(key).unwrap_or_else(|| panic!("{key} is gone"));
        assert!(at("model") < at("permissions"), "{text}");
        assert!(at("permissions") < at("env"), "{text}");
        assert!(
            at("env") < at("hooks"),
            "hooks is appended, not inserted: {text}"
        );

        let written: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(written["model"], "opus");
        assert_eq!(written["permissions"]["allow"][0], "Bash(git status)");
        assert_eq!(written["env"]["FOO"], "bar");
    }

    #[test]
    fn an_existing_file_is_backed_up_before_it_is_written() {
        let (_tmp, paths) = setup();
        write_settings(&paths, MESSY);

        let plan = init(&paths);
        assert!(
            matches!(plan.changes.first(), Some(Change::Backup { .. })),
            "{:?}",
            plan.changes
        );
        let applied = apply(&plan).unwrap();
        assert_eq!(applied.backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&applied.backups[0]).unwrap(), MESSY);
    }

    #[test]
    fn a_second_init_has_nothing_to_do() {
        let (_tmp, paths) = setup();
        apply(&init(&paths)).unwrap();
        let once = settings(&paths);

        let again = init(&paths);
        assert!(again.is_noop(), "{:?}", again.changes);
        assert!(again.render().contains("already"), "{}", again.render());
        apply(&again).unwrap();
        assert_eq!(settings(&paths), once, "an idempotent init changes nothing");
    }

    #[test]
    fn an_rtk_hook_is_reported_and_left_where_it_is() {
        let (_tmp, paths) = setup();
        let rtk_entry = r#"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {"type": "command", "command": "/opt/homebrew/bin/rtk hook claude"}
        ]
      }
    ]
  }
}
"#;
        write_settings(&paths, rtk_entry);

        // rtk hooks PreToolUse and we hook PostToolUse, so there is nothing to
        // chain behind: our entry goes in a list of its own and rtk's keeps
        // every byte it had.
        apply(&init(&paths)).unwrap();

        let written: Value = serde_json::from_str(&settings(&paths)).unwrap();
        let rtk_groups = written["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(rtk_groups.len(), 1);
        assert_eq!(rtk_groups[0]["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(
            rtk_groups[0]["hooks"][0]["command"],
            "/opt/homebrew/bin/rtk hook claude"
        );

        let ours = written["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0]["hooks"][0]["command"], HOOK);

        let found = ADAPTER.detect(&paths);
        assert!(
            found.rtk_present,
            "rtk is still reported from another event"
        );
        assert!(found.already_patched);

        // And uninstalling puts the file back exactly as rtk left it.
        apply(&crate::plan_uninstall(&paths, AgentId::ClaudeCode).unwrap()).unwrap();
        assert_eq!(
            std::fs::read(settings_path(&paths)).unwrap(),
            rtk_entry.as_bytes()
        );
    }

    #[test]
    fn init_then_uninstall_leaves_the_file_byte_for_byte() {
        let (_tmp, paths) = setup();
        write_settings(&paths, MESSY);

        apply(&init(&paths)).unwrap();
        assert_ne!(settings(&paths), MESSY, "init did write something");

        let undo = uninstall(&paths);
        assert!(matches!(undo.changes.as_slice(), [Change::Restore { .. }]));
        apply(&undo).unwrap();

        assert_eq!(
            std::fs::read(settings_path(&paths)).unwrap(),
            MESSY.as_bytes(),
            "whitespace, key order and the missing trailing newline all survive"
        );
        assert!(!ADAPTER.detect(&paths).already_patched);
    }

    #[test]
    fn uninstalling_a_file_we_created_removes_only_our_entry() {
        let (_tmp, paths) = setup();
        apply(&init(&paths)).unwrap();

        // Nothing was backed up, because nothing was there to back up.
        let undo = uninstall(&paths);
        apply(&undo).unwrap();
        assert_eq!(
            settings(&paths),
            "{}\n",
            "the hook keys we added are gone too"
        );
    }

    #[test]
    fn uninstalling_leaves_another_tools_hook_alone() {
        let (_tmp, paths) = setup();
        write_settings(
            &paths,
            r#"{
  "hooks": {
    "PreToolUse": [
      {"matcher": "Bash", "hooks": [{"type": "command", "command": "rtk hook claude"}]}
    ],
    "PostToolUse": [
      {"matcher": "Edit", "hooks": [{"type": "command", "command": "prettier"}]},
      {"matcher": "Bash", "hooks": [{"type": "command", "command": "lessr hook claude"}]}
    ]
  }
}
"#,
        );

        apply(&uninstall(&paths)).unwrap();

        let written: Value = serde_json::from_str(&settings(&paths)).unwrap();
        assert_eq!(
            written["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "rtk hook claude"
        );
        let post = written["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 1, "our group went, the formatter's stayed");
        assert_eq!(post[0]["hooks"][0]["command"], "prettier");
    }

    #[test]
    fn uninstalling_when_nothing_was_installed_says_so() {
        let (_tmp, paths) = setup();
        assert!(uninstall(&paths).is_noop());

        write_settings(&paths, "{\"model\": \"opus\"}\n");
        let undo = uninstall(&paths);
        assert!(undo.is_noop(), "{:?}", undo.changes);
        assert_eq!(settings(&paths), "{\"model\": \"opus\"}\n");
    }

    #[test]
    fn settings_we_cannot_parse_are_an_error_not_an_overwrite() {
        let (_tmp, paths) = setup();
        write_settings(&paths, "{\"model\": \"opus\",}");

        let err = crate::plan_init(&paths, AgentId::ClaudeCode, HOOK).unwrap_err();
        assert!(err.to_string().contains("not valid JSON"), "{err}");
        assert_eq!(settings(&paths), "{\"model\": \"opus\",}", "untouched");
        assert!(crate::plan_uninstall(&paths, AgentId::ClaudeCode).is_err());
    }

    #[test]
    fn settings_of_a_shape_we_do_not_understand_are_an_error() {
        let (_tmp, paths) = setup();
        write_settings(&paths, "{\"hooks\": \"please filter my tools\"}");
        let err = crate::plan_init(&paths, AgentId::ClaudeCode, HOOK).unwrap_err();
        assert!(err.to_string().contains("`hooks`"), "{err}");

        write_settings(&paths, "[]");
        let err = crate::plan_init(&paths, AgentId::ClaudeCode, HOOK).unwrap_err();
        assert!(err.to_string().contains("not a JSON object"), "{err}");

        write_settings(&paths, "{\"hooks\": {\"PostToolUse\": {}}}");
        let err = crate::plan_init(&paths, AgentId::ClaudeCode, HOOK).unwrap_err();
        assert!(err.to_string().contains("PostToolUse"), "{err}");
    }

    #[test]
    fn detection_reports_an_empty_machine_and_a_patched_one() {
        let (_tmp, paths) = setup();
        let found = ADAPTER.detect(&paths);
        assert!(!found.installed);
        assert_eq!(found.config_path, None);

        std::fs::create_dir_all(paths.home.join(".claude")).unwrap();
        let found = ADAPTER.detect(&paths);
        assert!(found.installed, "the directory is enough");
        assert_eq!(found.config_path, None, "there is no file yet");

        apply(&init(&paths)).unwrap();
        let found = ADAPTER.detect(&paths);
        assert_eq!(found.config_path, Some(settings_path(&paths)));
        assert!(found.already_patched);
        assert!(!found.rtk_present);
    }

    #[test]
    fn detection_survives_a_config_it_cannot_read() {
        let (_tmp, paths) = setup();
        write_settings(&paths, "{ not json at all");
        let found = ADAPTER.detect(&paths);
        assert!(found.installed);
        assert!(!found.already_patched);
        assert!(!found.rtk_present);
    }

    #[test]
    fn the_pre_tool_use_fixture_has_nothing_to_filter() {
        let input = fixture("claude_pre_tool_use.json");
        let payload = parse_hook_input(AgentId::ClaudeCode, &input).unwrap();

        assert!(payload.result().is_none(), "the tool has not run yet");
        assert!(
            render_hook_output(AgentId::ClaudeCode, &payload)
                .unwrap()
                .is_empty(),
            "nothing to replace, so nothing is printed"
        );
    }

    #[test]
    fn the_event_we_install_on_is_the_event_we_can_filter() {
        // Derived from the fixtures rather than written down twice: whichever
        // payload the parser can take a result from, that payload's event is
        // the event `lessr init` has to register for. Getting this wrong is
        // silent in production — a hook on the wrong event simply never has
        // anything to do.
        let mut filterable = Vec::new();
        for name in ["claude_pre_tool_use.json", "claude_post_tool_use.json"] {
            let raw = fixture(name);
            if parse_hook_input(AgentId::ClaudeCode, &raw)
                .unwrap()
                .result()
                .is_some()
            {
                let json: Value = serde_json::from_slice(&raw).unwrap();
                filterable.push(json["hook_event_name"].as_str().unwrap().to_string());
            }
        }
        assert_eq!(filterable, vec![LAYOUT.event.to_string()]);
    }

    #[test]
    fn the_post_tool_use_fixture_carries_the_command_and_the_output() {
        let payload =
            parse_hook_input(AgentId::ClaudeCode, &fixture("claude_post_tool_use.json")).unwrap();
        let result = payload.result().expect("a tool_response to filter");

        assert_eq!(result.tool, ToolKind::Shell);
        assert_eq!(result.tool_name, "Bash");
        assert_eq!(
            result.command,
            Some(Command {
                program: "cargo".to_string(),
                args: vec!["test".to_string(), "--workspace".to_string()],
            })
        );
        assert!(
            !result.explicit_selection,
            "a shell command asks for nothing"
        );
        assert!(result.content.starts_with(b"   Compiling lessr-core"));
        assert!(result.content.ends_with(b"\n\n"));
    }

    #[test]
    fn a_rewritten_result_is_printed_in_the_envelope_the_docs_document() {
        let mut payload =
            parse_hook_input(AgentId::ClaudeCode, &fixture("claude_post_tool_use.json")).unwrap();
        payload.result_mut().unwrap().content = Bytes::from_static(b"test result: ok. 3 passed\n");

        let out = render_hook_output(AgentId::ClaudeCode, &payload).unwrap();

        // The exact document from the hooks reference, byte for byte: the
        // whole Output object with only its text replaced, wrapped in
        // `hookSpecificOutput`. Anything else is ignored in silence.
        assert_eq!(
            String::from_utf8(out).unwrap(),
            concat!(
                r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","#,
                r#""updatedToolOutput":{"stdout":"test result: ok. 3 passed\n","#,
                r#""stderr":"","interrupted":false,"isImage":false}}}"#,
            )
        );
    }

    #[test]
    fn every_key_of_the_output_object_survives_with_its_type() {
        let input = br#"{"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_response": {"stdout": "long\n", "stderr": "warn\n", "interrupted": false, "isImage": false, "extra": {"n": 1}}}"#;
        let mut payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        payload.result_mut().unwrap().content = Bytes::from_static(b"short\n");

        let out = render_hook_output(AgentId::ClaudeCode, &payload).unwrap();
        let written: Value = serde_json::from_slice(&out).unwrap();
        let output = &written["hookSpecificOutput"]["updatedToolOutput"];

        assert_eq!(output["stdout"], "short\n");
        assert_eq!(output["stderr"], "warn\n", "not ours to touch");
        assert_eq!(output["interrupted"], false, "still a bool, not a string");
        assert_eq!(output["isImage"], false);
        assert_eq!(output["extra"]["n"], 1, "a key we have never heard of");
    }

    #[test]
    fn a_response_that_is_just_a_string_is_left_alone() {
        // Readable, but there is no `updatedToolOutput` shape we could vouch
        // for, and a wrong one is ignored without a word. So we do not claim
        // to have filtered it.
        let input =
            br#"{"tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_response": "a\nb\n"}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        assert!(payload.result().is_none());
        assert!(
            render_hook_output(AgentId::ClaudeCode, &payload)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_response_shape_we_do_not_read_leaves_the_payload_alone() {
        let input = br#"{"tool_name": "Read", "tool_response": {"file": {"content": "x"}}}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        assert!(payload.result().is_none());
        assert!(
            render_hook_output(AgentId::ClaudeCode, &payload)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_payload_from_another_event_is_not_ours_to_change() {
        // Belt and braces against the bug this adapter already had once: a
        // `tool_response` on an event that cannot take an `updatedToolOutput`
        // is not something to filter.
        let input = br#"{"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_response": {"stdout": "x"}}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        assert!(payload.result().is_none());
    }

    #[test]
    fn a_read_with_a_range_is_an_explicit_selection() {
        let ranged = br#"{"tool_name": "Read", "tool_input": {"file_path": "/src/lib.rs", "offset": 40, "limit": 20}, "tool_response": {"stdout": "x"}}"#;
        let result = parse_hook_input(AgentId::ClaudeCode, ranged).unwrap();
        let result = result.result().unwrap().clone();
        assert_eq!(result.tool, ToolKind::Read);
        assert_eq!(result.path, Some(PathBuf::from("/src/lib.rs")));
        assert!(result.explicit_selection, "loop-safety rule 1");
        assert!(result.command.is_none(), "a read is not a shell command");

        let whole = br#"{"tool_name": "Read", "tool_input": {"file_path": "/src/lib.rs"}, "tool_response": {"stdout": "x"}}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, whole).unwrap();
        assert!(!payload.result().unwrap().explicit_selection);
    }

    #[test]
    fn a_search_pattern_is_an_explicit_selection() {
        let searched = br#"{"tool_name": "Grep", "tool_input": {"pattern": "fn main", "path": "/src"}, "tool_response": {"stdout": "x"}}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, searched).unwrap();
        let result = payload.result().unwrap();
        assert_eq!(result.tool, ToolKind::Search);
        assert!(result.explicit_selection);
        assert_eq!(result.path, Some(PathBuf::from("/src")));

        let patternless = br#"{"tool_name": "Glob", "tool_input": {"pattern": ""}, "tool_response": {"stdout": "x"}}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, patternless).unwrap();
        assert!(!payload.result().unwrap().explicit_selection);
    }

    #[test]
    fn tool_names_map_to_the_kinds_stages_match_on() {
        assert_eq!(tool_kind("Bash"), ToolKind::Shell);
        assert_eq!(tool_kind("Read"), ToolKind::Read);
        assert_eq!(tool_kind("Grep"), ToolKind::Search);
        assert_eq!(tool_kind("Glob"), ToolKind::Search);
        assert_eq!(tool_kind("Write"), ToolKind::Edit);
        assert_eq!(tool_kind("Edit"), ToolKind::Edit);
        assert_eq!(tool_kind("WebFetch"), ToolKind::Other);
        assert_eq!(tool_kind(""), ToolKind::Other);
    }

    #[test]
    fn a_command_keeps_its_quoting_and_loses_its_directory() {
        let parsed = parse_command("/usr/bin/git commit -m \"two words\"").unwrap();
        assert_eq!(parsed.program, "git");
        assert_eq!(parsed.args, ["commit", "-m", "two words"]);

        // An unbalanced quote still has to produce something.
        let ragged = parse_command("echo \"unclosed").unwrap();
        assert_eq!(ragged.program, "echo");
        assert_eq!(ragged.args, ["\"unclosed"]);

        assert!(parse_command("   ").is_none());

        // The line is lexed as the shell would lex it, so a backslash in it is
        // an escape, not a separator. The separator only matters once it is
        // the program's own path, which is what `strip_directory` sees.
        assert_eq!(strip_directory(r"C:\tools\rg.exe"), "rg.exe");
        assert_eq!(strip_directory("/usr/local/bin/cargo"), "cargo");
        assert_eq!(strip_directory("cargo"), "cargo");
    }

    #[test]
    fn content_that_is_no_longer_utf8_is_refused_rather_than_sent() {
        let mut payload =
            parse_hook_input(AgentId::ClaudeCode, &fixture("claude_post_tool_use.json")).unwrap();
        payload.result_mut().unwrap().content = Bytes::from_static(&[0xff, 0xfe, b'x']);

        let err = render_hook_output(AgentId::ClaudeCode, &payload).unwrap_err();
        assert!(err.to_string().contains("UTF-8"), "{err}");
    }
}
