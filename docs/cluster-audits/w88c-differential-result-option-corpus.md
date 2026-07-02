# W88C Differential Result/Option Corpus

Scope: expand the per-commit VM-vs-JIT differential gate with small
Result/Option seeds only. This worker did not run `shape`, `cargo`, `rustc`,
`just`, `nextest`, or `miri`; expected convergence is inferred from existing
W13 corpus seeds and v0.3.3 error-handling acceptance probes.

## Added to `scripts/differential-gate.sh`

| Seed | Carrier/operator | Expected final stdout | Basis |
|---|---:|---:|---|
| `patterns/m03_option_some.shape` | top-level `Some` | `42` | Existing W13 curated seed, already in gate. |
| `patterns/m04_option_none.shape` | top-level `None` | `-1` | Existing W13 pattern seed. |
| `patterns/m05_result_ok.shape` | top-level `Ok` | `10` | Existing W13 pattern seed. |
| `patterns/m09_result_err.shape` | top-level `Err` | `-7` | Mirrors `m05_result_ok.shape` and book-acceptance `Result` matches. |
| `patterns/m10_option_question.shape` | `Option` `?` propagation | `43` | Narrowed from `error-handling/small.shape` `maybe_double`. |
| `patterns/m11_result_question.shape` | `Result` `?` propagation | `12` | Narrowed from `error-handling/small.shape` and `probe_q_infer.shape`, with explicit typed bindings to avoid inference-gap ambiguity. |
| `patterns/m12_result_context_bangbang.shape` | `!!` context plus `?` propagation | `53` | Narrowed from `evidence_bang_bang_explicit_annotation_works.shape`; retains the explicit success binding annotation. |

The script behavior is unchanged: every listed seed still runs through
`shape-fuzz run` and any VM/JIT divergence remains a gate failure.

## Not Promoted

| Candidate | Classification | Reason |
|---|---|---|
| `error-handling/evidence_bang_bang_typed_use_infers_unknown.shape` | Missing language inference feature | The filename and paired explicit-annotation fixture show the unannotated `!!` + `?` shape is an inference gap today. |
| `error-handling/evidence_coalesce_jit_pointer_leak.shape` | Missing JIT implementation feature | Documents a known JIT `??` carrier leak; not a passing per-commit seed. |
| `error-handling/evidence_coalesce_jit_string_abort.shape` | Missing JIT implementation feature | Documents the string form of the same `??` residual; not a passing per-commit seed. |
| `error-handling/large.shape` | Missing implementation feature | `large_runnable.shape` documents that the idiomatic recursive-descent version is blocked by a content-addressed-linker defect. |
| `error-handling/probe_misc.shape` | Spec-intended negative | Contains a top-level uncaught `Err` via `?`; useful as a negative acceptance probe, not as a passing differential-gate seed. |
| `error-handling/probe_uncaught_chain.shape` | Spec-intended negative | Contains a top-level uncaught `None` propagated through `!!`/`?`; useful for exception-display behavior, not this gate. |
| `error-handling/large_runnable.shape` | Not selected | It is a broad book-acceptance program, while W88C needs narrow per-commit seeds. Its small `?`, `!!`, and `??` fragments were used as source material instead. |

## Supervisor Verification

Allowed follow-up lane:

```bash
cargo build -p shape-cli --bin shape
cargo build -p shape-fuzz --bin shape-fuzz
bash scripts/differential-gate.sh
```

Optional focused rerun after a failure can set `SHAPE_FUZZ_FINDINGS_DIR` to a
throwaway directory, but should not change the gate's seed list to hide a
failure.
