# Shape - Chart Pattern Query Language

Shape is a domain-specific language (DSL) designed for querying and analyzing chart patterns in financial market data. It provides an intuitive, SQL-like syntax for finding candlestick patterns, analyzing their performance, and generating trading insights.

## Features

- **Pattern Recognition**: Built-in support for common candlestick patterns (hammer, doji, engulfing, etc.)
- **Custom Patterns**: Define your own patterns using simple, readable syntax
- **Fuzzy Matching**: Handle real-world market noise with configurable tolerance
- **Time-based Queries**: Search patterns within specific time windows
- **Statistical Analysis**: Comprehensive performance metrics and pattern statistics
- **Multi-timeframe Support**: Analyze patterns across different timeframes
- **LLM Integration**: MCP server for AI-powered pattern analysis

## Quick Start

### Installation

```bash
# Clone the repository
git clone https://github.com/your-org/shape.git
cd shape

# Build the project
cargo build --release

# Install the CLI tool
cargo install --path .
```

### Basic Usage

```bash
# Execute a simple query
shape query "find hammer" --data market_data.csv

# Start interactive REPL
shape repl --data market_data.csv

# Validate a query
shape validate "find doji where candle[0].volume > 1000000"

# Show examples
shape examples
```

## Query Language Syntax

### Basic Pattern Search

```shape
# Find all hammer patterns
find hammer

# Find patterns with conditions
find doji where candle[0].volume > 1000000

# Time-constrained search
data("market_data", {symbol: "ES"}).window(last(5, "days")).find("hammer")

# Search between specific dates
data("market_data", {symbol: "ES"}).window(between("2024-01-01", "2024-01-31")).find("hammer")
```

### Custom Pattern Definition

```shape
# Define a bullish engulfing pattern
pattern bullish_engulfing {
    candle[-1].close < candle[-1].open and   # Previous candle is bearish
    candle[0].close > candle[0].open and     # Current candle is bullish
    candle[0].open <= candle[-1].close and   # Opens at or below previous close
    candle[0].close > candle[-1].open        # Closes above previous open
}

# Use the pattern
find bullish_engulfing
```

### Fuzzy Matching

```shape
# Find doji with 5% tolerance
find doji ~0.05

# Custom pattern with fuzzy matching
pattern fuzzy_hammer ~0.02 {
    candle[0].close ~= candle[0].open and
    (candle[0].close - candle[0].low) > 2 * abs(candle[0].open - candle[0].close)
}
```

### Complex Queries

```shape
# Combine multiple conditions
find hammer where
    candle[0].volume > sma(volume, 20) * 2 and
    rsi(14) < 30 and
    candle[0].close > candle[-1].high

# Multi-symbol scan
data("market_data", {symbols: ["AAPL", "MSFT", "GOOGL"]}).map(s => s.find("hammer"))

# Pattern analysis
analyze hammer with [success_rate, avg_gain, best_timeframe]

# Backtesting
backtest "hammer_strategy" last(1 year) with
    entry = "hammer",
    exit = "close > entry_price * 1.02 or close < entry_price * 0.98",
    position_size = 0.1
```

## API Usage

### Rust API

```rust
use shape::query_executor::QueryExecutor;
use shape::statistics::StatisticsCalculator;
use market_data::MarketData;

// Create executor
let mut executor = QueryExecutor::new();

// Execute query
let result = executor.execute(
    "find hammer where candle[0].volume > 1000000",
    &market_data
)?;

// Get statistics
let stats_calc = StatisticsCalculator::new();
let stats = stats_calc.generate_report(&result)?;

// Print results
println!("Found {} patterns", result.matches.len());
println!("Win rate: {:.1}%", stats.basic.success_rate * 100.0);
```

## Pattern Library

### Built-in Patterns

| Pattern | Description | Reliability |
|---------|-------------|-------------|
| `hammer` | Bullish reversal pattern with long lower shadow | High |
| `doji` | Indecision pattern with equal open/close | Medium |
| `shooting_star` | Bearish reversal with long upper shadow | High |
| `engulfing` | Strong reversal pattern | High |
| `harami` | Trend reversal pattern | Medium |
| `morning_star` | Three-candle bullish reversal | High |
| `evening_star` | Three-candle bearish reversal | High |

### Indicators

| Indicator | Usage | Parameters |
|-----------|-------|------------|
| `sma(price, period)` | Simple Moving Average | price field, period |
| `ema(price, period)` | Exponential Moving Average | price field, period |
| `rsi(period)` | Relative Strength Index | period (default: 14) |
| `macd()` | MACD indicator | none |
| `bb_upper(period, std)` | Bollinger Band Upper | period, std deviations |
| `bb_lower(period, std)` | Bollinger Band Lower | period, std deviations |

## Advanced Features

### Time Navigation

```shape
# Relative time references
@today, @yesterday, @now

# Navigate backwards
back(5 days)
back(100 candles)

# Time windows
last(1 week)
last(500 candles)
session("09:30", "16:00")  # Market hours only
```

### Multi-timeframe Analysis

```shape
# Check pattern on different timeframe
on(1h) {
    find hammer
} and on(15m) {
    rsi(14) < 30
}
```

### Pattern Composition

```shape
# Combine patterns
pattern strong_reversal {
    (hammer or bullish_engulfing) and
    candle[0].volume > sma(volume, 50) * 3 and
    on(1d) { trend = bearish }
}
```

## Performance Considerations

- **Data Loading**: Pre-load market data for better performance
- **Pattern Complexity**: Simpler patterns execute faster
- **Time Windows**: Smaller windows improve query speed
- **Caching**: Results are cached for 15 minutes by default

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Shape is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Support

- Documentation: [https://shape.dev/docs](https://shape.dev/docs)
- Issues: [GitHub Issues](https://github.com/your-org/shape/issues)
- Discord: [Join our community](https://discord.gg/shape)

## Roadmap

- [ ] Real-time pattern detection
- [ ] Machine learning integration
- [ ] More built-in patterns
- [ ] Visual pattern editor
- [ ] Cloud-based execution
