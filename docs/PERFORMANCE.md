# Performance

Budget: 2 ms soft, 10 ms hard, per stage, on the 1 MB `cargo test` fixture on a 2020 laptop. CI fails above 10 ms p99.

## Techniques, by impact

1. **No allocation per line.** Filters scan `&[u8]` with `memchr` and write into one preallocated `Vec<u8>`. No `String`, no `lines()`, no `to_owned`.
2. **One pass, one `RegexSet` per rule.** Each line is matched once. Never loop over patterns.
3. **`bytes::Bytes` end to end.** Slicing is zero-copy.
4. **blake3** for dedup and handles; **ahash** for maps.
5. **Streaming passthrough.** The proxy never buffers the response; usage is parsed from the final SSE frame as it passes.
6. **Off-hot-path persistence.** Recorder = channel + thread; SQLite WAL, batched transactions.
7. **Hook startup.** Config and packs are memory-mapped snapshots; the hook parses no TOML and opens no database.
8. **mimalloc** as global allocator.
9. **Release profile.** Fat LTO, one codegen unit, `panic = abort`, stripped.

## Benchmarks

- `cargo bench -p lessr-core`: pipeline overhead on 4 KB, 64 KB, 1 MB fixtures.
- `bench/overhead.rs`: end-to-end hook invocation including process spawn.
- CI runs both in `--release` and compares p99 against `bench/budget.toml`.

## Never

- `regex::Regex` per pattern in a loop.
- `serde_json::Value` on the hot path for tool results; parse only needed fields.
- Synchronous I/O inside a stage.
- `println!` in the hook path; output is one `write_all`.
