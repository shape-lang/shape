# Shape CLI Usage Guide

## Overview

The Shape CLI supports both interactive (REPL) and non-interactive (script) modes, making it suitable for both exploratory analysis and automated workflows.

## Running Shape

### Interactive REPL Mode

Start the REPL for interactive exploration:

```bash
# Start REPL without pre-loaded data
cargo run -p shape --bin shape -- repl

# Start REPL with pre-loaded JSON data
cargo run -p shape --bin shape -- repl --data market_data.json
```

### Non-Interactive Script Mode

Execute scripts from files or via pipes:

```bash
# Execute a script file
cargo run -p shape --bin shape -- script < script.shape

# Execute with verbose output (shows commands being executed)
cargo run -p shape --bin shape -- script --verbose < script.shape

# Pipe commands directly
echo ":data ~/dev/finance/data ES 2020-01-01 2022-12-31" | cargo run -p shape --bin shape -- script

# Execute multiple commands
cat <<EOF | cargo run -p shape --bin shape -- script
:data ~/dev/finance/data ES 2020-01-01 2022-12-31
count(all candles)
candle[0].close
EOF
```

### Query Mode

Execute a single query on JSON data:

```bash
# Execute a query on market data
cargo run -p shape --bin shape -- query "data(\"market_data\", {symbol: \"ES\"}).window(last(100, \"candles\")).find(\"hammer\")" --data market.json

# Include statistics
cargo run -p shape --bin shape -- query "find hammer" --data market.json --stats

# Different output formats
cargo run -p shape --bin shape -- query "find doji" --data market.json --format json
```

### Run Mode

Execute a Shape program file:

```bash
# Run a program file
cargo run -p shape --bin shape -- run program.shape --data market.json

# With statistics and custom format
cargo run -p shape --bin shape -- run strategy.shape --data market.json --stats --format table
```

## Loading ES Futures Data

The `:data` command supports loading futures data with automatic contract rollover:

```shape
# Load ES futures data for a date range
:data ~/dev/finance/data ES 2020-01-01 2022-12-31

# The path is automatically expanded (~ becomes $HOME)
# Data is loaded with contract rollover handled automatically
```

## Example Script

Create a file `analysis.shape`:

```shape
# Load data
:data ~/dev/finance/data ES 2020-01-01 2022-12-31

# Basic analysis
count(all candles)
let sma20 = sma(20)
let sma50 = sma(50)

# Check for golden cross
sma20 > sma50

# Find patterns
data("market_data", {symbol: "ES"}).window(last(100, "candles")).find("hammer")
data("market_data", {symbol: "ES"}).find("doji").filter(candle[0].volume > 1000000)
```

Run it:

```bash
cargo run -p shape --bin shape -- script < analysis.shape
```

## ATR-Based Analysis Example

```shape
# Load data
:data ~/dev/finance/data ES 2020-01-01 2022-12-31

# Calculate ATR
let atr_value = atr(14)

# Find aggressive moves (>20% of ATR)
let price_change = abs(candle[0].close - candle[1].close)
let is_aggressive = price_change > atr_value * 0.2

# Show results
atr_value
price_change
is_aggressive
```

## Common Use Cases

### 1. Data Exploration
```bash
# Interactive exploration
cargo run -p shape --bin shape -- repl

# In REPL:
:data ~/dev/finance/data ES 2020-01-01 2020-12-31
count(all candles)
candle[0]
:functions
:patterns
```

### 2. Automated Analysis
```bash
# Create a daily analysis script
cat <<EOF > daily_analysis.shape
:data ~/dev/finance/data ES 2023-01-01 2023-12-31
let volatility = atr(14)
let trend = sma(20) > sma(50)
volatility
trend
data("market_data", {symbol: "ES"}).window(last(5, "days")).find("hammer")
EOF

# Run it
cargo run -p shape --bin shape -- script < daily_analysis.shape
```

### 3. Backtesting Preparation
```bash
# Script for finding high-volatility periods
echo ':data ~/dev/finance/data ES 2020-01-01 2022-12-31
let high_vol_threshold = atr(14) * 2
data("market_data", {symbol: "ES"}).filter(abs(candle[0].close - candle[0].open) > high_vol_threshold)' | \
cargo run -p shape --bin shape -- script
```

## Tips

1. **Path Expansion**: The `~` in paths is automatically expanded to your home directory
2. **Multi-line Input**: In scripts, statements can span multiple lines
3. **Comments**: Use `#` for comments in scripts
4. **Error Handling**: Errors are printed to stderr, making it easy to separate from output
5. **Performance**: For large date ranges, consider breaking analysis into smaller chunks

## Troubleshooting

### "Path does not exist" Error
- Ensure the path is correct and accessible
- The `~` expansion is now supported

### REPL Exits Immediately
- Use the `script` subcommand for non-interactive execution
- The REPL requires an interactive terminal (TTY)

### Pattern Not Recognized
- Basic patterns are being migrated to the new syntax
- Use simple expressions for now: `candle[0].close > candle[1].close`

### No Data Loaded
- Check the date range matches available data
- Verify the symbol (e.g., "ES") is correct
- Ensure the data directory follows CME/Databento structure