//! OpenCode. Confidence: verified.
//!
//! The odd one out: OpenCode loads TypeScript plugins rather than running a
//! command, so installing means writing a file Lessr owns —
//! `~/.config/opencode/plugin/lessr.ts` — and uninstalling means deleting it
//! again. The loader globs `{plugin,plugins}/*.{ts,js}`, so either spelling
//! works; the singular is what we write.
//!
//! The directory is `~/.config/opencode` on macOS, Linux and Windows alike:
//! OpenCode resolves it with `xdg-basedir` and has no platform branch. It does
//! honour an `OPENCODE_CONFIG_DIR` override, which this adapter does not read —
//! see the note in the module source.
//!
//! Two traps the generated plugin is written around:
//!
//! * In the legacy export style the loader walks everything a module exports
//!   and throws if any of it is not a function. The file therefore exports one
//!   function and nothing else; its constants stay unexported.
//! * The hook that can change what the model sees is `tool.execute.after`,
//!   which is handed the output object to mutate in place.
//!
//! The plugin talks to `lessr hook opencode` in a shape this crate defines on
//! both sides: `{"input": …, "output": …}` in, the same document back with
//! `output.output` replaced. Nothing about it leaves the machine.

use std::path::PathBuf;

use bytes::Bytes;
use lessr_core::{ToolKind, ToolResult};
use serde_json::Value;

use crate::agent::AgentId;
use crate::agents::{Adapter, Confidence, MARKER, claude_code};
use crate::detect::Detected;
use crate::error::Result;
use crate::file;
use crate::hook::{Parsed, Slot, first_slot};
use crate::paths::Paths;
use crate::plan::Plan;

/// See the module docs.
pub(crate) static ADAPTER: OpenCode = OpenCode;

/// The OpenCode adapter.
pub(crate) struct OpenCode;

/// The plugin we write, home-relative.
///
/// `OPENCODE_CONFIG_DIR` can move this, and we do not read it: an adapter that
/// consulted the environment could not be tested hermetically, and the cost of
/// ignoring it is a plugin that sits unread in the default directory rather
/// than a config that breaks. `lessr init --show` prints the path it will use,
/// so a user who has moved theirs can see it.
const PLUGIN: &str = ".config/opencode/plugin/lessr.ts";

/// The directories that say OpenCode is on this machine.
const PROBES: &[&str] = &[".config/opencode", ".opencode"];

/// Where the filterable text sits in the payload the plugin sends.
const CONTENT: &[Slot] = &[Slot::at(&["output", "output"])];

impl Adapter for OpenCode {
    fn id(&self) -> AgentId {
        AgentId::OpenCode
    }

    fn confidence(&self) -> Confidence {
        Confidence::Verified
    }

    fn detect(&self, paths: &Paths) -> Detected {
        let plugin = super::home_join(paths, PLUGIN);
        let installed = PROBES
            .iter()
            .any(|probe| super::home_join(paths, probe).exists());
        let ours = file::read(&plugin)
            .ok()
            .flatten()
            .is_some_and(|text| text.contains(MARKER));
        Detected {
            installed: installed || plugin.exists(),
            config_path: plugin.exists().then_some(plugin),
            already_patched: ours,
            // A plugin is a file of its own; rtk cannot be sharing it.
            rtk_present: false,
            ..Detected::absent(AgentId::OpenCode)
        }
    }

    fn plan_init(&self, paths: &Paths, hook_command: &str) -> Result<Plan> {
        super::plan_owned_file(
            paths,
            AgentId::OpenCode,
            super::home_join(paths, PLUGIN),
            plugin_source(hook_command),
            "an OpenCode plugin that filters tool output through Lessr",
        )
    }

    fn plan_uninstall(&self, paths: &Paths) -> Result<Plan> {
        super::plan_remove_owned_file(paths, AgentId::OpenCode, super::home_join(paths, PLUGIN))
    }

    fn parse_hook(&self, root: &Value) -> Result<Option<Parsed>> {
        Ok(read_payload(root))
    }
}

/// The plugin, with `hook_command` baked in.
///
/// The command is embedded as a JSON string literal, which is also a valid
/// TypeScript one, so a path with a space or a quote in it cannot break the
/// file it is written into.
fn plugin_source(hook_command: &str) -> String {
    let command = serde_json::to_string(hook_command).unwrap_or_else(|_| "\"lessr\"".to_string());
    format!(
        r#"// {MARKER}
// Safe to delete: `lessr init` writes it again, `lessr uninstall` removes it.
//
// OpenCode's legacy loader calls everything a plugin module exports, so this
// file exports one function and nothing else. Everything it runs is local;
// nothing here opens a socket.
import {{ spawnSync }} from "node:child_process";

const COMMAND = {command};

export const server = async () => ({{
  "tool.execute.after": async (input, output) => {{
    try {{
      const run = spawnSync(COMMAND, {{
        input: JSON.stringify({{ input, output }}),
        encoding: "utf8",
        shell: true,
        maxBuffer: 256 * 1024 * 1024,
      }});
      if (run.status !== 0 || !run.stdout) return;
      const filtered = JSON.parse(run.stdout);
      if (typeof filtered?.output?.output === "string") {{
        // Replace only what the model reads; the rest of the call is the
        // tool's own business.
        output.output = filtered.output.output;
      }}
    }} catch {{
      // A hook must never break the agent: leave the output as it was.
    }}
  }},
}});
"#
    )
}

/// Read the payload the plugin sends.
fn read_payload(root: &Value) -> Option<Parsed> {
    let (slot, text) = first_slot(root, CONTENT)?;
    let tool_name = Slot::at(&["input", "tool"]).get(root).unwrap_or("");
    let tool = tool_kind(tool_name);
    let args = root.get("input").and_then(|input| input.get("args"));

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
                .and_then(|args| args.get("filePath").or_else(|| args.get("path")))
                .and_then(Value::as_str)
                .map(PathBuf::from),
            explicit_selection: explicit_selection(tool, args),
            content: Bytes::copy_from_slice(text.as_bytes()),
        },
        slot,
    ))
}

/// OpenCode's tool names, mapped to the kinds stages match on.
fn tool_kind(tool_name: &str) -> ToolKind {
    match tool_name {
        "bash" => ToolKind::Shell,
        "read" => ToolKind::Read,
        "grep" | "glob" | "list" => ToolKind::Search,
        "edit" | "write" | "patch" => ToolKind::Edit,
        _ => ToolKind::Other,
    }
}

/// Loop-safety rule 1: a read with a range, or a search with a pattern, is the
/// agent naming what it wants.
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
    use crate::plan::{Change, apply};

    const HOOK: &str = "lessr hook opencode";

    fn setup() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("home"), tmp.path().join("config"));
        (tmp, paths)
    }

    fn plugin(paths: &Paths) -> PathBuf {
        super::super::home_join(paths, PLUGIN)
    }

    #[test]
    fn the_plugin_lands_in_the_singular_plugin_directory() {
        let (_tmp, paths) = setup();
        let plan = crate::plan_init(&paths, AgentId::OpenCode, HOOK).unwrap();
        apply(&plan).unwrap();

        let path = plugin(&paths);
        assert!(
            path.ends_with(".config/opencode/plugin/lessr.ts"),
            "{path:?}"
        );
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(source.contains(MARKER), "it has to be recognisable as ours");
        assert!(source.contains("\"lessr hook opencode\""), "{source}");
        assert!(source.contains("tool.execute.after"), "{source}");
    }

    #[test]
    fn the_plugin_exports_one_function_and_nothing_else() {
        // OpenCode's legacy loader calls everything a module exports; a
        // non-function export makes it throw and takes the agent with it.
        let source = plugin_source(HOOK);
        let exports: Vec<&str> = source
            .lines()
            .filter(|line| line.starts_with("export"))
            .collect();
        assert_eq!(exports.len(), 1, "{exports:?}");
        assert!(
            exports[0].starts_with("export const server = async () =>"),
            "{exports:?}"
        );
    }

    #[test]
    fn a_command_with_a_space_in_its_path_is_embedded_safely() {
        let source = plugin_source("/opt/my tools/lessr hook opencode");
        assert!(
            source.contains(r#"const COMMAND = "/opt/my tools/lessr hook opencode";"#),
            "{source}"
        );
    }

    #[test]
    fn uninstalling_removes_the_plugin_we_wrote() {
        let (_tmp, paths) = setup();
        apply(&crate::plan_init(&paths, AgentId::OpenCode, HOOK).unwrap()).unwrap();
        assert!(plugin(&paths).exists());

        let undo = crate::plan_uninstall(&paths, AgentId::OpenCode).unwrap();
        assert!(matches!(undo.changes.as_slice(), [Change::Delete { .. }]));
        apply(&undo).unwrap();
        assert!(!plugin(&paths).exists());

        // Twice is allowed.
        assert!(
            crate::plan_uninstall(&paths, AgentId::OpenCode)
                .unwrap()
                .is_noop()
        );
    }

    #[test]
    fn a_plugin_we_did_not_write_is_backed_up_and_then_restored() {
        let (_tmp, paths) = setup();
        let path = plugin(&paths);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let theirs = "export const server = async () => ({});\n";
        std::fs::write(&path, theirs).unwrap();

        let plan = crate::plan_init(&paths, AgentId::OpenCode, HOOK).unwrap();
        assert!(matches!(plan.changes.first(), Some(Change::Backup { .. })));
        apply(&plan).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(MARKER));

        apply(&crate::plan_uninstall(&paths, AgentId::OpenCode).unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), theirs);
    }

    #[test]
    fn the_payload_the_plugin_sends_round_trips() {
        let input = br#"{"input": {"tool": "bash", "args": {"command": "/usr/bin/cargo test"}}, "output": {"title": "cargo test", "output": "long\noutput\n", "metadata": {"exit": 0}}}"#;
        let mut payload = parse_hook_input(AgentId::OpenCode, input).unwrap();

        let result = payload.result().unwrap();
        assert_eq!(result.tool, ToolKind::Shell);
        assert_eq!(result.tool_name, "bash");
        assert_eq!(result.command.as_ref().unwrap().program, "cargo");
        assert_eq!(result.content.as_ref(), b"long\noutput\n");

        payload.result_mut().unwrap().content = Bytes::from_static(b"short\n");
        let out = render_hook_output(AgentId::OpenCode, &payload).unwrap();
        let written: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(written["output"]["output"], "short\n");
        assert_eq!(
            written["output"]["title"], "cargo test",
            "the rest is untouched"
        );
        assert_eq!(written["output"]["metadata"]["exit"], 0);
        assert_eq!(written["input"]["tool"], "bash");
    }

    #[test]
    fn a_read_with_a_range_is_an_explicit_selection() {
        let input = br#"{"input": {"tool": "read", "args": {"filePath": "/src/lib.rs", "offset": 10}}, "output": {"output": "x"}}"#;
        let payload = parse_hook_input(AgentId::OpenCode, input).unwrap();
        let result = payload.result().unwrap();
        assert_eq!(result.tool, ToolKind::Read);
        assert_eq!(result.path, Some(PathBuf::from("/src/lib.rs")));
        assert!(result.explicit_selection);
    }
}
