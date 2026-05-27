# strings_formatting classification

**HEAD:** 82f049dd
**Total tests in binary:** 203
**Passed:** 180 / Failed: 23 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test strings_formatting --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 11 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 12 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### methods::string_split

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: String.split: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. The deleted typed-array-data String `Arc<Buf<Arc<String>>>` result carrier DELETED at V3-S5 ckpt-1..ckpt-4 ... Rebuild lands at ckpt-6 STRICT close
```

- Dated user pull-in: 2026-05-18 V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade — String.split returns an `Array<string>` which is exactly the construction-cascade target. Also covered by 2026-05-21 "Array<string> must work" user pull-in.
- SURFACE defers to "ckpt-6 STRICT close"; in-scope per the dated rows.
- Test asserts on user-facing semantics (`"a,b,c".split(",")` returning an array); stays the same after fix.

### stress_methods::test_empty_string_split, test_split_by_comma, test_split_by_comma_first_element, test_split_by_comma_second_element, test_split_by_space, test_split_count_elements, test_split_multi_char_separator, test_split_no_match, test_split_then_join, test_split_single_char_separator, test_split_empty_string

Class: **SCOPE-RECLAIM** (11 tests, same root cause)

- All emit the same `String.split: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3` message.
- Same dated pull-ins (2026-05-18 ckpt-5/6 + 2026-05-21 Array<string>).
- All assert on user-facing semantics; stay the same after fix.

### stress_methods::test_string_in_loop_concat

Class: **SCOPE-RECLAIM**

```
Runtime error: Not implemented: range: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. ... UNREACHABLE until ckpt-6 STRICT close.
```

- Same family as operators.md `comparison_stability_loop` (range builtin). 2026-05-18 dated pull-in row. SCOPE-RECLAIM.

### stress_methods::test_chain_substring_to_upper

Class: **FN-REG-CORRECTNESS**

```
assertion `left == right` failed: Expected 'HELLO', got 'HELLO WORLD'
```

- `.substring(0, 5).to_upper()` returns the full string upper-cased, not just first 5 chars upper-cased. `substring(start, end)` is ignoring the `end` argument. Plausibly-correct code; substring is a fundamental builtin.

### stress_methods::test_char_at_out_of_bounds

Class: **FN-REG-CORRECTNESS**

```
Expected run ok, got error: Runtime error: Index 10 out of bounds (length 5) (line 2)
```

- Test expected `char_at(10)` on a 5-char string to return null/empty (graceful out-of-bounds), got a runtime error. Behavioral regression on out-of-bounds semantics of `String.char_at`.

### stress_methods::test_pad_end_basic, test_pad_end_with_multichar_fill, test_pad_start_basic, test_pad_start_with_multichar_fill

Class: **FN-REG-CORRECTNESS** (4 tests, same root cause)

```
Expected '00042', got '   42'
Expected 'xabab', got 'x    '
Expected 'hi...', got 'hi   '
Expected 'ababx', got '    x'
```

- `pad_start`/`pad_end` are ignoring the fill-char argument and always padding with spaces. Plausibly-correct code; `pad_start(n, fill)` API regression — fill string not applied.

### stress_methods::test_substring_empty_range, test_substring_with_start_and_end, test_substring_from_start, test_substring_single_char

Class: **FN-REG-CORRECTNESS** (4 tests, same root cause)

```
Expected '', got 'llo'
Expected 'hello', got 'hello world'
Expected 'hel', got 'hello'
Expected 'e', got 'ello'
```

- `String.substring(start, end)` is ignoring the `end` argument entirely (returning from `start` to end-of-string). Same root cause as `test_chain_substring_to_upper`. Plausibly-correct code; fundamental stdlib bug.
