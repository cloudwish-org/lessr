//! The open-source `lessr` binary.
//!
//! This file exists to do one thing: choose which stages the free engine runs,
//! then hand them to [`lessr_cli::run`]. The Pro binary is the same file with
//! more registrations, which is why tiering never needs a feature flag.

use std::process::ExitCode;

use lessr_core::PipelineBuilder;

fn main() -> ExitCode {
    let builder = PipelineBuilder::new();

    // The free stages register here, in pipeline order: trap and gate, then
    // dedup on the hook path, and detect on the proxy path. None of them exist
    // yet — they arrive in phases 1 to 3 — and an empty pipeline is exactly the
    // right behaviour until they do: a passthrough that cannot change a single
    // byte of what the agent sees.

    lessr_cli::run(builder)
}
