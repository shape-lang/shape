# Data Source Configuration

## Overview

Shape supports flexible data source configuration via the `configure_data_source()` function, allowing you to select between DuckDB and file-based storage backends.

## Syntax

```cql
configure_data_source({
    backend: "duckdb" | "file",
    db_path: "path/to/database.duckdb",     // For DuckDB backend
    data_dir: "path/to/data/directory"      // For file backend
})
```

## Backend Types

### DuckDB Backend (Default)

Recommended for production use with large datasets.

**Features:**
- SQL-based time-series storage
- Efficient querying and aggregation
- Compression and indexing
- Git LFS support for large files

**Configuration:**
```cql
configure_data_source({
    backend: "duckdb",
    db_path: "market_data.duckdb"  // Relative or absolute path
});
```

**Environment Variable Fallback:**
If `db_path` is not specified, uses `MARKET_DATA_DB` environment variable or defaults to `market_data.duckdb` in current directory.

### File Backend

Uses memory-mapped binary files for zero-copy access.

**Configuration:**
```cql
configure_data_source({
    backend: "file",
    data_dir: "./data"  // Directory containing market data files
});
```

## Usage Examples

### Basic Usage

```cql
// Configure DuckDB data source
configure_data_source({
    backend: "duckdb",
    db_path: "market_data.duckdb"
});

// Load instrument data
load_instrument("ES", "2023-01-01", "2023-12-31");

// Run analysis
let closes = series("close");
let sma_50 = sma(closes, 50);
```

### Custom Database Path

```cql
configure_data_source({
    backend: "duckdb",
    db_path: "/absolute/path/to/prod_data.duckdb"
});
```

### File Backend Example

```cql
configure_data_source({
    backend: "file",
    data_dir: "~/trading/historical_data"
});

load_instrument("AAPL", "2020-01-01", "2024-12-31");
```

## Data Source Lifecycle

1. **Initialization:** Call `configure_data_source()` once at the beginning of your script
2. **Loading:** Use `load_instrument()` to load specific symbols and date ranges
3. **Access:** Data automatically available via `series()`, `candle[]`, etc.
4. **Reconfiguration:** Call `configure_data_source()` again to switch backends

## Implementation Details

**Location:** `src/runtime/evaluation/functions/series.rs:426-527`

The function:
- Creates a `DataProviderBuilder` with specified backend type
- Configures DuckDB or file storage parameters
- Builds and installs the provider in the execution context
- All subsequent data operations use this provider

## Default Behavior

If `configure_data_source()` is not called:
- Uses DuckDB backend by default
- Checks `MARKET_DATA_DB` environment variable
- Falls back to `market_data.duckdb` in current directory

## Git LFS Integration

For large DuckDB files (>100MB):

```bash
# Initialize Git LFS
git lfs install

# Track DuckDB files
git lfs track "*.duckdb"

# Pull LFS files
git lfs pull
```

Configuration in `devenv.nix` ensures git-lfs is available in development environment.

## Error Handling

**Common Errors:**

1. **Database not found:**
   ```
   Error: Failed to create DataProvider: Failed to open DuckDB connection
   ```
   **Solution:** Check file path, run `git lfs pull` if using LFS

2. **Invalid backend:**
   ```
   Error: Unknown backend 'redis'. Valid options: 'duckdb', 'file'
   ```
   **Solution:** Use only 'duckdb' or 'file' as backend value

3. **Permission denied:**
   ```
   Error: Failed to read module file: Permission denied
   ```
   **Solution:** Check file permissions on data directory/database

## See Also

- `docs/market-data-loading.md` - Detailed guide on loading market data
- `docs/INSTRUMENT_DATA_LOADING.md` - Instrument loading reference
- `docs/warmup_system_implementation.md` - How warmup affects data loading
