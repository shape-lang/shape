# WF-2F close record — polyglot × distributed composition (2026-07-06)

Goal (user TOP priority): *"polyglot works with distributed computing together."*
A foreign-function-bearing program (`extern C` / `fn python` / `fn typescript`)
survives per-function remote transfer AND snapshot/resume AND the two combined.

Design: `docs/design/polyglot-distributed-integration.md` (ratified 2026-07-05).
Base: `b6147eb8`. Branch: `wave2/polyglot-distributed`.

## Gates (finisher, release build)

- `just check-clean` — EXIT 0 (workspace compiles clean; only pre-existing snake_case warn).
- `just check-no-dynamic` — EXIT 0 (no forbidden dispatch symbols).
- `scripts/verify-merge.sh` — 15/15 PASSED.
- `just test` (Tier 2, unit+deep) — only the 4 pinned pre-existing
  `shape-jit … jit_closure_capture_*` failures; shape-vm 2887 / shape-runtime
  1488 / all other crates green (0 new failures).
- `just diff-vmjit` — MATCH=466 (≥465), unexpected=0, 1 known-red pre-existing.
  Serve-dependent matrix cells are correctly NOT in the vm/jit corpus.

## Authoritative 9-cell matrix

Sender/executing VM under `--mode vm` and `--mode jit`; real `shape serve`
loopback nodes with `--ffi-languages python,typescript`; extensions loaded from
`extensions/libshape_ext_{python,typescript}.so`. `extern C` uses libc `labs`.

| foreign | transfer (`@remote`) | snapshot→resume | combined (`@remote` + `snapshot()` mid-exec) |
|---------|----------------------|-----------------|-----------------------------------------------|
| **C**          | `CELL_C_TRANSFER=42`  (vm+jit) | `CELL_C_SNAP=99`  (RESUMED)  | `CELL_C_COMBINED=106`  (vm+jit) |
| **python**     | `CELL_PY_TRANSFER=105` (vm+jit) | `CELL_PY_SNAP=120` (RESUMED) | `CELL_PY_COMBINED=106` (vm+jit) |
| **typescript** | `CELL_TS_TRANSFER=21` (vm+jit) | `CELL_TS_SNAP=30`  (RESUMED) | `CELL_TS_COMBINED=22`  (vm+jit) |

- **transfer** — foreign entry travels as a dependency of the `@remote`
  wrapper's blob; executes on the serve node; correct value returns to the sender.
- **snapshot→resume** — `snapshot()` between foreign calls returns
  `Ok(Snapshot::Hash)`; fresh process `shape --resume <hash>` (top-level form, no
  source file) replays as `Ok(Snapshot::Resumed)`; tail value byte-equal.
- **combined** — `@remote` function calls a foreign fn, `snapshot()`s
  mid-execution on the receiver, continues past the barrier, calls the foreign fn
  again. Foreign body executes on **both** sides of the snapshot point; result is
  arithmetically correct.

## Receiver enforcement (axis C, §4.6 / OQ-6)

- `wire-serve` defaults to strict-empty `ffi_languages`: a transferred
  `fn python`/`fn typescript` is refused unless the operator opts the language in
  (`--ffi-languages …`), with a sub-kind-shaped message naming the language +
  `shape ext install` remediation. `extern C` is not language-gated.
- Zero sender trust: the receiver installs its own scope + flips
  `ffi_receiver_strict`; `Ffi` is unioned by the linker and enforced at load/call.
- All marshal is on typed `KindedSlot`/`NativeKind` carriers (ADR-006). No
  ValueWord/tag-decode/bridge/Bool-default. `check-no-dynamic` EXIT 0.

## Residual (routed to lane `wf2f-combined-remote-snapshot-persist`)

The combined cells report `snapstate=0`: a `snapshot()` taken **inside a
remote-transferred function** currently returns a **clean barrier `Err`**
(surfaced, never silent corruption — design §4.5) instead of a persistable
`Snapshot::Hash`. Root cause: receiver-populated `HeapKind::ModuleFn` module
bindings have no `SerializableVMValue` arm (pre-existing W17-snapshot-ModuleFn
follow-up, phase-2d-playbook §3). Consequence: the full three-node X4 compose
(snapshot mid-remote on node A → resume on node B) is not yet persistable; the
program still composes and executes correctly. This is the single gap keeping
WF-2F acceptance at **yellow** rather than green.
