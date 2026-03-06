# ATR Spike Reversal: Old vs New Approach

## Old Approach (Hardcoded Rust)

```rust
// src/runtime/strategies/atr_reversal.rs
pub struct ATRReversalStrategy {
    atr_period: usize,
    spike_threshold: f64,
    lookforward: usize,
}

impl Strategy for ATRReversalStrategy {
    fn evaluate(&self, candles: &[Candle], index: usize) -> Signal {
        // Hardcoded ATR calculation
        let atr = calculate_atr(&candles[..index], self.atr_period);
        let spike = (candles[index].high - candles[index].low) / atr;
        
        if spike > self.spike_threshold {
            // Hardcoded reversal logic
            let is_bullish = candles[index].close > candles[index].open;
            return if is_bullish { Signal::Short } else { Signal::Long };
        }
        Signal::None
    }
}

// src/runtime/analysis/reversal_stats.rs
pub fn analyze_reversals(data: &MarketData) -> ReversalStats {
    let mut stats = ReversalStats::new();
    
    for i in 0..data.candles.len() {
        // Duplicate spike detection logic
        let atr = calculate_atr(&data.candles[..i], 14);
        let spike = (data.candles[i].high - data.candles[i].low) / atr;
        
        if spike > 0.2 {
            // Duplicate reversal detection
            // ... 50+ lines of code ...
        }
    }
    
    stats
}

// Problems:
// 1. Logic duplicated between strategy and analysis
// 2. Parameters hardcoded in Rust
// 3. Need to recompile to change strategy
// 4. No code reuse between statistics and backtesting
// 5. Complex state management in Rust
```

## New Approach (Shape)

```shape
// Everything in Shape - no Rust changes needed

// Shared spike detection - used by both stats and backtest
@export
function is_atr_spike(threshold_percent: number = 20) {
    let atr_value = atr(14)  
    if atr_value == null return false
    
    let price_change = candle[0].high - candle[0].low
    return price_change >= atr_value * (threshold_percent / 100)
}

// Statistical analysis process
process atr_spike_statistics {
    state { total_spikes: 0, reversals: [] }
    
    on_candle {
        if is_atr_spike(20) {  // Same function!
            let reversal = detect_reversal(10)
            state.reversals.push({...})
        }
    }
    
    output { summary: {...}, detailed_spikes: state.reversals }
}

// Backtesting process  
process atr_spike_backtest {
    state { capital: 100000, positions: [], trades: [] }
    
    on_candle {
        // Manage positions...
        
        if is_atr_spike(20) && state.positions.length == 0 {  // Same function!
            // Enter position
        }
    }
    
    output { performance: {...}, trades: state.trades }
}

// Run both with same syntax
let stats = run process atr_spike_statistics on "ES" with timeframe("15m") from @"2020-01-01" to @"2022-12-31"
let backtest = run process atr_spike_backtest on "ES" with timeframe("15m") from @"2020-01-01" to @"2022-12-31"
```

## Key Improvements

### 1. Single Source of Truth
- `is_atr_spike()` function used by both analysis types
- No logic duplication
- Changes automatically apply everywhere

### 2. Flexibility
- Modify parameters without recompiling
- Test variations quickly
- Add new analysis types easily

### 3. Clarity
- Business logic in Shape is readable
- State management is explicit
- Process flow is clear

### 4. First-Class Features
```shape
// Duration literals
let lookback = 30d

// Automatic warmup
@warmup(period + 1)
function atr(period: number = 14) { ... }

// Property-specific fuzzy matching
@fuzzy(body: 0.02, wick: 0.05)
pattern spike { ... }

// Datetime-based access
candle[@"2020-03-09 09:30"]
```

### 5. Unified Execution
The `process` construct works for:
- Statistical analysis
- Backtesting
- Real-time monitoring
- Optimization
- Walk-forward analysis

All using the same code patterns and state management.

## Migration Path

Old code:
```rust
let strategy = ATRReversalStrategy::new(14, 0.2, 10);
let results = backtest(strategy, data);
```

New code:
```shape
let results = run process atr_spike_backtest on "ES" with {
    atr_period: 14,
    spike_threshold: 20,
    lookforward: 10
}
```

The entire strategy logic now lives in Shape, making it:
- Easier to understand
- Faster to modify
- Simpler to test
- More maintainable