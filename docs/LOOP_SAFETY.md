# Loop-safety contract

Lessr can only make a session cheaper, never longer or less correct. If it ever does, it turns itself off and says so. Enforced in code and in the benchmark.

1. **Requested content is sacred.** Anything the agent asked for by path, range or search term is returned exactly. Gate, trap and (Pro) zoom apply only to output the agent did not ask for in that form.
2. **Nothing is deleted, only deferred.** Every cut leaves a handle; `lessr show <id>` returns the original bytes for the session. Handles survive compaction.
3. **Errors always pass.** Lines matching error, warning, fail, panic, exception, traceback and stack-frame patterns are never removed, truncated or reordered away. `lessr_gate::guard` verifies after every filter and fails open.
4. **No forced extra turns.** A mechanism may not require a second tool call to get what one call used to return, unless the first call is still cheaper than the original. Small files return whole; delta output includes counts.
5. **Self-healing filters.** If handles from one filter are expanded on more than 10 % of its outputs in a repo, that filter switches to `Off` for that repo and the receipt reports it.
6. **Kill switch and shadow mode per mechanism.** Every stage can be disabled alone; every new stage ships in `Shadow`.
7. **History is never rewritten while warm.** Nothing in the free engine mutates requests; Pro compaction runs only when the cache is already cold.
8. **Measured, not assumed.** `lessr bench` tracks tool calls per task and task success beside cost. A change that raises either for any filter does not ship.

## Error patterns (guard)

Case-insensitive, per line: `error`, `err:`, `warn`, `fail`, `panic`, `exception`, `traceback`, `fatal`, `denied`, `not found`, `cannot`, `unexpected`, `at .*:\d+:\d+` (stack frames), `^\s+at ` (JS frames), `-->` (rustc spans), `\bE\d{3,}\b` (lint codes). Extend in `crates/lessr-gate/src/guard.rs` with a fixture.
