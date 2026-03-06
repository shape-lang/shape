//! Pattern matching performance benchmarks
//!
//! Measures the performance of pattern matching operations on market data

use chrono::{Duration, TimeZone, Utc};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use market_data::Timeframe;
use shape_core::ast::Item;
use shape_core::parser::parse_program;
use shape_core::runtime::{context::MarketData, Runtime};
use shape_core::value::RowValue;

/// Generate market data with specified number of rows
fn generate_market_data(symbol: &str, num_rows: usize) -> MarketData {
    let mut rows = Vec::new();
    let base_time = Utc.timestamp_opt(1609459200, 0).unwrap(); // 2021-01-01
    let mut price = 100.0;

    for i in 0..num_rows {
        // Generate realistic price movement
        let change = ((i as f64 * 0.1).sin() * 2.0) + (rand::random::<f64>() - 0.5);
        price = (price + change).max(1.0);

        let open = price;
        let high = price * (1.0 + rand::random::<f64>() * 0.02);
        let low = price * (1.0 - rand::random::<f64>() * 0.02);
        let close = price + (rand::random::<f64>() - 0.5) * 2.0;
        let volume = 100000.0 + rand::random::<f64>() * 50000.0;

        rows.push(RowValue::new(
            base_time + Duration::hours(i as i64),
            open,
            high,
            low,
            close,
            volume,
        ));

        price = close;
    }

    MarketData {
        symbol: symbol.to_string(),
        timeframe: Timeframe::h1(),
        rows,
    }
}

/// Simple pattern matching benchmark
fn benchmark_simple_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("pattern_matching_simple");

    // Create test programs with different patterns
    let simple_pattern = r#"
        pattern hammer {
            body = abs(close - open);
            range = high - low;
            body < range * 0.3
        }
        find hammer in last(100 rows)
    "#;

    let fuzzy_pattern = r#"
        pattern doji ~0.9 {
            body = abs(close - open);
            range = high - low;
            body ~< range * 0.1
        }
        find doji in last(100 rows)
    "#;

    let complex_pattern = r#"
        pattern bullish_engulfing {
            data[-1].close < data[-1].open and
            data[0].open < data[-1].close and
            data[0].close > data[-1].open and
            data[0].volume > data[-1].volume * 1.5
        }
        find bullish_engulfing in last(100 rows)
    "#;

    // Test with different data sizes
    for &num_rows in &[100, 1000, 10000] {
        let market_data = generate_market_data("TEST", num_rows);
        let mut runtime = Runtime::new();

        // Benchmark simple pattern
        let program = parse_program(simple_pattern).unwrap();
        runtime.load_program(&program, &market_data).unwrap();

        group.bench_with_input(
            BenchmarkId::new("simple_pattern", num_rows),
            &(&program, &market_data),
            |b, (program, data)| {
                b.iter(|| {
                    let mut runtime = Runtime::new();
                    runtime.load_program(program, data).unwrap();
                    if let Some(query_item) = program.items.last() {
                        runtime.execute_query(query_item, data).unwrap();
                    }
                });
            },
        );

        // Benchmark fuzzy pattern
        let program = parse_program(fuzzy_pattern).unwrap();

        group.bench_with_input(
            BenchmarkId::new("fuzzy_pattern", num_rows),
            &(&program, &market_data),
            |b, (program, data)| {
                b.iter(|| {
                    let mut runtime = Runtime::new();
                    runtime.load_program(program, data).unwrap();
                    if let Some(query_item) = program.items.last() {
                        runtime.execute_query(query_item, data).unwrap();
                    }
                });
            },
        );

        // Benchmark complex pattern
        let program = parse_program(complex_pattern).unwrap();

        group.bench_with_input(
            BenchmarkId::new("complex_pattern", num_rows),
            &(&program, &market_data),
            |b, (program, data)| {
                b.iter(|| {
                    let mut runtime = Runtime::new();
                    runtime.load_program(program, data).unwrap();
                    if let Some(query_item) = program.items.last() {
                        runtime.execute_query(query_item, data).unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Multi-pattern matching benchmark
fn benchmark_multi_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("pattern_matching_multi");

    // Program with multiple patterns
    let multi_pattern_program = r#"
        pattern hammer {
            body = abs(close - open);
            range = high - low;
            lower_shadow = min(open, close) - low;
            
            body < range * 0.3 and
            lower_shadow > body * 2
        }
        
        pattern shooting_star {
            body = abs(close - open);
            range = high - low;
            upper_shadow = high - max(open, close);
            
            body < range * 0.3 and
            upper_shadow > body * 2
        }
        
        pattern doji {
            abs(close - open) < (high - low) * 0.1
        }
        
        pattern marubozu {
            body = abs(close - open);
            range = high - low;
            body > range * 0.95
        }
        
        pattern spinning_top {
            body = abs(close - open);
            range = high - low;
            upper_shadow = high - max(open, close);
            lower_shadow = min(open, close) - low;
            
            body < range * 0.4 and
            upper_shadow > body * 0.5 and
            lower_shadow > body * 0.5
        }
    "#;

    // Test finding each pattern
    let patterns = [
        "hammer",
        "shooting_star",
        "doji",
        "marubozu",
        "spinning_top",
    ];

    for &num_rows in &[1000, 5000, 10000] {
        let market_data = generate_market_data("TEST", num_rows);

        for pattern_name in &patterns {
            let full_program = format!(
                "{}\nfind {} in last({} rows)",
                multi_pattern_program,
                pattern_name,
                num_rows.min(1000)
            );

            let program = parse_program(&full_program).unwrap();

            group.bench_with_input(
                BenchmarkId::new(format!("find_{}", pattern_name), num_rows),
                &(&program, &market_data),
                |b, (program, data)| {
                    b.iter(|| {
                        let mut runtime = Runtime::new();
                        runtime.load_program(program, data).unwrap();
                        if let Some(Item::Query(query)) = program.items.last() {
                            runtime
                                .execute_query(&Item::Query(query.clone()), data)
                                .unwrap();
                        }
                    });
                },
            );
        }
    }

    group.finish();
}

/// Benchmark pattern matching with conditions
fn benchmark_conditional_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("pattern_matching_conditional");

    let conditional_pattern = r#"
        pattern high_volume_hammer {
            body = abs(close - open);
            range = high - low;
            lower_shadow = min(open, close) - low;

            body < range * 0.3 and
            lower_shadow > body * 2
        }

        find high_volume_hammer in last(500 rows) where {
            volume > sma_volume(20) * 1.5 and
            close > sma(50) and
            rsi(14) < 30
        }
    "#;

    for &num_rows in &[1000, 5000, 10000] {
        let market_data = generate_market_data("TEST", num_rows);
        let program = parse_program(conditional_pattern).unwrap();

        group.bench_with_input(
            BenchmarkId::new("conditional_pattern", num_rows),
            &(&program, &market_data),
            |b, (program, data)| {
                b.iter(|| {
                    let mut runtime = Runtime::new();
                    runtime.load_program(program, data).unwrap();
                    if let Some(query_item) = program.items.last() {
                        runtime.execute_query(query_item, data).unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark fuzzy matching performance
fn benchmark_fuzzy_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("fuzzy_matching");

    // Test different fuzzy tolerance levels
    let tolerances = [0.01, 0.02, 0.05, 0.10];

    for tolerance in &tolerances {
        let fuzzy_program = format!(
            r#"
            pattern fuzzy_reversal ~{} {{
                // Fuzzy conditions
                data[0].close ~> data[-1].close * 1.02 and
                data[0].volume ~> sma_volume(10) * 1.3 and
                data[0].high ~= data[-1].high and
                rsi(14) ~< 30
            }}

            find fuzzy_reversal in last(200 rows)
            "#,
            tolerance
        );

        let market_data = generate_market_data("TEST", 5000);
        let program = parse_program(&fuzzy_program).unwrap();

        group.bench_with_input(
            BenchmarkId::new("fuzzy_tolerance", format!("{:.2}", tolerance)),
            &(&program, &market_data),
            |b, (program, data)| {
                b.iter(|| {
                    let mut runtime = Runtime::new();
                    runtime.load_program(program, data).unwrap();
                    if let Some(query_item) = program.items.last() {
                        runtime.execute_query(query_item, data).unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark pattern scanning across multiple symbols
fn benchmark_pattern_scanning(c: &mut Criterion) {
    let mut group = c.benchmark_group("pattern_scanning");

    let scan_program = r#"
        pattern breakout {
            data[0].close > highest(high, 20) and
            data[0].volume > sma_volume(20) * 2
        }

        scan ["AAPL", "GOOGL", "MSFT", "AMZN", "TSLA", "META", "NVDA", "JPM", "V", "JNJ"] for breakout
    "#;

    // Generate data for multiple symbols
    let _symbols = [
        "AAPL", "GOOGL", "MSFT", "AMZN", "TSLA", "META", "NVDA", "JPM", "V", "JNJ",
    ];

    for &num_rows in &[100, 1000, 5000] {
        let program = parse_program(scan_program).unwrap();

        // Note: In real implementation, scan would load data for each symbol
        // For benchmarking, we simulate with one dataset
        let market_data = generate_market_data("TEST", num_rows);

        group.bench_with_input(
            BenchmarkId::new("scan_symbols", num_rows),
            &(&program, &market_data),
            |b, (program, data)| {
                b.iter(|| {
                    let mut runtime = Runtime::new();
                    runtime.load_program(program, data).unwrap();
                    if let Some(query_item) = program.items.last() {
                        runtime.execute_query(query_item, data).unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

// Helper to generate random values for fuzzy matching tests
mod rand {
    pub fn random<T>() -> T
    where
        T: From<f64>,
    {
        T::from(0.5) // Simple deterministic "random" for benchmarks
    }
}

criterion_group!(
    benches,
    benchmark_simple_patterns,
    benchmark_multi_patterns,
    benchmark_conditional_patterns,
    benchmark_fuzzy_matching,
    benchmark_pattern_scanning
);
criterion_main!(benches);
