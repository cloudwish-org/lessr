//! Where the user's home and Lessr's own config directory are.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The directory name Lessr owns inside the platform's config root.
const DIR_NAME: &str = "lessr";

/// The override every test and CI job uses instead of the real config root.
///
/// It names the config directory itself, not its parent: `LESSR_HOME=/tmp/x`
/// puts the backups in `/tmp/x/backups`. A single variable keeps a CI run from
/// scattering files through a developer's real `~/.config`.
const HOME_OVERRIDE: &str = "LESSR_HOME";

/// The two roots every adapter works from.
///
/// Adapters never call [`Paths::detect`] themselves; they are handed a
/// `Paths`. That is what makes the tests hermetic: [`Paths::with_roots`]
/// points both roots at a temporary directory and no code path reads an
/// environment variable behind the test's back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The user's home directory. Agent configs hang off this.
    pub home: PathBuf,
    /// Lessr's own config directory, per `docs/ARCHITECTURE.md`. Backups live
    /// here, not next to the agent's config: a stray `.bak` beside
    /// `settings.json` is something another tool will eventually read.
    pub config: PathBuf,
}

impl Paths {
    /// The real locations for this machine.
    ///
    /// Config directory: `LESSR_HOME` if set, else `$XDG_CONFIG_HOME/lessr` or
    /// `~/.config/lessr` on Linux, `~/Library/Application Support/lessr` on
    /// macOS, `%APPDATA%\lessr` on Windows.
    pub fn detect() -> Result<Paths> {
        let home = home_dir(std::env::var_os).ok_or(Error::NoHome)?;
        let config = config_dir(&home, std::env::var_os);
        Ok(Paths { home, config })
    }

    /// Inject both roots. Reads no environment variable, so a test that uses
    /// it cannot be perturbed by the machine it runs on.
    pub fn with_roots(home: impl Into<PathBuf>, config: impl Into<PathBuf>) -> Paths {
        Paths {
            home: home.into(),
            config: config.into(),
        }
    }

    /// `<config>/backups`: one directory per agent underneath, plus the
    /// index `lessr uninstall` reads.
    pub fn backups_dir(&self) -> PathBuf {
        self.config.join("backups")
    }
}

/// The home directory, as the platform names it.
///
/// The variable is looked up through `var` rather than read directly so the
/// tests can hand in an environment instead of mutating the process's own
/// (which is `unsafe` in edition 2024, and shared with every other test).
fn home_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    non_empty(&var, key).map(PathBuf::from)
}

/// The config directory, per `docs/ARCHITECTURE.md`.
///
/// The platform branches are `cfg!` expressions rather than `#[cfg]` blocks so
/// that all three type-check on every platform: a Windows-only path that only
/// compiles on Windows is a path that breaks on a Friday.
fn config_dir(home: &Path, var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    if let Some(over) = non_empty(&var, HOME_OVERRIDE) {
        return PathBuf::from(over);
    }
    if cfg!(windows) {
        return non_empty(&var, "APPDATA").map_or_else(
            || home.join("AppData").join("Roaming").join(DIR_NAME),
            |appdata| PathBuf::from(appdata).join(DIR_NAME),
        );
    }
    if cfg!(target_os = "macos") {
        return home.join("Library").join("Application Support").join(DIR_NAME);
    }
    // XDG says a relative `XDG_CONFIG_HOME` is to be ignored, and ignoring it
    // is also what keeps us from writing into whatever directory the agent
    // happened to be launched from.
    match non_empty(&var, "XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join(DIR_NAME),
        _ => home.join(".config").join(DIR_NAME),
    }
}

/// An environment variable, treating "set to nothing" as "not set": an empty
/// `HOME` would otherwise resolve every config path to the filesystem root.
fn non_empty(var: &impl Fn(&str) -> Option<OsString>, key: &str) -> Option<OsString> {
    var(key).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake environment: `fake(&[("HOME", "/home/dev")])`.
    fn fake(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(*value))
        }
    }

    #[test]
    fn with_roots_uses_exactly_the_roots_it_is_given() {
        let paths = Paths::with_roots("/tmp/home", "/tmp/config");
        assert_eq!(paths.home, PathBuf::from("/tmp/home"));
        assert_eq!(paths.config, PathBuf::from("/tmp/config"));
        assert_eq!(paths.backups_dir(), PathBuf::from("/tmp/config/backups"));
    }

    #[test]
    fn the_home_override_wins_over_every_platform_rule() {
        let env = fake(&[
            ("LESSR_HOME", "/ci/lessr"),
            ("XDG_CONFIG_HOME", "/ci/xdg"),
            ("APPDATA", "C:\\ci"),
        ]);
        assert_eq!(
            config_dir(Path::new("/home/dev"), env),
            PathBuf::from("/ci/lessr")
        );
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        assert!(home_dir(fake(&[("HOME", ""), ("USERPROFILE", "")])).is_none());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_follows_xdg_and_falls_back_to_dot_config() {
        let with_xdg = config_dir(Path::new("/home/dev"), fake(&[("XDG_CONFIG_HOME", "/xdg")]));
        assert_eq!(with_xdg, PathBuf::from("/xdg/lessr"));

        let relative = config_dir(Path::new("/home/dev"), fake(&[("XDG_CONFIG_HOME", "xdg")]));
        assert_eq!(relative, PathBuf::from("/home/dev/.config/lessr"));

        let bare = config_dir(Path::new("/home/dev"), fake(&[]));
        assert_eq!(bare, PathBuf::from("/home/dev/.config/lessr"));
    }
}
