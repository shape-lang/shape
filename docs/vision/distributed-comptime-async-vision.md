# Shape Language Vision: Distributed Computing, Comptime, and Async

> Analysis performed 2026-02-13 by a 3-specialist agent team.
> Updated 2026-02-13 with design decisions from architecture review.

## Design Decisions (Final)

### Three Primitives — The Complete Set

| Concept | What it does |
|---------|-------------|
| `trait` | Define behavioral contracts |
| `@annotation` | Attach compile-time and runtime behavior to **any target** (functions, types, expressions, await, blocks, bindings) |
| `comptime { }` | Standalone compile-time execution |

These three cover everything. No other metaprogramming concepts needed.

### `meta {}` is REMOVED

`meta` is replaced by:
- **`comptime` fields** on types for parameterization (symbol, decimals, etc.)
- **`Display` trait** (and other traits) for formatting behavior

```shape
// OLD (removed):
// meta Currency { symbol: "$"; decimals: 2; format: ... }

// NEW:
type Currency {
    comptime symbol: string = "$"
    comptime decimals: number = 2
}

trait Display {
    display(): string
}

impl Display for Currency {
    display() { this.symbol + round(this, this.decimals) }
}

// Type aliases with comptime parameter overrides
type EUR = Currency { symbol: "€" }
type JPY = Currency { symbol: "¥", decimals: 0 }
```

Comptime fields are **type-level constants**, baked in at compile time, zero runtime cost.
`type EUR = Currency { symbol: "€" }` means every EUR always has `symbol: "€"` — enforced by the compiler.

### `@` Means Annotation — Always and Only

**`@` is exclusively for annotations.** It is NOT used for comptime builtins.

`@` attaches to **the syntactic element immediately following it**. Always. Targets:

```shape
@cached fn compute() { ... }                    // target: function
@derive_debug type Candle { ... }               // target: type
let x = @timed heavy_computation()              // target: expression
let x = @checkpoint { step_a(); step_b() }      // target: block
let x = await @remote(gpu) computation()        // target: await_expr
@logged let x = compute()                       // target: binding
```

Annotations compose left-to-right as wrappers:
```shape
let x = await @retry(3) @timeout(5.seconds) fetch_prices()
// means: retry(timeout(fetch_prices()))
```

Annotations declare valid targets:
```shape
annotation checkpoint() {
    targets: [block, await_expr]
    comptime(target) { ... }
}
```

### Comptime Builtins Are Just Functions

There is NO `@typeInfo`, `@Type`, `@compileError` syntax. Comptime builtins are regular
functions that are only available inside comptime context:

```shape
comptime {
    let info = type_info(Currency)    // just a function
    let config = build_config()       // just a function
    warning("careful here")           // just a function
}
```

Calling these outside comptime → compile error. No special syntax needed.

### Comptime Code Generation — Just Write Code

Comptime does NOT use special APIs like `inject_method()` or `inject_type()`.
Instead, you write **normal Shape code** inside comptime blocks. This ensures full LSP
support (autocomplete, go-to-definition, rename, etc.).

**Injecting methods — use `extend target`:**
```shape
annotation serializable() {
    comptime(target) {
        extend target {
            fn to_bytes(self) -> Array<byte> { ... }
            fn from_bytes(bytes: Array<byte>) -> Self { ... }
        }
    }
}
```

**Injecting types — just write the type:**
```shape
comptime {
    type CacheKey {
        symbol: string,
        timeframe: string
    }
}
```

**Conditional compilation — `remove target`:**
```shape
annotation when(condition: bool) {
    comptime(target) {
        if not condition {
            remove target
        }
    }
}
```

**Reflective code generation — `comptime for`:**

For cases where generated code depends on the target's structure (e.g., iterating over
fields), use `comptime for` inside extend blocks. The loop body is unrolled at compile time:

```shape
annotation derive_debug() {
    comptime(target) {
        extend target {
            fn debug_string(self) -> string {
                let parts = []
                comptime for field in target.fields {
                    // Unrolled once per field at compile time
                    // field.name is a comptime constant → resolved to direct slot access
                    parts.push(field.name + ": " + self[field.name].display())
                }
                return target.name + "{" + parts.join(", ") + "}"
            }
        }
    }
}

// Applied to: @derive_debug type Candle { open, high, low, close, volume }
// Compiler unrolls to:
//   parts.push("open: " + self.open.display())
//   parts.push("high: " + self.high.display())
//   parts.push("low: " + self.low.display())
//   parts.push("close: " + self.close.display())
//   parts.push("volume: " + self.volume.display())
// Zero runtime reflection. Direct slot access. Extremely fast.
```

**Summary — no special APIs to learn:**

| ~~Old (API-based)~~ | New (just write code) |
|---|---|
| ~~`inject_method(target, "foo", fn() { ... })`~~ | `extend target { fn foo() { ... } }` |
| ~~`inject_type("Foo", { fields: [...] })`~~ | `type Foo { ... }` |
| ~~`remove_target(target)`~~ | `remove target` |
| ~~`emit_warning("msg")`~~ | `warning("msg")` |
| ~~`set_metadata(target, "key", val)`~~ | `target.metadata.key = val` |

One new primitive: **`comptime for`** for compile-time unrolling. Everything else is
existing Shape syntax reused inside comptime blocks.

### Distribution is NOT a Language Primitive

Distribution is achieved via user-defined `@annotations`, not keywords:
- No `remote {}` keyword
- No `teleport` keyword
- No `distribute()` builtin

Instead:
```shape
let x = await @remote(gpu_resolver) computation()
let results = await @distributed join all { a(), b(), c() }
```

Where `@remote` and `@distributed` are user-defined comptime annotations that:
1. At compile time: verify closure captures are Serializable (using `comptime(target)`)
2. At runtime: resolve target, serialize, transfer, execute, return (using `before/after` hooks)

Example — defining `@remote`:
```shape
annotation remote(resolver) {
    targets: [await_expr, block]

    comptime(target) {
        // At compile time: verify captures are serializable
        for capture in target.captures {
            if not implements(capture.type, Serializable) {
                error("Cannot distribute: " + capture.name + " is not Serializable")
            }
        }
    }

    before(args, ctx) {
        // At runtime: resolve the target node (user-defined logic!)
        ctx.target = resolver(ctx)
    }
    // Runtime distribution mechanism handles serialize → transfer → execute → return
}
```

### Async Primitives

The language provides exactly **two** async primitives:

1. **`await`** — the suspension point
2. **`join all|race|any|settle`** — the combinator

Everything else (timeouts, retries, distribution, checkpointing) is `@annotations`:
```shape
// Language primitives
let x = await fetch_prices("AAPL")
let (a, b) = await join all { fetch_a(), fetch_b() }

// Annotations (user-defined, composable)
let x = await @timeout(5.seconds) fetch_prices("AAPL")
let x = await @retry(3) @timeout(5.seconds) fetch_prices("AAPL")
let x = await @remote(gpu) @timeout(30.seconds) @checkpoint monte_carlo(1_000_000)

// Per-item annotations inside joins
let (a, b) = await @timeout(30.seconds) join all {
    @node(find_node("us-east")) compute_a(),
    @node(find_node("eu-west")) compute_b()
}
```

### Core Constraints

- **Extremely fast**: No dynamic dispatch where avoidable. Comptime fields are zero-cost. `comptime for` unrolls to direct slot access.
- **No dynamic types**: Everything resolved at compile time. TypedObjects with registered schemas.
- **Complete compile-time verification**: Trait bounds enforced, distribution safety verified, type mismatches caught.
- **Result types for errors**: No try/catch/throw.
- **LSP-first**: All code generation uses real Shape syntax (`extend`, `type`, `fn`) — never string-based. Full IDE support everywhere.

---
> This document captures the full vision for Shape's comptime unification, trait+async system, and distributed computing capabilities.

---

## Part I: Comptime, Meta{}, and @Annotation Systems

### 1. Zig's Comptime — Why It's Revolutionary

Zig's `comptime` lets you run *ordinary* Zig code during compilation. Key advantages over alternatives:

- **vs C++ templates**: C++ templates are a separate, accidentally-Turing-complete type-level language. Zig uses the *same language* at compile time and runtime.
- **vs Rust macros**: Rust has `macro_rules!` (limited) and proc macros (separate crates, steep learning curve). Zig comptime is just... Zig.
- **vs constexpr/consteval (C++20)**: Restricted subsets that grow each standard. Zig has no such restrictions.

Key capabilities:
1. Types as first-class values (at comptime)
2. `@typeInfo` / `@Type` for type reflection and construction
3. Generic programming through comptime parameters (no angle-bracket generics needed)
4. No I/O at compile time — hermetic, reproducible, cacheable
5. Compile-time code generation — specialized optimized code

### 2. Current State of Shape's Three Systems

#### 2A. The `meta {}` System

Attaches formatting, validation, and presentation logic to types.

```shape
meta Percent {
    decimals: number = 2;
    format: (v) => round(v * 100, this.decimals) + "%"
}

meta Currency {
    symbol: string = "$";
    decimals: number = 2;
    format: (v) => this.symbol + round(v, this.decimals)
}
```

**Strengths:** Clean syntax, parameter defaults, type alias variations, working comptime execution, cache infrastructure, extension support.

**Weaknesses:** Limited to formatting/validation, no type generation, no conditional compilation, no connection to annotations.

#### 2B. The `@annotation` System

Aspect-oriented programming with lifecycle hooks.

```shape
annotation cached() {
    before(args, ctx) {
        let key = hash(fn.name, args);
        let entry = ctx.cache.get_entry(key);
        if entry != null { entry.value }
    }
    after(args, result, ctx) {
        ctx.cache.set(key, result);
        result
    }
    metadata() {
        { cacheable: true, pure: true }
    }
}
```

**Strengths:** Rich lifecycle model, domain-agnostic primitives, defined in Shape stdlib, LSP integration, composable.

**Weaknesses:** Several features TODO (`on_define`, multiple annotation chaining, `ctx.set()` persistence), no compile-time execution, no type introspection.

#### 2C. The Comptime Infrastructure

Executes Shape code at compile time, currently only for meta method bodies.

**State:** Working mini-VM that re-compiles from AST, has extension support and caching, but only used for meta.

### 3. Unification Thesis

The three systems should become:
1. **`comptime` blocks** — general compile-time execution (absorbs meta)
2. **`@annotation` with comptime powers** — annotations that run at compile time

### 4. Proposed Syntax

#### Comptime Blocks

```shape
const TABLE_SIZE = comptime {
    let primes = sieve_of_eratosthenes(1000);
    primes.length
};

comptime fn fibonacci(n: int) -> int {
    if n <= 1 { return n }
    return fibonacci(n - 1) + fibonacci(n - 2)
}
```

#### Type Reflection

```shape
comptime fn describe_type(comptime T: type) -> string {
    let info = @typeInfo(T);
    match info {
        .Struct(s) => {
            let fields = s.fields.map(f => f.name + ": " + f.type_name);
            "struct { " + fields.join(", ") + " }"
        }
        .Enum(e) => "enum with " + e.variants.length + " variants"
        _ => "unknown"
    }
}
```

#### Annotations with Comptime Power

```shape
annotation derive(comptime trait_name: string) {
    comptime(target) {
        let info = @typeInfo(target);
        match trait_name {
            "Debug" => {
                let fields = info.fields;
                let format_body = fields.map(f =>
                    "\"" + f.name + ": \" + self." + f.name + ".display()"
                ).join(" + \", \" + ");
                @inject_method(target, "debug_string", fn(self) -> string {
                    return "{" + ${format_body} + "}"
                });
            }
        }
    }
}
```

#### Conditional Compilation

```shape
annotation when(comptime condition: bool) {
    comptime(target) {
        if not condition { @remove(target); }
    }
}

@when(DEBUG)
fn debug_dump(data: Series) { /* ... */ }
```

#### Finance-Specific Comptime

```shape
annotation indicator(comptime period: int) {
    comptime(target_fn) {
        let body_info = @analyze_function(target_fn);
        let max_lookback = body_info.rolling_calls.map(c => c.window_size).max();
        @set_metadata(target_fn, "warmup_periods", max_lookback);
        if period <= 4 {
            @set_metadata(target_fn, "simd_strategy", "f64x4_inline");
        }
    }
}
```

### 5. Experience Ladder

- **Level 1 (90%):** Use annotations — `@cached`, `@strategy` just work
- **Level 2 (library authors):** Define runtime annotations with before/after hooks
- **Level 3 (framework authors):** Comptime annotations with `@typeInfo`, `@inject_method`
- **Level 4 (language extension designers):** Raw comptime blocks for lookup tables, code gen

---

## Part II: Trait System and Async Capabilities

### 1. Trait System Current State

**AST:** `TraitDef` with name, type_params, extends, members. `ImplBlock` with trait_name, target_type, methods.

**Compiler:** Traits registered in `known_traits` HashSet. Impl blocks desugared to UFCS: `"Type::method"`. No compile-time signature validation.

**Type System:** `TypeConstraint::HasMethod` is a stub (accepts everything). No `ImplementsTrait` constraint.

**Strengths:** Clean familiar syntax, type params, inheritance, UFCS desugaring, meta integration.

**Critical Gaps:**
1. No compile-time trait bounds (`T: Comparable`)
2. No default method implementations
3. No trait objects / dynamic dispatch
4. No where clauses
5. No associated types
6. HasMethod constraint is a stub

### 2. Async Current State

**VM Opcodes:** Yield, Suspend, Resume, Poll, AwaitBar, AwaitTick, Await, EmitAlert, EmitEvent

**Executor:** `AsyncExecutionResult` (Continue/Yielded/Suspended), `WaitType` (NextBar/Timer/AnyEvent/Future)

**Strengths:** Platform-agnostic, cooperative scheduling, domain-focused (AwaitBar/AwaitTick), sync shortcut in Await, event-driven architecture, stream definitions.

**Critical Gaps:**
1. No async join primitives (all/race/any/settle)
2. No structured concurrency
3. No async trait methods
4. No timeout primitives
5. No cancellation tokens
6. No async iteration (`for await`)
7. Future values are opaque (just u64 ID)

### 3. Proposed: `join` Block Expressions

```shape
// ALL
let (prices, options, fundies) = await join all {
    fetch_prices("AAPL"),
    fetch_options("AAPL", expiry: "2026-03"),
    fetch_fundamentals("AAPL")
}

// RACE
let fastest_price = await join race {
    fetch_prices("AAPL", source: "bloomberg"),
    fetch_prices("AAPL", source: "reuters")
}

// ANY (first success)
let best_price = await join any {
    fetch_prices("AAPL", source: "bloomberg"),
    fetch_prices("AAPL", source: "reuters"),
    fetch_prices("AAPL", source: "yahoo")
}

// SETTLE (all complete, errors preserved)
let all_results = await join settle {
    fetch_prices("AAPL"),
    fetch_prices("GOOGL"),
    fetch_prices("MSFT")
}

// TIMEOUT
let data = await join all timeout(5.seconds) {
    fetch_prices("AAPL"),
    fetch_options("AAPL")
}

// NAMED FIELDS
let market = await join all {
    prices: fetch_prices("AAPL"),
    volume: fetch_volume("AAPL"),
    depth:  fetch_order_book("AAPL")
}

// DYNAMIC
let all_prices = await join all symbols.map(|s| fetch_prices(s))
```

### 4. Structured Concurrency

```shape
async scope {
    async let feed_a = subscribe("AAPL")
    async let feed_b = subscribe("GOOGL")

    for await tick in merge(feed_a, feed_b) {
        if tick.price > threshold {
            break  // exits scope, cancels both subscriptions
        }
    }
}
```

### 5. Financial-Domain Joins

```shape
// Time-aligned join
let aligned = await join aligned(on: "timestamp") {
    prices: fetch_ohlcv("AAPL", "1h"),
    sentiment: fetch_news_sentiment("AAPL", "1h")
}

// Stale-aware join
let snapshot = await join latest(max_age: 5.seconds) {
    bid: stream_bid("AAPL"),
    ask: stream_ask("AAPL")
}

// Priority cascade
let data = await cascade {
    fetch_from_cache("AAPL"),
    fetch_from_local_db("AAPL"),
    fetch_from_api("AAPL", "premium"),
    fetch_from_api("AAPL", "free")
}
```

### 6. Async Trait Methods

```shape
trait DataSource<T> {
    async load(query: DataQuery): Result<DataFrame<T>>,
    async subscribe(symbol: string): Stream<T>,
    has_data(symbol: string): bool
}
```

### 7. Implementation Roadmap

- Phase 1: JoinAll/JoinRace/JoinAny/JoinSettle VM opcodes + TaskGroup value type
- Phase 2: JoinExpr AST + parser + compiler
- Phase 3: Trait bounds, async trait methods, ImplementsTrait constraint
- Phase 4: Streams, `for await`, backpressure

---

## Part III: Snapshot System and Distributed Computing Vision

### 1. Current Snapshot Architecture

**Content-addressed, chunked, zstd-compressed binary store.**

Components:
- `SnapshotStore`: Content-addressed blob store, SHA256 keyed, dedup'd, zstd-compressed
- `ExecutionSnapshot`: Envelope with hash pointers to SemanticSnapshot, ContextSnapshot, VmSnapshot, bytecode
- `SerializableVMValue`: 35+ variant serializable mirror of VMValue (closures with upvalues, Arrow IPC DataTables, TypedObjects with heap_mask)
- `ChunkedBlob`: Large data split into content-addressed chunks (enables incremental transfer)

**What it captures:** VM state (IP, stack, locals, globals, call stack), execution context (scopes, data cache, registries), type system state, suspension state, semantic state, all data.

**Serialization pipeline:**
```
VMValue → SerializableVMValue → bincode → zstd → disk
```

**Limitations:** No incremental snapshots, no network awareness, no streaming serialization, no partial snapshots, HostClosure not serializable, Future(u64) is opaque.

### 2. Wire/Serialization Layer

Two paths optimized for different uses:
- **Snapshot (bincode):** Lossless, full VM state, for resume
- **Wire (MessagePack):** Lossy, display-oriented, for IPC

### 3. Distributed Computing Primitives

#### State Transfer
Content-addressed store enables incremental transfer — send blob hashes first, target requests only missing blobs.

#### Remote Function Execution (Distributed Lambda)
```shape
let result = await remote("compute-node-1") {
    prices.rolling(200).mean().last()
}
```
Compiler serializes closure + captured upvalues, sends to remote node, remote executes, result returns.

#### Work Distribution
```shape
let results = await symbols.distribute(cluster) |symbol| {
    simulate(strategy, data(symbol))
}
```

#### Transparent Distribution
```shape
let price = await@node("gpu-1") data("AAPL", { timeframe: "1m" })
let price = await@auto data("AAPL", { timeframe: "1m" })
```

### 4. Revolutionary Features

#### Snapshot-Based Fault Tolerance
```shape
@checkpoint(interval: 5.minutes)
fn backtest_pipeline(universe) {
    // Crashes resume from last checkpoint
}
```

#### Teleporting Computations
Move running code between nodes by capability.

#### Incremental Snapshots
Content-addressed chunks enable ~1-5% bandwidth for typical portfolio tick updates.

#### Time-Travel Debugging
Snapshot chains on every node enable stepping backward/forward across distributed computations.

### 5. Financial Domain Applications

- **Distributed Backtesting:** 1000 params x 50 symbols across 10 nodes = 50s vs 8.3min
- **Resumable Simulations:** 10M iteration Monte Carlo with checkpoints
- **Real-Time Risk Distribution:** Per-desk partitioning with incremental sync
- **Parallel Strategy Evaluation:** Multiple strategies on same data stream

### 6. Systems Architecture

```
Layer 4: Shape Distribution Protocol (distribute/remote/teleport semantics)
Layer 3: Snapshot Transfer Protocol (content-addressed blob exchange)
Layer 2: Shape Wire Protocol (MessagePack ValueEnvelopes) [EXISTING]
Layer 1: Transport (TCP/QUIC/Unix socket)
```

### 7. Trait Integration for Distribution Safety

```shape
trait Distributable: Serialize {
    fn wire_size(self) -> int
    fn is_deterministic(self) -> bool
}

trait IncrementalSync: Serialize {
    fn diff(self, previous: Self) -> Patch
    fn apply(self, patch: Patch) -> Self
}
```

### 8. Key Insight

`await` and `remote` are the same primitive at the VM level. Both suspend, wait, resume. Only difference is where computation runs. This unification means all async code works locally, adding distribution is a placement hint, type system ensures safety.

---

## Part IV: Cross-Team Synthesis

### The Three Systems Want to Be One

| System | Current | Unlocked Potential |
|--------|---------|-------------------|
| Comptime + Meta + Annotations | Working but disconnected | Zig-level metaprogramming with decorator ergonomics |
| Traits + Async | Traits parse but no enforcement. Async has opcodes but no joins | Trait-bounded generics + join blocks + structured concurrency |
| Snapshots + Distribution | Full VM state serialization, content-addressed | Transparent distributed computing |

### Convergence Points

1. **Comptime enables distribution safety:** `@typeInfo` + `Distributable` trait = compiler rejects non-serializable closures
2. **Join blocks generalize to distributed:** `join all {}` locally → `join all @parallel {}` distributed
3. **Traits enforce distributed contracts**
4. **Snapshots are the transport layer**
5. **Financial domain is the differentiator**

### Implementation Priority

| Phase | Work | Enables |
|-------|------|---------|
| 1 | Fix foundations (on_define, chaining, ctx.set) | Working annotations |
| 2 | Trait bounds (T: Trait, HasMethod, ImplementsTrait) | Type-safe polymorphism |
| 3 | Async joins (join all/race/any/settle) | Parallel data loading |
| 4 | Comptime unification (@typeInfo, comptime blocks) | Code generation, derive |
| 5 | Async traits (async methods, async bounds) | Polymorphic data sources |
| 6 | Structured concurrency (async let, async scope) | Resource safety |
| 7 | Distributed primitives (remote, distribute, snapshot transfer) | Cluster computing |
| 8 | Fault tolerance (@checkpoint, incremental snapshots) | Production resilience |

**Bottom line:** Shape has 70% of the infrastructure for something revolutionary. The snapshot system is remarkably complete, async suspension is well-designed, traits have the right shape. The gap is connecting these systems — comptime unification is the glue.
