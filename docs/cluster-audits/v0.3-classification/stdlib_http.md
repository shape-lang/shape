# stdlib_http classification

**HEAD:** 82f049dd
**Total tests in binary:** 6
**Passed:** 1 / Failed: 5 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test stdlib_http --no-fail-fast 2>&1`

Note: run executed serialized (`-- --test-threads=1`) because the parallel-default
run hung indefinitely under concurrent shape-test cargo contention (load avg
500+). Serialized run took 1284s (heavy stdlib JIT-compilation per `with_stdlib`
× 5 expect_run_ok tests × system contention). Per-test failure messages below
verbatim from the test output.

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 0 |
| FN-REG-DIAGNOSTIC  | 5 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

All 5 failures are stale test fixtures pinned to the pre-2026-05 http API
(single-arg + bare `post`/`put`). The language behavior is correct — the
stdlib was deliberately renamed/split with explicit close-out documentation
in commit `94dc8fa9` (2026-05-24, "R8 W6 G.3 http options-arg doc/code
alignment"), which states verbatim:

> Surfaced (a-class follow-up): `tools/shape-test/tests/stdlib_http/basic.rs`
> references the pre-split API (`http::post`/`http::put` — gone post-split)
> and calls all variants WITHOUT options. Tests appear unwired into the
> harness; v0.4 polish territory.

This is exactly the FN-REG-DIAGNOSTIC pattern: language correct, fixture
text/shape stale. Per-fn detail below.

## Per-test classification

### basic::http_delete_basic

Class: **FN-REG-DIAGNOSTIC**

```
thread 'basic::http_delete_basic' (2947183) panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: expected 2 arg(s), got 1 (line 3)")
```

- **Old expected text** (fixture): `http::delete("https://httpbin.org/delete")` returns Ok.
- **New actual text**: `Runtime error: expected 2 arg(s), got 1 (line 3)`.
- **Language change**: `http.delete` signature changed from `(url)` to
  `(url, options)` (commit `94dc8fa9`, 2026-05-24, "R8 W6 G.3 http
  options-arg doc/code alignment"; supervisor disposition chose doc-fix
  over making `options` actually optional). Fixture must pass `{}` as
  second arg.

### basic::http_get_basic

Class: **FN-REG-DIAGNOSTIC**

```
thread 'basic::http_get_basic' (2959055) panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Runtime error: expected 2 arg(s), got 1 (line 3)")
```

- **Old expected text**: `http::get("https://httpbin.org/get")` returns Ok.
- **New actual text**: `Runtime error: expected 2 arg(s), got 1 (line 3)`.
- **Language change**: same as `http_delete_basic` — `http.get` signature
  is now `(url, options)`. Same commit `94dc8fa9`.

### basic::http_get_with_invalid_url

Class: **passed (not failing)** — listed for completeness.

```
test basic::http_get_with_invalid_url ... ok
```

The 1-arg form on `http::get("not-a-valid-url")` still produces an error
(now an arity error rather than a URL-validation error), but the fixture's
`expect_run_err()` is satisfied by either, so it passes incidentally. Note
this is a latent bug-magnet — the fixture no longer exercises what it
claims to (URL validation), but does not fail. Tag for v0.4 fixture-rewrite
when the rest of the binary is repaired.

### basic::http_post_basic

Class: **FN-REG-DIAGNOSTIC**

```
thread 'basic::http_post_basic' (2964510) panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Semantic error: module 'http' has no export 'post'")
```

- **Old expected text**: `http::post(url, body)` returns Ok.
- **New actual text**: `Semantic error: module 'http' has no export 'post'`.
- **Language change**: `http.post` was split into `post_text` / `post_bytes`
  / `post_json` by-content-type in commits `d0a73e78` (Stage D N4 partial
  sign-off, 2026-05-07 "Shape API split (4 typed overloads)") and
  `3820d749` ("post_json + put_json migration N7-C8 + C9"). Bare `post` is
  intentionally removed (typed-marshal-only ABI per ADR N7 ε disposition).

### basic::http_post_with_json_body

Class: **FN-REG-DIAGNOSTIC**

```
thread 'basic::http_post_with_json_body' (2965156) panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Semantic error: module 'http' has no export 'post'")
```

- **Old expected text**: `http::post(url, json_body_string)` returns Ok.
- **New actual text**: `Semantic error: module 'http' has no export 'post'`.
- **Language change**: same as `http_post_basic` — fixture must use
  `post_text` for a string body or `post_json` for a structured body.

### basic::http_put_basic

Class: **FN-REG-DIAGNOSTIC**

```
thread 'basic::http_put_basic' (2967512) panicked at tools/shape-test/src/shape_test.rs:1236:9:
Expected run ok, got error: Some("Semantic error: module 'http' has no export 'put'")
```

- **Old expected text**: `http::put(url, body)` returns Ok.
- **New actual text**: `Semantic error: module 'http' has no export 'put'`.
- **Language change**: `http.put` was split into `put_text` / `put_bytes` /
  `put_json` in the same Stage D commits as `post`. Fixture must use
  `put_text` for a string body.
