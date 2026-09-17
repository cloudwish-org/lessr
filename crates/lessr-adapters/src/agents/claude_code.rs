//! Claude Code. Confidence: verified.
//!
//! Config: `~/.claude/settings.json`. Mechanism: a `PreToolUse` hook, exactly
//! as `docs/ADAPTERS.md` specifies:
//!
//! ```json
//! {"hooks": {"PreToolUse": [{"matcher": "Bash|Read|Glob|Grep",
//!  "hooks": [{"type": "command", "command": "lessr hook claude"}]}]}}
//! ```
//!
//! The file is the user's, not ours: it is parsed with key order preserved, the
//! one entry is added, and everything else is written back exactly as it came
//! in. An existing `rtk` hook is chained behind, never rewritten.
//!
//! On the hook path Claude Code's contract is the plain one: the same JSON
//! back, with `tool_response.stdout` replaced.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use lessr_core::{Command, ToolKind, ToolResult};
use serde_json::{Value, json};

use crate::agent::AgentId;
use crate::agents::matcher_groups::{Layout, Patch};
use crate::agents::{Adapter, Confidence};
use crate::backup;
use crate::detect::Detected;
use crate::error::Result;
use crate::file;
use crate::hook::{Parsed, Slot, first_slot};
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
/// The four tools whose output is worth filtering are the four the free stages
/// know how to shrink; anything else would pay a process spawn for nothing.
const LAYOUT: Layout = Layout {
    event: "PreToolUse",
    matcher: "Bash|Read|Glob|Grep",
};

/// Where the filterable text lives in a `PostToolUse` payload: `stdout` when
/// the response is an object, the response itself when it is a plain string.
const CONTENT: &[Slot] = &[
    Slot::at(&["tool_response", "stdout"]),
    Slot::at(&["tool_response"]),
];

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
        Patch::Added => format!("run `{hook_command}` before {} tools", LAYOUT.matcher),
    }
}

/// Read a hook payload into something a stage can filter.
///
/// Claude Code sends `hook_event_name`, `tool_name`, `tool_input`, and on
/// `PostToolUse` also `tool_response`. We key off `tool_response` rather than
/// the event name: no response means nothing has run yet, and there is nothing
/// to filter whatever the event is called.
fn read_payload(root: &Value) -> Option<Parsed> {
    let (slot, text) = first_slot(root, CONTENT)?;

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
        slot,
    ))
}

/// Claude Code's tool names, mapped to the kinds stages match on.
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
        let entry = &written["hooks"]["PreToolUse"][0];
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
    fn an_rtk_hook_is_chained_not_replaced() {
        let (_tmp, paths) = setup();
        write_settings(
            &paths,
            r#"{
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
"#,
        );

        let plan = init(&paths);
        assert!(plan.render().contains("chain"), "{}", plan.render());
        apply(&plan).unwrap();

        let written: Value = serde_json::from_str(&settings(&paths)).unwrap();
        let groups = written["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "chained inside rtk's group, not beside it");
        assert_eq!(groups[0]["matcher"], "Bash", "rtk's matcher is left alone");

        let chain = groups[0]["hooks"].as_array().unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0]["command"], "/opt/homebrew/bin/rtk hook claude");
        assert_eq!(chain[1]["command"], HOOK, "rtk first, then Lessr");

        let found = ADAPTER.detect(&paths);
        assert!(found.rtk_present);
        assert!(found.already_patched);
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
      {"matcher": "Bash", "hooks": [
        {"type": "command", "command": "rtk hook claude"},
        {"type": "command", "command": "lessr hook claude"}
      ]}
    ],
    "PostToolUse": [
      {"matcher": "Edit", "hooks": [{"type": "command", "command": "prettier"}]}
    ]
  }
}
"#,
        );

        apply(&uninstall(&paths)).unwrap();

        let written: Value = serde_json::from_str(&settings(&paths)).unwrap();
        let chain = written["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0]["command"], "rtk hook claude");
        assert_eq!(
            written["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "prettier"
        );
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

        write_settings(&paths, "{\"hooks\": {\"PreToolUse\": {}}}");
        let err = crate::plan_init(&paths, AgentId::ClaudeCode, HOOK).unwrap_err();
        assert!(err.to_string().contains("PreToolUse"), "{err}");
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

        assert!(payload.result().is_none(), "no tool_response yet");
        assert_eq!(
            render_hook_output(AgentId::ClaudeCode, &payload).unwrap(),
            input,
            "a PreToolUse payload goes back exactly as it arrived"
        );
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
    fn a_rewritten_result_goes_back_into_the_response() {
        let mut payload =
            parse_hook_input(AgentId::ClaudeCode, &fixture("claude_post_tool_use.json")).unwrap();
        payload.result_mut().unwrap().content = Bytes::from_static(b"test result: ok. 3 passed\n");

        let out = render_hook_output(AgentId::ClaudeCode, &payload).unwrap();
        let written: Value = serde_json::from_slice(&out).unwrap();

        assert_eq!(
            written["tool_response"]["stdout"],
            "test result: ok. 3 passed\n"
        );
        assert_eq!(
            written["tool_response"]["stderr"], "",
            "the rest is untouched"
        );
        assert_eq!(written["tool_response"]["interrupted"], false);
        assert_eq!(written["tool_name"], "Bash");
        assert_eq!(written["tool_input"]["command"], "cargo test --workspace");
        assert_eq!(written["hook_event_name"], "PostToolUse");
    }

    #[test]
    fn a_response_that_is_just_a_string_is_filterable_too() {
        let input =
            br#"{"tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_response": "a\nb\n"}"#;
        let mut payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        assert_eq!(payload.result().unwrap().content.as_ref(), b"a\nb\n");

        payload.result_mut().unwrap().content = Bytes::from_static(b"a\n");
        let out = render_hook_output(AgentId::ClaudeCode, &payload).unwrap();
        let written: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(written["tool_response"], "a\n");
    }

    #[test]
    fn a_response_shape_we_do_not_read_leaves_the_payload_alone() {
        let input = br#"{"tool_name": "Read", "tool_response": {"file": {"content": "x"}}}"#;
        let payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        assert!(payload.result().is_none());
        assert_eq!(
            render_hook_output(AgentId::ClaudeCode, &payload).unwrap(),
            input.to_vec()
        );
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
