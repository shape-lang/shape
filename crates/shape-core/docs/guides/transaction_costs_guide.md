# Transaction Costs in Shape

## Overview

Shape provides comprehensive transaction cost modeling to ensure realistic backtesting results. The system supports both built-in Rust-based cost models and custom Shape-defined models.

## Why Transaction Costs Matter

Without proper transaction cost modeling, backtest results can be misleadingly optimistic. Real-world trading incurs various costs:
- **Commission fees**: Broker charges per trade or per share
- **Market impact**: Price movement caused by your order
- **Slippage**: Difference between expected and actual execution price
- **Spread costs**: Bid-ask spread crossing
- **Regulatory fees**: SEC, TAF, and other regulatory charges

## Using Transaction Costs in Shape

### 1. Import the Execution Module

```shape
import { create_backtest_cost_model, calculate_transaction_cost } from "stdlib/execution"
```

### 2. Create a Cost Model

Shape provides pre-configured cost models for different asset classes:

```shape
// Equity markets (US stocks)
let cost_model = create_backtest_cost_model("equity")

// Cryptocurrency markets
let cost_model = create_backtest_cost_model("crypto")

// Foreign exchange markets
let cost_model = create_backtest_cost_model("forex")
```

### 3. Customize Cost Models

You can override default settings:

```shape
let cost_model = create_backtest_cost_model("equity", {
    commission: commission_per_share(0.005),      // $0.005 per share
    slippage: slippage_linear(2, 15),            // 2bp base + size impact
    min_commission: 1.0,                         // $1 minimum
    max_commission: 100.0                        // $100 maximum
})
```

## Cost Model Types

### Commission Models

1. **Fixed per trade**
```shape
commission_fixed_per_trade(5.00)  // $5 per trade
```

2. **Per share/contract**
```shape
commission_per_share(0.005)  // $0.005 per share
```

3. **Percentage of trade value**
```shape
commission_percentage(0.001)  // 0.1% of trade value
```

4. **Tiered commission**
```shape
commission_tiered([
    {min_value: 0, max_value: 10000, fixed: 0, rate: 0.0010},
    {min_value: 10000, max_value: null, fixed: 0, rate: 0.0008}
])
```

### Slippage Models

1. **Fixed slippage**
```shape
slippage_fixed(5)  // 5 basis points
```

2. **Linear impact (size-dependent)**
```shape
slippage_linear(2, 10)  // 2bp base + 10bp per 100% daily volume
```

3. **Square-root impact (Almgren-Chriss model)**
```shape
slippage_square_root(0.5)  // Impact coefficient
```

## Example: Realistic Strategy with Costs

```shape
strategy moving_average_crossover {
    // Configure realistic costs
    let cost_model = create_backtest_cost_model("equity", {
        commission: commission_per_share(0.005),
        slippage: slippage_linear(2, 15),
        min_commission: 1.0
    })
    
    let capital = 100000
    let position = null
    
    // Strategy logic
    when sma(20) > sma(50) and position == null {
        // Calculate costs before entry
        let shares = floor(capital * 0.02 / candle.close)  // 2% position
        let costs = calculate_transaction_cost(
            shares, 
            candle.close, 
            "buy", 
            cost_model
        )
        
        // Enter position with costs
        position = {
            shares: shares,
            entry_price: costs.execution_price,
            costs: costs.total_cost
        }
        capital -= (shares * costs.execution_price + costs.total_cost)
    }
    
    when sma(20) < sma(50) and position != null {
        // Calculate exit costs
        let costs = calculate_transaction_cost(
            position.shares,
            candle.close,
            "sell",
            cost_model
        )
        
        // Exit with costs
        capital += (position.shares * costs.execution_price - costs.total_cost)
        
        // Calculate net P&L
        let gross_pnl = (costs.execution_price - position.entry_price) * position.shares
        let net_pnl = gross_pnl - position.costs - costs.total_cost
        
        print("Trade complete - Net P&L: $", net_pnl)
        position = null
    }
}
```

## Advanced Features

### Market Context

For more accurate slippage modeling, provide market context:

```shape
let market_context = {
    daily_volume: candle.volume * 390,  // Estimate daily from minute bars
    volatility: atr(20) / candle.close, // Current volatility
    bid_ask_spread: 0.01               // 1 cent spread
}

let costs = calculate_transaction_cost(
    quantity, price, side, cost_model, market_context
)
```

### Cost Analysis

The transaction cost calculator returns detailed breakdown:

```shape
let costs = calculate_transaction_cost(100, 50.00, "buy", cost_model)

// Access components
print("Commission: $", costs.commission)
print("Slippage: $", costs.slippage)
print("Regulatory fees: $", costs.regulatory_fees)
print("Total cost: $", costs.total_cost)
print("Execution price: $", costs.execution_price)
```

## Best Practices

1. **Always include costs in backtests** - Results without costs are unrealistic
2. **Use appropriate models** - Different asset classes have different cost structures
3. **Consider market conditions** - Costs vary with volatility and liquidity
4. **Track cost impact** - Monitor how much costs affect your strategy
5. **Be conservative** - When in doubt, overestimate rather than underestimate costs

## Cost Impact Analysis

Track the impact of transaction costs on your strategy:

```shape
on complete {
    let gross_pnl = sum(trades, t => t.gross_pnl)
    let total_costs = sum(trades, t => t.total_costs)
    let net_pnl = gross_pnl - total_costs
    
    print("Gross P&L: $", gross_pnl)
    print("Total costs: $", total_costs)
    print("Net P&L: $", net_pnl)
    print("Cost impact: ", (total_costs / abs(gross_pnl) * 100), "% of gross")
    
    // Breakeven analysis
    let avg_cost_per_trade = total_costs / len(trades)
    let required_edge = avg_cost_per_trade / (capital / len(trades))
    print("Required edge to break even: ", required_edge * 100, "%")
}
```

## Integration with Built-in Cost Model

Shape's transaction cost models integrate seamlessly with the position manager:

```shape
// The position manager automatically applies costs when configured
strategy.set_cost_model(cost_model_equity())

// Positions opened through the position manager will include costs
position_manager.open("AAPL", "long", 100, candle.close)
```

This ensures consistent cost application across all trading operations.