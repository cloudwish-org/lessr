//! A unified diff, so `lessr init --show` can show the change instead of the
//! file.
//!
//! `docs/ADAPTERS.md` says init prints every change it would make. A settings
//! file is mostly the user's own keys; printing it whole buries the one hook
//! entry we are adding in a wall of unrelated JSON, and a user who cannot see
//! the change cannot consent to it.

use std::fmt::Write as _;

/// Lines of unchanged context around each change, as `diff -u` uses.
const CONTEXT: usize = 3;

/// Above this many lines on either side, we print a summary instead. The
/// comparison is quadratic in lines; an agent config that large is not a
/// config we are patching, it is something that went wrong.
const MAX_LINES: usize = 4_000;

/// One step of the edit script between two files.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    /// A line present in both, by index into the old side.
    Keep(usize),
    /// A line only in the old side.
    Remove(usize),
    /// A line only in the new side.
    Add(usize),
}

/// Render `before` → `after` as a unified diff, labelled with `label`.
///
/// Returns an empty string when the two are identical, so a caller can decide
/// whether a change is worth printing at all.
pub(crate) fn unified(before: &str, after: &str, label: &str) -> String {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    if old == new {
        return String::new();
    }

    let header = format!("--- {label}\n+++ {label}\n");
    if old.len() > MAX_LINES || new.len() > MAX_LINES {
        return format!(
            "{header}@@ {} lines replaced by {} @@\n",
            old.len(),
            new.len()
        );
    }

    let script = edits(&old, &new);
    let mut out = header;
    for (start, end) in hunks(&script) {
        write_hunk(&mut out, &script, &old, &new, start, end);
    }
    out
}

/// The edit script, from a longest-common-subsequence table.
///
/// LCS rather than a heuristic: the inputs are small and the point of the
/// printout is that the user can trust it, which means the removed lines have
/// to be the ones that were actually removed.
fn edits(old: &[&str], new: &[&str]) -> Vec<Edit> {
    let (n, m) = (old.len(), new.len());
    let width = m + 1;
    // `table[i * width + j]` is the LCS length of `old[i..]` and `new[j..]`.
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if old[i] == new[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }

    let mut script = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            script.push(Edit::Keep(i));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            script.push(Edit::Remove(i));
            i += 1;
        } else {
            script.push(Edit::Add(j));
            j += 1;
        }
    }
    script.extend((i..n).map(Edit::Remove));
    script.extend((j..m).map(Edit::Add));
    script
}

/// Half-open ranges of the script worth printing: every change, plus
/// [`CONTEXT`] lines around it, with overlapping ranges merged.
fn hunks(script: &[Edit]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (index, edit) in script.iter().enumerate() {
        if matches!(edit, Edit::Keep(_)) {
            continue;
        }
        let start = index.saturating_sub(CONTEXT);
        let end = (index + CONTEXT + 1).min(script.len());
        match ranges.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => ranges.push((start, end)),
        }
    }
    ranges
}

/// One `@@ … @@` block and its lines.
fn write_hunk(
    out: &mut String,
    script: &[Edit],
    old: &[&str],
    new: &[&str],
    start: usize,
    end: usize,
) {
    let (old_start, new_start) = position(script, start);
    let (old_end, new_end) = position(script, end);
    let (old_len, new_len) = (old_end - old_start, new_end - new_start);

    // Unified diff numbers lines from 1, except for an empty range, which is
    // reported at the line it would follow.
    let old_first = if old_len == 0 {
        old_start
    } else {
        old_start + 1
    };
    let new_first = if new_len == 0 {
        new_start
    } else {
        new_start + 1
    };
    let _ = writeln!(out, "@@ -{old_first},{old_len} +{new_first},{new_len} @@");

    for edit in &script[start..end] {
        match *edit {
            Edit::Keep(i) => {
                let _ = writeln!(out, " {}", old[i]);
            }
            Edit::Remove(i) => {
                let _ = writeln!(out, "-{}", old[i]);
            }
            Edit::Add(j) => {
                let _ = writeln!(out, "+{}", new[j]);
            }
        }
    }
}

/// How many lines of each side the script has consumed before `index`.
fn position(script: &[Edit], index: usize) -> (usize, usize) {
    let mut old = 0;
    let mut new = 0;
    for edit in &script[..index.min(script.len())] {
        match edit {
            Edit::Keep(_) => {
                old += 1;
                new += 1;
            }
            Edit::Remove(_) => old += 1,
            Edit::Add(_) => new += 1,
        }
    }
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_files_produce_nothing() {
        assert_eq!(unified("a\nb\n", "a\nb\n", "x"), "");
    }

    #[test]
    fn an_added_line_is_shown_with_context_not_the_whole_file() {
        let before = (1..=20).map(|n| format!("line {n}\n")).collect::<String>();
        let mut after = before.clone();
        after.push_str("line 21\n");

        let diff = unified(&before, &after, "settings.json");
        assert!(
            diff.starts_with("--- settings.json\n+++ settings.json\n"),
            "{diff}"
        );
        assert!(diff.contains("+line 21\n"), "{diff}");
        assert!(diff.contains("@@ -18,3 +18,4 @@"), "{diff}");
        // Context only: the first sixteen lines stay out of the printout.
        assert!(!diff.contains(" line 1\n"), "{diff}");
    }

    #[test]
    fn a_replaced_line_shows_both_sides() {
        let diff = unified("a\nb\nc\n", "a\nB\nc\n", "f");
        assert!(diff.contains("-b\n"), "{diff}");
        assert!(diff.contains("+B\n"), "{diff}");
        assert!(diff.contains(" a\n"), "{diff}");
    }

    #[test]
    fn a_new_file_is_all_additions() {
        let diff = unified("", "{\n}\n", "f");
        assert!(diff.contains("@@ -0,0 +1,2 @@"), "{diff}");
        assert!(diff.contains("+{\n"), "{diff}");
    }

    #[test]
    fn a_huge_file_is_summarised_rather_than_compared() {
        let big = "x\n".repeat(MAX_LINES + 1);
        let diff = unified(&big, "x\n", "f");
        assert!(diff.contains("lines replaced by"), "{diff}");
    }
}
