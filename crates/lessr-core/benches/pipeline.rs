//! Pipeline overhead on 4 KB, 64 KB and 1 MB of `cargo test` output.
//!
//! This is the gate CI enforces: p50 and p99 are compared against
//! `bench/budget.toml` and the bench exits non-zero when either is over. See
//! `docs/PERFORMANCE.md` for the budget and the techniques it assumes.
//!
//! The stage measured here is `LineFilter`, a stand-in shaped like a real hot
//! filter — one `memchr` pass, no allocation per line, one preallocated output
//! buffer. It exists so the budget is enforced from the first commit; the real
//! filters arrive with `lessr-gate` in phase 2 and replace it.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use lessr_core::{Mode, PipelineBuilder, Saving, Stage, Tier, Tokens, ToolKind, ToolResult};

/// Sizes from `docs/PERFORMANCE.md`. The last one is the fixture the budget
/// names.
const SIZES: [(&str, usize); 3] = [
    ("4kb", 4 * 1024),
    ("64kb", 64 * 1024),
    ("cargo-test-1mb", 1024 * 1024),
];

fn main() {
    // `LESSR_BUDGET` exists so the gate itself can be tested: point it at a
    // budget nothing can meet and this bench must fail. The path used is
    // printed on every run, so a CI job that quietly loosened the budget says
    // so in its own log.
    let budget_path = std::env::var_os("LESSR_BUDGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("bench/budget.toml"));
    let budget = Budget::load(&budget_path);
    println!(
        "pipeline overhead — budget p50 {} ms, p99 {} ms, gated on `{}`  [{}]\n",
        budget.p50_ms,
        budget.p99_ms,
        budget.fixture,
        budget_path.display()
    );

    let seed = std::fs::read(workspace_root().join("bench/fixtures/cargo-test-seed.txt"))
        .expect("bench seed fixture is missing");

    let mut over_budget = Vec::new();
    for (name, size) in SIZES {
        let content = grow(&seed, size);
        let measured = measure(&content);
        let gated = name == budget.fixture;

        println!(
            "  {name:>14}  {bytes:>9} B   p50 {p50:>7.3} ms   p99 {p99:>7.3} ms   {mark}",
            bytes = content.len(),
            p50 = ms(measured.p50),
            p99 = ms(measured.p99),
            mark = if gated { "(gated)" } else { "" }
        );

        if gated {
            if ms(measured.p99) > budget.p99_ms {
                over_budget.push(format!(
                    "{name}: p99 {:.3} ms over the {} ms budget",
                    ms(measured.p99),
                    budget.p99_ms
                ));
            }
            if ms(measured.p50) > budget.p50_ms {
                over_budget.push(format!(
                    "{name}: p50 {:.3} ms over the {} ms budget",
                    ms(measured.p50),
                    budget.p50_ms
                ));
            }
        }
    }

    if over_budget.is_empty() {
        println!("\nwithin budget");
        return;
    }
    eprintln!("\noverhead budget exceeded:");
    for line in &over_budget {
        eprintln!("  {line}");
    }
    eprintln!("\nSee docs/PERFORMANCE.md. The budget is a contract, not a target.");
    std::process::exit(1);
}

/// A stage shaped like a real hot filter: one pass, no per-line allocation.
struct LineFilter;

impl Stage for LineFilter {
    fn name(&self) -> &'static str {
        "linefilter"
    }

    fn tier(&self) -> Tier {
        Tier::Free
    }

    fn on_tool_result(&mut self, result: &mut ToolResult) -> Option<Saving> {
        let input = result.content.clone();
        let before = input.len() as u64;
        let mut out = BytesMut::with_capacity(input.len());

        let mut start = 0usize;
        while start < input.len() {
            let end = match memchr::memchr(b'\n', &input[start..]) {
                Some(offset) => start + offset,
                None => input.len(),
            };
            let line = &input[start..end];
            if keep(line) {
                out.extend_from_slice(line);
                out.extend_from_slice(b"\n");
            }
            start = end + 1;
        }

        let after = out.len() as u64;
        result.content = out.freeze();
        Some(Saving::bytes(before, after, Tokens::Exact(before - after)))
    }
}

/// Drop blank lines and passing test lines; keep everything else. Deliberately
/// cheap: the point is to measure the pipeline, not this predicate.
fn keep(line: &[u8]) -> bool {
    if line.is_empty() {
        return false;
    }
    !(line.starts_with(b"test ") && line.ends_with(b" ... ok"))
}

struct Measured {
    p50: Duration,
    p99: Duration,
}

fn measure(content: &Bytes) -> Measured {
    // Enough samples for a stable p99 without making the 1 MB case slow.
    let iterations = if content.len() >= 1024 * 1024 {
        300
    } else {
        2_000
    };

    let mut builder = PipelineBuilder::new();
    builder
        .register(LineFilter, Mode::Active)
        .expect("registering one stage cannot fail");
    let mut pipeline = builder.build();

    // Warm the caches and the branch predictors before measuring.
    for _ in 0..(iterations / 10).max(10) {
        let mut result = ToolResult::new(ToolKind::Shell, "Bash", content.clone());
        black_box(pipeline.run_tool_result(&mut result));
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut result = ToolResult::new(ToolKind::Shell, "Bash", content.clone());
        let start = Instant::now();
        let savings = pipeline.run_tool_result(&mut result);
        samples.push(start.elapsed());
        black_box(savings);
        black_box(&result);
    }

    samples.sort_unstable();
    Measured {
        p50: percentile(&samples, 50.0),
        p99: percentile(&samples, 99.0),
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

/// Repeat the seed until it reaches `target` bytes, cutting on a line boundary
/// so the content stays shaped like real output.
fn grow(seed: &[u8], target: usize) -> Bytes {
    let mut out = Vec::with_capacity(target + seed.len());
    while out.len() < target {
        out.extend_from_slice(seed);
    }
    out.truncate(target);
    if let Some(last) = memchr::memrchr(b'\n', &out) {
        out.truncate(last + 1);
    }
    Bytes::from(out)
}

struct Budget {
    p99_ms: f64,
    p50_ms: f64,
    fixture: String,
}

impl Budget {
    fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        // A `Table`, not a `Value`: `Value`'s `FromStr` reads one value, so a
        // whole document stops it at the first section header.
        let parsed: toml::Table = text.parse().expect("bench/budget.toml is not valid TOML");
        let overhead = parsed
            .get("overhead")
            .expect("bench/budget.toml has no [overhead] section");
        Self {
            p99_ms: number(overhead, "p99_ms"),
            p50_ms: number(overhead, "p50_ms"),
            fixture: overhead
                .get("fixture")
                .and_then(toml::Value::as_str)
                .expect("[overhead].fixture")
                .to_string(),
        }
    }
}

/// Read a budget field that may have been written as `10` or as `10.0`.
fn number(section: &toml::Value, key: &str) -> f64 {
    let value = section
        .get(key)
        .unwrap_or_else(|| panic!("bench/budget.toml: [overhead].{key} is missing"));
    value
        .as_float()
        .or_else(|| value.as_integer().map(|v| v as f64))
        .unwrap_or_else(|| panic!("bench/budget.toml: [overhead].{key} is not a number"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> always has two parents")
        .to_path_buf()
}
