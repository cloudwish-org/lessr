# Benchmarks

- `overhead`: pipeline and end-to-end hook timing on 4 KB, 64 KB and 1 MB fixtures. CI fails above `budget.toml` (p99 10 ms).
- `lessr bench`: paired A/B harness. Pinned public repo, 40+ tasks, arms baseline and lessr (optionally caveman, ponytail, rtk on the same tasks). Metrics: cost in dollars, task success, wall time, tool calls per task. Results committed under `results/<version>.md` with every release, red rows included.
