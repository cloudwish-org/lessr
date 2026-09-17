@AGENTS.md

Claude Code specific notes:
- Use `cargo test -p <crate>` for focused runs; the full suite includes the overhead bench in `--release`.
- The `lessr hook claude` command in this repo is what Claude Code calls as a PreToolUse hook. When testing it, run it against `fixtures/hook/claude_pre_tool_use.json`.
