# Mechanisms

Each mechanism is one crate implementing `lessr_core::Stage`. This file is the source of truth for what each does and how its saving is counted. No counting rule here means no claimed saving.

## Free engine

### detect — cache-break detector (report only)
- Path: proxy, `on_request` and `on_usage`.
- Fingerprints the cacheable prefix (system prompt, tool definitions, leading messages) with blake3 per turn; on change, finds the first differing byte and classifies the cause: ISO/epoch timestamp, UUID/random id, tool-order change, message reorder, unknown.
- Counted: `cache_creation_input_tokens` on turns where the prefix changed, at the provider's write rate. Exact.
- Reports only. Never mutates a request in this repo.

### gate — tool-output gate
- Path: hook, `on_tool_result`.
- Hot filters in Rust for `git status|diff|log`, `cargo test|build`, `pytest`, `npm|pnpm test`, `ls|tree`; declarative pack rules for the rest.
- Strips ANSI, progress redraws, repeated lines (kept as `×N`), passing test lines (counts kept), `node_modules` subtrees, log timestamps.
- After every filter, `guard::errors_preserved` checks that each input line matching the error patterns is present in the output; on failure the original bytes are returned and the event logged.
- Counted: bytes removed, exact; tokens estimated at bytes/4 with a per-provider correction learned from observed usage. Marked as estimate.

### trap — trap list (inside gate)
- Lockfiles, minified assets, sourcemaps, base64 blobs, generated code, JSON over 64 KB, binaries.
- Returns a structural summary (size, lines, top-level keys or package count) and a handle. A read with an explicit range or grep term is never trapped.

### dedup — re-read dedup
- Path: hook, `on_tool_result` for read-like tools.
- Keys on absolute path; stores blake3 and turn. Unchanged: `unchanged since turn N (hash)`. Changed: unified diff from last seen content, plus a handle to the full file.
- Counted: full content bytes − reply bytes. Exact.

### receipt
- Records every `Usage` and `Saving`; `lessr gain` prints per-session and per-day totals, savings by mechanism, and the cache-break diagnosis.

## Pro engine
cachefix, keepalive, compact, mcp, zoom, delta, editverify, batch, prefix, rent, router — see PRO.md. Their counting rules live in the Pro repository's `docs/MECHANISMS_PRO.md`.

## Token estimation

Provider usage fields are the truth. Where a mechanism must estimate (bytes removed before the request is sent), it uses bytes/4 with a per-provider correction from observed `(bytes sent, input_tokens)` pairs. The receipt marks estimates. Never report an estimate as exact.
