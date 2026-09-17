# lessr-core

The extension point. Defines Stage (on_tool_result, on_request, on_usage), Pipeline with Position insertion and the 2 ms soft / 10 ms hard Budget, HandleStore (append-only session file, 4-hex ids from blake3), the token Estimator (bytes/4 with per-provider correction), and shared types (Tier, Mode, ToolKind, ToolResult, Request, Usage, Saving). No I/O on the hot path, no workspace dependencies.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
