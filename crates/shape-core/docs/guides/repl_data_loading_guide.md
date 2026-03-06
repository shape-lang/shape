# REPL Data Loading Guide

This guide explains how to load market data into the Shape REPL for analysis.

## Loading ES Futures Data with Contract Rollover

The REPL now supports loading futures data with automatic contract rollover handling. This is essential for analyzing continuous price series across multiple contract expirations.

### Basic Usage

```bash
# Start the REPL
cargo run --bin shape -- repl

# Load ES futures data from your data directory
:data ~/dev/finance/data ES

# Load with specific date range
:data ~/dev/finance/data ES 2020-01-01 2022-12-31
```

### Command Syntax

```
:data <path> [symbol] [start_date] [end_date]
```

- `path`: Directory containing futures data (CME/Databento format)
- `symbol`: Base symbol (e.g., ES, CL, GC) - optional, inferred from directory
- `start_date`: Start date in YYYY-MM-DD format (optional)
- `end_date`: End date in YYYY-MM-DD format (optional)

### Data Directory Structure

The loader expects CME/Databento folder structure:
```
~/dev/finance/data/
└── ES/
    └── 2024/
        └── 01/
            └── glbx-mdp3-20240101.ohlcv-1m.csv
```

### Example: Finding Aggressive Price Movements

Once data is loaded, you can run queries like:

```shape
// Define ATR-based aggressive move pattern
pattern aggressive_move {
    let atr_14 = atr(14)
    let price_change = abs(candle[0].close - candle[1].close)
    price_change > atr_14 * 0.2
}

// Find all occurrences
find aggressive_move in last(1000 candles)
```

### Working with Different Timeframes

The data is loaded at 1-minute resolution by default. You can aggregate to higher timeframes:

```shape
// Analyze on 15-minute timeframe
find aggressive_move on(15m) in all
```

### Probability Analysis

To calculate probabilities of subsequent aggressive moves:

```shape
// Count pattern occurrences
let total_aggressive = count(find aggressive_move in all)

// Count when aggressive move follows another
let consecutive_aggressive = count(
    find aggressive_move 
    where candle[-15].matches(aggressive_move)
    in all
)

// Calculate conditional probability
let probability = consecutive_aggressive / total_aggressive
```

### Performance Tips

1. **Date Ranges**: Always specify date ranges to limit data size
2. **Caching**: The market-data crate caches loaded data for faster subsequent access
3. **Memory**: Large date ranges may consume significant memory

### Troubleshooting

- **"No files found"**: Check that the path exists and contains data in the expected format
- **"No futures contracts found"**: Ensure CSV files contain proper futures symbols (e.g., ESH4, ESM4)
- **Memory errors**: Reduce the date range or close other applications

## Next Steps

After loading data, you can:
- Define custom patterns for technical analysis
- Run backtests on trading strategies
- Calculate statistics and probabilities
- Export results for further analysis