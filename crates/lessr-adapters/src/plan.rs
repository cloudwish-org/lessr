//! What `lessr init` and `lessr uninstall` would do, before they do it.
//!
//! A [`Plan`] is inert. It is built by reading files, printed by
//! [`Plan::render`], and only turned into writes by [`apply`] once the user has
//! agreed — `docs/ADAPTERS.md`: "never write to an agent config without
//! confirmation, except `--yes` for CI".

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::agent::AgentId;
use crate::agents::{self, Adapter, Confidence};
use crate::backup;
use crate::diff;
use crate::error::{Error, Result};
use crate::file;
use crate::paths::Paths;

/// One step of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Patch a JSON config in place. `after` is the whole file as it will be
    /// written; `before` is `None` when the file does not exist yet. Only the
    /// diff between them is printed.
    WriteJson {
        /// The config being patched.
        path: PathBuf,
        /// The file as it is now, if it exists.
        before: Option<String>,
        /// The file as it will be.
        after: String,
        /// One line for the printout: what this write is for.
        summary: String,
    },
    /// Write a whole file Lessr owns, such as an agent plugin. Separate from
    /// [`Change::WriteJson`] because nothing is being merged: the contents are
    /// ours, and an existing file at that path is being replaced rather than
    /// patched.
    WriteFile {
        /// Where the file goes.
        path: PathBuf,
        /// The file as it is now, if something is already there.
        before: Option<String>,
        /// The file as it will be.
        after: String,
        /// One line for the printout: what this file is.
        summary: String,
    },
    /// Copy a file into `<config>/backups/` before anything touches it.
    Backup {
        /// The original.
        from: PathBuf,
        /// The copy.
        to: PathBuf,
    },
    /// Put a backup back, byte for byte.
    Restore {
        /// The copy.
        from: PathBuf,
        /// The original's path.
        to: PathBuf,
    },
    /// Remove a file Lessr wrote. Only ever a file this crate generated: the
    /// plan that produces one has already checked that the contents are ours,
    /// because nothing else here is allowed to delete a user's file.
    Delete {
        /// The file to remove.
        path: PathBuf,
    },
    /// Something the user must do themselves, e.g. the generic `base_url`, or
    /// an agent whose config format Lessr will not guess at.
    Manual {
        /// A one-line heading.
        title: String,
        /// The lines to follow, printed verbatim.
        snippet: String,
    },
    /// Nothing to do, and why.
    Nothing {
        /// What the user should read instead of a diff.
        reason: String,
    },
}

/// Everything `lessr init` or `lessr uninstall` would do to one agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The agent this plan configures.
    pub agent: AgentId,
    /// In the order [`apply`] performs them, backups first.
    pub changes: Vec<Change>,
}

impl Plan {
    /// Human-readable, exactly what `lessr init --show` prints.
    ///
    /// JSON changes are shown as a unified diff of the file, not the file: the
    /// user is being asked to approve one hook entry, and that entry has to be
    /// visible without scrolling through their own settings.
    pub fn render(&self) -> String {
        let mut out = format!("{}\n", self.agent.display_name());
        for change in &self.changes {
            match change {
                Change::WriteJson {
                    path,
                    before,
                    after,
                    summary,
                }
                | Change::WriteFile {
                    path,
                    before,
                    after,
                    summary,
                } => {
                    let label = path.display();
                    let verb = if before.is_some() { "patch" } else { "create" };
                    let _ = writeln!(out, "  {verb}   {label}: {summary}");
                    let body =
                        diff::unified(before.as_deref().unwrap_or(""), after, &label.to_string());
                    indent_into(&mut out, &body, "    ");
                }
                Change::Backup { from, to } => {
                    let _ = writeln!(out, "  backup  {}", from.display());
                    let _ = writeln!(out, "       -> {}", to.display());
                }
                Change::Restore { from, to } => {
                    let _ = writeln!(out, "  restore {}", to.display());
                    let _ = writeln!(out, "     from {}", from.display());
                }
                Change::Delete { path } => {
                    let _ = writeln!(out, "  delete  {}", path.display());
                }
                Change::Manual { title, snippet } => {
                    let _ = writeln!(out, "  manual  {title}");
                    indent_into(&mut out, snippet, "    ");
                }
                Change::Nothing { reason } => {
                    let _ = writeln!(out, "  nothing {reason}");
                }
            }
        }
        out
    }

    /// Whether applying this plan would touch nothing and ask nothing of the
    /// user. A [`Change::Manual`] is not a no-op: the user still has work to
    /// do, and `lessr init` has to print it.
    pub fn is_noop(&self) -> bool {
        self.changes
            .iter()
            .all(|change| matches!(change, Change::Nothing { .. }))
    }

    /// A plan that does nothing, and says why.
    pub(crate) fn nothing(agent: AgentId, reason: impl Into<String>) -> Plan {
        Plan {
            agent,
            changes: vec![Change::Nothing {
                reason: reason.into(),
            }],
        }
    }

    /// A plan the user carries out by hand.
    pub(crate) fn manual(
        agent: AgentId,
        title: impl Into<String>,
        snippet: impl Into<String>,
    ) -> Plan {
        Plan {
            agent,
            changes: vec![Change::Manual {
                title: title.into(),
                snippet: snippet.into(),
            }],
        }
    }
}

/// What [`apply`] did, for the line `lessr init` prints afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Files written or restored.
    pub changed: Vec<PathBuf>,
    /// Backups taken, in the order they were taken.
    pub backups: Vec<PathBuf>,
}

/// Plan the install of the Lessr hook for one agent.
///
/// `hook_command` is what the agent should run, e.g. `lessr hook claude`. The
/// caller owns it because the binary's own name and path are the caller's
/// business: a Homebrew install and a `cargo run` are not the same string.
pub fn plan_init(paths: &Paths, agent: AgentId, hook_command: &str) -> Result<Plan> {
    let adapter = agents::adapter(agent);
    let plan = adapter.plan_init(paths, hook_command)?;
    writes_allowed(adapter, &plan)?;
    Ok(plan)
}

/// Plan the removal of whatever `lessr init` did for one agent.
pub fn plan_uninstall(paths: &Paths, agent: AgentId) -> Result<Plan> {
    let adapter = agents::adapter(agent);
    let plan = adapter.plan_uninstall(paths)?;
    writes_allowed(adapter, &plan)?;
    Ok(plan)
}

/// Carry out a plan.
///
/// Backups run first, all of them, before any write — whatever order the plan
/// lists them in. A plan that half-applies is the one case where Lessr could
/// leave an agent worse than it found it, so the copies exist before anything
/// can fail.
pub fn apply(plan: &Plan) -> Result<Applied> {
    let mut applied = Applied::default();

    for change in &plan.changes {
        if let Change::Backup { from, to } = change {
            backup::take(plan.agent, from, to)?;
            applied.backups.push(to.clone());
        }
    }

    for change in &plan.changes {
        match change {
            Change::WriteJson { path, after, .. } | Change::WriteFile { path, after, .. } => {
                file::write(path, after)?;
                applied.changed.push(path.clone());
            }
            Change::Restore { from, to } => {
                file::copy(from, to)?;
                applied.changed.push(to.clone());
            }
            Change::Delete { path } => {
                file::remove(path)?;
                applied.changed.push(path.clone());
            }
            Change::Backup { .. } | Change::Manual { .. } | Change::Nothing { .. } => {}
        }
    }

    Ok(applied)
}

/// Refuse a plan that would write on behalf of an agent whose format is not
/// verified.
///
/// The rule is stated once in `crate::agents` and enforced here rather than
/// left to review, because the failure it guards against — a wrong entry in a
/// config Lessr did not understand — is silent, permanent and someone else's
/// agent. Nothing an [`Confidence::Unverified`] adapter returns today trips it;
/// that is the point.
fn writes_allowed(adapter: &dyn Adapter, plan: &Plan) -> Result<()> {
    if adapter.confidence() == Confidence::Verified {
        return Ok(());
    }
    let writes = plan.changes.iter().any(|change| {
        matches!(
            change,
            Change::WriteJson { .. }
                | Change::WriteFile { .. }
                | Change::Backup { .. }
                | Change::Restore { .. }
                | Change::Delete { .. }
        )
    });
    if writes {
        return Err(Error::UnverifiedWrite(adapter.id().display_name()));
    }
    Ok(())
}

/// Copy `body` into `out`, indenting every line. Blank lines stay blank rather
/// than becoming trailing whitespace.
fn indent_into(out: &mut String, body: &str, indent: &str) {
    for line in body.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "{indent}{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_of_nothings_is_a_noop() {
        let plan = Plan::nothing(AgentId::ClaudeCode, "already done");
        assert!(plan.is_noop());
        assert!(plan.render().contains("nothing already done"));
    }

    #[test]
    fn a_manual_plan_is_not_a_noop_because_the_user_still_has_work() {
        let plan = Plan::manual(AgentId::Generic, "set these", "FOO=1");
        assert!(!plan.is_noop());
        let rendered = plan.render();
        assert!(rendered.contains("manual  set these"), "{rendered}");
        assert!(rendered.contains("    FOO=1"), "{rendered}");
    }

    #[test]
    fn applying_a_manual_plan_writes_nothing() {
        let plan = Plan::manual(AgentId::Generic, "set these", "FOO=1");
        let applied = apply(&plan).unwrap();
        assert_eq!(applied, Applied::default());
    }

    #[test]
    fn a_write_is_rendered_as_a_diff_not_as_the_file() {
        let plan = Plan {
            agent: AgentId::ClaudeCode,
            changes: vec![Change::WriteJson {
                path: PathBuf::from("/home/dev/.claude/settings.json"),
                before: Some("{\n  \"model\": \"opus\"\n}\n".to_string()),
                after: "{\n  \"model\": \"opus\",\n  \"hooks\": {}\n}\n".to_string(),
                summary: "add the hook".to_string(),
            }],
        };
        let rendered = plan.render();
        assert!(
            rendered.contains("patch   /home/dev/.claude/settings.json: add the hook"),
            "{rendered}"
        );
        assert!(rendered.contains("    +  \"hooks\": {}"), "{rendered}");
        assert!(rendered.contains("    @@ "), "{rendered}");
    }

    #[test]
    fn an_unverified_adapter_is_not_allowed_to_write() {
        let cursor = agents::adapter(AgentId::Cursor);
        let manual = Plan::manual(AgentId::Cursor, "by hand", "FOO=1");
        assert!(writes_allowed(cursor, &manual).is_ok());

        let writing = Plan {
            agent: AgentId::Cursor,
            changes: vec![Change::WriteJson {
                path: PathBuf::from("/home/dev/.cursor/hooks.json"),
                before: None,
                after: "{}\n".to_string(),
                summary: "guesswork".to_string(),
            }],
        };
        let err = writes_allowed(cursor, &writing).unwrap_err();
        assert!(err.to_string().contains("not verified"), "{err}");
    }

    #[test]
    fn a_backup_runs_before_the_write_it_protects() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("home/.claude/settings.json");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, "{\"old\": true}\n").unwrap();
        let copy = tmp.path().join("config/backups/claude/settings.json.1.bak");

        // Deliberately the wrong way round: apply must still copy first.
        let plan = Plan {
            agent: AgentId::ClaudeCode,
            changes: vec![
                Change::WriteJson {
                    path: original.clone(),
                    before: None,
                    after: "{\"new\": true}\n".to_string(),
                    summary: "replace".to_string(),
                },
                Change::Backup {
                    from: original.clone(),
                    to: copy.clone(),
                },
            ],
        };

        let applied = apply(&plan).unwrap();
        assert_eq!(applied.backups, vec![copy.clone()]);
        assert_eq!(applied.changed, vec![original.clone()]);
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "{\"old\": true}\n");
        assert_eq!(
            std::fs::read_to_string(&original).unwrap(),
            "{\"new\": true}\n"
        );
    }
}
