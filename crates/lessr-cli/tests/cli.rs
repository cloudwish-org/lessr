//! End-to-end tests of the `lessr` binary.
//!
//! These run the real binary as a subprocess, because the properties worth
//! testing here are about a process: what it writes to stdout, what it writes
//! to stderr, and what it exits with. A hook that returns the right bytes but
//! exits non-zero still breaks the agent.
//!
//! Every test that could touch a real config sets `HOME` and `LESSR_HOME` to a
//! temporary directory first. A test suite that edits the developer's own
//! editor config is a bug.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The binary under test, built by cargo for us.
const LESSR: &str = env!("CARGO_BIN_EXE_lessr");

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hook")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Run `lessr <args>` with a sandboxed home and optional stdin.
///
/// The working directory is the sandbox too: `--repo` walks up for a `.git`,
/// and a test that ran in the checkout would find Lessr's own repository.
fn run(home: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    run_in(home, home, args, stdin)
}

/// The same, from a chosen directory. For everything `--repo` touches.
fn run_in(home: &Path, cwd: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(LESSR)
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("LESSR_HOME", home.join("lessr-config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn lessr");

    {
        let mut pipe = child.stdin.take().expect("stdin was piped");
        if let Some(bytes) = stdin {
            let _ = pipe.write_all(bytes);
        }
        // Dropping closes it, which is what tells a command reading stdin to
        // stop waiting.
    }

    child.wait_with_output().expect("failed to wait for lessr")
}

fn sandbox() -> tempfile::TempDir {
    tempfile::tempdir().expect("cannot make a temp dir")
}

/// Lessr's own config directory inside a sandbox.
fn config_dir(home: &Path) -> PathBuf {
    home.join("lessr-config")
}

/// `config.toml` in a sandbox.
fn config_file(home: &Path) -> PathBuf {
    config_dir(home).join("config.toml")
}

/// The snapshot the hook path maps, in a sandbox. The name is part of the
/// contract between this binary and the hook: they have to agree without being
/// told.
fn snapshot_file(home: &Path) -> PathBuf {
    config_dir(home).join("config.bin")
}

/// Write `config.toml` by hand, the way a user with an editor does.
fn hand_write(home: &Path, text: &str) {
    let path = config_file(home);
    std::fs::create_dir_all(path.parent().expect("a parent")).unwrap();
    std::fs::write(&path, text).unwrap();
}

/// A directory that looks like a repository to anything walking up for `.git`.
fn repository(home: &Path, name: &str) -> PathBuf {
    let root = home.join(name);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    root
}

/// stdout, or a panic naming stderr, which is where a failure explains itself.
fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn version_and_help_work() {
    let home = sandbox();
    let out = run(home.path(), &["--version"], None);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("0.1.0"));

    let out = run(home.path(), &["--help"], None);
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    for command in ["init", "gain", "show", "uninstall", "pro"] {
        assert!(help.contains(command), "`{command}` missing from --help");
    }
}

#[test]
fn the_hook_is_completely_silent_when_no_stage_is_registered() {
    // The free binary registers no stages yet, so the hook must be invisible:
    // exit 0 and not one byte on stdout. Silence is how every agent we support
    // is told "leave the output alone".
    let home = sandbox();
    let input = fixture("claude_post_tool_use.json");
    let out = run(home.path(), &["hook", "claude"], Some(&input));

    assert!(out.status.success(), "the hook must never fail the agent");
    assert!(
        out.stdout.is_empty(),
        "expected silence, got {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn the_hook_survives_everything_thrown_at_it() {
    let home = sandbox();
    // Each of these has broken a hook in some shipping tool at some point.
    let inputs: &[&[u8]] = &[
        b"",
        b"null",
        b"[]",
        b"{",
        b"not json at all",
        b"{\"tool_name\": 12345}",
        b"{\"tool_response\": {\"stdout\": null}}",
        &[0xff, 0xfe, 0x00, 0x01],
    ];
    for input in inputs {
        let out = run(home.path(), &["hook", "claude"], Some(input));
        assert!(
            out.status.success(),
            "exit {:?} on input {:?} — a hook must never break the agent",
            out.status.code(),
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn an_unknown_agent_still_exits_zero() {
    let home = sandbox();
    let out = run(home.path(), &["hook", "nosuchagent"], Some(b"{}"));
    assert!(out.status.success());
    assert_eq!(out.stdout, b"{}");
}

#[test]
fn the_hook_never_writes_anything_of_its_own_to_stdout() {
    // Everything on this stream enters the agent's context and is billed on
    // every later turn, so the only thing allowed here is a replacement the
    // agent asked for. Never a log line, never a word about Pro.
    let home = sandbox();
    for fixture_name in ["claude_post_tool_use.json", "claude_pre_tool_use.json"] {
        let input = fixture(fixture_name);
        let out = run(home.path(), &["hook", "claude"], Some(&input));
        assert!(
            out.stdout.is_empty(),
            "{fixture_name}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn pro_lists_both_tiers_and_links_once() {
    let home = sandbox();
    let out = run(home.path(), &["pro"], None);
    assert!(out.status.success());

    let page = String::from_utf8_lossy(&out.stdout);
    assert!(page.contains("cachefix"), "the Pro list is the point");
    assert!(page.contains("gate"), "so is what they already have");
    assert_eq!(
        page.matches("https://lessr.dev/pro").count(),
        1,
        "one link, not a campaign"
    );
}

#[test]
fn unshipped_commands_say_so_instead_of_pretending() {
    let home = sandbox();
    // Exit 2 is "not yet", distinct from 1 for "went wrong", so a script can
    // tell them apart.
    for args in [
        vec!["gain"],
        vec!["serve"],
        vec!["show", "abcd"],
        vec!["bench"],
    ] {
        let out = run(home.path(), &args, None);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`lessr {}` should exit 2",
            args.join(" ")
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("not implemented yet"),
            "`lessr {}` should say why: {err}",
            args.join(" ")
        );
        assert!(
            out.stdout.is_empty(),
            "`lessr {}` must not print a plausible-looking result",
            args.join(" ")
        );
    }
}

#[test]
fn init_show_changes_nothing_on_disk() {
    let home = sandbox();
    let claude_dir = home.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    let original = "{\n  \"theme\": \"dark\"\n}\n";
    std::fs::write(&settings, original).unwrap();

    let out = run(home.path(), &["init", "--show"], None);
    assert!(
        out.status.success(),
        "init --show failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        original,
        "--show must not write anything, ever"
    );
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(
        printed.contains("hook") || printed.contains("Claude Code"),
        "--show should describe the change it would make: {printed}"
    );
}

#[test]
fn init_without_yes_declines_when_there_is_no_one_to_ask() {
    // Closed stdin means no terminal. Reading that as consent would be how you
    // silently rewrite configs in CI.
    let home = sandbox();
    let claude_dir = home.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    let original = "{\n  \"theme\": \"dark\"\n}\n";
    std::fs::write(&settings, original).unwrap();

    let out = run(home.path(), &["init"], Some(b""));
    assert!(out.status.success());
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        original,
        "a config was written without confirmation"
    );
}

#[test]
fn init_then_uninstall_restores_the_file_byte_for_byte() {
    let home = sandbox();
    let claude_dir = home.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    // Deliberately idiosyncratic: odd indentation and key order that a
    // careless round trip would normalise away.
    let original = "{\n    \"theme\": \"dark\",\n    \"verbose\": true\n}\n";
    std::fs::write(&settings, original).unwrap();

    let out = run(home.path(), &["init", "--yes", "--agent", "claude"], None);
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patched = std::fs::read_to_string(&settings).unwrap();
    assert_ne!(patched, original, "init did not patch anything");
    assert!(patched.contains("lessr"), "the hook was not installed");
    assert!(patched.contains("dark"), "init lost an unrelated setting");

    let out = run(
        home.path(),
        &["uninstall", "--yes", "--agent", "claude"],
        None,
    );
    assert!(
        out.status.success(),
        "uninstall failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        original,
        "uninstall must restore byte-for-byte, whitespace and key order included"
    );
}

#[test]
fn init_is_idempotent() {
    let home = sandbox();
    let claude_dir = home.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    std::fs::write(&settings, "{}\n").unwrap();

    run(home.path(), &["init", "--yes", "--agent", "claude"], None);
    let once = std::fs::read_to_string(&settings).unwrap();
    run(home.path(), &["init", "--yes", "--agent", "claude"], None);
    let twice = std::fs::read_to_string(&settings).unwrap();

    assert_eq!(once, twice, "running init twice must not stack hooks");
    assert_eq!(
        once.matches("hook claude").count(),
        1,
        "the hook was registered more than once"
    );
}

#[test]
fn an_unknown_agent_name_is_an_error_not_a_no_op() {
    let home = sandbox();
    let out = run(home.path(), &["init", "--agent", "emacs-doctor"], None);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("emacs-doctor"), "{err}");
}

#[test]
fn uninstalling_an_agent_whose_integration_is_a_file_actually_deletes_it() {
    // OpenCode and pi are integrated by a plugin file Lessr owns rather than a
    // config we patch, so their whole uninstall is a Delete. If Delete is not
    // treated as touching the filesystem, uninstall reports "Nothing to
    // change" and silently leaves the plugin running.
    let home = sandbox();
    let out = run(home.path(), &["init", "--yes", "--agent", "opencode"], None);
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let plugin = home.path().join(".config/opencode/plugin/lessr.ts");
    assert!(
        plugin.exists(),
        "init did not write the plugin: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let out = run(
        home.path(),
        &["uninstall", "--yes", "--agent", "opencode"],
        None,
    );
    assert!(
        out.status.success(),
        "uninstall failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !plugin.exists(),
        "uninstall left the plugin behind: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn the_installed_hook_command_survives_a_path_with_a_space() {
    // Every agent hands the hook command to a shell. The default macOS install
    // directory contains a space, so an unquoted path breaks the hook on every
    // tool call, silently.
    let home = sandbox();
    let spaced = home.path().join("Application Support/lessr");
    std::fs::create_dir_all(&spaced).unwrap();
    let copied = spaced.join("lessr");
    std::fs::copy(LESSR, &copied).unwrap();

    let claude_dir = home.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(claude_dir.join("settings.json"), "{}\n").unwrap();

    let out = std::process::Command::new(&copied)
        .args(["init", "--yes", "--agent", "claude"])
        .env("HOME", home.path())
        .env("LESSR_HOME", home.path().join("lessr-config"))
        .output()
        .expect("failed to spawn the copied binary");
    assert!(
        out.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let settings = std::fs::read_to_string(claude_dir.join("settings.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&settings).unwrap();
    let command = parsed["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .expect("a hook command was written");
    assert!(
        command.contains('\'') || !command.contains(' ') || command.starts_with('"'),
        "the path was left unquoted and will break in a shell: {command}"
    );
    // Prove it rather than trusting the shape: run it through a real shell.
    let via_shell = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{command} < /dev/null"))
        .env("HOME", home.path())
        .output()
        .expect("failed to run the hook command through a shell");
    assert!(
        via_shell.status.success(),
        "the installed command does not run in a shell: {command}\nstderr: {}",
        String::from_utf8_lossy(&via_shell.stderr)
    );
}

// ---------------------------------------------------------------------------
// Configuration: `lessr on|off|shadow|level` and `lessr config`.
//
// The contract is `docs/CONFIG.md`. Two of its promises are worth more than
// the rest and are tested hardest here: nothing is hidden (every value says
// which layer set it), and nothing in a config file is fatal.
// ---------------------------------------------------------------------------

#[test]
fn config_on_a_machine_with_no_config_file_still_says_what_lessr_is_doing() {
    // The first thing a new user runs. It must print the mechanisms, their
    // defaults and where those come from — not an empty table, and certainly
    // not an error about a file they were never asked to write.
    let home = sandbox();
    let out = run(home.path(), &["config"], None);
    let printed = stdout(&out);

    for mechanism in ["gate", "trap", "dedup", "detect"] {
        assert!(
            printed.contains(mechanism),
            "{mechanism} missing: {printed}"
        );
    }
    assert!(
        printed.contains("shadow"),
        "every mechanism ships in shadow"
    );
    assert!(printed.contains("balanced"));
    assert!(printed.contains("max_repeated_lines"), "{printed}");
    assert!(
        printed.contains("none yet"),
        "it should say the file does not exist yet: {printed}"
    );
    assert!(
        !config_file(home.path()).exists(),
        "reading the configuration must not write one"
    );
}

#[test]
fn off_then_config_shows_off_and_who_said_so() {
    let home = sandbox();
    let out = run(home.path(), &["off", "gate"], None);
    let written = stdout(&out);
    assert!(written.contains("gate"), "{written}");
    assert!(
        config_file(home.path()).exists(),
        "the file should have been created: {written}"
    );

    let printed = stdout(&run(home.path(), &["config"], None));
    let row = printed
        .lines()
        .find(|line| line.starts_with("gate"))
        .unwrap_or_else(|| panic!("no gate row in {printed}"));
    assert!(row.contains("off"), "{row}");
    assert!(
        row.contains("global-stage"),
        "the layer that set it is half the answer: {row}"
    );
}

#[test]
fn level_rejects_a_level_that_does_not_exist_and_names_the_three() {
    let home = sandbox();
    let out = run(home.path(), &["level", "gate", "medium"], None);

    assert_eq!(out.status.code(), Some(1), "an unknown level is an error");
    let err = String::from_utf8_lossy(&out.stderr);
    for level in ["safe", "balanced", "aggressive"] {
        assert!(err.contains(level), "the three should be named: {err}");
    }
    assert!(
        !config_file(home.path()).exists(),
        "a rejected command must not write a file"
    );
}

#[test]
fn explain_names_the_layer_that_wins_and_the_one_it_shadows() {
    // The question `--explain` exists to answer is "I set this and nothing
    // happened". Printing only the winner cannot answer it.
    let home = sandbox();
    let repo = repository(home.path(), "monorepo");

    run(home.path(), &["on", "gate"], None);
    run_in(home.path(), &repo, &["off", "gate", "--repo"], None);

    let printed = stdout(&run_in(
        home.path(),
        &repo,
        &["config", "--repo", "--explain", "gate"],
        None,
    ));

    assert!(printed.contains("mode = off"), "{printed}");
    assert!(
        printed.contains("global-stage") && printed.contains("shadowed by repo-stage"),
        "the global value must be shown, and shown as shadowed: {printed}"
    );
    assert!(printed.contains("in force"), "{printed}");
}

#[test]
fn a_repo_setting_overrides_the_global_one_only_inside_that_repo() {
    let home = sandbox();
    let repo = repository(home.path(), "monorepo");
    let elsewhere = repository(home.path(), "other");

    run(home.path(), &["on", "gate"], None);
    run_in(home.path(), &repo, &["off", "gate", "--repo"], None);

    let inside = stdout(&run_in(home.path(), &repo, &["config", "--repo"], None));
    let row = inside
        .lines()
        .find(|line| line.starts_with("gate"))
        .unwrap_or_default();
    assert!(
        row.contains("off") && row.contains("repo-stage"),
        "{inside}"
    );

    let outside = stdout(&run_in(
        home.path(),
        &elsewhere,
        &["config", "--repo"],
        None,
    ));
    let row = outside
        .lines()
        .find(|line| line.starts_with("gate"))
        .unwrap_or_default();
    assert!(
        row.contains("active") && row.contains("global-stage"),
        "the repo layer is scoped to its repository: {outside}"
    );
}

#[test]
fn repo_outside_a_repository_stops_rather_than_writing_a_global_setting() {
    // Quietly widening the scope would change every repository the user has.
    let home = sandbox();
    let out = run(home.path(), &["off", "gate", "--repo"], None);

    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(".git"), "it should say what is missing: {err}");
    assert!(!config_file(home.path()).exists());
}

#[test]
fn a_value_that_does_not_parse_falls_back_and_is_reported_not_fatal() {
    // `docs/CONFIG.md`: a bad config line must not break someone's agent.
    let home = sandbox();
    hand_write(
        home.path(),
        "[stages.gate]\nmode = \"banana\"\nmax_repeated_lines = \"many\"\n",
    );

    let printed = stdout(&run(home.path(), &["config"], None));
    let row = printed
        .lines()
        .find(|line| line.starts_with("gate"))
        .unwrap_or_default();
    assert!(
        row.contains("shadow") && row.contains("default"),
        "the compiled-in default has to be what is shown as in force: {printed}"
    );
    assert!(
        printed.contains("stages.gate.mode") && printed.contains("off, shadow, active"),
        "the bad value must be reported, with the values that would work: {printed}"
    );
    assert!(
        printed.contains("stages.gate.max_repeated_lines"),
        "{printed}"
    );

    // And the hook still runs, which is the whole point of not being fatal.
    let out = run(home.path(), &["hook", "claude"], Some(b"{}"));
    assert!(out.status.success());
}

#[test]
fn a_key_nothing_reads_is_surfaced_rather_than_swallowed() {
    let home = sandbox();
    hand_write(
        home.path(),
        "[stages.gate]\nmax_repated_lines = 3\n\n[stagse]\ngate = \"on\"\n",
    );

    let printed = stdout(&run(home.path(), &["config"], None));
    assert!(
        printed.contains("stages.gate.max_repated_lines"),
        "a typo that silently does nothing is worse than one that says so: {printed}"
    );
    assert!(printed.contains("stagse"), "{printed}");
}

#[test]
fn config_set_keeps_the_comments_and_the_key_order_around_it() {
    // This file is something a person wrote. A tool that reformats it to make
    // one change is a tool people stop letting near their files.
    let home = sandbox();
    let original = "\
# how I like it
[stages.gate]
# the agent re-reads a lot, so keep this low
max_repeated_lines = 3   # ← deliberate
level = \"safe\"
mode = \"active\"

[stages.trap]
json_bytes = 131072
";
    hand_write(home.path(), original);

    let out = run(
        home.path(),
        &["config", "set", "stages.gate.level", "aggressive"],
        None,
    );
    stdout(&out);

    let after = std::fs::read_to_string(config_file(home.path())).unwrap();
    assert_eq!(
        after,
        original.replace("level = \"safe\"", "level = \"aggressive\""),
        "only the one value may change"
    );
}

#[test]
fn a_hand_edit_of_the_toml_regenerates_the_snapshot() {
    // The bug worth preventing above all others here: the file says one thing,
    // the snapshot the hook maps says another, and nobody is told.
    let home = sandbox();
    run(home.path(), &["off", "gate"], None);

    let snapshot = snapshot_file(home.path());
    let before = std::fs::read(&snapshot).expect("a write should have compiled one");

    hand_write(
        home.path(),
        "[stages.gate]\nmode = \"active\"\nlevel = \"aggressive\"\n",
    );
    let printed = stdout(&run(home.path(), &["config"], None));
    assert!(
        printed.contains("rebuilt"),
        "it must say it regenerated: {printed}"
    );

    let after = std::fs::read(&snapshot).unwrap();
    assert_ne!(before, after, "the snapshot did not follow the file");
    assert!(after.starts_with(b"LESSRCFG"), "and it is still a snapshot");

    // Twice in a row is not two writes: an identical configuration must leave
    // the file alone.
    let printed = stdout(&run(home.path(), &["config"], None));
    assert!(printed.contains("up to date"), "{printed}");
    assert_eq!(std::fs::read(&snapshot).unwrap(), after);
}

#[test]
fn on_refuses_to_promote_a_stage_the_self_healing_table_turned_off() {
    // Loop-safety rule 5 put it there; `docs/CONFIG.md` puts the floor above
    // every layer. Accepting the command in silence would leave the user
    // believing a mechanism is running that is not.
    let home = sandbox();
    let repo = repository(home.path(), "monorepo");
    std::fs::create_dir_all(config_dir(home.path())).unwrap();
    std::fs::write(
        config_dir(home.path()).join("healing.toml"),
        format!(
            "[[off]]\nstage = \"gate\"\nrepo = {:?}\nreason = \"handles expanded on 14 % of its outputs\"\n",
            repo.display().to_string()
        ),
    )
    .unwrap();

    let out = run_in(home.path(), &repo, &["on", "gate", "--repo"], None);
    assert_eq!(out.status.code(), Some(1), "it must not look like success");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("handles expanded on 14 %"),
        "the reason is required (loop-safety 5): {err}"
    );
    assert!(
        err.contains("cannot") && err.contains("Nothing was written"),
        "it must say configuration cannot override the floor: {err}"
    );
    assert!(
        !config_file(home.path()).exists(),
        "a refused command writes nothing"
    );

    // And the floor is visible without asking for it.
    let printed = stdout(&run_in(home.path(), &repo, &["config", "--repo"], None));
    let row = printed
        .lines()
        .find(|line| line.starts_with("gate"))
        .unwrap_or_default();
    assert!(
        row.contains("off") && row.contains("safety-floor"),
        "{printed}"
    );
    assert!(
        printed.contains("handles expanded on 14 %"),
        "with its reason: {printed}"
    );
}

#[test]
fn the_snapshot_lives_beside_the_config_file_under_a_name_the_hook_can_find() {
    // The hook maps this path without being told where it is, so its name and
    // place are a contract, not an implementation detail.
    let home = sandbox();
    run(home.path(), &["shadow", "gate"], None);

    let snapshot = snapshot_file(home.path());
    assert!(snapshot.exists(), "no snapshot at {}", snapshot.display());
    assert_eq!(snapshot.parent(), config_file(home.path()).parent());
    assert!(
        std::fs::read(&snapshot).unwrap().starts_with(b"LESSRCFG"),
        "the magic lessr-core writes and refuses to read without"
    );
}

/// Kept last: a compile-time reminder that the binary path is a real file.
#[test]
fn the_binary_exists() {
    assert!(PathBuf::from(LESSR).exists(), "{LESSR}");
}
