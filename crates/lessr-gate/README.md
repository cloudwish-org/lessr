# lessr-gate

Stage gate. Order inside: trap → hot filter or pack rule → guard → handle. Hot filters in Rust for git status/diff/log, cargo test/build, pytest, npm/pnpm test, ls/tree; declarative rules for the rest. guard::errors_preserved runs after every filter and fails open. Fixtures per filter: input.txt, expected.txt, errors.txt. Counting: bytes removed, exact; tokens estimated.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
