# Verification

How we prove that every supported agent × provider combination actually works, and how a
user sees what Lessr would save them *before* they change anything.

Two questions, one document:

- **(A) Conformance.** Does install → intercept → filter → uninstall actually work, on this
  agent, against this API shape, on both tiers? Not "it compiles".
- **(B) Dry-run savings.** What would Lessr have saved on real sessions that already
  happened, with no LLM call, no network and no money?

The binding constraint on (B) is `AGENTS.md`: *no LLM calls anywhere in this codebase, ever*.
The design below never makes one. It **records real sessions once and replays them offline**,
so byte savings are arithmetic and token savings come from the provider's own recorded
`usage` fields.

> Nothing in this file is built yet. It is the plan; §12 is the order to build it in.

## 0. Status this plan is written against

| Thing | State | Evidence |
| --- | --- | --- |
| `lessr-core` (Stage, Pipeline, Budget, HandleStore, Estimator) | built | `crates/lessr-core/src/` |
| `lessr-adapters` | built; 5 of 13 agents verified | `crates/lessr-adapters/src/agents/mod.rs` |
| Verified adapters | Claude Code, Codex, Gemini CLI, OpenCode, pi | `Confidence::Verified` in each module |
| Unverified adapters (print `base_url`, write nothing) | Cursor, Windsurf, Cline, Roo, Kilo, omp, Rakazo, generic | `agents::Unverified` |
| `lessr-gate`, `lessr-dedup`, `lessr-detect`, `lessr-proxy`, `lessr-receipt`, `lessr-packs` | doc-comment stubs | each `src/lib.rs` says "Not implemented yet" |
| `packs/base/` | README only, no rules, no manifest | `packs/base/README.md` |
| `serve`, `gain`, `show`, `bench`, `on`, `off` | exit 2, "not implemented" | `crates/lessr-cli/src/lib.rs::not_yet` |
| CI | fmt, clippy, `cargo test --workspace`, 2 bench gates, Linux + macOS | `.github/workflows/ci.yml` |
| Existing e2e tests | 10, real binary as subprocess, sandboxed `HOME`/`LESSR_HOME` | `crates/lessr-cli/tests/cli.rs` |
| Config design (`mode` × `level` × settings, 6 layers, safety floor, snapshot) | **specified** | `docs/CONFIG.md` |
| `Level` / per-stage `Settings` in `lessr-core`; `lessr on/off/shadow/level/config` | command surface exists, mechanism exits 2 | `crates/lessr-cli/src/cli.rs`, `lib.rs::not_yet` |

Everything below is therefore written so that each piece is independently buildable and so
that **a tier can be built before the mechanism it will eventually gate**. The two bench
gates already work that way (`crates/lessr-core/benches/pipeline.rs` measures a stand-in
`LineFilter` so the budget is enforced from the first commit); the replay harness and the
fake provider should be built the same way.

---

## 1. Test tiers

Cheapest and most deterministic first. **A tier only earns its cost if it proves something
the tier below it cannot.** The "Only this tier proves" column is the whole point of the
table: everything else is a false economy.

| Tier | Name | Cost per run | Deterministic | Network | Money |
| --- | --- | --- | --- | --- | --- |
| **T0** | Unit + fixture | seconds | yes | no | no |
| **T1** | Golden file / property | seconds–minutes | yes (seeded) | no | no |
| **T2** | Replay / simulation | seconds–minutes | yes | no | no |
| **T3** | Fake provider server | minutes | mostly | 127.0.0.1 only | no |
| **T4** | Real local agent + fake provider | minutes–hours | no | 127.0.0.1 only | no |
| **T5** | Live paid run | hours | no | yes | yes |

### T0 — Unit and fixture

One filter, one `input.txt` / `expected.txt` / `errors.txt` triple under
`crates/lessr-gate/fixtures/<command>/`, as `AGENTS.md` already mandates. Plus the pure
units already in the tree: `Budget::check`, `Estimator::observe`, `HandleId::parse`,
`AgentId::parse`, `Slot::get/put`.

- **Proves:** a named filter turns a named input into a named output; an error line
  survives *that* input; arithmetic is right.
- **Cannot prove:** anything about an input nobody wrote a fixture for. A fixture suite is
  a list of bugs already found, not a guarantee.

### T1 — Golden file and property

Golden files for whole-artefact output: a rendered `lessr init --show` plan, a `lessr gain`
receipt, a generated OpenCode plugin (`opencode::plugin_source`), a generated pi extension
(`pi::extension_source`). Properties for the universally-quantified invariants (§5).

- **Proves:** the invariants hold for *arbitrary* input, not just chosen input; that
  human-facing output did not drift by accident.
- **Cannot prove:** that the property is the right property. P1 (errors pass) is only as
  strong as `guard`'s pattern list in `docs/LOOP_SAFETY.md`.

### T2 — Replay / simulation (§4)

Recorded sessions replayed through the real `Pipeline`, offline. This is the tier that
answers the owner's second question and also the cheapest regression gate that uses *real*
data instead of fixtures someone wrote.

- **Proves:** exact byte savings on real tool output; exact turn multipliers; detector
  agreement against recorded `cache_write`; that a change to a filter moves the number,
  and by how much, on real sessions rather than on `bench/fixtures/cargo-test-seed.txt`.
- **Cannot prove:** that the agent *behaves* the same given the filtered output. A replay
  is a counterfactual on a fixed transcript: it assumes the agent would have made the same
  next tool call. That assumption is exactly what T5 exists to test, and the replay report
  must say so on its face (§4.6).

### T3 — Fake provider (§2)

A localhost server speaking both API shapes, with scriptable `usage`, SSE timing and
failure modes. `lessr serve` under test, real sockets, no money.

- **Proves:** the proxy forwards headers unchanged, streams without buffering, parses usage
  out of both shapes, degrades correctly on malformed frames, and — critically — that a
  request's **cacheable prefix reached the upstream byte-identical**, which can only be
  observed from the upstream's side.
- **Cannot prove:** that a real provider's cache actually *hits* on the prefix we produced.
  The fake believes whatever it is scripted to believe.

### T4 — Real local agent + fake provider

A real agent binary (Claude Code, Gemini CLI, OpenCode, pi, Codex) driven headless, with
`lessr init` applied and `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` pointed at `lessr serve`,
which forwards to the fake provider.

This is the highest-leverage tier in the plan. It converts "needs a real agent" from a
paid, flaky, rate-limited thing into a free, offline, repeatable one. **Everything except
model behaviour can be proved here.**

- **Proves:** the hook actually fires inside the shipped agent build; the agent's real
  payload shape matches what the adapter parses; a rewritten tool result actually reaches
  the model's context (observable in the request the fake provider receives); the agent
  does not crash, stall or retry because of us.
- **Cannot prove:** that the model does the same work with less context. Also cannot cover
  GUI-only agents (§3.4).

### T5 — Live paid run

`lessr bench`, as defined in `bench/README.md` and `docs/TESTING.md`: a pinned public repo,
40+ tasks, arms `baseline` and `lessr` (plus the competitor arms, §8), metrics cost, task
success, wall time, tool calls per task.

- **Only this tier proves:**
  1. **Loop safety rule 8** — that filtering did not raise tool calls per task or lower task
     success. `docs/LOOP_SAFETY.md`: "Measured, not assumed."
  2. That a real provider's prompt cache actually serves the prefix `cachefix` produced.
     A cache hit is the provider's decision, not ours.
  3. That real `usage` payloads have the shape we parse, across model families and versions.
  4. Behaviour under rate limits, 429s, retries and real latency.
- **Cost:** budget $50–200 per release for the two core arms; more with competitor arms.
  Pre-release only. Results committed under `bench/results/<version>.md` per
  `docs/RELEASE.md` step 2, red rows included.

> **The `lessr bench` / "no LLM calls" reconciliation.** `AGENTS.md` invariant: no LLM calls
> *in this codebase*. `lessr bench` does not call a model; it drives an external agent
> binary that does, and reads its receipt. The distinction is worth stating in
> `docs/TESTING.md` because a reader can otherwise conclude the two documents contradict
> each other. Nothing in `crates/` ever opens a connection to a model API except
> `lessr-proxy`'s forward of a request the agent made.

---

## 2. The fake provider

**New workspace member: `crates/lessr-fakeprovider`, `publish = false`, binary `lessr-fake`.**

Test infrastructure, never shipped. It opens a socket, which invariant 6 forbids the
*product* from doing; it binds `127.0.0.1` with port `0` (ephemeral) by default so parallel
tests cannot collide and nothing is reachable off the machine. Put it beside `bench/` in
`Cargo.toml` members, not in the release binary's dependency graph.

### 2.1 Surface

| Route | Purpose |
| --- | --- |
| `POST /v1/messages` | Anthropic shape |
| `POST /v1/chat/completions` | OpenAI shape, incl. OpenRouter-compatible upstreams |
| `GET /__fake/health` | readiness, so a test can wait without sleeping |
| `POST /__fake/script` | load a scenario at runtime |
| `GET /__fake/received` | **the assertion surface**: every request it got, as raw bytes, with headers, in order |
| `POST /__fake/reset` | clear received + scenario |

`GET /__fake/received` is the part that matters. Prefix byte-stability, header
preservation and "a Pro stage did not mutate a warm request" are all statements about what
the *upstream* saw. They are unobservable from the client side and untestable without this.

### 2.2 Scenarios

A scenario is TOML, one entry per expected request:

```toml
[[turn]]
status = 200
stream = true
model = "claude-sonnet-4-6"
# The usage the turn reports. Any field may be omitted entirely, which is the
# point: a receipt must degrade to "unknown", never to 0 and never to a guess.
usage = { input = 1200, output = 340, cache_read = 18_000, cache_write = 0 }
# Anthropic splits usage across frames; this controls where each half lands.
usage_placement = "message_start+message_delta"   # or "final", "none", "duplicated"
frame_delay_ms = 3
```

### 2.3 What it must be able to fake

Each row is a test the proxy has to survive. The right-hand column is the assertion.

| Scenario | Assertion |
| --- | --- |
| **Cache hit** (`cache_read > 0`, `cache_write = 0`) | receipt records a warm turn; `detect` reports prefix stable |
| **Cache write** (`cache_write > 0`) | receipt records a break; `detect` classifies the cause |
| **Break on turn k** (stable 1..k-1, write at k) | `detect` finds the *first differing byte* and names the cause; agreement with the scripted cause |
| **No cache fields at all** (bare OpenAI shape) | cache lines read "unknown", not `$0.00`; nothing marked exact |
| **`prompt_tokens_details.cached_tokens`** (OpenAI cache) | mapped to `Usage::cache_read`; `cache_write` stays unknown, not zero |
| **Usage split across frames** | parser assembles `message_start` + `message_delta` into one `Usage` |
| **Usage duplicated / in a non-final frame** | last-writer-wins, recorded once, no double count |
| **400 / 401 / 429 + `retry-after` / 500 / 529** | status, headers and body forwarded byte-identical; no usage recorded; the proxy does not retry on the agent's behalf |
| **Slow stream** (200 ms per frame) | first byte out is not delayed by us; measure client TTFB ≤ upstream TTFB + budget |
| **Stalled stream** (no frame for 60 s) | proxy does not buffer, does not time out before the client, does not hold the response in memory |
| **Truncated SSE event** | pass through, record **no** usage; never a partial `Usage` |
| **`data:` with invalid JSON** | same |
| **Missing `message_stop`** | same |
| **`usage` with a string where a number belongs** | same; and no panic — this is a fuzz target too |
| **Connection closed mid-stream** | client sees the same truncation it would have seen without us |
| **`content-length` disagreeing with the body** | forwarded as-is; we are not an HTTP corrector |
| **gzip / identity / chunked, no content-length** | byte-identical passthrough in every case |
| **HTTP/1.1 and HTTP/2** | both |
| **Body > 10 MB, and empty body** | no OOM, no panic |

### 2.4 Capsule playback

`lessr-fake --from <capsule>.lrec` serves a *recorded* session's responses in order.

This is the bridge that makes §2 and §4 one system: the same recorded session can be run at
T2 (offline replay), T3 (real proxy, fake upstream) and T4 (real agent, real proxy, fake
upstream). The numbers each tier produces must agree — see the tier-agreement gate in §4.7.

---

## 3. Agent conformance

### 3.1 The five gates

Every agent is proved by the same five, in order. Naming them lets the matrix in §10 be a
grid of gate marks rather than prose.

| Gate | Claim | How |
| --- | --- | --- |
| **G1 detect** | `lessr init` finds the agent and names the right config path | seeded fake `HOME`, assert on `Detected` |
| **G2 install** | the written config is exactly right; idempotent; chains behind `rtk` rather than replacing it | golden diff of the file; second `init` is a no-op |
| **G3 intercept** | the agent actually calls `lessr hook <agent>`, or its traffic actually reaches `lessr serve` | run the real agent; assert the hook ran / the proxy saw the request |
| **G4 filter** | output is actually reduced *inside the agent's context*, and errors still pass | assert on the request body the fake provider received on the **next** turn |
| **G5 uninstall** | config restored byte-for-byte; backup index left consistent | blake3 of the file before `init` == blake3 after `uninstall` |

**G4 is the only gate that proves the product works.** G1, G2 and G5 prove we did not break
the user's machine; G3 proves we are in the path; G4 proves being in the path was worth it.
Asserting G4 on the hook's stdout is not enough — an agent may accept our stdout and ignore
it. The assertion has to be that the *next request to the provider contains the shrunken
bytes*, which is why G4 is defined against `GET /__fake/received`.

### 3.2 What is automatable today

G1, G2 and G5 are automatable for **every agent, right now**, with no agent installed and
no vendor cooperation, using the pattern already in `crates/lessr-cli/tests/cli.rs`: set
`HOME` and `LESSR_HOME` into a `tempfile::tempdir()`, seed a fake config tree, run the real
binary as a subprocess. `Paths::with_roots` exists precisely so adapters read no environment
variable behind a test's back (`crates/lessr-adapters/src/paths.rs`).

Concretely, per agent, in `crates/lessr-adapters/tests/conformance.rs`:

```
for agent in AgentId::all():
    seed a fake HOME with that agent's config in each of:
      absent · empty file · realistic file · file with an rtk hook ·
      file with our hook already · unparseable file · wrong-shape file
    G1  detect(paths) == expected
    G2  plan_init(...).render() matches the committed golden
        apply; second plan_init is a no-op          (idempotence)
        the rtk case chains, never replaces          (ADAPTERS.md rule)
    G5  apply(plan_uninstall); blake3(file) == blake3(original)
        unverified agents: plan writes nothing at all, plan_uninstall is a no-op
```

The last line is already asserted for the whole registry by
`an_unverified_agent_is_told_about_by_hand_and_never_written_to` in
`crates/lessr-adapters/src/agents/mod.rs`. Generalise it into the table above.

Three more G2 assertions that apply to every agent and are easy to forget:

- **Opt-in-only agents are not installed by a bare `lessr init`.** `OPT_IN_ONLY` in
  `crates/lessr-cli/src/install.rs` holds Codex, because a hook that provably cannot save a
  token is still a process spawn on every tool call, and LOOP_SAFETY says Lessr may only make
  a session cheaper. Assert both halves: `lessr init --yes` on a machine with Codex installed
  writes nothing to `~/.codex/config.toml`; `lessr init --yes --agent codex` writes the block.
  When an agent gains output replacement, the test that has to change is this one — which is
  the right place for the decision to live.
- **The hook command survives the shell the agent hands it to.** `install::shell_quote`
  exists because the macOS config directory is `~/Library/Application Support/lessr`, and an
  unquoted space there breaks the hook on every tool call, for every agent, silently. Test
  the written command with an install path containing a space, a single quote, a `$`, and a
  backslash on Windows — and then *actually execute* the written string through `sh -c`
  (and `cmd /c`) and assert it runs. A quoting test that only compares strings proves the
  quoting is stable, not that it is correct.
- **The command is an absolute path, not `lessr`.** Agents launched from a desktop icon
  rarely inherit a shell's `PATH`. Assert the written command starts with the resolved
  `current_exe`, for every verified adapter.

**G5 deserves a property, not a fixture** (P10, §5). `init` then `uninstall` over a
*generated* corpus of valid JSON settings files is far stronger than the single fixture in
`crates/lessr-cli/tests/cli.rs::init_then_uninstall_restores_the_file_byte_for_byte`, and
costs one generator.

### 3.3 Per-agent conformance, by hook capability

The hook path only works for an agent whose hook can **replace tool output**. Most cannot:
they can rewrite tool *input*. This is the single biggest constraint on the matrix and it
splits the agents into three groups.

| Group | Agents | Adapter state | Path the gate runs on |
| --- | --- | --- | --- |
| **Output-rewriting hook, confirmed** | Gemini CLI, OpenCode, pi | `Confidence::Verified` | hook |
| **Output-rewriting hook, assumed but unconfirmed** | Claude Code | `Confidence::Verified` | hook — **needs T4 confirmation** |
| **Hook verified, but it cannot rewrite output** | Codex | `Confidence::Verified`, observe-only | proxy only |
| **Hook exists, schema unconfirmed** | Cursor, Windsurf | `Unverified` | proxy only |
| **No hook mechanism at all** | Cline, Roo, Kilo (rules-file injection only) | `Unverified` | proxy only |
| **Not a harness** | omp (runs on pi), Rakazo (server deployment), generic SDK | `Unverified` | pi's hook / proxy |

Codex is the informative case and the one to copy. Its adapter is **verified** — it writes a
real `[[hooks.PreToolUse]]` block into `~/.codex/config.toml` — and its `parse_hook` returns
`Ok(None)` on purpose, because `PostToolUse` cannot replace a tool's output
(`updatedMCPToolOutput` is unsupported) and `permissionDecision: allow` without an
`updatedInput` is rejected. So `lessr hook codex` reads its payload and prints nothing, and
the plan says so in as many words. That is the honest shape for every agent in the bottom
four rows: **install may still be verified even where filtering is impossible**, and the
conformance gates have to distinguish "installs correctly" from "filters". §3.1's G2/G5 and
G3/G4 split exists for exactly this.

Cline, Roo and Kilo are structurally excluded rather than merely unverified: rules-file
injection (`.clinerules`, `.roo/rules/`, `kilo.jsonc`) changes the prompt, which is
`docs/VISION.md`'s "someone else's product", not a tool-result filter. No amount of
verification work moves them onto the hook path. Their only route is the proxy, and their
base URL lives in a VS Code settings UI, so even that is a manual step.

The three confirmed ones each replace output differently, and the adapter layer already
encodes it — which is why `Adapter::render_hook` / `render_unchanged` exist
(`crates/lessr-adapters/src/agents/mod.rs`):

| Agent | Event | Output contract | Silence means |
| --- | --- | --- | --- |
| Gemini CLI | `AfterTool` | `{"decision":"deny","reason":"<filtered text>"}` — stdout is a *decision* | print nothing |
| OpenCode | `tool.execute.after` (TS plugin) | `{input, output}` back with `output.output` replaced | our envelope unchanged |
| pi | `pi.on("tool_result")` (TS extension) | `{"content":"<filtered text>"}` — a partial patch | print nothing |
| Claude Code | `PostToolUse` payload shape | same JSON back, `tool_response` / `tool_response.stdout` replaced | payload back byte-for-byte |

**The Claude Code row is the highest-value unknown in this plan.** The free gate's whole
value rests on it, it is the most-installed agent, and the assumption is untested. It is
also entangled with a live contradiction — see §11.1. First T4 run, before anything else.

Per-agent G3/G4 recipes:

| Agent | G3/G4 automatable at T4? | How |
| --- | --- | --- |
| **Claude Code** | yes | headless run, `lessr init --agent claude --yes`, base_url → `lessr serve` → `lessr-fake`. Assert the next request body carries the shrunken output. |
| **Gemini CLI** | yes | as above; additionally assert the `decision`/`reason` document is what the CLI consumes and that a no-op prints **zero bytes** — a stray byte on stdout is a denial |
| **OpenCode** | yes | the plugin is ours (`opencode::plugin_source`), so also run it under `node --test` directly against a stub `lessr` binary, which proves the JS half without OpenCode |
| **pi** | yes | same shape — test `pi::extension_source` under `node --test` with a stub, then the real harness |
| **Codex** | G2/G5 yes; G3 yes; **G4 N/A** | G2/G5 on the marker-delimited TOML block. G3: assert the hook was invoked and printed **zero bytes** — a non-empty stdout without `updatedInput` is rejected by Codex and would break the agent. G4 via `OPENAI_BASE_URL` → `lessr serve`, asserted on `/__fake/received` |
| **omp** | proxy, or inherited from pi | omp runs on the pi harness, so `lessr init --agent pi` installs the extension omp uses. Whether pi's extension actually loads under omp is unconfirmed — run T4 before claiming the cell |
| **Rakazo** | proxy only | a server deployment configured by `.env`, not a harness; point its model URL at the proxy |
| **Cursor, Windsurf** | **no** — manual | closed GUI; §3.4. Cursor is the near miss: it *has* hooks, in two vocabularies (`preToolUse`, `beforeShellExecution`), but the `postToolUse` schema we would need is unconfirmed. A T4 session is what would confirm it |
| **Cline, Roo, Kilo** | **no** — manual | no hook mechanism exists; proxy only, and the base URL is a VS Code settings-UI action |
| **generic SDK** | yes | a 20-line Rust and a 20-line Python client, both shapes, committed under `tests/sdk/` |

### 3.4 What needs a human, and how often

Cursor, Windsurf, Cline, Roo and Kilo are closed-source GUI editors. They cannot be driven
headless in CI, they auto-update without asking, and their settings are edited through a UI.
Pretending otherwise produces a green matrix that means nothing.

**The manual checklist**, per agent, target 10 minutes:

1. Record agent name and exact version.
2. `cp` the agent's config aside; `sha256sum` it.
3. `lessr init --show --agent <x>` → paste the output into the record.
4. Apply. Point the agent at `lessr serve`; start `lessr-fake` with the
   `cache-break-at-turn-5` scenario.
5. Run three fixed prompts from `docs/conformance/prompts.md`: one that reads a big file,
   one that runs a failing test, one that greps.
6. `curl /__fake/received` → confirm (a) the proxy was in the path, (b) tool output in the
   second request is smaller than what the tool produced, (c) the failing test's error
   lines are present verbatim.
7. `lessr uninstall --agent <x>`; `sha256sum` again; must match step 2.
8. Commit the artefacts to `docs/conformance/<agent>/<agent-version>.md`: the two hashes,
   the `--show` output, the received-request excerpt, the date, the operator.

**A manual result with no committed artefact is a memory, not a test.** Step 8 is not
optional.

**Cadence:**

| When | Scope |
| --- | --- |
| Pre-release | every manual agent, inserted between steps 2 and 3 of `docs/RELEASE.md` |
| Monthly | calendar sweep of every manual agent, because agents auto-update under the user with no Lessr change |
| On demand | whenever an agent ships a major version, or a user reports that agent |

**Staleness must be visible.** If `docs/conformance/<agent>/` has no entry newer than 60
days, `lessr init` prints `not verified against <agent> since <date>` for that agent. This
is cheap, honest, and keeps the matrix from rotting silently. A test asserts that every
agent in `AgentId::all()` has a conformance directory.

---

## 4. The replay harness — the dry-run savings test

> *"See actually dry test on how it will save when real usage?"*

A session transcript is a list of turns, each carrying tool results and the provider's own
`usage` fields. Replaying it through the pipeline offline computes **exact byte savings**
and, using the recorded usage, **exact turn multipliers** and **estimated token and dollar
savings** — with zero network, zero LLM calls and zero money.

### 4.1 Where recordings come from

Two routes. Both produce the same capsule format.

**R1 — Proxy recorder (universal, exact).** `lessr serve --record <dir>` writes a capsule
from what the proxy already sees. Works for any agent that honours a `base_url`, needs no
per-agent transcript parser, and captures the request bodies *as sent* — which is the only
way to get the cacheable prefix right. Requires the user to run one session through Lessr
in report-only mode first.

**R2 — Transcript importer (uses history the user already has).** Agents keep transcripts
on disk. Claude Code hands us the path in every hook payload:
`transcript_path` in `fixtures/hook/claude_pre_tool_use.json` points at
`~/.claude/projects/<slug>/<session>.jsonl`. So a user who has been running Claude Code for
weeks has weeks of replayable data and has never installed anything.

Rules for importers:

- **One importer per agent, and only for agents whose transcript format is verified** —
  the same `Confidence` discipline as adapters. A wrong importer produces a wrong number,
  which is worse than no number. Claude Code first, for the reason above.
- An importer that cannot find `usage` fields **skips that session and says so**. It never
  substitutes an estimate for a missing provider field.
- An importer never writes to the transcript. Read-only, always.

```
$ lessr replay import --agent claude --since 30d
  ~/.claude/projects  →  7 sessions, 412 turns
  4 imported · 3 skipped (no usage fields — older transcript format)
  capsules → ~/.config/lessr/capsules/   redaction: local
```

### 4.2 Capsule format

A capsule is a directory (tar+zstd for transport), schema-versioned.

```
<session>.lrec/
  meta.json        schema, agent, provider shape, models, repo_hash, recorded_at,
                   lessr version, recorder (proxy|import:<agent>), redaction profile,
                   savings_fidelity: exact | approximate
  turns.jsonl      one record per turn
  usage.jsonl      the provider's usage fields per turn, verbatim, unparsed
  blobs/<blake3>   tool-result bodies, content-addressed, deduplicated
  SHA256SUMS
```

A turn record is deliberately the vocabulary of `lessr_core::types` — a replay case *is* a
`ToolResult` plus that turn's `Usage`, and nothing else:

```json
{"n": 9,
 "model": "claude-sonnet-4-6",
 "request_bytes": 48213,
 "prefix_bytes": 12004,
 "prefix_blake3": "9a3f…",
 "tool_calls": [
   {"tool": "Shell", "tool_name": "Bash",
    "command": {"program": "cargo", "args": ["test", "--workspace"]},
    "path": null, "explicit_selection": false,
    "blob": "b3:ab12…", "bytes": 31488}],
 "usage": {"input": 1200, "output": 340, "cache_read": 18000, "cache_write": 0}}
```

`explicit_selection` is recorded, not re-derived, because it is loop-safety rule 1's input
and each adapter computes it differently (`claude_code::explicit_selection`,
`opencode::explicit_selection`).

### 4.3 Redaction

Happens **at record time**, never at replay time. A capsule that was written unredacted is
unredacted forever.

| Profile | Blobs | Use | Fidelity |
| --- | --- | --- | --- |
| `local` (default) | kept verbatim | the user's own machine, `chmod 600`, under the config dir | **exact** |
| `shareable` | rewritten | bug reports, the committed corpus | **approximate** |
| `metadata` | dropped; lengths, line counts, blake3 only | telemetry-shaped sharing | estimate only |

`shareable` removes: absolute paths (→ `/repo/<rel>`, home and username stripped),
`KEY=value` where `KEY` matches a secret-name pattern, high-entropy token shapes (`sk-`,
`ghp_`, AWS key ids, JWTs, PEM blocks), email addresses (the operator's own in particular),
hostnames, IPs, and anything the user adds to `redact.extra` in config.

**Redaction changes byte counts, therefore it changes the number.** A `shareable` capsule
records both `bytes_original` and `bytes_redacted` per blob and is stamped
`savings_fidelity: approximate`. Its report may not print "exact". This is the honest
consequence of sharing and the plan should not pretend otherwise.

`lessr replay redact --check <capsule>` prints exactly what would be removed and refuses to
write if it cannot parse a blob. `local` capsules never leave the machine — consistent with
invariant 6 and with `docs/product/CLOUD.md` ("No prompt bodies stored unless the user opts
in to full logs").

### 4.4 What a replay computes

Per turn, per stage:

1. Rebuild the `ToolResult` from the blob plus recorded metadata.
2. Run the real `Pipeline` on it. `bytes_before − bytes_after` → **exact bytes**.
3. Tokens via `lessr_core::Estimator`, seeded from *this capsule's own*
   `(request_bytes, input + cache_read + cache_write)` pairs, per provider. `Estimator`
   already clamps nonsense ratios and always returns `Tokens::Estimate`
   (`crates/lessr-core/src/estimator.rs`). → **estimate, marked**.
4. **Lifetime cost.** This is the point of the whole product (`docs/VISION.md`: "a 30k-token
   file read at turn 10 of a 60-turn session costs about six times its face value"). A
   result entering at turn *n* is billed once at write price, then at cache-read price on
   every later turn where the prefix held, and at write price again on each turn where it
   broke. **The capsule knows which turns broke**, from recorded `cache_write` — so the
   multiplier is *counted from the recording, not assumed*. → **exact count**.
5. Dollars: `tokens × rate(model, kind)` from the bundled price table, whose id is printed
   in the report header (`docs/RECEIPT.md`).
6. `detect`: replay `on_request` over recorded prefixes and compare the verdict against the
   recorded `cache_write`. A turn called "broken" must have `cache_write > 0`; a turn called
   stable must not. → the **detector agreement rate**, a falsifiable number that a simulation
   can otherwise never produce.

### 4.5 Exact vs estimated — the contract

Matching `docs/MECHANISMS.md` ("Provider usage fields are the truth… Never report an
estimate as exact"):

| Quantity | Exact when | Estimated when |
| --- | --- | --- |
| Bytes removed by gate / trap / dedup | always, on a `local` capsule | never |
| Bytes removed, on a `shareable` capsule | never | always — redaction moved the bytes |
| Tokens removed | never | always — `bytes/4` with the per-capsule correction |
| Turns a result is re-billed over | always — counted from the recording | never |
| Cache-write tokens avoided (`detect`, `cachefix`) | Anthropic: always, from `cache_creation_input_tokens` | OpenAI shape: **unavailable**, not zero (§11.4) |
| Dollars | inherits the token line above | inherits |

The headline is therefore **"exact bytes, exact turn counts, estimated tokens"** and the
report says so in those words.

### 4.6 The command, and how a user runs it on their own history

**Recommendation: a new top-level `lessr replay`.** Not `lessr bench --replay`, not part of
`lessr gain`.

Grounds, from the existing docs:

- `lessr bench` is *already defined* in `bench/README.md` and `docs/TESTING.md` as the
  paired A/B harness with baseline/lessr arms and cost/success/wall-time/tool-call metrics.
  Overloading it with an offline simulator makes "the bench numbers" ambiguous in exactly
  the place the project promises published, honest, re-runnable results.
- `lessr gain` is defined in `docs/RECEIPT.md` as "what Lessr saved, from local data only" —
  past tense, from the recorder's SQLite. A replay reports what Lessr *would have* saved on
  a session it was not running in. Different tense, different provenance. Folding them
  together risks a receipt where some lines are observed and some simulated with no visible
  seam, which is the one thing `docs/VISION.md` ("Honest numbers") forbids.
- `lessr replay` serves both audiences from one implementation: the user's "show me my
  number before I install anything" and CI's regression gate on the committed corpus.

```
lessr replay                                   # the front door: import + run + report
lessr replay import --agent claude [--from P] [--since 30d] [--redact local|shareable]
lessr replay run <capsule…> [--stages all|free|pro] [--level safe|balanced|aggressive]
                            [--json] [--against baseline.json]
lessr replay verify <capsule…>                 # §5 correctness gates only, no savings
lessr replay redact --check <capsule>
```

`lessr gain` gains one thing only: a `--simulated` section, clearly fenced, that shows the
latest replay total — never interleaved with observed lines.

The zero-argument front door is the product experience:

```
$ lessr replay
Reading Claude Code history from ~/.claude/projects  (7 sessions, last 14 days)
  redaction: local — bodies stay in ~/.config/lessr/capsules, nothing is sent
  3 sessions skipped: no usage fields (older transcript format)

lessr replay — 4 sessions · 231 turns · simulated

  Would have saved                            $ 9.10   (24 %)
    tool-output gate (base pack)      $ 3.90   exact bytes, est. tokens
    re-read dedup                     $ 2.10   exact bytes, est. tokens
    trap list                         $ 1.20   exact bytes, est. tokens
    cache breaks found                $ 1.90   exact, from your usage fields

  Cache
    231 turns · prefix stable 186 · breaks 45 (timestamp in system prompt, 41×)

  Most expensive read
    pnpm-lock.yaml, session 3 turn 9 → $ 4.10 by turn 60

  Method  bytes exact · tokens from 3.81 B/token learned on your own 231 turns
          turn counts from your recording · prices table 2026-09
  Limit   a replay assumes your agent would have taken the same next step.
          `lessr bench` is what tests that assumption.

  No agent was run. Nothing was sent. No tokens were spent.

  lessr replay --explain   the arithmetic for every line
  lessr init               make it real
```

The `Limit` line is mandatory. It is the one thing a simulation cannot prove and it belongs
on the face of the report, not in a footnote.

### 4.7 Determinism, corpus and the tier-agreement gate

**Determinism.** A replay must be byte-deterministic given `(capsule, pack hash, binary
version, level)`. That requires: no wall clock in filter output, no hash-map iteration order
in output, content-derived handle ids (already true — `HandleId::from_hash` over blake3).
The JSON report header pins capsule schema, pack id + hash, binary version, price-table id
and level. CI diffs the JSON; a deliberate change means committing a new baseline with a
reason in the commit message.

**Corpus.** `bench/replay/corpus/`, capsules recorded once and committed, `shareable`
redaction, checksummed, ≤ 2 MB compressed each, with a `PROVENANCE.md` line per capsule
(agent, agent version, provider shape, lessr version that recorded it, date, consent).
Target coverage: each verified agent × each provider shape × at least one session with a
mid-session cache break × one with a monorepo-sized read × one with a failing test suite.
A blob truncated to fit the size cap marks the capsule `approximate`.

**Tier agreement.** Run the *same* capsule at T2 (offline replay), T3 (real proxy against
`lessr-fake --from <capsule>`) and, for one agent, T4. Gate:

```
replay_bytes_saved(stage) == proxy_bytes_saved(stage)      exactly, for gate/trap/dedup
detect_verdicts(replay)   == detect_verdicts(proxy)        exactly
```

This is what proves the offline replay is not lying about what the real pipeline does, and
it is the reason §2.4 exists. Without it, T2 is a model of the system rather than a test of
it.

---

## 5. Correctness gates that must never regress

Expressed as properties. The "Kind" column is the recommendation asked for: which of these
deserve arbitrary-input property tests rather than fixtures.

| # | Property | Statement | Kind | Source |
| --- | --- | --- | --- | --- |
| **P1** | Errors always pass | ∀ bytes `B`, ∀ stage sets `S`, ∀ levels `L`: every line of `B` matching `guard`'s patterns appears in `run(S,L,B)` | **property**, raw bytes + corpus-seeded generator | LOOP_SAFETY 3; `AGENTS.md` 2 |
| **P2** | Every cut leaves a handle | ∀ `B`: `bytes_saved > 0 ⇒ handle.is_some()`, and `HandleStore::get(h)` returns bytes whose blake3 matches, and output ∪ handles covers every byte of `B` | **property** | LOOP_SAFETY 2; `AGENTS.md` 3 |
| **P3** | Requested content is sacred | ∀ `B`: `explicit_selection ⇒ output == B`, byte for byte, for every filtering stage | **property** — one line, universally quantified, ideal | LOOP_SAFETY 1 |
| **P4** | No forced extra turns | a file under the threshold returns whole; every handle line carries the counts needed to decide whether to expand; `outline + one zoom < whole file` | fixture + a threshold property | LOOP_SAFETY 4 |
| **P5** | Prefix stays byte-stable | ∀ recorded request `R`, ∀ request-mutating stage `s`: `prefix(s(R)) == prefix(R)` | **property** over capsule-seeded requests, **asserted at `/__fake/received`** | MECHANISMS_PRO cachefix |
| **P6** | Never mutate while warm | ∀ `R` with `warm(session)`: `s(R).body == R.body` | **property** | `AGENTS.md` 4; LOOP_SAFETY 7 |
| **P7** | The hook never breaks the agent | ∀ byte strings `X` — not only valid JSON: `lessr hook <a> < X` exits 0, and stdout is `X`, or valid agent JSON, or (Gemini/pi) empty | **property + fuzz** | ADAPTERS hook contract |
| **P8** | Idempotence | `filter(filter(B)) == filter(B)` | **property** — catches filters that eat their own summary lines | new; load-bearing under §11.2 |
| **P9** | Monotonic savings | `bytes_after ≤ bytes_before` wherever a saving is claimed; the saturation in `Saving::bytes_saved` never fires | **property** | `crates/lessr-core/src/types.rs` |
| **P10** | Uninstall is byte-for-byte | ∀ valid JSON config `C` in a generated corpus: `uninstall(init(C)) == C` | **property** over generated configs | ADAPTERS; README |
| **P11** | The receipt adds up | per-stage savings sum to the session total; no line marked `exact` derives from an estimated input | **property** over generated `Saving` vectors | MECHANISMS token estimation |
| **P12** | Nothing leaves the machine | the whole per-commit suite passes with outbound network blocked | test harness | `AGENTS.md` 6 |

**Which should be properties, and why.** P1, P2, P3, P7, P8, P9, P10 are universally
quantified over input with a cheap oracle — those are property tests, full stop, and
`docs/CODE_STYLE.md` already mandates P1 ("Property tests for filters: the guard invariant
must hold for arbitrary input"). P5, P6 and P11 need *structured* generators (a request, a
session, a savings vector), so they are properties over a **capsule-seeded generator**
rather than raw bytes. P4 resists a clean property because "self-sufficient" is a judgement;
it stays a threshold property plus fixtures.

**Tooling.** `proptest`, with `proptest-regressions/` committed so a shrunk counterexample
becomes a permanent fixture. `cargo-fuzz` nightly for P7 and P1 across the parse → filter →
render boundary. The loop between tiers is the point: every fuzz find becomes a T0 fixture.

**P12 is cheap and currently untested.** Run the per-commit suite under `unshare -n` on
Linux (and a deny-all rule on macOS). Everything must pass; only the T3 jobs need loopback.
This turns invariant 6 from a comment into a gate.

---

## 6. Configuration: modes, levels and per-stage settings

The design is specified in `docs/CONFIG.md`: three axes per mechanism —
`mode` (`off`/`shadow`/`active`), `level` (`safe`/`balanced`/`aggressive`) and per-stage
settings — resolved through six layers, under a safety floor configuration cannot reach.
`Mode` already exists in `crates/lessr-core/src/types.rs` and defaults to `Shadow`, which is
loop-safety rule 6 encoded in the type. `Level`, `Settings` and the snapshot are not built
yet; the command surface is (`lessr on/off/shadow/level/config`, each with `--repo`, in
`crates/lessr-cli/src/cli.rs`). This section is what must be true of them.

`docs/CONFIG.md` makes an unusually testable set of promises. Each row below is one of them.

### 6.1 The gates

| # | Gate | Test |
| --- | --- | --- |
| **C1** | `off` means *not run*, not *inert* | a stage whose `on_tool_result` panics, registered `Off`. The pipeline must complete. Today `Pipeline::run_tool_result` `continue`s on `Mode::Off` before touching the trait object — this test pins that behaviour so a refactor cannot turn it into "run and discard" |
| **C2** | `Off` claims nothing | an `Off` stage contributes no `Saving`, no handle, no receipt row, no budget event |
| **C3** | **The safety floor holds at every level** | `docs/CONFIG.md`, "What a level may never do". P1, P2, P3, P4 (§5) are quantified over `Level` as well as input. Aggressive may cut *more*; it may never weaken a check. **A config that could turn `guard` off must not parse** — assert the key does not exist, at the type level, so it cannot be added by accident |
| **C4** | Levels are ordered | ∀ `B`: `bytes_saved(safe,B) ≤ bytes_saved(balanced,B) ≤ bytes_saved(aggressive,B)`. A one-line property, and it is the only thing that makes "how much it cuts" a meaningful axis |
| **C5** | Level semantics match the table | `docs/CONFIG.md` states each level's cuts per mechanism: `safe` gate touches "nothing that was ever a distinct visible line"; `balanced` adds `×N` collapsing, passing-test counts, timestamps, `node_modules`; `aggressive` adds non-consecutive duplicates and a tail-only summary. One fixture triple per (mechanism, level) cell — 12 cells for the four free mechanisms |
| **C6** | Settings actually bite | each named tunable gets a test that changing it changes the output: `max_repeated_lines`, `summary_after_lines`, `strip_timestamps`, `json_bytes`, `max_bytes`, `context_lines`, `min_bytes`. A documented setting nothing reads is the failure mode here, and C9 catches the reporting half |
| **C7** | Layer precedence | table-driven over all **6** layers × present/absent: compiled default, `[stages.default]`, `[stages.<name>]`, `[repo."<p>".stages.default]`, `[repo."<p>".stages.<name>]`, `LESSR_STAGE_<NAME>_<KEY>`. Include "env set to the same value as the default", which a naive `Option` chain resolves to the wrong *provenance* even when the value is right — and provenance is what `lessr config --explain` prints |
| **C8** | Per-repo override applies, and matches the right repo | run the hook with `cwd` inside repo A and repo B with different overrides; assert different output from identical input. Plus the near-miss cases: a path prefix that is not a directory boundary (`/work/mono` must not match `/work/monorepo`), a symlinked repo, and a `cwd` outside every configured repo |
| **C9** | **Self-healing beats configuration** | `docs/CONFIG.md` safety floor + LOOP_SAFETY 5: a filter expanded on > 10 % of outputs in a repo stays off and **no layer can promote it**. Force the table off, set `mode = "active"` at the highest layer, assert the stage does not run and that `lessr config --explain` names the floor as the reason |
| **C10** | Budget-disable beats configuration | same shape, for the hard-budget disable in `Pipeline::charge` |
| **C11** | **The snapshot agrees with the TOML** | `docs/CONFIG.md` "Snapshot" + PERFORMANCE technique 7. Write a TOML, regenerate, run `lessr hook` → the effective config equals the TOML. Then **edit the TOML and re-run without regenerating**: the hook must see the new value or refuse the snapshot. It must never silently apply the old one |
| **C12** | A snapshot the hook does not understand is refused | bad magic, a future version, a truncated file, a zero-length file, a snapshot from another platform → compiled defaults, no panic, no misread. "Misreading old bytes as new ones would silently change what a mechanism does" is stated in `docs/CONFIG.md`; this makes it a test |
| **C13** | The hook parses no TOML and opens no DB | assert with `strace`/`dtruss`, or a build-time check that `toml` and the SQLite crate are not in `lessr hook`'s reachable graph. The end-to-end bench gate (`bench/overhead.rs`) catches the cost; this catches the cause |
| **C14** | A bad config is reported, never fatal | a value that does not parse falls back to its default **and is reported by `lessr config`**; a key nothing reads is reported too; neither is ever fatal on the hook path. Three separate assertions, all stated in `docs/CONFIG.md` |
| **C15** | `lessr config --explain` tells the truth | for a generated config, the layer it names for each value is the layer a reference resolver picked. Property test: generate a random layer stack, resolve both ways, compare value **and** provenance |
| **C16** | Config writes round-trip | `on`/`off`/`shadow`/`level`/`config set` write a file that reads back identically, preserve the user's comments and key order, and regenerate the snapshot in the same operation (`docs/CONFIG.md`: "All of them write `config.toml` and regenerate the snapshot") |

C11 and C15 are the two to build first. A stale snapshot silently ignoring a config change is
an invisible bug class — the user turns something off, the hook keeps doing it, and nothing
says so. And `lessr config --explain` is the *only* thing standing between a six-layer
resolver and an unfalsifiable "it must be a config problem" support load; if it can lie,
the layering is worse than no layering.

### 6.2 Which level the matrix is tested at

| Tier | Level(s) | Why |
| --- | --- | --- |
| T0 fixtures | `balanced`, plus the C5 cells | C5 needs one triple per (mechanism, level); everything else would triple for nothing |
| T1 properties | **all three**, quantified | C3 and C4 are properties *over* the level axis; this is where level coverage actually lives |
| T2 replay | `balanced` per-commit; **all three** nightly | the nightly run is also how the three levels' savings curves get published — `lessr replay run --level` exists for this |
| T3 fake provider | `balanced` | the proxy path is level-agnostic except through the stages |
| T4 real agent | `balanced` | a real-agent run is expensive in wall time; one level, plus `aggressive` once per release |
| T5 `lessr bench` | `balanced` **and** `aggressive` as separate arms | LOOP_SAFETY 8: if `aggressive` raises tool calls per task or lowers task success, it does not ship. Only this tier can measure that, and it is the whole reason `aggressive` needs a gate rather than a warning |

So the §10 matrix cells are **`balanced` unless marked**, and the level axis is covered by
properties (T1), the C5 fixtures (T0) and the nightly replay — not by multiplying the matrix
by three.

---

## 7. Pro-specific verification

Pro stages ship in `Shadow` first (`docs/product/MECHANISMS_PRO.md`, LOOP_SAFETY 6) and each
must produce a "would have saved" number for the free receipt's *left on the table* column.
The question is how that number is kept honest.

### 7.1 V1 — Shadow/Active replay equivalence (free, deterministic, per-commit)

Run the same capsule twice: stage in `Shadow`, then `Active`. The Shadow-claimed
`bytes_before`/`bytes_after` must **equal** the Active-measured ones.

This catches the most likely bug in the whole Pro tier, and it is latent in the code today.
`Pipeline::run_tool_result` runs a `Shadow` stage against `result.clone()` and an `Active`
stage against the live result. So in a multi-stage pipeline:

- **Active:** stage *k* sees the output of stage *k−1*.
- **Shadow:** stage *k* sees the *unfiltered* input, because *k−1* threw its rewrite away.

Therefore **shadow savings are not additive**, and V1 must compare like with like: one stage
at a time, every other stage `Off`.

### 7.2 V1b — Overlap, and how the column is computed

Two shadow stages can both claim the same bytes. Gate:

```
∀ pairs (A,B):  bytes_saved_shadow(A) + bytes_saved_shadow(B)
                  ≥ bytes_saved_active(A ∘ B)      # sum over-claims, or ties
```

**Design decision this document makes, because `MECHANISMS_PRO.md` does not:** the receipt's
*left on the table* **total** is computed by running **all** Pro stages `Active` on the
replay and taking the joint result — never by summing per-stage shadow numbers. Per-stage
lines may be shown, with a note that they overlap; the total is the joint figure. Summing
shadow numbers is precisely how that column becomes a lie, and the README's sample receipt
lists five Pro lines that visibly overlap (full pack and map-then-zoom both cut read output).

### 7.3 V2 — Counting-rule conformance (per-commit)

Each rule in `docs/product/MECHANISMS_PRO.md` becomes an executable assertion on the corpus.

| Mechanism | Assertion |
| --- | --- |
| cachefix | claim `== Σ cache_write(broken turns)×write_rate − Σ(same turns)×read_rate`; **zero** on a capsule with no breaks; **zero** for breaks `detect` classified `unknown` (the contract says known causes only) |
| keepalive | claim `== avoided write on return turn − ping cost`; the stage stops when expected pings > 12 (break-even); never a positive claim on a session it chose not to ping |
| compact | **zero** on every turn where keepalive reports warm (rule 7); never touches the last N turns |
| mcp | claim must *decrease* on a capsule where a rare tool is used — the extra `find_tool` call is subtracted |
| zoom | `shadow_claim ≤ active_claim` on the same capsule, per language. This is the "conservative in Shadow" mark made checkable |
| delta | full-output bytes − delta bytes, on a capsule with a repeated command; zero on first run |
| editverify | zero unless the preceding turn was an `Edit` on the same path |
| batch | zero unless the capsule carries an explicit `--batch`/CI marker; **never inferred from idle time** |
| prefix | avoided `cache_write` on first turns only |
| rent | reconciles: `Σ per-read lifetime cost == Σ(write + reads)` from the turns table |
| router | price difference on routed turns only; quality **never** folded into the guaranteed number |

### 7.4 V3 — Silent sampling and the 10 % target

`docs/product/CLOUD.md`: "1-in-50 sessions (consent required) run one mechanism in Shadow;
the difference between accounted and observed feeds the per-mechanism verified ratio."
`docs/business/ROADMAP.md` phase 5 exit: *"'Left on the table' lines verified within 10 % by
silent sampling."*

**The design problem:** you cannot observe the saving of a mechanism you did not run. So
sampling has to be a **two-armed, session-level A/B**, not one-sided shadowing:

```
cohort  = (repo_hash, agent, provider shape, model)
arm A   = M Active     → observed_rate = saved_tokens / input_tokens, per turn
arm S   = M Shadow     → claimed_rate  = claimed_tokens / input_tokens, per turn
ratio_M = mean(observed_rate | A) / mean(claimed_rate | S)
gate    = |ratio_M − 1| ≤ 0.10
```

**Statistical discipline, or "within 10 %" is noise:**

- Minimum **200 sessions per arm per mechanism per cohort** before a ratio is shown.
- Bootstrap **95 % CI** over sessions, not turns (turns within a session are correlated).
- The badge shows the **interval**, not a point.
- A mechanism whose CI is wider than ±10 % shows **"not yet verified"** — never a rounded
  point estimate. This is `docs/VISION.md`'s "Honest numbers… red rows included" applied to
  the badge, and the dashboard must not be able to render it any other way (assert it in
  the dashboard's own tests).

**Offline pre-flight.** The identical computation runs on `bench/replay/corpus/` before any
cloud data exists, per mechanism. The pre-release gate is `|ratio_M − 1| ≤ 0.10` on the
corpus. That makes phase 5's exit criterion testable today, with no users.

**Privacy assertion.** The sampling uses only the default metadata payload from
`docs/product/CLOUD.md` — `{stage, bytes_before, bytes_after, tokens_est, exact, mode}` plus
turn usage. A test serialises the event type and **fails if any field can hold content
bytes**. Type-level, so it cannot rot.

### 7.5 V4 — The honesty property

> A shadow saving may never be claimed for a turn on which the stage would not have run in
> Active.

Not-run cases: `Mode::Off`, over the hard budget, disabled by the self-healing table for
that repo, `explicit_selection` set (LOOP_SAFETY 1), or — for `compact` — the cache warm.
Property test over generated pipelines and generated turn states. This is the single
easiest way for *left on the table* to inflate, and it inflates in the direction that
sells upgrades, which is exactly why it needs a test rather than a review.

---

## 8. The competitor bench arms

`bench/README.md` and `docs/business/LAUNCH.md` commit to publishing **baseline vs Lessr vs
caveman vs ponytail vs rtk** on the same 40 tasks, red rows included — and to sending the
harness to independent benchmarkers who will re-run it. A comparison that cannot be
reproduced by a stranger is not a benchmark; it is a claim.

### 8.1 Reproducibility requirements

Each arm is pinned and recorded, in the results file, so any number is attributable:

| Field | Example |
| --- | --- |
| Arm name | `ponytail` |
| Version | exact tag or commit SHA, plus a lockfile hash |
| Install method | `bench/arms/<arm>/install.sh`, network-pinned, checksummed |
| Settings | the exact persona text / rule file / flags, committed verbatim under `bench/arms/<arm>/` |
| Agent + version | Claude Code `x.y.z` |
| Model | pinned model id, not an alias |
| Task set | `bench/tasks/<set>.toml` at a commit SHA |
| Repo | pinned public repo at a commit SHA |
| Date, operator, cost | in the results header |

`bench/arms/<arm>/` holds everything needed to reconstruct that arm from nothing. If an arm
cannot be pinned (a hosted service that changes under us), it is published with a date and
an explicit "not reproducible after this date" note, or it is not published.

### 8.2 How prompt/persona arms are run fairly

caveman and ponytail are prompt/persona-based: they change how the model is *asked* to
behave. The fair comparison injects their prompt into the identical task set through our own
proxy, so every arm differs in exactly one thing.

**Hard architectural constraint.** That injection lives in **the bench harness only** and
must **never** be registerable as a `lessr_core::Stage`.

- `docs/VISION.md`: *"Not a persona. Nothing in Lessr tells the model to be brief."*
- `docs/VISION.md`: *"Behavioural savings are someone else's product."*

If persona injection were a `Stage`, it could be registered into a real user's pipeline by
configuration, by a Pro crate, or by accident. So:

1. The injector lives under `bench/arms/prompt_inject.rs`, in the `lessr-bench` crate
   (`publish = false`), and **does not implement `Stage`**. It is a `lessr-fake`-side or
   harness-side request rewriter, not a pipeline stage.
2. A test asserts it: a compile-time or reflective check that no type outside
   `crates/lessr-*` implements `Stage`, and a grep-level CI check that `bench/` contains no
   `impl Stage`. Cheap, and it makes the boundary load-bearing rather than cultural.
3. `lessr-bench` stays out of `lessr-cli`'s dependency graph, so nothing persona-shaped can
   reach a shipped binary.

### 8.3 Tier and cadence

**T5, pre-release only.** Real agents, real models, real money. It cannot be per-commit CI
at any sane cost, and pretending otherwise produces a gate people disable.

| | |
| --- | --- |
| When | Pre-release, `docs/RELEASE.md` step 2; and for the launch post (`LAUNCH.md` step 1) |
| Arms | baseline, lessr (Balanced), lessr (Aggressive), caveman, ponytail, rtk |
| Cost | ~6 arms × 40 tasks; budget in the results header; expect low hundreds of dollars |
| Output | `bench/results/<version>.md`, red rows included, every field from §8.1 |
| Metrics | cost, task success, wall time, tool calls per task — per `bench/README.md` |

The rtk arm is special: `docs/ADAPTERS.md` requires Lessr to *chain behind* rtk rather than
replace it, so a fourth composite arm — **rtk + lessr** — is worth running. It is the
configuration a real user with rtk already installed will end up in, and it is the one that
would embarrass us if the two tools interact badly.

---

## 9. What runs where

| Cadence | Jobs | Rough runtime |
| --- | --- | --- |
| **Per commit** | fmt · clippy `-D warnings` · `cargo test --workspace` (T0 + T1 at 256 cases) · pipeline bench gate · end-to-end hook bench gate · T2 replay on 5 corpus capsules with `--against baseline.json` · T3 fake-provider integration · adapter G1/G2/G5 matrix on fake `HOME`s · P12 network-blocked run · C1–C12 config gates | **≤ 12 min** wall, jobs in parallel; matches today's two-job shape in `.github/workflows/ci.yml` |
| **Nightly** | T1 properties at 10 000 cases · `cargo-fuzz` 30 min per target (hook parse, filter, SSE parse) · T2 full corpus (50+ capsules) at all three levels · T3 chaos scenarios (malformed frames, stalls, mid-stream kills) · T4 real CLI agents against the fake provider · cross-platform (macOS, Linux musl, Windows) · ASan/valgrind on the long-lived proxy | **≤ 90 min** |
| **Pre-release** | full T2 corpus `--against` the previous tag · §3.4 manual GUI checklist · Pro shadow-honesty gate (V1, V1b, V2, V3 offline) · T5 `lessr bench` with all arms (§8) · install-from-release-artefact smoke per platform · **upgrade path**: `uninstall` byte-for-byte on a machine that had the *previous* version installed | **hours**, partly manual |
| **Monthly** | agent conformance sweep (§3.4); staleness warnings expire at 60 days | ~2 h of a person's time |
| **Quarterly** | price-table refresh, with a test that every model id in the corpus exists in the bundled table | ~1 h |

Existing CI already covers row 1's first four items. The rest are additive jobs, not a
restructure.

---

## 10. The matrix

**Legend.** `A` automated · `M` manual · `N` not applicable · `?` capability unconfirmed.
Subscript is the highest tier that proves the cell. Cells are `Balanced` level (§6.2).

### 10.1 Agent × provider × path

| Agent | Path | Anthropic `/v1/messages` | OpenAI `/v1/chat/completions` | OpenRouter / OAI-compatible |
| --- | --- | --- | --- | --- |
| **Claude Code** | hook | `?`→`A` T4 | `?`→`A` T4 | `?`→`A` T4 |
| | proxy | `A` T4 | `A` T3 | `A` T3 |
| **Gemini CLI** | hook | `N` | `A` T4 | `A` T4 |
| | proxy | `N` | `A` T4 | `A` T3 |
| **OpenCode** | hook | `A` T4 | `A` T4 | `A` T4 |
| | proxy | `A` T4 | `A` T4 | `A` T3 |
| **pi** | hook | `A` T4 | `A` T4 | `A` T4 |
| | proxy | `A` T4 | `A` T4 | `A` T3 |
| **Codex** | hook | `N` — installs, cannot rewrite output | `N` (install `A` T0) | `N` |
| | proxy | `N` | `A` T4 | `A` T3 |
| **Cursor** | hook | `N` — `postToolUse` schema unconfirmed | `N` | `N` |
| | proxy | `M` | `M` | `M` |
| **Windsurf** | hook | `N` | `N` | `N` |
| | proxy | `M` | `M` | `M` |
| **Cline** | hook | `N` — no hooks | `N` | `N` |
| | proxy | `M` | `M` | `M` |
| **Roo** | hook | `N` — no hooks | `N` | `N` |
| | proxy | `M` | `M` | `M` |
| **Kilo** | hook | `N` — no hooks | `N` | `N` |
| | proxy | `M` | `M` | `M` |
| **omp** | hook | `?` via pi | `?` via pi | `?` via pi |
| | proxy | `A` T3 | `A` T3 | `A` T3 |
| **Rakazo** | hook | `N` — server deployment | `N` | `N` |
| | proxy | `A` T3 | `A` T3 | `A` T3 |
| **generic SDK** | hook | `N` — no config to patch | `N` | `N` |
| | proxy | `A` T3 | `A` T3 | `A` T3 |

Gemini CLI's Anthropic column is `N` because it speaks Google's own shape; it reaches the
proxy via the OpenAI-compatible endpoint. Gemini's *native* API shape is a third shape the
proxy does not claim to parse — `lessr_core::Provider::Other` exists for exactly this, and
the honest matrix entry is that usage capture is unavailable there until a parser ships.

### 10.2 Gates, per agent

| Agent | G1 detect | G2 install | G3 intercept | G4 filter | G5 uninstall |
| --- | --- | --- | --- | --- | --- |
| Claude Code | `A` T0 | `A` T0 | `?` T4 | `?` T4 | `A` T0 |
| Gemini CLI | `A` T0 | `A` T0 | `A` T4 | `A` T4 | `A` T0 |
| OpenCode | `A` T0 | `A` T0 | `A` T4 | `A` T4 | `A` T0 |
| pi | `A` T0 | `A` T0 | `A` T4 | `A` T4 | `A` T0 |
| Codex | `A` T0 | `A` T0 (TOML block) | `A` T4 (asserts **empty** stdout) | `A` T4 proxy only | `A` T0 |
| Cursor, Windsurf | `A` T0 | `N` | `M` | `M` | `N` |
| Cline, Roo, Kilo | `A` T0 | `N` | `M` | `M` | `N` |
| omp | `A` T0 | `N` (use pi) | `?` T4 | `?` T4 | `N` |
| Rakazo | `A` T0 | `N` | `A` T3 | `A` T3 | `N` |
| generic SDK | `A` T0 | `N` | `A` T3 | `A` T3 | `N` |

G2 and G5 are `N` for the unverified agents because `lessr init` deliberately writes nothing
for them (`agents::Unverified`) — the cell is "correctly does nothing", already asserted by
`an_unverified_agent_is_told_about_by_hand_and_never_written_to`.

Codex is the row that shows why G2/G5 and G3/G4 are separate gates: its install is fully
verified and fully automatable while its filtering is impossible. Reporting one number per
agent would have to round that to either "works" or "does not", and both are wrong. G2 also
needs a Codex-specific assertion the JSON agents do not: the config is edited **as text
between two marker comments**, never parsed and re-serialised, because TOML round-trips lose
comments — so the golden test asserts that every byte outside the block is unchanged, and
that the result parses as TOML before it is ever proposed.

### 10.3 Which cells change if the gate moves to the proxy

If an agent's hook cannot replace tool output, the only way to shrink that output is to
rewrite the trailing tool-result block of the *next* request, on the proxy path via
`Stage::on_request`. Consequences, in order of importance:

1. **It collides with invariant 4.** `AGENTS.md`: "Never mutate a request while the cache is
   warm", and "Nothing in the free engine mutates requests". A trailing tool result normally
   sits *after* the last cache breakpoint, so mutating it is prefix-safe on the turn it is
   written — but it becomes part of the prefix on the following turn. So it is safe **only
   if the same tool result always filters to exactly the same bytes**. P8 (idempotence) and
   determinism stop being nice-to-have and become the safety argument.
2. **Under the invariant as written today, gate-on-proxy is a Pro mechanism, not a free
   one.** This is a product decision to surface, not one this document should make. Either
   invariant 4 gets an explicit, narrow carve-out — *trailing tool result, byte-stable,
   same input ⇒ same output* — or the agents without output-rewriting hooks get no gate in
   the free engine at all.
3. **Testability improves.** Every `M` in the proxy rows of §10.1 for Cursor, Windsurf,
   Cline, Roo and Kilo becomes `A` at T3/T4 — because the proxy path needs the agent to
   honour a `base_url` and nothing else. Nine agents move from manual to automated. The
   trade is: worse for the invariant, much better for verification.
4. **Codex's row is already this case.** Its cells are proxy-only today, so it is the
   working model for what the others would look like.
5. **detect, cachefix, keepalive, prefix, batch, router are unaffected** — proxy-only
   already. **trap and dedup follow the gate**, since they are hook-path stages in the same
   position (`docs/ARCHITECTURE.md`: `trap → gate → dedup`).

The plan works under either outcome: every gate in §3.1 is defined against *behaviour*
(the agent's next request to the provider), not against a mechanism, so G3 and G4 test
whichever path a given agent ends up using.

---

## 11. Contradictions and gaps found, with recommendations

Recorded here because §10 cannot be filled in honestly while they stand.

### 11.1 PreToolUse vs PostToolUse — the gate cannot currently fire

`docs/ADAPTERS.md`, `docs/ARCHITECTURE.md` (both the diagram and the "Hook path" paragraph)
and `CLAUDE.md` all say the Claude Code hook is a **`PreToolUse`** hook. The adapter writes
into `hooks.PreToolUse` (`PRE_TOOL_USE`, `groups_mut`, `unpatch` in
`crates/lessr-adapters/src/agents/claude_code.rs`). But `read_payload` in the same file
requires `tool_response`, which only a **`PostToolUse`** payload carries — and its own
comment says so: *"Claude Code sends `hook_event_name`, `tool_name`, `tool_input`, and on
`PostToolUse` also `tool_response`."*

So an install as it stands registers the hook on an event whose payload has nothing to
filter. The result is a permanent, silent passthrough: correct behaviour by the hook
contract, zero saving, and nothing anywhere says so.

The rest of the tree already assumes Post: `bench/overhead.rs` gates on
`fixtures/hook/claude_post_tool_use.json` and `bench/budget.toml` names the fixture
`claude-post-tool-use`. And the two adapters written since agree — Gemini CLI's `Layout`
uses `event: "AfterTool"`, the post-tool event, because that is the one that can replace a
result. Codex's uses `PreToolUse` *deliberately*, for a hook that observes and prints
nothing. So Claude Code is the only adapter whose event and whose parser disagree.

**Recommendation.** The gate is a post-tool mechanism. Change `claude_code::LAYOUT.event` to
`PostToolUse`, and fix `docs/ADAPTERS.md`, `docs/ARCHITECTURE.md` and `CLAUDE.md` to match
(`CLAUDE.md` currently tells agents to test the hook against
`fixtures/hook/claude_pre_tool_use.json`, which is the fixture that *cannot* exercise a
filter). Keep `PreToolUse` for stages that rewrite tool *input*.

**And add the test that would have caught it**, for every adapter at once:

```
for each Verified adapter:
    assert parse_hook(<a payload of the event this adapter installs>).is_some()
        OR the adapter documents that it is observe-only (Codex)
```

Two lines of generic test across the registry. The class of bug — "we hooked the event whose
payload we cannot use" — is invisible to every other test in the tree, because a passthrough
is indistinguishable from correct behaviour on the hook contract.

### 11.2 Claude Code's ability to replace tool output is unconfirmed

The free gate's entire value rests on it, and `docs/ADAPTERS.md`'s hook contract ("Output:
the same JSON with the tool result replaced") states it as settled for every agent. It is
confirmed only for Gemini CLI, OpenCode and pi — and each of those does it *differently*,
which is why `render_hook` was introduced. Codex explicitly cannot; Cline, Roo and Kilo have
no hooks. Claude Code is the assumption.

**Recommendation.** Make this the first T4 run, before any filter work. If it turns out the
output cannot be replaced, §10.3 is the plan, and it needs a product decision on invariant 4
before phase 2 rather than after. Until confirmed, `docs/ADAPTERS.md`'s hook contract should
be stated per agent, not globally.

### 11.3 `lessr bench` names two different things

`docs/TESTING.md` and `bench/README.md` define `lessr bench` as the paired A/B harness with
real agents and real money. `AGENTS.md` and `docs/VISION.md` say no LLM calls anywhere in
this codebase, ever. These reconcile — the harness drives an external agent — but nothing
says so, and the natural reading is a contradiction.

**Recommendation.** One sentence in `docs/TESTING.md`. And keep the offline simulator under
a different verb: `lessr replay`, per §4.6.

### 11.4 `detect`'s "exact" claim is Anthropic-only

`docs/MECHANISMS.md`: *"Counted: `cache_creation_input_tokens` on turns where the prefix
changed, at the provider's write rate. Exact."* That field is Anthropic's. The OpenAI shape
reports cached input under `prompt_tokens_details.cached_tokens` and has **no cache-write
field at all**; OpenRouter passes through whatever the upstream gave, which varies by
upstream.

So on the OpenAI shape the cache-break saving is not exact — it is unavailable. It must
render as "unknown", never as `$0.00` and never as an estimate wearing an exact mark.

**Recommendation.** Split the counting rule by provider in `docs/MECHANISMS.md`. Add the
"no cache fields" and "`cached_tokens` only" scenarios to the fake provider (§2.3) so the
degradation is tested rather than assumed. The same split applies to `cachefix`, `keepalive`
and `prefix` in `docs/product/MECHANISMS_PRO.md`, all of which claim "exact… from usage
fields".

### 11.5 The README prints the gate's saving as "exact"

`docs/MECHANISMS.md` says gate tokens are estimated and *"Marked as estimate."*
`docs/RECEIPT.md`'s sample prints `tool-output gate (base pack)  $ 2.10  exact bytes, est.
tokens` — correct. `README.md`'s sample prints `tool-output gate  $ 2.10  exact` — not
correct, on the project's own rule.

**Recommendation.** Fix the README, and make it a test: a golden-file check that no receipt
line marked `exact` has an estimated token count underneath it (P11, §5). The receipt is the
product's honesty claim; it should be gated, not proofread.

### 11.6 The hook startup budget in the docs and in `budget.toml` disagree

`docs/ARCHITECTURE.md`: *"One process spawn per call: keep startup under 2 ms."*
`bench/budget.toml` `[end_to_end]` allows p50 5 ms, p99 10 ms — and its own comment explains
why (CI runners are slower and noisier than a dev box, measured p50 1.70 / p99 2.33 ms).

Not a bug — a documented-tight-number versus a deliberately-loose-gate. But a reader will
think the gate enforces the doc, and it does not.

**Recommendation.** State both numbers in `docs/PERFORMANCE.md`: the target (2 ms, on a dev
box) and the gate (5/10 ms, on CI). Tighten the gate once a few real CI runs show the spread,
as `budget.toml` already says it intends to.

### 11.7 `docs/ADAPTERS.md`'s agent table is behind the adapters

The table says Windsurf/Cline/Roo/Kilo get a "rules file + optional hook"; the adapters now
record that Cline, Roo and Kilo have **no hook mechanism at all** and that a rules file
injects instructions into the prompt — which `docs/VISION.md` explicitly says is not what
Lessr does. It gives Codex a hook running `lessr hook codex`; Codex's hook is real but
cannot replace output, so `lessr init` skips it unless asked by name. It puts pi, omp and
Rakazo on one row; Rakazo turned out to be a server deployment, not a harness.

**Recommendation.** Regenerate the `docs/ADAPTERS.md` table from the registry rather than
maintaining it by hand — every fact in it (`display_name`, config path, mechanism,
confidence, whether output replacement is possible) is already a value in
`crates/lessr-adapters/src/agents/`. Then a test asserts the doc matches the code, and the
matrix in §10 can be generated the same way. A hand-maintained capability table in front of
thirteen independently-moving adapters will be wrong again within a release.

### 11.8 Overlapping shadow claims are unspecified

`docs/product/MECHANISMS_PRO.md` gives each mechanism a counting rule but never says how the
*left on the table* total is formed from them. The README's sample receipt lists five Pro
lines that visibly overlap (the full pack and map-then-zoom both cut read output).

**Recommendation.** §7.2 makes the call — total = all Pro stages `Active` on the replay,
jointly. Write it into `MECHANISMS_PRO.md` so it is a contract rather than a decision this
document made on its own.

---

## 12. Build order

Each step is independently useful and each gates the next. Steps 1–4 need no mechanism to
exist, which is the point: build the meter before the thing it measures, the way
`crates/lessr-core/benches/pipeline.rs` already gates a budget with a stand-in filter.

| # | Build | Unblocks | Rough size |
| --- | --- | --- | --- |
| 1 | **Adapter conformance harness** (G1/G2/G5 for all 13 agents, fake `HOME`) + **P10** | the whole left half of the matrix, today, with no mechanism shipped | 1–2 days |
| 2 | **`lessr-fakeprovider`** with the §2.3 scenario table and `/__fake/received` | T3 and T4; every proxy-path cell; P5/P6 | 3–4 days |
| 3 | **Capsule format + `lessr replay run`** over a hand-built corpus | T2, the regression gate, the dry-run report | 3–4 days |
| 4 | **Property suite** P1–P3, P7–P9, P12 against a stand-in filter | the safety floor exists before the filters do | 2 days |
| 5 | **T4 harness, Claude Code first** — resolves §11.1 and §11.2 | the free gate's whole design | 2–3 days |
| 6 | **`lessr replay import --agent claude`** | the user-facing dry run; real corpus data | 2 days |
| 7 | **Config gates C1–C16** as `Level`/`Settings`/snapshot land (`docs/CONFIG.md`) | §6; the safety floor across levels; `--explain` provenance | 3 days |
| 8 | **Manual checklist + `docs/conformance/`** + the 60-day staleness warning | the honest half of the matrix | 1 day |
| 9 | **Pro gates V1, V1b, V2, V4** offline on the corpus | Pro stages can ship in Shadow with a number that is checked | 3 days |
| 10 | **V3 offline pre-flight** on the corpus | ROADMAP phase 5's exit criterion, testable with no users | 2 days |
| 11 | **`lessr bench` arms** (§8), pinned and reproducible | LAUNCH step 1 and 3; the only tier that proves loop-safety rule 8 | 1 week + budget |

Steps 1 and 2 are the highest return: together they turn most of §10 from `M` into `A`
before a single filter exists.
