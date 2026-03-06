# Shape REPL Demo

## Starting the REPL

```bash
# Start the REPL without pre-loaded data
cargo run --bin shape -- repl

# Or with initial data (JSON format required)
cargo run --bin shape -- repl --data sample_data.json
```

## Loading ES Futures Data

Once in the REPL, use the `:data` command to load futures data:

```
shape> :data ~/dev/finance/data ES 2020-04-26 2020-04-30
```

This will:
- Load ES futures data from the specified directory
- Handle contract rollover automatically
- Build a continuous contract
- Load data for the specified date range

## Running Queries

After loading data, you can run queries:

```shape
// Check how many candles were loaded
count(all candles)

// Find basic patterns
find hammer in last(100 candles)

// Access candle data
candle[0].close

// Calculate simple indicators
sma(20)
```

## ATR-Based Analysis

```shape
// Note: The ATR indicator needs to be available in the runtime
// This is a conceptual example

// Check if a candle moved more than 20% of ATR
let atr_value = atr(14)
let price_change = abs(candle[0].close - candle[1].close)
price_change > atr_value * 0.2
```

## REPL Commands

- `:help` - Show available commands
- `:data <path> [symbol] [start] [end]` - Load futures data
- `:load <file.shape>` - Load and execute a Shape file
- `:history` - Show command history
- `:patterns` - List available patterns
- `:functions` - List available functions
- `:quit` - Exit the REPL

## Notes

1. The REPL requires interactive terminal input (TTY)
2. Data is loaded into memory, so large date ranges may consume significant RAM
3. The market-data crate handles futures contract rollover automatically
4. All timestamps are in UTC