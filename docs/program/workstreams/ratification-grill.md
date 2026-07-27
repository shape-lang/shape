# Ratification grill — five ruling questions (2026-07-27)

Status: **ANSWERED 2026-07-27** (user rulings recorded below; bundle text
updated to match). Enactment/publication awaits explicit go-ahead.

## Rulings (user, 2026-07-27)

- **Q1: B** — spine (#111 → #91) plus three lanes: A = PERF-SUITE,
  B = ERGO-FIX-CHANNEL + ERGO-VAR-TRUTH, C = POLY-STUB-CHANNEL + async
  lane per Q3. Nothing merges before #111's authority commit.
- **Q2: A** — tracker delta approved as disclosed (2 edges, 4
  scope-by-reference expansions), applied at publication with re-fetch
  audit.
- **Q3: C** — fast-track real foreign async. The user challenged the
  "multi-week" estimate, and the code-grounded feasibility scout upheld
  the challenge: "multi-week deep evaluator work" was the correct estimate
  only for TRUE interpreter suspension (a runtime-wide item —
  `VirtualMachine::resume()` is `todo!()`, Phase-2c), which is not how
  Shape's own shipped async works. At parity with Shape's actual await
  model (eager offload + resolve at await, the
  `spawn_async_module_future` pattern), foreign async is **days**: Python
  3–5 days, both languages 1.5–2.5 weeks including hardening. Scoped as
  POLY-ASYNC-OFFLOAD (poly-depth.md), which also fixes two latent defects
  found in the process (foreign-in-spawned-async is broken today;
  extension-instance aliasing is UB-shaped). Truthfulness ordering
  preserved: the rejection (POLY-ASYNC-TRUTH) lands first as OFFLOAD's
  flip-to-green control, collapsible into one branch at supervisor
  discretion.
- **Q4: as recommended** — 4a: Option 1, `!` effect clause + `effect F`
  binders (the `!`/`!!` coexistence judged acceptable); 4b:
  `with supervisor`; 4c: `quote { ... }` with `${hole}` blessed now.
- **Q5: B** — per-category bars (table below), multipliers subject to one
  recorded calibration after the first measured baseline.

---

Original questions as presented follow, for the record.

---

## Q1 — Scheduling: do ERGO/PERF/POLY run beside the semantic spine, or behind it?

Six tickets have zero blockers into the published program: PERF-SUITE,
PERF-ALLOC-SEAM, ERGO-FIX-CHANNEL, ERGO-VAR-TRUTH, POLY-STUB-CHANNEL,
POLY-ASYNC-TRUTH. The frozen program's sole frontier is #111.

Hard constraint under any option: nothing merges to main before #111 lands
the authority commit (this bundle rides it). Lanes may begin in pinned
worktrees immediately; their merges queue behind #111.

**A. Spine-first.** Workstreams wait until the early semantic tickets
(#91/#92, first tracers) land.
- Pro: one focus; supervisor verification undivided; authority proven in
  use before anything builds beside it.
- Con: ergonomics, performance, and polyglot — the axes ruled must reach
  9 — do not move for weeks, for no technical reason; the six tickets are
  code-disjoint from the spine.

**B. Spine + 2–3 lanes (recommended).** Spine claims #111 → #91. Lane A:
PERF-SUITE. Lane B: ERGO-FIX-CHANNEL + ERGO-VAR-TRUTH. Lane C:
POLY-STUB-CHANNEL + POLY-ASYNC-TRUTH.
- Pro: territories are disjoint (benchmarks/JIT vs diagnostics/LSP vs
  extensions/); every lane ticket is a prerequisite for the rest of its
  workstream, so nothing downstream starts on spec debt; verification load
  stays within one supervisor's bandwidth.
- Con: supervisor attention splits four ways; cross-lane merge traffic
  needs the verify-merge gate on every landing.

**C. Full fan-out (6+ implementers at once).**
- Pro: fastest wall-clock start.
- Con: unverified-claim backlog — the documented historical failure mode.
  Verification bandwidth, not implementer count, is the binding constraint.

Ruling needed: A / B / C, and if B, confirm the three lanes.

---

## Q2 — The tracker delta: approve the two edges and four scope expansions?

The published graph is frozen. Ratifying the bundle as drafted touches it
in exactly six disclosed places:

- New edge: ERGO-QUASIQUOTE blocks **#110** (universal-descriptor/string
  deletion) — because quasiquote is the typed alternative that unblocks
  deleting the retained `parse_type_annotation_payload` string arm.
- New edge: the R21 row-in-type slice precedes **#143** EFFECT-CONTRACT.
  #143's published `Blocked by` is exactly `#91, #92, #132`; this adds one.
- Scope-by-reference: **#112, #113, #143, #163** carry acceptance criteria
  "conforms to ADR-014 / ADR-016", and those ADRs gain §8 / §10.

**A. Approve as disclosed (recommended).**
- Pro: keeps the published tickets truthful — without the #143 edge, #143
  could land a contract-row representation the type system cannot express
  and technically "close"; without the #110 edge, #110 could close while
  the string→AST reparse survives (the walk-back shape).
- Con: touches frozen state; requires the issue-amendment + re-fetch audit
  pass at publication.

**B. Approve edges only; pin the four tickets to the ADR text as of the
original publication.**
- Pro: zero scope movement on published bodies.
- Con: the four tickets would then conform to superseded authority — a
  dual-authority reading (§8 exists but #143 ignores it) of exactly the
  kind R14 forbids; follow-up amendment tickets become necessary anyway.

**C. Reject the #110 edge.**
- Con: the deletion E5 already deferred once stays unanchored; documented
  defection-attractor.

Ruling needed: A / B / C.

---

## Q3 — `async fn python`: reject now, keep the lie, or fast-track real suspension?

Today this compiles and "works":

```shape
async fn python fetch_data(url: string) -> string {
    import urllib.request
    return urllib.request.urlopen(url).read().decode()
}
```

Reality: no `Future` exists in the Shape type; the VM thread blocks on a
fresh `asyncio.run` event loop per call. The `async` keyword changes
nothing except claiming a concurrency model that is not provided.

**A. Reject in v1 (drafted; recommended).** Compile error with a
structured diagnostic naming the owning issue; the fix-it deletes `async`,
which preserves the exact current semantics.
- Pro: truthful (ADR-011 §7); the migration is deleting one word with
  zero behavior change; greenfield/no-users ruling makes compat weight
  zero.
- Con: user-visible breakage for any existing `async fn python`; the
  matrix cell changes (paired ADR-012 amendment already drafted).

**B. Keep blocking-async, documented as a caveat.**
- Pro: nothing breaks.
- Con: ADR-016 would document a false contract as a limitation; the
  reviewer flagged this as live dual authority against the matrix.

**C. Fast-track real `Suspend` integration instead.**
- Pro: reaches the end state directly.
- Con: foreign suspension across the vtable is deep evaluator work,
  unscoped and unbudgeted; it would block a one-day truthfulness fix on a
  multi-week feature.

Also rule the timing: the rejection lands with POLY-ASYNC-TRUTH
immediately (recommended — it is a truthfulness fix), or rides
TARGET-PYTHON #163's schedule.

Ruling needed: A / B / C, plus timing.

---

## Q4 — Surface spellings (three sub-rulings)

### Q4a — Effect-row syntax

**Option 1 — `!` clause (as drafted in the ADRs):**

```shape
fn read_config(path: string) -> string ! {FsRead}
fn pure_hash(s: string) -> int ! {}
fn map<T, U, effect F>(self, f: fn(T) -> U ! F) -> Array<U> ! F
```

- Pro: compact; effect-system precedent (Koka-family); already the spelling
  used throughout ADR-012/014, so zero doc churn.
- Con: Shape already uses `!!` as the error-context operator. The parser
  disambiguates trivially, but a human skimming `-> string ! {FsRead}` next
  to `expr !! "context"` reads two meanings of `!`.

**Option 2 — `uses` clause:**

```shape
fn read_config(path: string) -> string uses {FsRead}
fn map<T, U, effect F>(self, f: fn(T) -> U uses F) -> Array<U> uses F
```

- Pro: reads as English; greppable; no sigil overloading.
- Con: new keyword; more verbose at every boundary; all ADR examples need
  a mechanical respell (semantics untouched).

Rejected without asking: annotation-style `@effects(...)` — annotations
are user-space compile-stage transforms (ADR-012); a core type component
must not wear annotation syntax.

Ruling needed: Option 1 or 2 (binder spelling `effect F` is shared by
both; object to it only if you want a different word).

### Q4b — Supervisor scope keyword

```shape
with supervisor durable_fs { let r = fetch(x)?; ... }   // Option 1 (drafted)
supervise durable_fs { let r = fetch(x)?; ... }          // Option 2
```

- `with`: pro — establishes a general scoped-context pattern the language
  may want again (deadline scopes, placement scopes); con — commits a
  general keyword whose other uses are not yet designed.
- `supervise`: pro — single-purpose, unambiguous; con — a one-off keyword
  that cannot be reused, and a second scoped-context feature later would
  introduce a second keyword.
- Note: `using` is not offered — the RAII ruling deliberately excluded
  `using`/`defer` from the language.

Ruling needed: `with supervisor` or `supervise`.

### Q4c — Quasiquote holes

```shape
quote {
    fn ${name}(x: ${ty}) -> ${ret} ! ${eff} { ${body} }
}
```

`${...}` holes (typed per ADR-017 §3: name/type/effect/fragment), chosen
over bare `{...}` which collides with blocks and with f-string `{expr}`.
Alternative: defer the spelling to ERGO-QUASIQUOTE with a required user
sign-off at ticket start.

Ruling needed: bless `quote { ... }` + `${hole}` now, or defer-with-gate.

---

## Q5 — Performance charter composition

### Q5a — Reference pin

The exact Node LTS current at ratification (pinned to a specific version
and V8 build in the suite manifest; upgraded only by explicit decision).
Optionally PyPy/Bun as informational, non-gating columns.

### Q5b — The bar

**A. Geomean parity (as drafted):** suite geomean ≥ 1.0× vs Node.
- Pro: one number; conservatively achievable.
- Con: your stated goal was *better* than V8 given the typing advantage;
  a geomean can hide losing an entire category behind numeric wins.

**B. Per-category bars (recommended):**

| Category | Example workloads | Bar |
|---|---|---|
| Numeric kernels | bspline (the historical 69×-vs-Rust case), matrix ops | ≥ 1.5× Node |
| Collection pipelines | map/filter/reduce chains with closures | ≥ 1.0× (post closure-nativity) |
| Strings/JSON | parse-transform-serialize | ≥ 1.0× |
| Allocation-heavy | object-graph churn | ≥ 0.8× pre-arena, ratcheting to ≥ 1.0× when PERF-ARENA lands |
| Startup | hello-world, CLI-tool cold start | ≥ 5× |

- Pro: honest about where the typing advantage applies; each PERF lane
  landing ratchets its own category; no cross-category hiding.
- Con: five gates instead of one; the exact multipliers are judgment
  calls that will need one revision after the first measured baseline.

**C. Aggressive single bar (≥ 1.25× geomean).**
- Pro: simple and ambitious.
- Con: fails until late in the workstream, making every intermediate
  ticket "red" in a way that carries no information.

Ruling needed: A / B / C; if B, accept or adjust the table's multipliers
(they become the committed gate after the first baseline run calibrates
them, with any adjustment recorded as a dated decision).
