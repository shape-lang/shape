# Audit resolution status — live tracker

Companion to `audit-2026-07-04-claimed-vs-real.md` (v0.3.2 @ `1fb805b3`). Tracks each confirmed finding against current main (`1ac7a922`, ~100+ commits ahead). Status grounded in **who verified it**: `Fable` = independent-model reproduction (`fable-verify-results.md`); `refuter` = the fixing workflow's adversarial verifier; `claimed` = workflow reported fixed, NOT independently re-verified; `recon` = WF-4-recon measurement.

**Legend:** ✅ RESOLVED · 🟡 PARTIAL/deopt · 🔵 IN-FLIGHT · ⏳ QUEUED · ⏸ DEFERRED (by ruling)

## Executive verticals (§1)

| Vertical | Audit | Now | Verified by |
|---|---|---|---|
| Q1 Comptime | Partial (segfault, broken introspection) | ✅ works (visibility/type_info/diagnostics/method-emit) | Fable (set-return+strict) + WF-1B/3D refuters |
| Q2 Polyglot | Broken (all foreign dead) | ✅ works locally (CPython/deno/extern-C real) | **Fable** (extension-removal probes) |
| Q3 Distributed+resume | Broken both | ✅ resume + plain `@remote` + **`@remote`×foreign + `remote::call` Result all GENUINE** (merged `800fb6b9`) | **Fable ×3** (WF-3E finisher + 2 independent re-proofs, 9/9 genuine server-side) |
| Q4 C bindings | Broken at runtime | ✅ works (real `.so`, out-params, both modes) | **Fable** |

## §6 confirmed critical/high findings (29)

| # | Finding | Status | Lane / verified by |
|---|---|---|---|
| 1 | op_call_foreign kills all polyglot | ✅ | WF-2A · **Fable** |
| 2 | extern C stubbed; declaration fatal | ✅ (lazy linking) | WF-2A · **Fable** |
| 3 | JIT foreign path dead | 🟡 clean deopt (D5); native path follow-up | Fable residual #5 |
| 4 | Extension V8 never invoked (overstated) | ✅ | **Fable** (ts works) |
| 5 | Comptime `set return` segfault | ✅ compile-error, exit≠139 | WF-1B · **Fable** |
| 6 | Annotation hooks dropped under JIT | 🟡 output correct via deopt; native = P9 | WF-1A(c) lane |
| 7 | `@remote`/`remote::__call` non-functional | ✅ **composition genuine** (transfer+snapshot+combined, server-side) | WF-3E merged `800fb6b9` · **Fable ×3** |
| 8 | `snapshot()` never completes | ✅ | WF-2B · **Fable** |
| 9 | `--resume` can never succeed | ✅ | WF-2B · **Fable** |
| 10 | async let zero concurrency | 🟡 **claimed** (WF-2D), NOT independently verified | ⚠ needs Fable |
| 11 | stdlib calls uncompilable in async fns | 🟡 claimed (WF-2D), unverified | ⚠ needs Fable |
| 12 | top-level `await time::sleep` panics | 🟡 claimed (WF-2D), unverified | ⚠ needs Fable |
| 13 | http segfaults every call | ✅ claimed (WF-2E) | verify in WF-4 |
| 14 | `xml::stringify` dumps core | 🟡 SIGSEGV+id-41 gone (M1); crash root is `op_new_array(0)` → **V3-S5 empty-array construction stub** (deferred; refuse-on-sight to band-aid) | → V3-S5 lane |
| 15 | `std::finance` unusable | 🟡 compiler stack-overflow **M1-fixed/no-repro**; survivor = **stale stdlib source** (~85 `let`→`let mut` immutable-reassignment sites + undefined `signal`) → **stdlib-source-modernization** (not a compiler bug) | → source sweep |
| 16 | json module mostly dead | ✅ **navigation FIXED** (marshalled-Json `Ok(v)=>v.get/is_null` via Result-payload inference) — pending merge (stdlib-tail) · **Opus-indep** | WF-3A-tail |
| 17 | msgpack 100% stubbed | ✅ **decode+navigate FIXED** (shared json root) — pending merge · **Opus-indep** | WF-3A-tail |
| 18 | time module broken | ✅ claimed (WF-2E) | verify in WF-4 |
| 19 | JIT double-execution | ✅ prints once | WF-1A · **Fable** |
| 20 | Tiered compilation inert | ⏸ deferred | D4 ruling |
| 21 | HashMap.filter garbage under JIT | ✅ vm==jit | WF-1A · **Fable** |
| 22 | Drop-error guarantee unimplemented | ✅ | WF-1C |
| 23 | Comptime descriptor key corruption | ✅ descriptors correct | WF-1B · **Fable** |
| 24 | `state::hash` constant digest | ✅ 3-distinct | WF-1B/2B · **Fable** |
| 25 | `__original__(args)` garbage | ✅ base(5)=12 | WF-1B · **Fable** |
| 26 | i64 overflow VM/JIT split-brain | ✅ traps under jit | WF-1A · **Fable** |
| 27 | `serve --sandbox strict` no-op | ✅ blocks file::write server-side | WF-1D · **Fable** |
| 28 | Load-time permission check dead | ✅ **over-wire enforced** (strict node refuses transferred fs.write at load, permission-union) | WF-3E merged `800fb6b9` · **Fable ×2 (D5)** |
| 29 | bigint unconstructible | 🟡 **feature-scale, not a bug** — no construction path, no arithmetic (`HeapValue::BigInt` is an i64-backed placeholder), no grammar literal/suffix, no numeric-lattice integration. D2 "implement" = a real feature build (like decimal), not a point fix → own feature lane | → bigint feature |
| 30 | Drop broken at escape boundaries | ✅ no use-after-finalize | WF-1C · **Fable** |
| 31 | Reference cycles leak unboundedly | ⏳ | WF-3C / D3 |
| 32 | LSP false error on valid extern C | ✅ merged `c2a34826` (LSP now mirrors the compiler oracle: `dynamic_language = !is_native_abi()`) · **Opus-indep** | WF-3B-LSP |

## Cross-cutting

| Item | Audit | Now |
|---|---|---|
| Strict typing / ReliableOnly bypass (§7) | works | ✅ **Fable re-confirmed** — no unsoundness, all probes compile-error |
| Book truth (§3) | 73% (450/616) | 🔵 recon measured **49% (368/748)** full universe → WF-4 close |
| Parser exponential blowup (§4.10) | critical | ✅ WF-0C |
| Dead code / gates (§5, ~10,500 LOC) | inventory | ⏳ WF-3B/cleanup (partly WF-0A) |

## Tallies (of the 32 §6 rows)

- ✅ RESOLVED: **18** (11 Fable-verified) · 🟡 PARTIAL: **8** · 🔵 IN-FLIGHT (WF-3E): **2** · ⏳ QUEUED (WF-3A/B/C): **5** · ⏸ DEFERRED: **1**

## Open verification gaps (things claimed-but-not-independently-verified)

1. **Async (§4.5, #10–12)** — WF-2D reported fixed; Fable never checked it; recon says VM-works/no-JIT/no-book-examples. **Meta-audit REFUTED the green: user-defined async fns still serial (2×1s = 2005ms).** → **WF-2D-fu async-real-concurrency**. Highest-priority unverified-claim correction.
2. **Stdlib serialization** (#14/16/17) — WF-2E "green" but recon found live bugs (`msgpack::decode`, `json`/`xml` navigation) → WF-3A + WF-4 re-verify.
3. **http/time** (#13/18) — WF-2E claimed; not re-run at HEAD.

## Meta-audit fold-in (2026-07-06, workflow `wf_189aa86e-6d5`, 12-lane independent)

**27/37 confirmed findings hands-on verified fixed at HEAD; gates green; self-correction loop proven.** Audit defect-claims 29/29 accurate; its meta-claims (parser root-cause, 616 denominator, 73% book-truth, "jit dead") were not.

### 12 NEW confirmed defects (each reproduced from scratch by an independent refuter) → owners
| Sev | Defect | Owner | Status |
|---|---|---|---|
| CRIT | SIGINT-save mid-builtin → silently-corrupt snapshot | WF-3F | ✅ merged `5dc83444` (2 layers: conditional resume-marker + `from_snapshot` stack-base double-reserve) · **Fable** (12/12, revert-proof). Residual routed: SIGINT-in-JIT dropped (not corruption) |
| CRIT | ~~`extensions/*.so` debug-profile load SIGSEGVs host~~ → **framing corrected: structural-ABI validation gap** (integer gate only, no repr(C) layout fingerprint) | WF-2A-fu | ✅ merged `ddb6a01e` (fingerprint gate; skew now fails cleanly, not SIGSEGV) · **Fable** |
| CRIT | `[native-dependencies]` alias resolution dead (`resolve_library_target` hardcoded) | WF-2A-fu | ✅ merged `ddb6a01e` (resolution set threaded into VM+JIT) · **Fable** (differential vs pre-fix) |
| HIGH | Module-scope closure-capture Drop finalizer leak (§2.7.30.4) | WF-3C | ⏳ (with real GC) |
| HIGH | `--max-output-bytes` inert; `--max-memory-bytes` `panic!`-exits (serve DoS) | WF-3B | ✅ merged `47c8e031` (record_output wired at print sink; grow refuses+surfaces, exit 1 not 101; serve worker survives) · **Opus-indep** |
| HIGH | `remote::call` Result fiction + closure args skip compile-check | WF-3E | ✅ merged `800fb6b9` (Ok+Err reachable; integration-tested) |
| MED | `time::millis()->float` breaks operand-position inference | WF-3A-tail | 🔄 partial (let-binding works; `millis()-start` operand **in repair** `wf_8b4bff6e` — inference-tier propagation). Root corrected: not a float/number alias — the module-call return type doesn't reach the inference tier |
| MED | Two closure-return compile bugs | → grammar lane | 🟡 **both are `\|`-grammar ambiguities, not type bugs**: (a) tail closure after a stmt (`\|a\| a+base`) consumed as bitwise-or across the newline (needs newline-significant/ASI grammar); (b) unbraced typed-param body (`\|x:int\| x+1`) reads `int \| x` as a union type. The "mis-proves number" symptom did NOT reproduce. → Pest grammar change |
| DOC | Book teaches retired `__original__(args)` ×5; `json.mdx` non-existent `as?` cast | WF-4 | ⏳ |

### WF-3E actual state (corrected 2026-07-06)
Branch `wave3/distributed-composition-fix` @ `0efa7561` has **4 wip commits, 982 insertions/13 files** (fixAB transfer+receiver-init, fixC remote-call-Result, fixDE perms+ffi, repair D1 arity + D7 namespace-snapshot + serve extension-double-load; new `remote_builtins.rs` +334). **The workflow process DIED before its finisher gates + Fable re-proof ran → these fixes are UNVERIFIED. Do NOT merge until the independent 9-cell re-proof + D5-perms-over-wire confirmation pass** (§6quaterdecies gate).

### Rulings applied (2026-07-06): SIGINT=release-blocking · D3=real GC · D4=delete · WF-3A=split · D2=implement · async=re-route.

### WF-3A M1 close (2026-07-06, merged `1f9b05be`) — schema-identity ROOT fix
Content-derived `SchemaContentId` + per-Runtime `intern_content` handle replaces the two counters; deleted all 4 point-patches + 2 dedup caches. **Retires the recurring schema-id collision family** (object-spread `extended.z`, json/xml id-41, cross-registry equality). Object-spread repros un-ignored + passing; independent Opus re-proof CONFIRMED (order-shuffle w/ Snapshot-enum trigger, named-nominal/anon-structural, no fallback reborn); supervisor code-inspection cleared the retained `reserve_handles_above` (preserve-path intern-index hygiene, not identity-suppression). Gates: check-clean/check-no-dynamic 0, verify-merge 15/15. **Downstream now likely resolved (re-verify in WF-4/stdlib tail):** `wf3e-remote-execute-projection` (shared the id-41 root), json navigation (#16), xml (#14) — the arity heuristic they leaned on is deleted. **M2** (content-id wire/snapshot carrier + blob-hash determinism) = ratified follow-up.

### WF-3E close (2026-07-06, merged `800fb6b9`) — user #1 priority, doubly-verified
Convergent verification: WF-3E's own Fable finisher (verify-merge 15/15, diff-vmjit MATCH=466) + an independent 2× Fable re-proof (GREEN 9/9 genuine server-side, client `.so` isolated aside). Merged-state re-gated: check-clean, check-no-dynamic EXIT 0, 7/7 remote integration tests, verify-merge 15/15. Semantic merge (WF-3E ∩ WF-3D on `compiler_impl_initialization.rs`) auto-resolved + `--all-targets`-verified.
**5 residual lanes routed** (all pre-existing, NOT regressions): `wf3e-remote-inframe-snapshot-persist` (persistable snapshot inside a remote frame — today clean §4.5 barrier Err) · `wf3e-annotation-args-heap-carrier` (@remote Array/object param = compile error) · `wf3e-remote-global-capture` (@remote reading module-global returns 0) · `wf3e-remote-execute-projection` (remote::execute renders via JsonValue projection — **shares the schema-id/projection family, high blast radius → couple with WF-3A-schema**) · `wf3e-extension-version-hash` (D8: content_hash omits extension version).
**Process note:** the original WF-3E was still running (~3.9h) when a stale 0-byte output file led me to think it had died; I launched a redundant independent verify. Net-positive (convergent 2-model verification) but the lesson: check a workflow's journal/live status before concluding it died from an empty output file.
