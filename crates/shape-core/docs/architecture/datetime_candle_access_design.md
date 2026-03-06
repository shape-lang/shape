# DateTime-Based Candle Access and On-Demand Loading Design

## Current Problems

1. **Data Loading Bug**: REPL loads all cached data regardless of requested date range
2. **Relative Indexing**: `candle[0]` is meaningless without context
3. **No DateTime Access**: Can't do `candle[@"2024-01-15 09:30:00"]`
4. **Timeframe Ambiguity**: Unclear which timeframe is active
5. **Memory Waste**: Loading millions of candles when only hundreds needed

## Proposed Solution

### 1. DateTime as First-Class Citizen

```shape
# Direct datetime access
candle[@"2024-01-15 09:30:00"]
candle[@"2024-01-15"]  # Defaults to market open
candle[@now]            # Current/latest candle
candle[@today]          # Today's open
candle[@yesterday]      # Yesterday's open

# Relative to datetime
candle[@"2024-01-15" - 1]  # One candle before
candle[@now - 5]            # 5 candles ago

# Range access
candles[@"2024-01-15 09:30:00" : @"2024-01-15 10:00:00"]
candles[@yesterday : @now]
```

### 2. Timeframe-Aware Access

```shape
# Explicit timeframe
candle[@"2024-01-15 09:30:00", 5m]  # 5-minute candle at this time
candle[@"2024-01-15", 1d]           # Daily candle

# Set working timeframe
use timeframe 15m;
candle[@now]  # Uses 15m timeframe

# Multi-timeframe
let daily = candle[@today, 1d];
let hourly = candle[@today, 1h];
```

### 3. Context-Aware Indexing

```shape
# In pattern/query context - relative to current position
pattern hammer {
    candle[0].body < candle[0].range * 0.3  # Current candle being tested
}

# In module-scope context - needs datetime
let current_price = candle[@now].close;
let yesterday_close = candle[@yesterday, 1d].close;

# Explicit iteration context
for each candle in candles[@"2024-01-01" : @"2024-12-31"] {
    # Here candle[0] means current iteration candle
    # candle[-1] means previous in iteration
}
```

### 4. On-Demand Loading Implementation

```rust
pub struct DataManager {
    /// Data source configurations
    sources: HashMap<String, DataSourceConfig>,
    
    /// Loaded data segments
    segments: BTreeMap<(String, Timeframe), DataSegments>,
    
    /// Loading strategy
    strategy: LoadingStrategy,
}

pub struct DataSourceConfig {
    path: PathBuf,
    symbol: String,
    loader_type: LoaderType,
    available_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
}

pub struct DataSegments {
    /// Ordered by time
    segments: Vec<Segment>,
}

pub struct Segment {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    candles: Vec<Candle>,
}

impl DataManager {
    /// Get candle at specific datetime
    pub fn get_candle_at(
        &mut self,
        symbol: &str,
        datetime: DateTime<Utc>,
        timeframe: Timeframe,
    ) -> Result<&Candle> {
        // Check if we have this data
        if !self.has_data_at(symbol, datetime, timeframe) {
            // Load segment containing this datetime
            self.load_segment_for(symbol, datetime, timeframe)?;
        }
        
        // Return the candle
        self.find_candle_at(symbol, datetime, timeframe)
    }
    
    /// Ensure data is available for a range
    pub fn ensure_range_loaded(
        &mut self,
        symbol: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        timeframe: Timeframe,
        warmup: usize,
    ) -> Result<()> {
        // Calculate actual start with warmup
        let actual_start = self.calculate_start_with_warmup(start, timeframe, warmup);
        
        // Load only missing segments
        let missing = self.find_missing_segments(symbol, actual_start, end, timeframe);
        
        for (seg_start, seg_end) in missing {
            self.load_segment(symbol, seg_start, seg_end, timeframe)?;
        }
        
        Ok(())
    }
}
```

### 5. Smart Loading Strategies

```rust
pub enum LoadingStrategy {
    /// Load fixed-size chunks
    FixedChunks { size: Duration },
    
    /// Load by calendar periods
    CalendarBased { period: CalendarPeriod },
    
    /// Load based on memory constraints
    MemoryConstrained { max_candles: usize },
    
    /// Custom strategy
    Custom(Box<dyn LoadStrategy>),
}

pub enum CalendarPeriod {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}
```

### 6. Updated REPL Commands

```shape
# Simple registration - no immediate loading
:data /path/to/data ES

# With initial window
:data /path/to/data ES --window 1000

# With date hint
:data /path/to/data ES --from 2024-01-01

# Set loading strategy
:data config --chunk-size 1d
:data config --max-memory 1GB

# Info about loaded data
:data info
```

### 7. Execution Context Updates

```rust
pub struct ExecutionContext {
    /// Current execution position (for patterns)
    current_position: Option<ExecutionPosition>,
    
    /// Data manager
    data_manager: DataManager,
    
    /// Active timeframe
    active_timeframe: Timeframe,
}

pub enum ExecutionPosition {
    /// Iterating at specific datetime
    DateTime(DateTime<Utc>),
    
    /// Iterating at index in loaded data
    Index(usize),
    
    /// Not in iteration context
    Global,
}

impl ExecutionContext {
    /// Get candle with proper context handling
    pub fn get_candle(&mut self, reference: &CandleReference) -> Result<&Candle> {
        match reference {
            CandleReference::Index(idx) => {
                match self.current_position {
                    Some(ExecutionPosition::DateTime(dt)) => {
                        // Relative to current datetime
                        let target = self.offset_datetime(dt, *idx, self.active_timeframe)?;
                        self.data_manager.get_candle_at(&self.symbol, target, self.active_timeframe)
                    }
                    Some(ExecutionPosition::Index(pos)) => {
                        // Relative to current index
                        let target_idx = (*pos as i32 + idx) as usize;
                        self.get_candle_at_index(target_idx)
                    }
                    None => {
                        return Err(ShapeError::RuntimeError {
                            message: "candle[index] requires execution context. Use candle[@datetime] instead.".to_string(),
                            location: None,
                        });
                    }
                }
            }
            CandleReference::DateTime(dt) => {
                // Absolute datetime reference
                self.data_manager.get_candle_at(&self.symbol, *dt, self.active_timeframe)
            }
            CandleReference::Named(name) => {
                match name.as_str() {
                    "now" => self.get_latest_candle(),
                    "today" => self.get_candle_at_date(Utc::today()),
                    "yesterday" => self.get_candle_at_date(Utc::today() - Duration::days(1)),
                    _ => Err(ShapeError::RuntimeError {
                        message: format!("Unknown named reference: {}", name),
                        location: None,
                    }),
                }
            }
        }
    }
}
```

### 8. Benefits

1. **Clear Semantics**: `candle[0]` only works in iteration, `candle[@datetime]` works everywhere
2. **Efficient Loading**: Only loads data actually needed
3. **Timeframe Clarity**: Always explicit or clearly defined
4. **Memory Friendly**: Can analyze years of data without loading it all
5. **Real-time Ready**: `candle[@now]` for live trading

### 9. Migration Examples

```shape
# Old (loads all data)
:data /path/to/data ES 2020-01-01 2024-12-31
find candles where close > sma(50)

# New (loads on demand)
:data /path/to/data ES
find candles[@"2020-01-01" : @"2024-12-31"] where close > sma(50)
# Only loads data as needed during iteration

# Old (ambiguous reference)
let price = candle[0].close  # Which candle?

# New (explicit)
let price = candle[@now].close  # Latest candle
let price = candle[@"2024-01-15 09:30:00"].close  # Specific time
```
