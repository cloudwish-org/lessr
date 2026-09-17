# Agent memory for this repository

Read this before touching code. It is the shared memory for any coding agent (Claude Code, Cursor, Codex, OpenCode) working in `lessr`.

## What this repo is

The open-source engine and base pack of Lessr, a local token-saving layer for coding agents. Rust workspace, one binary (`lessr`). No LLM calls anywhere in this codebase. Ever.

## Invariants (never break these)

1. **Overhead ≤ 10 ms p99** on the 1 MB fixture. `cargo bench -p lessr-core` and the CI gate enforce it. No allocation per line in filters; use `memchr`, `bytes`, `RegexSet`.
2. **Errors always pass.** `lessr_gate::guard::errors_preserved` runs after every filter. If it fails, the gate returns the original bytes. Do not bypass it.
3. **Every cut leaves a handle.** Anything removed goes into `HandleStore` first. No exceptions.
4. **Never mutate a request while the cache is warm.** `lessr-detect` in this repo is report-only. Request mutation belongs to Pro crates.
5. **Receipt writes are off the hot path.** `Recorder` uses a channel and a background thread. Never call SQLite from a stage.
6. **Nothing leaves the machine.** No network calls except the forwarded provider request in `lessr-proxy`.
7. **Free never depends on Pro; Pro may be named, never nagged.** No crate here may depend on, import or feature-flag a Pro crate — `lessr_cli::run(PipelineBuilder)` is the only seam, and Pro registers stages through it from outside. Free *may* print what Pro adds: a static list of names and one URL, in `lessr pro` and one suppressible section of `lessr gain`. It may never appear in hook output, because hook output enters the agent's context and is billed again on every later turn. See `crates/lessr-cli/src/pro.rs`.

## How the code is organised

- `lessr-core::Stage` is the only extension point. A mechanism = one crate = one `Stage` impl.
- `lessr-cli::run(builder)` builds the pipeline from stages the caller registers. The public `main.rs` registers the free stages. The Pro binary registers more. Do not add feature flags for Pro; add crates.
- Filters come in two kinds: hot filters in Rust (`lessr-gate/src/filters/`) for the top commands, and declarative rules in packs (`packs/base/rules/*.toml`) interpreted by `lessr-gate::rules`.

## Working rules for agents

- Run `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` before proposing a change.
- Add a fixture under `crates/lessr-gate/fixtures/<command>/` for every filter change: `input.txt`, `expected.txt`.
- New filter: also add an `errors.txt` fixture proving error lines survive.
- Keep commit messages conventional (`feat(gate): …`, `fix(dedup): …`).
- Do not edit `packs/base/manifest.toml` signatures by hand; `cargo run -p lessr-packs -- sign` regenerates them.

## Where things are decided

- Architecture: `docs/ARCHITECTURE.md`
- Mechanism definitions and counting rules: `docs/MECHANISMS.md`
- Safety rules: `docs/LOOP_SAFETY.md`
- Performance budget and techniques: `docs/PERFORMANCE.md`
- Per-mechanism settings, levels and kill switches: `docs/CONFIG.md`
