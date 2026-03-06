//! Real Strategy Benchmark
//!
//! Benchmarks actual strategy execution with real market data and multiple indicators.
//! This is the honest benchmark - not synthetic toy examples.

use shape_core::engine::ShapeEngine;
use shape_core::runtime::initialize_shared_runtime;
use shape_core::ExecutionMode;
use std::time::Instant;

fn init() {
    let _ = initialize_shared_runtime();
}

/// Complex multi-indicator strategy - realistic trading logic
const COMPLEX_STRATEGY: &str = r#"
// Load 6 months of ES futures data (~175K rows)
data("market-data", { symbol: "ES", start: "2024-01-01", end: "2024-06-30" });

let close = series("close");
let high = series("high");
let low = series("low");
let volume = series("volume");

// Calculate multiple indicators (this is where real work happens)
let sma_10 = __intrinsic_rolling_mean(close, 10);
let sma_20 = __intrinsic_rolling_mean(close, 20);
let sma_50 = __intrinsic_rolling_mean(close, 50);
let ema_12 = __intrinsic_ema(close, 12);
let ema_26 = __intrinsic_ema(close, 26);
let std_20 = __intrinsic_rolling_std(close, 20);

// Bollinger Bands
let bb_upper = sma_20 + (2.0 * std_20);
let bb_lower = sma_20 - (2.0 * std_20);

// MACD
let macd_line = ema_12 - ema_26;
let macd_signal = __intrinsic_ema(macd_line, 9);
let macd_hist = macd_line - macd_signal;

// RSI components
let changes = __intrinsic_diff(close);

// Volume analysis
let vol_sma = __intrinsic_rolling_mean(volume, 20);

// Return data length for verification
close.length()
"#;

/// Medium complexity - fewer indicators
const MEDIUM_STRATEGY: &str = r#"
data("market-data", { symbol: "ES", start: "2024-01-01", end: "2024-03-31" });

let close = series("close");

// Just a few indicators
let sma_20 = __intrinsic_rolling_mean(close, 20);
let sma_50 = __intrinsic_rolling_mean(close, 50);
let ema_12 = __intrinsic_ema(close, 12);

close.length()
"#;

/// Simple - just data loading and one indicator
const SIMPLE_STRATEGY: &str = r#"
data("market-data", { symbol: "ES", start: "2024-01-01", end: "2024-01-31" });

let close = series("close");
let sma_20 = __intrinsic_rolling_mean(close, 20);

close.length()
"#;

fn run_benchmark(name: &str, code: &str, mode: ExecutionMode, iterations: u32) {
    let mut engine = ShapeEngine::new().expect("Failed to create engine");
    engine.set_execution_mode(mode);

    // Set database path
    std::env::set_var(
        "SHAPE_DB_PATH",
        "/home/dev/dev/finance/analysis-suite/market_data.duckdb",
    );

    // Warmup run
    let warmup_result = engine.execute(code);
    let rows = match &warmup_result {
        Ok(result) => {
            if let Some(val) = result.value() {
                format!("{:?}", val)
            } else {
                "N/A".to_string()
            }
        }
        Err(e) => format!("ERROR: {}", e),
    };

    // Benchmark runs
    let start = Instant::now();
    for _ in 0..iterations {
        let _ = engine.execute(code);
    }
    let elapsed = start.elapsed();

    let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
    let mode_str = match mode {
        ExecutionMode::Interpreter => "Interpreter",
        ExecutionMode::Vm => "VM",
        ExecutionMode::Jit => "JIT",
    };

    println!(
        "{} [{}]: {:.2}ms avg, rows={}",
        name, mode_str, avg_ms, rows
    );
}

fn main() {
    init();

    println!("\n╔══════════════════════════════════════════════════════════════════╗");
    println!("║     Shape REAL Strategy Benchmark (with market data)           ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    println!("--- Simple Strategy (1 month, ~27K rows, 1 indicator) ---");
    run_benchmark("Simple", SIMPLE_STRATEGY, ExecutionMode::Vm, 5);
    run_benchmark("Simple", SIMPLE_STRATEGY, ExecutionMode::Jit, 5);

    println!("\n--- Medium Strategy (3 months, ~65K rows, 3 indicators) ---");
    run_benchmark("Medium", MEDIUM_STRATEGY, ExecutionMode::Vm, 3);
    run_benchmark("Medium", MEDIUM_STRATEGY, ExecutionMode::Jit, 3);

    println!("\n--- Complex Strategy (6 months, ~175K rows, 10 indicators) ---");
    run_benchmark("Complex", COMPLEX_STRATEGY, ExecutionMode::Vm, 2);
    run_benchmark("Complex", COMPLEX_STRATEGY, ExecutionMode::Jit, 2);

    println!("\n--- Performance Analysis ---");
    println!("Real throughput = rows / time (including data loading + indicator calculation)");
}
