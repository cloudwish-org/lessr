# Packs

Filters are data. The engine is open; the rules it runs are packs.

```
packs/base/
  manifest.toml        pack id, version, signature, rule list
  rules/_template.toml
  rules/git-status.toml
  ...
```

## Rule format

```toml
id = "npm-install"
match = { program = ["npm", "pnpm", "yarn"], args_any = ["install", "i", "add"] }

[lines]
drop = ['^\s*$', '^npm (WARN|notice) deprecated', '^\s*(added|removed|changed) \d+ packages? in .*']
keep = ['^npm ERR', 'peer dep', 'ERESOLVE']
collapse = { pattern = '^(warning|WARN) .*', label = "warnings" }

[summary]
tail = 3
counts = ["added", "removed", "vulnerabilities"]
```

- `match`: program and argument predicates; a rule with no match is invalid.
- `lines.keep` wins over `lines.drop`; `guard` runs after both regardless.
- `collapse`: consecutive matches become one line with a count.
- `summary`: appended when content was removed (tail lines, extracted counts, the handle line).

## Compilation

All `drop`, `keep` and `collapse` patterns of a rule compile into `RegexSet`s. Line scanning uses `memchr` for newlines; each line matches once per set. The compiled form is cached under the config dir keyed by pack hash and memory-mapped on the hook path.

## Signing

`manifest.toml` carries an ed25519 signature over the canonical bytes of every rule file. The engine ships the public key; unsigned or altered packs are refused. `cargo run -p lessr-packs -- sign --key <path>` regenerates the manifest.

## Base pack (this repo)

Twenty rules: git status/diff/log/add/commit/push, cargo test/build/clippy, npm/pnpm test/install/run, pytest, go test, ls/tree, grep/rg, cat/head, docker ps/logs, ruff/eslint/tsc.

## Full pack (Lessr Pro)

200+ rules including delta-mode parsers and zoom grammars, updated weekly. Same format, different signature.
