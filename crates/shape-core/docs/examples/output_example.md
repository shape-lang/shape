# ATR Reversal Analysis Output Example

This shows how the Shape query system outputs both statistical analysis and backtest results.

## Query Execution

```shape
:data /home/amd/dev/finance/data ES 2020-01-01 2022-12-31
run analyze_and_trade_atr_reversals on timeframe 15m
```

## Output

```
=== Analysis & Backtest Query Results ===
Symbol: ES | Timeframe: 15m | Period: 2020-01-01 to 2022-12-31

STATISTICAL ANALYSIS:
  Total Occurrences: 2,847
  Success Rate: 68.32%
  Avg Magnitude: 0.42% (σ=0.18%)
  
  Pattern Distribution:
    Bullish Aggressive Moves: 1,423 (49.98%)
      → Reversal Rate: 71.2%
    Bearish Aggressive Moves: 1,424 (50.02%)  
      → Reversal Rate: 65.4%
  
  Best Trading Hours (EST):
    1. 09:30-10:00: 76.3% reversal rate (312 occurrences)
    2. 14:30-15:00: 72.1% reversal rate (198 occurrences)
    3. 02:00-02:30: 70.5% reversal rate (156 occurrences)
  
  Best Trading Days:
    1. Tuesday: 71.2% reversal rate (584 occurrences)
    2. Thursday: 69.8% reversal rate (612 occurrences)

BACKTEST RESULTS:
  Initial Capital: $10,000
  Total Return: $3,847.52 (38.48%)
  Annualized Return: 12.82%
  
  Risk Metrics:
    Sharpe Ratio: 1.42
    Sortino Ratio: 1.78
    Max Drawdown: -8.73% (42 days)
    Calmar Ratio: 1.47
  
  Trading Statistics:
    Total Trades: 2,847
    Winning Trades: 1,542 (54.16%)
    Profit Factor: 1.68
    
  Trade Analysis:
    Avg Winner: $18.42
    Avg Loser: $-9.87
    Largest Winner: $124.30
    Largest Loser: $-45.60
    Avg Trade Duration: 3.2 hours
    
  Risk-Reward Analysis:
    Target RR: 2.0
    Achieved RR: 1.87
    Avg Risk per Trade: 0.98%
    
  Monthly Performance:
    Best Month: Mar 2020 (+8.4%, 156 trades)
    Worst Month: Jun 2021 (-2.1%, 98 trades)
    Positive Months: 26/36 (72.2%)

KEY INSIGHTS:
  Edge per Trade: $1.35
  Expectancy: $1.31
  Kelly Criterion: 8.4%
  Optimal Position Size: 4.2%
  Suggested Risk per Trade: 1.0%
  
  Pattern-to-Trade Correlation: 0.73 (Strong)
  Confidence Score: 78/100

✓ HIGH CONFIDENCE: Strong statistical edge with good risk/reward

TRADE DISTRIBUTION:
  By Hour (Top 5):
    09:30-10:00: 312 trades, 58.3% win rate, $2.14 avg
    14:30-15:00: 198 trades, 56.1% win rate, $1.87 avg
    10:00-10:30: 245 trades, 55.2% win rate, $1.54 avg
    13:00-13:30: 189 trades, 54.7% win rate, $1.42 avg
    02:00-02:30: 156 trades, 53.8% win rate, $1.28 avg
    
  By Day of Week:
    Monday:    487 trades, 52.8% win rate
    Tuesday:   584 trades, 55.8% win rate
    Wednesday: 523 trades, 54.1% win rate
    Thursday:  612 trades, 54.9% win rate
    Friday:    641 trades, 53.2% win rate

RECOMMENDATIONS:
  1. Focus trading during morning session (9:30-10:30 EST)
  2. Tuesdays and Thursdays show highest edge
  3. Use 1% risk per trade with 4% total position size
  4. Consider reducing position size during summer months
  5. Monitor for regime changes if reversal rate drops below 65%

SAMPLE TRADES:
  Best Trade: 2020-03-23 09:42 
    Short ES @ 2,341.50, Stop: 2,358.00, Target: 2,308.50
    Exit: 2,308.50 (Target), P&L: +$1,625.00, RR: 2.0
    
  Worst Trade: 2021-06-15 14:12
    Long ES @ 4,247.25, Stop: 4,235.00, Target: 4,271.75  
    Exit: 4,235.00 (Stop), P&L: -$612.50, RR: -1.0

WARNINGS:
  ⚠ Performance degraded in low volatility periods (VIX < 15)
  ⚠ Overnight gaps affected 12% of trades
  ⚠ Consider adding volatility filter for improved performance
```

## Interactive Features

The Shape system also supports:

1. **Drill-down Analysis**
   ```shape
   // Analyze specific period
   analyze trades where date between "2020-03-01" and "2020-04-30"
   ```

2. **Parameter Optimization**
   ```shape
   optimize atr_threshold from 0.15 to 0.30 step 0.05
   optimize risk_reward from 1.5 to 3.0 step 0.5
   ```

3. **Real-time Monitoring**
   ```shape
   monitor analyze_and_trade_atr_reversals 
   alert when reversal_rate < 0.65 or win_rate < 0.50
   ```

4. **Export Results**
   ```shape
   export results to "atr_reversal_analysis.json"
   export trades to "atr_trades.csv"
   export equity_curve to "performance.png"
   ```