# WF-4 coverage matrix — book + test completeness (recon 2026-07-06, main eacbed65)

Read-only recon over all wave-0..3 feature areas + full 748-fence book-truth measurement.

## Headline gate number

(not found)


**Delta:** Gate reports 246/247 = 99.6% pass. Real full-universe VM pass = 368/748 = 49.2%. DELTA = ~50.4 percentage points; the gate over-states book truth by 2.0x.

Root cause of the gap: the gate measures only the 247 `runnable=true` fences (33% of the book) and skips 501 `runnable=false` fences; 379 of those 501 skipped fences (75.6%) actually fail under VM. The gate's near-perfect number is an artifact of the curated denominator, exactly the documented denominator trap.

Actionable delta (excluding legitimate fragment/excerpt noise): ~278 of the 380 fails are excerpt-style fragments that plausibly should stay non-runnable, but they mean many features (state::capture snapshot API, remote::*, ode::rk45, distributions, property_testing, most annotations, most enums pages) have NO runnable gate-green example at all â violating the 'every feature needs a runnable vm+jit example' requirement even though nothing is technically broken. Beyond fragments there are 10 hard runtime stubs (Intrinsic{Median,Max,RollingMean,LinearRecurrence,DistUniform}, Temporal GetProp, HashMap.keys, WrapTypeAnnotation, foreign_marshal) that are documented-but-surface-and-stop = real product gaps; 4 nonexistent annotations documented (@traced/@serializable/@registered/@remote); and 40 fences that don't parse. NOTE: state::/comptime failures sit under WF-2G and WF-3D in-flight lanes â re-run this probe after they land before treating those subsets as final.


## Per-area severity


### polyglot-ffi (extern C fn + out-params, fn python, fn typescript, modular extension system / ext install / LanguageRunti
HIGH â the three headline features (extern C, fn python, fn typescript) genuinely WORK in the VM on current main (the 2026-07-04 "dead stub" audit referred to shipped v0.3.2; main is functional), so this is NOT a dead-code area. Severity is driven by the book gate, not by broken runtime: 33 of 34 shape fences across the entire area are runnable=false, so the hard "every feature has a gate-green vm+jit example" gate is essentially 0% met here; plus two documented marshalling features (Vec<struct> python return, HashMap param) are actually broken and would mislead users; plus real fn python/typescript execution tests are ignored outside the opt-in `just test-ffi` tier and `ext install` + Ffi-permission-enforcement have no tests at all. Not CRITICAL because nothing silently produces wrong values (the two broken cases fail loudly; JIT deopts but stays value-correct). Note: polyglot-distributed.mdx is in-flight (WF-2G) â re-verify after it lands.


### snapshot-resume
HIGH â Shipped, release-blocking feature (W17/WF-2G) whose book chapters actively MISINFORM: they declare it "not available in v0.3.3 / aborts at runtime" when it works, show the wrong signature (Snapshot vs Result<Snapshot,SnapshotError>), document a call form (snapshot::snapshot()) that is a compile error, and present a non-functional recompile-and-resume path as working. Zero gate-green vm+jit examples exist for the whole area; multiple working surfaces (short-hash, SIGINT-130, error model) are undocumented; and JIT runs snapshot code only via [jit-fallback]. Test coverage is decent at the unit level but has no CLI/e2e coverage of the actual --resume, SIGINT-130, or short-hash paths. NOTE: heap-element-array projection is WF-2G in-flight (currently Barriers) â re-verify that slice after WF-2G lands.


### remote-distributed (@remote per-function transfer, remote::call/execute/ping, closure-over-wire, TLS-on-TCP, permission-
HIGH â feature is shipped and partially working, but the book is internally contradictory (remote.mdx says "v0.4, not in v0.3.3" while polyglot-distributed.mdx says "genuine today"), every remote example is runnable=false (ZERO book-gate coverage), multiple documented examples are factually wrong (Ok(42) vs bare 42, non-existent `assert`, retired `__call`, Array params that don't compile), and 3 confirmed functional bugs (remote::execute returns constant {bindings,schemas} not the value; @remote Array param = compile error; @remote module-global capture returns 0 e2e) are unguarded by any test. Rust-side unit/e2e coverage is otherwise strong (TLS, auth, permission-over-wire, resupply all tested).


### polyglot-distributed â foreign-fn blobs transfer+execute remotely, snapshot/resume across foreign frames, Ffi permissi
HIGH â The feature area's headline capability (the {C,py,ts}Ã{transfer,snapshot,combined} composition matrix) has NEITHER a gate-runnable book example (3 of 4 shape fences are runnable=false; the only gate-green one is the trivial extern-C labs primitive) NOR an automated end-to-end test (all compose-genuineness is manual-only; only unit-level Ffi-union/opt-in enforcement and a simulated snapshot barrier are covered). The individual enforcement pieces are solid, which downgrades this from CRITICAL, but the composition itself â the entire point of the new chapter â is unverified by both the book gate and the test suite. Flag WF-2G dependency for re-verification.


### async (async fn, async let, await, async scope, for await x in stream, join all|race|any|settle, real concurrency WF-2D)
HIGH â core language feature area (async fn/let/await/join/scope/for-await + real WF-2D concurrency) is functional in VM but has (1) ZERO runnable book examples (entire chapter runnable=false â a hard book-gate violation), (2) no JIT-green path (universal async deopt), and (3) thin tests (join strategies never assert output, async_runtime scheduler untested, no real-concurrency-overlap proof, stale TDD-marked async_let tests). Not a correctness emergency â VM output is correct and v0.4 caveats are mostly honest â but a large documentation+coverage gap for a headline feature.


### comptime (comptime blocks/fns, comptime for, annotations before/after/comptime-pre/post, directives set/replace/extend/r
HIGH for the book gate, LOW-MEDIUM for tests. The tests are in decent shape; the failure is the book: two flagship chapters (annotations.mdx = 100% runnable=false, cookbook mostly, llm-patterns = 0 runnable markers) hide working, shipped comptime/annotation functionality behind STALE v0.4-preview caveats and non-runnable fences, so per the user's HARD gate ("every implemented feature must have a gate-runnable green example") a large swath of comptime â runtime wrappers on typed params, comptime-derive, set return/replace body/remove/extend, the entire serde/llm showcase â currently has ZERO gate coverage despite passing vm+jit today. Concrete broken items that are still small in number: `set param` (ct_45), `replace module` (untested), and the missing `--diagnostics json` surface. WF-3D touches the serde/llm showcases â re-verify chapters 2 and the showcases suite after it lands.


### security-permissions (17 permissions incl Ffi, compile-time capability derivation, runtime check_permission gating, Scop
HIGH. The security model is real and enforced at runtime (verified live across all three tiers), but the book is actively WRONG: it tells users the entire permission/sandbox/scope surface is unavailable in v0.3.3 (v0.4 preview) when a fully-runnable frontmatter/shape.toml surface ships and enforces today â so users cannot discover or use security features that exist. Compounding: the 17th Ffi permission is undocumented, there are ZERO runnable examples for a security-critical area, resource-limit enforcement has ZERO tests despite working, and there is NO end-to-end CLI test that permission/scope/limit denial actually fails closed (a correctness-critical gap for a sandboxing feature). Data-structure/parse layers are well unit-tested, which keeps this at HIGH rather than CRITICAL.


### drop-raii (Drop trait, automatic scope-based drop, escape/Drop-deferral semantics ADR-006 Â§2.7.30, & / &mut references,
HIGH for BOOK: the Drop trait + automatic scope-drop + escape/Drop-deferral (Â§2.7.30) are shipped, wave-0-3 features that fully work under VM, yet the entire Drop chapter (resource-management.mdx) has ZERO gate-runnable examples (17/17 runnable=false, fictional stdlib) and the escape-deferral behavior is undocumented â a direct violation of the book hard-gate. LOW-MEDIUM for TESTS: coverage is strong (~300+ tests across drop_raii, borrow_refs, auto_drop, storage_planning, lsp), with two named thin spots (owned-value escape/module-bound Drop-side-effect ordering). JIT native drop codegen is a known v0.4 SURFACE, not a regression.


### serialization-stdlib (json, msgpack, toml, yaml, xml, http, time/DateTime)
HIGH â 4 shipped serialization features are broken while documented as working, and the book's runnable=false examples mean the truth-gate catches none of them: msgpack::decode (no round-trip), toml::parse + yaml::parse (parsed value not navigable), xml::stringify (crashes on empty children), and json's entire documented .get/.at/.len/.keys/.is_null navigation API (unimplemented). Round-trip is the core promise of a serialization stdlib, and existing tests are smoke-only (encode/parse + expect_run_ok, no value assertion) so they mask these. Additionally json.mdx's `as?` syntax and import block don't even parse/run as written. Not in-flight WF-2G/WF-3D territory, so no re-verify caveat applies; audited at main HEAD with target/release/shape.


### strict-typing (let-generalization / no-truthiness / numeric-conversion / generic-types-require-args / strict TypeDiagnos
HIGH â Violates the user's HARD GATE (every shipped feature must be book-documented + covered by a gate-runnable vm+jit example): 4 of 5 strict-typing features (let-generalization, no-truthiness, generic-require-args, strict-mode) are entirely undocumented, and numeric-conversion is only partially documented. Additionally surfaced a concrete enforcement bug: the generic-types-require-args rule leaks in struct-field position (`type T { f: Option }` compiles) with zero test catching it. Test coverage is strong for numeric-conversion/truthiness/let-gen but thin for generic-require-args and strict-mode-default. Not release-catastrophic (rules mostly enforce correctly at runtime), but a large book+test gap plus one real correctness leak.


### core-language (enums, traits+extends, generics, pattern matching, Result/?/!!, control flow/break-value, strings/f-strin
MEDIUM â No runnable book example FAILS the vm+jit gate in this area (120/120 green), so nothing is release-blocking-catastrophic. But two in-scope, shipped, WORKING features (generics and the `!!` error-context operator) have ZERO gate-green book example because they are wrongly marked runnable=false with stale 'broken' comments â this directly violates the hard rule that every implemented feature carry a gate-runnable example. Plus Content builder (~0 integration tests) and generics (~25 tests) are materially under-tested. Concrete and actionable; recommend prioritizing items 1-2 (stale-flip) and 7-8 (generics + Content test suites).


## Failing-fence classes
(not found)


## WF-4 close structure (recommended)
RECOMMENDED WF-4 CLOSE STRUCTURE â dependency-ordered, then parallel lanes.

=== PHASE 0: PRE-REQUISITE MERGES (serialize before fan-out) ===
 - Land WF-2G (snapshot-resume heap-element projection) and WF-3D (comptime serde/llm showcases) FIRST. Six audited areas depend on them: snapshot-resume, polyglot-distributed (snapshot row + combined barrier), comptime (chapters 2/4 + showcases suite), plus the state.mdx/resumability.mdx/content-addressed-bytecode.mdx gate subsets. Starting their book/test lanes before merge risks writing examples against a capture path that changes.
 - Also confirm WF-3A (if it touches the strict-typing checker) â the generic-field-position leak fix and set-param/ct_45 are code fixes that should not race a checker refactor.
 - Immediately after Phase 0, re-run the FULL 748-fence universe probe to refresh the numbers for the WF-2G/WF-3D subsets (they are stale on current main).

=== PHASE 1: CODE FIXES (must precede the book flips that depend on them; run as a small serial-ish lane while book lanes work on independent areas) ===
 One "impl-gap" lane fixes the shipped-but-broken items that block gate-green examples, roughly in cost order:
  - Serialization (4 bugs): msgpack::decode wrapper, toml/yaml::parse non-navigable, xml::stringify empty-array crash (op_new_array(0) SURFACE), json .get/.at/.len/.keys/.is_null. HIGHEST product-impact â round-trip is the module's whole purpose.
  - remote::execute {bindings,schemas} value bug; @remote Array-param carrier; @remote module-global capture returns 0.
  - The 10 gate-surfaced Intrinsic/Temporal/HashMap.keys/WrapTypeAnnotation/foreign_marshal stubs (some are V3-S5/W17 monomorphization territory â may exceed WF-4 scope; for those, the book action is "mark not-implemented" rather than block).
  - strict-typing generic-field-position enforcement leak; comptime ct_45 set-param.
 Items the impl lane CANNOT quickly fix (V3-S5 ckpt-5, W17-typed-carrier-monomorphization, async W36 JIT return-kind proof) â hand to their owning lanes as "mark not-yet-implemented in prose"; do not block WF-4 on them.

=== PHASE 2: PARALLEL BOOK+TEST LANES (6 lanes; each owns book flips + test suites for its cluster; worktree-per-lane, pre-created and pinned â Agent isolation:worktree is unreliable per project_parallel_dispatch_hygiene) ===
 LANE 1 â Distributed cluster (pipelines snapshot-resume â remote â polyglot-distributed): heavy shared dependency on WF-2G + the serve/foreign harness + jit-fallback tolerance. One lane owns all three so the harness decision (does the gate spin up a loopback serve + extension?) is made once. RE-VERIFY-after-WF-2G stamp on all three.
 LANE 2 â Polyglot-ffi + extensions: extern C / fn python / fn typescript gate examples, ext install/list tests, Ffi permission enforcement e2e, marshalling-matrix pins. Depends on Phase-1 HashMap-param + Vec<struct> decisions.
 LANE 3 â Security-permissions: caution-block deletion, 17th-permission + frontmatter/[sandbox] docs, prefix-vs-glob + port accuracy fixes, and the HIGHEST-VALUE missing suite (e2e CLI fail-closed) + resource_limits enforcement tests. Self-contained; no WF-2G/3D dependency; can start immediately.
 LANE 4 â Comptime + annotations (RE-VERIFY-after-WF-3D): caution rewrites, type_info/string_lit table entries, flip llm-patterns/annotations/comptime-directive fences, replace_module + extend_expr + carrier test suites. Gated on Phase 0 WF-3D merge.
 LANE 5 â Async + drop-raii: both are 0-runnable-fence chapters with strong existing test suites; both jit-fallback (shared "vm-authoritative or tolerate-fallback" harness decision). No WF-2G/3D dependency; can start immediately. Owns join_strategies output-assertions, async_runtime units, real-concurrency suite (blocked on a resolvable sleep primitive â flag), drop escape.rs.
 LANE 6 â Type-system + core-language + serialization-book: strict-typing NEW chapter + generic_requires_args test (drives the Phase-1 leak fix), core-language stale-flips (generics + !!) + content/generics test suites, and the serialization BOOK flips (gated on Phase-1 impl-gap lane). Largest doc surface; can start the strict-typing + core-language stale-flips immediately, holding the serialization book flips until Phase 1 lands.

=== PHASE 3: HARNESS + GATE-DENOMINATOR WORK (one dedicated infra lane, runs alongside Phase 2) ===
 - Decide + implement the jit-fallback tolerance policy in run-book-truth-gate.mjs (async/drop/ownership/snapshot/remote/serialization all deopt; the gate must either classify a tolerated `[jit-fallback]` line as pass or mark those areas vm-authoritative â otherwise flipping their fences to runnable=true turns them into jit failures).
 - Decide whether the gate spins up a loopback serve + builds the python/ts extension (unblocks LANE 1/2 remote+polyglot matrix cells) or those cells are explicitly annotated manual-verified and excluded from the "documented+gate-green" count.
 - Fix the extractor context bug that split comptime-llm-patterns.mdx:160 `extend(...)` out of its comptime{} block (the gate's single known runnable=true fail).
 - Own the FINAL acceptance measurement: re-run the FULL 748-fence universe (not the curated 247) and report BOTH the gate-reported and real-full-universe numbers, so the denominator trap is closed on record.

=== SEQUENCING SUMMARY ===
 Phase 0 (serial: WF-2G/3D/3A merges + refresh probe) â Phase 1 impl-gap lane + Phase 2 lanes 3/5/6-partial start immediately in parallel â lanes 1/2/4 and the serialization book flips unblock as Phase 0/1 complete â Phase 3 infra lane runs throughout and gates the final measurement. Lanes 3 and 5 are the "start now, no dependencies" fast-start lanes; Lane 1 (distributed) is the critical path (most WF-2G-coupled + needs the harness decision). Target 6 book/test lanes + 1 impl-gap lane + 1 infra lane = 8 parallel agents, with lanes 1/4 and the serialization-book portion of lane 6 held until their upstream merge/fix lands.


## Book work-list
CONSOLIDATED BOOK WORK-LIST FOR WF-4 (ordered by severity; goal = every wave-0..3 feature has â¥1 runnable gate-green vm+jit example, and zero documented-but-false claims). "RE-VERIFY AFTER WF-2G" = snapshot/resume lane; "RE-VERIFY AFTER WF-3D" = comptime serde/llm lane.

=== TIER 1: DOCS ACTIVELY WRONG (highest â they misinform users about shipped features) ===

[A] snapshot-resume â advanced/resumability.mdx + stdlib/core/snapshot.mdx  (RE-VERIFY AFTER WF-2G)
 1. DELETE the "v0.4 preview / not available in v0.3.3 / aborts at runtime" caution blocks (resumability.mdx L10-14, L92-94; snapshot.mdx L40-42). Feature ships and works on main.
 2. Fix signature everywhere: snapshot() returns Result<Snapshot,SnapshotError>. Rewrite every match to `match snapshot() { Ok(Snapshot::Hash(id))=>.., Ok(Snapshot::Resumed)=>.., Err(SnapshotError::Barrier(m))=>.., Err(SnapshotError::PersistFailed(m))=>.. }`.
 3. Fix call form in snapshot.mdx: replace `snapshot::snapshot()` (compile error) with `from std::core::snapshot use { snapshot, Snapshot, SnapshotError }` + bare `snapshot()`.
 4. Correct Recompile-and-Resume section (resumability.mdx Â§CLI Resume Modes #2 + Boundary Rule + Safe Edit Guidance): `shape --resume <hash> <source>` currently hard-errors ("cannot resume with an edited source file in this build"). Mark not-yet-available or move to a flagged roadmap section; do not present as working.
 5. Document currently-undocumented WORKING surface: short-hash git-style prefix resolve + ambiguous-prefix error; SIGINT interrupt-save â exit 130 + "Resume with: shape --resume <hash>"; the SnapshotError Barrier/PersistFailed error model and when a Barrier fires (live heap value across checkpoint on current build).
 6. ADD example `snapshot-basic` (runnable=true): corrected from..use{} + Result-match, scalar-only state, prints a hash line. Under --mode jit it deopts (EnumPayload) â gate harness must tolerate the `[jit-fallback]` line or chapter must note jit runs snapshot code via interpreter.
 7. ADD `resume-roundtrip` (scripted): runâcapture hashâ`shape --resume <hash>`âshow Resumed path.
 8. DO NOT add a heap-array-survives-checkpoint example yet â today it Barriers; add only after WF-2G lands per-element kinded projection.

[B] remote-distributed â stdlib/core/remote.mdx + advanced/polyglot-distributed.mdx + transport-layer/wire-protocol/module-distribution/execution-server
 1. Resolve the contradiction: remote.mdx L13-15 says "v0.4 preview, not in v0.3.3" while polyglot-distributed.mdx says @remote is "genuine today" (both verified working). Delete the false caution in remote.mdx (feature ships) OR gate consistently.
 2. Correct @remote return contract (remote.mdx L43-49, L66-88, L182-197): shipped @remote via __call_raising returns the BARE callee value and RAISES on failure â NOT Ok(value)/Err(message). Remove all `assert ... == Ok(...)` (assert is not a builtin â E0101).
 3. Fix retired-API section (remote.mdx L148-165): `remote::__call` was retired (Q35/OQ-11). Document the real `remote::call` returning Result<R,RemoteError> and the RemoteError enum (stdlib-src/core/remote.shape L104-137) which is entirely undocumented.
 4. Fix remote::execute doc (remote.mdx L116-133): shipped execute returns a constant {bindings,schemas} wrapper, not the value; the "// 42" comment is wrong (also a code bug â see test/impl list).
 5. Remove/annotate Array-param @remote examples (`first`, `double_all` L75-88): @remote on an Array<int> param is a SEMANTIC compile error ("no statically proven typed-array element carrier"); narrow the "works with all Shape value types" claim (L66).
 6. ADD â¥1 gate-runnable remote example (currently ZERO). Candidate: a loopback `remote::ping`-style scalar @remote via a documented harness, or annotate that remote examples require a serve harness the gate spins up.
 7. Document RemoteError enum, receiver-owned permission-over-wire posture, MissingModuleFunction resupply (mechanics exist + Rust-tested but no prose).

[C] security-permissions â advanced/security-permissions.mdx (+ module-distribution.mdx signing)
 1. DELETE the two v0.4-preview caution blocks (L19-21, L378-380) claiming the whole surface is host-side-Rust-only / "every block non-runnable." FALSE: a runnable frontmatter/shape.toml `[permissions]`/`[sandbox]` surface ships and enforces live across all 3 tiers (verified).
 2. Add the 17th permission Ffi (name `ffi.call`, PermissionCategory::Foreign) â chapter says "16 permissions" and omits Ffi + the Foreign category (shape-abi-v1/src/lib.rs:1117,1142,1181).
 3. Document the runnable frontmatter/shape.toml surface: coarse booleans (fs.read/fs.write/net.connect/net.listen/process/env/time/random/ffi), shorthands pure/readonly/full, sub-tables [permissions.fs] allowed/read_only + [permissions.net] allowed_hosts, ffi_languages/ffi_libraries/ffi_symbols.
 4. Document the [sandbox] section (deterministic, seed, memory_limit, time_limit, virtual_fs, seed_files) and CLI flags --max-instructions/--max-memory-bytes/--max-time-ms.
 5. Document behavioural rules: Ffi fail-closed asymmetry (unset ffi defaults false in explicit [permissions] while other perms default true); deterministic-foreign LOAD gate; and that --mode jit is SILENTLY downgraded to interpreter whenever any resource limit is active (materially affects the vm+jit gate story).
 6. Fix accuracy bugs: scope paths are PREFIX-match not glob (module_exports.rs:219-224 strips trailing */** + starts_with, so `/tmp/*` also matches `/tmp/a/b`); net-scope IGNORES the port (module_exports.rs:260).
 7. ADD gate examples (vm-primary, documented jit-fallback): a permission-denied example ([permissions] fs.read=false â "Permission denied"), a resource-limit example (--max-instructions on a loop â "Instruction limit exceeded"), a scope-constraint example. Note these are interpreter-only under jit by design.

[D] comptime â advanced/annotations.mdx + comptime.mdx + comptime-annotations-cookbook.mdx + comptime-llm-patterns.mdx  (RE-VERIFY AFTER WF-3D)
 1. annotations.mdx (13 fences all runnable=false, blanket v0.4 caution L17-34 is FALSE for the common case): DELETE the blanket caution. Flip to runnable=true: (a) @traced before/after on a TYPED-param fn (rewrite the string-concat body at L38-51 to f-strings; add `print(add(2,3))` â vm+jit green), (b) @serializable comptime-derive on a `type`. Keep ONE narrow runnable=false for the genuine limitation: untyped-param `fn process(data)` â "no statically proven typed-array element carrier". (Note: the gate's @traced/@serializable/@registered "Unknown annotation" fails are excerpt fragments missing the annotation decl, not real gaps.)
 2. comptime.mdx: rewrite the pre/post-directive v0.4 caution (L113-120) â set return / replace body / remove / extend WORK now; carve out ONLY `set param` as not-working (ct_45). Add runnable=true examples for set return / replace body / remove target. Add `type_info` and `string_lit` to the Comptime Builtins table (type_info has a 20-test suite yet is absent from the table).
 3. comptime-annotations-cookbook.mdx: drop the Recipe 2 / 2b / 13 v0.4 cautions where the path works; re-verify Recipe-13 `error()` inside comptime fn.
 4. comptime-llm-patterns.mdx: flip all 5 bare ```shape fences (@json_schema, @to_json, @llm_tool, @prompt, extend-expr) to runnable=true â all pass vm+jit. Fix the gate's one real fail: comptime-llm-patterns.mdx:160 `extend (...)` was extracted out of its `comptime {}` context â ensure the fence is self-contained.
 5. Document or explicitly mark unimplemented: there is NO `--diagnostics json` CLI flag; LSDS is internal-only (bundle_compiler.rs:1666-1846). Either add the flag+prose or remove the scope expectation.

[E] serialization-stdlib â stdlib/native/{json,msgpack,toml,yaml,xml}.mdx + datetime.mdx/time.mdx  (blocked on 4 impl fixes below)
 1. json.mdx: replace ALL `x as? T` (parse error) with `x as T?`; fix the "Importing" block from `from std::core::json use {...}` to `use std::core::json` (examples call `json::parse` qualified so from..use breaks them); flip typed-parse + stringify + is_valid + Json-pattern-match to runnable=true (these round-trip in vm AND jit); add a runnable End-to-End example not depending on file IO.
 2. After impl fixes land: msgpack/toml/yaml/xml.mdx â flip encode/decode + parse/stringify + is_valid to runnable=true and add a real round-trip fence (value in == value out) per module.
 3. datetime.mdx/time.mdx: flip pure-computation examples (millis, now/elapsed, DateTime.parse/components/format/iso8601/add_*/to_timezone/is_before) to runnable=true (all vm-correct; jit deopts but correct); drop the `fetch(...)`/`load_csv(...)` pseudo-API examples. NOTE gate stubs: datetime.mdx:344 & :380 SURFACE "GetProp on Ptr(Temporal) not yet kinded (W17-typed-carrier-monomorphization)" â blocked on that fix.
 4. Add a serialization overview note: which modules round-trip natively vs serialize-only (until msgpack/toml/yaml/xml fixes land).

=== TIER 2: SHIPPED+WORKING FEATURE, ZERO GATE-GREEN EXAMPLE (book-gate violations, not wrong claims) ===

[F] async â fundamentals/async.mdx (ALL 11 fences runnable=false â full hard-gate violation)
 1. Add runnable=true self-contained examples (all verified vm-green): async let single (`async fn f(){async let x=42; print(await x)} await f()`â42), multiple async let, join all (show current [TaskGroup] reality), join race, join any, async scope, for-await-over-array.
 2. Resolve the JIT-gate decision: every async fn deopts (`[jit-fallback]` W36 return-kind proof, ADR-006 Â§2.7.5). Either (a) mark async examples vm-only in the gate, or (b) fix the JIT return-kind proof. Document the decision â do not ship silently-fallback jit examples as "jit-green".
 3. Fix book/reality mismatch: join settle prints `[TaskGroup:Settle(2)]` (no runtime error) contra the "surfaces an unimplemented error" caution.
 4. Add a real-concurrency section (WF-2D): async backed by a Tokio worker-thread runtime with real cancellation (task_scheduler.rs) + async-scope cancellation-on-exit semantics. Fix the parse-error fence async.mdx:50.

[G] drop-raii â fundamentals/resource-management.mdx (17/17 runnable=false, fictional stdlib)
 1. Rewrite with gate-runnable self-contained Shape (type + impl Drop { method drop(){print(...)} } + verifiable output). Minimum 5 flipped (all vm-green): (a) basic scope-exit drop, (b) reverse LIFO order, (c) block-scoping early release, (d) early-return drop, (e) loop break/continue drop.
 2. NEW section â ESCAPE / Drop DEFERRAL (ADR-006 Â§2.7.30), the wave-0-3 headline behavior, currently undocumented: "a returned Drop referent's drop() defers to the caller's binding scope" + "a module-bound Drop referent's drop() runs at program end". Add 2 runnable examples (both verified green).
 3. Make async-drop + drop-error-is-logged examples runnable, or clearly retain runnable=false with accurate wording.
 4. Honesty note: Drop/ownership code runs via interpreter fallback under jit (native JIT Drop codegen is v0.4). Fix parse-error ownership-deep-dive.mdx:54.

[H] polyglot-ffi â advanced/native-c-interop.mdx + tooling/polyglot.mdx + python-extension/typescript-extension/extensions.mdx
 1. native-c-interop.mdx (ZERO runnable examples for its core feature): add a gate-runnable extern-C example (`extern "C" fn labs`/`cos` â both vm-green; jit deopts but value-correct).
 2. polyglot.mdx: flip the `std_dev` fn python example to runnable (vm-green sd=4.317...); make one fn typescript example runnable (taddâ21.0).
 3. python-extension.mdx + typescript-extension.mdx + extensions.mdx: add â¥1 runnable executing example each (all 15/16 fences currently runnable=false).
 4. FIX documented-but-broken claims (block until code lands or mark not-implemented): polyglot.mdx:96-118 "Returning Structured Data" Vec<Struct> python return FAILS (foreign_marshal::unmarshal_result no scalar-element arm, ffi-rebuild Â§4.4 stage-7) â mark not-yet-implemented like DataTable; polyglot.mdx:52 marshalling table HashMap<K,V> param FAILS to compile ("Generic HashMap cannot have fields") â remove/flag the HashMap row.

[I] polyglot-distributed â advanced/polyglot-distributed.mdx  (RE-VERIFY AFTER WF-2G)
 1. Only 1 of 4 fences is gate-runnable (trivial `extern C labs`). Add a gate-runnable combined-barrier cell: call snapshot() inside a live foreign frame, match Err(SnapshotError::Barrier(..)) â provable locally with `extern C`, no server needed.
 2. Add a gate-runnable extern-C snapshotâresume cell (the snapshot row is claimed genuine but has zero gate coverage; extern C needs no extension).
 3. The 3 py/ts matrix cells (transfer/snap/combined) need server+extension â WF-4 must decide whether the gate grows a serve+extension harness or these are explicitly annotated manual-verified (must NOT silently count as documented+gate-green).

=== TIER 3: STALE-FLIP + UNDOCUMENTED (working features mismarked or missing prose) ===

[J] strict-typing â NEW fundamentals/strict-typing.mdx (4 of 5 features undocumented)
 1. NEW chapter with gate-runnable vm+jit examples for the 4 load-bearing rules: (a) no-truthiness (`if 0 {}` is a compile error), (b) int/number never unify â intânumber implicit, numberâint + narrowing need `as`, (c) let-generalization (`fn get_none(){None}` used at Option<int> and Option<string>), (d) bare generic names invalid (`let x: Option` error â must write Option<int>).
 2. control-flow.mdx + operators.mdx: add explicit "conditions must be bool; no truthiness coercion" note + runnable rejected/green examples.
 3. variables.mdx + builtin-types.mdx: add the numeric-conversion rule + "generic types require type arguments" rule + a runnable intânumber `as`-cast example.
 4. integer-types.mdx: the `as i8`/`as u8` example emits `[jit-fallback] W11-jit-new-array` â annotate VM-authoritative or replace with a JIT-clean scalar cast.
 5. Document (appendix/strict-typing) that type errors are strict/hard by default (TypeDiagnosticMode flip) â no user-facing guarantee is written today.

[K] core-language â fundamentals/{functions,operators,error-handling,pattern-matching,content,strings}.mdx (MEDIUM: 120/120 runnable fences pass, but 2 working features have ZERO gate example due to stale "broken" comments)
 1. functions.mdx Â§Generics (L135-193): remove the stale "residual cluster" comment; flip fence #7 (L115) to runnable=true (verified `fn first<T>(items:Vec<T>)->T`â10 vm+jit). Add runnable examples for bounds `<T: Display>`, where-clauses, multi-param `<T,U>`, generic struct/enum.
 2. operators.mdx Â§!! (L436) + error-handling.mdx Â§!! (L186/207/224/275/287): DELETE the false "type inference currently rejects Result<T,E> !! string" claim; flip to runnable=true (verified `(call() !! "ctx")?` yields Err-with-context vm+jit). Give `!!` â¥1 gate-green example.
 3. pattern-matching.mdx: add runnable=true fences for struct-pattern `Point{x,y}`, array-pattern `[a,b,c]`, object-pattern, rest `..` (all verified running).
 4. content.mdx L72/L139 + strings.mdx L335-336: replace retired `c"..."` (retired W18.3) with Content-builder / f-string-spec equivalents. Fix parse-errors content.mdx:70,126,463 and enums.mdx:154.
 5. strings.mdx L231/L247: make f-string styling specs (table(...), align, precision, color) gate-green or mark unsupported (only fixed(N) is gated today).
 6. operators.mdx L461 `?.`: keep runnable=false, link the JIT-SIGSEGV tracking; re-verify at JIT close-out.

=== HARD RUNTIME STUBS surfaced by the full-universe probe (documented features that SURFACE/NotImplemented â code fix required before the fence can be gate-green) ===
 - stdlib/core/math.mdx:97 IntrinsicMedian, :123 IntrinsicMax (body migration NotImplemented)
 - stdlib/core/rolling.mdx:37 IntrinsicRollingMean, :62 IntrinsicLinearRecurrence
 - stdlib/core/distributions.mdx:37 IntrinsicDistUniform
 - fundamentals/datetime.mdx:344,:380 SURFACE GetProp on Ptr(Temporal) (W17-typed-carrier-monomorphization)
 - fundamentals/objects-arrays.mdx:372 HashMap.keys SURFACE V3-S5 ckpt-5 (Arc<TypedArrayData> deleted)
 - fundamentals/traits.mdx:71 SURFACE WrapTypeAnnotation depends on deleted ValueWord wrapper
 - tooling/polyglot.mdx:96 foreign_marshal Array<Element> no scalar-element carrier
Plus serialization IMPL bugs (fix before those chapters gate-green): msgpack::decode returns {bindings,schemas} wrapper; toml::parse + yaml::parse return non-navigable Ptr(TypedObject); xml::stringify crashes on empty children (op_new_array(0) SURFACE); json .get/.at/.len/.keys/.is_null unimplemented. Plus remote::execute value bug + @remote Array-param compile error + @remote module-global capture returns 0.


## Test work-list
CONSOLIDATED TEST WORK-LIST FOR WF-4 (named suites per sub-feature). "WF-2G already adds" / "WF-3D already adds" noted where the in-flight lane covers it.

=== snapshot-resume (RE-VERIFY AFTER WF-2G â that lane changes the capture path in crates/shape-vm/src/executor/snapshot.rs) ===
 - NEW bin/shape-cli/tests/cli/resume_roundtrip.rs â spawn `shape run <f>`, capture hash from stdout, spawn `shape --resume <hash>`, assert the Resumed arm + post-snapshot output; plus a `--resume <shortprefix>` variant. (No CLI resume round-trip test exists today â all resume tests are in-process builder calls.)
 - NEW SIGINT interrupt-save e2e â spawn a long-running script, send SIGINT, assert exit 130 + "Interrupting â saving snapshot..." + "Resume with:" line, then resume that hash and assert completion (code at script_cmd.rs:304/:483, untested e2e).
 - NEW short-hash resolve suite â unique-short-prefix success + ambiguous error ("matches N snapshots") + sha256: prefix stripping (CLI e2e and/or resolve_hash units in crates/shape-runtime/src/snapshot.rs).
 - NEW shape-test snapshots_resume/ Result-surface test â assert snapshot() yields Ok(Snapshot::Hash) on scalar state and Err(SnapshotError::Barrier(_)) when a heap value is live across the checkpoint (Barrier-on-live-heap-array is untested).
 - NEW documentation-contract regression â pin `snapshot::snapshot()` is a compile error and `from std::core::snapshot use {snapshot}` + `snapshot()` compiles (guards the doc call-form bug).
 - NEW recompile-resume contract â assert `shape --resume <hash> <source>` returns the "edited source file in this build" error (the passing in-process recompile_same_source_runs_ok masks that the CLI path is non-functional).
 - WF-2G RE-VERIFY: heap-element-array + ModuleFn round-trip green test; round-trip tests for the opaque-at-landing arms DequeOpaque/ChannelOpaque/FilterExprOpaque/MutexOpaque (snapshot.rs:700/709/774/787).

=== serialization-stdlib (impl bugs first â tests must pin round-trip) ===
 - stdlib_json â real value assertions: typed deserialize json::parse(str,Type) field checks, @alias mapping, .get/.at/.len/.keys/.is_null navigation, `as T?` success + Err path, stringify(pretty) round-trip, vm==jit parity. (Current ~17 fns are smoke-only expect_run_ok.)
 - stdlib_modules/msgpack_tests.rs â rewrite: the 7 fns named _encode_decode_* NEVER call decode; add real round-trip equality for int/number/string/bool/array/object + encode_bytes/decode_bytes (guards the {bindings,schemas} decode bug).
 - NEW stdlib_toml â toml::parse value-navigation, stringify round-trip, is_valid true/false (ZERO integration tests today).
 - NEW stdlib_yaml â yaml::parse, parse_all multi-doc count, stringify round-trip, is_valid (ZERO today).
 - NEW stdlib_xml â xml::parse node structure (name/attributes/children/text), stringify incl. the empty-children case that currently crashes, self-closing elements (ZERO today).
 - stdlib_http â add post_json/post_text/post_bytes + put_* + options.headers/timeout coverage (currently compile-level only, ~6 fns).
 - datetime/time â add time::benchmark() + sleep_sync coverage.
 - NEW serialization JIT-parity regression (msgpack/toml/yaml/xml under --mode jit) once deopts addressed or asserted VM-equivalent.

=== polyglot-ffi ===
 - NEW bin/shape-cli ext-install/list test for bin/shape-cli/src/commands/ext_cmd.rs (ZERO coverage today) â `shape ext list` output test + install-flow dry/error-path test.
 - NEW Ffi permission enforcement e2e â assert extern C AND fn python are REFUSED under a deny-Ffi sandbox and admitted under grant (Permission::Ffi, functions_foreign.rs derivation). Also surface a CLI Ffi allow/deny flag if none exists.
 - NEW native_interop marshalling-matrix pins â positive/negative per documented Shape<->Python/TS type incl. the two broken cases (Vec<struct> return, HashMap param). native_interop/marshalling.rs is extern-C-only today.
 - Promote fn python / fn typescript scalar-execution into a NON-ignored tier â today only extern-C labs runs by default; the python/ts e2e in ffi_e2e.rs are #[ignore]d behind `just test-ffi`. Prevents silent regression.
 - NEW JIT polyglot parity â assert fn python/extern-C output identical under --mode jit (documents the deopt-but-correct contract).
 - NEW LanguageRuntimeVTable contract test â exercise error_model (DynamicâResult<T>) + get_shape_source namespace import.

=== remote-distributed (Rust side already strong: remote.rs 43, serve_cmd.rs 9 incl TLS; gaps are the confirmed bugs + integration surface) ===
 - execute-value regression (serve_cmd e2e) â assert execute("7*6").value decodes to 42 (guards the {bindings,schemas} constant bug); cover string/array/object results.
 - @remote-array-param suite â @remote on `fn f(arr: Array<int>)` is a compile error; pin expected behavior (fix or explicit surfaced refusal).
 - @remote-global/closure-capture e2e â ship a fn referencing a module-level binding through the REAL @remote sender path, assert correct value (returns 0 today; existing closure test bypasses sender extraction via manual captures).
 - VM==JIT parity â assert remote programs identical under --mode vm/jit.
 - NEW shape-test integration suite for remote (none exists) â end-to-end @remote / remote::call / remote::execute / remote::ping through the annotation surface.
 - wire-serve status test â pin whether legacy `wire-serve` (181 LOC) is supported/deprecated vs `serve`.

=== polyglot-distributed (RE-VERIFY AFTER WF-2G) ===
 - crates/shape-vm/src/remote.rs â add remote_transfer_of_extern_c_fn_executes_foreign_body_on_receiver (+ python/ts variants extension-gated): all 43 existing remote tests transfer plain Shape fns; the transfer-row genuineness (42/105/21) is manual-only.
 - crates/shape-vm/src/executor/snapshot.rs â add snapshot_between_foreign_calls_resumes_genuinely (pre-snapshot foreign value survives + post-resume re-link); today only the refusal-while-live is tested and it SIMULATES a frame by hand-pushing foreign_frame_stack.
 - add snapshot_inside_transferred_fn_returns_module_fn_barrier (combined limitation #1, ModuleFn no-SerializableVMValue arm) + parameterized_transferred_fn_with_snapshot_refuses_frame_descriptor_arity (limitation #2). Both ZERO coverage.
 - add foreign jit-fallback parity test (foreign-bearing program identical VM/JIT via [jit-fallback]).
 - promote the simulated foreign-frame barrier test to also exercise a real fn python/extern C live frame (extension-gated), proving the barrier against actual invoke_foreign_kinded bracketing.

=== async (WF-2D concurrency) ===
 - join_strategies.rs â add .expect_output(...) value assertions to all 4 strategies (currently run_ok-only, 0 expect_output â tests pass even if results wrong); add a distinguishing race/any winner test.
 - async_let.rs â reconcile the 6 stale "TDD currently fail" comments with shipped reality (binary runs async let correctly): fix analyzer/#[ignore] with accurate reason OR remove misleading comments; add value assertions.
 - NEW real-concurrency suite â prove async let / join all tasks overlap in wall-clock (needs a resolvable sleep/delay primitive; `std::native::time::sleep` currently errors "Unknown qualified call" â dependency).
 - NEW async_runtime.rs unit tests â the Tokio scheduler core has 0 unit tests (spawn, completion, AbortHandle cancellation, oneshot delivery).
 - async_scope.rs â add a deterministic child-cancellation-on-exit test.
 - NEW negative/pinning tests for v0.4-unimplemented surfaces: join-all destructuring, join settle unpack, named-branch `.left` GetProp (currently "GetProp on Ptr(TaskGroup) not yet kinded, ADR-006 Â§2.7.24") â pin exact errors.
 - NEW jit-mode async test â pin async fns deopt gracefully + produce correct output under --mode jit.
 - for_await â add a test over a real async stream (not just a literal array).

=== comptime (RE-VERIFY AFTER WF-3D â showcases.rs already covers @json_schema/@to_json/@llm_tool/@prompt vm+jit) ===
 - NEW annotations_comptime/replace_module suite â `replace module (expr)` payload contract, JSON + source-text forms, vm+jit (0 tests today).
 - NEW extend_expr_codegen suite â `extend (f"...{string_lit(s)}...")` computed-string parse+splice path, independent of the serde/llm stdlib.
 - set_param green test once ct_45 fixed; keep the TDD stub tracked until then (shipped-but-broken directive documented as working in the book table).
 - NEW runtime-wrapper carrier suite â positive typed-param wrapper-executes test + negative untyped-param test pinning "no statically proven typed-array element carrier".
 - build_config()-target/field test â assert {debug,version,target_os,target_arch} shape (only warning-print smoke exists).

=== security-permissions (data-structure layer well-covered; runtime enforcement is the gap) ===
 - HIGHEST VALUE â NEW end-to-end CLI suite (bin/shape-cli or tools/shape-test): assert `shape run` with [permissions]/[sandbox] frontmatter FAILS CLOSED. derive_run_security/apply_run_security in script_cmd.rs are untested (its 18 tests cover only downgrade_mode_for_limits + local-path-deps). Zero e2e coverage of the user-facing surface today.
 - resource_limits.rs enforcement â NO test asserts a real program HALTS with "Instruction limit exceeded"/"Wall time limit exceeded"/"Memory limit exceeded"/"Output limit exceeded" (grep = 0 hits). Tier-3 works live but tick_instruction/record_allocation/record_output/check_wall_time are uncovered.
 - Runtime FFI gate + deterministic-foreign load gate â check_ffi_permission (control_flow/mod.rs:1128) + deterministic_foreign_gate (program.rs:393,423) have no enforcement-site test.
 - Permission-baked-into-content-hash property â no test that identical code + different permissions â different content hash (content_addressed.rs has 0 #[test]).
 - Scope path prefix-vs-glob edge (recursive `/tmp/*` match) + net port-ignored behavior â untested.

=== drop-raii (coverage strong ~300+ tests; two named thin spots) ===
 - NEW tools/shape-test/tests/drop_raii/escape.rs â Drop-side-effect ORDERING for escape deferral: assert a returned Drop value runs drop() in the CALLER's scope AFTER the caller's later statements (observe print side-effects, not just return value). Closes the gap where borrow_refs/drop.rs:338 only checks the return value (42.0).
 - same file â module-bound-owned-Drop-at-program-end runtime assertion (drop() side-effect fires at program exit; verified working but unasserted).
 - (known-v0.4, optional) JIT-native Drop codegen test once emit_drop gains user-Drop dispatch.

=== strict-typing (numeric/truthiness/let-gen strong; generic-require-args + strict-mode thin, one real leak) ===
 - NEW tools/shape-test/tests/type_inference/generic_requires_args.rs â assert bare Option/Vec/Array/HashMap/Result rejected in EACH position: annotation, param, return, AND struct-field. WILL FAIL today on field position (real enforcement leak: `type T { f: Option }` / `{ f: HashMap }` compile clean) â file the leak as a resolver bug + add the regression fence.
 - NEW tools/shape-test/tests/regression/strict_mode_default.rs â assert the shipped default rejects the previously-ReliableOnly-suppressed class (stringâint, heap-ptr reinterpret) as hard errors; if testable, assert default mode == Strict.
 - extend control_flow/stress_if_basic.rs + operators/stress_logical.rs â add unary `!<int>` rejection and `while <int>` / `for`-guard rejection assertions (only if + &&/|| covered today).
 - (optional) promote let-generalization to a shape-test integration suite mirroring the unit corpus (get_none dual-instantiation + value-restriction rejects) for VM+JIT parity.

=== core-language (suite strong; two materially under-tested features) ===
 - tools/shape-test/tests/generics/ â add generics_structs.rs, generics_enums.rs, generics_methods.rs, generics_bounds_where.rs, generics_nested_instantiation.rs (vm+jit). Generics now works end-to-end but coverage is only ~25 tests.
 - NEW tools/shape-test/tests/content/ â Content.text/table/list, nested content, render() dispatch (vm+jit). A 16KB-documented feature with ~0 integration tests today.
 - tools/shape-test/tests/strings_formatting/fstring_format_specs.rs â fixed(N), table(...), align, precision, color specs (styling path untested).
 - tools/shape-test/tests/operators/ â deepen pipe |> (chained, into closures, into methods) beyond the current ~5.
