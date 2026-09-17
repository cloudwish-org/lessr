//! The hook contract: the agent's tool-call JSON in, whatever that agent
//! expects on stdout, out.
//!
//! `docs/ADAPTERS.md`: "Input: the agent's tool-call JSON on stdin. Output: the
//! same JSON with the tool result replaced, on stdout, exit 0. Any internal
//! error → original JSON, exit 0, error logged. A hook must never break the
//! agent."
//!
//! "The same JSON back" is Claude Code's contract, and OpenCode's, because we
//! wrote the OpenCode side too. Others want a small decision document instead,
//! so the shape of the output belongs to the adapter — see
//! [`crate::agents::Adapter::render_hook`]. What every agent shares is the rule
//! underneath: when there is nothing to filter, say nothing new.
//!
//! That is why [`HookPayload`] keeps the bytes it was given as well as the
//! parsed form: an unchanged payload goes back exactly as it arrived rather
//! than re-serialised, so nothing about the agent's own JSON changes on the way
//! through.

use lessr_core::ToolResult;
use serde_json::Value;

use crate::agent::AgentId;
use crate::agents;
use crate::error::{Error, Result};

/// Where in the agent's JSON the filterable text sits: the object keys to walk
/// from the root.
///
/// A path rather than a variant per agent, because every agent so far keeps its
/// tool output in a string a few keys down, and the only thing that differs is
/// which keys. An empty path is the root itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Slot(&'static [&'static str]);

impl Slot {
    /// A slot at this chain of keys.
    pub(crate) const fn at(path: &'static [&'static str]) -> Slot {
        Slot(path)
    }

    /// The string this slot points at, if there is one there.
    pub(crate) fn get(self, root: &Value) -> Option<&str> {
        let mut node = root;
        for key in self.0 {
            node = node.get(key)?;
        }
        node.as_str()
    }

    /// Replace the string this slot points at.
    ///
    /// A path that no longer resolves leaves the document alone: the caller is
    /// substituting into a clone of what it parsed, so this cannot happen
    /// without a bug, and a bug here should lose a saving, not a payload.
    pub(crate) fn put(self, root: &mut Value, content: &str) {
        let mut node = root;
        for key in self.0 {
            let Some(next) = node.get_mut(key) else {
                return;
            };
            node = next;
        }
        *node = Value::String(content.to_string());
    }
}

/// The first of `candidates` that holds a string, and what it holds.
///
/// Adapters read defensively: an agent whose payload shape we have not pinned
/// down gets a list of the places its output could be, and finding none means
/// there is nothing to filter — never an error. A hook that refuses a payload
/// it does not recognise is a hook that breaks the agent.
pub(crate) fn first_slot<'a>(root: &'a Value, candidates: &[Slot]) -> Option<(Slot, &'a str)> {
    candidates
        .iter()
        .find_map(|&slot| slot.get(root).map(|text| (slot, text)))
}

/// A parsed hook payload: the result a stage can filter, and where it sits.
pub(crate) type Parsed = (ToolResult, Slot);

/// One tool call, as the agent sent it.
#[derive(Clone, Debug)]
pub struct HookPayload {
    /// The agent whose shape this was parsed as.
    agent: AgentId,
    /// Exactly what arrived on stdin.
    raw: Vec<u8>,
    /// The same bytes parsed, with key order preserved.
    json: Value,
    /// Where [`HookPayload::result`]'s content came from, if there is any.
    slot: Option<Slot>,
    /// What the pipeline may rewrite.
    result: Option<ToolResult>,
}

impl HookPayload {
    /// The tool result, when the payload carries one.
    ///
    /// `None` for a payload with nothing to filter: a `PreToolUse` call, which
    /// has not run yet, or a response shape this adapter does not read.
    pub fn result(&self) -> Option<&ToolResult> {
        self.result.as_ref()
    }

    /// The tool result, for the pipeline to rewrite in place.
    pub fn result_mut(&mut self) -> Option<&mut ToolResult> {
        self.result.as_mut()
    }
}

/// Parse an agent's hook JSON.
///
/// `Ok` even when there is nothing to filter. The errors here are the cases
/// where we were not given a hook payload at all — it is not JSON, or it is not
/// an object — plus agents that have no hook.
pub fn parse_hook_input(agent: AgentId, input: &[u8]) -> Result<HookPayload> {
    let json: Value = serde_json::from_slice(input).map_err(Error::HookJson)?;
    if !json.is_object() {
        return Err(Error::HookShape);
    }
    let (result, slot) = match agents::adapter(agent).parse_hook(&json)? {
        Some((result, slot)) => (Some(result), Some(slot)),
        None => (None, None),
    };
    Ok(HookPayload {
        agent,
        raw: input.to_vec(),
        json,
        slot,
        result,
    })
}

/// Render the payload back into what the agent expects on stdout, with the
/// (possibly rewritten) result substituted in.
///
/// With nothing to substitute, the agent gets whatever it reads as "no
/// opinion": its own payload back for the agents whose contract is an echo,
/// and nothing at all for the agents that read stdout as a decision.
pub fn render_hook_output(agent: AgentId, payload: &HookPayload) -> Result<Vec<u8>> {
    if agent != payload.agent {
        return Err(Error::AgentMismatch {
            parsed: payload.agent.display_name(),
            rendered: agent.display_name(),
        });
    }
    let adapter = agents::adapter(agent);
    let (Some(result), Some(slot)) = (payload.result.as_ref(), payload.slot) else {
        return Ok(adapter.render_unchanged(&payload.raw));
    };
    // A filter that cut mid-character would otherwise put invalid text into the
    // agent's context; the caller falls back to the original bytes instead.
    let content = std::str::from_utf8(&result.content).map_err(|_| Error::NonUtf8)?;
    adapter.render_hook(&payload.json, slot, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_that_is_not_json_is_an_error_not_a_panic() {
        let err = parse_hook_input(AgentId::ClaudeCode, b"{\"tool_name\":").unwrap_err();
        assert!(err.to_string().contains("not valid JSON"), "{err}");
    }

    #[test]
    fn input_that_is_not_an_object_is_an_error() {
        assert!(parse_hook_input(AgentId::ClaudeCode, b"[1, 2]").is_err());
        assert!(parse_hook_input(AgentId::ClaudeCode, b"null").is_err());
    }

    #[test]
    fn an_agent_without_a_hook_says_so() {
        let err = parse_hook_input(AgentId::Generic, b"{}").unwrap_err();
        assert!(err.to_string().contains("base_url"), "{err}");
    }

    #[test]
    fn rendering_under_the_wrong_agent_is_refused() {
        let payload = parse_hook_input(AgentId::ClaudeCode, b"{}").unwrap();
        assert!(render_hook_output(AgentId::Cursor, &payload).is_err());
    }

    #[test]
    fn a_payload_with_nothing_to_filter_renders_byte_for_byte() {
        let input = b"{\n  \"hook_event_name\": \"PreToolUse\",\n  \"tool_name\": \"Bash\"\n}\n";
        let payload = parse_hook_input(AgentId::ClaudeCode, input).unwrap();
        assert!(payload.result().is_none());
        assert_eq!(
            render_hook_output(AgentId::ClaudeCode, &payload).unwrap(),
            input.to_vec()
        );
    }

    #[test]
    fn a_slot_walks_keys_and_only_reads_strings() {
        let doc = serde_json::json!({"a": {"b": "text", "c": 3}});
        assert_eq!(Slot::at(&["a", "b"]).get(&doc), Some("text"));
        assert_eq!(Slot::at(&["a", "c"]).get(&doc), None);
        assert_eq!(Slot::at(&["a", "z"]).get(&doc), None);

        let mut doc = doc;
        Slot::at(&["a", "b"]).put(&mut doc, "other");
        assert_eq!(doc["a"]["b"], "other");
        Slot::at(&["a", "z", "y"]).put(&mut doc, "ignored");
        assert_eq!(doc["a"]["b"], "other");
    }

    #[test]
    fn the_first_slot_that_holds_a_string_wins() {
        let doc = serde_json::json!({"output": {"output": "x"}});
        let candidates = [Slot::at(&["stdout"]), Slot::at(&["output", "output"])];
        let (slot, text) = first_slot(&doc, &candidates).unwrap();
        assert_eq!(text, "x");
        assert_eq!(slot, Slot::at(&["output", "output"]));
        assert!(first_slot(&doc, &[Slot::at(&["nope"])]).is_none());
    }
}
