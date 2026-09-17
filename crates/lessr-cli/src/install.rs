//! `lessr init` and `lessr uninstall`.
//!
//! The rules come from `docs/ADAPTERS.md`: back up before patching, never write
//! to an agent config without confirmation (except `--yes` for CI), and
//! `--show` prints every change it would make and exits.

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use lessr_adapters::{AgentId, Change, Paths, Plan};

/// `lessr init`.
pub fn init(show: bool, yes: bool, agent: Option<String>) -> Result<ExitCode> {
    let paths = Paths::detect().context("cannot work out where your config lives")?;
    let targets = targets(&paths, agent.as_deref())?;

    let mut plans = Vec::new();
    for target in targets {
        let command = hook_command(target);
        plans.push(
            lessr_adapters::plan_init(&paths, target, &command)
                .with_context(|| format!("planning {}", target.display_name()))?,
        );
    }

    run_plans("init", &plans, show, yes)
}

/// `lessr uninstall`.
pub fn uninstall(show: bool, yes: bool, agent: Option<String>) -> Result<ExitCode> {
    let paths = Paths::detect().context("cannot work out where your config lives")?;
    let targets = targets(&paths, agent.as_deref())?;

    let mut plans = Vec::new();
    for target in targets {
        plans.push(
            lessr_adapters::plan_uninstall(&paths, target)
                .with_context(|| format!("planning {}", target.display_name()))?,
        );
    }

    run_plans("uninstall", &plans, show, yes)
}

/// Print the plans, then apply them unless this is a dry run or the user says
/// no.
fn run_plans(verb: &str, plans: &[Plan], show: bool, yes: bool) -> Result<ExitCode> {
    println!("lessr {verb}\n");
    for plan in plans {
        println!("{}", plan.render());
    }

    // Only the plans that would actually touch a file get as far as the
    // prompt. A plan whose whole content is `Manual` advice has already done
    // its job by being printed; asking permission to write files we are not
    // going to write would train people to say yes without reading.
    let writing: Vec<&Plan> = plans.iter().filter(|p| writes_files(p)).collect();

    if writing.is_empty() {
        println!("Nothing to change.");
        return Ok(ExitCode::SUCCESS);
    }

    if show {
        println!("Dry run: nothing was changed. Drop --show to apply.");
        return Ok(ExitCode::SUCCESS);
    }

    if !yes && !confirm("Apply these changes to your agent configs?")? {
        println!("Nothing changed. Re-run with --yes to skip this prompt.");
        return Ok(ExitCode::SUCCESS);
    }

    for plan in writing {
        let applied = lessr_adapters::apply(plan)
            .with_context(|| format!("applying {}", plan.agent.display_name()))?;
        for path in &applied.backups {
            println!("  backed up  {}", path.display());
        }
        for path in &applied.changed {
            println!("  wrote      {}", path.display());
        }
    }

    if verb == "init" {
        println!("\nDone. Restart your agent, then run `lessr gain` after a couple of minutes.");
    } else {
        println!("\nDone. Your agent configs are back to exactly what they were.");
    }
    Ok(ExitCode::SUCCESS)
}

/// Whether a plan would actually touch the filesystem.
///
/// `Manual` and `Nothing` are things we tell the user; the rest are things we
/// do to their machine, and only those need consent.
fn writes_files(plan: &Plan) -> bool {
    plan.changes.iter().any(|change| {
        matches!(
            change,
            Change::WriteJson { .. }
                | Change::WriteFile { .. }
                | Change::Backup { .. }
                | Change::Restore { .. }
        )
    })
}

/// Which agents this invocation is about.
///
/// With no `--agent`, that is every agent we actually found, plus the generic
/// base_url advice, which is never installed and never skipped: an SDK user has
/// no config for us to detect.
fn targets(paths: &Paths, agent: Option<&str>) -> Result<Vec<AgentId>> {
    if let Some(name) = agent {
        let id = AgentId::parse(name).ok_or_else(|| {
            anyhow!(
                "unknown agent `{name}`. Known: {}",
                AgentId::all()
                    .iter()
                    .map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        return Ok(vec![id]);
    }

    let mut found: Vec<AgentId> = lessr_adapters::detect(paths)
        .into_iter()
        .filter(|d| d.installed)
        .map(|d| d.agent)
        .collect();
    if !found.contains(&AgentId::Generic) {
        found.push(AgentId::Generic);
    }
    Ok(found)
}

/// The command we ask the agent to run.
///
/// An absolute path, not the bare name: `lessr` may not be on the agent's
/// `PATH` — agents launched from a desktop icon rarely inherit a shell's — and
/// a hook the agent cannot find is a hook that fails on every tool call.
fn hook_command(agent: AgentId) -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| "lessr".to_string());
    format!("{exe} hook {}", agent.as_str())
}

/// Ask before touching someone's editor config.
///
/// The prompt goes to stderr so stdout stays clean for anything piping our
/// output. End of input means "not a terminal", which we read as no.
fn confirm(question: &str) -> Result<bool> {
    eprint!("{question} [y/N] ");
    io::stderr().flush().ok();

    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(false);
    }
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
