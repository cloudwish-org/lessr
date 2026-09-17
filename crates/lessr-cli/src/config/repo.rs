//! Which repository the user is standing in.
//!
//! `--repo` scopes a change to the repository the command was run in
//! (`docs/CONFIG.md`), so something has to decide where that repository
//! starts. Walking up for `.git` is what every other tool in an agent's
//! toolchain does, and agreeing with them matters more than being clever: the
//! path written into `[repo."<path>"]` has to be a prefix of the directory the
//! hook will later run in, or the scope silently matches nothing.

use std::path::{Path, PathBuf};

/// The repository containing `start`, or `None` if it is not in one.
///
/// `.git` is tested for existence, not for being a directory: a worktree and a
/// submodule both carry a `.git` *file* pointing elsewhere, and both are
/// repositories a user would expect `--repo` to mean.
pub fn root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// The repository the process is standing in.
///
/// The working directory comes from the OS already absolute and with symlinks
/// resolved, and it is left exactly as it is: the hook resolves its own
/// directory the same way, and normalising one side and not the other is how a
/// repo scope stops matching the repository it names.
pub fn here() -> Option<PathBuf> {
    root(&std::env::current_dir().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subdirectory_resolves_to_the_repository_root() {
        let dir = tempfile::tempdir().unwrap();
        let root_dir = dir.path().join("work/monorepo");
        std::fs::create_dir_all(root_dir.join(".git")).unwrap();
        let deep = root_dir.join("crates/lessr-cli/src");
        std::fs::create_dir_all(&deep).unwrap();

        assert_eq!(root(&deep), Some(root_dir));
    }

    #[test]
    fn a_dot_git_file_counts_as_a_repository() {
        // What a worktree and a submodule both have.
        let dir = tempfile::tempdir().unwrap();
        let root_dir = dir.path().join("worktree");
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::write(root_dir.join(".git"), "gitdir: /elsewhere\n").unwrap();

        assert_eq!(root(&root_dir), Some(root_dir.clone()));
    }

    #[test]
    fn no_repository_is_none_rather_than_a_guess() {
        // The caller turns this into "say so and stop", never into a silent
        // global write.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(root(dir.path()), None);
    }
}
