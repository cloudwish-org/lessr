# Contributing

Thanks for helping make agents cheaper without making them worse.

## Ground rules

- Every filter change ships with fixtures: `input.txt`, `expected.txt`, `errors.txt`.
- Every PR runs the overhead bench; p99 above 10 ms fails CI.
- Do not add LLM calls, telemetry, or network calls. The only outbound request is the forwarded provider call in `lessr-proxy`.
- New mechanisms are new crates implementing `lessr_core::Stage`. Discuss in an issue first; some mechanisms live in Lessr Pro by design (see `docs/PRO.md`).

## Adding a declarative rule

1. Copy `packs/base/rules/_template.toml`.
2. Fill `match`, `keep`, `drop`, `collapse`, `summary`.
3. Add fixtures under `crates/lessr-gate/fixtures/<rule-id>/`.
4. `cargo test -p lessr-gate` and `cargo run -p lessr-packs -- sign`.

## Adding a hot filter

Only for commands where a declarative rule cannot hit the budget or the format needs real parsing (e.g. `cargo test`, `git diff`). Implement `lessr_gate::filter::Filter`, register in `lessr_gate::filters::all()`, add fixtures.

## Benchmarks

`cargo bench -p lessr-core` for overhead. `lessr bench` for the paired A/B harness; see `bench/README.md`.
