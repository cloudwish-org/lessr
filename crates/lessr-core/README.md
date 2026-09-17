# lessr-core

The extension point. Defines Stage (configure, on_tool_result, on_request, on_usage), Pipeline with Position insertion, a StageConfig per stage and the 2 ms soft / 10 ms hard Budget, the config model every mechanism is tuned through (Mode, Level, Settings; layered by Config, compiled into the versioned Snapshot the hook path mmaps instead of parsing TOML), HandleStore (append-only session file, 4-hex ids from blake3), the token Estimator (bytes/4 with per-provider correction), shared types (Tier, ToolKind, ToolResult, Request, Usage) and the Saving the pipeline stamps with stage, tier, mode and level. No parse and no allocation on the hot path, no workspace dependencies.

See ../../docs/ARCHITECTURE.md, ../../docs/CONFIG.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
