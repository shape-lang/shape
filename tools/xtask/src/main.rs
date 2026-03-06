use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "xtask")]
#[command(about = "Workspace maintenance tasks for Shape")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    WorkspaceSmoke,
    Vmvalue {
        #[command(subcommand)]
        command: VmvalueCommand,
    },
    LineBudget {
        #[arg(value_enum, default_value = "check")]
        mode: GuardMode,
    },
    BenchmarkSpecialization {
        #[arg(value_enum, default_value = "check")]
        mode: BenchmarkMode,
    },
    NativeDocs {
        #[arg(value_enum, default_value = "check")]
        mode: GuardMode,
    },
    PerfRegressionGate {
        history_file: Option<PathBuf>,
        #[arg(long, default_value_t = 1.05)]
        threshold: f64,
    },
    MigrationMetrics {
        #[arg(value_enum, default_value = "metrics")]
        mode: MigrationMode,
    },
    LocCheck {
        #[arg(long, default_value_t = 1000)]
        warn_threshold: usize,
        #[arg(long, default_value_t = 1500)]
        fail_threshold: usize,
    },
    GrammarParity {
        corpus_dir: Option<PathBuf>,
    },
    Doctest {
        #[arg(long)]
        verbose: bool,
    },
}

#[derive(Subcommand, Debug)]
enum VmvalueCommand {
    Inventory,
    WriteBaseline,
    Check,
    CheckTrend,
    SnapshotCounts,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum GuardMode {
    Check,
    Report,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BenchmarkMode {
    Check,
    Inventory,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MigrationMode {
    Metrics,
    Report,
}

#[derive(Debug)]
struct VmvalueSnapshot {
    files: Vec<String>,
    total_references: usize,
    per_crate: BTreeMap<String, usize>,
    per_category_file_count: BTreeMap<String, usize>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("ERROR: {err:?}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = repo_root();

    match cli.command {
        Commands::WorkspaceSmoke => workspace_smoke(&root),
        Commands::Vmvalue { command } => vmvalue_command(&root, command),
        Commands::LineBudget { mode } => line_budget_guard(&root, mode),
        Commands::BenchmarkSpecialization { mode } => benchmark_specialization_guard(&root, mode),
        Commands::NativeDocs { mode } => native_docs_guard(&root, mode),
        Commands::PerfRegressionGate {
            history_file,
            threshold,
        } => perf_regression_gate(&root, history_file, threshold),
        Commands::MigrationMetrics { mode } => migration_metrics(&root, mode),
        Commands::LocCheck {
            warn_threshold,
            fail_threshold,
        } => loc_check(&root, warn_threshold, fail_threshold),
        Commands::GrammarParity { corpus_dir } => grammar_parity(&root, corpus_dir),
        Commands::Doctest { verbose } => run_doctest(&root, verbose),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask should be in repo root/tools/xtask")
        .to_path_buf()
}

fn normalize_rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sorted_rs_files(root: &Path, rel_dirs: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();

    for rel_dir in rel_dirs {
        let dir = root.join(rel_dir);
        if !dir.exists() {
            continue;
        }

        for entry in WalkDir::new(&dir)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.into_path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

fn count_lines(path: &Path) -> Result<usize> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(content.lines().count())
}

fn run_command(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
    println!("$ {} {}", program, args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to run command: {program}"))?;

    if !status.success() {
        bail!("command failed: {} {}", program, args.join(" "));
    }

    Ok(())
}

fn collect_vmvalue_snapshot(root: &Path) -> Result<VmvalueSnapshot> {
    let matcher = Regex::new(r"\bVMValue\b").expect("valid VMValue regex");

    let mut files = Vec::new();
    let mut total_references = 0usize;
    let mut per_crate = BTreeMap::new();
    let mut per_category_file_count = BTreeMap::new();

    for file in sorted_rs_files(root, &["crates", "bin", "tools"]) {
        let content = fs::read_to_string(&file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let matches = matcher.find_iter(&content).count();
        if matches == 0 {
            continue;
        }

        let rel = normalize_rel(&file, root);
        files.push(rel.clone());
        total_references += matches;

        let crate_name = crate_name_for_rel(&rel);
        *per_crate.entry(crate_name).or_insert(0) += matches;

        let category = classify_vmvalue_category(&rel).to_string();
        *per_category_file_count.entry(category).or_insert(0) += 1;
    }

    files.sort();

    Ok(VmvalueSnapshot {
        files,
        total_references,
        per_crate,
        per_category_file_count,
    })
}

fn crate_name_for_rel(rel: &str) -> String {
    let mut parts = rel.split('/');
    let first = parts.next().unwrap_or_default();
    if first == "crates" || first == "bin" || first == "tools" {
        return parts.next().unwrap_or(first).to_string();
    }
    first.to_string()
}

fn classify_vmvalue_category(rel: &str) -> &'static str {
    if rel.contains("/tests/") || rel.contains("/benches/") {
        return "test_or_bench";
    }

    const HOT_PATH_PREFIXES: &[&str] = &[
        "crates/shape-vm/src/executor",
        "crates/shape-runtime/src/engine",
        "crates/shape-runtime/src/context",
        "crates/shape-runtime/src/join_executor",
        "crates/shape-runtime/src/stream_executor",
        "crates/shape-runtime/src/window_executor",
        "crates/shape-runtime/src/window_manager",
        "crates/shape-runtime/src/simulation",
    ];

    const BOUNDARY_PREFIXES: &[&str] = &[
        "crates/shape-runtime/src/wire_conversion",
        "crates/shape-runtime/src/provider_registry",
        "crates/shape-runtime/src/plugins",
        "crates/shape-runtime/src/module_loader",
    ];

    if HOT_PATH_PREFIXES
        .iter()
        .any(|prefix| rel.starts_with(prefix))
    {
        return "hot_path";
    }

    if BOUNDARY_PREFIXES
        .iter()
        .any(|prefix| rel.starts_with(prefix))
    {
        return "boundary";
    }

    "support_or_legacy"
}

fn vmvalue_paths(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    (
        root.join("tasks/vmvalue_allowlist.txt"),
        root.join("tasks/vmvalue_allowlist_categories.tsv"),
        root.join("tasks/vmvalue_baseline_counts.json"),
    )
}

fn vmvalue_command(root: &Path, command: VmvalueCommand) -> Result<()> {
    match command {
        VmvalueCommand::Inventory => vmvalue_inventory(root),
        VmvalueCommand::WriteBaseline => vmvalue_write_baseline(root),
        VmvalueCommand::Check => vmvalue_check_allowlist(root),
        VmvalueCommand::CheckTrend => vmvalue_check_trend(root),
        VmvalueCommand::SnapshotCounts => vmvalue_snapshot_counts(root),
    }
}

fn vmvalue_inventory(root: &Path) -> Result<()> {
    let snapshot = collect_vmvalue_snapshot(root)?;

    println!("VMValue inventory");
    println!("  references: {}", snapshot.total_references);
    println!("  files:      {}", snapshot.files.len());

    println!();
    println!("Per-crate references:");
    let mut per_crate_rows: Vec<(String, usize)> = snapshot
        .per_crate
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    per_crate_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (crate_name, count) in per_crate_rows {
        println!("{:>7} {}", count, crate_name);
    }

    println!();
    println!("Location categories (file count):");
    for (category, count) in &snapshot.per_category_file_count {
        println!("  {:<17} {}", category, count);
    }

    Ok(())
}

fn vmvalue_write_baseline(root: &Path) -> Result<()> {
    let snapshot = collect_vmvalue_snapshot(root)?;
    let (allowlist_path, category_path, _) = vmvalue_paths(root);

    let mut allowlist_body = String::new();
    let mut category_body = String::new();

    for file in &snapshot.files {
        allowlist_body.push_str(file);
        allowlist_body.push('\n');

        category_body.push_str(file);
        category_body.push('\t');
        category_body.push_str(classify_vmvalue_category(file));
        category_body.push('\n');
    }

    fs::write(&allowlist_path, allowlist_body)
        .with_context(|| format!("failed to write {}", allowlist_path.display()))?;
    fs::write(&category_path, category_body)
        .with_context(|| format!("failed to write {}", category_path.display()))?;

    println!(
        "Wrote VMValue allowlist baseline to {} ({} files)",
        allowlist_path.display(),
        snapshot.files.len()
    );
    println!(
        "Wrote VMValue category baseline to {}",
        category_path.display()
    );

    Ok(())
}

fn read_lines_set(path: &Path) -> Result<BTreeSet<String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    Ok(lines)
}

fn vmvalue_check_allowlist(root: &Path) -> Result<()> {
    let snapshot = collect_vmvalue_snapshot(root)?;
    let (allowlist_path, _, _) = vmvalue_paths(root);

    if !allowlist_path.exists() {
        bail!(
            "Missing allowlist: {}\nRun: cargo xtask vmvalue write-baseline",
            allowlist_path.display()
        );
    }

    let baseline = read_lines_set(&allowlist_path)?;
    let current: BTreeSet<String> = snapshot.files.into_iter().collect();

    let unexpected: Vec<String> = current.difference(&baseline).cloned().collect();
    let stale: Vec<String> = baseline.difference(&current).cloned().collect();

    if !unexpected.is_empty() {
        eprintln!("ERROR: new VMValue usage detected outside baseline allowlist:");
        for file in unexpected {
            eprintln!("{file}");
        }
        bail!("VMValue guard failed");
    }

    if !stale.is_empty() {
        println!("Note: allowlist entries no longer using VMValue (cleanup opportunity):");
        for file in stale {
            println!("{file}");
        }
    }

    println!("VMValue guard passed: no new non-allowlisted VMValue references.");
    Ok(())
}

fn vmvalue_check_trend(root: &Path) -> Result<()> {
    let snapshot = collect_vmvalue_snapshot(root)?;
    let (_, _, baseline_path) = vmvalue_paths(root);

    if !baseline_path.exists() {
        bail!(
            "Missing baseline counts: {}\nRun: cargo xtask vmvalue snapshot-counts",
            baseline_path.display()
        );
    }

    let baseline_raw = fs::read_to_string(&baseline_path)
        .with_context(|| format!("failed to read {}", baseline_path.display()))?;
    let baseline: Value = serde_json::from_str(&baseline_raw)
        .with_context(|| format!("failed to parse {}", baseline_path.display()))?;

    let baseline_total = baseline
        .get("total_references")
        .and_then(Value::as_u64)
        .unwrap_or(0) as i64;
    let baseline_files = baseline
        .get("total_files")
        .and_then(Value::as_u64)
        .unwrap_or(0) as i64;

    let current_total = snapshot.total_references as i64;
    let current_files = snapshot.files.len() as i64;

    let ref_delta = current_total - baseline_total;
    let file_delta = current_files - baseline_files;

    println!("VMValue trend check");
    println!("  Baseline:  {baseline_total} refs in {baseline_files} files");
    println!("  Current:   {current_total} refs in {current_files} files");
    println!("  Delta:     {ref_delta} refs, {file_delta} files");

    println!();
    println!("Per-crate trend:");

    let baseline_per_crate = baseline
        .get("per_crate")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for (crate_name, current_count) in &snapshot.per_crate {
        let baseline_count = baseline_per_crate
            .get(crate_name)
            .and_then(Value::as_u64)
            .unwrap_or(0) as i64;

        let delta = *current_count as i64 - baseline_count;
        let marker = if delta > 0 {
            format!(" [REGRESSION +{delta}]")
        } else if delta < 0 {
            format!(" [reduced {delta}]")
        } else {
            String::new()
        };

        println!(
            "  {:<20} {:>4} (baseline {:>4}){}",
            crate_name, current_count, baseline_count, marker
        );
    }

    println!();

    if ref_delta > 0 {
        bail!("FAIL: VMValue references increased by {ref_delta} since baseline.");
    }

    if ref_delta == 0 {
        println!("OK: VMValue references unchanged from baseline.");
    } else {
        println!(
            "OK: VMValue references decreased by {} since baseline.",
            -ref_delta
        );
    }

    Ok(())
}

fn vmvalue_snapshot_counts(root: &Path) -> Result<()> {
    let snapshot = collect_vmvalue_snapshot(root)?;
    let (_, _, baseline_path) = vmvalue_paths(root);

    let mut per_crate_json = Map::new();
    for (crate_name, count) in &snapshot.per_crate {
        per_crate_json.insert(crate_name.clone(), Value::from(*count));
    }

    let mut per_category_json = Map::new();
    for (category, count) in &snapshot.per_category_file_count {
        per_category_json.insert(category.clone(), Value::from(*count));
    }

    let payload = json!({
        "snapshot_date": Utc::now().date_naive().to_string(),
        "total_references": snapshot.total_references,
        "total_files": snapshot.files.len(),
        "per_crate": Value::Object(per_crate_json),
        "per_category": Value::Object(per_category_json),
    });

    let serialized = serde_json::to_string_pretty(&payload)? + "\n";
    fs::write(&baseline_path, serialized)
        .with_context(|| format!("failed to write {}", baseline_path.display()))?;

    println!(
        "Wrote VMValue baseline counts to {}",
        baseline_path.display()
    );
    Ok(())
}

fn line_budget_guard(root: &Path, mode: GuardMode) -> Result<()> {
    let scan_root = root.join("crates/shape-jit/src/translator");
    if !scan_root.exists() {
        bail!(
            "line budget scan root does not exist: {}",
            scan_root.display()
        );
    }

    let default_limit = env::var("LINE_BUDGET_DEFAULT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(800);

    let mut rows: Vec<(usize, usize, bool, String)> = Vec::new();
    let mut violations = 0usize;

    for file in sorted_rs_files(root, &["crates/shape-jit/src/translator"]) {
        let rel = normalize_rel(&file, root);
        let lines = count_lines(&file)?;
        let limit = default_limit;
        let violation = lines > limit;

        if violation {
            violations += 1;
        }

        rows.push((lines, limit, violation, rel));
    }

    if matches!(mode, GuardMode::Report) {
        println!("Line Budget Report (crates/shape-jit/src/translator)");
        println!("default_limit={default_limit}");
        println!();

        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.3.cmp(&b.3)));
        for (lines, limit, violation, rel) in &rows {
            if *violation {
                println!("FAIL  {:>4} > {:>4}  {rel}", lines, limit);
            } else {
                println!("OK    {:>4} <= {:>4}  {rel}", lines, limit);
            }
        }

        println!();
        println!("violations={violations}");
    }

    if violations > 0 {
        if matches!(mode, GuardMode::Check) {
            println!("Line budget violations detected:");
            rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.3.cmp(&b.3)));
            for (lines, limit, violation, rel) in &rows {
                if *violation {
                    println!("  FAIL  {:>4} > {:>4}  {rel}", lines, limit);
                }
            }
        }
        bail!("line budget guard failed with {violations} violation(s)");
    }

    if matches!(mode, GuardMode::Check) {
        println!("Line budget guard passed.");
    }

    Ok(())
}

fn benchmark_specialization_guard(root: &Path, mode: BenchmarkMode) -> Result<()> {
    let source_dirs = [
        "crates/shape-jit/src",
        "crates/shape-vm/src",
        "crates/shape-runtime/src",
        "crates/shape-core/src",
    ];

    let pattern = Regex::new(
        r"01_fib|02_fib_iter|03_sieve|04_mandelbrot|05_spectral|06_ackermann|07_sum_loop|08_collatz|09_matrix_mul|10_primes_count|benchmark[ _-]?profile|profiled_bench|benchmark_kernel",
    )
    .expect("valid benchmark specialization regex");

    println!("[bench-specialization-guard] scanning runtime/compiler sources");

    let mut hits = Vec::new();

    for dir in source_dirs {
        for file in sorted_rs_files(root, &[dir]) {
            let rel = normalize_rel(&file, root);
            let content = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;

            for (idx, line) in content.lines().enumerate() {
                if pattern.is_match(line) {
                    hits.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
                }
            }
        }
    }

    if !hits.is_empty() {
        for hit in &hits {
            println!("{hit}");
        }

        if matches!(mode, BenchmarkMode::Check) {
            println!();
            println!(
                "[bench-specialization-guard] found benchmark-specific logic in runtime/compiler sources"
            );
            println!(
                "[bench-specialization-guard] remove benchmark-name/profile special cases and keep generic optimizations only"
            );
            bail!("benchmark specialization guard failed");
        }
    } else if matches!(mode, BenchmarkMode::Inventory) {
        println!("[bench-specialization-guard] no benchmark-specific patterns found");
    }

    println!("[bench-specialization-guard] OK");
    Ok(())
}

fn native_docs_guard(root: &Path, mode: GuardMode) -> Result<()> {
    let docs_dir = root.join("docs/book/book/book-site/src/content/docs/advanced");
    if !docs_dir.exists() {
        println!(
            "native_docs_guard: skipped (docs directory missing: {})",
            docs_dir.display()
        );
        return Ok(());
    }

    let canonical = docs_dir.join("native-c-interop.mdx");
    let required_link = "[Native C Interop](/advanced/native-c-interop/)";

    let link_back_files = vec![
        docs_dir.join("annotations.mdx"),
        docs_dir.join("comptime.mdx"),
        docs_dir.join("comptime-annotations-cookbook.mdx"),
        docs_dir.join("projects.mdx"),
    ];

    let required_canonical_patterns = vec![
        "This chapter is the single normative source for Shape native C interop.",
        "## Core Syntax",
        "## Marshalling Matrix",
        "## `type C` Layout Contract",
        "## JIT Execution Contract",
    ];

    let banned_legacy_patterns = vec![
        "### Native C Interop (Core Syntax)",
        "## Native C Interop (Core + Comptime)",
        "## Recipe 3: Native C Bindings (`extern C` + `type C`)",
        "Supported native scalar families include:",
        "What this gives you:",
        "For core native C interop, `[native-dependencies]` supports provider metadata:",
    ];

    let declaration_regex = Regex::new(r"extern C fn|type C [A-Za-z_][A-Za-z0-9_]*|cview<|cmut<")
        .expect("valid native declaration regex");

    let mut failures = Vec::new();
    let mut declaration_hits = Vec::new();

    if !canonical.exists() {
        failures.push(format!(
            "Missing canonical chapter: {}",
            canonical.display()
        ));
    } else {
        let canonical_content = fs::read_to_string(&canonical)
            .with_context(|| format!("failed to read {}", canonical.display()))?;

        for pattern in &required_canonical_patterns {
            if !canonical_content.contains(pattern) {
                failures.push(format!(
                    "Canonical chapter missing required section marker: {pattern}"
                ));
            }
        }
    }

    for file in &link_back_files {
        if !file.exists() {
            failures.push(format!(
                "Missing required link-back file: {}",
                file.display()
            ));
            continue;
        }

        let content = fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;

        if !content.contains(required_link) {
            failures.push(format!("Missing canonical link in {}", file.display()));
        }

        for pattern in &banned_legacy_patterns {
            if content.contains(pattern) {
                failures.push(format!(
                    "Legacy duplicated native docs content found in {}: {pattern}",
                    file.display()
                ));
            }
        }
    }

    for entry in WalkDir::new(&docs_dir)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mdx") {
            continue;
        }
        if path == canonical {
            continue;
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        for (line_no, line) in content.lines().enumerate() {
            if declaration_regex.is_match(line) {
                declaration_hits.push(format!(
                    "{}:{}:{}",
                    normalize_rel(path, root),
                    line_no + 1,
                    line.trim()
                ));
            }
        }
    }

    if !declaration_hits.is_empty() {
        failures.push("Native declaration examples found outside canonical chapter:".to_string());
        for hit in &declaration_hits {
            failures.push(format!("  {hit}"));
        }
    }

    if matches!(mode, GuardMode::Report) {
        println!("native_docs_guard: canonical={}", canonical.display());
        println!("native_docs_guard: checked_files={}", link_back_files.len());
        println!(
            "native_docs_guard: declaration_hits={}",
            declaration_hits.len()
        );
        println!("native_docs_guard: failures={}", failures.len());
    }

    if !failures.is_empty() {
        for failure in failures {
            println!("FAIL: {failure}");
        }
        bail!("native docs guard failed");
    }

    if matches!(mode, GuardMode::Check) {
        println!("Native docs guard passed.");
    }

    Ok(())
}

fn parse_numeric(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let len = values.len();
    if len == 1 {
        values[0]
    } else if len % 2 == 1 {
        values[len / 2]
    } else {
        (values[len / 2 - 1] + values[len / 2]) / 2.0
    }
}

fn perf_regression_gate(root: &Path, history_file: Option<PathBuf>, threshold: f64) -> Result<()> {
    let history = history_file
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .unwrap_or_else(|| root.join("benchmarks/tracking/jit_weekly_history.tsv"));

    if !history.exists() {
        println!(
            "[perf-regression-gate] History file not found: {}",
            history.display()
        );
        println!("[perf-regression-gate] No history to check - skipping (pass).");
        return Ok(());
    }

    let content = fs::read_to_string(&history)
        .with_context(|| format!("failed to read {}", history.display()))?;
    let lines: Vec<&str> = content.lines().collect();

    if lines.len() < 2 {
        println!("[perf-regression-gate] History file has no data rows - skipping (pass).");
        return Ok(());
    }

    let data_rows = lines.len() - 1;
    if data_rows < 2 {
        println!(
            "[perf-regression-gate] Only {} data row(s) - need at least 2 for comparison. Skipping (pass).",
            data_rows
        );
        return Ok(());
    }

    let headers: Vec<&str> = lines[0].split('\t').collect();
    let mut numeric_cols = Vec::new();

    for (idx, header) in headers.iter().enumerate() {
        if header.contains("geomean") || header.contains("ratio") {
            numeric_cols.push((idx, (*header).to_string()));
        }
    }

    let rows: Vec<Vec<&str>> = lines[1..]
        .iter()
        .map(|line| line.split('\t').collect())
        .collect();

    println!(
        "[perf-regression-gate] Checking {} numeric columns across {} data rows",
        numeric_cols.len(),
        rows.len()
    );
    println!(
        "[perf-regression-gate] Regression threshold: {:.0}% above rolling median",
        (threshold - 1.0) * 100.0
    );
    println!();

    let mut regressions = 0usize;

    for (col_idx, name) in numeric_cols {
        let mut recent_values = Vec::new();

        for row in rows.iter().rev() {
            if let Some(raw) = row.get(col_idx)
                && let Some(value) = parse_numeric(raw)
            {
                recent_values.push(value);
                if recent_values.len() == 4 {
                    break;
                }
            }
        }

        if recent_values.len() < 2 {
            println!(
                "  {:<35}  SKIP  (only {} valid value(s))",
                name,
                recent_values.len()
            );
            continue;
        }

        let latest = recent_values[0];
        let mut window = recent_values[1..].to_vec();
        let median_value = median(&mut window);

        if median_value == 0.0 {
            println!("  {:<35}  SKIP  (median is zero)", name);
            continue;
        }

        let regression_limit = median_value * threshold;

        if latest > regression_limit {
            let pct = ((latest / median_value) - 1.0) * 100.0;
            println!(
                "  {:<35}  FAIL  latest={:.4}  median={:.4}  (+{:.1}%)",
                name, latest, median_value, pct
            );
            regressions += 1;
        } else if latest > median_value {
            let pct = ((latest / median_value) - 1.0) * 100.0;
            println!(
                "  {:<35}  OK    latest={:.4}  median={:.4}  (+{:.1}%, within threshold)",
                name, latest, median_value, pct
            );
        } else {
            let pct = (1.0 - (latest / median_value)) * 100.0;
            println!(
                "  {:<35}  OK    latest={:.4}  median={:.4}  (-{:.1}%)",
                name, latest, median_value, pct
            );
        }
    }

    println!();

    if regressions > 0 {
        bail!(
            "[perf-regression-gate] FAIL: {} regression(s) detected (>{:.0}% above rolling median)",
            regressions,
            (threshold - 1.0) * 100.0
        );
    }

    println!("[perf-regression-gate] PASS: no regressions detected");
    Ok(())
}

fn migration_metrics(root: &Path, mode: MigrationMode) -> Result<()> {
    let vmvalue_re = Regex::new(r"\bVMValue\b").expect("valid VMValue regex");
    let conversion_re =
        Regex::new(r"\b(to_vmvalue|from_vmvalue)\b").expect("valid conversion regex");

    let total_refs = count_regex_matches(root, &["crates", "bin", "tools"], &vmvalue_re)?;

    let hot_path_files = count_files_with_regex(
        root,
        &[
            "crates/shape-vm/src/executor",
            "crates/shape-vm/src/compiler",
        ],
        &vmvalue_re,
    )?;

    let conversion_calls =
        count_regex_matches(root, &["crates/shape-vm/src/executor"], &conversion_re)?;

    let file_loc_violations = count_loc_violations(root, &["crates", "bin", "tools"], 1500)?;

    match mode {
        MigrationMode::Metrics => {
            println!("vmvalue_total_count={total_refs}");
            println!("vmvalue_hot_path_count={hot_path_files}");
            println!("to_vmvalue_hot_path_calls={conversion_calls}");
            println!("file_loc_violations={file_loc_violations}");
        }
        MigrationMode::Report => {
            const BASELINE_TOTAL: i64 = 1345;
            const BASELINE_HOT_PATH: i64 = 36;
            const BASELINE_CONVERSION_CALLS: i64 = 58;

            println!("=== Shape Migration Metrics ===");
            println!();
            println!(
                "{:<30} {:>6}  (baseline: {}, delta: {:+})",
                "vmvalue_total_count",
                total_refs,
                BASELINE_TOTAL,
                (total_refs as i64) - BASELINE_TOTAL
            );
            println!(
                "{:<30} {:>6}  (baseline: {}, delta: {:+})",
                "vmvalue_hot_path_count",
                hot_path_files,
                BASELINE_HOT_PATH,
                (hot_path_files as i64) - BASELINE_HOT_PATH
            );
            println!(
                "{:<30} {:>6}  (baseline: {}, delta: {:+})",
                "to_vmvalue_hot_path_calls",
                conversion_calls,
                BASELINE_CONVERSION_CALLS,
                (conversion_calls as i64) - BASELINE_CONVERSION_CALLS
            );
            println!("{:<30} {:>6}", "file_loc_violations", file_loc_violations);
        }
    }

    Ok(())
}

fn count_regex_matches(root: &Path, dirs: &[&str], regex: &Regex) -> Result<usize> {
    let mut count = 0usize;
    for file in sorted_rs_files(root, dirs) {
        let content = fs::read_to_string(&file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        count += regex.find_iter(&content).count();
    }
    Ok(count)
}

fn count_files_with_regex(root: &Path, dirs: &[&str], regex: &Regex) -> Result<usize> {
    let mut count = 0usize;
    for file in sorted_rs_files(root, dirs) {
        let content = fs::read_to_string(&file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        if regex.is_match(&content) {
            count += 1;
        }
    }
    Ok(count)
}

fn count_loc_violations(root: &Path, dirs: &[&str], fail_threshold: usize) -> Result<usize> {
    let mut count = 0usize;
    for file in sorted_rs_files(root, dirs) {
        let lines = count_lines(&file)?;
        if lines > fail_threshold {
            count += 1;
        }
    }
    Ok(count)
}

fn loc_check(root: &Path, warn_threshold: usize, fail_threshold: usize) -> Result<()> {
    let mut warn_count = 0usize;
    let mut fail_count = 0usize;

    for file in sorted_rs_files(root, &["crates", "bin", "tools", "extensions"]) {
        let loc = count_lines(&file)?;
        let rel = normalize_rel(&file, root);

        if loc >= fail_threshold {
            println!("FAIL: {rel} ({loc} LOC, threshold {fail_threshold})");
            fail_count += 1;
        } else if loc >= warn_threshold {
            println!("WARN: {rel} ({loc} LOC, threshold {warn_threshold})");
            warn_count += 1;
        }
    }

    println!();
    println!(
        "Summary: {} file(s) over {} LOC (fail), {} file(s) over {} LOC (warn)",
        fail_count, fail_threshold, warn_count, warn_threshold
    );

    if fail_count > 0 {
        bail!("loc check failed");
    }

    Ok(())
}

fn find_tree_sitter_cli(root: &Path) -> Option<PathBuf> {
    let status = Command::new("tree-sitter")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if let Ok(ok) = status
        && ok.success()
    {
        return Some(PathBuf::from("tree-sitter"));
    }

    let local_candidates = [
        root.join("tree-sitter-shape/node_modules/.bin/tree-sitter"),
        root.join("tree-sitter-shape/node_modules/tree-sitter-cli/tree-sitter"),
    ];

    for candidate in local_candidates {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn grammar_parity(root: &Path, corpus_dir: Option<PathBuf>) -> Result<()> {
    let corpus = corpus_dir
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .unwrap_or_else(|| root.join("crates/shape-core/examples"));

    let Some(tree_sitter_cli) = find_tree_sitter_cli(root) else {
        bail!("tree-sitter CLI not found. Install it or build tree-sitter-shape.");
    };

    let shape_bin = root.join("target/release/shape");
    if !shape_bin.exists() {
        bail!(
            "shape binary not found at {}. Run: cargo build --release -p shape-cli --bin shape",
            shape_bin.display()
        );
    }

    let mut shape_files = Vec::new();
    for entry in WalkDir::new(&corpus)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.into_path();
        if path.extension().and_then(|e| e.to_str()) == Some("shape") {
            shape_files.push(path);
        }
    }
    shape_files.sort();

    let mut pest_ok = BTreeSet::new();
    let mut ts_ok = BTreeSet::new();

    for file in &shape_files {
        let rel = normalize_rel(file, root);

        let pest_status = Command::new(&shape_bin)
            .arg("parse")
            .arg(file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("failed to run {} parse", shape_bin.display()))?;

        if pest_status.success() {
            pest_ok.insert(rel.clone());
        }

        let ts_status = Command::new(&tree_sitter_cli)
            .arg("parse")
            .arg(file)
            .arg("--quiet")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| "failed to run tree-sitter parse")?;

        if ts_status.success() {
            ts_ok.insert(rel);
        }
    }

    let pest_only: Vec<String> = pest_ok.difference(&ts_ok).cloned().collect();
    let ts_only: Vec<String> = ts_ok.difference(&pest_ok).cloned().collect();

    if !pest_only.is_empty() {
        println!("Files parsed by Pest but NOT tree-sitter:");
        for file in &pest_only {
            println!("  {file}");
        }
    }

    if !ts_only.is_empty() {
        println!("Files parsed by tree-sitter but NOT Pest:");
        for file in &ts_only {
            println!("  {file}");
        }
    }

    println!();
    println!("Summary: {} files tested", shape_files.len());
    println!("  Pest parse OK:         {}", pest_ok.len());
    println!("  Tree-sitter parse OK:  {}", ts_ok.len());
    println!(
        "  Divergences:           {}",
        pest_only.len() + ts_only.len()
    );

    if !pest_only.is_empty() || !ts_only.is_empty() {
        bail!("grammar parity check failed");
    }

    Ok(())
}

fn run_doctest(root: &Path, verbose: bool) -> Result<()> {
    let docs_dir = root.join("docs/book/book/book-site/src/content/docs");

    println!("=== Shape Doctest CI ===");
    println!("Docs directory: {}", docs_dir.display());

    let mut args = vec![
        "run",
        "-p",
        "shape-cli",
        "--bin",
        "shape",
        "--",
        "doctest",
        docs_dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid docs directory path"))?,
    ];

    if verbose {
        args.push("--verbose");
    }

    run_command("cargo", &args, root)
}

fn workspace_smoke(root: &Path) -> Result<()> {
    println!("[smoke] VMValue guard");
    vmvalue_check_allowlist(root)?;

    println!("[smoke] Line budget guard");
    line_budget_guard(root, GuardMode::Check)?;

    println!("[smoke] Benchmark specialization guard");
    benchmark_specialization_guard(root, BenchmarkMode::Check)?;

    println!("[smoke] Native docs single-source guard");
    native_docs_guard(root, GuardMode::Check)?;

    println!("[smoke] Cargo check (workspace)");
    run_command("cargo", &["check", "--workspace"], root)?;

    println!("[smoke] Cargo tests (workspace, all targets)");
    run_command("cargo", &["test", "--workspace", "--all-targets"], root)?;

    println!("[smoke] JIT perf regression gate (advisory)");
    if let Err(err) = perf_regression_gate(root, None, 1.05) {
        println!("[smoke] WARNING: perf regression gate FAILED (advisory, non-blocking)");
        println!("[smoke] detail: {err}");
    } else {
        println!("[smoke] perf gate: PASSED");
    }

    println!("[smoke] OK");
    Ok(())
}
