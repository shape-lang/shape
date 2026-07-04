# A-final ROOT D — `string.length()` checker / PHF-registry desync

**Verdict: FP_fix_checker** (valid code over-rejected; sync the checker — one-line add).

## Failing tests

- `stress_booleans_none::test_string_length_method`
  (`tools/shape-test/tests/literals/stress_booleans_none.rs:484`)
  Program: `fn test() -> int { "hello".length() }` then `test()` — `.expect_number(5.0)`
- `stress_booleans_none::test_empty_string_length`
  (`tools/shape-test/tests/literals/stress_booleans_none.rs:494`)
  Program: `fn test() -> int { "".length() }` then `test()` — `.expect_number(0.0)`

## Reproduction on the strict-flip binary (verbatim)

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape` (@f01e8323, let-gen landed).

```
$ printf 'fn test() -> int { "hello".length() }\ntest()\n' > /tmp/t.shape
$ .../target/release/shape run /tmp/t.shape
Error: Runtime error: Bytecode compilation failed: Semantic error: Method 'length' not found on type 'string'
```

Empty-string variant (`"".length()`) gives the identical rejection. Both reproduce verbatim.

Cross-checks confirming the method is genuinely shipped:
- strict-flip `"hello".len()` → returns 5 (raw print shows `4607182418800017408` = f64 bit pattern for `5.0`; the `print` builtin shows raw i64 bits of the number — value is correct).
- main release binary `"hello".length()` → returns 5 (same bit pattern). So `.length()` is a real, shipped, working string method on main; only the strict checker rejects it.

## Root cause — the exact seam

`crates/shape-runtime/src/type_system/checking/method_table.rs`

The strict type-checker runs over the user program only and never sees the stdlib `extend string` block, so `register_builtin_collection_methods()` (the strict-flip collection-dispatch root #1 seed) hand-mirrors the canonical string methods into the monomorphic `methods` map. The string seed list `str_methods` (lines 377–444) registers:

```
378:            ("len", vec![], BuiltinTypes::integer()),
```

…but **never registers `"length"`**. Resolution path:
`resolve_method_call` (line 742) → `extract_receiver_info` returns `("string", [])` → no generic sig → `self.lookup(receiver, "length")` (line 802) → key `("string","length")` absent from `methods` → universal-receiver fallback absent → `None` → caller surfaces `MethodNotFound` ("Method 'length' not found on type 'string'", `errors.rs:120`).

The runtime PHF registry HAS both aliases, both → `v2_string_len`:

```
crates/shape-vm/src/executor/objects/method_registry.rs:901   "len"    => ...string_methods::v2_string_len,
crates/shape-vm/src/executor/objects/method_registry.rs:902   "length" => ...string_methods::v2_string_len,
```

This is a pure checker/runtime **desync**, not an intentional removal. Evidence it was an oversight in the same seeding effort:
- The Vec seed in the same function carries **both** aliases — `("length", 0, vec![], int())` at line 331 and `("len", 0, vec![], int())` at line 334, with a comment explicitly calling `len` an alias for `length`.
- HashMap/Set/Deque/PriorityQueue/Range/TypedArray PHF maps in `method_registry.rs` all carry `len` + `length` pairs (lines 282, 464, 499, 527, 546, 721, 754, 798, 1055–1057). The string seed simply dropped one of the pair.

## Canonical-method determination (TP vs FP gate)

The task asks whether `.length()` is canonical or whether `.len()` is canonical and `.length()` should NOT exist (which would make this TP).

Both are canonical aliases — `.length()` is NOT a method that "should not exist":
- Runtime PHF registers BOTH for string (and for every other collection), both → `v2_string_len`.
- `.length()` works end-to-end on the main binary (returns 5).
- The checker already accepts `.length()` on `Vec` (line 331), so removing string `.length()` would be inconsistent, not stricter.

Therefore this is **FP_fix_checker**, not TP_rebaseline_test. The tests assert valid, shipped behavior; the checker is the thing out of sync.

## Minimal fix (exact edit)

In `crates/shape-runtime/src/type_system/checking/method_table.rs`, in the `str_methods` vec, add the `length` alias immediately after the `len` entry (line 378):

```rust
            ("len", vec![], BuiltinTypes::integer()),
            ("length", vec![], BuiltinTypes::integer()),   // ADD — PHF registry has len+length (method_registry.rs:901-902); both -> v2_string_len
```

`register_method` (line 551) pushes into the monomorphic `methods` map that `lookup` (line 611) reads, so this one line makes `("string","length")` resolve to `int`, matching `("string","len")`. No other file changes required.

### Note on related (non-blocking) desyncs in the same string seed

The string seed also omits a few other PHF-registered string aliases (e.g. `to_upper_case`, `to_lower_case`, `trim_start`, `trim_end`, `starts_with`, `ends_with`, `index_of`, `char_at`, `is_digit`, `is_alpha`, `is_ascii`, `code_point_at`, `grapheme_len`, `to_string`/`toInt`/`toFloat` snake/extra aliases). These are NOT in scope for ROOT D (no failing test cited) and should be handled only if their own A-final roots surface — flagging for awareness, not folding in. The ROOT-D fix is the single `length` line.

## Files the fix touches (for conflict-grouping)

- `crates/shape-runtime/src/type_system/checking/method_table.rs` (single line added in `register_builtin_collection_methods` / `str_methods`)

Same-file conflict group: any other A-final root that edits `register_builtin_collection_methods` (the collection-dispatch strict-flip seed) — coordinate to avoid stomping the `str_methods` / Vec / HashMap seed lists.

## Clears

- `stress_booleans_none::test_string_length_method`
- `stress_booleans_none::test_empty_string_length`

Both reproduce verbatim on the strict-flip binary (`Method 'length' not found on type 'string'`) and are cleared by adding the one `("length", ...)` entry to the checker's string method table.
