//! The `lessr` binary, as a library.
//!
//! Stage: none. Path: both. Counting rule: none of its own.
//!
//! [`run`] takes a [`PipelineBuilder`] instead of building one, and that is the
//! entire seam between the free binary and the Pro binary: the Pro binary
//! registers its extra stages on the builder and calls this same function.
//! Nothing in this workspace knows whether that happened, depends on a Pro
//! crate, or is gated behind a feature flag. Tiering is crates.

mod cli;
mod hook;
mod install;
mod pro;

use std::process::ExitCode;

use clap::Parser;
use lessr_core::PipelineBuilder;

pub use cli::{Cli, Command, ConfigAction};

/// Exit code for a command that exists but whose mechanism has not shipped.
/// Distinct from 1 so a script can tell "not yet" from "went wrong".
const NOT_YET: u8 = 2;

/// Parse the command line and do what it says.
///
/// The caller supplies the stages; see the module header.
pub fn run(builder: PipelineBuilder) -> ExitCode {
    let cli = Cli::parse();

    // The hook path returns before anything else can allocate, open a file or
    // parse a config: it runs on every tool call and owes the agent 2 ms.
    if let Command::Hook { agent } = &cli.command {
        let mut pipeline = builder.build();
        return hook::run(agent, &mut pipeline);
    }

    match dispatch(cli.command) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("lessr: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Everything that is not the hook path.
fn dispatch(command: Command) -> anyhow::Result<ExitCode> {
    match command {
        Command::Hook { .. } => unreachable!("handled before dispatch"),

        Command::Init { show, yes, agent } => install::init(show, yes, agent),
        Command::Uninstall { show, yes, agent } => install::uninstall(show, yes, agent),

        Command::Pro => {
            print!("{}", pro::page());
            Ok(ExitCode::SUCCESS)
        }

        // Below: the command surface is stable, the mechanism is not built.
        // Saying so is the only honest option — a `gain` that printed a
        // plausible receipt from no data would be worse than no `gain` at all.
        Command::Serve { .. } => Ok(not_yet(
            "serve",
            "phase 1",
            "the proxy, streaming passthrough and usage capture",
        )),
        Command::Gain { .. } => Ok(not_yet(
            "gain",
            "phase 1",
            "the recorder and the receipt it prints from",
        )),
        Command::Show { .. } => Ok(not_yet(
            "show",
            "phase 1",
            "the session handle store `lessr show` reads",
        )),
        Command::Bench => Ok(not_yet(
            "bench",
            "phase 2",
            "the paired A/B harness described in bench/README.md",
        )),
        Command::On { .. }
        | Command::Off { .. }
        | Command::Shadow { .. }
        | Command::Level { .. }
        | Command::Config { .. } => Ok(not_yet(
            "on/off/shadow/level/config",
            "phase 1",
            "the config file and snapshot these read and write (docs/CONFIG.md)",
        )),
    }
}

/// Report a command whose mechanism has not shipped, and say what is missing.
fn not_yet(command: &str, phase: &str, missing: &str) -> ExitCode {
    eprintln!("lessr: `{command}` is not implemented yet — it needs {missing} ({phase}).");
    eprintln!("       Roadmap: docs/business/ROADMAP.md in the Pro repository.");
    ExitCode::from(NOT_YET)
}
