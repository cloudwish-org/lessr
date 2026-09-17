//! The pi harness. Confidence: verified.
//!
//! Config: `~/.pi/agent/extensions/lessr.ts`, a TypeScript extension with a
//! default-exported function that is handed pi's `ExtensionAPI`. The hook is
//! `pi.on("tool_result", …)`, whose handler returns a partial patch —
//! `{content, details, isError, usage}` — so unlike most harnesses this one can
//! genuinely replace a tool result.
//!
//! The extension talks to `lessr hook pi` in a shape this crate defines on both
//! sides: the event in, `{"content": "<text>"}` back, and nothing at all when
//! there is nothing to change. Only `content` is ever returned, because the
//! other three fields are the tool's own account of what happened.
//!
//! `docs/ADAPTERS.md` puts omp and Rakazo on this row too. omp runs on this
//! harness, so configuring pi configures omp; Rakazo turned out to be a server
//! deployment rather than a harness, and is a `base_url`.

use bytes::Bytes;
use lessr_core::{ToolKind, ToolResult};
use serde_json::{Value, json};

use crate::agent::AgentId;
use crate::agents::{Adapter, Confidence, MARKER};
use crate::detect::Detected;
use crate::error::{Error, Result};
use crate::file;
use crate::hook::{Parsed, Slot, first_slot};
use crate::paths::Paths;
use crate::plan::Plan;

/// See the module docs.
pub(crate) static ADAPTER: Pi = Pi;

/// The pi adapter.
pub(crate) struct Pi;

/// The extension we write, home-relative.
const EXTENSION: &str = ".pi/agent/extensions/lessr.ts";

/// The directories that say pi is on this machine.
const PROBES: &[&str] = &[".pi/agent/extensions", ".pi", ".config/pi"];

/// Where the filterable text might be in a `tool_result` event. The event's
/// own shape is pi's; ours is the envelope around it.
const CONTENT: &[Slot] = &[
    Slot::at(&["event", "content"]),
    Slot::at(&["event", "result", "content"]),
    Slot::at(&["event", "output"]),
    Slot::at(&["content"]),
];

impl Adapter for Pi {
    fn id(&self) -> AgentId {
        AgentId::Pi
    }

    fn confidence(&self) -> Confidence {
        Confidence::Verified
    }

    fn detect(&self, paths: &Paths) -> Detected {
        let extension = super::home_join(paths, EXTENSION);
        let installed = PROBES
            .iter()
            .any(|probe| super::home_join(paths, probe).exists());
        let ours = file::read(&extension)
            .ok()
            .flatten()
            .is_some_and(|text| text.contains(MARKER));
        Detected {
            installed: installed || extension.exists(),
            config_path: extension.exists().then_some(extension),
            already_patched: ours,
            rtk_present: false,
            ..Detected::absent(AgentId::Pi)
        }
    }

    fn plan_init(&self, paths: &Paths, hook_command: &str) -> Result<Plan> {
        super::plan_owned_file(
            paths,
            AgentId::Pi,
            super::home_join(paths, EXTENSION),
            extension_source(hook_command),
            "a pi extension that filters tool results through Lessr",
        )
    }

    fn plan_uninstall(&self, paths: &Paths) -> Result<Plan> {
        super::plan_remove_owned_file(paths, AgentId::Pi, super::home_join(paths, EXTENSION))
    }

    fn parse_hook(&self, root: &Value) -> Result<Option<Parsed>> {
        Ok(read_payload(root))
    }

    /// The handler returns a patch, so that is what we print: the one field we
    /// are qualified to change.
    fn render_hook(&self, _root: &Value, _slot: Slot, content: &str) -> Result<Vec<u8>> {
        serde_json::to_vec(&json!({"content": content})).map_err(Error::HookJson)
    }

    /// No patch at all, which the extension reads as "leave the result alone".
    fn render_unchanged(&self, _raw: &[u8]) -> Vec<u8> {
        Vec::new()
    }
}

/// The extension, with `hook_command` baked in as a JSON — and therefore
/// TypeScript — string literal.
fn extension_source(hook_command: &str) -> String {
    let command = serde_json::to_string(hook_command).unwrap_or_else(|_| "\"lessr\"".to_string());
    format!(
        r#"// {MARKER}
// Safe to delete: `lessr init` writes it again, `lessr uninstall` removes it.
//
// Everything this runs is local; nothing here opens a socket.
import {{ spawnSync }} from "node:child_process";

const COMMAND = {command};

export default function (pi) {{
  pi.on("tool_result", (event) => {{
    try {{
      const run = spawnSync(COMMAND, {{
        input: JSON.stringify({{ event }}),
        encoding: "utf8",
        shell: true,
        maxBuffer: 256 * 1024 * 1024,
      }});
      if (run.status !== 0 || !run.stdout) return undefined;
      const filtered = JSON.parse(run.stdout);
      if (typeof filtered?.content === "string") {{
        // Only the content: whether the call failed, and what it cost, are the
        // tool's own account of what happened.
        return {{ content: filtered.content }};
      }}
    }} catch {{
      // A hook must never break the agent: leave the result as it was.
    }}
    return undefined;
  }});
}}
"#
    )
}

/// Read the payload the extension sends.
fn read_payload(root: &Value) -> Option<Parsed> {
    let (slot, text) = first_slot(root, CONTENT)?;
    let event = root.get("event");
    let tool_name = event
        .and_then(|event| event.get("tool").or_else(|| event.get("name")))
        .and_then(Value::as_str)
        .unwrap_or("");

    Some((
        ToolResult {
            tool: tool_kind(tool_name),
            tool_name: tool_name.to_string(),
            // pi's event carries the result, not the call that produced it, so
            // there is no command or path to report until that is verified.
            command: None,
            path: None,
            explicit_selection: false,
            content: Bytes::copy_from_slice(text.as_bytes()),
        },
        slot,
    ))
}

/// pi's tool names are not pinned down, so everything that is not obviously a
/// shell is [`ToolKind::Other`], which stages leave alone.
fn tool_kind(tool_name: &str) -> ToolKind {
    match tool_name {
        "bash" | "shell" | "run" => ToolKind::Shell,
        "read" | "read_file" => ToolKind::Read,
        "grep" | "glob" | "search" => ToolKind::Search,
        "edit" | "write" => ToolKind::Edit,
        _ => ToolKind::Other,
    }
}
