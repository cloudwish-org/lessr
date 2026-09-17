# lessr-adapters

Per-agent config knowledge: where an agent keeps its settings, how a Lessr hook
gets into them, and how it comes back out. `lessr init` plans, prints and
applies; `lessr uninstall` restores byte-for-byte from `<config>/backups/`.

Every agent in `../../docs/ADAPTERS.md` has an adapter, and every adapter states
its confidence. **Verified** adapters write the agent's config: Claude Code
(`PostToolUse` hook in `~/.claude/settings.json`), Codex (a marker-delimited
block in `~/.codex/config.toml`), Gemini CLI (`hooks.AfterTool` in
`~/.gemini/settings.json`), OpenCode (`~/.config/opencode/plugin/lessr.ts`) and
pi (`~/.pi/agent/extensions/lessr.ts`). **Unverified** adapters — Cursor,
Windsurf, Cline, Roo, Kilo, omp, Rakazo and the generic `base_url` — print what
to do and write nothing, because a hook entry in the wrong shape does not fail
once, it fails on every tool call afterwards.

An existing `rtk` hook is reported and never rewritten: Lessr goes beside it, or
behind it where they share an event.

The hook contract is per agent, not universal: Claude Code takes a
`hookSpecificOutput` envelope, Gemini CLI a decision document, pi a patch, and
the OpenCode plugin the document it sent. All of them read silence as "no
change", which is what any internal error produces — exit 0, always. A hook must
never break the agent.

See ../../docs/ARCHITECTURE.md and ../../docs/ADAPTERS.md for the contracts this
crate implements.
