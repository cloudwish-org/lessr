//! The hook path: the one command an agent calls, thousands of times a day.
//!
//! Two contracts meet here.
//!
//! `docs/ADAPTERS.md`: any internal error returns the original JSON and exits
//! 0. A hook must never break the agent, so every failure below degrades to a
//! passthrough instead of surfacing.
//!
//! `docs/PERFORMANCE.md`: no `println!` on this path; output is one
//! `write_all`. Nothing but the payload may ever reach stdout — whatever lands
//! there lands in the agent's context and is billed again on every later turn.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use lessr_adapters::AgentId;
use lessr_core::Pipeline;

/// Read stdin, filter, write stdout. Always exits 0.
pub fn run(agent: &str, pipeline: &mut Pipeline) -> ExitCode {
    let mut input = Vec::new();
    if io::stdin().read_to_end(&mut input).is_err() {
        // We never saw a payload, so there is nothing to hand back. Staying
        // silent is the only safe answer.
        return ExitCode::SUCCESS;
    }

    let output = filter_bytes(agent, &input, pipeline);

    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(&output);
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

/// Run the pipeline over one hook payload, falling back to the original bytes.
///
/// Separated from the I/O so the fallback behaviour is testable, because it is
/// the part that must not regress.
pub fn filter_bytes(agent: &str, input: &[u8], pipeline: &mut Pipeline) -> Vec<u8> {
    filtered(agent, input, pipeline).unwrap_or_else(|| input.to_vec())
}

/// The happy path. `None` at any step means "hand back what we were given".
fn filtered(agent: &str, input: &[u8], pipeline: &mut Pipeline) -> Option<Vec<u8>> {
    let agent = AgentId::parse(agent)?;
    let mut payload = lessr_adapters::parse_hook_input(agent, input).ok()?;

    // A payload with no tool result — a pre-tool-use event, say — has nothing
    // for the pipeline to do. Rendering it back unchanged is correct, not a
    // failure.
    if let Some(result) = payload.result_mut() {
        pipeline.run_tool_result(result);
    }

    lessr_adapters::render_hook_output(agent, &payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lessr_core::PipelineBuilder;

    fn empty_pipeline() -> Pipeline {
        PipelineBuilder::new().build()
    }

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/hook")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn an_unknown_agent_is_a_passthrough_not_an_error() {
        let input = br#"{"tool_name":"Bash"}"#;
        let out = filter_bytes("nosuchagent", input, &mut empty_pipeline());
        assert_eq!(out, input);
    }

    #[test]
    fn malformed_json_is_a_passthrough() {
        let input = b"this is not json at all {{{";
        let out = filter_bytes("claude", input, &mut empty_pipeline());
        assert_eq!(out, input);
    }

    #[test]
    fn empty_input_is_a_passthrough() {
        let out = filter_bytes("claude", b"", &mut empty_pipeline());
        assert_eq!(out, b"");
    }

    #[test]
    fn a_pre_tool_use_payload_comes_back_with_its_tool_call_intact() {
        let input = fixture("claude_pre_tool_use.json");
        let out = filter_bytes("claude", &input, &mut empty_pipeline());

        let parsed: serde_json::Value = serde_json::from_slice(&out).expect("still valid JSON");
        assert_eq!(parsed["tool_name"], "Bash");
        assert_eq!(parsed["tool_input"]["command"], "cargo test --workspace");
    }

    #[test]
    fn an_empty_pipeline_returns_the_tool_output_byte_for_byte() {
        let input = fixture("claude_post_tool_use.json");
        let before: serde_json::Value = serde_json::from_slice(&input).unwrap();

        let out = filter_bytes("claude", &input, &mut empty_pipeline());
        let after: serde_json::Value = serde_json::from_slice(&out).expect("still valid JSON");

        assert_eq!(
            before["tool_response"]["stdout"], after["tool_response"]["stdout"],
            "with no stages registered the gate must be invisible"
        );
    }

    #[test]
    fn nothing_about_pro_is_ever_written_to_the_hook_stream() {
        // Hook output is billed to the user on every later turn. This is the
        // structural guarantee behind that rule; see `crate::pro`.
        let input = fixture("claude_post_tool_use.json");
        let out = filter_bytes("claude", &input, &mut empty_pipeline());
        let text = String::from_utf8_lossy(&out).to_lowercase();
        for marketing in ["lessr.dev", "upgrade", "pro ", "left on the table"] {
            assert!(
                !text.contains(marketing),
                "hook output must not contain {marketing:?}"
            );
        }
    }
}
