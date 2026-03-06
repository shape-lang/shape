# Indicator Warmup System Design

## Overview

Shape makes indicators first-class citizens by allowing them to be defined entirely in Shape code (not hardcoded in Rust) and by automatically handling their data requirements through a warmup annotation system.

## Key Concepts

### 1. Warmup Annotations

Indicators declare their historical data requirements using the `@warmup` annotation:

```shape
@warmup(period)
function sma(period: number) -> number {
    // SMA needs 'period' candles of history
}

@warmup(period + 1)  
function atr(period: number) -> number {
    // ATR needs period + 1 (for previous close)
}
```

### 2. Dynamic Warmup Expressions

The warmup expression can reference function parameters and use any valid Shape expression:

```shape
@warmup(max(fast, slow))
function macd(fast: number, slow: number, signal: number) -> number {
    // MACD needs enough data for the slowest MA
}

@warmup(lookback * 2 + extra)
function complex_indicator(lookback: number, extra: number = 10) -> number {
    // Complex warmup calculation
}
```

### 3. Runtime Behavior

When an indicator is called, the runtime:

1. **Evaluates the warmup expression** with the actual parameters
2. **Checks data availability** at the current position
3. **Returns appropriate value**:
   - Valid calculation if enough data exists
   - `null` if insufficient data
   - Error if configured to be strict

### 4. Automatic Query Adjustment

Queries automatically respect warmup requirements:

```shape
# This query automatically starts from candle[50] onwards
find candles where close > sma(50)

# The runtime knows it needs 51 candles minimum (50 + 1 for ATR)
find candles where atr(14) > 20 and close > sma(50)
```

### 5. Standard Library Integration

Indicators are defined in `stdlib/indicators.shape`:

```shape
# stdlib/indicators.shape
export module indicators {
    @warmup(period)
    export function sma(period: number) -> number {
        let sum = 0.0;
        for i in range(0, period) {
            sum = sum + candle[-i].close;
        }
        return sum / period;
    }
    
    # ... more indicators
}
```

Usage:
```shape
import { sma, atr, rsi } from "stdlib/indicators";

# Or import all
import * as ind from "stdlib/indicators";
```

### 6. Special Cases

#### Session-Based Indicators
```shape
@warmup(dynamic)  # Evaluated at runtime
function vwap() -> number {
    // Warmup depends on time since session start
}
```

#### Stateful Indicators
```shape
@warmup(2)
@stateful  # Maintains state between calls
function parabolic_sar(af: number = 0.02) -> number {
    // SAR maintains trend state
}
```

#### No Warmup Required
```shape
@warmup(0)
function pivot_point() -> {pp: number, r1: number, s1: number} {
    // Uses only current candle
}
```

## Implementation Details

### Parser Changes

1. Add annotation support to grammar
2. Parse warmup expressions as regular expressions
3. Store annotations in FunctionDef AST node

### Runtime Changes

1. **Function Call Evaluation**:
   ```rust
   // When evaluating a function call:
   if let Some(warmup_annotation) = function.get_annotation("warmup") {
       let warmup_period = evaluate_warmup_expr(warmup_annotation, &actual_params)?;
       if ctx.current_candle() < warmup_period {
           return Ok(Value::Null); // Or error based on config
       }
   }
   ```

2. **Query Processing**:
   ```rust
   // When processing queries, calculate minimum start position:
   let min_position = query.get_required_warmup();
   for i in min_position..candles.len() {
       // Process query from safe starting point
   }
   ```

3. **Caching**:
   - Cache calculated indicator values per position
   - Reuse calculations when possible
   - Clear cache on data updates

## Benefits

1. **Self-Documenting**: Warmup requirements are explicit in the code
2. **Type-Safe**: Can't accidentally use indicators without enough data  
3. **Flexible**: Supports any warmup calculation logic
4. **Portable**: Indicators defined in Shape work anywhere
5. **Optimizable**: Runtime can pre-calculate indicators for efficiency

## Migration Path

1. Remove hardcoded indicators from Rust
2. Implement annotation parsing
3. Create standard library with annotated indicators
4. Update runtime to respect warmup requirements
5. Update documentation and examples

## Future Extensions

1. **Multiple Annotations**:
   ```shape
   @warmup(period)
   @cache(true)
   @gpu_accelerated
   function sma(period: number) -> number { }
   ```

2. **Conditional Warmup**:
   ```shape
   @warmup(mode == "fast" ? period : period * 2)
   function adaptive_ma(period: number, mode: string) -> number { }
   ```

3. **Data Requirements Beyond Warmup**:
   ```shape
   @requires_volume
   @requires_timeframe("1m", "5m", "15m")
   function volume_indicator() -> number { }
   ```