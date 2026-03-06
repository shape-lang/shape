# Walk-Forward Analysis Guide

## What is Walk-Forward Analysis?

Walk-forward analysis is a robust method for testing trading strategies that helps prevent overfitting. It simulates how a strategy would perform in real trading by:

1. **Optimizing** parameters on historical data (in-sample)
2. **Testing** those parameters on future unseen data (out-of-sample)
3. **Rolling forward** and repeating the process

This mimics real trading where you optimize based on past data and trade on future data.

## Why Walk-Forward Analysis?

### The Overfitting Problem

When you optimize a strategy on historical data, you risk finding parameters that work perfectly on that specific data but fail on new data. This is overfitting.

**Example of Overfitting:**
- Backtest on 2020-2023 data: 50% annual return, Sharpe 3.0
- Live trading in 2024: -20% return

### The Walk-Forward Solution

Walk-forward analysis prevents overfitting by:
- Never testing on the same data used for optimization
- Showing how parameters perform on truly unseen data
- Revealing if your edge is real or just curve-fitting

## How Walk-Forward Works

```
Timeline: [====|====|====|====|====|====]
           2019  2020  2021  2022  2023  2024

Window 1:  [Optimize][Test]
           2019-2020  2021

Window 2:       [Optimize][Test]
                2020-2021  2022

Window 3:            [Optimize][Test]
                     2021-2022  2023
```

Each window:
1. Optimizes parameters on in-sample period
2. Tests those exact parameters on out-of-sample period
3. Records the performance degradation

## Using Walk-Forward in Shape

### Basic Usage

```shape
import { run_walk_forward } from "stdlib/walk_forward"

// Define your parameter ranges
let parameter_ranges = {
    fast_ma: [10, 15, 20, 25, 30],
    slow_ma: [30, 40, 50, 60, 70],
    stop_loss: [0.01, 0.02, 0.03]
}

// Run walk-forward analysis
let results = run_walk_forward(
    "my_strategy",
    parameter_ranges,
    {
        in_sample_ratio: 0.6,     // 60% for optimization
        out_sample_ratio: 0.2,    // 20% for testing
        step_ratio: 0.2,          // Step forward 20%
        optimization_metric: "sharpe"
    }
)

// Check robustness
print("Robustness score: ", results.robustness_score, "/100")
```

### Configuration Options

```shape
let config = {
    // Data split ratios
    in_sample_ratio: 0.6,      // Optimization period
    out_sample_ratio: 0.2,     // Test period
    step_ratio: 0.2,           // How much to step forward
    
    // Quality controls
    min_trades_per_window: 30, // Minimum trades required
    
    // Optimization target
    optimization_metric: "sharpe", // Options: sharpe, return, calmar
    
    // Window type
    anchored: false  // false = rolling, true = expanding
}
```

## Interpreting Results

### Robustness Score (0-100)

The robustness score combines multiple factors:

- **80-100**: Excellent - Strategy is robust and tradeable
- **60-80**: Good - Strategy shows promise but needs monitoring
- **40-60**: Moderate - Consider further testing or improvements
- **0-40**: Poor - Likely overfitted, not recommended for live trading

### Key Metrics to Check

1. **Out-of-Sample Win Rate**
   - What percentage of windows were profitable?
   - Should be > 60% for confidence

2. **Performance Degradation**
   - How much does performance drop from in-sample to out-sample?
   - < 30% degradation is good
   - > 50% degradation suggests overfitting

3. **Parameter Stability**
   - Do optimal parameters change drastically between windows?
   - Stable parameters = robust strategy

4. **Consistency**
   - Is out-of-sample performance consistent across windows?
   - High variance = unstable strategy

## Example: Complete Walk-Forward Test

```shape
strategy trend_following {
    param lookback: number = 20
    param multiplier: number = 2.0
    param risk_pct: number = 0.02
    
    // Strategy logic here...
}

test "Validate trend following strategy" {
    let results = run_walk_forward(
        "trend_following",
        {
            lookback: [10, 20, 30, 40],
            multiplier: [1.5, 2.0, 2.5, 3.0],
            risk_pct: [0.01, 0.02, 0.03]
        }
    )
    
    // Detailed analysis
    print("=== Walk-Forward Results ===")
    print("Windows tested: ", results.summary_stats.total_windows)
    print("Profitable windows: ", results.summary_stats.profitable_windows)
    print("Robustness score: ", results.robustness_score)
    
    // Check each window
    for window in results.windows {
        if window.degradation > 0.5 {
            print("Warning: High degradation in window ", window.window_index)
        }
    }
    
    // Parameter analysis
    print("\n=== Most Stable Parameters ===")
    for param in keys(results.parameter_stability) {
        let stability = results.parameter_stability[param]
        print(param, ": ", stability.most_common, 
              " (stability: ", stability.stability_score, ")")
    }
    
    // Decision
    if results.robustness_score > 60 {
        print("\n✓ Strategy passes walk-forward validation")
    } else {
        print("\n✗ Strategy fails walk-forward validation")
    }
}
```

## Types of Walk-Forward Analysis

### 1. Rolling Window
- Fixed-size windows that roll forward
- Each optimization uses same amount of data
- Good for adapting to changing markets

```shape
let results = run_walk_forward(strategy, params, {
    anchored: false  // Rolling window
})
```

### 2. Anchored/Expanding Window
- Start date is fixed, end date expands
- Each optimization uses more data
- Good for strategies that benefit from more history

```shape
let results = run_walk_forward(strategy, params, {
    anchored: true  // Expanding window
})
```

### 3. Quick Robustness Check
- Simplified test with fixed parameters
- Faster but less thorough
- Good for initial screening

```shape
let score = quick_robustness_check("my_strategy", {
    fast_ma: 20,
    slow_ma: 50
})
```

## Best Practices

### 1. Adequate Sample Size
- Each window needs sufficient trades (minimum 30-50)
- Total analysis should cover multiple market conditions
- Include both trending and choppy periods

### 2. Reasonable Parameter Ranges
- Don't test every possible value
- Use domain knowledge to set sensible ranges
- Fewer parameters = more robust

### 3. Multiple Metrics
- Don't optimize only for returns
- Consider risk-adjusted metrics (Sharpe, Calmar)
- Check multiple performance aspects

### 4. Out-of-Sample Size
- Too small: Not enough data for validation
- Too large: Not enough windows for analysis
- Typical: 20-40% out-of-sample ratio

## Common Pitfalls

### 1. Too Few Windows
**Problem**: Only 2-3 windows tested
**Solution**: Ensure at least 5-10 windows

### 2. Tiny Parameter Steps
**Problem**: Testing 20, 21, 22, 23...
**Solution**: Use meaningful steps (10, 20, 30...)

### 3. In-Sample Bias
**Problem**: Selecting strategy based on in-sample results
**Solution**: Focus on out-of-sample performance

### 4. Ignoring Degradation
**Problem**: 80% degradation but still profitable
**Solution**: High degradation = overfitting warning

## Real Example: MA Crossover

```shape
// Historical full backtest
Full period return: 25% annual
Sharpe ratio: 1.5

// Walk-forward results
Window 1: In: 30%, Out: 18% (40% degradation)
Window 2: In: 25%, Out: 20% (20% degradation)
Window 3: In: 35%, Out: 15% (57% degradation)
Window 4: In: 20%, Out: 22% (-10% degradation)
Window 5: In: 28%, Out: 12% (57% degradation)

Average out-of-sample: 17.4%
Robustness score: 58/100

Conclusion: Moderate robustness, some overfitting present
```

## Summary

Walk-forward analysis is essential for validating trading strategies. It:
- Prevents overfitting by testing on unseen data
- Shows realistic expected performance
- Reveals parameter stability
- Provides confidence before live trading

Always run walk-forward analysis before trusting any backtest results!