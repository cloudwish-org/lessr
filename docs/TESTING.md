# Testing

- `cargo test`: unit and fixture tests. Every filter has `input.txt`, `expected.txt`, `errors.txt`.
- Guard invariant property test: for random byte input, every error-pattern line in input is in output.
- `cargo bench -p lessr-core`: overhead; CI compares p99 to `bench/budget.toml`.
- `lessr bench`: paired A/B harness. Pinned repo, 40+ tasks, arms baseline and lessr; metrics cost, task success, wall time, tool calls. Results committed under `bench/results/<version>.md` with every release.
