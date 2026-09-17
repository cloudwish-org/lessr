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

/// Agents `lessr init` patches only when asked for by name.
///
/// Codex's config format is verified and its writer works, but its
/// `PostToolUse` cannot replace tool output, so a hook installed there today
/// is a process spawn on every tool call that provably cannot save a token.
/// Lessr may only make a session cheaper, never slower (`docs/LOOP_SAFETY.md`),
/// so Codex waits until either it gains output replacement or the gate moves
/// to a path that its hooks can serve.
const OPT_IN_ONLY: &[AgentId] = &[AgentId::Codex];

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
                | Change::Delete { .. }
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
        .filter(|d| d.installed && !OPT_IN_ONLY.contains(&d.agent))
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
    format!("{} hook {}", shell_quote(&exe), agent.as_str())
}

/// Quote a path so it survives the shell every agent hands this command to.
///
/// Not hypothetical: the default install directory on macOS is
/// `~/Library/Application Support/lessr`, and an unquoted space there breaks
/// the hook on every tool call, for every agent, silently.
fn shell_quote(path: &str) -> String {
    // Characters that mean nothing to a shell. A backslash is safe on Windows,
    // where it is a path separator, and an escape everywhere else.
    let plain = |b: u8| {
        b.is_ascii_alphanumeric() || b"/._-+=:@%".contains(&b) || (cfg!(windows) && b == b'\\')
    };
    if !path.is_empty() && path.bytes().all(plain) {
        return path.to_string();
    }
    if cfg!(windows) {
        format!("\"{}\"", path.replace('"', "\\\""))
    } else {
        // Inside single quotes everything is literal except a single quote,
        // which has to be closed, escaped and reopened.
        format!("'{}'", path.replace('\'', r"'\''"))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_is_left_alone() {
        assert_eq!(shell_quote("/usr/local/bin/lessr"), "/usr/local/bin/lessr");
        assert_eq!(
            shell_quote("/opt/homebrew/bin/lessr"),
            "/opt/homebrew/bin/lessr"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_a_space_is_quoted() {
        assert_eq!(
            shell_quote("/Users/dev/Library/Application Support/lessr/lessr"),
            "'/Users/dev/Library/Application Support/lessr/lessr'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_with_a_quote_survives() {
        // Closed, escaped, reopened: '\'' is the only way through single quotes.
        assert_eq!(
            shell_quote("/home/o'brien/bin/lessr"),
            r"'/home/o'\''brien/bin/lessr'"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_metacharacters_cannot_escape_the_quotes() {
        for nasty in [
            "/tmp/a;rm -rf ~/b",
            "/tmp/$(whoami)/lessr",
            "/tmp/a`id`b",
            "/tmp/a&&b",
        ] {
            let quoted = shell_quote(nasty);
            assert!(
                quoted.starts_with('\'') && quoted.ends_with('\''),
                "{quoted}"
            );
            // Nothing but the wrapping quotes; the payload keeps its bytes.
            assert_eq!(&quoted[1..quoted.len() - 1], nasty, "{quoted}");
        }
    }

    #[test]
    fn deleting_a_file_counts_as_touching_the_filesystem() {
        // Uninstalling an agent whose integration is a file we own is a Delete
        // and nothing else. If that does not count as a write, uninstall says
        // "Nothing to change" and leaves the plugin behind.
        let plan = Plan {
            agent: AgentId::OpenCode,
            changes: vec![Change::Delete {
                path: std::path::PathBuf::from("/tmp/lessr.ts"),
            }],
        };
        assert!(writes_files(&plan));
    }

    #[test]
    fn advice_alone_is_not_a_write() {
        let plan = Plan {
            agent: AgentId::Generic,
            changes: vec![Change::Manual {
                title: "Point your SDK at the proxy".into(),
                snippet: "ANTHROPIC_BASE_URL=http://127.0.0.1:7433".into(),
            }],
        };
        assert!(!writes_files(&plan));
    }
}
