//! Gemini CLI. Confidence: verified format, defensive reader.
//!
//! Config: `~/.gemini/settings.json` (a project's own `.gemini/settings.json`
//! wins, which is the user's business, not ours). Mechanism: the same nested
//! matcher-group shape Claude Code uses, under a `hooks` key:
//!
//! ```json
//! {"hooks": {"AfterTool": [{"matcher": "run_shell_command",
//!  "hooks": [{"name": "lessr", "type": "command",
//!             "command": "lessr hook gemini", "timeout": 5000}]}]}}
//! ```
//!
//! Two things differ from Claude Code, and both are load-bearing:
//!
//! * `timeout` is in **milliseconds** here.
//! * stdout is read as a decision, not as a replacement payload. Replacing a
//!   tool result means printing `{"decision": "deny", "reason": "<text>"}` and
//!   nothing else — so anything we want to say goes to stderr, and a call we
//!   are not changing prints nothing at all.
//!
//! What is *not* verified is the shape of the payload on stdin, so the reader
//! tries the places the text could be and treats "none of them" as "nothing to
//! filter". Guessing in that direction costs a saving; guessing in the writing
//! direction would cost the user their agent.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use lessr_core::{ToolKind, ToolResult};
use serde_json::{Value, json};

use crate::agent::AgentId;
use crate::agents::matcher_groups::{Layout, Patch};
use crate::agents::{Adapter, Confidence, claude_code};
use crate::backup;
use crate::detect::Detected;
use crate::error::{Error, Result};
use crate::file;
use crate::hook::{Parsed, Slot, first_slot};
use crate::paths::Paths;
use crate::plan::{Change, Plan};

/// See the module docs.
pub(crate) static ADAPTER: GeminiCli = GeminiCli;

/// The Gemini CLI adapter.
pub(crate) struct GeminiCli;

/// `~/.gemini/settings.json`, home-relative.
const SETTINGS: &str = ".gemini/settings.json";

/// The event that can replace a tool result, and the tools worth filtering.
///
/// `matcher` is a regex over the tool name. Only `run_shell_command` is
/// confirmed; the others are the names the CLI is documented to use, and a
/// name that turns out to be wrong simply never matches — it cannot break a
/// call.
const LAYOUT: Layout = Layout {
    event: "AfterTool",
    matcher: "run_shell_command|read_file|read_many_files|search_file_content|glob",
};

/// How long Gemini CLI waits for us, in milliseconds. The pipeline's own hard
/// budget is 10 ms per stage (`docs/PERFORMANCE.md`); five seconds is the
/// process, not the work.
const TIMEOUT_MS: u64 = 5000;

/// Where the tool's text might be. Tried in order; the first string wins.
const CONTENT: &[Slot] = &[
    Slot::at(&["tool_response", "stdout"]),
    Slot::at(&["tool_response", "output"]),
    Slot::at(&["tool_response"]),
    Slot::at(&["toolResponse"]),
    Slot::at(&["output"]),
    Slot::at(&["result"]),
    Slot::at(&["stdout"]),
];

/// Where the tool's name might be.
const TOOL_NAME: &[Slot] = &[
    Slot::at(&["tool_name"]),
    Slot::at(&["toolName"]),
    Slot::at(&["tool"]),
];

impl Adapter for GeminiCli {
    fn id(&self) -> AgentId {
        AgentId::GeminiCli
    }

    fn confidence(&self) -> Confidence {
        Confidence::Verified
    }

    fn detect(&self, paths: &Paths) -> Detected {
        let path = settings_path(paths);
        let mut found = Detected {
            installed: path.exists() || path.parent().is_some_and(Path::exists),
            ..Detected::absent(AgentId::GeminiCli)
        };
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

        let entry = json!({
            "name": "lessr",
            "type": "command",
            "command": hook_command,
            "timeout": TIMEOUT_MS,
        });
        let outcome = LAYOUT.patch(&path, &mut root, entry)?;
        if outcome == Patch::AlreadyThere {
            return Ok(Plan::nothing(
                AgentId::GeminiCli,
                format!("a Lessr hook is already in {}", path.display()),
            ));
        }

        let mut changes = Vec::with_capacity(2);
        if before.is_some() {
            changes.push(Change::Backup {
                from: path.clone(),
                to: backup::destination(paths, AgentId::GeminiCli, &path),
            });
        }
        let summary = if outcome == Patch::ChainedBehindRtk {
            format!("chain `{hook_command}` behind the existing rtk hook")
        } else {
            format!("run `{hook_command}` after {} tools", LAYOUT.matcher)
        };
        changes.push(Change::WriteJson {
            after: file::pretty(&root),
            summary,
            path,
            before,
        });
        Ok(Plan {
            agent: AgentId::GeminiCli,
            changes,
        })
    }

    fn plan_uninstall(&self, paths: &Paths) -> Result<Plan> {
        let records = backup::read_index(&backup::index_path(paths))?;
        if let Some(record) = backup::newest_for(&records, AgentId::GeminiCli) {
            return Ok(Plan {
                agent: AgentId::GeminiCli,
                changes: vec![Change::Restore {
                    from: record.stored_as.clone(),
                    to: record.original_path.clone(),
                }],
            });
        }

        let path = settings_path(paths);
        let Some(text) = file::read(&path)? else {
            return Ok(Plan::nothing(
                AgentId::GeminiCli,
                format!("no backup to restore and no {}", path.display()),
            ));
        };
        let mut root = file::parse(&path, &text)?;
        if !LAYOUT.unpatch(&mut root) {
            return Ok(Plan::nothing(
                AgentId::GeminiCli,
                format!(
                    "no backup to restore and no Lessr hook in {}",
                    path.display()
                ),
            ));
        }
        Ok(Plan {
            agent: AgentId::GeminiCli,
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

    /// `reason` is what the user sees in place of the tool's output, so the
    /// filtered text goes there and nothing else goes to stdout.
    fn render_hook(&self, _root: &Value, _slot: Slot, content: &str) -> Result<Vec<u8>> {
        serde_json::to_vec(&json!({"decision": "deny", "reason": content})).map_err(Error::HookJson)
    }

    /// Silence is how a hook that reads stdout as a decision says "no opinion".
    fn render_unchanged(&self, _raw: &[u8]) -> Vec<u8> {
        Vec::new()
    }
}

/// `~/.gemini/settings.json`.
fn settings_path(paths: &Paths) -> PathBuf {
    super::home_join(paths, SETTINGS)
}

/// Read a hook payload into something a stage can filter.
fn read_payload(root: &Value) -> Option<Parsed> {
    let (slot, text) = first_slot(root, CONTENT)?;
    let tool_name = first_slot(root, TOOL_NAME).map_or("", |(_, name)| name);
    let tool = tool_kind(tool_name);
    let args = root
        .get("tool_input")
        .or_else(|| root.get("args"))
        .or_else(|| root.get("toolArgs"));

    Some((
        ToolResult {
            tool,
            tool_name: tool_name.to_string(),
            command: if tool == ToolKind::Shell {
                args.and_then(|args| args.get("command"))
                    .and_then(Value::as_str)
                    .and_then(claude_code::parse_command)
            } else {
                None
            },
            path: args
                .and_then(|args| {
                    args.get("absolute_path")
                        .or_else(|| args.get("file_path"))
                        .or_else(|| args.get("path"))
                })
                .and_then(Value::as_str)
                .map(PathBuf::from),
            explicit_selection: explicit_selection(tool, args),
            content: Bytes::copy_from_slice(text.as_bytes()),
        },
        slot,
    ))
}

/// Gemini CLI's tool names, mapped to the kinds stages match on. An unknown
/// name is [`ToolKind::Other`], which stages leave alone.
fn tool_kind(tool_name: &str) -> ToolKind {
    match tool_name {
        "run_shell_command" => ToolKind::Shell,
        "read_file" | "read_many_files" => ToolKind::Read,
        "search_file_content" | "glob" => ToolKind::Search,
        "write_file" | "replace" => ToolKind::Edit,
        _ => ToolKind::Other,
    }
}

/// Loop-safety rule 1, as it applies here: a read with a range and a search
/// with a pattern are the agent naming what it wants.
fn explicit_selection(tool: ToolKind, args: Option<&Value>) -> bool {
    let Some(args) = args else {
        return false;
    };
    match tool {
        ToolKind::Read => {
            !args.get("offset").unwrap_or(&Value::Null).is_null()
                || !args.get("limit").unwrap_or(&Value::Null).is_null()
        }
        ToolKind::Search => args
            .get("pattern")
            .and_then(Value::as_str)
            .is_some_and(|pattern| !pattern.is_empty()),
        ToolKind::Shell | ToolKind::Edit | ToolKind::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::{parse_hook_input, render_hook_output};
    use crate::plan::apply;

    const HOOK: &str = "lessr hook gemini";

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

    fn settings(paths: &Paths) -> Value {
        serde_json::from_str(&std::fs::read_to_string(settings_path(paths)).unwrap()).unwrap()
    }

    #[test]
    fn the_entry_carries_a_name_and_a_timeout_in_milliseconds() {
        let (_tmp, paths) = setup();
        apply(&crate::plan_init(&paths, AgentId::GeminiCli, HOOK).unwrap()).unwrap();

        let group = &settings(&paths)["hooks"]["AfterTool"][0];
        assert_eq!(group["matcher"], LAYOUT.matcher);
        let entry = &group["hooks"][0];
        assert_eq!(entry["name"], "lessr");
        assert_eq!(entry["type"], "command");
        assert_eq!(entry["command"], HOOK);
        assert_eq!(entry["timeout"], 5000, "milliseconds, not seconds");
    }

    #[test]
    fn an_rtk_hook_is_chained_and_the_file_comes_back_byte_for_byte() {
        let (_tmp, paths) = setup();
        let original = "{\n\t\"theme\":\"dark\",\n  \"hooks\": {\"AfterTool\": [\n    {\"matcher\": \"run_shell_command\", \"hooks\": [{\"type\": \"command\", \"command\": \"rtk hook gemini\"}]}\n  ]}\n}";
        write_settings(&paths, original);

        apply(&crate::plan_init(&paths, AgentId::GeminiCli, HOOK).unwrap()).unwrap();
        let groups = settings(&paths)["hooks"]["AfterTool"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(groups.len(), 1, "chained inside rtk's group");
        assert_eq!(groups[0]["hooks"][0]["command"], "rtk hook gemini");
        assert_eq!(groups[0]["hooks"][1]["command"], HOOK);

        let found = ADAPTER.detect(&paths);
        assert!(found.rtk_present && found.already_patched);

        apply(&crate::plan_uninstall(&paths, AgentId::GeminiCli).unwrap()).unwrap();
        assert_eq!(
            std::fs::read(settings_path(&paths)).unwrap(),
            original.as_bytes()
        );
    }

    #[test]
    fn a_filtered_result_is_printed_as_a_decision_and_nothing_else() {
        let input = br#"{"tool_name": "run_shell_command", "tool_input": {"command": "/usr/bin/cargo test"}, "tool_response": {"stdout": "long output\n"}}"#;
        let mut payload = parse_hook_input(AgentId::GeminiCli, input).unwrap();

        let result = payload.result().unwrap();
        assert_eq!(result.tool, ToolKind::Shell);
        assert_eq!(result.command.as_ref().unwrap().program, "cargo");
        assert_eq!(result.content.as_ref(), b"long output\n");

        payload.result_mut().unwrap().content = Bytes::from_static(b"short\n");
        let out = render_hook_output(AgentId::GeminiCli, &payload).unwrap();
        let decision: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(decision["decision"], "deny");
        assert_eq!(decision["reason"], "short\n");
        assert_eq!(
            decision.as_object().unwrap().len(),
            2,
            "nothing else on stdout"
        );
    }

    #[test]
    fn a_payload_we_cannot_read_prints_nothing_at_all() {
        let payload =
            parse_hook_input(AgentId::GeminiCli, br#"{"tool_name": "web_fetch"}"#).unwrap();
        assert!(payload.result().is_none());
        assert!(
            render_hook_output(AgentId::GeminiCli, &payload)
                .unwrap()
                .is_empty(),
            "silence is how a decision hook says `no opinion`"
        );
    }

    #[test]
    fn the_reader_tries_the_places_the_output_could_be() {
        for input in [
            br#"{"tool_response": {"output": "x"}}"#.as_slice(),
            br#"{"tool_response": "x"}"#.as_slice(),
            br#"{"output": "x"}"#.as_slice(),
            br#"{"result": "x"}"#.as_slice(),
        ] {
            let payload = parse_hook_input(AgentId::GeminiCli, input).unwrap();
            assert_eq!(
                payload.result().map(|r| r.content.clone()),
                Some(Bytes::from_static(b"x")),
                "{}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn a_search_with_a_pattern_is_an_explicit_selection() {
        let input = br#"{"tool_name": "search_file_content", "args": {"pattern": "fn main"}, "tool_response": {"stdout": "x"}}"#;
        let payload = parse_hook_input(AgentId::GeminiCli, input).unwrap();
        let result = payload.result().unwrap();
        assert_eq!(result.tool, ToolKind::Search);
        assert!(result.explicit_selection, "loop-safety rule 1");
    }
}
