//! Parser performance benchmarks
//!
//! Measures the performance of parsing various Shape programs

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use shape_core::parser::parse_program;

/// Small Shape program with basic patterns
const SMALL_PROGRAM: &str = r#"
pattern hammer {
    body = abs(close - open);
    range = high - low;
    
    body < range * 0.3 and
    close > open and
    low < min(open, close) - range * 0.6
}

find hammer in last(100 rows)
"#;

/// Medium Shape program with multiple patterns and queries
const MEDIUM_PROGRAM: &str = r#"
// Import indicators
from indicators use { sma, ema, rsi }

// Define patterns
pattern doji {
    body = abs(close - open);
    range = high - low;
    body < range * 0.1
}

pattern bullish_engulfing {
    data[-1].close < data[-1].open and
    data[0].open < data[-1].close and
    data[0].close > data[-1].open
}

pattern morning_star {
    data[-2] is long_black and
    data[-1] is small_body and
    data[-1].low < data[-2].low and
    data[0] is long_white and
    data[0].close > data[-2].open * 0.5
}

// Functions
function is_trending_up(period = 20) {
    let ma_now = sma(period);
    let ma_prev = sma(period, 5);
    return ma_now > ma_prev * 1.02;
}

// Queries
find doji in last(50 rows) where volume > avg_volume(20) * 1.5
scan ["AAPL", "GOOGL", "MSFT"] for bullish_engulfing
"#;

/// Large Shape program with complete strategy
const LARGE_PROGRAM: &str = r#"
// Comprehensive trading strategy with patterns, indicators, and risk management

from indicators use { sma, ema, rsi, macd, bollinger_bands }
from volatility use { atr, adx }
from risk use { kelly_criterion, position_size }

// Pattern definitions
pattern hammer {
    body = abs(close - open);
    range = high - low;
    lower_shadow = min(open, close) - low;
    
    body < range * 0.3 and
    close > open and
    lower_shadow > body * 2
}

pattern shooting_star {
    body = abs(close - open);
    range = high - low;
    upper_shadow = high - max(open, close);
    
    body < range * 0.3 and
    upper_shadow > body * 2 and
    close < open
}

pattern doji {
    body = abs(close - open);
    range = high - low;
    body < range * 0.1
}

pattern bullish_engulfing {
    data[-1].close < data[-1].open and
    data[0].open < data[-1].close and
    data[0].close > data[-1].open
}

pattern bearish_engulfing {
    data[-1].close > data[-1].open and
    data[0].open > data[-1].close and
    data[0].close < data[-1].open
}

// Technical indicator functions
function trend_strength() {
    let adx_value = adx(14);
    let ma_short = sma(10);
    let ma_long = sma(50);
    
    if adx_value > 25 and ma_short > ma_long {
        return "strong_uptrend";
    } else if adx_value > 25 and ma_short < ma_long {
        return "strong_downtrend";
    } else {
        return "sideways";
    }
}

function momentum_signal() {
    let rsi_value = rsi(14);
    let macd_data = macd(12, 26, 9);
    
    return {
        rsi: rsi_value,
        macd_signal: macd_data.macd > macd_data.signal,
        momentum: (rsi_value > 50 and macd_data.macd > 0) ? "bullish" : "bearish"
    };
}

// Risk management
function calculate_position_size(stop_loss_pct, account_risk = 0.02) {
    let account_balance = get_account_balance();
    let risk_amount = account_balance * account_risk;
    let position_size = risk_amount / stop_loss_pct;
    
    // Apply Kelly Criterion
    let kelly_pct = kelly_criterion(win_rate(), avg_win(), avg_loss());
    position_size = min(position_size, account_balance * kelly_pct);
    
    return position_size;
}

// Main strategy
strategy TrendFollowingStrategy {
    parameters {
        fast_ma = 10;
        slow_ma = 50;
        rsi_period = 14;
        position_risk = 0.02;
        max_positions = 5;
    }
    
    state {
        var positions = [];
        var performance = {
            wins: 0,
            losses: 0,
            total_pnl: 0
        };
    }
    
    on_start() {
        print("Starting Trend Following Strategy");
        print("Initial capital: " + get_account_balance());
    }
    
    on_bar(row) {
        // Update indicators
        let fast_sma = sma(fast_ma);
        let slow_sma = sma(slow_ma);
        let rsi_val = rsi(rsi_period);
        let bb = bollinger_bands(20, 2);
        let atr_val = atr(14);

        // Check for entry signals
        if positions.length < max_positions {
            // Bullish entry conditions
            if fast_sma > slow_sma and
               rsi_val > 30 and rsi_val < 70 and
               row.close > bb.middle and
               (row matches hammer or row matches bullish_engulfing) {

                let stop_loss = row.close - (2 * atr_val);
                let take_profit = row.close + (3 * atr_val);
                let size = calculate_position_size((row.close - stop_loss) / row.close);

                open_position("long", size, {
                    stop_loss: stop_loss,
                    take_profit: take_profit,
                    entry_reason: "trend_following_bullish"
                });

                positions.push({
                    side: "long",
                    entry_price: row.close,
                    size: size,
                    stop_loss: stop_loss,
                    take_profit: take_profit
                });
            }
        }

        // Check exit conditions for existing positions
        for position in positions {
            if position.side == "long" {
                // Exit on bearish reversal
                if fast_sma < slow_sma or
                   rsi_val > 80 or
                   (row matches shooting_star or row matches bearish_engulfing) {

                    close_position("long", position.size);

                    // Update performance
                    let pnl = (row.close - position.entry_price) * position.size;
                    performance.total_pnl += pnl;
                    if pnl > 0 {
                        performance.wins += 1;
                    } else {
                        performance.losses += 1;
                    }
                }
            }
        }

        // Risk management checks
        let current_drawdown = calculate_drawdown();
        if current_drawdown > 0.15 {
            // Close all positions if drawdown exceeds 15%
            close_all_positions();
            positions = [];
        }
    }
    
    on_end() {
        print("Strategy completed");
        print("Total trades: " + (performance.wins + performance.losses));
        print("Win rate: " + (performance.wins / (performance.wins + performance.losses) * 100) + "%");
        print("Total P&L: " + performance.total_pnl);
    }
}

// Portfolio management
portfolio QuantPortfolio {
    initial_capital: 100000;
    
    allocation {
        strategy TrendFollowingStrategy: 40%;
        strategy MeanReversionStrategy: 30%;
        strategy ArbitrageStrategy: 30%;
    }
    
    risk_limits {
        max_drawdown: 20%;
        max_leverage: 2.0;
        position_limits {
            max_positions: 20;
            max_position_size: 10%;
        }
    }
    
    rebalancing {
        frequency: monthly;
        threshold: 5%;
        method: volatility_weighted;
    }
}

// Execute backtest
backtest QuantPortfolio on ["AAPL", "GOOGL", "MSFT", "AMZN", "TSLA"] {
    period: @"2020-01-01" to @"2023-12-31",
    initial_capital: 100000,
    commission: 0.001,
    slippage_model: "linear"
}
"#;

/// Extra large program for stress testing
fn generate_extra_large_program(num_patterns: usize) -> String {
    let mut program = String::from("// Auto-generated large program\n\n");

    // Generate patterns
    for i in 0..num_patterns {
        program.push_str(&format!(
            r#"
pattern pattern_{} {{
    condition_{} = data[0].close > data[-1].close * 1.{:02};
    volume_check = data[0].volume > avg_volume(20) * 1.5;

    condition_{} and volume_check
}}
"#,
            i,
            i,
            i % 100,
            i
        ));
    }

    // Generate queries
    for i in 0..num_patterns.min(10) {
        program.push_str(&format!("find pattern_{} in last(100 rows)\n", i));
    }

    program
}

fn benchmark_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");

    // Benchmark small program
    group.bench_function("small_program", |b| {
        b.iter(|| parse_program(black_box(SMALL_PROGRAM)));
    });

    // Benchmark medium program
    group.bench_function("medium_program", |b| {
        b.iter(|| parse_program(black_box(MEDIUM_PROGRAM)));
    });

    // Benchmark large program
    group.bench_function("large_program", |b| {
        b.iter(|| parse_program(black_box(LARGE_PROGRAM)));
    });

    // Benchmark scaling with program size
    for size in [10, 50, 100, 200] {
        let program = generate_extra_large_program(size);
        group.bench_with_input(BenchmarkId::new("scaling", size), &program, |b, program| {
            b.iter(|| parse_program(black_box(program)));
        });
    }

    group.finish();
}

fn benchmark_individual_constructs(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_constructs");

    // Benchmark pattern parsing
    let pattern_code = r#"
    pattern complex_pattern {
        body = abs(close - open);
        range = high - low;
        upper_shadow = high - max(open, close);
        lower_shadow = min(open, close) - low;
        
        body < range * 0.3 and
        upper_shadow > body * 2 and
        lower_shadow < body * 0.5 and
        volume > avg_volume(20) * 1.5
    }
    "#;

    group.bench_function("pattern_definition", |b| {
        b.iter(|| parse_program(black_box(pattern_code)));
    });

    // Benchmark function parsing
    let function_code = r#"
    function complex_calculation(period = 20, multiplier = 2.0) {
        let sma_val = sma(period);
        let ema_val = ema(period);
        let bb = bollinger_bands(period, multiplier);

        if data[0].close > bb.upper {
            return "overbought";
        } else if data[0].close < bb.lower {
            return "oversold";
        } else {
            return "neutral";
        }
    }
    "#;

    group.bench_function("function_definition", |b| {
        b.iter(|| parse_program(black_box(function_code)));
    });

    // Benchmark strategy parsing
    let strategy_code = r#"
    strategy BenchmarkStrategy {
        parameters {
            period = 20;
            threshold = 0.02;
        }

        state {
            var position_open = false;
            var entry_price = 0;
        }

        on_bar(row) {
            let signal = calculate_signal(period);

            if signal > threshold and !position_open {
                open_position("long", 0.1);
                position_open = true;
                entry_price = row.close;
            } else if signal < -threshold and position_open {
                close_position("long");
                position_open = false;
            }
        }
    }
    "#;

    group.bench_function("strategy_definition", |b| {
        b.iter(|| parse_program(black_box(strategy_code)));
    });

    group.finish();
}

criterion_group!(benches, benchmark_parser, benchmark_individual_constructs);
criterion_main!(benches);
