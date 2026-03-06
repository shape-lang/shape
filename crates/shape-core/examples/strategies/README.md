# Shape Strategy Examples

This directory contains complete trading strategy implementations in Shape, demonstrating the unified execution architecture where the same logic serves both statistical analysis and backtesting.

## Contents

- **atr_spike_reversal_complete.shape** - Comprehensive ATR-based reversal strategy showing:
  - Statistical analysis of 20%+ ATR price spikes
  - Full backtesting with position management
  - Risk management with ATR-based stops/targets
  - Performance metrics calculation

- **strategy_example.shape** - General strategy template showing:
  - Entry/exit logic
  - Position sizing
  - State management across candles

- **unified_execution_example.shape** - Demonstrates the unified execution model:
  - Single codebase for statistics and trading
  - Process statement usage
  - State management patterns

## Key Concepts

All strategies use the `process` statement:

```shape
process my_strategy {
    state {
        // Track positions, capital, etc.
    }
    
    on_candle {
        // Logic executed for each candle
    }
    
    output {
        // Results and metrics
    }
}
```

The same process can be run for:
- Statistical analysis: `run process my_strategy ...`
- Backtesting: Same syntax, different state management
- Real-time monitoring: With streaming data

## Best Practices

1. **No Look-ahead Bias**: Only access `candle[0]` and negative indices
2. **Transaction Costs**: Include realistic slippage and commissions
3. **Risk Management**: Always define stops and position sizing
4. **State Management**: Use the `state` block for persistence