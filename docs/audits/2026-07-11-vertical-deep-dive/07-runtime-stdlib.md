# Vertical Deep-Dive 07 — Runtime Stdlib & Builtins

Auditor: 07 of 19
Date: 2026-07-11
Territory: `crates/shape-runtime/src/stdlib/`, `crates/shape-runtime/src/stdlib_io/`, `crates/shape-runtime/src/stdlib_time.rs`, `crates/shape-runtime/src/intrinsics/`, `crates/shape-runtime/src/marshal.rs`, `crates/shape-runtime/src/typed_module_exports.rs`, `crates/shape-runtime/src/module_exports.rs`, the Shape-language stdlib at `crates/shape-runtime/stdlib-src/`, capability tags, intrinsic gating, and the method-registry contents (PHF tables) as they intersect stdlib.
Working tree: dirty (audited as-is, not just HEAD).
Debug binary used for all runtime transcripts: `/home/dev/dev/shape-lang/shape/target/debug/shape` (`shape run -m vm <file>`).

All runtime transcripts below have the two `libshape_ext_*` extension-load warnings and the "Shape engine initialized" banner stripped for readability.

## 0. Executive Summary

### 0.1 Overall health verdict

The runtime stdlib is in **good-to-strong shape on its native (Rust-backed) core** and **weaker at the Shape-language method surface**. The native modules — crypto, json, compress, csv, regex, unicode, env, file, toml, yaml, xml, msgpack, http, arrow, time, io — are cleanly structured behind a single, genuinely well-designed typed marshal layer (`marshal.rs` + `typed_module_exports.rs`). Data-integrity spot checks pass on known vectors (SHA-256/SHA-1/MD5/base64, gzip round-trip, JSON round-trip). The strict-typing discipline is real here: I found **zero live forbidden-pattern symbols** (`ValueWord`, `synthesize_value_word_from_raw`, `is_tagged`, `SlotKind::Dynamic`, `exec_*_dynamic_fallback`, runtime `tag_bits`) anywhere in the stdlib or intrinsics territory. The two-tier `TypedReturn`/`ConcreteReturn` carrier is a model ADR-005/006 conformant design and deserves to be held up as the reference for how the rest of the codebase should marshal.

The **big systemic problem is a three-way split-brain in method dispatch** created by the "methods moved from Rust to Shape stdlib files" (method-unification) effort. There are **three independent, hand-maintained tables** that must agree for any `x.method()` call to work:

1. `crates/shape-runtime/stdlib-src/core/*_methods.shape` — `extend` blocks, described in-code as the "canonical stdlib definitions".
2. `crates/shape-runtime/src/type_system/checking/method_table.rs` — a Rust-coded **mirror** the strict type-checker consults (the checker "never sees the stdlib `extend` block", per its own comments at `method_table.rs:206` and `:718`).
3. `crates/shape-vm/src/executor/objects/method_registry.rs` — the runtime PHF dispatch table.

These drift. I empirically demonstrated **both failure directions**: methods the checker accepts that crash at runtime (`"x".isEmpty()`, `"x".chars()` → `no method 'X' on receiver kind String`), and methods the runtime supports that the checker rejects (`(3.7).floor()`, `(3.7).abs()`, `(42).toInt()` → `Method 'X' not found on type`). Scalar `number`/`int` have essentially **no working method surface at all** despite `number_methods.shape` declaring `abs/floor/ceil/round/sign/clamp` and the runtime PHF `NUMBER_METHODS` implementing them — the type-checker table simply has no `number` section. This is the single largest correctness/completeness hole in the vertical.

Secondary problems: the "no global builtins / everything namespaced" rule in project memory is **not** what ships — there is a large set of un-namespaced global builtins (`sqrt`, `sin`, `abs`, `min`, `max`, `floor`, `format`, `range`, `print`, …); capability_tags coverage is **partial and largely inert on the default local run path** (permissions only enforce when a `[permissions]` section is declared); and several `.shape` module declarations (`parallel`, `file.read_bytes`/`write_bytes`) are dead — declared but not wired to a runtime export.

### 0.2 Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | **P1** | Three-way method-table split-brain: `method_table.rs` (checker) ↔ `method_registry.rs` (runtime PHF) ↔ `*_methods.shape` drift. Checker-accepted string methods crash at runtime. | `"hello".isEmpty()` → `Runtime error: no method 'isEmpty' on receiver kind String`; declared at `method_table.rs:705`, absent from string `STRING_METHODS` PHF (`method_registry.rs:906`). §5.1, §9-B1 |
| 2 | **P1** | `number`/`int` have no method surface in the type-checker: `(3.7).floor()`, `(-5).abs()`, `(3.7).round()` all fail `Method 'X' not found`, though `NUMBER_METHODS` PHF (`method_registry.rs:871`) implements them and `number_methods.shape` declares them. | Transcripts §2.6, §9-B2; `method_table.rs` has no `number`/`int` section (only Vec/Table/string/HashMap/Set/Deque/PQ/Mutex/Range/Iterator, `:374`–`:1075`) |
| 3 | **P1** | `"x".chars()`, `"x".toBool()`, `"x".graphemes()`, `HashMap.keys()/values()` type-check but abort at runtime (`no method` / `Not implemented … v0.4` / `SURFACE … V3-S5 ckpt-6`). 4 of 10 sampled string methods broken. | Sweep transcript §2.5, §14.3 |
| 3b | **P1** | Entire `Option`/`Result` method surface (`unwrap`/`unwrapOr`/`isSome`/`isNone`/`isOk`/`isErr`/`map`/`mapErr`) missing from the checker `MethodTable` — all fail `Method not found on type 'Option'/'Result'`. Only `match`/`?`/`??` work. | §14.2 |
| 3c | **P1** | `env::get`, `env::all`, `env::args` are dead: declared in `env.shape` + tagged in `capability_tags` but never registered in native `env.rs` → `module 'env' has no export 'get'`. Reading an env var value and CLI args do not work. (Book honestly flags it.) | §14.1 |
| 4 | **P2** | "No global builtins; everything namespaced" (project memory) contradicts shipped reality — `sqrt/sin/cos/abs/min/max/floor/ceil/round/pow/exp/log/sign/gcd/lcm/hypot/clamp/print/format/range/exit` are all callable un-imported. | `classify_builtin_function` `helpers.rs:4961`–`5135`; transcript §2.7 (`bare_sqrt=4.0`) |
| 5 | **P2** | Capability enforcement is inert on the default `shape run` path: `check_permission` no-ops unless `granted_permissions` is `Some`, and the CLI leaves it `None` (allow-all) unless a `[permissions]` section exists. `capability_tags.rs` compile-time derivation is real but the runtime gate only fires under explicit sandbox config. | `module_exports.rs:222`–`236`; `script_cmd.rs:49`–`58` |
| 6 | **P2** | `capability_tags.rs` maps only 6 modules (`io/file/http/env/time/csv`); real I/O-capable modules `arrow` (`FsRead`), `crypto.random_bytes` (`Random`), and everything in `stdlib_io` beyond the coarse `io` entry are **not** represented in the static tag table — the per-call runtime `check_permission` in each module is the only backstop, and it too is inert by default (see #5). | `capability_tags.rs:14`–`27`; `arrow_module.rs:53` gates `FsRead` at runtime but has no tag-table entry |
| 7 | **P2** | Dead `.shape` declarations: `parallel::map/filter/reduce/...` (`parallel.shape:26`) and `file.read_bytes`/`file.write_bytes` (`file.shape:106`,`:124`) are declared `builtin fn` but have no runtime export → `module 'parallel' has no export 'map'` / `module 'file' has no export 'read_bytes'`. Book honestly flags parallel as "not yet wired". | Transcripts §2.8; `stdlib/file.rs:11`–`18` (deferral note) |
| 8 | **P2** | Arity-monomorphization boilerplate: `marshal.rs` is 2,904 lines, of which ~21 near-identical `register_typed_fn_{0..6}` / `_full` / `register_typed_async_fn_*` helpers form the bulk. A single declarative macro would collapse ~1,200 lines. | `marshal.rs:1400`–`2600` |
| 9 | **P2** | `regex::match` and `regex::find` are byte-identical implementations (`find` is a documented alias) — copy-paste rather than delegation; divergence risk if one is patched. | `stdlib/regex.rs:40`–`60` vs `:62`–`82` |
| 10 | **P2** | `build_json_enum_heap_value` uses `panic!` (`json.rs:212`) on an internal-invariant miss instead of returning `Err`; a malformed recursion would abort the whole VM rather than surface a JSON parse error. Low likelihood, high blast radius. | `json.rs:208`–`217` |

### 0.3 Feature-completeness score: **72 / 100**

Native module breadth is excellent and most native functions work end-to-end (crypto, json, compress, csv, regex, unicode, env, toml, yaml, xml, msgpack, file-text, arrow, time all verified live). The score is pulled down by: the scalar-method surface being largely non-functional through method syntax (findings #2, #3); dead declarations (finding #7); `parallel` unimplemented; and `file.read_bytes` deferred. The *functional* form (`abs(x)`, `floor(x)`) covers the gap for math but not for the string/collection methods that only exist as methods.

### 0.4 Code-quality score: **80 / 100**

The native-module code is idiomatic, well-commented, and strict-typing-clean (zero forbidden symbols). The marshal layer is a genuinely deep, well-factored module. Points off for: heavy arity boilerplate (finding #8), copy-paste in regex (finding #9), a production `panic!` on the JSON hot path (finding #10), and the very large number of over-detailed historical comments (many stdlib files carry 40+ lines of migration archaeology in module headers — e.g. `http.rs:1`–`47` — which is admirable provenance but signals churn and raises the reading tax).

### 0.5 Biggest risk

The **method-table split-brain (findings #1–#3)** is the biggest risk because it is a *silent, recurring, correctness-affecting* defect class with no mechanical enforcement. The type-checker mirror in `method_table.rs` is maintained by hand against three other sources of truth, and its own comments record that it has already dropped entries before (`method_table.rs:697`: "Checker seed dropped one of the pair. A-final ROOT D."). Every new stdlib method must be added in three places or it either fails to type-check or crashes at runtime — and there is no test that cross-validates the three tables against each other. For a language whose entire value proposition is "statically-typed, no dynamic fallback, if it type-checks it runs", a class of expressions that type-check and then abort at runtime with `no method 'X' on receiver kind String` is a direct contradiction of the core promise. This should be closed with a generated single-source-of-truth table plus a consistency test, not more hand-patching.

## 1. Architecture & Code-Structure Map

### 1.1 Where the stdlib actually lives

The stdlib is split across **four physical homes**, which is itself worth knowing:

| Home | Path | What it holds | LOC (rust `.rs` / shape `.shape`) |
|------|------|---------------|-----|
| Native Rust modules | `crates/shape-runtime/src/stdlib/` | reqwest/serde-backed native `ModuleExports` (crypto, json, http, csv, regex, unicode, env, file, toml, yaml, xml, msgpack, compress, archive, arrow) + capability tags + sandbox providers | 6,113 rust |
| Native io/time | `crates/shape-runtime/src/stdlib_io/`, `stdlib_time.rs` | file handles, path ops, networking, process, wall-clock/monotonic time | 1,947 + 279 rust |
| Math intrinsics | `crates/shape-runtime/src/intrinsics/` | `__intrinsic_*` native modules: vector, matrix, fft, distributions, statistical, stochastic, rolling, convolution, recurrence, random | 4,068 rust |
| Shape-language stdlib | `crates/shape-runtime/stdlib-src/` | 109 `.shape` files: `extend` method blocks, pure-Shape modules (finance, physics, iot, llm, math/*), `builtin fn` declarations that bind to native exports | 12,836 shape |

Marshal + export plumbing that everything routes through:

| File | LOC | Responsibility |
|------|-----|----------------|
| `marshal.rs` | 2,904 | `FromSlot`/`ToSlot` traits, `register_typed_fn_{0..6}[_full]`, async variants, `install`, `MarshalError` |
| `typed_module_exports.rs` | 523 | `ConcreteReturn` (leaf carriers), `TypedReturn` (wrappers), `ConcreteType` (registration descriptors) |
| `module_exports.rs` | 752 | `ModuleExports`, `ModuleContext`, `check_permission`/`check_fs_permission`/`check_net_permission` |
| `native_resolution.rs` | 875 | maps `module::function` call sites to native exports |

### 1.2 Native module inventory (LOC + export count)

From `find … | wc -l` and per-file `register_typed*` counts:

| Module | Rust LOC | Exports | Verified working end-to-end |
|--------|----------|---------|------------------------------|
| `http.rs` | 660 | 8 (get, delete, post_text, post_bytes, put_text, put_bytes, post_json, put_json) | not exercised live (needs network); registration present |
| `json.rs` | 630 | 4 (parse, __parse_typed, stringify, is_valid) | yes — round-trip §2.2 |
| `runtime_policy.rs` | 600 | (provider traits: RealFileSystem, PolicyEnforcedFs) | n/a |
| `virtual_fs.rs` | 546 | (VFS provider for sandbox) | n/a |
| `xml.rs` | 518 | 2 (parse, stringify) | yes — parse §2.3 |
| `csv_module.rs` | 459 | 6 (parse, parse_records, stringify, stringify_records, read_file, is_valid) | yes — §2.3 |
| `crypto.rs` | 368 | 13 (sha1/256/512, md5, hmac_sha256, base64/hex enc/dec, random_bytes, ed25519 ×3) | yes — vectors §2.1 |
| `capability_tags.rs` | 346 | (compile-time permission map) | see §6 |
| `regex.rs` | 209 | 7 (match, find, is_match, match_all, replace, replace_all, split) | yes — §2.3 |
| `yaml.rs` | 203 | 4 (parse, parse_all, stringify, is_valid) | yes — §2.3 |
| `msgpack_module.rs` | 198 | 4 (encode, decode, encode_bytes, decode_bytes) | yes — §2.3 |
| `deterministic.rs` | 190 | (seeded PRNG + virtual clock) | n/a |
| `toml_module.rs` | 187 | 3 (parse, stringify, is_valid) | yes — §2.3 |
| `unicode.rs` | 170 | 5 (normalize, category, is_letter, is_digit, graphemes) | yes — §2.3 |
| `file.rs` | 168 | 4 live (read_text, write_text, read_lines, append); 2 dead (read_bytes, write_bytes) | text yes §2.3; bytes dead §2.8 |
| `compress.rs` | 163 | 6 (gzip, gunzip, zstd, unzstd, deflate, inflate) | yes — round-trip §2.3 |
| `arrow_module.rs` | 142 | 3 (read_table, read_tables, metadata) | registration present; FsRead-gated |
| `env.rs` | 136 | 4 (has, cwd, os, arch) + get via native | yes — §2.3 |
| `archive.rs` | 122 | 2 (zip_extract, tar_extract) | registration present |
| `byte_utils.rs` | 29 | (helpers) | n/a |

### 1.3 Intrinsics inventory

`crates/shape-runtime/src/intrinsics/` provides the `__intrinsic_*` native modules registered in `all_stdlib_modules()` (`stdlib/mod.rs:56`–`68`):

| File | LOC | Content |
|------|-----|---------|
| `matrix_kernels.rs` | 637 | matmul kernels (AVX2-detected) |
| `vector.rs` | 609 | elementwise vec abs/sqrt/ln/exp/add/sub/mul/div/max/min/select |
| `math.rs` | 469 | trig (`__intrinsic_sin/cos/...`), mean/std/variance, char-code |
| `fft.rs` | 313 | FFT |
| `matrix.rs` | 301 | matrix add/sub |
| `stochastic.rs` | 300 | brownian/gbm/ou/random-walk |
| `distributions.rs` | 294 | uniform/lognormal/exponential/poisson samplers + PDF/CDF |
| `convolution.rs` | 225 | conv kernels |
| `array_transforms.rs` | 206 | shift/diff/pct_change/fillna/cumsum/cumprod/clip |
| `statistical.rs` | 177 | correlation/covariance/percentile/median |
| `rolling.rs` | 159 | rolling sum/mean/std/min/max, ema |
| `random.rs` | 132 | ChaCha8 PRNG |
| `recurrence.rs` | 126 | linear recurrence |
| `mod.rs` | 120 | module registration |

### 1.4 Data flow (a native module call, end to end)

1. **Parse**: `use std::core::crypto` then `crypto::sha256("abc")` → AST call node with module-qualified name.
2. **Compile-time resolution**: `resolve_scoped_module_builtin_function` / `compile_module_builtin_function_call` (`compiler/expressions/function_calls.rs:2661`) resolves the export against the module's registered `ModuleExports` schema and emits a typed builtin-call opcode carrying the argument `NativeKind`s.
3. **Capability derivation**: at compile time `capability_tags::required_permissions(module, fn)` (`compiler_impl_initialization.rs:330`, `compiler/statements.rs:2030`) folds the required `PermissionSet` into the `FunctionBlob` (baked into content hash).
4. **Runtime dispatch**: the VM pops the argument slots, wraps them as `KindedSlot`s, and calls the module's `TypedInvoke` closure (`marshal.rs:1449`). Each closure decodes args via `FromSlot::from_kinded`, runs the native body, and returns a `TypedReturn`.
5. **Runtime gate**: inside the body, I/O modules call `check_permission(ctx, Permission::X)` (`module_exports.rs:222`), which is a no-op if `ctx.granted_permissions` is `None`.
6. **Result projection**: the dispatcher projects the `TypedReturn`/`ConcreteReturn` directly into a typed VM slot via the registered `ConcreteType` (no `ValueWord`, no tag decode).

### 1.5 Key types

- `ConcreteReturn` (`typed_module_exports.rs:55`): 27 leaf variants — `I64`, `F64`, `Bool`, `Unit`, `String`, `Instant`, `ArrayI64`, `ArrayF64`, `ArrayString`, `ArrayStringRows`, `ArrayHeapValue`, `Bytes`, `HashMapStringString`, `HashMapStringHeapValue`, `JsonValue`, `OpaqueTypedObject`, `DataTable`, `IoHandle`, … Each is a **leaf** — no `ConcreteReturn`-in-`ConcreteReturn` nesting.
- `TypedReturn` (`typed_module_exports.rs:196`): wrapper layer — `Concrete`, `Ok`, `Err`, `Some`, `None`, `ObjectPairs`, `OkObjectPairs`, `SomeObjectPairs`, `TypedObject`, … Every wrapper takes a `ConcreteReturn` payload; `TypedReturn`-in-`TypedReturn` is unrepresentable (the comment at `:184` notes this makes the deleted `TypedReturn::ValueWord` escape hatch unreachable).
- `ModuleContext` (`module_exports.rs`): carries `granted_permissions: Option<PermissionSet>` and `scope_constraints` for the runtime gate.

### 1.6 Entry points

- `all_stdlib_modules()` (`stdlib/mod.rs:37`): the canonical native-module registry — 30 `create_*_module()` calls, each invoked exactly once.
- `create_file_module_with_provider(fs)` (`stdlib/file.rs:30`): the provider-injection pattern that makes VFS/sandbox transparent — a genuinely good seam (see §10).
- `register_typed_fn_N` (`marshal.rs:1400`+): the registration surface every native body uses.

## 2. Feature Completeness (code-exists vs works-end-to-end)

I distinguish three states: **WORKS** (verified live), **CODE EXISTS** (registered but not exercised here, or blocked by an orthogonal issue), **DEAD/MISSING** (declared but no runtime binding, or absent).

### 2.1 Crypto — WORKS, correct against known vectors

Transcript (`t08.shape`, `-m vm`):

```
$ shape run -m vm t08.shape
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855   # sha256("")
a9993e364706816aba3e25717850c26c9cd0d89d                           # sha1("abc")
900150983cd24fb0d6963f7d28e17f72                                   # md5("abc")
108c2ab4adc527ea7478e97310776218e674dfb2bb67ae6954ce684f7e3a1c45   # hmac_sha256("key","msg")
TWFu                                                               # base64_encode("Man")
Man                                                                # base64_decode("TWFu") -> Ok("Man")
```

All five hashes match published test vectors (sha256 of empty string, sha1/md5 of "abc"). base64 round-trips. `crypto.rs` also registers `sha512`, `hex_encode`/`hex_decode`, `random_bytes` (Random-gated, `crypto.rs:195`), and `ed25519_generate_keypair`/`ed25519_sign`/`ed25519_verify`. Crypto is the **most complete and most correct** module in the vertical.

### 2.2 JSON — WORKS (round-trip verified)

```
$ shape run -m vm t04_json.shape       # parse -> stringify -> is_valid
Ok("{"a":1,"b":[2,3]}")                # json::stringify(parse("{...}")) round-trips
true                                    # is_valid("{"x":1}")
false                                   # is_valid("{bad")
```

`json::parse` returns `Result<Json>` (the `Json` enum from `json_value.shape`); `json::stringify` returns `Result<string>`. Note the round-trip output is byte-accurate (`{"a":1,"b":[2,3]}`). Earlier I mis-passed the whole `Result` to `stringify` and saw the internal enum representation `{"variant":0,"payload":...}` leak — that is **not** a bug (it was correctly serializing a `Result` value), but it is a sharp edge: stringifying a `Result<Json>` instead of the unwrapped `Json` silently produces the enum's structural encoding rather than an error. `json.rs` additionally exports `__parse_typed` (schema-driven, `OpaqueTypedObject` return).

### 2.3 Parser / codec / util modules — WORKS

Combined transcripts (`t03`, `t05b`, `t07b`, `z.shape`):

```
regex::is_match("hello","h.llo")     -> true
regex::replace_all("a1b2c3","[0-9]","#") -> a#b#c#
unicode::is_letter("A")              -> true
unicode::graphemes("abc").len()      -> 3
env::os()                            -> linux
env::arch()                          -> x86_64
compress::gzip(...) / gunzip(...)    -> roundtrip_ok=true  (gz_len=35 for a 35-byte input)
toml::parse("x=1\ny=\"hi\"")         -> toml_ok
yaml::parse("a: 1\nb: 2")            -> yaml_ok
csv::parse("a,b\n1,2\n3,4").len()    -> 3
csv::is_valid("a,b\n1,2")            -> true
msgpack::encode_bytes([1,2,3])       -> Ok(len=4)
xml::parse("<root><a>1</a>...")      -> xml_ok
math::PI()                           -> 3.141592653589793
math::sin(1.0)                       -> 0.8414709848078965
math::mean([1.0,2.0,3.0])            -> 2.0
crypto::* (see §2.1)
```

Every native parser/codec module I could exercise without a network or an external file works end-to-end. The return-type conventions are inconsistent between modules (see §11): `compress::gzip` returns a bare `Array<int>` (no `Result`), `csv::parse` returns a bare `Array<Array<string>>`, while `toml/yaml/xml/msgpack::parse` return `Result<...>`. That inconsistency cost me two test iterations (my `match Ok/Err` on a non-`Result` gzip result produced `Uncaught exception: No match arm matched the value` — a *user-facing* footgun, not a stdlib bug, but a symptom of unprincipled return-type conventions).

### 2.4 File (text) — WORKS

```
$ shape run -m vm t01b_file.shape
READ: hello world
LINES: 2
```

`file::write_text` / `read_text` / `append` / `read_lines` all work against the real filesystem through the `FileSystemProvider` seam. Note this transcript also demonstrated the JIT `[jit-fallback]` deopt on `match Ok(_)/Err(_)` (the run silently fell to the interpreter and still produced correct output — see §5.4); I re-ran everything under `-m vm` thereafter.

### 2.5 String methods — PARTIAL (4 of 10 sampled broken)

This is finding #3. Sweep (`sm.shape`, each `"hello,x".<method>`):

```
isEmpty()        => Runtime error: no method 'isEmpty' on receiver kind String
chars().len()    => Runtime error: no method 'chars' on receiver kind String
toBool()         => Not implemented: phase-1b-vm-wave-5c-conversion: ToBool body migration ... pending (v0.4 / planned)
trim()           => hello,x
toUpperCase()    => HELLO,X
split(",").len() => 2
padStart(8,"*")  => *hello,x
graphemes().len()=> Not implemented: String.graphemes: SURFACE — V3-S5 ckpt-5 ... Rebuild lands at ckpt-6
isAscii()        => true
codePointAt(0)   => 104
```

`trim`, `toUpperCase`, `split`, `padStart`, `isAscii`, `codePointAt` work. `isEmpty`, `chars`, `toBool`, `graphemes` type-check (they are declared in `method_table.rs` `str_methods`, `:705`,`:747`,`:749`) but fail at runtime. `toBool`/`graphemes` are *marked* pending (v0.4 / V3-S5 ckpt-6) — those are honest surface stubs. `isEmpty` and `chars` are **unmarked drift**: they exist in the checker table and for other receiver types, just missing from the string PHF (`STRING_METHODS`, `method_registry.rs:906`, which has `len/length/…/slice/toInt/…` but not `isEmpty`/`chars`).

### 2.6 Number / int methods — BROKEN through method syntax

Finding #2. `(3.7).floor()`, `(3.7).abs()`, `(3.7).round()`, `(-5).abs()`:

```
(3.7).floor()   => error[SEMANTIC]: Method 'floor' not found on type 'number'
(3.7).abs()     => error[SEMANTIC]: Method 'abs' not found on type 'number'
(3.7).round()   => error[SEMANTIC]: Method 'round' not found on type 'number'
(3.7).toString()=> 3.7            # only toString works
(-5).abs()      => error[SEMANTIC]: Method 'abs' not found on type 'int'
(42).toString() => 42
```

These *fail at compile time* even though the runtime `NUMBER_METHODS` PHF (`method_registry.rs:871`–`875`: floor/ceil/round/abs/sign → `number_methods::number_*_v2`) implements them and `number_methods.shape:8`–`11` declares them. The type-checker's `MethodTable` (`method_table.rs`) has **no `number` or `int` section** (its section list at `:374`–`:1075` covers Vec/Table/string/HashMap/Set/Deque/PriorityQueue/Mutex/Range/Iterator only). The functional form is the only working path:

```
$ shape run -m vm y.shape
3     # floor(3.7)
5     # abs(-5)
4     # ceil(3.2)
4     # round(3.5)
```

So scalar math *works* as global builtins but the entire `.method()` surface for `number`/`int` (beyond `toString`/`toNumber`) is inaccessible. This is a real completeness hole for anyone writing `x.abs()`.

### 2.7 Global builtins — PRESENT (contradicts "no global builtins" memory)

Finding #4. Un-imported:

```
$ shape run -m vm t06.shape        # note: `use std::core::math` present but not required for bare forms
bare_sqrt=4.0
bare_sin=0.8414709848078965
math_sin=0.8414709848078965        # identical: math::sin routes to the SAME BuiltinFunction::Sin
abs=5
```

`classify_builtin_function` (`helpers.rs:4961`) registers a large surface of `ResolutionScope::ModuleBinding` (i.e. globally-visible) builtins: `abs/min/max/sqrt/ln/pow/exp/log/floor/ceil/round/sin/cos/tan/asin/acos/atan/stddev/sign/gcd/lcm/hypot/clamp/isNaN/isFinite`, plus `print/format/range/exit/reflect/snapshot`, plus type predicates `is_number/is_string/...` and conversions `to_string/to_number/to_bool`. These need no `use`. The book acknowledges this for math ("bare global builtins + `std::core::math` constants and trig", `overview.mdx:32`), so the book is accurate; the *project-memory* rule "everything must be namespaced" is aspirational, not shipped.

Notably `math::sin(1.0)` and bare `sin(1.0)` produce identical output because `__intrinsic_sin` maps to `BuiltinFunction::Sin` (`helpers.rs:5101`) — the same VM handler bare `sin` uses. So there is **no drift** between the namespaced trig wrappers and the bare builtins (they are one implementation), which is the *good* side of this design.

### 2.8 Dead declarations — parallel, file bytes

Finding #7.

```
$ shape run -m vm par.shape
error[SEMANTIC]: module 'parallel' has no export 'map'
```

`parallel.shape:26`–`94` declares `builtin fn map/filter/for_each/chunks/reduce/sort/num_threads`, but no runtime `create_parallel_module` exists in `all_stdlib_modules()`. The book (`overview.mdx:31`) is honest: "parallel — Data-parallel array operations (planned; bindings not yet wired)".

```
$ shape run -m vm t01_file.shape       # (with read_bytes)
error[RUNTIME]: module 'file' has no export 'read_bytes'
```

`file.shape:106`,`:124` declare `read_bytes`/`write_bytes`, but `stdlib/file.rs:11`–`18` documents them as **deferred** ("until the `Array<number>` marshal extension … lands"). So the `.shape` surface over-promises relative to the wired native surface. Byte I/O *does* exist via `io::read_bytes` (`stdlib_io/file_ops.rs:210`), just not via the `file` module.

### 2.9 Intrinsics — WORK, gated correctly

`__intrinsic_vec_abs`, `__intrinsic_sin`, etc. work when called from stdlib (`allow_internal_builtins = true`) but are correctly rejected from user code (§2.10). The math/vector/matrix/fft/distribution/stochastic intrinsic families are all registered in `all_stdlib_modules()` and reachable via their pure-Shape wrappers (`math.shape`, `vec.shape`, `distributions.shape`, etc.).

### 2.10 Intrinsic gating — WORKS as documented

Finding-adjacent verification (CLAUDE.md claim: `__intrinsic_*`/`__json_*`/`__native_*` blocked for user code). A real internal name from user code:

```
$ shape run -m vm g2.shape           # __intrinsic_vec_abs([-1.0,2.0])
error[SEMANTIC]: '__intrinsic_vec_abs' resolves to internal intrinsic scope and is not
available from ordinary user code. Internal intrinsics are reserved for std::*
implementations and compiler-generated code.
```

The gate is `is_internal_intrinsic_name` (`helpers.rs:5155`: prefixes `__native_`, `__intrinsic_`, `__json_`) combined with `allow_internal_builtins` (`function_calls.rs:2663`, `:2676`). A **fabricated** internal name (`__intrinsic_foo`, not in the classify table) errors as plain `Undefined function` — also correct. The CLAUDE.md note that `__into_*`/`__try_into_*` are "NOT gated" is **stale**: those builtins were *removed entirely* (`helpers.rs:5038`: "`__into_*`/`__try_into_*` builtins removed — primitive conversions now use typed `ConvertTo*`/`TryConvertTo*` opcodes"). The `as` cast path that replaced them works:

```
$ shape run -m vm g4.shape
3 5.0     # (3.7 as int)=3, (5 as number)=5.0
```

## 3. Code Quality

### 3.1 Idiom & naming

Native modules follow a consistent, readable pattern: a `create_<name>_module() -> ModuleExports` factory that calls `register_typed_fn_N` per export with `(name, description, param spec, ConcreteType return, |args, ctx| body)`. Naming is idiomatic Rust (snake_case, no Hungarian, clear closure captures). The `ConcreteReturn`/`TypedReturn` naming is precise and the doc-comments explaining *why* a variant is a leaf are excellent (`typed_module_exports.rs:81`–`182`).

The one naming smell is the surface-vs-namespace duplication of names across the three method tables (see §5) — a `method len()` in `string_methods.shape`, a `"len"` PHF entry, and a `("len", …)` checker entry — but that is a structural issue, not a naming one.

### 3.2 Error handling

Native bodies return `Result<TypedReturn, String>` and propagate errors as `Err(format!(...))`. This is idiomatic and user-visible errors are decent (`file.read_text() failed: ...`, `file.read_text() invalid UTF-8: ...`, `stdlib/file.rs:52`,`:54`). The stdlib correctly uses `Result`, not exceptions, matching the language's no-throw discipline.

The one **hard-fail** in an error path is finding #10: `json.rs:212` uses `panic!("build_json_enum_heap_value must return TypedObject, got {:?}", …)` in the object-construction recursion. This is an internal invariant (the recursive helper is expected to always return a `TypedObject`), so it should be unreachable — but if the invariant is ever violated by a future edit, a *user's* `json::parse` of an object aborts the entire VM rather than returning `Err`. On a parser that runs over untrusted input, a `panic!` on the hot path is the wrong failure mode; it should be `return Err(...)` or `debug_assert!` + graceful fallback.

### 3.3 Unsafe usage

Counted **22 `unsafe` blocks** in `stdlib/` and **12** in `intrinsics/`. Breakdown:

- **SIMD (justified)**: `intrinsics/matrix_kernels.rs` (10+ blocks) and `intrinsics/math.rs` — all of the form `f64x4::from(unsafe { *(ptr.add(offset) as *const [f64;4]) })` (`matrix_kernels.rs:29`). These are aligned-load reinterpretations for wide vector math with bounds established by the surrounding loop. Standard, justified, and the surrounding code establishes the length invariant.
- **Typed-array / HashMapData construction (justified)**: `json.rs:181`,`:217`, `xml.rs:94`,`:135`, `csv_module.rs:312`–`417` — building strict `TypedArray<T>` / `HashMapData<T>` heap structures via `from_slice`/`get_unchecked`/`stamp_elem_type`/`insert`. Each carries a detailed comment explaining the refcount-share transfer (`json.rs:187`–`191`). These are the price of the zero-tag heap model and are correctly commented.
- **Lock unwraps (not unsafe, but panic-on-poison)**: `virtual_fs.rs` has 24 production `.read().unwrap()`/`.write().unwrap()` on `RwLock` — standard poison-propagation, acceptable.

No **unjustified** unsafe found in the territory. The unsafe is concentrated where the zero-tag heap model forces it (typed-array construction) and where SIMD demands it. This is a good result for a language that leans this hard on `unsafe` at its core.

### 3.4 Complexity hotspots

- Longest native module: `http.rs` (660) — but ~47 lines are historical header comments; the actual code is moderate. No single function is pathological (no lines over 120 chars flagged; `awk 'length>120'` returned 0 for stdlib).
- `csv_module.rs:305`–`420` has the deepest nesting — a `match HashMapKindedRef { … }` per-variant projection inside a `map` closure inside a records-stringify path. It is readable but is the one place a future maintainer will need care.
- `marshal.rs` (2,904) is long by *repetition*, not by per-function complexity (§4).

### 3.5 Dead code in-territory

- **`parallel.shape`** — fully dead runtime-wise (declared, no export). Book-flagged as planned.
- **`file.read_bytes`/`write_bytes`** in `file.shape` — dead (deferred; `stdlib/file.rs:11`).
- **`number_methods.shape` / `int_methods.shape` scalar methods** — effectively dead through method syntax because the checker table never learned them (§2.6). The `.shape` file exists and the runtime PHF exists, but the two are not bridged by the checker, so users can't reach `x.floor()`.
- **`string_methods.shape` `isEmpty`/`chars`/`toBool`** — declared and checker-accepted, runtime-dead (§2.5).

The `.shape` `extend` method files (`string_methods.shape`, `number_methods.shape`, `hashmap_methods.shape`, etc.) have a self-referential body idiom — `method len() -> int { self.len() }` — where the body calls the method it defines. The header comment (`string_methods.shape:4`) explains: "All methods delegate to VM PHF dispatch at runtime — they exist only so the compiler can type-check calls." But per §2.6 the *checker* does not actually read these blocks (it uses `method_table.rs`), so it is unclear these files are load-bearing at all for the strict checker. They may be vestigial relative to the hand-maintained checker mirror — a question the maintainers should resolve (are they parsed on the non-strict path? are they only for LSP/docs?). Either way, three sources claiming to define the same methods is the core defect (§5.1).

## 4. Duplication & DRY Violations

### 4.1 `regex::match` vs `regex::find` — byte-identical bodies

`stdlib/regex.rs:40`–`60` (`match`) and `:62`–`82` (`find`) are line-for-line identical except the diagnostic string (`regex.match()` vs `regex.find()`):

```rust
// match, regex.rs:49
|text, pattern, _ctx| {
    let re = regex::Regex::new(pattern.as_str())
        .map_err(|e| format!("regex.match() invalid pattern: {}", e))?;
    match re.captures(text.as_str()) {
        Some(caps) => { let m = caps.get(0).unwrap(); Ok(TypedReturn::SomeObjectPairs(match_to_pairs(&m, &caps))) }
        None => Ok(TypedReturn::None),
    }
},
// find, regex.rs:71 — identical, only "regex.find()" differs
```

`find` is documented as "alias for regex.match" (`regex.rs:59`). Danger level: **low but real** — if someone fixes a bug in `match` (e.g. named-group handling) they must remember to patch `find` too. The right shape is one private helper both call, or registering the same closure under two names.

### 4.2 Marshal arity boilerplate — ~1,200 lines of near-clones

`marshal.rs` has 21 `register_typed_fn_*` / `register_typed_async_fn_*` functions (`marshal.rs:1400`–`2600`). `register_typed_fn_1` (`:1433`) and `register_typed_fn_2` (`:1478`) differ only in: arg count, the `slots.len() != N` check, and the per-arg `P{i}::from_kinded(&slots[i])?` unpacking. Each is ~45 lines. This pattern repeats for arities 0–6, each with a plain and a `_full` variant, then again for async. This is the classic Rust "no variadic generics" tax, but a `macro_rules!` generator (`impl_register_typed_fn!(1, P0; 2, P0 P1; …)`) would collapse the whole block to ~100 lines of macro + invocations. Danger level: **low** (the clones are correct and mechanical) but it is the single biggest *maintenance-surface* concentration in the vertical — every change to the marshal contract (e.g. adding a new `ModuleContext` field) touches 21 functions.

### 4.3 Argument-decode boilerplate across native modules

Every native body re-derives the same `x.as_str()` / `as_bytes()` / `format!("{module}.{fn}() failed: {e}")` pattern. This is mild and largely unavoidable given the per-function typed closures, but a small set of helpers (`fs_err(fn_name, e)`, `utf8(bytes, fn_name)`) would remove the repeated `.map_err(|e| format!("file.read_text() invalid UTF-8: {}", e))` idiom that appears in `file.rs`, `csv_module.rs`, `toml_module.rs`, etc. Danger level: **negligible** — cosmetic.

### 4.4 Permission-check call-site duplication

`check_permission(ctx, Permission::FsRead)?` / `check_fs_permission(...)` appears at every I/O body entry (30+ sites across `stdlib_io/*.rs`, `arrow_module.rs`, `crypto.rs`, `file.rs`). This is *correct* duplication (each site genuinely gates), not a DRY violation, but see §6 for the coverage-vs-tag-table mismatch it papers over.

## 5. Split-Brain Analysis

This is the heart of the vertical's problems. There are three genuine split-brains and one benign duplication.

### 5.1 THREE-way method table (P1) — the dominant defect

The same method concept is defined in three independently-maintained places:

| Source | Path | Role | Example: string `isEmpty` | Example: number `floor` |
|--------|------|------|---------------------------|--------------------------|
| A. Shape `extend` block | `stdlib-src/core/string_methods.shape`, `number_methods.shape` | claimed "canonical" | declared (`:9`) | declared (`:9`) |
| B. Checker mirror | `type_system/checking/method_table.rs` | what strict checker consults | declared (`:705`) | **absent** (no number section) |
| C. Runtime PHF | `shape-vm/.../method_registry.rs` | actual dispatch | **absent** from `STRING_METHODS` | present (`:871`) |

The checker's own comment admits the mirror is hand-maintained and has drifted before:

- `method_table.rs:204`–`209`: "The runtime dispatches these correctly through shape-vm's PHF method registry; the checker's MethodTable was simply incomplete. The seed below mirrors the canonical stdlib `.shape` definitions … so the checker resolves them."
- `method_table.rs:697`–`699`: "PHF registry has len+length …; both -> v2_string_len. **Checker seed dropped one of the pair. A-final ROOT D.**"
- `method_table.rs:718`–`733` (`register_json_methods`): "the strict checker … **never sees the stdlib `extend Json { … }` block** … so a `json::parse(text)` result had `lookup` return `None` … surfacing a spurious 'Method not found on type Json'".

So the architecture is: strict checker runs over the user program only, does **not** load stdlib `extend` blocks, and instead relies on a Rust-coded mirror that must be manually kept in lockstep with both the `.shape` files (A) and the runtime PHF (C). Any of the three getting out of sync produces one of two failures:

- **B has it, C doesn't** → type-checks, crashes at runtime. Demonstrated: `"x".isEmpty()`, `"x".chars()` (§2.5).
- **C has it, B doesn't** → runtime supports it, checker rejects it. Demonstrated: `(3.7).floor()`, `(42).toInt()` (§2.6, §5.2).

**Drift evidence (both directions, empirical):**

```
"hello".isEmpty()   # B✓ C✗  -> Runtime error: no method 'isEmpty' on receiver kind String
(3.7).floor()       # B✗ C✓  -> error[SEMANTIC]: Method 'floor' not found on type 'number'
(42).toInt()        # B✗ C✓  -> error[SEMANTIC]: Method 'toInt' not found on type 'string'
```

For `toInt`: `STRING_METHODS` PHF (`method_registry.rs:906`) lists `"toInt" "to_int" "toFloat" "to_float"` but `method_table.rs` `str_methods` (`:695`) does not, so the checker blocks a method the runtime would happily execute.

There is **no test that cross-validates A vs B vs C.** That absence is the reason this class keeps recurring (the "A-final ROOT D" comment is an artifact of a *previous* instance of exactly this drift being found and hand-patched).

### 5.2 `STRING_METHODS` PHF vs checker `str_methods` — concrete divergence table

| Method | Checker (`method_table.rs`) | Runtime PHF (`method_registry.rs`) | Result |
|--------|------|------|--------|
| `isEmpty` | ✓ `:705` | ✗ | type-checks, crashes |
| `chars` | ✓ `:749` | ✗ | type-checks, crashes |
| `toBool` | ✓ `:747` | stub (v0.4) | type-checks, `NotImplemented` |
| `graphemes` | ✓ | SURFACE stub (V3-S5 ckpt-6) | type-checks, `NotImplemented` |
| `toInt`/`toFloat` | ✗ | ✓ `STRING_METHODS` | checker rejects working method |
| `slice` | ✓ `:808` | ✓ | works |
| `len`/`length` | ✓ (both, after ROOT-D fix) | ✓ | works |
| `trim`/`toUpperCase`/`split`/`padStart`/`isAscii`/`codePointAt` | ✓ | ✓ | works |

### 5.3 Math: three layers but NO drift (the good case)

Math is implemented in three places too — `math.shape` wrappers, `intrinsics/math.rs` native `__intrinsic_*`, and `shape-vm` `BuiltinFunction::Sin` handlers — but here they **converge**: `math.shape`'s `pub fn sin(x) { __intrinsic_sin(x) }` → `__intrinsic_sin` maps to `BuiltinFunction::Sin` (`helpers.rs:5101`) → the same VM handler that bare `sin()` uses (`vm_impl/builtins.rs:277`). One implementation, three names. `math::sin(1.0)` == `sin(1.0)` == `0.8414709848078965` (§2.7). This is how the method tables *should* be structured: multiple surfaces, one implementation, no parallel logic to drift. The contrast with §5.1 is instructive — math avoided the split-brain by routing all surfaces to a single handler; the method tables did not.

### 5.4 VM vs JIT — the `[jit-fallback]` observation

While testing `file` with `match Ok(_)/Err(_)`, the default `-m jit` run emitted a whole-program deopt to the interpreter:

```
[jit-fallback] function main failed JIT compile: ... EnumPayload (R8 W9 G.2 Step 2 Bucket 2):
`Pattern::Constructor` payload binder (`Ok(_)` / `Err(_)` / `Some(_)`) codegen has
receiver-recovery soundness gap ... Tracked v0.4 ... running under interpreter
READ: hello world
LINES: 2
```

This is **not** a stdlib bug (it is a JIT-lowering gap tracked for v0.4, and the output is correct), but it matters for the vertical: **any stdlib code that pattern-matches on `Result`/`Option`** — which is *every* `file`/`compress`/`json`/`toml`/`csv` result-consuming program — currently deopts the whole program to the interpreter under the default `-m jit`. The stdlib's own return-type conventions (heavy `Result`/`Option`) thus route straight into the JIT's weakest path. Config-duplication risk is nil here, but the *performance* story for idiomatic stdlib use is "you get the interpreter" until the v0.4 EnumPayload work lands. I confirmed this reproduces on `t01b_file.shape`.

### 5.5 Return-type convention split — doc/code inconsistency

Not a table split-brain but a *convention* one: `compress::gzip -> Array<int>` (no Result), `csv::parse -> Array<Array<string>>` (no Result), but `toml/yaml/xml/msgpack::parse -> Result<...>`, and `file::read_text -> Result<string>`. There is no principled rule for when a fallible-looking operation returns `Result` vs. a bare value. `gzip` cannot fail on valid input so bare is defensible; `csv::parse` returning a bare array (swallowing malformed rows) is more questionable given `csv::is_valid` exists separately. This inconsistency is a documented (via signatures) but unprincipled surface that surprises users (§2.3).

## 6. ADR & Spec Conformance

The ADRs binding this territory are ADR-005 (single-discriminator / typed-slot construction) and ADR-006 (value & memory model, incl. the `ConcreteReturn`/`TypedReturn` marshal layer and the `KindedSlot` method ABI). I grepped `ADR-005`/`ADR-006` markers (54 hits across `stdlib/xml.rs`, `json.rs`, `arrow_module.rs`, `typed_module_exports.rs`, `marshal.rs`) and checked forbidden-pattern symbols.

### 6.1 Forbidden patterns — CONFORMS (zero live hits)

`grep -rn 'ValueWord|synthesize_value_word_from_raw|is_tagged|exec_.*_dynamic_fallback|SlotKind::Dynamic|tag_bits'` over `stdlib/` and `intrinsics/` returns **only** references inside deletion-fate comments (e.g. `stdlib/file.rs:16` "the deleted `as_any_array().to_generic()` tag_bits dispatch", `typed_module_exports.rs:119` "Replaces the deleted `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(arc)))`"). No live forbidden symbol exists in the territory. This is a **clean pass** — the strict-typing plan's core deletion held here.

### 6.2 ADR-005 §1 single-discriminator — CONFORMS

`ConcreteReturn` is explicitly designed to *not* project 1:1 to `HeapKind`. The doc-comments enforce this discipline: `ArrayHeapValue` is "**one** `ConcreteReturn::ArrayHeapValue`" and "Per-element-kind variants (`ArrayDataTable`/…) are rejected on the same grounds as the parametric-NativeKind pattern" (`typed_module_exports.rs:89`–`95`). Same for `HashMapStringHeapValue` (`:110`–`116`). Heap dispatch goes through `HeapValue::kind()`, not a parallel discriminator. **Conforms**, and the comments show the author actively resisting the drift ADR-005 warns about.

### 6.3 ADR-005 §2 String exception — CONFORMS

`ConcreteReturn::String(String)` and `ArrayString(Vec<String>)` use owned `String` at the leaf; the single sanctioned String exception is honored and not extended into unjustified per-type variants. `ArrayStringRows(Vec<Vec<Arc<String>>>)` uses `Arc<String>` for the nested carrier (`:80`), consistent with the interning story.

### 6.4 ADR-006 §2.3 typed `Arc<T>` payloads — CONFORMS

`ConcreteReturn` heap carriers are `DataTable(Arc<DataTable>)`, `IoHandle(Arc<IoHandleData>)`, `OpaqueTypedObject(Arc<HeapValue>)`, `JsonValue(JsonValue)` — typed `Arc<T>`, no `Box<HeapValue>` wrapping. The `OpaqueTypedObject` variant is explicitly justified against the ADR (`:140`–`172`): "the dispatcher projects the `Arc<HeapValue>` directly into a slot via `NativeKind::Ptr(HeapKind::TypedObject)` — `TypedObject` is a **specific** `HeapKind`, NOT wildcard." **Conforms.**

### 6.5 ADR-006 §2.7.10 method ABI (`MethodFnV2`, `&[KindedSlot]`) — CONFORMS at the marshal boundary

The marshal `TypedInvoke` closures take `&[KindedSlot]`-shaped slots and decode via `FromSlot::from_kinded(&slots[i])` (`marshal.rs:1457`), not raw `u64` slices, and not a parallel `&[NativeKind]` side-slice. This matches the §2.7.10 kind-carrier-bound rule. **Conforms.** (The method *registry* itself lives in shape-vm, out of this territory, but the marshal layer's contribution to the ABI conforms.)

### 6.6 ADR-006 §2.7.7 no `Unknown`/`Bool-default` — CONFORMS in territory

No `NativeKind::Unknown` or `Option<NativeKind>` placeholder or Bool-default fabrication in the stdlib bodies. Arg kinds are declared at registration (`arg_kinds = vec![P0::NATIVE_KIND, …]`, `marshal.rs:1448`) — stamped from the `FromSlot` impl's associated const, never fabricated from bits. **Conforms.**

### 6.7 Capability model conformance — PARTIAL (documented but inert by default)

CLAUDE.md's Security-Model §2 says "Every stdlib I/O call guarded by `check_permission()` (~5ns per call)". Reality:

- The guard *exists* at each I/O site (`module_exports.rs:222`) — structurally conformant.
- But `check_permission` (`:226`) is `if let Some(ref granted) = ctx.granted_permissions { … } Ok(())` — a **no-op when `granted_permissions` is `None`**.
- The CLI leaves `granted = None` unless the project config has an explicit `[permissions]` section (`script_cmd.rs:49`–`58`: "only enforce when the user explicitly declared a `[permissions]` section — otherwise stay allow-all (trusted local)").

So a plain `shape run foo.shape` executes with **all permissions granted and no gating** — I confirmed this by writing to `/tmp` with no config and no error (§2.4). This is a deliberate "trusted local" posture (documented in the CLI comment) and matches the "granted_permissions == None, trusted-local `shape run`" note in `executor/control_flow/mod.rs:1093`. It is *conformant to the CLI's stated intent* but **not** conformant to the CLAUDE.md claim that "every stdlib I/O call [is] guarded" in the sense a reader would assume (they are guarded *only under an explicit permissions envelope*). This is a doc-vs-reality gap worth flagging (finding #5) — the security tier is opt-in, not default-on.

### 6.8 capability_tags coverage vs actual I/O surface — INCOMPLETE

`capability_tags.rs:14`–`27` maps exactly six modules to permissions: `io`, `file`, `http`, `env`, `time`, `csv`. Everything else falls through to `PermissionSet::pure()` (`:25`). Missing from the static tag table but genuinely I/O- or entropy-capable:

- `std::core::arrow` — `read_table`/`read_tables`/`metadata` all read files (`arrow_module.rs:53` runtime-gates `FsRead`), but `arrow` has **no** `capability_tags` entry → its compile-time `required_permissions` derivation yields `pure()`, so a `FunctionBlob` that reads an arrow file is content-hashed as requiring no filesystem permission.
- `crypto.random_bytes` / `ed25519_generate_keypair` — consume `Random` entropy (`crypto.rs:195` runtime-gates `Random`), but `crypto` is listed as a pure module (`capability_tags.rs:23`) → no compile-time `Random` derivation.
- The `stdlib_io` surface beyond the coarse `io` mapping (path ops, `mkdir`, `remove`, `rename`, tcp/udp) — `io_permissions` (`:61`) only maps `open/read_file/write_file/tcp_connect/listen/spawn/exec`; the many other `io` exports (`mkdir`, `remove`, `rename`, `read_dir`, `udp_bind`, …) fall through to `pure()`.

Because runtime `check_permission` is the backstop but is itself inert by default (§6.7), these gaps do not currently cause a *security bypass on the default path* (nothing is enforced there anyway). But under an explicit `[permissions]` envelope, the compile-time content-hash permission profile for a program using `arrow.read_table` or `io.mkdir` would **understate** the true capability requirement — the load-time subset check (`execution.rs:278`) would pass a program that then hits the runtime gate. The two layers are inconsistent: runtime gates `arrow`/`crypto.random_bytes` but the static tag table does not. Finding #6.

## 7. Test Coverage In-Territory

### 7.1 Counts

| Location | `#[test]` count | Character |
|----------|-----------------|-----------|
| `stdlib/*.rs` (in-crate units) | 92 | dominated by `virtual_fs.rs` (23), `capability_tags.rs` (20), `runtime_policy.rs` (14), `deterministic.rs` (10) |
| `intrinsics/*.rs` | 41 | numeric kernel correctness |
| `marshal.rs` | 9 | FromSlot/ToSlot round-trips |
| `typed_module_exports.rs` | 0 | — |
| `module_exports.rs` | 0 | — |
| `shape-test/stdlib_modules/*.rs` (integration, Shape source) | 45 (crypto 14, set 14, csv 9, msgpack 8) | end-to-end via `ShapeTest::new(...).with_stdlib()` |

### 7.2 The coverage cliff

The **native codec/parser modules have almost no in-crate tests**: `json.rs`, `http.rs`, `csv_module.rs`, `compress.rs`, `xml.rs`, `arrow_module.rs`, `archive.rs` all have **0** `#[test]`. The module headers explain why (e.g. `http.rs:44`: "Tests deleted along with the legacy ValueWord-based fixtures"; `file.rs:164`: "Behavioural roundtrip tests removed — they used `module.invoke_export` with `ValueWord` arrays (deleted dynamic dispatch entry point). End-to-end coverage through typed-slot dispatch belongs in `shape-test`'s integration suite.").

The intent — move behavioral coverage to `shape-test` integration — is sound. But the migration is **incomplete**: `shape-test/stdlib_modules/` only covers **crypto, csv, msgpack, set** (4 modules, 45 tests). There is **no** integration coverage for `json`, `http`, `compress`, `toml`, `yaml`, `xml`, `unicode`, `regex`, `file`, `env`, `archive`, `arrow`. So those modules currently have effectively **no automated behavioral test** — the schema-shape unit tests that remain (`file.rs:143` `test_file_module_creation`, `test_file_schemas`) only assert that exports are *registered*, not that they *work*. My manual transcripts in §2 are, as far as I can tell, more end-to-end coverage than the automated suite has for compress/toml/yaml/xml/unicode/regex.

### 7.3 Assertion quality where tests exist

Where tests exist they are decent:

- `capability_tags.rs` tests (`:121`–`345`) are thorough: they assert exact permission sets, module-vs-function subset relationships (`function_perms_subset_of_module_perms`, `:302`), and unknown-module fall-through. Good. But they only test the six mapped modules — they cannot catch the arrow/crypto coverage gaps (§6.8) because those modules simply aren't in the table the tests iterate.
- `shape-test/stdlib_modules/crypto_tests.rs` (14 tests) exercises real Shape source through the VM — the right shape.
- `deterministic.rs` (10) and `virtual_fs.rs` (23) are well-covered — these are the sandbox-critical modules and it shows.

### 7.4 The missing test: cross-table method consistency

The single most valuable **absent** test is a consistency check across the three method tables (§5.1). There is no test asserting that `STRING_METHODS`/`NUMBER_METHODS`/… PHF keys (C) match the checker `MethodTable` entries (B) match the `.shape` `extend` declarations (A). Such a test would have caught findings #1–#3 mechanically. Its absence is why "A-final ROOT D" (a prior instance of the same drift, `method_table.rs:697`) was found by hand rather than by CI.

### 7.5 Ignored tests

Zero `#[ignore]` in `stdlib/` or `intrinsics/`. Good — no hidden-failure accumulation in the native tests. (The known `#[ignore]`'d sim tests noted in CLAUDE.md live in `bin/shape-cli/tests/stdlib/simulation.rs`, outside this territory.)

## 8. Book / Docs vs Reality

Book source: `shape-web/book/book-site/src/content/docs/stdlib/`.

### 8.1 Overview page — mostly accurate

`stdlib/overview.mdx` lists native modules, core modules, math sub-modules, domain libraries. Cross-checking against `all_stdlib_modules()` and the `.shape` inventory:

| Book claim | Reality | Verdict |
|------------|---------|---------|
| io, file, json, csv, msgpack, toml, yaml, xml, http, time, env, regex, crypto, unicode, compress, archive native | all registered in `all_stdlib_modules()` | accurate |
| "parallel — planned; bindings not yet wired" | confirmed dead (§2.8) | **honest** |
| "math — bare global builtins + `std::core::math` constants and trig" | confirmed (§2.7) | accurate — and correctly admits the global builtins |
| finance/physics/iot/llm domain libs | present in `stdlib-src/{finance,physics,iot,llm}` | accurate (not exercised live this audit) |

The overview is one of the more honest doc pages I've seen in this codebase — it flags the unfinished `parallel` and openly documents the bare-global-builtin math surface.

### 8.2 Per-module native docs — present for 19 modules

`stdlib/native/` has 19 `.mdx` files (archive, compress, crypto, csv, env, file, http, io, json, log, math, msgpack, parallel, regex, time, toml, unicode, xml, yaml). Coverage of the shipped native surface is good.

### 8.3 String-method docs vs runtime — DOC/REALITY GAP

The book documents `isEmpty()` for collections (`objects-arrays.mdx:346`) and the checker table's comments repeatedly cite "book strings.mdx §Methods" as the source for string method names (`method_table.rs:709`,`:743`). But `"x".isEmpty()` and `"x".chars()` **crash at runtime** (§2.5). So the book documents methods that type-check-then-crash. This is the doc-facet of finding #1/#3: the book (D), the checker (B), and the runtime (C) form a four-way inconsistency for string methods, with the book and checker agreeing against the runtime.

### 8.4 "No global builtins" — memory note contradicted, book correct

Project memory (`feedback_no_global_builtins.md`) says "No global builtins; everything must be namespaced (std::core::math, etc.)". The shipped binary has ~30 global builtins (§2.7). The *book* does not claim namespacing-of-everything; it correctly describes bare math builtins. So the book is accurate and the memory note is aspirational. Auditors reading the memory note should not treat "no globals" as a spec.

### 8.5 CLAUDE.md `format()` shadowing note — CONFIRMED

CLAUDE.md Known-Constraints: "bare `format()` resolves to the global builtin (`intrinsics.shape:138`), not `DateTime.format()`." Confirmed: `format("hello")` → `hello` (§ transcript `fmt.shape`). `classify_builtin_function` maps `"format" => BuiltinFunction::Format` (`helpers.rs:5022`) at `ResolutionScope::ModuleBinding`, so it wins as a global. Accurate.

### 8.6 CLAUDE.md security claim — overstated (see §6.7)

"Every stdlib I/O call guarded by `check_permission()` (~5ns per call)" is *structurally* true but *operationally* misleading: the guard no-ops on the default `shape run` path (no `[permissions]` section → `granted = None` → allow-all). A reader would reasonably conclude untrusted code is gated by default; it is not. The gate is real but opt-in.

## 9. Bugs & Correctness Risks

Severity key: **P0** = unsound / wrong-results / security bypass; **P1** = broken feature; **P2** = paper cut.

### B1 (P1) — String methods `isEmpty`/`chars` type-check then crash

Repro:

```
$ cat t09.shape
let s = "hello"
print(s.isEmpty())
$ shape run -m vm t09.shape
Error: Runtime error: no method 'isEmpty' on receiver kind String (line 2)
```

Root cause: `method_table.rs:705` declares `("isEmpty", vec![], boolean())` for string; `STRING_METHODS` PHF (`method_registry.rs:906`) lacks an `isEmpty` arm for string (it exists for array/hashmap/set/deque/pq at `:286`,`:468`,`:503`,`:531`,`:550` but not string). Same for `chars` (`method_table.rs:749`). This directly violates the language's "if it type-checks it runs" contract. Fix: add `isEmpty`/`chars` to `STRING_METHODS` (or generate all three tables from one source, §12). Effort: **S** (add two PHF arms + handlers) or **L** (do the single-source refactor).

### B2 (P1) — Scalar `number`/`int` methods unreachable via method syntax

Repro:

```
$ shape run -m vm  (print((3.7).floor()))
error[SEMANTIC]: Method 'floor' not found on type 'number'
$ shape run -m vm  (print((-5).abs()))
error[SEMANTIC]: Method 'abs' not found on type 'int'
```

Root cause: no `number`/`int` section in the checker `MethodTable` (`method_table.rs` sections `:374`–`:1075` are Vec/Table/string/HashMap/Set/Deque/PQ/Mutex/Range/Iterator only), despite `NUMBER_METHODS` PHF (`method_registry.rs:871`–`875`) implementing floor/ceil/round/abs/sign and `number_methods.shape:8`–`11` declaring them. Only `toString`/`toNumber` resolve. Workaround exists (global `floor(x)`/`abs(x)`), so this is completeness rather than soundness, but it silently makes `x.abs()` uncompilable across the whole language. Fix: seed a `number`/`int` section into `MethodTable`. Effort: **S**.

### B3 (P1) — Book/checker document methods the runtime stubs out

`"x".toBool()` → `Not implemented: phase-1b-vm-wave-5c-conversion: ToBool body migration … pending (v0.4)`; `"x".graphemes()` → `Not implemented: String.graphemes: SURFACE — V3-S5 ckpt-6`. These are *marked* pending in the runtime, but they still type-check (checker accepts them) and are documented, so a user hits a `Not implemented` abort at runtime for a documented method. Distinct from B1 (unmarked drift) — these are known-incomplete but not surfaced to the type checker as unavailable. Fix: either finish the impls or remove from the checker table until they land. Effort: **S** (remove from checker) / **M** (finish impls).

### B4 (P2) — `json.rs` `panic!` on internal invariant in parse hot path

`json.rs:212`: `panic!("build_json_enum_heap_value must return TypedObject, got {:?}", other.kind())` inside object construction during `json::parse`. If the recursive helper ever returns a non-`TypedObject` (future refactor bug, or an unhandled `serde_json::Value` shape), a user's `json::parse` of untrusted input aborts the VM instead of returning `Err`. Fix: convert to `return Err(...)`. Effort: **S**. Severity P2 only because the invariant currently holds; the risk is future-refactor + untrusted-input blast radius.

### B5 (P2) — capability_tags understates arrow / crypto-random permission requirements

`capability_tags::required_permissions("std::core::arrow", "read_table")` returns `pure()` (falls through `:25`) even though `arrow_module.rs:53` reads files and runtime-gates `FsRead`. Likewise `crypto` is a "pure" module (`:23`) but `crypto.random_bytes` consumes `Random`. Under an explicit `[permissions]` envelope, the compile-time content-hash permission profile understates the true requirement; the load-time subset check (`execution.rs:278`) would admit a program that then hits the runtime gate — an *inconsistency* between the compile-time and runtime capability layers. Not a bypass on the default path (nothing is enforced there). Fix: add `arrow`→FsRead and `crypto.random_bytes`→Random to `capability_tags`. Effort: **S**.

### B6 (P2) — `parallel` and `file.read_bytes`/`write_bytes` are dead declarations

`use std::core::parallel; parallel::map(...)` → `module 'parallel' has no export 'map'`; `file::read_bytes(...)` → `module 'file' has no export 'read_bytes'`. The `.shape` `builtin fn` declarations over-promise vs. the wired native surface. `parallel` is book-flagged as planned; `file` bytes are deferred (`stdlib/file.rs:11`). Fix: either wire the exports or remove the stale `.shape` declarations so imports fail-fast with a clearer "not implemented" rather than "no export". Effort: **M** (wire) / **S** (remove decls).

### B7 (P2) — Return-type convention inconsistency is a user footgun

`compress::gzip -> Array<int>` and `csv::parse -> Array<Array<string>>` (bare) vs `toml/yaml/xml/msgpack::parse -> Result<...>`. A user who wraps a bare-return call in `match Ok/Err` gets a runtime `No match arm matched the value` abort (I hit this twice, §2.3). The stdlib should adopt a principled rule (fallible parse → `Result`; infallible transform → bare) and `csv::parse` in particular should arguably return `Result` given `csv::is_valid` exists. Fix: convention doc + selective signature changes. Effort: **M** (breaking).

### B8 (P2) — `regex::match`/`find` copy-paste divergence risk

`regex.rs:40`–`82`: two byte-identical bodies. A bug fixed in one won't propagate to the other. No current divergence, but a latent one. Fix: shared helper. Effort: **S**.

### B9 (observation, not a bug) — idiomatic stdlib use deopts the JIT

Every `Result`/`Option`-returning stdlib call consumed via `match Ok(_)/Err(_)` triggers the whole-program `[jit-fallback]` deopt (§5.4, tracked v0.4). Correctness is preserved (interpreter runs), but the *performance* narrative for idiomatic stdlib code under default `-m jit` is "interpreter until v0.4". Not a stdlib bug — flagged for cross-vertical awareness.

## 10. What Is Done Well

### 10.1 The two-tier `TypedReturn`/`ConcreteReturn` marshal design

This is the standout. Making `ConcreteReturn` a **leaf-only** enum and `TypedReturn` a wrapper that can only hold a `ConcreteReturn` means "nesting `TypedReturn` inside `TypedReturn` is unrepresentable, which … makes the long-deleted `TypedReturn::ValueWord` escape hatch unreachable from any container variant" (`typed_module_exports.rs:184`–`190`). The *type system* enforces the strict-typing discipline the CLAUDE.md Forbidden-Patterns section is worried about. This is exactly the "make illegal states unrepresentable" move, and it is documented with the reasoning. Other verticals should copy this shape.

### 10.2 Single-discriminator discipline actively defended in comments

The `ArrayHeapValue`/`HashMapStringHeapValue` variants have multi-paragraph comments explaining why per-element-kind variants are *refused* (`typed_module_exports.rs:89`–`116`), citing `docs/defections.md`. The author is not just conforming to ADR-005 §1 — they are documenting the temptation and the refusal inline, which is exactly the discipline the Forbidden-Patterns section asks for.

### 10.3 Provider-injection seam for the filesystem

`create_file_module_with_provider(fs: Arc<dyn FileSystemProvider>)` (`stdlib/file.rs:30`) with `RealFileSystem`/`VirtualFilesystem`/`PolicyEnforcedFs` implementations means sandbox/VFS mode is transparent to the module body — the body calls `fs.read(path)` and the provider decides real-disk vs in-memory vs policy-checked. This is a clean, testable seam (and `virtual_fs.rs` has 23 tests exercising it). Deterministic mode gets the same treatment (`deterministic.rs`: seeded ChaCha8 + virtual clock).

### 10.4 Zero live forbidden patterns

For a codebase whose CLAUDE.md documents a multi-session history of `ValueWord`/dynamic-dispatch re-introduction, finding **zero** live forbidden symbols in a 12k-LOC native stdlib territory is a genuine achievement. The deleted mechanisms are referenced only by name in deletion-fate comments, exactly as the guidance requires.

### 10.5 Crypto correctness

All spot-checked crypto vectors are correct (§2.1). Using vetted crates (sha2, md5, hmac, base64, ed25519) rather than hand-rolling, with `random_bytes` bounded (`crypto.rs:198`: "n must be between 0 and 65536") — sensible defensive limits.

### 10.6 Honest book on unfinished surface

The book flags `parallel` as "not yet wired" and documents the bare-global math builtins accurately (§8.1). Honest docs on unfinished features are rarer than they should be.

## 11. What Is Done Poorly / Tech Debt

### 11.1 The method-table triplication (the debt that keeps recurring)

Three hand-maintained sources of truth for one concept (§5.1), with the checker's own comments recording a prior drift instance ("A-final ROOT D", `method_table.rs:697`) and *no* mechanical cross-check. This is not a one-off bug; it is a **defect factory**. Every method added to any type must be added in three places (`.shape`, `method_table.rs`, `method_registry.rs`) or it half-works. The `.shape` `extend` blocks may even be vestigial for the strict checker (§3.5). This is the highest-leverage cleanup in the vertical.

### 11.2 Marshal arity boilerplate (2,904-line `marshal.rs`)

21 near-identical `register_typed_fn_*` variants (§4.2). Correct but a large maintenance surface — every marshal-contract change is a 21-site edit. A `macro_rules!` generator is the standard remedy.

### 11.3 Over-detailed historical comments

Many stdlib module headers carry 40+ lines of migration archaeology: `http.rs:1`–`47` (Stage C/D/N4/N7 sign-offs, commit hashes, supervisor-relay dates), `typed_module_exports.rs` variant docs reference `docs/defections.md` clusters and Wave numbers. This provenance is *valuable* for understanding *why* the code is shaped as it is, but it also (a) signals a very high churn history and (b) raises the reading tax for a new maintainer who must parse "Wave 2 Round 3b C2-joint ckpt-4" (`json.rs:199`) to understand a HashMap construction. Consider moving deep provenance to `docs/` and keeping headers to the essentials.

### 11.4 Behavioral test coverage gap for native codecs

json/http/compress/toml/yaml/xml/unicode/regex/file/env/archive/arrow have **no** behavioral automated test (§7.2) — the ValueWord-era tests were deleted and the shape-test migration only covered crypto/csv/msgpack/set. The remaining unit tests assert *registration*, not *behavior*. My manual §2 transcripts exceed the automated behavioral coverage for several modules. This is real debt: a regression in `compress::gunzip` or `toml::parse` would not be caught by CI.

### 11.5 Return-type conventions are unprincipled

`Result` vs bare-value is inconsistent across parse/transform functions (§5.5, B7), producing user-facing `No match arm matched` aborts when the convention is guessed wrong.

### 11.6 `panic!` on a parser hot path

`json.rs:212` (§B4). A parser over untrusted input should never `panic!` on an internal invariant.

### 11.7 Capability model: opt-in but documented as default

The gap between "every I/O call is guarded" (CLAUDE.md) and "guard no-ops unless `[permissions]` declared" (`script_cmd.rs:49`) plus the incomplete static tag table (arrow/crypto/io-extras missing, §6.8) is debt: the security tier looks more complete on paper than in default operation. This is arguably fine as a *design* (trusted-local default) but the docs should say so plainly and the tag table should at least be *complete* so that the opt-in path is sound.

### 11.8 Dead `.shape` declarations

`parallel` (whole module), `file.read_bytes`/`write_bytes` — declarations without bindings (§B6). They fail with a confusing "no export" rather than "not implemented".

## 12. Prioritized Recommendations

### P0 — none

No unsoundness, wrong-results, or security *bypass* found in the territory. (The capability model is inert-by-default but that is a documented design choice, not a bypass — nothing claims to be enforcing and then fails to; §6.7.)

### P1

1. **Unify the method tables into one generated source of truth.** (Addresses B1, B2, B3, §5.1, §11.1.) Pick one canonical table (the `.shape` `extend` blocks are the natural choice given they are already "canonical" per comment) and *generate* both the checker `MethodTable` seed and validate the runtime PHF against it (or generate the PHF too). Minimum viable step: add a `#[test]` that asserts, per receiver type, that the set of methods in `method_table.rs` equals the set of PHF keys in `method_registry.rs` (and flags stubs). This alone would have caught findings #1–#3. Effort: **M** for the consistency test; **L** for full generation. *This is the single most important recommendation in the vertical.*

2. **Fix the concrete string/number drift now** (interim, before the unification lands): add `isEmpty`/`chars` to `STRING_METHODS` (B1), add a `number`/`int` section to the checker `MethodTable` (B2), and remove `toBool`/`graphemes` from the checker table until their runtime impls land — or finish them (B3). Effort: **S** each.

### P2

3. **Complete the `capability_tags` table** (B5): add `arrow`→FsRead, `crypto.random_bytes`→Random, and the missing `io` exports (mkdir/remove/rename/read_dir/udp*). Add a test that every module in `all_stdlib_modules()` with a runtime `check_permission` call also has a matching static tag entry. Effort: **S–M**.

4. **Replace the `json.rs` `panic!` with `Err`** (B4). Effort: **S**.

5. **Resolve dead declarations** (B6): either wire `parallel`/`file.read_bytes` or delete the stale `.shape` declarations so failures are clear. Effort: **S** (delete) / **M** (wire).

6. **De-duplicate `regex::match`/`find`** into a shared helper (B8). Effort: **S**.

7. **Collapse marshal arity boilerplate** with a `macro_rules!` generator (§4.2, §11.2). Effort: **M**.

8. **Adopt a return-type convention** and align the parse functions (`csv::parse` → `Result`) (B7, §5.5). Effort: **M** (breaking — schedule with a version bump).

9. **Backfill behavioral integration tests** for json/http/compress/toml/yaml/xml/unicode/regex/file/env/arrow in `shape-test/stdlib_modules/` (§7.2, §11.4). The §2 transcripts here are a ready-made starting corpus. Effort: **M**.

10. **Clarify the security-posture docs** (§6.7, §8.6): state plainly that the runtime capability gate is opt-in (`[permissions]`-gated) and that plain `shape run` is allow-all trusted-local. Effort: **S**.

### Effort legend
- **S** = < half a day; **M** = 1–3 days; **L** = multi-day / cross-cutting.

## 13. Appendix — Method & Evidence Index

All scratch programs under `/tmp/.../scratchpad/verticals/runtime-stdlib/`. Key transcripts referenced:

- `t08.shape` — crypto vectors (§2.1)
- `t04_json.shape` — json round-trip (§2.2)
- `t03/t05b/t07b/z.shape` — parser/codec modules (§2.3)
- `t01b_file.shape` — file text + JIT-fallback observation (§2.4, §5.4)
- `sm.shape` sweep — string method drift (§2.5)
- `x.shape`/`y.shape` — number method drift + global math builtins (§2.6, §2.7)
- `t06.shape` — bare vs namespaced math convergence (§2.7)
- `par.shape`/`t01_file.shape` — dead declarations (§2.8)
- `g2/g4.shape` — intrinsic gating + `as` casts (§2.10)
- `t09.shape` — `isEmpty` runtime crash (§B1)

Key source anchors:

- Marshal: `marshal.rs:1400`–`2600`; carriers `typed_module_exports.rs:55`,`:196`,`:267`
- Native registry: `stdlib/mod.rs:37`
- Capability: `capability_tags.rs:14`; runtime gate `module_exports.rs:222`; CLI wiring `script_cmd.rs:49`
- Intrinsic gating: `helpers.rs:5155`,`function_calls.rs:2661`
- Global builtins: `helpers.rs:4961`–`5135`
- Method tables (the three): `stdlib-src/core/*_methods.shape`, `type_system/checking/method_table.rs:695`+, `shape-vm/.../method_registry.rs:871`,`:906`


## 14. Supplementary Deep Empirical Sweep

This section adds breadth-of-evidence gathered after the core analysis, and surfaces three additional findings that materially affect the completeness score.

### 14.1 `env` module — `get`/`all`/`args` are dead (P1, book-honest)

The single most consequential dead-declaration cluster. `env.shape` declares seven functions:

```
env.shape:35  pub builtin fn get(name: string) -> _;
env.shape:52  pub builtin fn has(name: string) -> bool;
env.shape:65  pub builtin fn all() -> HashMap<string, string>;
env.shape:78  pub builtin fn args() -> Array<string>;
env.shape:91  pub builtin fn cwd() -> string;
env.shape:104 pub builtin fn os() -> string;
env.shape:117 pub builtin fn arch() -> string;
```

But native `env.rs` registers only **four** (`has`, `cwd`, `os`, `arch` — `env.rs:34`,`:50`,`:65`,`:79`). Empirical:

```
env::get("HOME") => error[SEMANTIC]: module 'env' has no export 'get'
env::all()       => error[SEMANTIC]: module 'env' has no export 'all'
env::args()      => error[SEMANTIC]: module 'env' has no export 'args'
env::cwd()       => ok
```

So **reading an environment variable's value (`env::get`) and reading command-line arguments (`env::args`) do not work** — these are not exotic functions. Compounding the inconsistency, `capability_tags.rs` *does* map all of `get/has/all/args/cwd` to `Permission::Env` (`:88`,`:194`,`:330`) — the capability table believes functions exist that the runtime never registers. And the checker accepts the calls far enough to emit "no export" at resolution rather than "unknown function".

The **book is honest** about this (`stdlib/native/env.mdx:81`–`85`): "The `env` stdlib source also declares `env::get`, `env::all`, and `env::args`, but their native bindings are not wired up at HEAD — calling them is a compile error … Until they ship, use `env::has` to test." Credit to the book. But an honest doc does not make the feature present: `env::get`/`args` are core operations that are simply missing, and their `.shape` declarations + capability entries are stale scaffolding for an unfinished wire-up. This should be raised from the P2 "paper cut" framing of B6 — for `env` it is a **P1 missing-core-feature**.

### 14.2 Option / Result methods — entire surface missing from the checker (P1)

`option_methods.shape` and `result_methods.shape` declare the canonical monadic surface:

```
option_methods.shape:  unwrap unwrapOr isSome isNone map
result_methods.shape:  unwrap unwrapOr isOk isErr map mapErr
```

Every one of them fails to type-check:

```
Some(5).isSome()      => error[SEMANTIC]: Method 'isSome' not found on type 'Option'
Some(5).unwrap()      => error[SEMANTIC]: Method 'unwrap' not found on type 'Option'
Some(5).unwrapOr(0)   => error[SEMANTIC]: Type constraint violation: Generic { Option<int> } cannot have fields
Some(5).map(|x| x+1)  => error[SEMANTIC]: Type constraint violation: Generic { Option<int> } cannot have fields
Ok(10).unwrapOr(0)    => error[SEMANTIC]: Method 'unwrapOr' not found on type 'Result'
```

Root cause is the same as B2: the checker `MethodTable` (`method_table.rs`) has **no `Option` and no `Result` section** (confirmed: its section list is Vec/Table/string/HashMap/Set/Deque/PriorityQueue/Mutex/Range/Iterator only). So `.map()`, `.unwrapOr()`, `.isSome()` — the ergonomic core of `Option`/`Result` in every ML-family language — are unreachable as methods. The only way to consume an `Option`/`Result` is `match` (which works) or the `?` operator / `??` null-coalescing.

This is a **major ergonomics/completeness hole**. It is arguably worse than the number-method gap because `Option`/`Result` are used pervasively (every stdlib fallible call returns one), so the missing `.unwrapOr(default)` forces a `match` at every call site. It escalates the method-table split-brain (§5.1) from "some string/number methods drift" to "two of the language's foundational generic types have no working method surface at all."

Note the two distinct error shapes: named methods that the checker knows the *name* of but not for this type give "Method 'X' not found on type 'Option'"; methods invoked in a field-access-like position give the lower-level "Generic { Option<int> } cannot have fields" — i.e. the checker fell through to the HasField fallback, exactly the "#1 strict false-positive class" its own comment describes (`method_table.rs:198`–`209`).

### 14.3 HashMap `keys`/`values` — surface-stubbed (P1, marked)

```
m.keys()   => Not implemented: HashMap.keys: SURFACE — V3-S5 ckpt-5 ... Rebuild lands at ckpt-6
m.values() => Not implemented: HashMap.values: SURFACE — V3-S5 ckpt-6
```

`m.get`, `m.set`, `m.has`, `m.len`, `m.isEmpty` all work (verified). But `keys()`/`values()` type-check and then abort at runtime with a V3-S5 ckpt-6 surface stub (the same `Arc<TypedArrayData>`-deletion cascade that stubs `String.graphemes` in §2.5). These are **marked** pending (like `toBool`/`graphemes`, class B3), so they are known-incomplete rather than silent drift — but they still type-check, so a user iterating a map's keys hits a runtime `Not implemented`. Iterating a HashMap by keys is a basic operation; its absence is felt.

### 14.4 Working-surface confirmation sweep (for balance)

To be fair to the vertical, a large surface *does* work end-to-end. Confirmed live under `-m vm`:

| Surface | Result |
|---------|--------|
| `crypto::sha512("abc")` | `ddaf35a1…54ca49f` (correct vector) |
| `crypto::hex_decode("414243")` | `Ok("ABC")` |
| `crypto::random_bytes(8)` | 16-hex-char string (8 bytes hex-encoded) |
| `crypto::ed25519_generate_keypair()` | succeeds |
| `compress::zstd/unzstd` round-trip | `true` |
| `compress::deflate/inflate` round-trip | `true` |
| `yaml::parse_all` (multi-doc) | `multi_ok` |
| `yaml::is_valid` | `true` |
| `unicode::normalize/is_digit/is_letter` | correct |
| `HashMap` get/set/has/len/isEmpty | correct |
| `Set` add/len/includes/isEmpty (dedup) | `2/true/false` (dup collapsed) |
| Array `map/filter/reduce/first/reverse/sort/slice/some/join/sum/len` | all correct (§2, one arg-order footgun on `reduce`) |
| `env::has/cwd/os/arch` | correct |
| `String` trim/toUpperCase/split/padStart/isAscii/codePointAt | correct |

The array-method surface in particular is solid and complete — the drift is concentrated in **scalar (`number`/`int`)**, **`Option`/`Result`**, **a few `string` methods**, and **`HashMap.keys`/`values`**.

### 14.5 Revised completeness read

Factoring in §14.1–14.3, the earlier feature-completeness score of 72 is, if anything, slightly generous — the missing `env::get`/`args`, the absent `Option`/`Result` method surface, and stubbed `HashMap.keys`/`values` are all *frequently-needed* operations. The native codec/crypto/compress core remains genuinely strong, which keeps the score in the low-70s rather than lower. The pattern is consistent: **native Rust module bodies are complete and correct; the Shape-facing method/declaration surface over-promises and under-delivers wherever it depends on the hand-maintained checker mirror or on a not-yet-wired native export.**

### 14.6 Consolidated "declared-but-dead" and "type-checks-but-crashes" inventory

Single table of every discrepancy found (the actionable core of this audit):

| Surface | Declared where | Checker (B) | Runtime (C) | Symptom | Class |
|---------|----------------|-------------|-------------|---------|-------|
| `env::get` | env.shape:35 + captags | resolves | **no export** | `module 'env' has no export 'get'` | dead |
| `env::all` | env.shape:65 + captags | resolves | **no export** | compile error | dead |
| `env::args` | env.shape:78 + captags | resolves | **no export** | compile error | dead |
| `file::read_bytes` | file.shape:106 | resolves | **no export** | `no export 'read_bytes'` | dead (deferred) |
| `file::write_bytes` | file.shape:124 | resolves | **no export** | compile error | dead (deferred) |
| `parallel::*` | parallel.shape | resolves | **no module** | `no export 'map'` | dead (planned) |
| `String.isEmpty` | method_table.rs:705 | ✓ | ✗ | runtime `no method` | **drift (unmarked)** |
| `String.chars` | method_table.rs:749 | ✓ | ✗ | runtime `no method` | **drift (unmarked)** |
| `String.toInt/toFloat` | STRING_METHODS PHF | ✗ | ✓ | `Method not found` | **drift (unmarked)** |
| `String.toBool` | method_table.rs:747 | ✓ | stub | `Not implemented v0.4` | stub (marked) |
| `String.graphemes` | method_table.rs | ✓ | stub | `Not implemented ckpt-6` | stub (marked) |
| `number.floor/abs/ceil/round/sign` | number_methods.shape + NUMBER_METHODS PHF | ✗ | ✓ | `Method not found on type 'number'` | **drift (unmarked)** |
| `int.abs` | int_methods.shape | ✗ | ✓/PHF | `Method not found on type 'int'` | **drift (unmarked)** |
| `Option.isSome/isNone/unwrap/unwrapOr/map` | option_methods.shape | ✗ | ? | `Method not found` / `cannot have fields` | **drift (whole surface)** |
| `Result.isOk/isErr/unwrap/unwrapOr/map/mapErr` | result_methods.shape | ✗ | ? | `Method not found on type 'Result'` | **drift (whole surface)** |
| `HashMap.keys/values` | method_table.rs + HASHMAP_METHODS | ✓ | stub | `Not implemented ckpt-6` | stub (marked) |

The "drift (unmarked)" rows are the dangerous ones — they type-check-then-crash (B→C) or block working runtime methods (C→B) with **no** pending marker to warn the user, and would all be caught by the single cross-table consistency test recommended in §12.1. The "stub (marked)" rows are known-incomplete with tracking (V3-S5 ckpt-6). The "dead" rows are unwired declarations; `env::get`/`args` are the ones that hurt.

### 14.7 Note on the surface-stub family (V3-S5 ckpt-6)

`String.graphemes`, `HashMap.keys`, `HashMap.values` all abort with the same root message: the `Arc<TypedArrayData>` result carrier was deleted at V3-S5 ckpt-1..ckpt-4 and the rebuild "lands at ckpt-6 STRICT close per the per-T v2-raw `TypedArray<T>` carrier shape." This is a coherent, tracked deletion-then-rebuild — the methods that return a *freshly-allocated typed array of heap elements* (grapheme list, key list, value list) are the ones blocked, because that specific carrier shape is mid-migration. This is legitimate in-flight work (not a defection), but it means "give me the keys of this map" is currently a runtime abort. When ckpt-6 lands, re-test this whole family. The refusal note ("REFUSED ON SIGHT: TypedArrayData resurrection under any rename") is exactly the Forbidden-Patterns discipline being enforced at the surface-stub level — good hygiene, but the user-facing consequence is a broken `keys()` until the strict carrier is rebuilt.

## 15. The Marshal Layer (`marshal.rs`, `module_exports.rs`) — Deep Dive

The marshal layer is the load-bearing seam between native Rust bodies and the typed VM slot ABI. It deserves a dedicated deep-dive because it is both the best-engineered part of the vertical and the largest concentration of boilerplate.

### 15.1 `FromSlot` / `ToSlot` traits

`FromSlot` (`marshal.rs:43`) and `ToSlot` (`:101`) each carry an associated `const NATIVE_KIND: NativeKind`. This is the mechanism that makes the whole registration path strict: a native function's argument kinds are computed as `vec![P0::NATIVE_KIND, P1::NATIVE_KIND, …]` (`marshal.rs:1448`) directly from the parameter types' `FromSlot` impls — the kind is **stamped from the type**, never fabricated from bits. This is exactly the ADR-006 §2.7.7 discipline ("Stamped at compile time; never fabricated from raw bits") realized at the FFI-registration boundary.

Counted **20 `impl FromSlot for`** and **13 `impl ToSlot`** blocks. The `FromSlot` coverage:

- Scalars: `i64` (`:157`, `Int64`), `f64` (`:165`, `Float64`), `bool` (`:203`, `Bool`), `Arc<String>` (`:217`, `String`).
- Nullable scalar: `Option<f64>` (`:176`, `NullableFloat64`) — a genuinely nice touch, letting a native fn accept an optional number without a heap box.
- Heap handles: `TypedObjectPtr` (`:260`), `Arc<DataTable>` (`:322`), `Arc<IoHandleData>` (`:370`) — each keyed to its specific `HeapKind`, never a wildcard.
- Typed arrays: `Vec<u8>` (`:454`, projected from a `TypedArray` heap pointer), `Vec<i64>` (`:476`), plus f64/string array forms.

Every heap `FromSlot` uses `NativeKind::Ptr(specific HeapKind)` — no `Ptr(Any)`, no dynamic probe. This is the single-discriminator discipline (ADR-005 §1) enforced at the argument-decode boundary. When a body declares `P0 = Arc<DataTable>`, the arg kind is `Ptr(HeapKind::DataTable)` and `from_kinded` fails cleanly if the slot carries anything else — no `is_heap()` probe, no tag decode.

### 15.2 The `install` funnel

All 21 arity variants converge on a single `install(module, name, description, params, return_type, arg_kinds, invoke)` call (e.g. `marshal.rs:1466`). This is good: the *policy* (how a `TypedInvoke` is registered, schema built, arg-kinds recorded) lives in one place; the 21 variants only differ in *arity-specific unpacking*. So the boilerplate (§4.2) is shallow — it is 21 copies of "check arg count, unpack N slots, call body" wrapped around one shared `install`. A macro would remove the copies without changing the architecture.

### 15.3 `MarshalError`

Arg-count and kind mismatches surface as `MarshalError::ArgCount { expected, got }` (`marshal.rs:1452`) converted into the VM error channel — typed, not stringly. Good.

### 15.4 `ModuleContext` and the permission seam

`module_exports.rs` defines `ModuleContext` carrying `granted_permissions: Option<PermissionSet>` and `scope_constraints`. The three gate functions — `check_permission` (`:222`), `check_fs_permission` (`:244`, adds glob path-scope), `check_net_permission` (`:278`, adds host/port scope with `*.example.com` wildcard support `:292`) — are the runtime capability enforcement points. As established in §6.7 they no-op when `granted_permissions` is `None`. The path-scope matcher (`:254`) is a prefix match after stripping `**`/`*` — coarse but functional (`/data/**` matches anything under `/data/`). The host-scope matcher (`:289`) handles exact host and `*.domain` wildcard. These are reasonable, if simple, and their 20 `capability_tags.rs` tests plus the `check_fs_permission` scope logic are the best-tested security surface in the vertical.

### 15.5 Assessment

The marshal layer is the reference implementation of "strict typing all the way to the FFI boundary" that the rest of the codebase should emulate. Its faults are cosmetic (arity boilerplate) not architectural. If every subsystem marshaled values the way `marshal.rs` does, the Forbidden-Patterns section of CLAUDE.md would have far less to worry about.

## 16. Sandbox Provider Modules (`runtime_policy`, `virtual_fs`, `deterministic`) — Deep Dive

These three modules (1,336 LOC combined) implement the sandboxing substrate that the capability model and the `file`/`io` modules sit on. They are among the best-tested code in the territory (`virtual_fs` 23 tests, `runtime_policy` 14, `deterministic` 10).

### 16.1 `FileSystemProvider` abstraction (`runtime_policy.rs:143`)

An 8-method trait — `read`, `write`, `append`, `exists`, `remove`, `list_dir`, `metadata`, `create_dir_all`. Three implementations:

- `RealFileSystem` (`:167`) — delegates to `std::fs`. The default.
- `PolicyEnforcedFs` (`:232`) — wraps another provider and enforces `RuntimePolicy` (path scope + read/write permission) before delegating.
- `RoutingFileSystem` (`:315`) — routes paths to different providers (e.g. `/vfs/*` → in-memory, everything else → real).

This is a clean, composable provider stack. The `file` module (`file.rs:30`) takes `Arc<dyn FileSystemProvider>`, so swapping in the VFS or the policy-enforced provider is transparent to the module body — the body just calls `fs.read(path)`. This is the seam that makes "sandbox mode works transparently" (from the module doc-comment) actually true.

### 16.2 `VirtualFilesystem` (`virtual_fs.rs`)

An in-memory `FileSystemProvider` backed by `RwLock<HashMap<PathBuf, VfsEntry>>` (`:28`) plus a `read_only: RwLock<HashSet<PathBuf>>` set. The host can pre-seed read-only files before execution and extract written files after — the intended shape for a sandbox that must not touch disk. Thread-safe. The 24 production `.read()/.write().unwrap()` lock-poison unwraps (§3.3) are standard. 23 tests cover the provider surface. This is solid, and it is what makes the `Vfs` permission (one of the 16) meaningful.

### 16.3 `DeterministicRuntime` (`deterministic.rs`)

A `ChaCha8Rng` seeded PRNG plus a virtual clock that starts at 0 and advances a fixed increment per read (`:13`, default 1ms). API: `next_random_f64`, `next_random_range`, `current_time_ms`, `advance_clock`, `reset`, `seed` (`:53`–`:84`). "same code + same seed = identical output" (module header). When `sandbox.deterministic = true`, the VM routes `time.millis()` and random through this instead of system sources. This is the substrate for the `Deterministic` permission and the reproducible-execution story. 10 tests. The design is textbook (ChaCha8 is a standard reproducible CSPRNG; virtual monotonic clock is the right model). One thing worth noting for the *distributed* vertical: the deterministic-foreign gate (`deterministic_foreign_gate`, referenced from `execution.rs`) refuses foreign code under determinism because extension boundaries can't be attested deterministic — the right call, and the reason `Deterministic` is surfaced into the granted set even for otherwise-full-trust runs (`script_cmd.rs:73`).

### 16.4 Assessment

The sandbox substrate is genuinely well-built and well-tested — the *mechanisms* (VFS, deterministic runtime, policy-enforced provider, scope constraints) are all present and correct. The gap (§6.7) is purely that the *default CLI path* does not engage them (allow-all trusted-local). So the sandbox is a capable engine that is idling in neutral unless a `[permissions]`/`[sandbox]` config turns it on. For an embedding host (shape-app, shape-server) that *does* set `granted_permissions`, this substrate is production-shaped.

## 17. Intrinsics Correctness Spot-Checks

Numeric intrinsics back the `math`/`vec`/`stochastic`/`distributions` Shape modules. Spot-checked statistics against textbook values:

```
math::mean([2,4,4,4,5,5,7,9])      => 5.0    (correct)
math::std([2,4,4,4,5,5,7,9])       => 2.0    (correct — population std of the canonical σ=2 dataset)
math::variance([2,4,4,4,5,5,7,9])  => 4.0    (correct — σ²=4)
math::percentile([1,2,3,4,5], 50)  => 3.0    (correct — median)
math::median([1,2,3,4,5])          => 3.0    (correct)
```

The classic {2,4,4,4,5,5,7,9} dataset (mean 5, variance 4, std 2) round-trips exactly, giving confidence the statistical intrinsics (`intrinsics/statistical.rs`, `intrinsics/math.rs`) are correct, not just present. The intrinsics crate also carries 41 in-crate `#[test]`s — the best native-test coverage in the territory, appropriate for numeric kernels where correctness is subtle. The AVX2 matmul kernels (`matrix_kernels.rs`, 637 LOC, SIMD unsafe §3.3) are the highest-risk intrinsic surface; they were not exercised live in this audit (would need a matrix-heavy Shape program) but are unit-tested in-crate.

## 18. Cross-Cutting Observations & Final Notes

### 18.1 The consistent pattern

Every problem in this vertical reduces to one of two shapes:

1. **Native body complete, Shape-facing surface incomplete/drifted.** (env::get dead, Option/Result methods missing, string isEmpty drift, number methods unreachable.) The Rust that *does the work* is almost always present and correct; the *declaration/checker/PHF surface* that exposes it is where things break.
2. **Mechanism present, default path doesn't engage it.** (Capability gates real but inert; sandbox providers capable but idle under plain `shape run`.)

Neither shape is an unsoundness — the strict-typing core holds, the marshal layer is clean, crypto/compress/codecs are correct. But both shapes produce a real user experience of "the docs/types say X works, and it doesn't." For a statically-typed language whose pitch is "if it type-checks it runs," the type-checks-then-crashes rows in §14.6 are the most damaging, because they break the core promise silently.

### 18.2 What a maintainer should do first

The §12.1 recommendation (single-source method table + cross-table consistency test) would mechanically eliminate every "drift (unmarked)" row in §14.6 at once and prevent recurrence — a much better return than hand-patching `isEmpty` then `floor` then `unwrapOr` one at a time (which is how "A-final ROOT D" happened). Second, wire `env::get`/`args` (§14.1) — they are core and their absence is disproportionately felt. Third, complete the `capability_tags` table (§6.8) so the opt-in security path is at least *sound* when engaged.

### 18.3 Scope note

This audit stayed inside `/home/dev/dev/shape-lang/shape/` and the named book sibling. The method_registry.rs and BuiltinFunction dispatch live in shape-vm (adjacent vertical) but were read because the method-table split-brain spans the runtime↔vm boundary; findings about the PHF tables are reported here because their *content* (which methods exist) is the stdlib surface, while their *dispatch mechanism* belongs to the VM vertical. No stale worktree or orphan `crates/` copy was read.

### 18.4 One-line health

Native stdlib core: strong and correct. Shape-facing method/declaration surface: drifted and partially dead. Security substrate: capable but off by default. Fix the three-way method table and wire env::get, and this vertical jumps from "good core, leaky surface" to "solid."

## 19. Appendix B — Native Module Function Reference (as wired at HEAD)

The authoritative list of what is *actually registered* natively (not merely declared in `.shape`), from the `create_*_module` bodies. Return-type conventions noted to document the §5.5 inconsistency.

| Module | Wired exports | Return convention notes |
|--------|---------------|-------------------------|
| `crypto` (13) | sha1, sha256, sha512, md5, hmac_sha256, base64_encode, base64_decode, hex_encode, hex_decode, random_bytes, ed25519_generate_keypair, ed25519_sign, ed25519_verify | hashes → bare `string`; base64/hex_decode → `Result<string>`; random_bytes → hex `string` (Random-gated) |
| `json` (4) | parse, __parse_typed, stringify, is_valid | parse → `Result<Json>`; stringify → `Result<string>`; is_valid → bare `bool` |
| `http` (8) | get, delete, post_text, post_bytes, put_text, put_bytes, post_json, put_json | all async → `Result<HttpResponse>`; NetConnect |
| `regex` (7) | match, find, is_match, match_all, replace, replace_all, split | match/find → `Option<{...}>`; is_match → bare `bool`; replace* → bare `string` |
| `compress` (6) | gzip, gunzip, zstd, unzstd, deflate, inflate | **bare** `Array<int>`/`string` (no Result) — §5.5 |
| `csv` (6) | parse, parse_records, stringify, stringify_records, read_file, is_valid | parse → **bare** `Array<Array<string>>`; read_file → `Result<...>` (FsRead); is_valid → bare `bool` — §5.5 |
| `unicode` (5) | normalize, category, is_letter, is_digit, graphemes | bare returns; category takes `int` codepoint |
| `msgpack` (4) | encode, decode, encode_bytes, decode_bytes | all → `Result<...>` |
| `yaml` (4) | parse, parse_all, stringify, is_valid | parse/parse_all → `Result`; is_valid → bare `bool` |
| `file` (4) | read_text, write_text, read_lines, append | all → `Result<...>` (FsRead/FsWrite); read_bytes/write_bytes **dead** |
| `env` (4) | has, cwd, os, arch | bare returns; **get/all/args dead** (§14.1) |
| `toml` (3) | parse, stringify, is_valid | parse → `Result`; is_valid → bare `bool` |
| `arrow` (3) | read_table, read_tables, metadata | `Result<DataTable>`; FsRead-gated but **no captags entry** (§6.8) |
| `xml` (2) | parse, stringify | parse → `Result`; stringify → `Result` |
| `archive` (2) | zip_extract, tar_extract | `Result<...>` |
| `parallel` (0) | — | **whole module dead** (§2.8) |

Intrinsic modules registered in `all_stdlib_modules()` (`stdlib/mod.rs:56`–`68`): vector, math, array_transforms, rolling, statistical, random, distributions, convolution, stochastic, matrix, fft, recurrence — plus `stdlib_time` and `stdlib_io`.

### 19.1 Return-convention verdict

Of 16 native modules, the `Result` vs bare split is genuinely unprincipled: `compress` and `csv::parse` return bare collections while `toml`/`yaml`/`xml`/`msgpack` parse to `Result`. `is_valid` is consistently bare `bool` across modules (good). `hash`/`encode`-family are bare strings (defensible — infallible on valid input). The one that should change is `csv::parse` → `Result` (malformed CSV currently yields a possibly-truncated bare array rather than an error), especially since a separate `csv::is_valid` exists, implying failure is a real possibility the parse swallows.

### 19.2 Reconciling the three method-table sources — a concrete work item

For the maintainer implementing §12.1, the reconciliation targets are:

- **Source of truth**: `stdlib-src/core/{string,number,int,option_methods,result_methods,hashmap_methods,...}.shape` `extend` blocks (already labeled "canonical").
- **Generate/validate B**: the `str_methods`/collection sections in `type_system/checking/method_table.rs`.
- **Generate/validate C**: the `STRING_METHODS`/`NUMBER_METHODS`/`HASHMAP_METHODS`/... PHF maps in `shape-vm/.../method_registry.rs`.
- **Add `number`/`int`/`Option`/`Result` sections to B** (currently absent — the largest single gap).
- **Add `isEmpty`/`chars` to string C**, add `toInt`/`toFloat` to string B, and reconcile the marked stubs (`toBool`, `graphemes`, `HashMap.keys/values`) so the checker reports them as unavailable until their V3-S5 ckpt-6 / v0.4 impls land, rather than accepting a call that then aborts.

A `#[test]` iterating each receiver type and asserting `set(B methods) == set(C non-stub methods)` and `set(A declarations) ⊇ set(B)` would turn the entire §14.6 "drift" column into a compile-time CI failure the next time it regresses. That test is the highest-value single artifact this audit can recommend.
