# Instrument Data Loading Architecture

## Overview

The data loading system in Shape is built on top of the `market-data` crate, which provides a sophisticated trait-based architecture for loading various market data types.

## Key Features

### 1. Recursive Directory Scanning

When a directory path is specified, the system automatically:
- Recursively scans all subdirectories using `walkdir`
- Finds all files matching the expected patterns
- Groups files by contract/symbol
- Handles complex folder structures (e.g., year/month organization)

```rust
// From market-data/src/loaders/mod.rs
pub fn file_paths(&self) -> Result<Vec<PathBuf>> {
    match self {
        Self::Path(p) => {
            if p.is_file() {
                Ok(vec![p.clone()])
            } else if p.is_dir() {
                // Recursively find all files
                let mut files = Vec::new();
                for entry in walkdir::WalkDir::new(p)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_type().is_file() {
                        files.push(entry.path().to_path_buf());
                    }
                }
                Ok(files)
            }
        }
    }
}
```

### 2. Multi-Instrument Detection in CSV Files

The CSV loader automatically detects and handles multiple instruments within a single file:
- Groups candles by symbol column
- Creates separate `CandleData` for each symbol
- Supports files with mixed contracts (e.g., ESH4, ESM4 in same file)

```rust
// From market-data/src/loaders/formats.rs
// Group candles by symbol
let mut data_by_symbol: HashMap<String, Vec<Candle>> = HashMap::new();

for result in reader.records() {
    let symbol = if let Some(idx) = col_indices.symbol {
        record.get(idx).unwrap_or("UNKNOWN").to_string()
    } else if let Some(ref sym) = metadata.symbol {
        sym.clone()
    } else {
        "UNKNOWN".to_string()
    };
    
    data_by_symbol
        .entry(symbol)
        .or_insert_with(Vec::new)
        .push(candle);
}
```

### 3. Automatic Contract Rollover for Futures

When loading futures data, the system:
- Automatically parses contract symbols (e.g., ESH4 → ES + March 2024)
- Groups data by contract
- Detects rollover points using volume-based analysis
- Builds continuous contracts automatically
- Supports various rollover strategies:
  - Volume-based (default)
  - Days before expiry
  - Fixed date rules

```rust
// From market-data/src/loaders/futures.rs
// Build continuous contract
if config.save_continuous && !merged_contracts.is_empty() {
    let rollover_manager = RolloverManager::new(config.rollover_strategy);
    let continuous = rollover_manager.build_continuous(
        merged_contracts.clone(),
        &actual_base_symbol,
        &config.timeframe
    )?;
}
```

### 4. Data Loading Methods in Shape

Shape provides several ways to load data:

#### Single CSV File
```shape
load_instrument("ES", "/path/to/data.csv")
```

#### Directory (Recursive)
```shape
load_instrument("ES")  // Uses default path: ~/dev/finance/data/ES/
```

#### With Custom Path
```shape
load_instrument("ES", "/custom/path/to/ES/data/")
```

### 5. Lazy Loading and Caching

- Data is NOT loaded when instrument is registered
- Data loads on first access (`get_candles()`)
- Aggregated timeframes are cached
- Each instrument maintains its own cache

## Example Data Structure

The system expects data organized like:
```
~/dev/finance/data/
├── ES/
│   ├── 2024/
│   │   ├── 01/
│   │   │   ├── glbx-mdp3-20240101.ohlcv-1m.csv
│   │   │   ├── glbx-mdp3-20240102.ohlcv-1m.csv
│   │   │   └── ...
│   │   └── 02/
│   │       └── ...
│   └── 2025/
│       └── ...
└── NQ/
    └── ...
```

## CSV Format Support

The system auto-detects CSV schemas and supports various formats:
- Headers: timestamp, open, high, low, close, volume, symbol
- Timestamp formats: Unix timestamp, ISO8601, custom formats
- Symbol detection: Automatic from "symbol" column or filename
- Multi-contract files: Automatically splits by symbol

## Integration with Shape

```shape
// Initialize instruments (optional - happens automatically)
init_instruments()

// Load futures with automatic rollover
load_instrument("ES")  // Loads all ES contracts, builds continuous

// Load specific file
load_instrument("SPY", "/data/SPY_daily.csv")

// Access data (triggers lazy loading)
set_instrument("ES")
let sma20 = sma(20)  // Data loads here if not already loaded
```

## Benefits

1. **Flexibility**: Single files or entire directory trees
2. **Intelligence**: Automatic contract detection and rollover
3. **Performance**: Lazy loading with caching
4. **Simplicity**: Simple Shape functions hide complexity
5. **Scalability**: Handles large datasets efficiently