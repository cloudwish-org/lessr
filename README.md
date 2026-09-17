<p align="center"><strong>Lessr</strong> · Send your model less.</p>
<p align="center">Counts every token it saves. Never touches your model.</p>

---

Your coding agent pays for every token in its context on every single turn. That test output from turn 9 is still billing you at turn 60. Lessr sits between your agent and the API, on your machine, and stops the leak — then shows you the receipt.

```
curl -fsSL https://lessr.dev/install | sh
lessr init        # finds Claude Code, Cursor, Codex, Gemini CLI, OpenCode, pi — one confirm
lessr gain        # two minutes later: your first receipt
```

No account. No card. No LLM in the middle. Works offline.

## Your first receipt

```
lessr gain — today

  Saved by Lessr                              $ 6.40   (21 %)
    tool-output gate                 $ 2.10   exact
    re-read dedup                    $ 0.95   exact
    trap list                        $ 0.45   exact
    cache-break report → your fix    $ 2.90   verified by usage

  Cache
    40 turns · prefix stable 31 · broken 9: a timestamp in your system prompt

  Most expensive read this session
    package-lock.json at turn 9 → $ 4.10 by turn 60

  Left on the table (Lessr Pro would have)    $ 11.80  (39 %)
    cache auto-fix                   $ 3.20   exact
    keepalive across 3 idle gaps     $ 2.70   exact
    full pack, 61 unfiltered cmds    $ 2.40   exact
    map-then-zoom, 14 large reads    $ 2.10   conservative
    delta mode, 9 repeated runs      $ 1.40   exact

  lessr gain --explain   the method behind every line
  lessr pro              upgrade in place, no reinstall
```

Every number is either a byte count or a figure from the provider's own usage fields. Estimates are marked. If a line is ever wrong, that's a bug we fix in public.

## Why not the other token savers

The viral ones advertise 60–90 % and measure −8 % to +7 % in paired A/B tests, because they change how the model *talks* while most tokens are tool output and re-sent history. Lessr never changes model behaviour. It only does things whose saving is arithmetic:

| What Lessr does | How it saves |
| --- | --- |
| **Cache-break detector** | Finds what re-writes your prompt cache every turn (timestamps, tool order, random ids) and tells you the cost |
| **Tool-output gate** | Strips ANSI, progress bars, repeated lines, passing tests, `node_modules` trees before they enter history. Errors always pass. Everything removed is one command away |
| **Re-read dedup** | Unchanged file read twice → one line. Changed → a diff |
| **Trap list** | Lockfiles, minified bundles, base64 blobs never enter history raw |
| **Receipt** | Exact per-session accounting; `lessr gain --share` for the card |

`lessr bench` ships in this repo. Every release publishes its paired results, red rows included.

## It cannot make your session worse

Enforced in code, not in this README:

- Content your agent asked for by path, range or grep term is returned exactly.
- Nothing is deleted, only deferred: `lessr show <id>` returns the original bytes.
- Error, warning, panic and stack-frame lines are never removed. The gate checks after every filter and fails open.
- A filter that gets expanded more than 10 % of the time turns itself off for that repo and says so.
- Every mechanism has a kill switch and ships in shadow mode first.
- `lessr uninstall` restores your agent configs byte-for-byte.

Full contract: [docs/LOOP_SAFETY.md](docs/LOOP_SAFETY.md)

## Lessr Pro

Same binary, more crates, one command. Pro adds cache auto-fix, keepalive across breaks, map-then-zoom reads, delta mode, MCP lazy loading, batch routing, the full pack of 200+ rules updated weekly, a hosted dashboard and the verified savings badge. The free receipt already tells you what it would have saved you today.

```
lessr pro
```

[lessr.dev/pro](https://lessr.dev/pro) · [What's in Pro](docs/PRO.md)

## For developers

Under 10 ms overhead, streaming untouched, single static Rust binary, zero config, self-updating. `lessr-core` and `lessr-gate` are libraries first: embed them in your harness instead of installing a proxy.

- [Vision](docs/VISION.md) · [Architecture](docs/ARCHITECTURE.md) · [Mechanisms](docs/MECHANISMS.md) · [Packs](docs/PACKS.md)
- [Performance](docs/PERFORMANCE.md) · [Adapters](docs/ADAPTERS.md) · [Receipt](docs/RECEIPT.md) · [Config](docs/CONFIG.md) · [Contributing](CONTRIBUTING.md)
- Coding agents working in this repo: start at [AGENTS.md](AGENTS.md)

MIT.
