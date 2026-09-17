# Receipt

`lessr gain` prints what Lessr saved, from local data only.

```
lessr gain — today

  Saved                                     $ 6.40   (21 %)
    tool-output gate (base pack)    $ 2.10   exact bytes, est. tokens
    re-read dedup                   $ 0.95   exact
    trap list                       $ 0.45   exact
    cache-break report              $ 2.90   your fix, verified by usage

  Cache
    turns 40 · prefix stable 31 · breaks 9 (timestamp in system prompt)

  Most expensive read this session
    package-lock.json, turn 9 → $4.10 by turn 60

  lessr gain --explain   shows the method behind every line
```

- Provider usage fields are the truth; estimates are marked.
- Prices come from a bundled per-model table, overridable in config; the receipt names the table used.
- The receipt never leaves the machine unless the user signs in to Lessr Cloud.

## Schema

`sessions(id, agent, repo, started_at)` · `turns(session, n, model, input, output, cache_read, cache_write, prefix_stable)` · `savings(session, turn, stage, bytes_before, bytes_after, tokens_est, mode)` · `handles(session, id, stage, expanded)`
