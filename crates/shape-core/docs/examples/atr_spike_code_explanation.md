# ATR Spike Reversal Analysis - Code Walkthrough

This example demonstrates Shape's unified execution architecture where the same logic framework serves both statistical analysis and backtesting.

## Core Components

### 1. ATR Spike Detection

```shape
@export
function is_atr_spike(threshold_percent: number = 20) {
    let atr_value = atr(14)  // Uses @warmup(15) from stdlib
    if atr_value == null {
        return false
    }
    
    let price_change = candle[0].high - candle[0].low
    let threshold = atr_value * (threshold_percent / 100)
    
    return price_change >= threshold
}
```

The function:
- Calculates 14-period ATR (with automatic warmup)
- Measures the current candle's range (high - low)
- Returns true if range exceeds 20% of ATR

### 2. Reversal Detection

```shape
@export
function detect_reversal(lookforward: number = 10) {
    let is_bullish_spike = candle[0].close > candle[0].open
    
    for i in 1..min(lookforward, remaining_candles()) {
        if is_bullish_spike {
            // Bullish reversal: price drops below spike's low
            if candle[i].close < candle[0].low {
                return { occurred: true, bars_to_reversal: i, ... }
            }
        } else {
            // Bearish reversal: price rises above spike's high
            if candle[i].close > candle[0].high {
                return { occurred: true, bars_to_reversal: i, ... }
            }
        }
    }
    return { occurred: false, ... }
}
```

### 3. Unified Process Structure

Both statistical analysis and backtesting use the same `process` construct:

```shape
process atr_spike_statistics {
    // Configuration
    let atr_threshold = 20
    
    // State management
    state {
        total_spikes: 0
        reversals: []
        // ... more state
    }
    
    // Main loop - executed for each candle
    on_candle {
        if is_atr_spike(atr_threshold) {
            // Collect statistics
            let reversal = detect_reversal(10)
            state.reversals.push({ ... })
        }
    }
    
    // Final output
    output {
        summary: { ... },
        detailed_spikes: state.reversals
    }
}
```

### 4. Key Language Features Used

#### Duration Type
```shape
from @"2020-01-01" to @"2022-12-31"  // Date literals
with timeframe("15m")                  // 15-minute bars
let lookback = 30d                     // Duration literal
```

#### Warmup System
The `atr(14)` function automatically ensures 15 candles of historical data are loaded before the first calculation (14 + 1 for true range).

#### State Management
```shape
state {
    capital: 100000
    positions: []
    trades: []
}
```
State persists across candle iterations but is scoped to the process.

#### Pattern Matching & Fuzzy Logic
While not used in this example, the spike detection could use fuzzy matching:
```shape
@fuzzy(body: 0.02, wick: 0.05)
pattern spike {
    candle[0].range >= atr(14) * 0.2
}
```

## Output Structure

### Statistics Output
```json
{
  "summary": {
    "total_spikes": 847,
    "reversal_stats": {
      "reversal_rate": 64.0,
      "avg_bars_to_reversal": 3.8
    }
  },
  "detailed_spikes": [...]
}
```

### Backtest Output
```json
{
  "performance": {
    "total_return": 47.82,
    "win_rate": 59.8,
    "sharpe_ratio": 1.82
  },
  "trades": [...],
  "daily_returns": [...]
}
```

## Execution Flow

1. **Data Loading**: Market data is loaded on-demand with warmup
2. **Process Execution**: Each candle is processed in chronological order
3. **State Updates**: State accumulates results across iterations
4. **Output Generation**: Final statistics/performance calculated

The same `is_atr_spike()` and `detect_reversal()` functions work in both contexts, demonstrating true code reuse between analysis and trading.