# v1 close-summary audit

**Generated:** 2026-05-17 (v1-close seam; pre-tag ratification audit deliverable).
**Worktree:** `bulldozer-strictly-typed-v1-close-summary-audit` (base commit `1d150d3e`).
**Dispatch:** single audit-only sub-agent per supervisor (b) disposition 2026-05-17; ceiling-c bounded.
**Scope:** §0 acceptance criteria + §1 cluster-by-cluster trajectory + §2 ADR-006 amendments + §3 CLAUDE.md modifications + §4 cumulative discipline stats + §5 outstanding post-v1 territory + §6 draft v1 tag annotation + §7 carry-forward post-v1 roadmap.
**Audit discipline:** every commit hash / tag presence / symbol presence / file:line cite grep-verified at worktree HEAD pre-landing (Reading 6 empirical-verification-first; recursive 85-imprecision-instance pattern); any unverifiable claim FLAGGED in §0' below as candidate imprecision.

---

## §0 v1 acceptance criteria

### Original v1 contract (Phase 3 kickoff)

Per `docs/cluster-audits/phase-3-kickoff-prompt.md:89-110`, the v1 close criterion is the 4 kickoff smokes producing IDENTICAL output under `--mode vm` and `--mode jit`:

| Smoke | Fixture | Expected |
|---|---|---|
| s1 | `let mut sum = 0; for i in 0..100 { sum += i }; print(sum)` | `4950` |
| s2 | `let xs = [1,2,3,4,5]; let doubled = xs.map(\|x\|x*2); print(doubled.sum())` | `30` |
| s3 | trait T / impl T for X / `let t: dyn T = box(X{})` / `print(t.name())` (kickoff-prompt prose) | `x` |
| s4 | `let mut s = Set(); s.add("a"); s.add("b"); print(s.size())` | `2` |

**Smoke 3 fixture-vs-prose split (Surface A, user disposition (c) 2026-05-14):**
- Cluster-0/1 close fixture: `let t = X {}` (UFCS dispatch; `/tmp/smokes/s3.shape`)
- Cluster-1.5 close fixture: `let t: dyn T = X {}` (trait-object dispatch; `/tmp/smokes/s5.shape`)

Smoke 5 added at cluster-1.5-q25c close 2026-05-16 per Surface A (c) split (`phase-3-cluster-0-status.md:8351-8358`).

### Smoke matrix at canonical HEAD (4368bc60 = phase-3-cluster-1.5-close target; e2941ef3 = status-doc commit; 1d150d3e = handover-doc commit)

| Smoke | VM | JIT | Status |
|---|---|---|---|
| s1 (scalar loop) | 4950 | 4950 | PASS |
| s2 (`xs.map(\|x\|x*2).sum()`) | 30 | 30 | PASS (V3-S6f cluster-2 closure-wave-1 fix at `cc5ceb0e`) |
| s3 (canonical UFCS `let t = X{}`) | x | x | PASS (γ JITContext at `28bd0a7f`→`14494605`) |
| s4 (Set + .add + .size) | 2 | 2 | PASS |
| s5 (dyn T trait dispatch) | x | x | PASS (cluster-1.5 Q25.C producer-flip at `c421d30a`→`0f347fa4`) |

Matrix preserved verbatim from cluster-1.5 close subsection at `phase-3-cluster-0-status.md:8389` (verified by supervisor 2026-05-17). **5/5 VM == JIT at canonical fixture.** Per dispatch contract, this is ground-truth-of-record for v1 acceptance.

### All 4 cluster tags landed on canonical

Verified at worktree via `git rev-parse <tag>^{commit}` 2026-05-17:

| Tag | Annotated-tag SHA | Points-to-commit | Subject |
|---|---|---|---|
| `phase-3-cluster-0-close` | `c06279fa` | `bb5b2109` | "Phase 3 cluster-0+1 close attempt (status doc + handover doc): V3-S6 5-checkpoint chain MERGED at 50e5c024" |
| `phase-3-cluster-1-close` | `c81d7256` | `bb5b2109` | (same commit as cluster-0; combined close per β2 disposition) |
| `phase-3-cluster-2-close` | `efcf805c` | `938929de` | "Merge cluster-2-cw-D-fam3-collection: per-HeapKind kinded jit_print Family 3 Collection 6 arms; ADR-006 §2.7.5.B Family 3 extension" |
| `phase-3-cluster-1.5-close` | `3bfb0502` | `4368bc60` | "Merge cluster-1.5-v2-raw-empirical-isolation-and-fix: hashmap_filter_all_match SIGABRT RESOLVED + op_set_field_typed:608 ReceiverGuard mirror + CLAUDE.md Known Constraints PARTIALLY RESOLVED" |

### Cumulative discipline-pattern preservation

- **85 imprecision instances** cumulative across cluster-0+1 + cluster-2 + cluster-1.5 trajectory; ALL caught pre-merge (instance log at `phase-3-cluster-0-status.md:8455-8467`)
- **0 bad-code merges into canonical** preserved across entire v1 trajectory
- **4 successful multi-session chains** (D4 + Round 3b + V3-S5 + V3-S6) per status doc cluster-2 close subsection at `phase-3-cluster-0-status.md:8300`
- All gates passing at canonical: `cargo check --workspace --lib --tests` EXIT=0 + `bash scripts/verify-merge.sh` 12/12 PASS + `bash scripts/check-no-dynamic.sh` EXIT=0 (verified pre-flight 2026-05-17 at worktree base commit `1d150d3e`)

### §0' Candidate imprecisions surfaced at pre-flight ground-truth verification

Per Reading 6 empirical-verification-first discipline, the following discrepancies were caught between dispatch text and grep-verified HEAD reality. These are the 86th+ imprecision-instance candidates; surfaced here for status-doc renumbering at merge ceremony:

1. **Dispatch §3 cites "Renames-to-refuse-on-sight broader-family regex (R17, e55b8e71)" as the 2nd CLAUDE.md modification.** Grep-verified HEAD reality: the broader-family regex landed at commit `bcf2b69b` (2026-05-09, "phase-1b-vm Wave-α gate cleanup-rules-baseline: extend Renames-to-refuse with tag-decode family + add regex-shaped baseline rule") — PRE-Phase-3. The R17 commit `e55b8e71` (2026-05-13) landed the §Parallel-implementation-across-producer/consumer-carrier-shape-boundaries entry (a separate sub-section, NOT the broader-family regex). The 2026-05-14 compaction commit `535619cb` preserved both. Classification: audit-text imprecision in handover doc + dispatch prompt; same provenance pattern as instance 32 (stale-count carry-forward) per `phase-3-cluster-0-status.md:8043`. Material to §3 inventory: there are actually 4 modifications under cluster-0+1+2+1.5 trajectory, but their attribution differs from dispatch.

2. **Dispatch §0 "all 4 cluster tags landed" implicit equivalence between `phase-3-cluster-0-close` and `phase-3-cluster-1-close`.** Grep-verified: both tags exist and BOTH point to commit `bb5b2109`. This is correct per β2 supervisor disposition combined-close (`phase-3-cluster-0-status.md:7791-7806`), but the two tags share a single canonical commit — they are distinct annotated-tag objects (SHAs `c06279fa` and `c81d7256` respectively) for ceremonial separation, not separate close events. Surfaced here for §1 trajectory accuracy.

---

## §1 Cluster-by-cluster close trajectory

### §1.1 Cluster-0+1 — CLOSED 2026-05-16 at `bb5b2109`

**Tags:** `phase-3-cluster-0-close` (annot `c06279fa`) + `phase-3-cluster-1-close` (annot `c81d7256`) — both point to `bb5b2109`.

**Close subsection:** `docs/cluster-audits/phase-3-cluster-0-status.md:7749-7858` (Wave 3 Round 3 V3-S6 chain close + cluster-0+1 close attempt; β2 supervisor disposition 2026-05-16).

**Key architectural deliverables (per status doc §"Cluster-0+1-close-criterion gates" at `phase-3-cluster-0-status.md:7791-7806`):**

- Q25.A SUPERSEDED (R20 S2-prime + Wave 3 R2 V3-S5)
- Q25.B SUPERSEDED (Wave 2 Round 3b C2-joint HashMapData<V>)
- Path B canonical Ptr-newtype (D4 Round 4 atomic single-commit at `47b55a63`)
- Audit §4.3 O-3.a + O-3a (TypedObjectStorage + TraitObjectStorage HeapHeader migration) RESOLVED (D4)
- Producer-side foundation (Wave 3 R1 V3-A2-followup-producer-cascade)
- V3-S5 wholesale TypedArrayData enum + TypedBuffer<T> + AlignedTypedBuffer deletion at `9523d57a` (10-checkpoint multi-session chain; 1223 LIVE refs → 0 LIVE refs)
- JIT FFI String/Decimal build (Group X, ckpt-6-prime)
- V3-S6 retroactive 5-checkpoint chain MERGED at `50e5c024` (V3-S6a `43ac9b7a` + V3-S6b `48e05f3f` + V3-S6c `2544f89f` + V3-S6d `2f011e93` + V3-S6e `d4d5454c`)
- 4-table HeapKind::TypedArray lockstep 0/4 LIVE arms (unreachable!() bodies per Rust exhaustive-match requirement)
- Smoke matrix 3/4 VM == JIT at canonical fixture under β2 (Smoke 2 JIT TIMEOUT folded to cluster-2 as V3-S6f canonical fixture; resolved at cluster-2-closure-wave-1)

**Multi-session chains:** 4 successful chains landed under cluster-0+1 trajectory:
- D4 6-sub-agent chain (TypedObjectStorage Arc → HeapHeader; Path B atomic single-commit landing)
- Round 3b C2-joint 4-sub-agent chain (HashMapData<V> per-V monomorphization; ~5k LoC / 40 files atomic)
- V3-S5 10-checkpoint chain (wholesale enum/buffer deletion)
- V3-S6 5-checkpoint chain (retroactive resolver/substitution + side-table + JIT routing + stamping + Gap B+C fix)

**Cumulative through cluster-0+1 close:** 41 imprecision-pattern instances; 9 self-detected defection-attractor refusals; 5 S1-R18 DURABLE PATTERN instances; 8 parallel-implementation defection-attractor instances all structurally resolved at V3-S5 architectural sunset (`phase-3-cluster-0-status.md:7841-7846`).

### §1.2 Cluster-2 — CLOSED 2026-05-16 at `938929de`

**Tag:** `phase-3-cluster-2-close` (annot `efcf805c`) → points to `938929de`.

**Close subsection:** `docs/cluster-audits/phase-3-cluster-0-status.md:8261-8329` (Wave 3 Round 11 cluster-2 close; ALL §A-§I CRITERION GATES MET).

**Key architectural deliverables (per status doc §"Cluster-2 close-criterion gates" at `phase-3-cluster-0-status.md:8273-8288`):**

- §A V3-S6f Smoke 2 JIT TIMEOUT RESOLVED (closure-wave-1 at `cc5ceb0e`; lower_for_expr per-iterable monomorphic state machine via existing MIR vocabulary)
- §B 8/8 user-fn-class coverage matrix complete (Round 1 closure-wave-1 + Round 2 cw-B + Round 3 cw-IB Class B + Round 3 cw-IC Class C)
- §C hashmap-value-v-arm RESOLVED (cw-C at `3db69306`)
- §D 10 shape-test failure classes structured-defer (cw-shape-test-residuals-triage at `0acc3fad`)
- §E per-HeapKind kinded jit_print 13/35 wired (22 UNCOVERED structured-defer to Round 5+ / cluster-3+ per inventory §E.5 family partition)
- §F compile-time-boxed string-constant leak (cw-E audit + cw-E-fix dedup; full elimination Option B deferred cluster-1.5)
- §G W12-collection-constructor-mir-lowering (audit-CLOSED at inventory; ALL 8 named constructors COVERED at HEAD)
- §H cluster-2 wave partition + tracing-crate migration (cw-F + ADR §2.7.5 cross-crate amendment)
- §I Q25.C TraitObject absorb-vs-separate (PRESERVE cluster-1.5 separation per Surface A (c) split)
- Char-literal MIR-lowering (cw-char-literal-mir at `98b5c15d`)
- UAF correctness string-const-in-loop (jit-string-const-loop-retain at `a8a3f50d`)
- Smoke matrix 4/4 VM == JIT preserved at canonical 938929de

**Sub-cluster dispatches:** 6 total per `phase-3-cluster-0-status.md:8298`:
- 1 empirical-verification (cluster-2-empirical-verification at `e0e92613`)
- 1 inventory audit-day (cluster-2-inventory at `71007603`)
- 1 closure-wave-1 (hypothesis a fix at `cc5ceb0e`)
- 4 closure-wave rounds across 4 rounds (Round 1: cw-E + cw-F + cw-2 + cw-C; Round 2: cw-E-fix + cw-2 + cw-C cleanup; Round 3: cw-IB + cw-IC + cw-D-fam12; Round 4: cw-char-literal-mir + cw-shape-test-residuals-triage + cw-D-fam3)

### §1.3 Cluster-1.5 — CLOSED 2026-05-17 at `4368bc60`

**Tag:** `phase-3-cluster-1.5-close` (annot `3bfb0502`) → points to `4368bc60`.

**Close subsection:** `docs/cluster-audits/phase-3-cluster-0-status.md:8417-8479` (Wave 3 Round 15 cluster-1.5 close; ALL CRITERION GATES MET).

**Key architectural deliverables (per status doc §"Cluster-1.5 close-criterion gates" at `phase-3-cluster-0-status.md:8430-8444`):**

- Smoke 5 dyn T VM == JIT LANDED (Q25.C TraitObject rebuild at `c421d30a` → `0f347fa4`)
- Q25.C.1 universal-dyn for TypedObject LANDED
- Q25.C.2 Self-arg vtable-identity check LANDED at invoke_dyn_unified
- Q25.C.5 VTable + 6-variant + thunk emission LANDED (producer/consumer lockstep flip COMPLETE per cluster-1.5-q25c + ADR addendum at `86ad6676`)
- v2-raw-heap-audit Phase 1 (audit-only at `783919d0` → merge `6bc80014`; CLAUDE.md RE-CLASSIFIED 2026-05-16 at `1c1bd64d`)
- Consolidated empirical-isolation-and-fix at `4368bc60` (hashmap_filter_all_match SIGABRT RESOLVED via share-accounting double-release fix at `call_*_with_nb_args*` closure-call boundary)
- op_set_field_typed:608 ReceiverGuard mirror at `fe61d29c` (Phase 4 imprecision 84 residual CLOSED)
- CLAUDE.md Known Constraints PARTIALLY RESOLVED 2026-05-17 at `5c42790f`
- Smoke matrix 5/5 VM == JIT preserved at canonical 4368bc60

**Sub-cluster dispatches:** 4 total per `phase-3-cluster-0-status.md:8448`:
- cluster-1.5-q25c Q25.C TraitObject rebuild
- ADR-006 §Q25.C.5 addendum at `86ad6676`
- v2-raw-heap-audit Phase 1 (audit-only deliverable + §3.A bonus 4 #[ignore] reason-string updates + CLAUDE.md RE-CLASSIFIED)
- Consolidated empirical-isolation-and-fix (which itself bundled audit Phase 2 territory + Phase 4 imprecision 84 territory per supervisor consolidation 2026-05-17)

**Reading 6 candidate confirmed via empirical-isolation surprise:** the consolidated empirical-isolation sub-agent REFUTED all 3 audit-enumerated hypotheses A/B/D (HashMap-carrier territory); actual root cause in `call_*_with_nb_args*` closure-call boundary (Round 13 T5 sibling). Imprecision 85 = audit-scope-expansion class; validates Reading 6 audit-territory-bound-misdispatch extension (`phase-3-cluster-0-status.md:8451`).

### §1.4 Phase 4 — CLOSED 2026-05-16 at `726d6a6a`

**Tag:** none (no separate phase tag per dispatch; merged inside cluster-1.5 trajectory).

**Close commit subject:** "Merge phase-4-trait-add-addassign: Add/AddAssign user-type support + Smoke 2 regression fix + bonus ReceiverGuard RAII pre-existing UB partial fix" (verified `git log 726d6a6a`).

**Key architectural deliverables:**
- trait Add/AddAssign user-type support (language feature dispatch shape)
- UFCS dispatch fallback
- Bonus ReceiverGuard UB partial fix at `op_get_field_typed:341-353` (Phase 4 imprecision 84 territory; mirror at `op_set_field_typed:608` landed at cluster-1.5 ReceiverGuard mirror `fe61d29c` 2026-05-17)
- Smoke 2 regression fix (sub-agent CRITICAL recovery; instance 82 per `phase-3-cluster-0-status.md:8462`)

**Imprecision instances landed:** 80, 81 (team-lead-prompt), 82 (CRITICAL recovery via pre-existing-framing instance 82), 83 (return-typeless prelude-imported trait method poisons Vec.map<U> monomorphization architectural compiler gap), 84 (Arc::from_raw on v2-raw TypedObjectStorage wrong-type recovery flake) per `phase-3-cluster-0-status.md:8460-8464`.

---

## §2 ADR-006 amendment landings inventory (5 amendments)

Per dispatch contract; each amendment grep-verified at `docs/adr/006-value-and-memory-model.md` HEAD.

### §2.1 Q25.A SUPERSEDED (cluster-0+1 trajectory)

**ADR-006 §-cite:** §2.7.24 Q25.A SUPERSEDED preamble at `docs/adr/006-value-and-memory-model.md:5285-5325` ("Q25.A SUPERSEDED — Round 17 cluster-0-transition deletion target (Round 20 S2-prime amendment, 2026-05-14)").

**Driving sub-cluster:** R20 S2-prime W12-typed-array-data-heap-element-migration-prime + Wave 3 R2 V3-S5 wholesale TypedArrayData enum deletion (10-checkpoint multi-session chain).

**Landed at:** R20 S2-prime audit-first deliverables 2026-05-14 (amendment preamble landing); V3-S5 at `9523d57a` (mechanical deletion completion).

**Substance:** Q25.A's specialized-variants-inside-`TypedArrayData` monomorphization SUPERSEDED by Round 17 W12-typed-array-data-deletion authorization (cluster-0-transition strategic-owner authorization 2026-05-13). Per-T monomorphization via `<X>Obj` newtypes (carrier OUTSIDE the enum) replaces specialized arms. Body of Q25.A preserved for historical provenance.

**Verified at HEAD:** `grep -n "Q25.A SUPERSEDED" docs/adr/006-value-and-memory-model.md` returns 3 hits (lines 5285, 5325, 5405).

### §2.2 Q25.B SUPERSEDED (cluster-0+1 trajectory)

**ADR-006 §-cite:** §2.7.24 Q25.B SUPERSEDED preamble at `docs/adr/006-value-and-memory-model.md:5367-5409` ("Q25.B SUPERSEDED — `HashMapValueBuf` deletion (Wave 2 Agent C partial close, 2026-05-15)").

**Driving sub-cluster:** Round 3b C2-joint 4-sub-agent multi-session chain (HashMapData<V> per-V monomorphization runtime + JIT FFI cannot split per type-confusion-window invariant).

**Landed at:** C1 close (Round 1 merge) preamble landing; Round 3b C2-joint at `5654e576` (mechanical migration completion).

**Substance:** `HashMapValueBuf` enum-tagged value-buffer monomorphization SUPERSEDED. `HashMapData` migrates to `HashMapData<V>` per audit §C.4 option (a.2) — HashMapKindedRef carrier with per-V monomorphization at the method tier; HeapValue::HashMap gains a kinded constructor per §2.7.6 / Q8 carrier-API-bound rule.

**Verified at HEAD:** `grep -n "Q25.B SUPERSEDED" docs/adr/006-value-and-memory-model.md` returns 2 hits (lines 5367, 5409).

### §2.3 Path B §2.3 TypedObjectPtr/TraitObjectPtr canonical (cluster-0+1 trajectory)

**ADR-006 §-cite:** §2.3 amendment at `docs/adr/006-value-and-memory-model.md:302-381` ("§2.3 amendment (Wave 2 Round 4 D4 ckpt-final-prime², 2026-05-14): Path B TypedObjectPtr/TraitObjectPtr newtype-as-variant-payload canonical pattern").

**Driving sub-cluster:** D4 6-sub-agent multi-session chain (TypedObjectStorage Arc → HeapHeader migration; PATH B atomic single-commit landing).

**Landed at:** `47b55a63` (Phase 3 cluster-0+1 Wave 2 Round 4 D4 ckpt-final-prime² STRICT CLOSE).

**Substance:** `HeapValue::TypedObject(TypedObjectPtr)` + `HeapValue::TraitObject(TraitObjectPtr)` payloads use `#[repr(transparent)]` newtypes around raw `*const T` storage; manual Drop calling `release_elem → v2_release + Self::_drop`; manual Clone calling `v2_retain`; manual `unsafe impl Send + Sync` (orphan-rule workaround). CANONICAL for v2-raw HeapHeader-equipped storage types only. Arc<String> remains canonical for String payload (ADR-005 §2 exception); no "StringPtr" sibling. Bounded forbidden framings: "TypedObjectPtr shim" / "TraitObjectPtr bridge" / "Ptr-newtype helper" / parallel `Arc<TypedObjectStorage>` payloads alongside Ptr-newtype shapes.

**Verified at HEAD:** §2.3 amendment text present at lines 302-381; "Path B TypedObjectPtr" hit at line 302 + lines 335-337 + line 348.

### §2.4 §2.7.5.B per-HeapKind kinded jit_print (cluster-2 trajectory)

**ADR-006 §-cite:** §2.7.5.B at `docs/adr/006-value-and-memory-model.md:970-1059` ("§2.7.5.B per-HeapKind-family kinded jit_print dispatch arms (cluster-2-cw-D-fam12-jit-print, 2026-05-16)") + extension at `docs/adr/006-value-and-memory-model.md:1061-1154` ("§2.7.5.B extension Round 4 cw-D-fam3 — Family 3 Collection arms (2026-05-16)").

**Driving sub-cluster:** cw-D-fam12 (cluster-2 Round 3, Char + Concurrency family 5 arms) + cw-D-fam3 (cluster-2 Round 4, Collection family 6 arms; extension landed at `a6d4b042`).

**Landed at:** cw-D-fam12 at `50d8e7db` (cluster-2 Round 3); cw-D-fam3 extension at `a6d4b042` (cluster-2 Round 4; close ceremony at `938929de`).

**Substance:** Per-HeapKind `jit_print_<heap_kind>` FFI bodies + routing arms at `crates/shape-jit/src/mir_compiler/terminators.rs` per family partition (per inventory §E.5 at `docs/cluster-audits/cluster-2-inventory.md:691-724`). 13/35 arms wired at v1 close; 22 UNCOVERED structured-defer to cluster-3+ per inventory family partition.

**Verified at HEAD:** `grep -n "§2.7.5.B per-HeapKind\|cw-D-fam12\|cw-D-fam3" docs/adr/006-value-and-memory-model.md` returns 6 hits at lines 970, 1050, 1061, 1063, 1133, 1149.

### §2.5 §Q25.C.5 producer-side cascade addendum (cluster-1.5 trajectory)

**ADR-006 §-cite:** §Q25.C.5 producer-side cascade completion addendum at `docs/adr/006-value-and-memory-model.md:5714-5752` ("Producer-side cascade completion (cluster-1.5 Q25.C TraitObject rebuild ...)").

**Driving sub-cluster:** cluster-1.5-q25c Q25.C TraitObject rebuild + discipline-text lesson instances 77+78.

**Landed at:** `86ad6676` (ADR-006 §Q25.C.5 addendum bundled with cluster-1.5-q25c merge ceremony 2026-05-16 per supervisor authorization).

**Substance:** Producer-side cascade flip completion (3 producer-site flips at `trait_object_ops.rs::op_box_trait_object` + `op_dyn_method_call` + `rebox_self_value` from `Arc::new(TraitObjectStorage{...}) + Arc::into_raw` to direct `TraitObjectStorage::_new(...)` ALLOC-pattern). Discipline lesson instance 77: producer/consumer owner attribution discipline (future ADR amendments name producer-flip-owner + consumer-flip-owner SEPARATELY). Discipline lesson instance 78: struct-`new` (POD constructor) vs `Arc::new(struct-new(...))` (forbidden post-cascade) distinction.

**Verified at HEAD:** `grep -n "Producer-side cascade completion\|cluster-1.5 Q25.C\|instance 77\|instance 78" docs/adr/006-value-and-memory-model.md` returns 4 hits at lines 5714, 5726, 5732, 5752.

---

## §3 CLAUDE.md modifications landings inventory (4 modifications)

Per dispatch contract; each modification grep-verified at `CLAUDE.md` HEAD + commit hash grep-verified via `git log --all --oneline -- CLAUDE.md`.

### §3.1 2026-05-14 compaction (cluster-0+1 trajectory)

**Commit hash:** `535619cb` ("CLAUDE.md: compact Renames-to-refuse-on-sight + Parallel-implementation entries (44.9k → 35.6k chars; rules preserved verbatim)").

**Section modified:** "Renames to refuse on sight" + "Parallel-implementation across producer/consumer carrier-shape boundaries" (compacted to bullet-list comma-separated lines grouped by family; broader-family regex moved to a fenced code block for grep-visibility).

**User-authorization date:** 2026-05-14 (compaction content) + 2026-05-14 (landing as separate dedicated commit per team-lead handover §Decision-authority pattern, R17 + 2026-05-14-compaction precedents). Verified via commit-message verbatim.

**Driving cluster:** cluster-0+1 (Wave 2 trajectory preparation).

**Verified at HEAD:** sections present at `CLAUDE.md:253-279` ("Renames to refuse on sight" + broader-family regex block) + `CLAUDE.md:271-279` (Parallel-implementation entry compacted).

### §3.2 Renames-to-refuse-on-sight §Parallel-implementation entry (R17 cluster-0+1 trajectory)

**Commit hash:** `e55b8e71` ("W12-typed-array-data-deletion audit (Phase 3 cluster-0 Round 17)").

**Section modified:** "Renames to refuse on sight" — appended §"Parallel-implementation across producer/consumer carrier-shape boundaries" sub-section (35 lines added; names 5-instance defection-attractor class R12/R14/R15/R16/R17).

**User-authorization date:** Strategic-owner authorization retroactively-ratified at R17 close (per commit message: "CLAUDE.md amendment landed per supervisor's ADDITIONAL DIRECTIVE").

**Driving cluster:** cluster-0+1 Round 17 W12-typed-array-data-deletion audit (audit-only deliverable; CLAUDE.md amendment bundled).

**Note (per §0' candidate imprecision 1):** Dispatch text + handover doc characterize this as "Renames-to-refuse-on-sight broader-family regex" — actually the broader-family regex landed at `bcf2b69b` on 2026-05-09 (pre-Phase-3 phase-1b-vm Wave-α). What landed at `e55b8e71` was the §Parallel-implementation sub-section under "Renames to refuse on sight". Both texts are at HEAD; clarified here.

**Verified at HEAD:** §Parallel-implementation entry at `CLAUDE.md:271-279` (compacted form post-535619cb). Broader-family regex at `CLAUDE.md:263-267` (separate provenance; predates Phase 3).

### §3.3 Known Constraints v2-raw-heap-audit RE-CLASSIFIED 2026-05-16 (cluster-1.5 trajectory)

**Commit hash:** `1c1bd64d` ("cluster-1.5 v2-raw-heap-audit bonus: CLAUDE.md Known Constraints re-classification + 4 simulation.rs #[ignore] reason-string updates").

**Section modified:** "Known Constraints" — v2-raw-heap-audit entry replaced with re-classified text per audit §3.C (4 sim tests at HEAD blocked by V3-S5 ckpt-5/ckpt-6 SURFACE class, NOT v2-raw-heap aliasing).

**User-authorization date:** 2026-05-16 (supervisor authorization + user explicit authorization 2026-05-16; per commit message verbatim).

**Driving cluster:** cluster-1.5 v2-raw-heap-audit Phase 1 (bundled with merge ceremony at `6bc80014`).

**Verified at HEAD:** Known Constraints entry at `CLAUDE.md:340` references both 2026-05-16 RE-CLASSIFIED + 2026-05-17 PARTIALLY RESOLVED (post-cluster-1.5-close-merge).

### §3.4 Known Constraints v2-raw-heap-audit PARTIALLY RESOLVED 2026-05-17 (cluster-1.5 trajectory)

**Commit hash:** `5c42790f` ("CLAUDE.md Known Constraints v2-raw-heap-audit entry: RE-CLASSIFIED + PARTIALLY RESOLVED 2026-05-17 (user authorization verbatim)").

**Section modified:** "Known Constraints" — v2-raw-heap-audit entry replaced with PARTIALLY RESOLVED status (hashmap_filter_all_match anchor RESOLVED via call_*_with_nb_args* closure-call boundary fix; Phase 4 imprecision 84 territory CLOSED; residuals tracked).

**User-authorization date:** 2026-05-17 (verbatim user authorization + supervisor 2026-05-17 paste-ready text per R17 + 2026-05-14 compaction precedents).

**Driving cluster:** cluster-1.5 consolidated empirical-isolation-and-fix (bundled with merge ceremony at `4368bc60`).

**Verified at HEAD:** Known Constraints entry at `CLAUDE.md:340` includes "PARTIALLY RESOLVED 2026-05-17" + cite to `cluster-1.5-v2-raw-empirical-isolation-and-fix.md` + 4 sim tests + hashmap_filter_all_match SIGABRT RESOLVED + ReceiverGuard mirrors at `op_get/op_set_field_typed` + remaining v2-raw class residuals enumerated.

---

## §4 Cumulative discipline-pattern statistics

Per dispatch contract; tabulated from cluster-1.5 close subsection imprecision instance log at `docs/cluster-audits/phase-3-cluster-0-status.md:8453-8467`.

### §4.1 85 cumulative imprecisions caught pre-merge

Breakdown per source layer (cluster-1.5 close cumulative breakdown line at `phase-3-cluster-0-status.md:8467`):

| Source layer | Count |
|---|---|
| Supervisor-layer | 11 |
| Audit-layer | 17 |
| Team-lead-prompt | 8 |
| Agent-execution-report | 12 |
| Candidate | 37 |
| **TOTAL** | **85** |

**All 85 instances caught pre-merge.** **0 bad-code merges into canonical** preserved across the entire cluster-0+1 → cluster-2 → cluster-1.5 trajectory.

### §4.2 4 successful multi-session chains

Per cluster-2 close trajectory summary at `phase-3-cluster-0-status.md:8300`:

| Chain | Trajectory | Scope |
|---|---|---|
| D4 6-sub-agent | cluster-0+1 Wave 2 Round 4 | TypedObjectStorage Arc → HeapHeader; Path B atomic single-commit landing at `47b55a63` |
| Round 3b 4-sub-agent | cluster-0+1 Wave 2 Round 3b | HashMapData<V> per-V monomorphization (~5k LoC / 40 files atomic; landed at `5654e576`) |
| V3-S5 10-checkpoint | cluster-0+1 Wave 3 Stabilize Round 2 | Wholesale TypedArrayData + TypedBuffer + AlignedTypedBuffer deletion (1223 LIVE refs → 0; landed at `9523d57a`) |
| V3-S6 5-checkpoint | cluster-0+1 Wave 3 Stabilize Round 3 | Retroactive resolver/substitution + side-table + JIT routing + stamping + Gap B+C fix (V3-S6a `43ac9b7a` + V3-S6b `48e05f3f` + V3-S6c `2544f89f` + V3-S6d `2f011e93` + V3-S6e `d4d5454c`; merged at `50e5c024`) |

### §4.3 9 parallel-implementation defection refusals

Per cluster-1.5 close trajectory summary at `phase-3-cluster-0-status.md:8450`: 9 instances total — 8 cluster-0+1 (per supervisor handover §"Defection-attractor instance count — Parallel-implementation class" at `phase-3-supervisor-handover.md:171-184`) + 1 cluster-1.5 audit-scope-expansion class (instance 85 per `phase-3-cluster-0-status.md:8465`).

All 9 instances surfaced and structurally resolved (8 at V3-S5 architectural sunset per `phase-3-cluster-0-status.md:7845`; 1 at consolidated empirical-isolation-and-fix per `phase-3-cluster-0-status.md:8451`).

### §4.4 5 S1-R18 DURABLE PATTERN instances

Per cluster-0+1 close subsection at `phase-3-cluster-0-status.md:7843` (unchanged through cluster-2 + cluster-1.5; pattern operational per user 2026-05-14 4-criterion ratification).

### §4.5 Reading observations operational status

Per team-lead handover §"Reading observations operational at v1-close seam" at `phase-3-team-lead-handover.md:26-31`:

- **Reading 3** (cadence-tightening 2026-05-16): OPERATIONAL. Max ~100 lines / relay; one refinement pass / dispatch prompt; surfacings = facts + one ask; no taxonomy ceremony.
- **Reading 4** (architectural-prediction-subclass-recovery 2026-05-16): OPERATIONAL across 8+ cluster-2 sub-clusters + cluster-1.5-q25c + cluster-1.5-empirical-isolation. In-scope-recovery pattern.
- **Reading 5 candidate** (parallel-sub-agent imprecision-numbering reconciliation): OPERATIONAL across Round 1+2+3+4 (cluster-2) + Phase 4 + cluster-1.5; team-lead renumbers in merge order at status doc subsection.
- **Reading 6 candidate** (territory-misdispatch class; prefer empirical-verification-first for un-mapped or partially-known territories): OPERATIONAL at cw-B (team-lead territory misdispatch instance 55) + cluster-1.5-empirical-isolation (audit-territory misdispatch instance 85). Binding for v1-close audit dispatch per dispatch contract.

### §4.6 Cumulative authorization landings

- **5 ADR-006 amendments** (per §2 inventory above)
- **4 user-authorized CLAUDE.md modifications** (per §3 inventory above)

---

## §5 Outstanding post-v1 territory inventory

NOT v1-blocking; explicit-cite structured-defer per dispatch contract (distinct from refuse #10 hand-wave deferral). Each item: scope estimate + driving subsection + cluster-fold candidate.

### §5.1 Q25.C polish/perf/tooling follow-ups (cluster-1.5-followup territory)

Per cluster-1.5-q25c close subsection per-Q25.C.x disposition table at `phase-3-cluster-0-status.md:8363-8371`:

| Item | Status at HEAD | Scope | Cluster-fold candidate |
|---|---|---|---|
| Q25.C.3 generic method TypeInfo threading | PARTIAL (VTableEntry::Generic dispatch treats as Direct; TypeInfo struct never constructed; not load-bearing for Smoke 5) | small | cluster-1.5-fast-path |
| Q25.C.4 `#[static_only]` opt-out | UNCOVERED at HEAD (zero parser/AST refs; ETO-002 unreachable) | small | cluster-1.5-fast-path |
| Q25.C.6 IC devirtualization | UNCOVERED; depends on Q25.C.3; optimization tier | medium (~500-1500 LoC) | cluster-1.5-fast-path optimization tier |
| Q25.C.7 LSP cost-class inlay hints | UNCOVERED; depends on Q25.C.6; tooling tier | small-medium (~300-500 LoC) | cluster-1.5-lsp tooling tier |

### §5.2 22/35 remaining kinded jit_print arms (cluster-3+ territory)

Per cluster-2-inventory §E.5 family partition at `docs/cluster-audits/cluster-2-inventory.md:691-724`; 7 remaining families with arm counts:

- Numeric/temporal family: Decimal (3), BigInt (4), Temporal (9), Instant (12). 4 arms. MEDIUM.
- DataTable/Content family: DataTable (5), TableView (10), Content (11), IoHandle (13). 4 arms. MEDIUM.
- Native-foreign family: NativeScalar (14), NativeView (15). 2 arms. SMALL.
- Pure-discriminator family: FilterExpr (18), Reference (19), SharedCell (20). 3 arms. SMALL (unusual print-target shape).
- Async family: Future (6), TaskGroup (7). 2 arms. SMALL.
- Matrix family: Matrix (34), MatrixSlice (35). 2 arms. SMALL.
- TraitObject + Closure + TypedObject family: TraitObject (29), Closure (2), TypedObject (1 SURFACE per terminators.rs:620-661). 3 arms. MEDIUM (gated on TypedObject carrier migration).
- ModuleFn: ModuleFn (33). 1 arm. SMALL.

Total 22 UNCOVERED arms across 8 sub-family groupings (cw-D-fam12 + cw-D-fam3 covered Char + Concurrency + Collection at 11 arms; baseline TypedObject/Option/Result preserved; 13/35 wired at v1 close). **Print-formatting fidelity per family, NOT v1 blocker.**

### §5.3 v2-raw class residuals (cluster-3+ candidates)

Per CLAUDE.md Known Constraints update 2026-05-17 (entry at `CLAUDE.md:340`):

- `length_typed_object_empty` SIGABRT
- `w17_comptime_*` SIGABRTs

Territory NOT enumerated; needs empirical-isolation follow-up if pursued. Post-cluster-1.5-close v2-raw class residuals.

### §5.4 ~48 pre-existing shape-test failures (cluster-3+ per-class scope)

Per CLAUDE.md Known Constraints "Pre-existing shape-test failure clusters" entry at `CLAUDE.md:343`; 10 failure classes per V3-baseline-classification (Wave 3 R1 baseline-classification at `phase-3-cluster-0-status.md:7593-7607`):

(a) generic-fn instantiation returning `Null` (stress_generics::generic_identity_*); (b) typed-closure inference regressions (stress_inference_complex::typed_closure_in_array_*); (c) array transformation chains (complex::test_complex_array_transformation_chain, test_complex_bubble_sort); (d) string `.join` (strings::test_string_join_*); (e) window functions (window_functions::basic::window_*); (f) array slice/sort/some (collections::test_array_slice_*, _sort_*, _some_*); (g) destructuring rest (destructuring::array_destructuring_rest).

Mix of inference-loss / monomorphization / v2-raw-heap. Tracked as `shape-test-residuals-audit`. Structured-deferred at cluster-2-cw-shape-test-residuals-triage close (Round 4 at `0acc3fad` per `phase-3-cluster-0-status.md:8270`); per-class disposition for cluster-3+.

### §5.5 Phase 4 imprecision 83 follow-up (architectural compiler gap)

Per `phase-3-cluster-0-status.md:8463`: return-typeless prelude-imported trait method poisons Vec.map<U> monomorphization. Tracked as `phase-4-followup-return-typeless-trait-method-monomorphization`. Architectural compiler gap; cluster-3+ territory.

### §5.6 AGENTS.md V3-S6 chain rows annotation (doc-discipline sub-cluster candidate)

Per cluster-2 Round 3 close subsection at `phase-3-cluster-0-status.md:8238` (cluster-2-closure-wave-1 close subsection at line 7995): ~15+ edits deferred at cluster-2 Round 3 budget overrun. Doc-discipline sub-cluster candidate.

### §5.7 Phase 4 imprecision 84 remaining residuals (cluster-3+)

Per cluster-1.5 close subsection at `phase-3-cluster-0-status.md:8464`: `Arc::from_raw` on v2-raw TypedObjectStorage wrong-type recovery flake territory. ReceiverGuard mirrors at `op_get_field_typed:341` (Phase 4) + `op_set_field_typed:608` (cluster-1.5 mirror at `fe61d29c`) LANDED; residuals tracked post-v1.

---

## §6 Draft v1 tag annotation message

Paste-ready text for `git tag -a v1 -F <message-file>` at user authorization time. Mirrors `phase-3-cluster-2-close` + `phase-3-cluster-1.5-close` tag annotation shape. Commit hash TBD (placeholder `<HEAD>`; likely `e2941ef3` or the v1-close-summary-audit merge commit per supervisor disposition at ratification time).

```
v1: Shape language strict-typing migration complete — 4 cluster tags landed; 5/5 Smoke matrix VM == JIT at canonical fixture

v1 acceptance criteria MET per Phase 3 trajectory:
- Smoke 5/5 VM == JIT at canonical fixture (s1 4950/4950; s2 30/30;
  s3 x/x; s4 2/2; s5 dyn T x/x; verified at canonical 4368bc60).
- All 4 cluster tags landed: phase-3-cluster-0-close +
  phase-3-cluster-1-close both at bb5b2109; phase-3-cluster-2-close
  at 938929de (annot efcf805c); phase-3-cluster-1.5-close at 4368bc60
  (annot 3bfb0502).
- Cumulative discipline preserved: 85 imprecision instances all caught
  pre-merge; 0 bad-code merges into canonical across entire cluster-0+1
  → cluster-2 → cluster-1.5 trajectory; 4 successful multi-session
  chains (D4 + Round 3b + V3-S5 + V3-S6); 9 parallel-implementation
  defection refusals; 5 S1-R18 DURABLE PATTERN instances; Reading
  3-6 operational.

Cluster-by-cluster close trajectory:
- Cluster-0+1 (bb5b2109, 2026-05-16): V3-S5 architectural sunset
  (TypedArrayData enum + TypedBuffer<T> wholesale deletion) + V3-S6
  5-checkpoint chain + Q25.A SUPERSEDED + Q25.B SUPERSEDED + Path B
  Ptr-newtype canonical (D4) + bulldozer cadence operational.
- Cluster-2 (938929de, 2026-05-16): V3-S6f Smoke 2 JIT TIMEOUT
  RESOLVED + 8/8 user-fn coverage + Char-literal MIR + UAF
  correctness + tracing migration + 13/35 kinded jit_print arms +
  ALL §A-§I gates MET.
- Cluster-1.5 (4368bc60, 2026-05-17): Smoke 5 dyn T LANDED + Q25.C.1
  + Q25.C.2 + Q25.C.5 + v2-raw-heap-audit PARTIALLY RESOLVED +
  hashmap_filter_all_match SIGABRT FIXED via call_*_with_nb_args*
  closure-call boundary share-accounting + ReceiverGuard mirrors at
  op_get/op_set_field_typed.
- Phase 4 (726d6a6a, 2026-05-16; merged inside cluster-1.5
  trajectory; no separate phase tag): Add/AddAssign user-type
  support + UFCS dispatch fallback + bonus ReceiverGuard UB partial
  fix.

ADR-006 amendments shipped under v1 (5 total):
1. Q25.A SUPERSEDED (cluster-0+1; R20 + V3-S5)
2. Q25.B SUPERSEDED (cluster-0+1; Round 3b C2-joint)
3. Path B §2.3 TypedObjectPtr/TraitObjectPtr canonical (cluster-0+1;
   D4 at 47b55a63)
4. §2.7.5.B per-HeapKind kinded jit_print (cluster-2; cw-D-fam12 +
   Family 3 extension cw-D-fam3)
5. §Q25.C.5 producer-side cascade addendum (cluster-1.5; at 86ad6676)

CLAUDE.md modifications shipped under v1 (4 total):
1. 2026-05-14 compaction (44.9k → 35.6k chars; 535619cb)
2. §Parallel-implementation entry under Renames-to-refuse-on-sight
   (e55b8e71; R17 audit landing)
3. Known Constraints v2-raw-heap-audit RE-CLASSIFIED 2026-05-16
   (1c1bd64d)
4. Known Constraints v2-raw-heap-audit PARTIALLY RESOLVED 2026-05-17
   (5c42790f)

Post-v1 territory explicitly deferred (NOT v1 scope):
- Q25.C.3/.4/.6/.7 polish/perf/tooling follow-ups (cluster-1.5-fast-
  path / cluster-1.5-lsp candidates)
- 22/35 remaining kinded jit_print arms (cluster-3+ per inventory
  §E.5 family partition)
- length_typed_object_empty + w17_comptime_* SIGABRTs (cluster-3+
  v2-raw class residuals per CLAUDE.md Known Constraints 2026-05-17)
- ~48 pre-existing shape-test failures (10 failure classes per
  V3-baseline-classification; cluster-3+ per-class scope)
- Phase 4 imprecision 83 follow-up: return-typeless prelude-imported
  trait method poisons Vec.map<U> monomorphization (architectural
  compiler gap)
- AGENTS.md V3-S6 chain rows annotation (~15+ edits deferred;
  doc-discipline sub-cluster candidate)
- Phase 4 imprecision 84 remaining residuals (Arc::from_raw v2-raw
  wrong-type recovery flake; ReceiverGuard mirrors LANDED, residuals
  tracked)
```

---

## §7 Carry-forward post-v1 roadmap candidate

NOT v1 scope; surfaced for next-cycle planning. Stop short of session-count estimates; defer to next-cycle supervisor.

### §7.1 cluster-1.5-followup (Q25.C polish/perf/tooling)

| Theme | Items | Scope | Dependency | Recommended dispatch |
|---|---|---|---|---|
| Q25.C.3 generic TypeInfo threading | PARTIAL → COMPLETE TypeInfo struct construction + VTableEntry::Generic dispatch differentiation from Direct | small | none | audit-first (small territory; behavior changes language semantics for generic methods on dyn) |
| Q25.C.4 `#[static_only]` opt-out | UNCOVERED → parser/AST/desugar + ETO-002 generation | small | none | audit-first (language-design surface; ETO-002 user-facing) |
| Q25.C.6 IC devirtualization | UNCOVERED → JIT IC state machine extends to dyn-call sites | medium (~500-1500 LoC) | Q25.C.3 (TypeInfo needed for IC arm classification) | bulldozer-wave after Q25.C.3 (mechanical territory once architecture ratified) |
| Q25.C.7 LSP cost-class inlay hints | UNCOVERED → LSP hover/inlay surfaces Direct/Generic/IC cost class | small-medium (~300-500 LoC) | Q25.C.6 (cost class derived from IC state) | bulldozer-wave after Q25.C.6 |

### §7.2 kinded jit_print family completion (cluster-3+)

| Theme | Items | Scope | Dependency | Recommended dispatch |
|---|---|---|---|---|
| Per-HeapKind family completion | 22 arms across 8 sub-family groupings per cluster-2-inventory §E.5 (Numeric/temporal + DataTable/Content + Native-foreign + Pure-discriminator + Async + Matrix + TraitObject/Closure/TypedObject + ModuleFn) | medium per family (4-arm families); 2-3 closure-waves total | TraitObject family gated on TypedObject carrier migration | bulldozer-wave per family; mirror cluster-2 cw-D-fam12 + cw-D-fam3 dispatch shape |

### §7.3 v2-raw class residuals (cluster-3+)

| Theme | Items | Scope | Dependency | Recommended dispatch |
|---|---|---|---|---|
| length_typed_object_empty + w17_comptime_* SIGABRTs | territory NOT enumerated; needs empirical-isolation per CLAUDE.md Known Constraints 2026-05-17 | unknown (audit-first required) | none | audit-first (mirror cluster-1.5-v2-raw-empirical-isolation-and-fix shape) |

### §7.4 shape-test residuals 10-class triage (cluster-3+)

| Theme | Items | Scope | Dependency | Recommended dispatch |
|---|---|---|---|---|
| Per-class triage of 10 failure classes per V3-baseline-classification | inference-loss / monomorphization / v2-raw-heap mix; ~48 tests total | medium per class | none | audit-first per class (cluster-2-cw-shape-test-residuals-triage triage doc at 0acc3fad provides per-class anchor cites) |

### §7.5 Phase 4 imprecision 83+84 follow-ups (compiler gap territory)

| Theme | Items | Scope | Dependency | Recommended dispatch |
|---|---|---|---|---|
| imprecision 83 return-typeless prelude trait poisoning | architectural compiler gap; affects Vec.map<U> monomorphization for prelude-imported trait methods | medium-large | none | audit-first (compiler-architecture decision required) |
| imprecision 84 Arc::from_raw v2-raw wrong-type recovery | ReceiverGuard mirrors LANDED at op_get/op_set_field_typed; remaining residuals territory unknown | unknown (audit-first required) | none | audit-first |

### §7.6 doc-discipline sub-cluster (process)

| Theme | Items | Scope | Dependency | Recommended dispatch |
|---|---|---|---|---|
| AGENTS.md V3-S6 chain rows annotation + carry-forward sub-cluster row backfill | ~15+ edits deferred at cluster-2 Round 3 budget overrun; cumulative across cluster-2 + cluster-1.5 trajectory likely 30+ rows | small-medium | none | bulldozer-wave (mechanical doc edits; bounded scope) |

---

*End of v1 close-summary audit. Per dispatch contract: audit-only close; NO source changes; standard close gate; AGENTS.md row append + status doc subsection line at merge ceremony. Tag annotation §6 paste-ready for user authorization at ratification time.*
