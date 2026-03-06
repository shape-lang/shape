# On-Demand Data Loading Design

## Overview

Enable Shape to load market data on-demand based on actual usage patterns rather than requiring users to specify exact date ranges upfront.

## Current Limitations

1. Must specify exact date range: `:data /path ES 2020-01-01 2020-12-31`
2. Loads all data upfront (memory intensive)
3. No way to extend data range during analysis
4. Users must guess how much historical data they need

## Proposed Solution

### 1. Lazy Data Source Registration

```shape
# Register data source without loading
:data /home/amd/dev/finance/data ES

# Or with initial window hint
:data /home/amd/dev/finance/data ES --initial-window 100
```

### 2. Smart Data Loading

The system tracks:
- What data is currently loaded (date ranges)
- What data source can provide more
- Warmup requirements from indicators

When executing:
```shape
# This needs 200 candles before current position
find candles where close > sma(200)
```

The system:
1. Checks if enough data is loaded
2. If not, loads additional data from source
3. Caches loaded data for future use

### 3. Implementation Components

#### DataSourceManager
```rust
pub struct DataSourceManager {
    /// Registered data sources
    sources: HashMap<String, DataSource>,
    
    /// Currently loaded data ranges
    loaded_ranges: HashMap<String, Vec<DateRange>>,
    
    /// Cache of loaded data
    data_cache: DataCache,
}

impl DataSourceManager {
    /// Register a data source without loading
    pub fn register_source(&mut self, path: &Path, symbol: &str) -> Result<()> {
        let source = DataSource::new(path, symbol)?;
        self.sources.insert(symbol.to_string(), source);
        Ok(())
    }
    
    /// Ensure data is available for given requirements
    pub fn ensure_data_available(
        &mut self, 
        symbol: &str,
        end_date: DateTime<Utc>,
        required_history: usize
    ) -> Result<()> {
        let start_date = self.calculate_start_date(end_date, required_history)?;
        
        if !self.is_data_loaded(symbol, start_date, end_date) {
            self.load_data_range(symbol, start_date, end_date)?;
        }
        
        Ok(())
    }
}
```

#### Integration with ExecutionContext
```rust
impl ExecutionContext {
    /// Get candle with automatic data loading
    pub fn get_candle_with_warmup(&mut self, index: i32, warmup: usize) -> Result<&Candle> {
        // Calculate total data needed
        let total_needed = (index.abs() as usize) + warmup;
        
        // Ensure data is available
        self.data_manager.ensure_data_available(
            &self.symbol,
            self.current_date,
            total_needed
        )?;
        
        // Return the candle
        self.get_candle(index)
    }
}
```

#### Warmup-Aware Query Execution
```rust
impl QueryExecutor {
    fn execute_find(&mut self, pattern: &Pattern, conditions: &[Condition]) -> Result<Vec<Match>> {
        // Calculate required warmup from all indicators used
        let total_warmup = self.calculate_total_warmup(pattern, conditions)?;
        
        // Start iteration from first valid position
        let start_index = total_warmup;
        
        for i in start_index..self.available_candles() {
            // Pattern matching with guaranteed data availability
        }
    }
}
```

### 4. Benefits

1. **User-Friendly**: No need to calculate date ranges
2. **Memory Efficient**: Only loads required data
3. **Flexible**: Can extend analysis without restarting
4. **Smart**: Automatically handles indicator warmup
5. **Fast**: Caches loaded data

### 5. Example Usage

```shape
# Register data source
:data /home/amd/dev/finance/data ES

# Simple query - loads minimal data
candle[0].close  # Loads just recent data

# Complex query - loads more as needed
find candles where close > sma(200) and atr(14) > 20
# Automatically loads 201 candles (200 + 1 for safety)

# Extending analysis - loads more data seamlessly
for i in range(0, 1000) {
    let ma = sma(50);  # Loads more data as loop progresses
}
```

### 6. Configuration Options

```shape
# Set loading preferences
:config data.chunk_size 1000        # Load in 1000-candle chunks
:config data.cache_size 1000000     # Cache up to 1M candles
:config data.preload_warmup true    # Preload warmup data

# Or in CLAUDE.md
[data_loading]
chunk_size = 1000
cache_size = 1000000
preload_warmup = true
```

### 7. Implementation Priority

1. **Phase 1**: Basic lazy loading
   - Register sources without loading
   - Load on first access
   - Simple date-based caching

2. **Phase 2**: Smart warmup integration
   - Calculate warmup from queries
   - Preload required data
   - Optimize chunked loading

3. **Phase 3**: Advanced features
   - Multi-source management
   - Distributed caching
   - Streaming updates