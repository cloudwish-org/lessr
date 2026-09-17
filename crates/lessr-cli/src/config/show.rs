//! What `lessr config` prints.
//!
//! The promise at the top of `docs/CONFIG.md` is that nothing is hidden: the
//! value in force, and where it came from. Two renderings serve it.
//!
//! The table answers "what is my Lessr doing": every mechanism, its two axes,
//! its settings, and the layer behind each one.
//!
//! `--explain` answers the harder question, the one a table cannot: *why*.
//! It prints every layer that had an opinion about a field, not just the one
//! that won, so the answer to "I set this and nothing happened" is on the
//! screen — your global value is there, and the repo one below it is what is
//! in force. That is what `mode_trace`, `level_trace` and `setting_trace` are
//! for.

use std::fmt::Write as _;

use lessr_core::{Layer, Resolved};

use super::Session;
use super::known;

/// `lessr config`: every mechanism, and which layer decided each value.
pub fn table(session: &Session) -> String {
    let mut out = String::new();
    header(session, &mut out);
    out.push('\n');

    let stages = stages(session);
    let rows: Vec<Row> = stages.iter().map(|name| Row::of(session, name)).collect();

    let name_width = width(rows.iter().map(|row| row.stage.len()).chain([9]));
    let mode_width = width(rows.iter().map(|row| row.mode.len()).chain([4]));
    let mode_by = width(rows.iter().map(|row| row.mode_layer.len()).chain([6]));
    let level_width = width(rows.iter().map(|row| row.level.len()).chain([5]));

    let _ = writeln!(
        out,
        "{:name_width$}  {:mode_width$}  {:mode_by$}  {:level_width$}  set by",
        "mechanism", "mode", "set by", "level"
    );
    for row in &rows {
        let _ = writeln!(
            out,
            "{:name_width$}  {:mode_width$}  {:mode_by$}  {:level_width$}  {}",
            row.stage, row.mode, row.mode_layer, row.level, row.level_layer
        );
        if let Some(reason) = &row.floor {
            let _ = writeln!(
                out,
                "{:name_width$}  ! off by the self-healing table: {reason}",
                ""
            );
            let _ = writeln!(
                out,
                "{:name_width$}    no layer can promote it (loop-safety 5)",
                ""
            );
        }
        let key_width = width(row.settings.iter().map(|(key, _, _)| key.len()));
        let value_width = width(row.settings.iter().map(|(_, value, _)| value.len()));
        for (key, value, layer) in &row.settings {
            let _ = writeln!(
                out,
                "{:name_width$}    {key:key_width$}  {value:value_width$}  {layer}",
                ""
            );
        }
    }

    problems(session, &mut out);
    let _ = writeln!(
        out,
        "\n`lessr config --explain <mechanism>` says why a value is what it is."
    );
    out
}

/// `lessr config --explain <stage>`: every layer that spoke, winner marked.
pub fn explain(session: &Session, stage: &str) -> String {
    let mut out = String::new();
    match known::stage(stage) {
        Some(known) => {
            let _ = writeln!(out, "{stage} — {}", known.summary);
        }
        None => {
            let _ = writeln!(
                out,
                "{stage} — no mechanism of that name is registered in this build"
            );
        }
    }
    header(session, &mut out);

    let resolved = session.config.resolve(stage, session.scope.as_deref());

    let mode: Vec<(Layer, String)> = resolved
        .mode_trace()
        .iter()
        .map(|(layer, mode)| (*layer, mode.as_str().to_string()))
        .collect();
    field(&mut out, "mode", &mode, None);

    let level: Vec<(Layer, String)> = resolved
        .level_trace()
        .iter()
        .map(|(layer, level)| (*layer, level.as_str().to_string()))
        .collect();
    field(&mut out, "level", &level, None);

    for (key, value, layer) in settings_of(&resolved, stage) {
        let trace: Vec<(Layer, String)> = resolved
            .setting_trace(&key)
            .iter()
            .map(|(layer, value)| (*layer, value.to_string()))
            .collect();
        if trace.is_empty() {
            field(
                &mut out,
                &key,
                &[(layer, value)],
                Some("the mechanism's own default; no layer sets it"),
            );
        } else {
            field(&mut out, &key, &trace, None);
        }
    }

    if let Some(reason) = resolved.floor_reason() {
        let _ = writeln!(out, "\nsafety floor");
        let _ = writeln!(out, "  Lessr turned {stage} off here: {reason}.");
        let _ = writeln!(
            out,
            "  It sits above every layer, so no config file can promote it, and"
        );
        let _ = writeln!(
            out,
            "  `lessr on {stage}` will say so rather than appear to work (loop-safety 5)."
        );
    }

    problems(session, &mut out);
    out
}

/// One field, with every layer that set it and the one in force marked.
fn field(out: &mut String, name: &str, trace: &[(Layer, String)], note: Option<&str>) {
    let Some((winner, value)) = trace.last() else {
        return;
    };
    let _ = writeln!(out, "\n{name} = {value}");

    let layer_width = width(trace.iter().map(|(layer, _)| layer.as_str().len()));
    let value_width = width(trace.iter().map(|(_, value)| value.len()));

    for (index, (layer, value)) in trace.iter().enumerate() {
        let last = index + 1 == trace.len();
        let marker = if last { "->" } else { "  " };
        let said = if last {
            match note {
                Some(note) => format!("in force — {note}"),
                None => "in force".to_string(),
            }
        } else {
            // The whole point of keeping the losers: this is the line that
            // says your global value is being shadowed by the repo one.
            format!("shadowed by {}", winner.as_str())
        };
        let _ = writeln!(
            out,
            "  {marker} {:layer_width$}  {value:value_width$}  {said}",
            layer.as_str()
        );
    }
}

/// One line per file, so every path in play is on the screen.
fn header(session: &Session, out: &mut String) {
    let file = match (&session.unparseable, session.exists) {
        (Some(why), _) => format!("{} — {why}", session.file.display()),
        (None, false) => format!(
            "{} — none yet; these are the defaults",
            session.file.display()
        ),
        (None, true) => session.file.display().to_string(),
    };
    let _ = writeln!(out, "config    {file}");
    let _ = writeln!(
        out,
        "snapshot  {} — {}",
        session.snapshot.display(),
        session.snapshot_state.note()
    );

    let scope = match (&session.scope, &session.here) {
        (Some(repo), _) => format!("{} (--repo)", repo.display()),
        (None, Some(here)) => format!(
            "global — `--repo` scopes to {}, which you are in",
            here.display()
        ),
        (None, None) => "global — `--repo` needs a repository, and this is not one".to_string(),
    };
    let _ = writeln!(out, "scope     {scope}");
    if let Some(port) = session.port {
        let _ = writeln!(out, "proxy     port {port}");
    }
}

/// Everything wrong with the configuration, at the bottom where it is read
/// last and remembered.
fn problems(session: &Session, out: &mut String) {
    if session.problems.is_empty() {
        return;
    }
    let _ = writeln!(out, "\nreported, and not fatal:");
    let at_width = width(session.problems.iter().map(|problem| problem.at.len()));
    for problem in &session.problems {
        let _ = writeln!(
            out,
            "  {:at_width$}  {}",
            problem.at.as_str(),
            problem.says.as_str()
        );
    }
}

/// One line of the table.
struct Row {
    stage: String,
    mode: String,
    mode_layer: String,
    level: String,
    level_layer: String,
    floor: Option<String>,
    settings: Vec<(String, String, String)>,
}

impl Row {
    fn of(session: &Session, stage: &str) -> Row {
        let resolved = session.config.resolve(stage, session.scope.as_deref());
        Row {
            stage: stage.to_string(),
            mode: resolved.mode().as_str().to_string(),
            mode_layer: resolved.mode_layer().as_str().to_string(),
            level: resolved.level().as_str().to_string(),
            level_layer: resolved.level_layer().as_str().to_string(),
            floor: resolved.floor_reason().map(str::to_string),
            settings: settings_of(&resolved, stage)
                .into_iter()
                .map(|(key, value, layer)| (key, value, layer.as_str().to_string()))
                .collect(),
        }
    }
}

/// Every tunable in force for a stage: the ones a layer set, and the ones the
/// mechanism will fall back to.
///
/// The second kind is most of a fresh install, and leaving it out would make
/// `lessr config` print nothing at all on a machine nobody has configured —
/// exactly when a user is trying to find out what Lessr does.
fn settings_of(resolved: &Resolved, stage: &str) -> Vec<(String, String, Layer)> {
    let mut out: Vec<(String, String, Layer)> = Vec::new();

    for tunable in known::stage(stage).map_or(&[][..], |known| known.tunables) {
        let (value, layer) = match resolved.settings().get(tunable.key) {
            Some(value) => (
                value.to_string(),
                resolved
                    .setting_layer(tunable.key)
                    .unwrap_or(Layer::Default),
            ),
            None => (tunable.default.to_string(), Layer::Default),
        };
        out.push((tunable.key.to_string(), value, layer));
    }

    // Keys this build does not know are still in force for whatever does:
    // printing them is how a Pro key, or a typo, is visible here at all.
    for (key, value) in resolved.settings().iter() {
        if known::tunable(stage, key).is_none() {
            out.push((
                key.to_string(),
                value.to_string(),
                resolved.setting_layer(key).unwrap_or(Layer::Default),
            ));
        }
    }

    out
}

/// Which mechanisms the table covers: the ones this build has, plus anything
/// the configuration, the environment or the floor names.
fn stages(session: &Session) -> Vec<String> {
    let mut names: Vec<String> = known::STAGES
        .iter()
        .map(|stage| stage.name.to_string())
        .collect();
    for named in session.config.stage_names() {
        if named != known::DEFAULT && !names.iter().any(|name| name == named) {
            names.push(named.to_string());
        }
    }
    names
}

/// The width of the widest of a set of lengths.
fn width(lengths: impl IntoIterator<Item = usize>) -> usize {
    lengths.into_iter().max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(pairs: &[(Layer, &str)]) -> Vec<(Layer, String)> {
        pairs
            .iter()
            .map(|(layer, value)| (*layer, (*value).to_string()))
            .collect()
    }

    #[test]
    fn the_winner_is_marked_and_the_losers_say_who_beat_them() {
        let mut out = String::new();
        field(
            &mut out,
            "mode",
            &trace(&[
                (Layer::Default, "shadow"),
                (Layer::GlobalStage, "active"),
                (Layer::RepoStage, "off"),
            ]),
            None,
        );

        assert!(out.contains("mode = off"));
        assert!(
            out.contains("global-stage  active  shadowed by repo-stage"),
            "{out}"
        );
        assert!(out.contains("-> repo-stage    off     in force"), "{out}");
    }

    #[test]
    fn a_single_layer_is_still_a_trace() {
        let mut out = String::new();
        field(
            &mut out,
            "level",
            &trace(&[(Layer::Default, "balanced")]),
            Some("nothing sets it"),
        );
        assert!(out.contains("in force — nothing sets it"), "{out}");
    }
}
