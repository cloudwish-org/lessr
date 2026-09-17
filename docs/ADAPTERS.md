# Adapters

`lessr init` detects installed agents, backs up their configs, and installs a pre-tool hook or a `base_url`.

| Agent | Mechanism | Config touched |
| --- | --- | --- |
| Claude Code | `PreToolUse` hook → `lessr hook claude` | `~/.claude/settings.json` |
| Cursor | hook → `lessr hook cursor` | Cursor hooks config |
| Windsurf, Cline, Roo, Kilo | rules file + optional hook | agent rules file |
| Codex (OpenAI) | hook → `lessr hook codex` | Codex config |
| Gemini CLI | hook → `lessr hook gemini` | Gemini hooks |
| OpenCode | plugin `lessr.ts` | `~/.config/opencode/plugins/` |
| pi, omp, Rakazo | pi harness adapter | pi config; Rakazo bots inherit |
| Any SDK or custom agent | `base_url = http://127.0.0.1:7433` | your code |

## Rules

- Back up before patching; `lessr uninstall` restores byte-for-byte.
- Detect an existing `rtk` hook and chain it (`rtk` first, then Lessr). Report it in `lessr init --show`.
- Never write to an agent config without confirmation, except `--yes` for CI.
- `lessr init --show` prints every change it would make and exits.

## Hook contract

Input: the agent's tool-call JSON on stdin. Output: the same JSON with the tool result replaced, on stdout, exit 0. Any internal error → original JSON, exit 0, error logged. A hook must never break the agent.
