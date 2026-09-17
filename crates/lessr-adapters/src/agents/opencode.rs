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
