# Hammer Pattern Definition in Shape

## Language Expressiveness Analysis

The Shape language is indeed powerful enough to define complex patterns like the hammer. Here's a detailed breakdown:

### Basic Hammer Definition

```shape
pattern hammer ~0.02 {
    // Small body (close is near open)
    abs(candle[0].close - candle[0].open) / candle[0].open < 0.01 and
    
    // Long lower shadow (at least 2x the body size)
    (min(candle[0].open, candle[0].close) - candle[0].low) > 
        2 * abs(candle[0].close - candle[0].open) and
    
    // Small or no upper shadow
    (candle[0].high - max(candle[0].open, candle[0].close)) < 
        0.1 * abs(candle[0].close - candle[0].open)
}
```

### Language Features Demonstrated

1. **Mathematical Operations**
   - `abs()` - Absolute value for body size
   - `min()`, `max()` - Finding body boundaries
   - Division, multiplication for ratios

2. **Candle Access**
   - `candle[0]` - Current candle
   - `.open`, `.close`, `.high`, `.low` - OHLC data
   - Relative indexing with `candle[-1]` for lookback

3. **Fuzzy Matching**
   - `~0.02` - Pattern-level tolerance (2%)
   - `~=` operator for approximate equality

4. **Logical Composition**
   - `and` for combining conditions
   - Complex boolean expressions

### More Sophisticated Variants

```shape
// Hammer with market context
pattern contextual_hammer {
    // Basic hammer shape
    hammer and
    
    // Must be at a local low
    candle[0].low < lowest(low, 10) * 1.01 and
    
    // Declining trend before
    sma(close, 5) < sma(close, 5)[5]
}

// Probabilistic hammer
pattern fuzzy_hammer ~0.05 {
    // Relaxed body constraint
    abs(candle[0].close - candle[0].open) / candle[0].open < 0.02 and
    
    // Shadow ratio with tolerance
    (min(candle[0].open, candle[0].close) - candle[0].low) ~> 
        1.5 * abs(candle[0].close - candle[0].open) and
    
    // Upper shadow check
    (candle[0].high - max(candle[0].open, candle[0].close)) ~< 
        0.3 * abs(candle[0].close - candle[0].open)
}

// Weighted conditions for scoring
pattern scored_hammer {
    // Primary characteristic (most important)
    (min(candle[0].open, candle[0].close) - candle[0].low) > 
        2 * abs(candle[0].close - candle[0].open) weight 3.0 and
    
    // Small body (important)
    abs(candle[0].close - candle[0].open) / candle[0].open < 0.01 weight 2.0 and
    
    // Small upper shadow (nice to have)
    (candle[0].high - max(candle[0].open, candle[0].close)) < 
        0.1 * abs(candle[0].close - candle[0].open) weight 1.0
}
```

### Advanced Capabilities

1. **Indicator Integration**
   ```shape
   pattern hammer_oversold {
       hammer and rsi(14) < 30
   }
   ```

2. **Multi-timeframe Analysis**
   ```shape
   pattern mtf_hammer {
       hammer and
       on(1h) { sma(close, 20) > sma(close, 50) }
   }
   ```

3. **Dynamic Thresholds**
   ```shape
   pattern adaptive_hammer {
       // Body size relative to volatility
       abs(candle[0].close - candle[0].open) < atr(14) * 0.2 and
       
       // Shadow length adaptive to range
       (min(candle[0].open, candle[0].close) - candle[0].low) > 
           atr(14) * 0.5
   }
   ```

## Comparison with Traditional Approaches

### Shape Advantages:
- **Declarative**: Describes what a hammer IS, not how to find it
- **Readable**: Close to how traders describe patterns
- **Flexible**: Fuzzy matching handles real-world data
- **Composable**: Patterns can reference other patterns
- **Contextual**: Can include market conditions

### Traditional Code Example (for comparison):
```python
# Traditional imperative approach
def is_hammer(candle):
    body = abs(candle.close - candle.open)
    body_pct = body / candle.open
    
    if body_pct >= 0.01:  # Body too large
        return False
        
    lower_shadow = min(candle.open, candle.close) - candle.low
    if lower_shadow <= 2 * body:  # Shadow too short
        return False
        
    upper_shadow = candle.high - max(candle.open, candle.close)
    if upper_shadow >= 0.1 * body:  # Upper shadow too long
        return False
        
    return True
```

## Conclusion

Shape is not only powerful enough to define a hammer pattern, but it does so in a way that is:
- More expressive than traditional code
- Closer to trader terminology
- Flexible with fuzzy matching
- Extensible with indicators and multi-timeframe analysis
- Maintainable and readable

The language successfully bridges the gap between how traders think about patterns and how computers need to identify them.