//! What this build's mechanisms are called, and what they are tuned with.
//!
//! `docs/CONFIG.md` is the source of every line here: it names each free
//! mechanism's tunables and the default behind each level.
//!
//! `lessr-core` deliberately holds no such table. A stage names the key and
//! the fallback where it reads them — `settings.u64("max_repeated_lines", 3)`
//! — so there is nothing to drift out of step with the code. This table exists
//! for the CLI alone, and only for the two things the resolver genuinely
//! cannot know:
//!
//! - printing a value that is in force but appears in no layer, which is most
//!   of what a fresh `lessr config` has to say;
//! - telling a typo from a key some other build reads. A Pro stage's key is
//!   unknown here and is *reported*, never rejected: the Pro binary is this
//!   same CLI with more stages registered, and refusing what it understands
//!   would make the free binary unable to read a Pro config file.

use lessr_core::ValueKind;

/// One tunable, as `docs/CONFIG.md` documents it.
pub struct Tunable {
    /// The key as it is spelled in `config.toml`.
    pub key: &'static str,
    /// The kind the stage asks for. A value that cannot be one falls back to
    /// the stage's own default and is reported.
    pub kind: ValueKind,
    /// The stage's default, rendered. Printed when no layer sets the key, so
    /// `lessr config` shows the value in force rather than a blank.
    pub default: &'static str,
}

/// One mechanism.
pub struct Stage {
    /// The name `lessr on <stage>` takes.
    pub name: &'static str,
    /// One line, for the header of `--explain`.
    pub summary: &'static str,
    /// Its tunables, in the order `docs/CONFIG.md` lists them.
    pub tunables: &'static [Tunable],
}

/// Every mechanism in the free engine, in pipeline order.
///
/// Order matters only to the eye: `lessr config` prints them in it, and a
/// table that reshuffled itself between runs would be a table nobody reads
/// twice.
pub const STAGES: &[Stage] = &[
    Stage {
        name: "gate",
        summary: "strips tool output before it enters history; errors always pass",
        tunables: &[
            Tunable {
                key: "max_repeated_lines",
                kind: ValueKind::Int,
                default: "3",
            },
            Tunable {
                key: "summary_after_lines",
                kind: ValueKind::Int,
                default: "400",
            },
            Tunable {
                key: "strip_timestamps",
                kind: ValueKind::Bool,
                default: "true",
            },
        ],
    },
    Stage {
        name: "trap",
        summary: "lockfiles, minified bundles and base64 never enter history raw",
        tunables: &[
            Tunable {
                key: "json_bytes",
                kind: ValueKind::Int,
                default: "65536",
            },
            Tunable {
                key: "max_bytes",
                kind: ValueKind::Int,
                default: "262144",
            },
        ],
    },
    Stage {
        name: "dedup",
        summary: "a file read twice is a hash line; changed is a diff",
        tunables: &[
            Tunable {
                key: "context_lines",
                kind: ValueKind::Int,
                default: "3",
            },
            Tunable {
                key: "min_bytes",
                kind: ValueKind::Int,
                default: "2048",
            },
        ],
    },
    Stage {
        name: "detect",
        summary: "finds what re-writes your prompt cache, and what it costs",
        tunables: &[],
    },
];

/// The name of the layer that speaks for every stage at once: `[stages.default]`.
///
/// Not a stage. It is spelled where a stage name goes, so every path that
/// takes one has to know it means the layer.
pub const DEFAULT: &str = "default";

/// One mechanism by name, or `None` for a name this build does not register.
pub fn stage(name: &str) -> Option<&'static Stage> {
    STAGES.iter().find(|stage| stage.name == name)
}

/// One tunable of one stage, or `None` if nothing in this build reads it.
pub fn tunable(stage_name: &str, key: &str) -> Option<&'static Tunable> {
    stage(stage_name)?.tunables.iter().find(|t| t.key == key)
}

/// The keys a stage takes, for a message that has to list them.
pub fn keys(stage_name: &str) -> Vec<&'static str> {
    let mut keys = vec!["mode", "level"];
    if let Some(stage) = stage(stage_name) {
        keys.extend(stage.tunables.iter().map(|t| t.key));
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_tunable_is_reachable_by_name() {
        // The table is only useful if lookups agree with it; a key listed
        // under the wrong stage would print a default that is not in force.
        for entry in STAGES {
            for documented in entry.tunables {
                assert!(
                    tunable(entry.name, documented.key).is_some(),
                    "{}.{}",
                    entry.name,
                    documented.key
                );
            }
        }
        assert!(tunable("gate", "json_bytes").is_none(), "that is trap's");
    }

    #[test]
    fn an_unregistered_stage_is_unknown_rather_than_empty() {
        // `cachefix` is a Pro stage: unknown to this build, and its config is
        // still something this build has to read without complaining at every
        // key.
        assert!(stage("cachefix").is_none());
        assert_eq!(keys("cachefix"), ["mode", "level"]);
    }
}
