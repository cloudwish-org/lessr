# lessr-adapters

Per-agent config knowledge: Claude Code, Cursor, Windsurf, Cline/Roo/Kilo, Codex, Gemini CLI, OpenCode plugin, pi/omp/Rakazo, generic base_url. init backs up, patches, prints; uninstall restores byte-for-byte; existing rtk hooks are chained, not replaced. Hook contract: any internal error → original JSON, exit 0.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
