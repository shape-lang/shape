# Multi-Symbol Analysis Query Examples

This document provides practical examples of using Shape's multi-symbol analysis capabilities.

## Basic Multi-Symbol Queries

### Loading Multiple Symbols
```shape
// Load data for multiple symbols
let tech_stocks = {
    aapl: load_csv("data/AAPL_1h.csv", "AAPL", "1h"),
    googl: load_csv("data/GOOGL_1h.csv", "GOOGL", "1h"),
    msft: load_csv("data/MSFT_1h.csv", "MSFT", "1h"),
    amzn: load_csv("data/AMZN_1h.csv", "AMZN", "1h")
};
```

### Symbol Alignment
```shape
// Align symbols to common timestamps
let aligned = align_symbols([tech_stocks.aapl, tech_stocks.googl], "intersection");

// Union mode includes all timestamps from any symbol
let all_times = align_symbols([tech_stocks.aapl, tech_stocks.googl], "union");
```

## Correlation Analysis

### Pairwise Correlation
```shape
// Calculate correlation between two symbols
let correlation_value = correlation(tech_stocks.aapl, tech_stocks.googl);

if (correlation_value > 0.8) {
    print("High positive correlation: " + correlation_value);
} else if (correlation_value < -0.8) {
    print("High negative correlation: " + correlation_value);
}
```

### Rolling Correlation
```shape
// Calculate correlation over sliding windows
function rolling_correlation(data1, data2, window_size, step) {
    let results = [];
    let candles1 = get_candles(data1);
    let candles2 = get_candles(data2);
    
    for (let i = window_size; i < candles1.length; i += step) {
        // Create temporary datasets for window
        let window_corr = correlation(
            slice_data(data1, i - window_size, i),
            slice_data(data2, i - window_size, i)
        );
        
        results.push({
            timestamp: candles1[i].timestamp,
            correlation: window_corr
        });
    }
    
    return results;
}
```

## Divergence Detection

### Price Divergences
```shape
// Find divergences with customizable window
let divergences = find_divergences(tech_stocks.aapl, tech_stocks.googl, 20);

// Filter for strong divergences
let strong_divergences = divergences.filter(d => d.strength > 1.0);

// Alert on recent divergences
let recent_div = strong_divergences.filter(d => 
    d.timestamp > now() - days(7)
);

if (recent_div.length > 0) {
    alert("Strong divergence detected between AAPL and GOOGL");
}
```

### Divergence Patterns
```shape
// Detect specific divergence patterns
pattern bullish_divergence {
    // Price making lower lows while indicator makes higher lows
    let divs = find_divergences($symbol1, $symbol2, 20);
    
    divs.length > 0 && 
    divs[0].symbol1_trend < 0 &&
    divs[0].symbol2_trend > 0
}

// Scan for divergence patterns
data("market_data", {symbols: [tech_stocks.aapl, tech_stocks.googl]}).map(s => s.find("bullish_divergence"));
```

## Spread Trading

### Basic Spread Calculation
```shape
// Calculate spread with fixed ratio
let spread_values = spread(tech_stocks.aapl, tech_stocks.googl, 1.5);

// Statistical properties of spread
let spread_mean = average(spread_values);
let spread_std = stdev(spread_values);
let z_score = (spread_values[spread_values.length - 1] - spread_mean) / spread_std;
```

### Mean Reversion Signals
```shape
// Generate trading signals based on spread
strategy spread_mean_reversion {
    parameters {
        symbol1: "AAPL",
        symbol2: "GOOGL",
        ratio: 1.5,
        z_threshold: 2.0
    }
    
    signals {
        let spread_vals = spread(load(symbol1), load(symbol2), ratio);
        let z = calculate_zscore(spread_vals);
        
        // Enter long when spread is too low
        when (z < -z_threshold) {
            enter_long(symbol1);
            enter_short(symbol2);
        }
        
        // Enter short when spread is too high
        when (z > z_threshold) {
            enter_short(symbol1);
            enter_long(symbol2);
        }
        
        // Exit when spread returns to mean
        when (abs(z) < 0.5) {
            exit_all();
        }
    }
}
```

## Portfolio Analysis

### Sector Correlation Matrix
```shape
// Analyze sector correlations
let sectors = {
    tech: load_csv("data/XLK_1d.csv", "XLK", "1d"),
    finance: load_csv("data/XLF_1d.csv", "XLF", "1d"),
    energy: load_csv("data/XLE_1d.csv", "XLE", "1d"),
    healthcare: load_csv("data/XLV_1d.csv", "XLV", "1d"),
    consumer: load_csv("data/XLY_1d.csv", "XLY", "1d")
};

// Build correlation matrix
let sector_correlations = {};
for (let [name1, data1] of Object.entries(sectors)) {
    sector_correlations[name1] = {};
    for (let [name2, data2] of Object.entries(sectors)) {
        sector_correlations[name1][name2] = correlation(data1, data2);
    }
}

// Find least correlated sectors for diversification
let min_corr = 1.0;
let best_pair = null;
for (let [s1, corrs] of Object.entries(sector_correlations)) {
    for (let [s2, corr] of Object.entries(corrs)) {
        if (s1 != s2 && corr < min_corr) {
            min_corr = corr;
            best_pair = [s1, s2];
        }
    }
}
```

### Market Breadth Analysis
```shape
// Analyze market breadth using multiple indices
let market_indices = align_symbols([
    load_csv("data/SPY_1d.csv", "SPY", "1d"),
    load_csv("data/QQQ_1d.csv", "QQQ", "1d"),
    load_csv("data/IWM_1d.csv", "IWM", "1d"),
    load_csv("data/DIA_1d.csv", "DIA", "1d")
], "intersection");

// Count how many indices are above their moving averages
let breadth_score = 0;
for (let index_data of market_indices.data) {
    let prices = index_data.map(c => c.close);
    let ma20 = sma(prices, 20);
    
    if (prices[prices.length - 1] > ma20[ma20.length - 1]) {
        breadth_score += 1;
    }
}

let breadth_pct = (breadth_score / market_indices.symbols.length) * 100;
print("Market breadth: " + breadth_pct + "% of indices above MA20");
```

## Real-Time Multi-Symbol Monitoring

### Correlation Alerts
```shape
// Monitor correlation changes in real-time
stream correlation_monitor {
    symbols: ["AAPL", "GOOGL", "MSFT"],
    interval: "5m",
    
    init {
        let baseline_corr = {};
        for (let i = 0; i < symbols.length; i++) {
            for (let j = i + 1; j < symbols.length; j++) {
                let key = symbols[i] + "_" + symbols[j];
                baseline_corr[key] = correlation(
                    load(symbols[i]), 
                    load(symbols[j])
                );
            }
        }
    }
    
    on_tick {
        // Recalculate correlations
        for (let i = 0; i < symbols.length; i++) {
            for (let j = i + 1; j < symbols.length; j++) {
                let key = symbols[i] + "_" + symbols[j];
                let current_corr = correlation(
                    load(symbols[i]), 
                    load(symbols[j])
                );
                
                // Alert on significant correlation changes
                if (abs(current_corr - baseline_corr[key]) > 0.2) {
                    alert("Correlation shift: " + key + 
                          " from " + baseline_corr[key] + 
                          " to " + current_corr);
                }
            }
        }
    }
}
```

### Divergence Scanner
```shape
// Scan multiple pairs for divergences
let pairs_to_scan = [
    ["AAPL", "MSFT"],
    ["GOOGL", "META"],
    ["AMZN", "NFLX"],
    ["JPM", "GS"],
    ["XOM", "CVX"]
];

let active_divergences = [];

for (let [sym1, sym2] of pairs_to_scan) {
    let data1 = load_csv(`data/${sym1}_1h.csv`, sym1, "1h");
    let data2 = load_csv(`data/${sym2}_1h.csv`, sym2, "1h");
    
    let divs = find_divergences(data1, data2, 20);
    
    if (divs.length > 0) {
        let latest_div = divs[divs.length - 1];
        active_divergences.push({
            pair: sym1 + "/" + sym2,
            timestamp: latest_div.timestamp,
            strength: latest_div.strength,
            direction: latest_div.symbol1_trend > 0 ? "bullish" : "bearish"
        });
    }
}

// Sort by strength
active_divergences.sort((a, b) => b.strength - a.strength);

// Display top divergences
print("Top Active Divergences:");
for (let i = 0; i < min(5, active_divergences.length); i++) {
    let div = active_divergences[i];
    print(`${div.pair}: ${div.direction} divergence, strength ${div.strength}`);
}
```

## Advanced Applications

### Cointegration Testing
```shape
// Test for cointegration between pairs
function test_cointegration(data1, data2, lookback) {
    // Calculate spread for different ratios
    let ratios = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
    let best_ratio = 1.0;
    let min_variance = Infinity;
    
    for (let ratio of ratios) {
        let spread_vals = spread(data1, data2, ratio);
        let variance = stdev(spread_vals.slice(-lookback));
        
        if (variance < min_variance) {
            min_variance = variance;
            best_ratio = ratio;
        }
    }
    
    // Test stationarity of optimal spread
    let optimal_spread = spread(data1, data2, best_ratio);
    let adf_stat = adf_test(optimal_spread); // Augmented Dickey-Fuller test
    
    return {
        ratio: best_ratio,
        variance: min_variance,
        is_stationary: adf_stat < -2.86, // 5% critical value
        adf_statistic: adf_stat
    };
}
```

### Multi-Symbol Pattern Recognition
```shape
// Find correlated pattern occurrences
pattern synchronized_breakout {
    // Multiple symbols breaking out simultaneously
    let symbols = ["AAPL", "GOOGL", "MSFT"];
    let breakout_count = 0;
    
    for (let sym of symbols) {
        let data = load(sym);
        let high_20 = highest(data.high, 20);
        
        if (data.close > high_20 * 1.02) {
            breakout_count += 1;
        }
    }
    
    // Trigger when majority break out
    breakout_count >= symbols.length * 0.6
}
```

These examples demonstrate the power of Shape's multi-symbol analysis capabilities for:
- Correlation analysis and monitoring
- Divergence detection and trading
- Spread calculation and mean reversion
- Portfolio diversification
- Market breadth analysis
- Real-time multi-symbol monitoring
- Statistical arbitrage strategies