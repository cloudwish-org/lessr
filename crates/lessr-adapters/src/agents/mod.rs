//! One module per agent, and the registry that dispatches to them.
//!
//! Every agent in `docs/ADAPTERS.md` has a module here, and every module
//! states its **confidence**:
//!
//! * [`Confidence::Verified`] — we know the real on-disk format, so
//!   `lessr init` patches the file: back up, write, and restore byte-for-byte
//!   on uninstall.
//! * [`Confidence::Unverified`] — we do not, so `lessr init` prints
//!   instructions and writes nothing. A hook entry in the wrong shape does not
//!   fail once; it fails on every tool call the agent makes afterwards, and the
//!   user has no reason to suspect the thing they installed to save tokens.
//!
//! Promoting an agent is a local edit: replace its module's [`Unverified`]
//! with a type of its own that patches the format, the way
//! [`claude_code`] does. Adding an agent is a new module here, its line in
//! [`registry`] and [`adapter`], and a variant on [`AgentId`].

mod claude_code;
mod cline;
mod codex;
mod cursor;
mod gemini_cli;
mod generic;
mod kilo;
mod matcher_groups;
mod omp;
mod opencode;
mod pi;
mod rakazo;
mod roo;
mod windsurf;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent::AgentId;
use crate::detect::Detected;
use crate::error::{Error, Result};
use crate::hook::{Parsed, Slot};
use crate::paths::Paths;
use crate::plan::{Change, Plan};

/// The environment every agent that speaks an Anthropic or OpenAI shaped API
/// can be pointed at, per `docs/ARCHITECTURE.md`. It is the fallback an
/// unverified adapter offers, because it works today and cannot corrupt
/// anything: the agent either reaches the proxy or it does not.
pub(crate) const PROXY_ENV: &str =
    "ANTHROPIC_BASE_URL=http://127.0.0.1:7433\nOPENAI_BASE_URL=http://127.0.0.1:7433/v1";

/// How sure we are of an agent's real on-disk config format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Confidence {
    /// The format is known; `lessr init` writes it.
    Verified,
    /// The format is not confirmed; `lessr init` explains and writes nothing.
    Unverified,
}

/// Everything the CLI needs to know about one agent.
///
/// Implementations are `'static` values in their own module, so the registry
/// costs nothing to build and an adapter cannot carry per-run state.
pub(crate) trait Adapter: Sync {
    /// Which agent this is.
    fn id(&self) -> AgentId;

    /// Whether this adapter may write to the agent's config. Stated per
    /// module rather than defaulted: an author has to decide.
    fn confidence(&self) -> Confidence;

    /// Look for the agent. Reads only.
    fn detect(&self, paths: &Paths) -> Detected;

    /// What installing the hook would do.
    fn plan_init(&self, paths: &Paths, hook_command: &str) -> Result<Plan>;

    /// What removing it would do.
    fn plan_uninstall(&self, paths: &Paths) -> Result<Plan>;

    /// Read this agent's hook JSON.
    ///
    /// `Ok(None)` means the payload carries nothing to filter. The default is
    /// an error, because an agent with no verified hook shape has no hook:
    /// `lessr hook <agent>` for it would be a promise we cannot keep.
    fn parse_hook(&self, _root: &Value) -> Result<Option<Parsed>> {
        Err(Error::NoHook(self.id().display_name()))
    }

    /// Render what this agent expects on stdout once a stage has rewritten the
    /// content.
    ///
    /// The default is the contract `docs/ADAPTERS.md` states — the same JSON
    /// back, with the result substituted in — which is what an agent that reads
    /// stdout as a replacement payload wants. An agent that reads stdout as a
    /// decision document overrides this.
    fn render_hook(&self, root: &Value, slot: Slot, content: &str) -> Result<Vec<u8>> {
        let mut json = root.clone();
        slot.put(&mut json, content);
        serde_json::to_vec(&json).map_err(Error::HookJson)
    }

    /// What to print when there is nothing to filter.
    ///
    /// The payload back, byte for byte, for the agents whose contract is an
    /// echo. An agent that reads stdout as a decision gets nothing at all,
    /// which is how every one of them spells "no opinion".
    fn render_unchanged(&self, raw: &[u8]) -> Vec<u8> {
        raw.to_vec()
    }
}

/// Every adapter, in [`AgentId::all`] order.
pub(crate) fn registry() -> &'static [&'static dyn Adapter] {
    REGISTRY
}

/// See [`registry`].
static REGISTRY: &[&dyn Adapter] = &[
    &claude_code::ADAPTER,
    &cursor::ADAPTER,
    &windsurf::ADAPTER,
    &cline::ADAPTER,
    &roo::ADAPTER,
    &kilo::ADAPTER,
    &codex::ADAPTER,
    &gemini_cli::ADAPTER,
    &opencode::ADAPTER,
    &pi::ADAPTER,
    &omp::ADAPTER,
    &rakazo::ADAPTER,
    &generic::ADAPTER,
];

/// The adapter for one agent.
///
/// A match rather than a search through [`registry`], so that a new [`AgentId`]
/// does not compile until it has an adapter.
pub(crate) fn adapter(id: AgentId) -> &'static dyn Adapter {
    match id {
        AgentId::ClaudeCode => &claude_code::ADAPTER,
        AgentId::Cursor => &cursor::ADAPTER,
        AgentId::Windsurf => &windsurf::ADAPTER,
        AgentId::Cline => &cline::ADAPTER,
        AgentId::Roo => &roo::ADAPTER,
        AgentId::Kilo => &kilo::ADAPTER,
        AgentId::Codex => &codex::ADAPTER,
        AgentId::GeminiCli => &gemini_cli::ADAPTER,
        AgentId::OpenCode => &opencode::ADAPTER,
        AgentId::Pi => &pi::ADAPTER,
        AgentId::Omp => &omp::ADAPTER,
        AgentId::Rakazo => &rakazo::ADAPTER,
        AgentId::Generic => &generic::ADAPTER,
    }
}

/// An agent we can find but will not edit.
///
/// Detection still runs — knowing an agent is installed is worth printing —
/// but `lessr init` produces a [`crate::Change::Manual`]. One type serves every
/// such agent so that a module is a name, some paths and a sentence; promoting
/// one to verified means replacing that value with a real adapter, and nothing
/// else in the crate moves.
pub(crate) struct Unverified {
    /// Which agent.
    pub(crate) id: AgentId,
    /// Home-relative paths that say the agent is on this machine, most
    /// specific first. Guesses: they are only ever read, never written, so a
    /// wrong one costs a line of output.
    pub(crate) probes: &'static [&'static str],
    /// Why Lessr does not write this agent's config, in whole sentences. It is
    /// printed to the user, so it says what is true of *this* agent rather
    /// than something generic about verification.
    pub(crate) note: &'static str,
}

impl Unverified {
    /// The first probe that exists on this machine.
    fn found(&self, paths: &Paths) -> Option<PathBuf> {
        self.probes
            .iter()
            .map(|relative| home_join(paths, relative))
            .find(|path| path.exists())
    }

    /// What the user is told instead of being written to.
    fn instructions(&self, found: Option<&Path>) -> String {
        let mut text = format!("Nothing was written.\n\n{}\n\n", self.note);
        if let Some(path) = found {
            let _ = writeln!(text, "Found on this machine: {}\n", path.display());
        }
        text.push_str("What works today, in your shell or your agent's environment:\n\n");
        for line in PROXY_ENV.lines() {
            let _ = writeln!(text, "    {line}");
        }
        text
    }
}

impl Adapter for Unverified {
    fn id(&self) -> AgentId {
        self.id
    }

    fn confidence(&self) -> Confidence {
        Confidence::Unverified
    }

    fn detect(&self, paths: &Paths) -> Detected {
        let found = self.found(paths);
        Detected {
            installed: found.is_some(),
            config_path: found,
            // Nothing was ever written, so there is nothing of ours to find,
            // and we do not read a format we have not verified well enough to
            // make a claim about someone else's hooks.
            ..Detected::absent(self.id)
        }
    }

    fn plan_init(&self, paths: &Paths, _hook_command: &str) -> Result<Plan> {
        let found = self.found(paths);
        Ok(Plan::manual(
            self.id,
            format!("{}: set it up by hand", self.id.display_name()),
            self.instructions(found.as_deref()),
        ))
    }

    fn plan_uninstall(&self, _paths: &Paths) -> Result<Plan> {
        Ok(Plan::nothing(
            self.id,
            format!(
                "Lessr never wrote to {}'s config, so there is nothing to undo",
                self.id.display_name()
            ),
        ))
    }
}

/// The line every file Lessr generates carries.
///
/// It is how `lessr uninstall` tells a plugin it wrote from one the user wrote
/// at the same path: ours is reproducible, so it is deleted, and theirs is
/// left exactly where it is.
pub(crate) const MARKER: &str = "Generated by `lessr init` (lessr.dev)";

/// Plan writing a file Lessr owns outright, such as an agent plugin.
///
/// Not a JSON patch: the contents are ours end to end. A file already at that
/// path is backed up first unless it is one of ours, which is reproducible by
/// definition.
pub(crate) fn plan_owned_file(
    paths: &Paths,
    id: AgentId,
    path: PathBuf,
    contents: String,
    summary: &str,
) -> Result<Plan> {
    let before = crate::file::read(&path)?;
    if before.as_deref() == Some(contents.as_str()) {
        return Ok(Plan::nothing(
            id,
            format!("{} is already installed and current", path.display()),
        ));
    }

    let mut changes = Vec::with_capacity(2);
    if before.as_deref().is_some_and(|text| !text.contains(MARKER)) {
        changes.push(Change::Backup {
            from: path.clone(),
            to: crate::backup::destination(paths, id, &path),
        });
    }
    changes.push(Change::WriteFile {
        after: contents,
        summary: summary.to_string(),
        path,
        before,
    });
    Ok(Plan { agent: id, changes })
}

/// Plan removing a file Lessr owns.
///
/// A backup taken at install time wins: it is the user's own file from before
/// Lessr existed here. Otherwise the file goes only if it carries [`MARKER`] —
/// deleting something we did not write, at a path we happen to use, is not
/// ours to do.
pub(crate) fn plan_remove_owned_file(paths: &Paths, id: AgentId, path: PathBuf) -> Result<Plan> {
    let records = crate::backup::read_index(&crate::backup::index_path(paths))?;
    if let Some(record) = crate::backup::newest_for(&records, id) {
        return Ok(Plan {
            agent: id,
            changes: vec![Change::Restore {
                from: record.stored_as.clone(),
                to: record.original_path.clone(),
            }],
        });
    }

    let Some(text) = crate::file::read(&path)? else {
        return Ok(Plan::nothing(
            id,
            format!("there is no {} to remove", path.display()),
        ));
    };
    if !text.contains(MARKER) {
        return Ok(Plan::nothing(
            id,
            format!(
                "{} was not written by Lessr; leaving it alone",
                path.display()
            ),
        ));
    }
    Ok(Plan {
        agent: id,
        changes: vec![Change::Delete { path }],
    })
}

/// Join a `/`-separated, home-relative path onto the home directory.
///
/// Written with `/` in the tables above whatever the platform, and split here,
/// so a probe list reads the same everywhere.
pub(crate) fn home_join(paths: &Paths, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(paths.home.clone(), |path, part| path.join(part))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Change;

    fn paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("home"), tmp.path().join("config"));
        (tmp, paths)
    }

    #[test]
    fn the_registry_holds_every_agent_once_and_in_order() {
        let ids: Vec<AgentId> = registry().iter().map(|a| a.id()).collect();
        assert_eq!(ids, AgentId::all().to_vec());
    }

    #[test]
    fn the_lookup_agrees_with_the_registry() {
        for &id in AgentId::all() {
            assert_eq!(adapter(id).id(), id);
        }
    }

    /// The adapters that may write to an agent's own config. Every other
    /// agent prints instructions instead; see the module docs.
    const VERIFIED: &[AgentId] = &[
        AgentId::ClaudeCode,
        AgentId::Codex,
        AgentId::GeminiCli,
        AgentId::OpenCode,
        AgentId::Pi,
    ];

    #[test]
    fn only_the_verified_adapters_say_so() {
        for &id in AgentId::all() {
            let expected = if VERIFIED.contains(&id) {
                Confidence::Verified
            } else {
                Confidence::Unverified
            };
            assert_eq!(adapter(id).confidence(), expected, "{id:?}");
        }
    }

    #[test]
    fn an_unverified_agent_is_told_about_by_hand_and_never_written_to() {
        let (_tmp, paths) = paths();
        for &id in AgentId::all() {
            if VERIFIED.contains(&id) {
                continue;
            }
            let plan = adapter(id).plan_init(&paths, "lessr hook x").unwrap();
            assert!(!plan.is_noop(), "{id:?}");
            assert!(
                plan.changes
                    .iter()
                    .all(|change| matches!(change, Change::Manual { .. })),
                "{id:?} would write something: {:?}",
                plan.changes
            );
            let rendered = plan.render();
            assert!(rendered.contains("127.0.0.1:7433"), "{id:?}: {rendered}");

            let undo = adapter(id).plan_uninstall(&paths).unwrap();
            assert!(undo.is_noop(), "{id:?}");
        }
    }

    #[test]
    fn an_unverified_agent_reports_the_config_it_finds() {
        let (_tmp, paths) = paths();
        let found = adapter(AgentId::Cursor).detect(&paths);
        assert!(!found.installed);
        assert_eq!(found.config_path, None);

        std::fs::create_dir_all(paths.home.join(".cursor")).unwrap();
        let found = adapter(AgentId::Cursor).detect(&paths);
        assert!(found.installed);
        assert_eq!(found.config_path, Some(paths.home.join(".cursor")));
        assert!(!found.already_patched);
    }

    #[test]
    fn a_verified_adapter_writes_and_then_takes_it_back() {
        let (_tmp, paths) = paths();
        for &id in VERIFIED {
            let plan = crate::plan_init(&paths, id, "lessr hook x").unwrap();
            assert!(
                !plan.is_noop(),
                "{id:?} had nothing to do on a clean machine"
            );
            crate::apply(&plan).unwrap();

            let found = adapter(id).detect(&paths);
            assert!(found.already_patched, "{id:?} did not detect its own work");
            assert!(
                crate::plan_init(&paths, id, "lessr hook x")
                    .unwrap()
                    .is_noop(),
                "{id:?} is not idempotent"
            );

            let undo = crate::plan_uninstall(&paths, id).unwrap();
            crate::apply(&undo).unwrap();
            assert!(
                !adapter(id).detect(&paths).already_patched,
                "{id:?} left itself behind"
            );
        }
    }

    #[test]
    fn an_agent_without_a_verified_hook_has_no_hook() {
        let err = adapter(AgentId::Cursor)
            .parse_hook(&serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("base_url"), "{err}");
    }
}
