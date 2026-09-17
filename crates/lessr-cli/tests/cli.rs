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
fn run(home: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut child = Command::new(LESSR)
        .args(args)
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
fn the_hook_returns_its_input_and_exits_zero_when_no_stage_is_registered() {
    let home = sandbox();
    let input = fixture("claude_post_tool_use.json");
    let out = run(home.path(), &["hook", "claude"], Some(&input));

    assert!(out.status.success(), "the hook must never fail the agent");

    let before: serde_json::Value = serde_json::from_slice(&input).unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("hook stdout must be JSON");
    assert_eq!(
        before["tool_response"]["stdout"], after["tool_response"]["stdout"],
        "an empty pipeline must be invisible"
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
fn the_hook_writes_nothing_but_the_payload_to_stdout() {
    // Everything on this stream enters the agent's context and is billed on
    // every later turn. A stray log line is a real cost, not untidiness.
    let home = sandbox();
    let input = fixture("claude_post_tool_use.json");
    let out = run(home.path(), &["hook", "claude"], Some(&input));

    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .expect("stdout must be exactly one JSON document and nothing else");

    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    for marketing in ["lessr.dev", "upgrade", "left on the table"] {
        assert!(
            !text.contains(marketing),
            "hook stdout contains {marketing:?}"
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
        vec!["on", "gate"],
        vec!["off", "gate"],
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

/// Kept last: a compile-time reminder that the binary path is a real file.
#[test]
fn the_binary_exists() {
    assert!(PathBuf::from(LESSR).exists(), "{LESSR}");
}
