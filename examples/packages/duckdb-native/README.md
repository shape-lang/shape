# duckdb-native (PoC)

Pure Shape package that binds DuckDB's C API directly using `extern C`.

## What this demonstrates

- native dependency alias resolution via `[native-dependencies]`
- pointer-cell handling for C out-parameters (`duckdb_open`, `duckdb_connect`, etc.)
- strict schema import into `Result<Table<T>>` using Arrow C pointers

## Usage as dependency

```toml
[dependencies]
duckdb_native = { path = "./shape/examples/packages/duckdb-native" }
```

```shape
from duckdb_native use { connect, close, load_candles }

let s = connect("market_data.duckdb")
let rows = load_candles(
  s,
  "SELECT ts, open, high, low, close, volume FROM candles ORDER BY ts"
)
close(s)
```

`load_candles(...)` returns `Result<Table<CandleRow>, AnyError>`.
