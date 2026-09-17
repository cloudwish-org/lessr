//! What is on this machine, and what has already been done to it.

use std::path::PathBuf;

use crate::agent::AgentId;
use crate::agents;
use crate::paths::Paths;

/// One agent, as found on this machine.
///
/// Detection only ever reads. It is what `lessr init` shows before it proposes
/// anything, so a wrong guess here costs a line of output, not a config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    /// Which agent this is about.
    pub agent: AgentId,
    /// The config we found, when the agent keeps one and it exists.
    pub config_path: Option<PathBuf>,
    /// We found this agent on the machine.
    pub installed: bool,
    /// A Lessr hook is already there, so `lessr init` has nothing to add.
    pub already_patched: bool,
    /// An `rtk` hook is already there. Lessr chains behind it rather than
    /// replacing it (`docs/ADAPTERS.md`), and `lessr init --show` says so.
    pub rtk_present: bool,
}

impl Detected {
    /// An agent we looked for and did not find. Adapters start from this and
    /// fill in what they learn.
    pub(crate) fn absent(agent: AgentId) -> Detected {
        Detected {
            agent,
            config_path: None,
            installed: false,
            already_patched: false,
            rtk_present: false,
        }
    }
}

/// Look for every agent Lessr knows about.
///
/// Returns one entry per [`AgentId`], in [`AgentId::all`] order, including the
/// ones that are not installed: `lessr init` lists what it could configure,
/// not only what it can configure right now. Nothing here returns an error —
/// an unreadable or malformed config means "not patched", because a config we
/// cannot parse is one we are not going to touch anyway.
pub fn detect(paths: &Paths) -> Vec<Detected> {
    agents::registry()
        .iter()
        .map(|adapter| adapter.detect(paths))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_is_reported_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("home"), tmp.path().join("config"));

        let found: Vec<AgentId> = detect(&paths).iter().map(|d| d.agent).collect();
        assert_eq!(found, AgentId::all().to_vec());
    }

    #[test]
    fn an_empty_machine_has_nothing_installed_but_the_generic_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(tmp.path().join("home"), tmp.path().join("config"));

        for found in detect(&paths) {
            assert!(!found.already_patched, "{:?}", found.agent);
            assert!(!found.rtk_present, "{:?}", found.agent);
            assert_eq!(
                found.installed,
                found.agent == AgentId::Generic,
                "{:?}",
                found.agent
            );
        }
    }
}
