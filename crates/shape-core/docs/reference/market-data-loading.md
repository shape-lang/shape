# Market Data Loading Guide

This guide explains how to load market data into Shape for analysis and backtesting.

## Loading Data in REPL

The REPL provides the `:data` command for loading futures data with automatic contract rollover:

```bash
:data <path> [symbol] [start_date] [end_date]
```

### Basic Usage

Load ES (E-mini S&P 500) futures data:
```
shape> :data ~/dev/finance/data ES
```

### With Date Range

Load specific date range:
```
shape> :data ~/dev/finance/data ES 2020-01-01 2022-12-31
```

### Directory Structure

The data loader expects futures contract files in the following structure:
```
data/
├── ES/
│   ├── ESH20.csv    # March 2020 contract
│   ├── ESM20.csv    # June 2020 contract
│   ├── ESU20.csv    # September 2020 contract
│   └── ESZ20.csv    # December 2020 contract
└── CL/
    ├── CLF20.csv    # January 2020 contract
    ├── CLG20.csv    # February 2020 contract
    └── ...
```

## Using Market Data in Scripts

Once loaded, market data is available to all Shape expressions:

```cql
// Access current candle
let current_close = candle[0].close;
let current_volume = candle[0].volume;

// Access historical candles
let prev_close = candle[1].close;  // Previous candle
let old_high = candle[10].high;    // 10 candles ago

// Work with candle properties
let body_size = candle[0].body;
let upper_wick = candle[0].upper_wick;
let lower_wick = candle[0].lower_wick;
```

## Running Scripts with Data

When running Shape scripts from the command line:

```bash
# Run script with market data file
cargo run --bin shape -- run analysis.shape --data market_data.json

# Execute query directly
cargo run --bin shape -- query "find hammer last(100 candles)" --data es_data.json
```

## Lazy Loading

Shape implements lazy loading - market data is only loaded when actually accessed:

```cql
// This doesn't require market data
let x = 2 + 2;
print(x);  // Works without data

// This requires market data
let close = candle[0].close;  // Will error if no data loaded
```

## Common Patterns

### Pattern Finding
```cql
// Find patterns in recent data
data("market_data", {symbol: "ES"}).window(last(100, "candles")).find("hammer")
data("market_data", {symbol: "ES"}).find("doji").filter(candle[0].volume > 1000000)

// Scan multiple patterns
data("market_data", {symbol: "ES"}).window(last(500, "candles")).find("morning_star")
```

### Indicator Calculation
```cql
// Calculate indicators
let ma20 = sma(20);
let rsi = rsi(14);

// Use in conditions
if candle[0].close > ma20 {
    print("Price above 20-day MA");
}
```

### Time-based Queries
```cql
// Query specific time ranges
data("market_data", {symbol: "ES"}).window(between("2022-01-01", "2022-12-31")).find("hammer")

// Use relative time
data("market_data", {symbol: "ES"}).window(last(30, "days")).find("doji")
```

## Market Data Format

Shape expects market data with the following fields:
- `timestamp`: Unix timestamp
- `open`: Opening price
- `high`: High price
- `low`: Low price
- `close`: Closing price
- `volume`: Trading volume

The market-data crate handles various formats including CSV, JSON, and binary formats.

## Continuous Contracts

When loading futures data, Shape automatically handles contract rollover to create a continuous price series:

```
shape> :data ~/dev/finance/data ES 2020-01-01 2022-12-31
Success: Loaded 126720 candles for symbol: ES
Date range: 2020-01-01 to 2022-12-31
```

The system automatically:
- Detects contract expiration dates
- Handles price adjustments at rollover
- Creates seamless continuous data

## Error Handling

Common errors and solutions:

```
Error: Path does not exist: /path/to/data
→ Check the path exists and contains market data files

Error: Queries require market data. Use :data <file> to load data
→ Load data first using :data command

Error: No data available for symbol XYZ
→ Ensure data files follow naming convention (e.g., ESH20.csv)
```

## Best Practices

1. **Load appropriate timeframes**: Load only the data you need to keep memory usage low
2. **Use relative paths**: Use `~` for home directory to make scripts portable
3. **Check data quality**: Verify loaded data with simple queries before complex analysis
4. **Cache data**: The system caches loaded data for 15 minutes for better performance

## Example Session

```
$ cargo run --bin shape -- repl
Shape REPL v0.1.0
Type :help for help, :quit to exit

shape> :data ~/dev/finance/data ES 2022-01-01 2022-12-31
Success: Loaded 63360 candles for symbol: ES
Date range: 2022-01-01 to 2022-12-31

shape> let ma = sma(20)
shape> data("market_data", {symbol: "ES"}).find("hammer").filter(candle[0].close > ma)
3 match(es) found:
  1. hammer at 2022-03-15 14:30:00 (confidence: 95.50%)
  2. hammer at 2022-06-21 10:15:00 (confidence: 92.30%)
  3. hammer at 2022-10-13 15:45:00 (confidence: 89.70%)

shape> :quit
Goodbye!
```