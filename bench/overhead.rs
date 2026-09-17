//! End-to-end `lessr hook claude` timing, process spawn included.
//!
//! `crates/lessr-core/benches/pipeline.rs` measures the pipeline in process and
//! deliberately excludes spawn, because the per-stage budget in
//! `docs/PERFORMANCE.md` is about the filters. This bench measures the other
//! half: what the agent actually waits for on every tool call — fork, exec,
//! dynamic linking, the mmap of config and packs that technique 7 in
//! `docs/PERFORMANCE.md` buys us, the pipeline, and the single `write_all` back
//! out.
//!
//! Those are different quantities, so they have different budgets. This one
//! gates on `[end_to_end]`; `[overhead]` stays the per-stage contract.
//!
//! It also asserts the hook contract from `docs/ADAPTERS.md` on every sample:
//! exit 0, and stdout that is either empty — the signal for "nothing changed"
//! — or JSON the agent can parse. A hook that breaks the agent is a failure
//! whatever its p99, so a bad invocation aborts the run instead of becoming a
//! data point.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The binary under test. Built by `cargo build --release -p lessr-cli`.
const BIN_NAME: &str = "lessr";

/// A real PostToolUse payload: a `cargo test` run with the output the filters
/// exist to cut. Small on purpose — this bench isolates the fixed cost of an
/// invocation, and `pipeline.rs` already covers scaling with payload size.
const FIXTURE_PATH: &str = "fixtures/hook/claude_post_tool_use.json";

/// Must match `[end_to_end].fixture` in `bench/budget.toml`; the gate refuses
/// to run against a budget written for some other input.
const FIXTURE_NAME: &str = "claude-post-tool-use";

/// Spawn is far noisier than an in-process loop — scheduler, page cache and
/// dynamic linker all land in the tail — so the p99 needs samples behind it.
/// 200 puts the reported p99 at the third-slowest run, which is stable enough
/// to gate on without making CI wait minutes.
const SAMPLES: usize = 200;

/// The first exec pays for faulting in the binary and its libraries. Warming
/// keeps that one-off cost out of the percentiles instead of letting it define
/// the tail.
const WARMUP: usize = 20;

fn main() {
    let root = workspace_root();
    // `LESSR_BUDGET` exists so the gate itself can be tested: point it at a
    // budget nothing can meet and this bench must fail. The path used is
    // printed on every run, so a CI job that quietly loosened the budget says
    // so in its own log. Same knob as `crates/lessr-core/benches/pipeline.rs`.
    let budget_path = std::env::var_os("LESSR_BUDGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("bench/budget.toml"));
    let budget = Budget::load(&budget_path, "end_to_end");
    println!(
        "end-to-end hook overhead — budget p50 {} ms, p99 {} ms, gated on `{}`  [{}]\n",
        budget.p50_ms,
        budget.p99_ms,
        budget.fixture,
        budget_path.display()
    );

    if budget.fixture != FIXTURE_NAME {
        eprintln!(
            "[end_to_end].fixture is `{}`, but this bench feeds `{FIXTURE_NAME}`.\n\
             A budget measured on one payload cannot gate another.",
            budget.fixture
        );
        std::process::exit(1);
    }

    let bin = match locate_binary(&root) {
        Ok(path) => path,
        Err(reason) => {
            // Skipping would hide a regression behind a green run, so this is a
            // failure, not a skip.
            eprintln!("cannot find the lessr binary to measure:\n  {reason}\n");
            eprintln!("  build it first: cargo build --release -p lessr-cli");
            eprintln!("  or point $LESSR_BIN at the binary you want gated.");
            std::process::exit(1);
        }
    };
    let payload = std::fs::read(root.join(FIXTURE_PATH))
        .unwrap_or_else(|e| panic!("cannot read {FIXTURE_PATH}: {e}"));

    println!("  binary  {}", bin.display());
    println!("  samples {SAMPLES} (after {WARMUP} warm-up invocations)\n");

    let measured = measure(&bin, &payload);
    println!(
        "  {FIXTURE_NAME:>20}  {bytes:>9} B   p50 {p50:>7.3} ms   p99 {p99:>7.3} ms   (gated)",
        bytes = payload.len(),
        p50 = ms(measured.p50),
        p99 = ms(measured.p99),
    );

    let mut over_budget = Vec::new();
    if ms(measured.p99) > budget.p99_ms {
        over_budget.push(format!(
            "{FIXTURE_NAME}: p99 {:.3} ms over the {} ms budget",
            ms(measured.p99),
            budget.p99_ms
        ));
    }
    if ms(measured.p50) > budget.p50_ms {
        over_budget.push(format!(
            "{FIXTURE_NAME}: p50 {:.3} ms over the {} ms budget",
            ms(measured.p50),
            budget.p50_ms
        ));
    }

    if over_budget.is_empty() {
        println!("\nwithin budget");
        return;
    }
    eprintln!("\nend-to-end budget exceeded:");
    for line in &over_budget {
        eprintln!("  {line}");
    }
    eprintln!(
        "\nSee docs/PERFORMANCE.md. This is the install-level gate: every tool call\n\
         the agent makes pays it. The per-stage contract is [overhead], enforced by\n\
         `cargo bench -p lessr-core`."
    );
    std::process::exit(1);
}

struct Measured {
    p50: Duration,
    p99: Duration,
}

fn measure(bin: &Path, payload: &[u8]) -> Measured {
    for _ in 0..WARMUP {
        if let Err(reason) = run_once(bin, payload) {
            fail(&reason);
        }
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        match run_once(bin, payload) {
            Ok(elapsed) => samples.push(elapsed),
            Err(reason) => fail(&reason),
        }
    }

    samples.sort_unstable();
    Measured {
        p50: percentile(&samples, 50.0),
        p99: percentile(&samples, 99.0),
    }
}

/// One full invocation, timed from before the spawn to after the child is
/// reaped — the same window the agent blocks for.
fn run_once(bin: &Path, payload: &[u8]) -> Result<Duration, String> {
    let start = Instant::now();

    let mut child = Command::new(bin)
        .args(["hook", "claude"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot spawn {}: {e}", bin.display()))?;

    // Taking the handle here means it drops at the end of this statement, which
    // closes the pipe. The hook reads stdin to EOF, so without that close it
    // would wait forever and so would we.
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(payload)
        .map_err(|e| format!("cannot write the fixture to the hook's stdin: {e}"))?;

    // `wait_with_output` drains stdout and stderr together; reading them in
    // sequence would deadlock the moment one pipe filled.
    let output = child
        .wait_with_output()
        .map_err(|e| format!("cannot wait for the hook: {e}"))?;
    let elapsed = start.elapsed();

    if !output.status.success() {
        let how = match output.status.code() {
            Some(code) => format!("with status {code}"),
            None => "on a signal".to_string(),
        };
        return Err(format!(
            "the hook exited {how}; docs/ADAPTERS.md requires exit 0 on every path,\n  \
             internal errors included — a non-zero exit breaks the agent{}",
            stderr_head(&output.stderr)
        ));
    }
    // Empty stdout is correct, not a failure: it is how every agent we support
    // is told "nothing changed, keep your own output". What must never happen
    // is a non-empty body that is not the replacement envelope — Claude Code
    // discards a malformed one in silence and uses the original, so a broken
    // shape is indistinguishable from a working hook unless we check here.
    if !output.stdout.is_empty()
        && serde_json::from_slice::<serde_json::Value>(&output.stdout).is_err()
    {
        return Err(format!(
            "the hook wrote something that is not JSON; agents parse this stream,\n  \
             and anything they cannot parse is dropped in silence{}",
            stderr_head(&output.stderr)
        ));
    }

    Ok(elapsed)
}

/// A broken contract is not a slow run: folding it into a percentile would
/// report a healthy number for a hook that does not work.
fn fail(reason: &str) -> ! {
    eprintln!("\nend-to-end hook invocation failed:\n  {reason}");
    std::process::exit(1);
}

/// Whatever the hook said on the way down. A panic message comes first and the
/// backtrace after, so the head is the useful end; the cap keeps a full
/// backtrace out of the CI log.
fn stderr_head(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let head: String = text.chars().take(400).collect();
    let cut = if head.len() < text.len() { " …" } else { "" };
    format!("\n  stderr: {head}{cut}")
}

/// `$LESSR_BIN` first, so a packaged or cross-built binary can be gated exactly
/// as it ships; then `$CARGO_TARGET_DIR`, which CI caches often relocate; then
/// this workspace's own `target/release`.
fn locate_binary(root: &Path) -> Result<PathBuf, String> {
    // An override that is set but wrong is reported, never skipped over:
    // silently gating a different binary is worse than not gating at all.
    if let Some(explicit) = std::env::var_os("LESSR_BIN") {
        let path = PathBuf::from(explicit);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!(
                "$LESSR_BIN is set to {}, which is not a file",
                path.display()
            ))
        };
    }

    let path = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir).join("release").join(BIN_NAME),
        None => root.join("target").join("release").join(BIN_NAME),
    };
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("no binary at {}", path.display()))
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct Budget {
    p99_ms: f64,
    p50_ms: f64,
    fixture: String,
}

impl Budget {
    fn load(path: &Path, section: &str) -> Self {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        // `toml` 1.x parses a *value* from a `&str` — `[overhead]` alone reads as
        // an array and then trips on the rest of the file. A whole document is a
        // `Table`, so ask for one.
        let parsed: toml::Table = text
            .parse()
            .unwrap_or_else(|e| panic!("{} is not valid TOML: {e}", path.display()));
        let table = parsed
            .get(section)
            .unwrap_or_else(|| panic!("{} has no [{section}] section", path.display()));
        Self {
            p99_ms: number(table, section, "p99_ms"),
            p50_ms: number(table, section, "p50_ms"),
            fixture: table
                .get("fixture")
                .and_then(toml::Value::as_str)
                .unwrap_or_else(|| panic!("[{section}].fixture"))
                .to_string(),
        }
    }
}

/// Budgets are written the way a human would write them, so `10` and `10.0`
/// both have to parse.
fn number(table: &toml::Value, section: &str, key: &str) -> f64 {
    table
        .get(key)
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|int| int as f64))
        })
        .unwrap_or_else(|| panic!("[{section}].{key}"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bench/ always has a parent")
        .to_path_buf()
}
