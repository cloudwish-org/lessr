//! The agents Lessr knows about, and the names a user may call them by.

/// An agent `lessr init` can configure.
///
/// One variant per row of `docs/ADAPTERS.md`. A variant is *not* a promise
/// that Lessr will patch that agent's config: most of them are still
/// unverified, and an unverified adapter prints instructions instead of
/// writing (see `crate::agents`). It is a promise that the CLI can name the
/// agent, detect it and say what to do about it.
///
/// Adding an agent means a variant here, a module under `src/agents/` and its
/// line in the registry. The matches in this crate are exhaustive, so the
/// compiler names every place that still owes an answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentId {
    /// Claude Code: a `PreToolUse` hook in `~/.claude/settings.json`.
    ClaudeCode,
    /// Cursor.
    Cursor,
    /// Windsurf.
    Windsurf,
    /// Cline.
    Cline,
    /// Roo Code.
    Roo,
    /// Kilo Code.
    Kilo,
    /// Codex, OpenAI's CLI.
    Codex,
    /// Gemini CLI.
    GeminiCli,
    /// OpenCode, which loads a TypeScript plugin rather than a JSON hook.
    OpenCode,
    /// The pi harness.
    Pi,
    /// omp, which runs on the pi harness.
    Omp,
    /// Rakazo bots, which inherit pi's configuration.
    Rakazo,
    /// Any SDK or custom agent, pointed at the proxy with a `base_url`. There
    /// is no file to patch, which is why it is an agent and not an absence of
    /// one: the user still has to be told exactly what to set.
    Generic,
}

/// Extra spellings [`AgentId::parse`] accepts, beyond every agent's own
/// [`AgentId::as_str`]. These exist because people type the product's name,
/// not the flag's: `--agent claude-code` must not be an error message.
const ALIASES: &[(&str, AgentId)] = &[
    ("claude-code", AgentId::ClaudeCode),
    ("claudecode", AgentId::ClaudeCode),
    ("roo-code", AgentId::Roo),
    ("roocode", AgentId::Roo),
    ("kilo-code", AgentId::Kilo),
    ("kilocode", AgentId::Kilo),
    ("openai-codex", AgentId::Codex),
    ("gemini-cli", AgentId::GeminiCli),
    ("open-code", AgentId::OpenCode),
    ("sdk", AgentId::Generic),
    ("custom", AgentId::Generic),
];

impl AgentId {
    /// The stable machine name: the `<agent>` in `lessr hook <agent>`, the
    /// directory under `<config>/backups/` and the key in the backup index.
    /// Changing one of these orphans existing backups, so they do not change.
    pub const fn as_str(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "claude",
            AgentId::Cursor => "cursor",
            AgentId::Windsurf => "windsurf",
            AgentId::Cline => "cline",
            AgentId::Roo => "roo",
            AgentId::Kilo => "kilo",
            AgentId::Codex => "codex",
            AgentId::GeminiCli => "gemini",
            AgentId::OpenCode => "opencode",
            AgentId::Pi => "pi",
            AgentId::Omp => "omp",
            AgentId::Rakazo => "rakazo",
            AgentId::Generic => "generic",
        }
    }

    /// The name shown to a human, matching the agent column of
    /// `docs/ADAPTERS.md`.
    pub const fn display_name(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "Claude Code",
            AgentId::Cursor => "Cursor",
            AgentId::Windsurf => "Windsurf",
            AgentId::Cline => "Cline",
            AgentId::Roo => "Roo Code",
            AgentId::Kilo => "Kilo Code",
            AgentId::Codex => "Codex (OpenAI)",
            AgentId::GeminiCli => "Gemini CLI",
            AgentId::OpenCode => "OpenCode",
            AgentId::Pi => "pi",
            AgentId::Omp => "omp",
            AgentId::Rakazo => "Rakazo",
            AgentId::Generic => "Any SDK or custom agent",
        }
    }

    /// Read an agent from what the user typed. Case-insensitive, because a
    /// mistyped `--agent Claude` should configure an agent rather than teach
    /// the user about ASCII.
    pub fn parse(s: &str) -> Option<AgentId> {
        if let Some(&id) = AgentId::all()
            .iter()
            .find(|id| s.eq_ignore_ascii_case(id.as_str()))
        {
            return Some(id);
        }
        ALIASES
            .iter()
            .find(|(alias, _)| s.eq_ignore_ascii_case(alias))
            .map(|&(_, id)| id)
    }

    /// Every agent, in the order `lessr init` should offer them: the ones with
    /// a config of their own first, the manual `base_url` fallback last.
    pub const fn all() -> &'static [AgentId] {
        &[
            AgentId::ClaudeCode,
            AgentId::Cursor,
            AgentId::Windsurf,
            AgentId::Cline,
            AgentId::Roo,
            AgentId::Kilo,
            AgentId::Codex,
            AgentId::GeminiCli,
            AgentId::OpenCode,
            AgentId::Pi,
            AgentId::Omp,
            AgentId::Rakazo,
            AgentId::Generic,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_names_the_cli_accepts() {
        assert_eq!(AgentId::parse("claude"), Some(AgentId::ClaudeCode));
        assert_eq!(AgentId::parse("claude-code"), Some(AgentId::ClaudeCode));
        assert_eq!(AgentId::parse("Claude-Code"), Some(AgentId::ClaudeCode));
        assert_eq!(AgentId::parse("gemini"), Some(AgentId::GeminiCli));
        assert_eq!(AgentId::parse("gemini-cli"), Some(AgentId::GeminiCli));
        assert_eq!(AgentId::parse("opencode"), Some(AgentId::OpenCode));
        assert_eq!(AgentId::parse("generic"), Some(AgentId::Generic));
        assert_eq!(AgentId::parse("emacs"), None);
        assert_eq!(AgentId::parse(""), None);
    }

    #[test]
    fn every_agent_round_trips_through_its_machine_name() {
        for &agent in AgentId::all() {
            assert_eq!(AgentId::parse(agent.as_str()), Some(agent));
            assert!(!agent.display_name().is_empty());
        }
    }

    #[test]
    fn machine_names_are_unique() {
        // They key backup directories and `lessr hook <agent>`; a collision
        // would restore one agent's config over another's.
        let mut seen: Vec<&str> = AgentId::all().iter().map(|id| id.as_str()).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before);
    }

    #[test]
    fn aliases_do_not_shadow_a_machine_name() {
        for (alias, _) in ALIASES {
            assert!(
                !AgentId::all().iter().any(|id| id.as_str() == *alias),
                "{alias} is both an alias and a machine name"
            );
        }
    }
}
