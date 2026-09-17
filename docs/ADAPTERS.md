# Adapters

`lessr init` detects installed agents, backs up their configs, and installs
whichever integration that agent can actually carry. Not every agent can carry
one; this table says so plainly rather than promising a hook that does nothing.

The gate shrinks tool **output**, so it needs a hook that can *replace* output.
Most agents' hooks only rewrite the tool *input*. That single fact decides every
row below.

| Agent | How the gate reaches it | What is touched |
| --- | --- | --- |
| Claude Code | **Hook** — `PostToolUse` → `lessr hook claude`, replacing output via `hookSpecificOutput.updatedToolOutput`. Needs 2.1.121 or newer. | `~/.claude/settings.json` |
| Gemini CLI | **Hook** — `AfterTool` → `lessr hook gemini`; the reply replaces the tool result | `~/.gemini/settings.json` |
| OpenCode | **Hook** — plugin `lessr.ts`, `tool.execute.after` mutates the output. Proxy also works. | `~/.config/opencode/plugin/` |
| pi, omp | **Hook** — extension `lessr.ts`, `tool_result` returns a patch. Proxy also works. | `~/.pi/agent/extensions/`; omp runs the pi harness |
| Codex (OpenAI) | **Proxy.** Its hooks can block a call or rewrite the input, never replace output, so a hook there costs a process spawn and saves nothing. Opt-in only. | `~/.codex/config.toml` |
| Cline | **Proxy.** Its hooks return cancel and context only. | custom provider base URL |
| Kilo Code | **Proxy.** No hook mechanism. | OpenAI-compatible base URL |
| Rakazo | **Proxy.** A self-hosted server, not a local agent: point its upstream at us. | `.env`, `RAKAZO_LOCAL_MODELS_URL` |
| Any SDK or custom agent | **Proxy** — `base_url = http://127.0.0.1:7433`. Any OpenAI-compatible upstream, OpenRouter included, is reached this way. | your code |
| Cursor | **Neither.** Its pre-hook only allows or denies, its post-hook replaces MCP output only, and its base-URL override is called from Cursor's servers, so localhost is unreachable. | — |
| Windsurf | **Neither.** Pre-hooks block, post-hooks are informational, no base URL. | — |
| Roo Code | **Neither** — reported discontinued. Verify before spending any work here. | — |

"Neither" means nothing on the developer's machine can see the tool output or
the API call. There is no gate, no dedup, no trap and no receipt, and the only
remaining lever would be a rules file asking the model to be brief — which is
the thing [VISION.md](VISION.md) exists to refuse. `lessr init` says "not
supported" and installs nothing.

## Rules

- Back up before patching; `lessr uninstall` restores byte-for-byte.
- **Never guess a format.** An adapter either knows an agent's real on-disk
  shape and patches it, or it prints instructions and writes nothing. A wrong
  hook entry does not fail once, it fails on every tool call afterwards. The
  rule is enforced in code: an unverified adapter that produces a write is an
  error, not a review comment.
- Detect an existing `rtk` hook and leave it exactly as it is. Add Lessr as a
  separate entry, never by wrapping theirs: rtk's own recogniser requires its
  entry to be exactly three tokens, so a compound command is invisible to it
  and its next `init` would re-add itself. On Claude Code the two do not even
  collide — rtk installs on `PreToolUse`, Lessr on `PostToolUse`.
- Never write to an agent config without confirmation, except `--yes` for CI.
- `lessr init --show` prints every change it would make and exits.

## Hook contract

Input: the agent's finished tool call as JSON on stdin. Output: **the
replacement, in whatever envelope that agent reads** — and nothing at all when
there is nothing to change. Always exit 0. Any internal error means the
original output stands. A hook must never break the agent.

The envelope is per agent, and echoing the input document back is not it:

| Agent | Replace output with | Unchanged |
| --- | --- | --- |
| Claude Code | `{"hookSpecificOutput":{"hookEventName":"PostToolUse","updatedToolOutput":{…}}}` | empty stdout |
| Gemini CLI | `{"decision":"deny","reason":"<replacement>"}` | empty stdout |
| OpenCode | the plugin mutates `output.output` in place | no mutation |
| pi | `{"content":"<replacement>"}` | empty stdout |

Claude Code **silently ignores** a value that does not match the tool's own
output shape and uses the original, so a wrong shape is indistinguishable from
success. Round-trip the whole object and replace only the text, and only for
tools whose shape is known from a real payload.
