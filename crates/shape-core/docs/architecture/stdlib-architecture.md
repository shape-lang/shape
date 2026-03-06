# Shape Standard Library Architecture

## Overview

The Shape standard library is organized in a layered architecture to ensure clean dependencies and maintainability. Each layer can only depend on layers below it, preventing circular dependencies.

## Layer Structure

### Layer 0: Primitives (No Dependencies)
Core utilities and primitive operations that don't depend on any other modules.

- **primitives/candle_analysis.shape** - Basic candle analysis functions
- **utils.shape** - General utility functions (lerp, clamp, sigmoid, etc.)
- **execution.shape** - Order execution utilities

### Layer 1: Core Types (No Dependencies)
Fundamental type definitions used throughout the library.

- **types/signal.shape** - Trading signal interface

### Layer 2: Basic Indicators (No Dependencies)
Core technical indicators that operate on raw price data.

- **indicators/moving_averages.shape** - SMA, EMA, WMA, VWMA
- **indicators/atr.shape** - Average True Range
- **indicators/vwap.shape** - Volume Weighted Average Price
- **indicators/volume.shape** - Volume-based indicators

### Layer 3: Dependent Types & Advanced Indicators
Types and indicators that build on lower layers.

- **types/strategy.shape** - Strategy interface (depends on Signal)
- **types/backtest.shape** - Backtesting types (depends on Signal)
- **indicators/oscillators.shape** - RSI, MACD, Stochastic (depends on MAs)
- **indicators/volatility.shape** - Bollinger Bands, Keltner Channels

### Layer 4: Composite Types & Analysis
Complex types and pattern analysis that combine multiple indicators.

- **types/portfolio.shape** - Portfolio interface (depends on Strategy)
- **patterns.shape** - Chart pattern detection (depends on indicators)
- **risk.shape** - Risk metrics and analysis (depends on indicators)
- **walk_forward.shape** - Walk-forward optimization

### Layer 5: High-Level Modules
Advanced functionality that orchestrates lower layers.

- **statistics.shape** - Statistical analysis (depends on risk)
- **backtesting/simulate_trades.shape** - Backtesting engine (depends on types)

### Layer 6: Root Aggregators
Top-level modules that re-export functionality.

- **index.shape** - Main entry point, re-exports all modules
- **types/index.shape** - Re-exports all types

## Dependency Rules

1. **Upward Only**: Modules can only import from lower layers
2. **No Circular Imports**: A depends on B means B cannot depend on A
3. **Type-First**: When in doubt, extract shared types to Layer 1
4. **Minimal Dependencies**: Import only what you need

## Import Examples

```shape
// Good - Layer 3 importing from Layer 1
// In types/strategy.shape
import { Signal } from "./signal";

// Good - Layer 5 importing from Layer 4
// In statistics.shape
import { sharpe_ratio } from "./risk";

// Bad - Would create circular dependency
// In risk.shape
// import { correlation } from "./statistics"; // Don't do this!
```

## Adding New Modules

When adding a new module:

1. **Identify Dependencies**: What existing modules does it need?
2. **Determine Layer**: Place it in the lowest layer above all its dependencies
3. **Document Imports**: Add a comment listing all imports at the top
4. **Update This Doc**: Add the module to the appropriate layer section

## Module Guidelines

### Types Modules
- Define interfaces and type aliases
- No implementation logic
- Minimal dependencies

### Indicator Modules
- Pure functions operating on price/volume data
- No side effects
- Well-documented parameters and return types

### Pattern Modules
- Composable pattern detection functions
- Clear naming conventions
- Include confidence scores

### Utility Modules
- General-purpose helper functions
- No domain-specific logic
- Extensive unit tests

## Future Considerations

1. **Versioning**: Consider semantic versioning for stdlib
2. **Performance**: Profile and optimize hot paths
3. **Testing**: Maintain high test coverage
4. **Documentation**: Keep examples up-to-date
5. **Compatibility**: Ensure backward compatibility