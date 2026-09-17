# Release

1. `cargo test --release && cargo bench -p lessr-core` green; p99 under budget.
2. `lessr bench` run; results added under `bench/results/`.
3. Bump `workspace.package.version`; tag `vX.Y.Z`.
4. CI builds static binaries (macOS arm64/x86_64, Linux x86_64/arm64 musl, Windows x86_64), signs, publishes to GitHub Releases, Homebrew tap, crates.io, npm shim, winget.
5. The Pro repository bumps its git dependency tag the same day.
