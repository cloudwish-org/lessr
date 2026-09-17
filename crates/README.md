# Crates

One mechanism, one crate. Tiering is by crate, never by feature flag.

| Crate | Role | Stage | Path |
| --- | --- | --- | --- |
| lessr-core | Stage trait, Pipeline, HandleStore, Budget, token Estimator | — | both |
| lessr-gate | Tool-output gate, trap list, error guard, hot filters, pack rule engine | gate | hook |
| lessr-dedup | Re-read dedup: unchanged → hash line, changed → diff | dedup | hook |
| lessr-detect | Cache-break detector, report only | detect | proxy |
| lessr-receipt | Recorder (channel + thread), SQLite WAL, `lessr gain` queries | — | off hot path |
| lessr-proxy | localhost endpoint, streaming passthrough, usage capture | — | proxy |
| lessr-adapters | `lessr init`, `lessr hook <agent>`, rtk chaining, uninstall | — | both |
| lessr-packs | Pack format, ed25519 verify, base pack embed, compiled cache | — | load |
| lessr-cli | The `lessr` binary and `run(PipelineBuilder)` entry point | — | both |

Dependency rule: `lessr-core` depends on nothing in the workspace; every other crate depends on it; `lessr-cli` depends on all; nothing depends on `lessr-cli` except the Pro binary.
