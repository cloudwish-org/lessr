//! The hook contract: the agent's tool-call JSON in, the same JSON out.
//!
//! `docs/ADAPTERS.md`: "Input: the agent's tool-call JSON on stdin. Output: the
//! same JSON with the tool result replaced, on stdout, exit 0. Any internal
//! error → original JSON, exit 0, error logged. A hook must never break the
//! agent."
//!
//! That is why [`HookPayload`] keeps the bytes it was given as well as the
//! parsed form. When there is nothing to filter — a `PreToolUse` payload has no
//! result yet — [`render_hook_output`] hands back the original bytes rather
//! than a re-serialised copy, so nothing about the agent's own JSON changes on
//! the way through.

use lessr_core::ToolResult;
use serde_json::Value;

use crate::agent::AgentId;
use crate::agents;
use crate::error::{Error, Result};

/// Where in the agent's JSON the filterable content came from, and therefore
/// where a rewritten version has to go back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Slot {
    /// `tool_response` is itself the text.
    Whole,
    /// `tool_response.stdout`.
    Stdout,
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
/// `Ok` even when there is nothing to filter: a hook that refuses a payload it
/// does not recognise is a hook that breaks the agent. The errors here are the
/// two cases where we were not given a hook payload at all — it is not JSON, or
/// it is not an object — plus agents that have no hook.
pub fn parse_hook_input(agent: AgentId, input: &[u8]) -> Result<HookPayload> {
    let json: Value = serde_json::from_slice(input).map_err(Error::HookJson)?;
    if !json.is_object() {
        return Err(Error::HookShape);
    }
    let parsed = agents::adapter(agent).parse_hook(&json)?;
    let (result, slot) = match parsed {
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

/// Render the payload back into the agent's JSON shape, with the (possibly
/// rewritten) result substituted in.
///
/// With nothing to substitute, the input is returned byte for byte. The output
/// is compact and carries no trailing newline: it is read by a program, and the
/// caller decides how to end its own stdout.
pub fn render_hook_output(agent: AgentId, payload: &HookPayload) -> Result<Vec<u8>> {
    if agent != payload.agent {
        return Err(Error::AgentMismatch {
            parsed: payload.agent.display_name(),
            rendered: agent.display_name(),
        });
    }
    let (Some(result), Some(slot)) = (payload.result.as_ref(), payload.slot) else {
        return Ok(payload.raw.clone());
    };
    // A filter that cut mid-character would otherwise put invalid text into the
    // agent's context; the caller falls back to the original bytes instead.
    let content = std::str::from_utf8(&result.content).map_err(|_| Error::NonUtf8)?;

    let mut json = payload.json.clone();
    agents::adapter(agent).substitute(&mut json, slot, content);
    serde_json::to_vec(&json).map_err(Error::HookJson)
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
}
