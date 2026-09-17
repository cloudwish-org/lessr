# Architecture

One binary, two entry paths, one pipeline. Everything runs on localhost; the only outbound request is the forwarded provider call.

```mermaid
flowchart LR
  A[Agent] -->|post-tool hook: tool result| H[lessr hook]
  A -->|base_url| P[lessr proxy]
  H --> PL[Pipeline: Stage list]
  P --> PL
  PL -->|forward| API[Provider API]
  API -->|stream, untouched| A
  PL -->|usage, savings| R[Recorder: channel]
  R --> DB[(SQLite, WAL)]
  PL --> HS[(HandleStore)]
```

## Entry paths

**Hook path.** Agents whose hooks can replace tool *output* call `lessr hook <agent>` with the finished tool call as JSON on stdin: Claude Code (`PostToolUse`, `hookSpecificOutput.updatedToolOutput`, 2.1.121 or newer), Gemini CLI (`AfterTool`), OpenCode (`tool.execute.after`) and pi (`tool_result`). The pipeline runs `on_tool_result`; the replacement goes back on stdout in whatever envelope that agent reads, and **nothing at all when there is nothing to change**. Gate, trap and dedup run here. One process spawn per call: keep startup under 2 ms (memory-mapped config and pack snapshots, no SQLite open on this path).

A *pre*-tool hook is the wrong event for this work: it fires before the tool runs, so there is no output to shrink. Rewriting the command instead is worse than useless — Claude Code evaluates the user's permission rules against the rewritten input, so a rewrite silently stops matching their own allow and deny rules. Lessr does not do it. See [ADAPTERS.md](ADAPTERS.md) for which agents can be reached and how.

**Proxy path.** Agents and SDKs point `base_url` at `http://127.0.0.1:7433`. `lessr-proxy` accepts `/v1/messages` (Anthropic) and `/v1/chat/completions` (OpenAI shape), runs `on_request`, forwards with the original headers, streams the body back byte-for-byte, and parses usage from the final SSE event or JSON body. Detection runs here; the free engine only reports.

This is how every agent without a usable hook is reached, and the only path for an SDK. It is not the default on an agent that has a hook: pointing Claude Code at a non-Anthropic `base_url` makes it treat Lessr as a third-party gateway and turns off Remote Control, tool search and server-managed settings. A tool that claims to be invisible cannot do that silently, so on Claude Code the proxy is opt-in and prints what it costs.

## The pipeline

`lessr_core::Pipeline` holds an ordered `Vec<Box<dyn Stage>>`. Each stage has a `Tier` (Free or Pro) for the receipt, a `Mode` (Off, Shadow, Active) from config and the self-healing table, and three optional hooks: `on_tool_result`, `on_request`, `on_usage`.

The pipeline times every stage with a monotonic clock. A stage over `Budget::soft` (2 ms) is logged; one over `Budget::hard` (10 ms) is skipped for the rest of the session and reported. The budget is a contract.

Order on the hook path: `trap → gate → dedup`. Proxy path: `detect`. Pro crates insert stages at declared positions (`Position::Before("gate")`, `Position::After("detect")`).

## Handles

Anything removed goes into `HandleStore` first and is referenced by a 4-hex-char id from the blake3 hash of the removed bytes. Append-only file per session under the config dir with an in-memory index; `lessr show <id>` reads it. Handles survive compaction because compaction is a stage that must write handles.

## Receipt

`lessr_receipt::Recorder` owns an mpsc channel and a background thread that batches inserts into SQLite (WAL). Stages send `Saving` and `Usage` events; nothing on the hot path touches the database.

## Packs

Filters are data. A pack is a directory of TOML rules plus a manifest signed with ed25519. The base pack is embedded with `include_bytes!`; packs on disk override it if their signature verifies. Rules compile at load into `RegexSet`s and are cached as a memory-mapped blob keyed by pack hash.

## Adapters

`lessr-adapters` knows, per agent, where the config lives and how to add a hook or a `base_url`. `lessr init` detects installed agents, backs up configs, patches them, and prints what changed. `lessr uninstall` restores backups. An existing `rtk` hook is chained, not overwritten.

## Crate dependency graph

```
lessr-cli ─┬─ lessr-proxy ─── lessr-core
           ├─ lessr-adapters ─ lessr-core
           ├─ lessr-gate ──┬─ lessr-core
           │               └─ lessr-packs
           ├─ lessr-dedup ── lessr-core
           ├─ lessr-detect ─ lessr-core
           └─ lessr-receipt ─ lessr-core
```

`lessr-core` depends on nothing in the workspace. Pro crates depend on `lessr-core` and, where needed, `lessr-gate` or `lessr-receipt`; never the reverse.

## Configuration

`~/.config/lessr/config.toml` (XDG; `~/Library/Application Support/lessr` on macOS; `%APPDATA%\lessr` on Windows). Every mechanism has three axes — `mode` (off, shadow, active), `level` (safe, balanced, aggressive) and its own settings — resolved through global, per-stage and per-repo layers, under a safety floor no layer can overrule. Plus proxy port and provider upstreams. The hook path parses no TOML; it reads a binary snapshot regenerated whenever the TOML changes. Full contract: [CONFIG.md](CONFIG.md).
