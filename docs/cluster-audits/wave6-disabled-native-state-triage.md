# Wave 6 Disabled Native/State Book Triage

Generated from `/tmp/shape-async-snippets/manifest.json` (`2026-07-09T09:52:40.362Z`).
Scope covered 114 disabled snippets across the requested native/runtime/resource pages.

I did not execute cargo, just, rustc, nextest, shape-test, book-truth, or the
Shape binary over snippets. This is a source/manifest triage only.

## Summary

| Classification | Count | Notes |
|---|---:|---|
| `stale_disabled_candidate` | 27 | Implementation likely exists; next worker should rewrite into deterministic standalone snippets, then smoke under the serialized verification lane. |
| `active_feature_gap` | 32 | Current source or tests show a real implementation/testability blocker. |
| `external_environment_or_permission` | 41 | Requires filesystem, network, process, stdio, environment, or permissions. Keep disabled unless rewritten against deterministic fixtures/policies. |
| `preview_or_out_of_scope` | 14 | Conceptual pseudocode, host API preview, comment-only block, infinite loop pattern, or future stream/fetch surface. |

Source anchors used:

- Manifest slice: `/tmp/shape-async-snippets/manifest.json`.
- Book pages in sibling `../shape-web/book/book-site/src/content/docs/**`.
- Archive registration: `crates/shape-runtime/src/stdlib/archive.rs` documents `zip_create`/`tar_create` as not registered, while `zip_extract`/`tar_extract` are registered.
- State builtins: `crates/shape-vm/src/executor/state_builtins/{core.rs,introspection.rs}` shows `state::hash`, `state::fn_hash`, and `state::schema_hash` have bodies, while capture/serialize/diff/patch/resume-family surfaces still carry W17 follow-ups.
- Async file I/O: `crates/shape-runtime/src/stdlib_io/async_file_ops.rs` explicitly defers `io.read_file_async` registration.
- Testing/property-testing blockers: `bin/shape-cli/tests/stdlib/stdlib_advanced.rs` and Wave-5 notes show generic assertion and property-spec function-field gaps.
- Resource management: `tools/shape-test/tests/drop_raii/**`, `crates/shape-vm/src/executor/tests/auto_drop.rs`, and `drop_deep_tests.rs` show the RAII core exists, but the book snippets use placeholder DB/resource APIs.

## Page Triage

### `advanced/security-permissions.mdx`

Counts: `preview_or_out_of_scope` 9.

`preview_or_out_of_scope`:

- L162 `E__advanced__security-permissions__4__L162.shape`: comment-only per-function gating sketch.
- L333 `E__advanced__security-permissions__8__L333.shape`: `PermissionGrant`/`ScopeConstraints` Shape-like host concept, not a runnable Shape API.
- L350 `E__advanced__security-permissions__9__L350.shape`: scoped network grant sketch.
- L364 `E__advanced__security-permissions__10__L364.shape`: resource limit grant sketch.
- L387 `E__advanced__security-permissions__11__L387.shape`: v0.4 host embedding pseudocode (`compile`, `vm.load_program_with_permissions`).
- L418 `E__advanced__security-permissions__12__L418.shape`: intended compile-time denial example, not a positive runnable snippet.
- L445 `E__advanced__security-permissions__13__L445.shape`: runtime gating example over network I/O.
- L470 `E__advanced__security-permissions__14__L470.shape`: host-side `ResourceLimits`/`ResourceUsage` pseudocode.
- L502 `E__advanced__security-permissions__15__L502.shape`: combined host/runtime/resource-tier pseudocode.

### `fundamentals/resource-management.mdx`

Counts: `stale_disabled_candidate` 14, `active_feature_gap` 2, `preview_or_out_of_scope` 1.

`stale_disabled_candidate`:

- L14 `A__fundamentals__resource-management__0__L14.shape`: automatic scope drop; rewrite fake `db.connect` as a tiny `Drop` counter/log.
- L29 `A__fundamentals__resource-management__1__L29.shape`: block scoping; same deterministic `Drop` rewrite.
- L45 `A__fundamentals__resource-management__2__L45.shape`: reverse drop order; rewrite with two local `Drop` types.
- L58 `A__fundamentals__resource-management__3__L58.shape`: early return reverse order; rewrite fake `acquire`.
- L75 `A__fundamentals__resource-management__4__L75.shape`: loop `break`/`continue` drop; rewrite with local counters.
- L110 `A__fundamentals__resource-management__6__L110.shape`: user `impl Drop` exists; replace fake `close_fd`.
- L126 `A__fundamentals__resource-management__7__L126.shape`: function-exit drop; replace fake file API.
- L139 `A__fundamentals__resource-management__8__L139.shape`: custom connection wrapper; replace fake pool API.
- L165 `A__fundamentals__resource-management__9__L165.shape`: drop-error containment has tests; rewrite fake resources.
- L183 `A__fundamentals__resource-management__10__L183.shape`: early-return drop; rewrite fake DB rows.
- L200 `A__fundamentals__resource-management__11__L200.shape`: nested scopes; rewrite fake DB calls.
- L224 `A__fundamentals__resource-management__12__L224.shape`: sync/async drop opcode selection is covered in source/tests; needs a deterministic snippet.
- L298 `A__fundamentals__resource-management__15__L298.shape`: block lifetime best practice; rewrite fake DB.
- L314 `A__fundamentals__resource-management__16__L314.shape`: dependency order; rewrite fake transaction/cursor.

`active_feature_gap`:

- L256 `A__fundamentals__resource-management__13__L256.shape`: `for await`/stream merge cleanup example depends on not-yet-honest stream semantics.
- L278 `A__fundamentals__resource-management__14__L278.shape`: async scoped lifetime example combines fake DB async queries with broader async resource semantics; keep disabled until a focused async-drop book lane proves it.

`preview_or_out_of_scope`:

- L100 `A__fundamentals__resource-management__5__L100.shape`: conceptual builtin `Drop` trait definition, not a snippet users should run as source.

### `stdlib/core/math.mdx`

Counts: `stale_disabled_candidate` 4.

`stale_disabled_candidate`:

- L71 `B__stdlib__core__math__5__L71.shape`: `correlation` wrapper exists; replace undefined `prices`/`volumes` with literals.
- L80 `B__stdlib__core__math__6__L80.shape`: `covariance` wrapper exists; replace undefined series.
- L88 `B__stdlib__core__math__7__L88.shape`: `percentile` wrapper exists; replace undefined `latencies`.
- L124 `B__stdlib__core__math__10__L124.shape`: `spread` wrapper exists; likely direct flip after import check.

### `stdlib/core/property_testing.mdx`

Counts: `active_feature_gap` 4.

`active_feature_gap`:

- L19 `B__stdlib__core__property_testing__0__L19.shape`: import-only surface is not useful until the module's function-field/schema blockers are resolved.
- L32 `B__stdlib__core__property_testing__1__L32.shape`: `property<T>` currently blocked by `PropertyResult<T>`/function-field schema representation.
- L49 `B__stdlib__core__property_testing__2__L49.shape`: `run_properties<T>` still has specialization and empty-array inference blockers.
- L77 `B__stdlib__core__property_testing__3__L77.shape`: generator example depends on the same property/function-field path; `gen_array` is also deliberately narrowed to `int`.

### `stdlib/core/state.mdx`

Counts: `stale_disabled_candidate` 3, `active_feature_gap` 18.

`stale_disabled_candidate`:

- L237 `B__stdlib__core__state__14__L237.shape`: `state::hash` is implemented; rewrite with explicit `use std::core::state` and local assertion/check helper.
- L257 `B__stdlib__core__state__15__L257.shape`: `state::fn_hash` has a body; next worker should smoke whether book-truth dispatch supplies content-hash metadata.
- L271 `B__stdlib__core__state__16__L271.shape`: `state::schema_hash` has a body; rewrite with a local type and stable length/determinism checks.

`active_feature_gap`:

- L137 `B__stdlib__core__state__8__L137.shape`: `state::capture` still surfaces W17 marshal-return follow-up.
- L149 `B__stdlib__core__state__9__L149.shape`: `state::capture_all` still surfaces W17 marshal-return follow-up.
- L160 `B__stdlib__core__state__10__L160.shape`: `state::capture_module` still surfaces W17 marshal-return follow-up.
- L172 `B__stdlib__core__state__11__L172.shape`: `state::capture_call` plus transport send is not book-runnable.
- L202 `B__stdlib__core__state__12__L202.shape`: user-level `state::resume` is still an active snapshot/resume gap.
- L220 `B__stdlib__core__state__13__L220.shape`: `state::resume_frame` still needs typed-object field decode/marshal follow-up.
- L289 `B__stdlib__core__state__17__L289.shape`: `state::serialize` computes bytes internally but cannot project `Array<int>` yet.
- L298 `B__stdlib__core__state__18__L298.shape`: `state::deserialize` remains W17 surface.
- L306 `B__stdlib__core__state__19__L306.shape`: cache example depends on `serialize`/`deserialize` plus external cache object.
- L346 `B__stdlib__core__state__20__L346.shape`: `state::diff` still depends on the kind-threaded rebuild.
- L366 `B__stdlib__core__state__21__L366.shape`: `state::patch` still depends on the kind-threaded rebuild.
- L374 `B__stdlib__core__state__22__L374.shape`: transport delta example depends on `diff`, `serialize`, `deserialize`, `patch`, and transport.
- L395 `B__stdlib__core__state__23__L395.shape`: `state::caller` return projection still surfaces W17 follow-up.
- L409 `B__stdlib__core__state__24__L409.shape`: `state::args` return projection still surfaces W17 follow-up.
- L420 `B__stdlib__core__state__25__L420.shape`: `state::locals` return projection still surfaces W17 follow-up.
- L434 `B__stdlib__core__state__26__L434.shape`: remote dispatch example depends on `capture_call`, serialization, transport, spread args, and `Any`.
- L464 `B__stdlib__core__state__27__L464.shape`: state synchronization example depends on `capture_module`, `diff`, serialization, transport, and dynamic call/spread.
- L491 `B__stdlib__core__state__28__L491.shape`: content-addressed cache example depends on `fn_hash`, `serialize`/`deserialize`, dynamic store, and spread calls.

### `stdlib/core/stochastic.mdx`

Counts: `stale_disabled_candidate` 4.

`stale_disabled_candidate`:

- L27 `B__stdlib__core__stochastic__1__L27.shape`: Brownian motion intrinsic exists; rewrite with seeded/invariant checks.
- L36 `B__stdlib__core__stochastic__2__L36.shape`: GBM intrinsic exists; rewrite with seeded/invariant checks.
- L45 `B__stdlib__core__stochastic__3__L45.shape`: OU process intrinsic exists; rewrite with length/range sanity checks.
- L53 `B__stdlib__core__stochastic__4__L53.shape`: random walk intrinsic exists; acceptance corpus already uses a length contract.

### `stdlib/core/testing.mdx`

Counts: `active_feature_gap` 4.

`active_feature_gap`:

- L44 `B__stdlib__core__testing__2__L44.shape`: `assert_eq<T>` still blocked by imported generic call-site inference in book-smoke form.
- L59 `B__stdlib__core__testing__3__L59.shape`: `assert_ne<T>` same generic inference blocker.
- L88 `B__stdlib__core__testing__5__L88.shape`: `assert_ok` blocked by current Result method dispatch in this smoke path.
- L103 `B__stdlib__core__testing__6__L103.shape`: `assert_err` blocked by current Result method dispatch in this smoke path.

### `stdlib/native/archive.mdx`

Counts: `stale_disabled_candidate` 2, `active_feature_gap` 3.

`stale_disabled_candidate`:

- L49 `B__stdlib__native__archive__2__L49.shape`: `zip_extract` is registered; needs deterministic fixture bytes or paired helper.
- L73 `B__stdlib__native__archive__4__L73.shape`: `tar_extract` is registered; needs deterministic fixture bytes.

`active_feature_gap`:

- L37 `B__stdlib__native__archive__1__L37.shape`: `zip_create` is declared but not registered in runtime source.
- L62 `B__stdlib__native__archive__3__L62.shape`: `tar_create` is declared but not registered in runtime source.
- L79 `B__stdlib__native__archive__5__L79.shape`: roundtrip depends on the unregistered create functions.

### `stdlib/native/env.mdx`

Counts: `external_environment_or_permission` 4.

`external_environment_or_permission`:

- L29 `B__stdlib__native__env__1__L29.shape`: depends on `API_KEY` environment.
- L39 `B__stdlib__native__env__2__L39.shape`: depends on current working directory.
- L48 `B__stdlib__native__env__3__L48.shape`: OS-dependent output.
- L57 `B__stdlib__native__env__4__L57.shape`: architecture-dependent output.

### `stdlib/native/file.mdx`

Counts: `external_environment_or_permission` 5.

`external_environment_or_permission`:

- L28 `B__stdlib__native__file__1__L28.shape`: reads `config.toml`.
- L38 `B__stdlib__native__file__2__L38.shape`: writes `output.txt`.
- L46 `B__stdlib__native__file__3__L46.shape`: reads `data.csv`.
- L57 `B__stdlib__native__file__4__L57.shape`: appends to `log.txt`.
- L75 `B__stdlib__native__file__5__L75.shape`: reads `image.png` through `io`.

### `stdlib/native/http.mdx`

Counts: `external_environment_or_permission` 9.

`external_environment_or_permission`:

- L39 `B__stdlib__native__http__1__L39.shape`: network GET to `api.example.com`; also stale `await` over current Result-returning builtins.
- L50 `B__stdlib__native__http__2__L50.shape`: network DELETE; same `await` drift.
- L64 `B__stdlib__native__http__3__L64.shape`: network POST text; same `await` drift.
- L73 `B__stdlib__native__http__4__L73.shape`: network POST bytes; same `await` drift.
- L82 `B__stdlib__native__http__5__L82.shape`: network POST JSON; same `await` drift.
- L100 `B__stdlib__native__http__6__L100.shape`: network PUT text; same `await` drift.
- L109 `B__stdlib__native__http__7__L109.shape`: network PUT bytes; same `await` drift.
- L118 `B__stdlib__native__http__8__L118.shape`: network PUT JSON; same `await` drift.
- L136 `B__stdlib__native__http__9__L136.shape`: network GET with auth header; secrets/permission-sensitive plus `await` drift.

### `stdlib/native/io.mdx`

Counts: `external_environment_or_permission` 23, `active_feature_gap` 1, `preview_or_out_of_scope` 1.

`external_environment_or_permission`:

- L26 `B__stdlib__native__io__1__L26.shape`: reads `data.csv`.
- L36 `B__stdlib__native__io__2__L36.shape`: reads `binary.dat`.
- L44 `B__stdlib__native__io__3__L44.shape`: reads `image.png`.
- L52 `B__stdlib__native__io__4__L52.shape`: reads `data.csv` inside a function.
- L64 `B__stdlib__native__io__5__L64.shape`: writes `output.csv`.
- L75 `B__stdlib__native__io__6__L75.shape`: appends to `log.txt`.
- L97 `B__stdlib__native__io__7__L97.shape`: stats `data.csv`.
- L118 `B__stdlib__native__io__8__L118.shape`: reads `reports` directory.
- L128 `B__stdlib__native__io__9__L128.shape`: creates directories and writes files.
- L146 `B__stdlib__native__io__10__L146.shape`: outbound TCP to `api.example.com`.
- L156 `B__stdlib__native__io__11__L156.shape`: binds TCP listener on `0.0.0.0:9000`.
- L183 `B__stdlib__native__io__12__L183.shape`: UDP bind/send/receive.
- L213 `B__stdlib__native__io__13__L213.shape`: executes `ls`.
- L230 `B__stdlib__native__io__14__L230.shape`: executes `shape check`.
- L242 `B__stdlib__native__io__15__L242.shape`: spawns `tail -f`.
- L274 `B__stdlib__native__io__16__L274.shape`: standard stream handles.
- L282 `B__stdlib__native__io__17__L282.shape`: interactive stdin/stdout.
- L326 `B__stdlib__native__io__19__L326.shape`: gzip file write/read path.
- L366 `B__stdlib__native__io__21__L366.shape`: mixed file/TCP/process cleanup with undefined `request`.
- L386 `B__stdlib__native__io__22__L386.shape`: filesystem transform with undefined `process`.
- L408 `B__stdlib__native__io__23__L408.shape`: reads `records.csv`.
- L431 `B__stdlib__native__io__24__L431.shape`: TCP client to localhost.
- L449 `B__stdlib__native__io__25__L449.shape`: executes `git log`.

`active_feature_gap`:

- L311 `B__stdlib__native__io__18__L311.shape`: `io::read_file_async` is declared in stdlib source but async file registration is deferred in runtime source.

`preview_or_out_of_scope`:

- L468 `B__stdlib__native__io__26__L468.shape`: indefinite file-watcher pattern with undefined `process_file`; should remain prose or become a bounded deterministic polling example.

### `stdlib/native/time.mdx`

Counts: `preview_or_out_of_scope` 3.

`preview_or_out_of_scope`:

- L84 `B__stdlib__native__time__4__L84.shape`: conceptual polling loop over undefined `fetch`.
- L121 `B__stdlib__native__time__6__L121.shape`: conceptual backoff loop over undefined `fetch`.
- L193 `B__stdlib__native__time__9__L193.shape`: conceptual rate-limited API loop over undefined `fetch`.

## Next Recommended Wave

1. `resource-management` deterministic flip worker: own only `fundamentals/resource-management.mdx`. Replace fake DB/resource APIs with small `Drop` counter/log snippets. Keep `for await`/stream examples disabled.
2. `state-hash` flip worker: own only `stdlib/core/state.mdx` hash/fn_hash/schema_hash snippets. Convert `state::hash` and `state::schema_hash` to deterministic checks; smoke `fn_hash` before flipping.
3. `math-stochastic` flip worker: own only `stdlib/core/math.mdx` and `stdlib/core/stochastic.mdx`. Add literals and seeded/invariant checks; avoid raw random output expectations.
4. `archive-extract` worker: own only `stdlib/native/archive.mdx` plus runtime archive create registration if explicitly assigned. Without code ownership, only flip extract examples with literal fixture bytes.
5. `io-file-env-http` rewrite worker: keep disabled by default; only flip examples that can be rewritten against deterministic temp/VFS fixtures and permission policy. Do not use live network, real user env, stdin, subprocesses, or infinite loops.
6. `state-resume-property-testing-testing` implementation lanes: do not flip docs first. Close W17 marshal-return/resume gaps, property-testing function-field/specialization gaps, and imported generic/Result assertion blockers before revisiting disabled snippets.
