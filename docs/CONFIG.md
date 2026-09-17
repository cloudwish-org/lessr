# Configuration

Every mechanism has a kill switch, a level and its own settings. Nothing is
all-or-nothing, and nothing is hidden: `lessr config` prints the value in
force and where it came from.

`~/.config/lessr/config.toml` (XDG; `~/Library/Application Support/lessr` on
macOS; `%APPDATA%\lessr` on Windows). The hook path never reads it — see
[Snapshot](#snapshot).

## The three axes

| Axis | Values | What it decides |
| --- | --- | --- |
| `mode` | `off`, `shadow`, `active` | Whether the stage runs, and whether its output is applied |
| `level` | `safe`, `balanced`, `aggressive` | How much it cuts |
| settings | per stage | The thresholds behind the level |

`shadow` runs the stage, counts the saving and throws the rewrite away. Every
new mechanism ships in it (loop-safety 6); it is what fills the "left on the
table" column of the receipt.

## What a level may never do

A level changes **how much** a mechanism cuts. It never changes **whether the
safety checks run**. At `aggressive`, exactly as at `safe`:

- Error lines pass (invariant 2). `guard::errors_preserved` runs after every
  filter at every level.
- Every cut leaves a handle (invariant 3).
- Content the agent asked for by range or search term is returned exactly
  (loop-safety 1).
- No mechanism forces a second tool call to get what one call used to return
  (loop-safety 4).

There is no level, setting or config file that turns any of those off. A
mechanism that needed one would be a mechanism that does not ship.

## Levels, per mechanism

Counting rules live in [MECHANISMS.md](MECHANISMS.md); this is only intensity.

### gate
| Level | Cuts |
| --- | --- |
| `safe` | ANSI escapes, progress redraws, trailing whitespace. Nothing that was ever a distinct visible line. |
| `balanced` | + consecutive duplicate lines as `×N`, passing test lines as counts, log timestamps, `node_modules` subtrees |
| `aggressive` | + non-consecutive duplicates, and a tail-only summary past `summary_after_lines` |

`max_repeated_lines` (3) · `summary_after_lines` (400) · `strip_timestamps` (true)

### trap
| Level | Traps |
| --- | --- |
| `safe` | Binaries and lockfiles |
| `balanced` | + minified assets, sourcemaps, base64 blobs, generated code, JSON over `json_bytes` |
| `aggressive` | + anything over `max_bytes`, and `json_bytes` halved |

`json_bytes` (65536) · `max_bytes` (262144)

### dedup
| Level | Answers a re-read with |
| --- | --- |
| `safe` | Unchanged → one hash line. Changed → the file, whole. |
| `balanced` | + changed → unified diff with `context_lines`, plus a handle |
| `aggressive` | + `context_lines` 0 and unchanged hunks elided |

`context_lines` (3) · `min_bytes` (2048)

### detect
Report-only in this repo (invariant 4), so the level is how hard it looks.

| Level | Reports |
| --- | --- |
| `safe` | That the prefix changed, and what it cost |
| `balanced` | + the first differing byte, classified: timestamp, random id, tool order, message reorder |
| `aggressive` | + attribution across a rolling window of turns |

Pro mechanisms take the same three axes. Their level semantics are in
`docs/product/MECHANISMS_PRO.md` in the Pro repository.

## Layers

Lowest to highest. The last one to set a value wins.

1. Compiled-in defaults
2. `[stages.default]`
3. `[stages.<name>]`
4. `[repo."<path>".stages.default]`
5. `[repo."<path>".stages.<name>]`
6. Environment: `LESSR_STAGE_GATE_MODE=off`, `LESSR_STAGE_GATE_LEVEL=safe`

Above all of them sits the **safety floor**: a filter the self-healing table
has turned off for a repo (loop-safety 5) stays off, and no layer can promote
it. Configuration tunes a mechanism; it does not overrule the evidence that the
mechanism is hurting this repo.

```toml
[stages.default]
mode = "shadow"

[stages.gate]
mode = "active"
level = "balanced"
max_repeated_lines = 3

[stages.trap]
mode = "active"
level = "safe"
json_bytes = 131072

# This repo generates enormous fixtures the agent genuinely needs to read.
[repo."/home/dev/work/monorepo".stages.gate]
mode = "off"
```

A value that does not parse falls back to its default and is reported by
`lessr config`. A key nothing reads is reported too: a typo that silently does
nothing is worse than a typo that says so. Neither is ever fatal — a bad config
line must not break someone's agent.

## Snapshot

The hook path runs on every tool call and owes the agent 2 ms, so it parses no
TOML (see technique 7 in [PERFORMANCE.md](PERFORMANCE.md)). `config.toml` is
compiled into a versioned binary snapshot under the config directory, which the
hook memory-maps and reads without allocating. It is regenerated whenever the
TOML changes.

A snapshot whose magic or version is not recognised is refused, and the
defaults are used. Misreading old bytes as new ones would silently change what
a mechanism does.

## Commands

| Command | Does |
| --- | --- |
| `lessr on <stage>` / `lessr off <stage>` | The kill switch |
| `lessr shadow <stage>` | Count it, do not apply it |
| `lessr level <stage> <level>` | `safe`, `balanced` or `aggressive` |
| `lessr config` | Every value in force, and which layer set it |
| `lessr config --explain <stage>` | Why this stage is doing what it is doing |
| `lessr config set <key> <value>` | Edit without opening the file |

All of them write `config.toml` and regenerate the snapshot. `--repo` scopes any
of them to the current repository.
