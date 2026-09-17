# Code style

- `cargo fmt` default; `-D warnings` on default clippy lints.
- Errors: `thiserror` in libraries, `anyhow` only in the binary.
- Public items documented with `///`. Every crate has a `//!` header stating its stage, path and counting rule.
- No `unsafe` outside `lessr-core::mmap` and `lessr-packs::mmap`; each block carries `// SAFETY:`.
- Tests beside code; fixtures under `fixtures/`. Property tests for filters: the guard invariant must hold for arbitrary input.
- Feature flags only for optional dependencies, never for tiering. Tiering is crates.
