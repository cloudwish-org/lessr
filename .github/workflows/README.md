# CI

- `ci.yml`: fmt, clippy -D warnings, test, overhead bench in release with the p99 gate, on macOS and Linux.
- `release.yml`: on tag, build static binaries (macOS arm64/x86_64, Linux x86_64/arm64 musl, Windows x86_64), sign, publish to GitHub Releases, Homebrew tap, crates.io, npm shim, winget.
