# Wave 25 State/Resume Book Targets

Date: 2026-07-09

Scope: supervisor target note for the state/resume/content-addressed lane.
This is not a proof that the rows can flip; it records the exact disabled book
surface the implementation worker should measure against.

## Current Manifest

Source:
`/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`

- Generated: `2026-07-09T21:02:13.224Z`
- Total snippets: 745
- Runnable snippets: 535
- Disabled snippets: 210
- Deferred snippets: 0

## State Rows

`stdlib/core/state.mdx` has 14 disabled rows in scope:

- Line 163: conceptual `state.capture_all()` / `VmState` fields.
- Line 174: `state.capture_module()`.
- Line 186: `state.capture_call(train, [my_data, 100])`.
- Line 216: `state.resume(vm)` after `capture_all`.
- Line 234: `state.resume_frame(state.capture())`.
- Line 327: `state.hash` / `serialize` / `deserialize` cache shape.
- Line 367: `state.diff(before, after)` on a `Portfolio`.
- Line 387: `state.patch(before, delta)`.
- Line 395: diff/patch over transport.
- Line 416: `state.caller()`.
- Line 450: `state.locals()`.
- Line 473: `capture_call` remote payload.
- Line 503: `capture_module` plus diff-based sync.
- Line 530: cached call using `fn_hash`, argument hash, and deserialize.

## Content-Addressed Rows

`advanced/content-addressed-bytecode.mdx` has 9 disabled rows adjacent to this
lane:

- Line 154: `state.capture()` / `state.capture_all()`.
- Line 168: `state.resume(vm)`.
- Line 226: broad Portfolio example using most state APIs.
- Line 264: content-addressed store with `hash` / `serialize` / `deserialize`.
- Line 282: remote annotation pseudocode.
- Line 321: FaaS annotation pseudocode.
- Line 396: migratable annotation pseudocode.
- Line 515: transport example.
- Line 541: `caller` / `args` / `locals`.

Only rows backed by real state API behavior should become flip candidates.
Annotation, transport, and host-store rows remain proof/fixture work unless the
implementation genuinely supports the complete snippet.

## Resumability Rows

`advanced/resumability.mdx` has 2 disabled rows around CLI `snapshot()` resume.
These are not state-builtin flips unless a state change directly improves the
snapshot/resume CLI path and a release-binary book gate proves it.

## Honest Flip Criteria

- `capture_module` must return a structured `ModuleState`, not a diagnostic.
- `capture_call` must carry a real function identity/hash and real argument
  payloads, not synthesized placeholders.
- `diff` / `patch` may start narrow, but the supported value domain must be
  explicit and tested.
- `resume` must not claim full live-frame restore unless captured state carries
  validated frame slots and resume IPs.
- `resume_frame` should remain disabled until `FrameState` grows resumable
  structural fields; metadata-only `FrameState` is not enough.
- `caller` / `locals` can flip only if the top-level Option and non-string local
  carrier issues seen in Wave 23 are actually fixed.

## Wave 25 Result

Wave 25 closed the bounded `capture_module` row only.

- `state.capture_module()` now returns a schema-backed `ModuleState` with real
  `schemas: HashMap<string, string>` and bounded homogeneous module binding
  carriers.
- Imported module namespace objects are skipped by schema identity (`__mod_*`)
  before module binding projection, so the normal `use std::core::state` call
  path works.
- `stdlib/core/state.mdx` line 174 was rewritten as a deterministic runnable
  schema-hash example.
- Extraction now reports 745 total / 536 runnable / 209 disabled.
- Full release book gate passed 536/536 in
  `run-p205564-i30836681.service`; report:
  `/tmp/shape-wave25-book-truth-report.json`.

Rows still disabled from this target set are real remaining gaps or fixtures:
`capture_call`, `diff`, `patch`, full `resume`, metadata-only `resume_frame`,
heterogeneous `ModuleState.bindings`, transport/security/comptime pseudocode,
and CLI snapshot/resume examples.
