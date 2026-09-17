//! What is in the free engine, and what Pro adds.
//!
//! Three rules govern everything in this module, and they are the reason it is
//! a module and not a sprinkling of `println!`s:
//!
//! 1. **Nothing here is ever printed on the hook path.** Hook output goes into
//!    the agent's context, where every byte is billed on every later turn.
//!    Advertising there would cost the user money to read an advertisement,
//!    which is the exact leak this product exists to stop.
//! 2. **It is a list and a link, never a nag.** `lessr pro` is something the
//!    user typed. The one line the receipt carries is suppressible with
//!    `--no-pro`. There is no banner, no countdown and no repetition.
//! 3. **No Pro code is reachable from here.** This is a static table of names
//!    and a URL. The free workspace does not depend on a Pro crate, and tiering
//!    is never a feature flag.

/// Where the upgrade lives. The only URL the free engine ever prints.
pub const PRO_URL: &str = "https://lessr.dev/pro";

/// A mechanism, as the user should understand it.
struct Mechanism {
    name: &'static str,
    summary: &'static str,
    /// When it lands. `None` once it has shipped.
    due: Option<&'static str>,
}

/// The free engine. Complete and unlimited; see `docs/MECHANISMS.md`.
const FREE: &[Mechanism] = &[
    Mechanism {
        name: "detect",
        summary: "finds what re-writes your prompt cache, and what it costs",
        due: Some("phase 1"),
    },
    Mechanism {
        name: "receipt",
        summary: "exact per-session accounting behind `lessr gain`",
        due: Some("phase 1"),
    },
    Mechanism {
        name: "gate",
        summary: "strips tool output before it enters history; errors always pass",
        due: Some("phase 2"),
    },
    Mechanism {
        name: "trap",
        summary: "lockfiles, minified bundles and base64 never enter history raw",
        due: Some("phase 2"),
    },
    Mechanism {
        name: "dedup",
        summary: "a file read twice is a hash line; changed is a diff",
        due: Some("phase 3"),
    },
];

/// Lessr Pro, from `docs/PRO.md`. Names and one-liners only: no code, no
/// dependency, nothing that has to stay in sync with a private repository
/// beyond this table.
const PRO: &[Mechanism] = &[
    Mechanism {
        name: "cachefix",
        summary: "auto-fixes the breaks the free detector reports",
        due: None,
    },
    Mechanism {
        name: "keepalive, compact",
        summary: "keeps a long session's cache warm; compacts only when it is already cold",
        due: None,
    },
    Mechanism {
        name: "mcp",
        summary: "lazy-loads MCP tool schemas",
        due: None,
    },
    Mechanism {
        name: "zoom",
        summary: "map-then-zoom reads: outline first, exact range on demand",
        due: None,
    },
    Mechanism {
        name: "delta",
        summary: "repeated commands show only what changed",
        due: None,
    },
    Mechanism {
        name: "editverify",
        summary: "post-edit re-reads return the changed region plus a syntax check",
        due: None,
    },
    Mechanism {
        name: "batch",
        summary: "routes non-interactive runs through provider batch endpoints",
        due: None,
    },
    Mechanism {
        name: "prefix",
        summary: "one warm prefix across subagents, bots and sessions",
        due: None,
    },
    Mechanism {
        name: "rent",
        summary: "lifetime cost of every read",
        due: None,
    },
    Mechanism {
        name: "router",
        summary: "cheapest model per task, with a quality guard",
        due: None,
    },
    Mechanism {
        name: "full pack",
        summary: "200+ rules, updated weekly",
        due: None,
    },
];

/// The body of `lessr pro`.
pub fn page() -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("Lessr — what you have, and what Pro adds\n\n");

    out.push_str("  In this binary. Free, offline, unlimited.\n");
    for m in FREE {
        out.push_str(&row(m));
    }

    out.push_str("\n  Lessr Pro adds\n");
    for m in PRO {
        out.push_str(&row(m));
    }

    out.push_str(concat!(
        "\n  Pro is the same binary with more crates: `lessr pro` upgrades in place,\n",
        "  nothing to reinstall and no change to your agent configs.\n",
        "\n  Before you decide, make us prove it. `lessr gain` counts what Pro would\n",
        "  have saved you today from your own sessions, measured the same way as\n",
        "  everything else on the receipt. `lessr gain --no-pro` hides that section\n",
        "  for good.\n",
    ));
    out.push_str(&format!("\n  {PRO_URL}\n"));
    out
}

/// One `name  summary  (when)` line, aligned.
fn row(m: &Mechanism) -> String {
    let due = m.due.map_or_else(String::new, |d| format!("  ({d})"));
    format!("    {:<18} {}{}\n", m.name, m.summary, due)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_names_both_tiers_and_links_once() {
        let page = page();
        assert!(page.contains("cachefix"), "the Pro list is the point");
        assert!(page.contains("gate"), "so is what they already have");
        assert_eq!(page.matches(PRO_URL).count(), 1, "one link, not a campaign");
    }

    #[test]
    fn the_receipt_section_is_advertised_as_suppressible() {
        // If a user cannot turn it off, it is a nag rather than a line.
        assert!(page().contains("--no-pro"));
    }
}
