# Shape Analyze Query Reference

## Overview

The `analyze` query in Shape provides powerful data aggregation and grouping capabilities for market data analysis. It allows you to perform complex statistical analysis, group data by various dimensions, and calculate multiple metrics in a single query.

## Syntax

```
analyze <target> 
[where <condition>] 
[group by <grouping_expression>, ...] 
calculate <aggregation>, ...
```

### Components

#### 1. Target
Specifies what data to analyze:
- **Time Windows**: `last(N days/hours/minutes)`, `today`, `yesterday`, `this week`
- **Pattern Matches**: `find(pattern_name, window)`
- **Expressions**: Any expression that evaluates to a dataset

#### 2. Where Clause (Optional)
Filters the data before analysis:
```
where candle.volume > 1000000
where candle.close > candle.open and candle.volume > avg(candle.volume)
```

#### 3. Group By (Optional)
Groups data before aggregation:
- **Field Grouping**: `group by color` (red/green candles)
- **Time Intervals**: `group by 1 hour`, `group by 1 day`
- **Expressions**: `group by round(candle.close, 1.0)`
- **Special Functions**: See "Grouping Functions" below

#### 4. Calculate (Required)
Specifies aggregations to compute:
```
calculate 
    count = count,
    avg_price = avg(candle.close),
    total_volume = sum(candle.volume)
```

## Aggregation Functions

### Standard Aggregations
- `count` - Count of items
- `sum(expr)` - Sum of values
- `avg(expr)` - Average
- `min(expr)` - Minimum value
- `max(expr)` - Maximum value
- `stddev(expr)` - Standard deviation
- `percentile(expr, n)` - Nth percentile (0-100)
- `first(expr)` - First value in the group
- `last(expr)` - Last value in the group

### Custom Aggregations
- `median(expr)` - Median value
- `variance(expr)` or `var(expr)` - Statistical variance
- `mode(expr)` - Most common value
- `range(expr)` - Maximum - minimum
- `iqr(expr)` - Interquartile range (75th - 25th percentile)
- `skewness(expr)` - Distribution skewness
- `kurtosis(expr)` - Distribution kurtosis
- `weighted_avg(expr, weight)` - Weighted average

## Grouping Functions

### Time-based Grouping
- `session()` - Groups by trading session (PreMarket, Regular, AfterHours, Closed)
- `hour_of_day()` - Groups by hour (0-23)
- `day_of_week()` - Groups by day name (Mon, Tue, etc.)
- `month_of_year()` - Groups by month name (January, February, etc.)
- `business_day()` - Groups by business days (excludes weekends)
- `fiscal_quarter(start_month)` - Groups by fiscal quarters

### Examples

#### Basic Count
```
analyze last(30 days) calculate count
```

#### Volume Profile
```
analyze last(100 candles) 
group by round(candle.close, 0.50)
calculate 
    volume = sum(candle.volume),
    vwap = sum(candle.close * candle.volume) / sum(candle.volume)
```

#### Session Analysis
```
analyze last(30 days)
group by session()
calculate 
    avg_volume = avg(candle.volume),
    volatility = stddev(candle.close),
    count = count
```

#### Conditional Aggregation
```
analyze last(7 days)
calculate 
    total_volume = sum(candle.volume),
    green_volume = sum(candle.volume and candle.close > candle.open)
```

## Advanced Features

### Multiple Grouping
You can group by multiple dimensions:
```
analyze last(60 days)
group by session(), day_of_week()
calculate avg_volume = avg(candle.volume)
```

### Complex Expressions
Use any valid Shape expression in grouping or calculations:
```
analyze last(30 days)
group by candle.volume > percentile(candle.volume, 75)
calculate 
    count = count,
    avg_move = avg(abs(candle.close - candle.open))
```

### Pattern Analysis
Analyze pattern occurrences:
```
analyze find(hammer, last(90 days))
group by hour_of_day()
calculate 
    pattern_count = count,
    success_rate = sum(pattern.confirmed) / count
```

## Output Format

The analyze query returns an `AnalysisResult` with:
- `rows`: Array of result rows, each containing:
  - `group_keys`: Map of grouping dimension to value
  - `metrics`: Map of metric name to calculated value
- `totals`: Optional totals row (when grouping is used)

### Example Output
```json
{
  "rows": [
    {
      "group_keys": {"session": "Regular"},
      "metrics": {
        "avg_volume": 1234567.89,
        "count": 1950
      }
    },
    {
      "group_keys": {"session": "PreMarket"},
      "metrics": {
        "avg_volume": 456789.12,
        "count": 650
      }
    }
  ],
  "totals": {
    "group_keys": {},
    "metrics": {
      "avg_volume": 1045678.50,
      "count": 2600
    }
  }
}
```

## Performance Considerations

1. **Use appropriate time windows** - Larger windows require more processing
2. **Filter early with WHERE clause** - Reduces data before grouping
3. **Limit grouping dimensions** - Each additional dimension increases result size
4. **Consider caching** - Results are cacheable for repeated queries

## Common Use Cases

### Market Microstructure Analysis
- Volume distribution by price level
- Trading activity by time of day
- Session-based performance metrics

### Risk Analysis
- Volatility calculations
- Value at Risk approximations
- Drawdown analysis

### Pattern Analysis
- Pattern frequency by market conditions
- Success rates by time of day
- Pattern performance metrics

### Trend Analysis
- Moving statistics over time intervals
- Momentum indicators
- Volume-price relationships