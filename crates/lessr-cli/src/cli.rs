//! The command surface.
//!
//! Every command Lessr will ever have is declared here, including the ones
//! whose mechanism has not shipped. A command that exists but says "not yet"
//! is honest; a command that appears later and changes the shape of everyone's
//! scripts is not.

use clap::{Parser, Subcommand};

/// Send your model less. Counts every token it saves; never touches your model.
#[derive(Parser, Debug)]
#[command(name = "lessr", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// What the user asked for.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Find your agents and install the Lessr hook.
    Init {
        /// Print every change that would be made, then exit without making any.
        #[arg(long)]
        show: bool,
        /// Skip the confirmation prompt. For CI.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Only touch this agent. Defaults to every agent found.
        #[arg(long)]
        agent: Option<String>,
    },

    /// Filter one tool result. This is what your agent calls, not you.
    #[command(hide = true)]
    Hook {
        /// Which agent's JSON shape is on stdin.
        agent: String,
    },

    /// Run the proxy on localhost for agents that take a base_url.
    Serve {
        /// Port to listen on.
        #[arg(long, default_value_t = 7433)]
        port: u16,
    },

    /// Your receipt: what Lessr saved, from local data only.
    Gain {
        /// Show the method behind every line.
        #[arg(long)]
        explain: bool,
        /// Render the shareable card.
        #[arg(long)]
        share: bool,
        /// Leave out the "left on the table" section.
        #[arg(long)]
        no_pro: bool,
    },

    /// Print the original bytes behind a handle.
    Show {
        /// The handle id, as the receipt printed it.
        id: String,
    },

    /// Run the paired A/B benchmark.
    Bench,

    /// Turn a mechanism on.
    On {
        /// The stage name, as `lessr gain` prints it.
        stage: String,
        /// Scope the change to the current repository.
        #[arg(long)]
        repo: bool,
    },

    /// Turn a mechanism off.
    Off {
        /// The stage name, as `lessr gain` prints it.
        stage: String,
        /// Scope the change to the current repository.
        #[arg(long)]
        repo: bool,
    },

    /// Count a mechanism's saving without applying it.
    Shadow {
        /// The stage name, as `lessr gain` prints it.
        stage: String,
        /// Scope the change to the current repository.
        #[arg(long)]
        repo: bool,
    },

    /// Set how hard a mechanism works. A level changes how much is cut, never
    /// whether the safety checks run: errors pass at every level.
    Level {
        /// The stage name, as `lessr gain` prints it.
        stage: String,
        /// `safe`, `balanced` or `aggressive`.
        level: String,
        /// Scope the change to the current repository.
        #[arg(long)]
        repo: bool,
    },

    /// Show or change settings.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
        /// Why is this stage doing what it is doing? Prints every value in
        /// force for it and which layer set each one.
        #[arg(long, value_name = "STAGE")]
        explain: Option<String>,
        /// Scope to the current repository.
        #[arg(long)]
        repo: bool,
    },

    /// Restore every agent config Lessr touched, byte for byte.
    Uninstall {
        /// Print every change that would be made, then exit without making any.
        #[arg(long)]
        show: bool,
        /// Skip the confirmation prompt. For CI.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Only restore this agent. Defaults to every agent Lessr patched.
        #[arg(long)]
        agent: Option<String>,
    },

    /// What is in this binary, and what Lessr Pro adds.
    Pro,
}

/// Changes `lessr config` can make.
#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Set one value without opening the file.
    Set {
        /// Dotted key, e.g. `stages.gate.level`.
        key: String,
        /// The new value.
        value: String,
    },
}
