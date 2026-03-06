# Shape Tutorial Examples

This directory contains simple, educational examples to help you learn Shape step by step.

## Learning Path

### 1. Basic Queries
- **simple_atr_spike_query.shape** - Find price spikes using ATR
  - Basic query syntax
  - Using indicators in conditions
  - Selecting and filtering data

### 2. Pattern Recognition
- **pattern_definitions.shape** - Common candlestick patterns
  - Defining patterns
  - Using fuzzy matching
  - Pattern composition

### 3. Indicators
- **simple_indicator_test.shape** - Working with indicators
  - Calling indicator functions
  - Understanding warmup
  - Combining indicators

## Key Concepts for Beginners

### Candle Access
```shape
candle[0]     // Current candle
candle[-1]    // Previous candle
candle[0].close  // Close price
```

### Time References
```shape
@today        // Today's date
@"2024-01-01" // Specific date
15m          // Duration literal
```

### Basic Queries
```shape
query find_spikes {
    from candles
    where candle.range > atr(14) * 0.2
    select {
        time: candle.timestamp,
        size: candle.range / atr(14)
    }
}
```

## Next Steps

After mastering these tutorials, explore:
- `/strategies/` - Complete trading strategies
- `/benchmarks/` - Performance optimization examples
- The Shape documentation for advanced features