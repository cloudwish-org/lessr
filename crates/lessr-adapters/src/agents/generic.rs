//! Any SDK or custom agent. Confidence: not applicable — nothing is written.
//!
//! The row in `docs/ADAPTERS.md` is `base_url = http://127.0.0.1:7433`, in
//! "your code". There is no file of the user's that Lessr could patch, so the
//! plan is a [`crate::Change::Manual`] by design rather than by caution, and
//! `lessr uninstall` has nothing to undo.

use crate::agent::AgentId;
use crate::agents::{Adapter, Confidence, PROXY_ENV};
use crate::detect::Detected;
use crate::error::Result;
use crate::paths::Paths;
use crate::plan::Plan;

/// See the module docs.
pub(crate) static ADAPTER: Generic = Generic;

/// The `base_url` fallback.
pub(crate) struct Generic;

impl Adapter for Generic {
    fn id(&self) -> AgentId {
        AgentId::Generic
    }

    fn confidence(&self) -> Confidence {
        // Nothing is written, so there is no format to be wrong about. It is
        // listed as unverified so that the one verified adapter in the crate
        // is the one that actually edits a file.
        Confidence::Unverified
    }

    fn detect(&self, _paths: &Paths) -> Detected {
        Detected {
            // Always available: it is the user's own code, and there is
            // nothing on disk that would say yes or no.
            installed: true,
            ..Detected::absent(AgentId::Generic)
        }
    }

    fn plan_init(&self, _paths: &Paths, _hook_command: &str) -> Result<Plan> {
        let mut snippet =
            String::from("Point the agent at the Lessr proxy and leave everything else alone:\n\n");
        for line in PROXY_ENV.lines() {
            snippet.push_str("    ");
            snippet.push_str(line);
            snippet.push('\n');
        }
        snippet.push_str(
            "\nAnything that speaks either API shape reaches it this way, including an\n\
             OpenAI-compatible upstream such as OpenRouter: the upstream is configured in\n\
             Lessr, not in your agent, so your agent only ever knows about 127.0.0.1.\n\
             The proxy streams the provider's response back byte for byte.",
        );

        Ok(Plan::manual(
            AgentId::Generic,
            "Set two environment variables",
            snippet,
        ))
    }

    fn plan_uninstall(&self, _paths: &Paths) -> Result<Plan> {
        Ok(Plan::nothing(
            AgentId::Generic,
            "Lessr wrote nothing for a custom agent; unset ANTHROPIC_BASE_URL and \
             OPENAI_BASE_URL to stop using it",
        ))
    }
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
    fn the_base_url_is_a_manual_change_and_writes_nothing() {
        let (_tmp, paths) = paths();
        let plan = ADAPTER.plan_init(&paths, "lessr hook generic").unwrap();
        assert!(matches!(plan.changes.as_slice(), [Change::Manual { .. }]));

        let applied = crate::plan::apply(&plan).unwrap();
        assert!(applied.changed.is_empty());
        assert!(applied.backups.is_empty());
        assert!(!paths.home.exists(), "nothing under home was created");
        assert!(
            !paths.config.exists(),
            "nothing under the config dir was created"
        );
    }

    #[test]
    fn the_snippet_names_both_api_shapes_and_the_openrouter_case() {
        let (_tmp, paths) = paths();
        let rendered = ADAPTER
            .plan_init(&paths, "lessr hook generic")
            .unwrap()
            .render();
        assert!(
            rendered.contains("ANTHROPIC_BASE_URL=http://127.0.0.1:7433"),
            "{rendered}"
        );
        assert!(
            rendered.contains("OPENAI_BASE_URL=http://127.0.0.1:7433/v1"),
            "{rendered}"
        );
        assert!(rendered.contains("OpenRouter"), "{rendered}");
    }

    #[test]
    fn uninstalling_a_custom_agent_is_a_no_op_that_says_what_to_unset() {
        let (_tmp, paths) = paths();
        let plan = ADAPTER.plan_uninstall(&paths).unwrap();
        assert!(plan.is_noop());
        assert!(plan.render().contains("ANTHROPIC_BASE_URL"));
    }
}
