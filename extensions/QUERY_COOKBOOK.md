# Extension Query Cookbook

How to implement `Queryable` for a new data source extension.

## Architecture

```
Queryable trait (stdlib/core/queryable.shape)
    |
    +-- Table           (in-memory, SIMD-optimized, native PHF methods)
    +-- DuckDbQuery     (lazy SQL builder -> duckdb.execute_sql)
    +-- PgQuery         (lazy SQL builder -> postgres.execute_sql)
    +-- ApiQuery        (lazy param builder -> openapi.execute_request)
```

Each extension follows the same layered pattern:

1. **Rust crate** (`shape/extensions/<name>/`) - module functions as `ModuleExports`
2. **Bundled Shape file** (`<name>.shape`) - `impl Queryable for <QueryType>` + `extend` block
3. **CLI registration** (`shape-cli/src/extensions.rs`) - feature-gated `register_extension()`

## The Proxy Pattern for Filter Pushdown

Filter pushdown converts Shape lambdas into backend-native filters (SQL WHERE, query params).

### How it works

```
.filter(|u| u.age >= 18)
        |
        v
1. make_proxy(columns)  ->  { age: ExprProxy("age"), name: ExprProxy("name"), ... }
2. predicate(proxy)      ->  ExprProxy("age") >= 18  ->  FilterExpr(Compare{age, Gte, 18})
3. Accumulate FilterExpr into query plan
4. On execute():
   - SQL backends:  filter_to_sql(expr)  ->  "age >= 18"
   - API backends:  filter_to_params(expr)  ->  { age_gte: "18" }
```

The key types from `shape-value`:
- `VMValue::ExprProxy(Arc<String>)` - placeholder for a column/field name
- `VMValue::FilterExpr(Arc<FilterNode>)` - comparison tree from proxy operations
- `FilterNode::Compare { column, op, value }` - leaf comparison
- `FilterNode::And/Or/Not` - logical combinators

## Required Native Functions

Every Queryable extension needs these native primitives:

| Function | Signature | Purpose |
|----------|-----------|---------|
| `make_proxy` | `(Array<string>) -> object` | Create ExprProxy map for lambda analysis |
| `column_name` | `(ExprProxy) -> string` | Extract column name from proxy |
| `filter_to_*` | `(FilterExpr) -> ...` | Convert filter tree to backend format |
| `execute_*` | `(uri, query) -> Table` | Run the actual query |

SQL backends use `filter_to_sql()` from `shape_runtime::query_builder`.
API backends implement their own `filter_to_params()`.

## Query Object Shape

Each extension builds a query object with these fields:

| Field | DuckDB/Postgres | OpenAPI | Purpose |
|-------|-----------------|---------|---------|
| `__type` | `"DuckDbQuery"` / `"PgQuery"` | `"ApiQuery"` | Type tag for dispatch |
| columns/fields | `columns: Array<string>` | `fields: Array<string>` | Known column/field names |
| `filters` | `Array<FilterExpr>` | `Array<FilterExpr>` | Accumulated filter expressions |
| `projections` | `Array<string>` | `Array<string>` | Selected columns |
| order | `order_by_cols` | `order_by` | Sort specifications |
| `limit_val` | `int \| None` | `int \| None` | Row limit |

Note: The column list field name differs (`columns` vs `fields`) and the order field
name differs (`order_by_cols` vs `order_by`). This is fine since Shape code in each
extension's `.shape` file references the correct field names. The Queryable trait
abstracts over these differences.

## Shape File Template

```shape
// Public API
pub fn connect(uri) { myext.connect(uri) }

pub enum Order { Asc, Desc }

impl Queryable for MyQuery {
    method filter(predicate) {
        let proxy = myext.make_proxy(this.columns)
        let result = predicate(proxy)
        { ...this, filters: this.filters.concat([result]) }
    }

    method select(cols) {
        { ...this, projections: cols }
    }

    method orderBy(key_fn, direction) {
        let proxy = myext.make_proxy(this.columns)
        let key = key_fn(proxy)
        let col_name = myext.column_name(key)
        let dir_str = match direction {
            Order::Asc => "ASC",
            Order::Desc => "DESC"
        }
        { ...this, order_by_cols: this.order_by_cols.concat([...]) }
    }

    method limit(n) {
        { ...this, limit_val: n }
    }

    method execute() {
        // Backend-specific: build SQL, params, etc.
        let query = this.build_query()
        myext.execute(this.uri, query)
    }
}
```

## Rust Crate Template

```rust
use shape_runtime::module_exports::{ModuleFunction, ModuleExports, ModuleParam};
use shape_value::NanBoxed;

pub fn create_module() -> ModuleExports {
    let mut module = ModuleExports::new("myext");
    module.add_shape_source("myext.shape", include_str!("myext.shape"));

    // Required: proxy/filter primitives (sync)
    module.add_function_with_schema("make_proxy", make_proxy, ...);
    module.add_function_with_schema("column_name", column_name, ...);
    module.add_function_with_schema("filter_to_...", filter_to, ...);

    // Required: execution (sync or async)
    module.add_function_with_schema("execute_...", execute, ...);
    // OR for async:
    module.add_async_function_with_schema("execute_...", execute_async, ...);

    // Optional: connect for schema discovery
    module.add_function_with_schema("connect", connect, ...);

    module
}
```

## CLI Registration

In `shape-cli/Cargo.toml`:
```toml
[features]
ext-myext = ["shape-ext-myext"]

[dependencies]
shape-ext-myext = { path = "../extensions/myext", optional = true }
```

In `shape-cli/src/extensions.rs`:
```rust
#[cfg(feature = "ext-myext")]
{
    executor.register_extension(shape_ext_myext::create_module());
}
```

## Audit Summary (2026-02-12)

All four Queryable implementations verified:

| Implementation | Type | Proxy | Filter Pushdown | Tests |
|----------------|------|-------|-----------------|-------|
| Table | In-memory | N/A (native) | N/A (native) | stdlib |
| DuckDbQuery | SQL (sync) | make_proxy -> filter_to_sql | WHERE clause | 24 pass |
| PgQuery | SQL (async) | make_proxy -> filter_to_sql | WHERE clause | 16 pass |
| ApiQuery | HTTP (async) | make_proxy -> filter_to_params | Query params | 18 pass |

Pattern consistency:
- All SQL backends share `shape_runtime::query_builder::filter_to_sql()`
- All extensions use the same ExprProxy -> FilterExpr -> backend conversion pipeline
- All extensions bundle a `.shape` file with `impl Queryable` + `extend` block
- All query objects use immutable spread (`{ ...this, field: newval }`) for method chaining
- All extensions register via feature-gated `register_extension()` in CLI
- Database extensions share `DataSourceSchemaCache` from `shape_runtime::schema_cache`
