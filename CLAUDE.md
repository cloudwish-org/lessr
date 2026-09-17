@AGENTS.md

Claude Code specific notes:
- Use `cargo test -p <crate>` for focused runs; the full suite includes the overhead bench in `--release`.
- The `lessr hook claude` command in this repo is what Claude Code calls as a **PostToolUse** hook. When testing it, run it against `fixtures/hook/claude_post_tool_use.json` — that is the payload that carries `tool_response`, and so the only one with anything to filter. `claude_pre_tool_use.json` exists to prove the hook passes such a payload through untouched.
- Replacing output means emitting `{"hookSpecificOutput":{"hookEventName":"PostToolUse","updatedToolOutput":{…}}}`, and nothing at all when there is nothing to change. `updatedToolOutput` needs Claude Code 2.1.121 or newer. A value that does not match the tool's own output shape is **silently ignored** and the original is used, so a wrong shape looks exactly like success — round-trip the whole object and replace only the text.
