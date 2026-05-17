# ADR-006 Alternative B (DRAFT) — Aggressive state-of-the-art

Status: **DRAFT — design alternative B of three.** Not a recommendation. Not a
commit. Surfaces tradeoffs honestly so a decision can be made among A / B / C.

Author: runtime architect, 2026-05-08.

Audience: language designers, compiler engineers, anyone evaluating which of
the three ADR-006 alternatives Shape should pursue.

---

## 0. How to read this document

Alternative B answers the supervisor's 2026-05-08 brief by combining
**production-proven techniques more aggressively than any single existing
language has shipped**. Each component is shipped somewhere; the *combination*
is novel. The chapters that the document leans on most heavily are:

- *Survey 01* (`docs/research/01-ownership-gc.md`): Perceus / Lean 4 / Roc /
  Mojo / Vale / OCaml-modes / FBIP / Frame-Limited Reuse / frozen-cyclic RC.
- *Survey 02* (`docs/research/02-layout-runtime.md`): JVM Lilliput / Wasm-GC /
  HotSpot tagless interpreter / V8 hidden classes / Cranelift+ISLE /
  Project Panama / Mojo MLIR.
- *Survey 03* (`docs/research/03-strings-arrays.md`): V8 strings / Swift SSO /
  Mojo string / Roc list / Erlang refc-binary / Arrow C Data Interface /
  Mojo SIMD / NumPy strided.

Surface semantics are pinned by `shape-web/book/` and the pest grammar — the
design changes only the runtime *underneath* those semantics. Forbidden patterns
in `CLAUDE.md` (single-discriminator discipline, no NaN-box reintroduction, no
`SlotKind::Dynamic`, no `Convert<X>To<Y>` opcodes) are honored throughout: this
design has no dynamic dispatch path to gate, rename, or "keep for one edge
case".

---

## 1. Executive summary

### 1.1 The thesis in one paragraph

Shape's runtime is a **modal-typed, region-and-RC hybrid with a tag-free Wasm-GC-style
slot ABI, an MLIR-inspired single-IR pipeline, and content-addressed everything**.
The compile-time analysis is OCaml-modes-shaped (three-axis modes:
*affinity / uniqueness / locality* — survey 01 §5.5) inferred bidirectionally
with no required user annotations in the common case. The runtime carries:

1. **Tag-free typed slots** — every stack slot's kind is statically known,
   like the JVM verifier and Wasm-GC (survey 02 §1.5, §4.7, §4.1).
   No `ValueWord`. No NaN-boxing. No low-bit tagging at the value level.
2. **Per-binding storage policy** chosen at compile time:
   `let` (unique-immutable) → stack/region/Box; `let mut` (unique-mutable) →
   stack/region/Box with FBIP reuse; `var` (shared-mutable) →
   per-allocation-prefix non-atomic RC + CoW.
3. **Compile-time Perceus + reuse + borrow-inference** elides 80–95% of RC
   ops (Lobster ≈95%, Roc Morphic, Lean 4 — survey 01 §1.4 §1.5 §2.3 §5.3,
   survey 03 §5).
4. **Frozen-immutable cycles handled in-RC** (ISMM'24 — survey 01 §5.7).
   Mutable cycles in `var`-graphs handled by Bacon-Rajan trial deletion as a
   bounded background sweep (survey 01 §1.7). User can opt into *no cycles*
   for hard real-time code via the `@noncyclic` annotation, which the compiler
   verifies and disables the trial-deletion thread.
5. **Compact 8-byte heap header** modeled after JEP 519 (survey 02 §1.1):
   24-bit kind+flags + 8-bit RC-mode + 32-bit refcount, no class pointer
   (Shape resolves type at the slot-kind level, not at the heap level).
6. **Cranelift+ISLE one-IR pipeline.** A Mojo-style MLIR-ish typed IR
   ("Shape MIR") is the single IR consumed by the VM bytecode lowerer, the
   tier-1 baseline JIT, the tier-2 optimizing JIT, and the static
   distribution serializer (survey 02 §5.4, web verification: Cranelift ISLE
   active development 2025). Both the interpreter and the JIT see the
   *same* slot ABI; OSR and deopt are frame-format-compatible (HotSpot
   precedent — survey 02 §4.1, §4.4, §8.1).
7. **Strings**: 16-byte `String` value with 15-byte UTF-8 SSO (Swift / Mojo /
   ecow precedent — survey 03 §1.4, §1.8, §1.6). Heap form is a
   prefix-refcounted `Arc<[u8]>` with CoW. **No interning at runtime**;
   compile-time interning for literals only (Crystal symbols precedent —
   survey 03 §2.3). **No ConsString.** Concatenation eagerly allocates;
   reuse analysis covers the build-then-write idiom (Roc precedent — survey
   03 §1.12). Slicing produces a `Str` view — never an
   indirect-shared-buffer-with-leak risk (Erlang precedent — survey 03 §1.11).
8. **Arrays**: contiguous typed buffers (`TypedArray<T>`) with element-type
   stored in the heap header. Multi-dim arrays carry shape+strides à la
   NumPy (survey 03 §3.2). `SIMD<T, N>` is a first-class type (Mojo / Rust
   `core::simd` — survey 03 §3.4, §4.2). Arrow C Data Interface is the FFI
   contract for zero-copy export (survey 03 §3.3, §6.1).
9. **Polyglot via thin marshal layer**: each foreign runtime gets a bidi
   "value bridge" implemented as a small Rust shim. Python via PyO3 (existing
   pattern, kept), TypeScript via deno_core (existing, kept). The Shape side
   exposes typed slots; the foreign side translates to its own native repr.
   No deep VMValue materialization on hot paths — the marshal layer reads
   directly from the typed slot (Project Panama precedent — survey 02 §5.1).
10. **Content-addressed everything**: function blobs, type schemas, foreign
    function bundles, native dependency hashes, *and the full transitive
    permission set* form the program's manifest hash. Lockfile-equivalent
    reproducibility falls out for free.

### 1.2 What's novel about the combination

Each individual component is shipped somewhere — but no production language
has all of these together. The novelty is geometric, not mechanical:

- **Modes + Perceus + Wasm-GC slot ABI**: OCaml has modes, Koka has Perceus,
  Wasm has typed slots — none of those three has the other two. Modes give us
  the static information; Perceus uses the static information to elide RC;
  the Wasm-GC slot ABI lets the codegen *trust* that information all the way
  down to native machine code without per-value tags.
- **One-IR + tagless slots + Cranelift**: HotSpot has tagless slots and
  unified frames across tiers; Mojo has a one-IR pipeline; neither uses
  Cranelift. Combining all three buys us 10× faster compile times than LLVM
  at HotSpot-quality runtime perf (survey 02 §4.5, §8.1).
- **Frozen-cyclic RC + Bacon-Rajan + no-cycles annotation**: the cycle story
  in RC-first runtimes is the largest unsolved problem (survey 01 §6.2). We
  ship all three escape valves. The user picks per-region.
- **Permissions in the content hash + compile-time scan + 5ns runtime gate**:
  Tier 0 zero-cost permissions enumerate purely from opcode-class (filesystem
  opcode → `FsRead` permission); Tier 1 path-checks at the stdlib boundary;
  permissions baked into hash so a function with the same logic but different
  permissions hashes differently. This makes distribution and signing
  meaningful — the hash *is* the trust statement.

### 1.3 What this design costs

- **Compiler complexity**: modal inference + Perceus + reuse analysis +
  borrow-inference + permission-scan adds ~30k LoC of compiler. Production
  precedent says this is achievable (Lean 4 ≈ 60k LoC of compiler, Mojo's
  ownership pass ≈ 15k LoC).
- **Mode-inference learning curve for advanced users**: in the 95% case,
  inference works without annotations. The remaining 5% requires writing
  explicit modes (`local_`, `unique_`, `once_` à la OCaml). LSP inlay hints
  (a hard requirement) make this discoverable.
- **Cycle-collection background thread**: even with frozen-cyclic + opt-out,
  a small unbounded set of programs (mutable graphs in `var` bindings) needs
  Bacon-Rajan trial deletion. This is the only piece of the runtime that isn't
  fully predictable. The `@noncyclic` annotation is the user's way to disable it.
- **Polyglot marshal layer, while shallow, is still a layer**: cross-language
  calls are not free. We optimize for hot loops staying in one language.

### 1.4 What this design buys

- **Python-feel + Rust-feel + AI-friendly**: the user writes `let x = ...` and
  the compiler infers the entire ownership story. They see clear errors when
  inference fails, with structured fields for LLM consumption. This is the
  "best-in-class ergonomics" requirement.
- **Tagless interpreter + Cranelift JIT + same frame format**: the VM runs
  fast; the JIT runs faster; deopt is straightforward. Bytecode is
  introspectable in VM mode (frame metadata, slot kinds, source positions);
  JIT can drop introspection metadata as an optimization tier.
- **Content-addressed reproducibility**: every function, every dependency,
  every permission, every native lib version → a single SHA-256. Two builds
  on two machines with matching `shape.lock` produce byte-identical hashes
  (Nix-style guarantees without the Nix tax — see §9).
- **Polyglot is cheap at the boundary**: `extern C fn` is one `call`
  instruction (Project Panama precedent). `fn python` is one PyO3 dispatch.
  The marshal layer is shallow because each foreign runtime gets a typed
  bidi bridge, not a generic VMValue conversion.

### 1.5 Risk profile

Medium. Each component has production precedent, but the combination has
never been shipped. The largest risk is **mode inference convergence on real
Shape programs**. Shape supports closures, async, traits, generics, `@ai`
annotations, `extern C`, polyglot — all of which interact with modes. We
mitigate by (a) starting from a known-converging algorithm (OCaml modes —
survey 01 §5.5), (b) staging the rollout (§12), and (c) keeping `clone`
syntactically explicit so users always have an out. The anti-defection
mechanism is: there is no dynamic-fallback path to retreat to. Failure mode is
"compile error with structured fix suggestion" — the same shape as today's
type errors, not a degraded runtime path.

---

## 2. `let` vs `var` at the runtime level

The book pins surface semantics (`book/.../variables.mdx`,
`book/.../references-borrowing.mdx`, `book/.../ownership-deep-dive.mdx`):

- `let` — immutable, uniquely owned, move-by-default, "zero-overhead".
- `let mut` — mutable, uniquely owned, single-owner direct mutation.
- `var` — shared mutable, copy-on-write, refcounted internally.
- `const` — compile-time constant.

**Alternative B preserves these semantics exactly** but tightens the runtime
underneath each one to specific storage policies.

### 2.1 `let` runtime model

`let x = expr` is the most common case and gets the most attention. Storage
policy is **chosen by the compiler** based on the binding's mode-inferred
shape:

| Inferred shape of `x` | Storage | Cost |
|---|---|---|
| `local_` mode (compiler proves doesn't escape function) | Stack slot (or region) | Zero (no allocation) |
| `unique_` mode (no aliases, escapes function) | `Box`-equivalent: prefix-RC=1 heap allocation | One alloc on bind |
| Aliased (shared with another binding/closure) | Promoted to RC heap allocation; `let` gains an inc | One inc on alias creation |

Mode inference happens during the same pass as type inference (OCaml
modes precedent — survey 01 §5.5). For the **Python-easy-entry case**:
`let name = "Alice"` infers `local_, unique_, immutable` and stores on the
stack as a 16-byte Shape `String` (15-byte SSO).

**Escape rules**: `let` is move-by-default. Liveness analysis at MIR level
detects the last use; the value flows to the next consumer without copy. If
inference can't prove unique ownership at a use site (e.g. `let y = x; let z = x`
without `clone`), the compiler emits an error with a `CLONE` repair suggestion
(book precedent — `ownership-deep-dive.mdx` §"Repair Engine").

**Closure capture of `let`**: capture-by-move if last use, capture-by-borrow
otherwise (and the closure inherits the borrow's lifetime constraints).

### 2.2 `let mut` runtime model

`let mut count = 0` is the "single-owner mutation" form. Storage policy:

| Inferred shape | Storage | Cost |
|---|---|---|
| Scalar (`int`, `number`, `bool`) | Stack slot, raw native | Zero |
| Heap value, `local_` mode | Stack-allocated header + buffer (region) | Zero |
| Heap value, escapes | `Box`-equivalent prefix-RC=1 | One alloc |

`let mut` is **never RC-shared** at the type level; the borrow checker enforces
exclusive access, and the runtime is therefore allowed to skip atomics and CoW
checks. Direct in-place mutation: `items.push(4)` on a `let mut items: Array<int>`
is a one-instruction store.

**Reuse via FBIP**: when `let mut` flows through a `match`/reconstruct pattern,
Perceus reuse analysis (survey 01 §1.1, §5.1; survey 03 §5.1) inserts a
reuse token so the destination slot reuses the source allocation. Lean 4 and
Koka show this approach works in production.

### 2.3 `var` runtime model

`var shared = [1, 2, 3]` is the "shared mutable" form. Storage policy:

- Always RC-allocated on heap (prefix RC).
- RC is **non-atomic by default** (single-thread / actor-bound).
- When mode inference detects cross-thread sharing (only via `Mutex`, `Atomic`,
  `Lazy`, `async let`, or detached tasks), the compiler promotes to atomic RC
  for that allocation only — *not* the global runtime mode.
- On mutation, `if rc==1 { mutate in place } else { clone; mutate clone }`
  (Swift CoW precedent — survey 03 §3.7; Roc precedent — survey 03 §1.12).
- Closures capturing `var` capture the RC pointer (capture-by-share).

**Cross-thread `var`**: legal but the compiler must prove the only flow is via
explicit primitives (`Mutex<T>`, `Atomic<T>`, `Lazy<T>`). The book guarantees
this in §"Concurrency Model"; we preserve it.

### 2.4 Why this preserves the book's semantics

- Book says `let` is "zero-overhead" → we lower `local_` `let`s to stack;
  escapes get one prefix-RC allocation, which is the minimum any RC-based
  language pays. The book's reader sees zero-overhead-feel for the Python-like
  default.
- Book says `let mut` is "single-owner direct mutation" → exclusivity check at
  compile time, no atomics, no CoW. Same machine ops as a Rust `let mut`.
- Book says `var` uses "reference counting + CoW" → exactly that, with
  non-atomic RC except where threading proves multi-threaded access.
- Book says move-by-default → liveness drives last-use detection.
- Book says `clone` is explicit → required at any use that would need a copy.

### 2.5 Const

`const PI = 3.14159` evaluates at compile time and is folded into the
constant pool of every blob that uses it. No runtime cost. Same as today.

### 2.6 Closures

Closure capture mode is inferred. The book's table maps to runtime as:

| Captured binding | Closure capture mode | Closure storage |
|---|---|---|
| `let x` (last use in closure) | move | Captured value moved into closure |
| `let x` (further use after closure) | borrow | Captured `&x`; closure inherits lifetime |
| `let mut x` (read only in closure) | borrow shared | `&x`; closure inherits lifetime |
| `let mut x` (write in closure) | borrow exclusive | `&mut x`; closure is exclusive |
| `var x` (any use) | share | `Arc`-clone; CoW on write |

For **detached tasks** (`async let` with detached lifetime), only `move` of
owned and `share` of `var` are legal; borrows are rejected at the type level
(book precedent: §"Three Rules" in `references-borrowing.mdx`).

---

## 3. Ownership model

### 3.1 Type-system axes

Three orthogonal modes, OCaml-shaped (survey 01 §5.5):

- **Affinity**: `affine` (≤1 use) or `many` (unrestricted). Default `many`
  for primitives, `affine` for heap types. Gives us linear-by-default for
  heap values, Copy-by-default for scalars (book §"Copy types").
- **Uniqueness**: `unique` (no aliases live) or `shared`. Drives RC elision
  and CoW skipping. Default inferred from binding form: `let` / `let mut`
  start `unique`; `var` starts `shared`.
- **Locality**: `local` (lifetime ≤ enclosing function frame) or `global`
  (escapes). Drives stack vs heap allocation. Default `local`; promotes to
  `global` if the value flows into a return, struct field, or non-local
  binding.

These three are **orthogonal and inferable**. OCaml's *Oxidizing OCaml* design
demonstrated they compose without explosion (survey 01 §5.5). User-visible
keywords are:

- `clone x` — explicit duplication (always available; required when uniqueness
  inference fails).
- `move x` — explicit transfer (rarely needed; the compiler infers move at
  last use).
- `&x` / `&mut x` — borrow operators, scoped (book pinned syntax).

That's all. No `'a` lifetimes, no `unique_`, no `local_` in user-visible
syntax. Modes are an *implementation* of the inference; users see
`clone`/`move`/`&`/`&mut` and the inferred result.

### 3.2 Inference

Bidirectional. Shape's existing type inference is bidirectional with checked
returns (CLAUDE.md §"Bidirectional closure inference"). Modes ride the same
pass:

```
infer_expr(expr, expected_type, expected_mode) -> (Type, Mode, MirNode)
```

**Per-function whole-body inference** is the default. Cross-function inference
relies on declared parameter modes; we infer parameter modes at function-def
time using the same algorithm OCaml uses (survey 01 §5.5: "designed to be
inferable so programmers rarely write them"). The function's signature exposes
the inferred modes — visible via LSP inlay hints (§10).

**Worst-case complexity**: O(n²) in the size of a single function body
(Hindley-Milner with monotone fixed-point on modes). **Expected complexity**:
O(n) — production OCaml-modes work reports linear-in-practice. Modal
inference does not require whole-program analysis (Roc Morphic does, and
that's a tradeoff we explicitly avoid — Morphic is one of the slowest parts of
the Roc compiler today).

**Annotation requirements**:

- **Never required at function arguments** in monomorphic code. Inferred.
- **Never required for closures**. Inferred from usage (book precedent —
  "Inferred Reference Mode").
- **Required at exported public function signatures** — only the *types*,
  not the modes; modes are derived from the public type signature plus
  the body. Same as Rust without lifetime params.
- **Required when the user wants to assert** something (`@unique`, `@local`
  annotations) — these are checks, not declarations. Documented in §10 as
  diagnostic affordances.

### 3.3 Python-easy-entry walkthrough

```shape
let xs = [1, 2, 3]
let total = xs.sum()
print(total)
```

Mode inference produces:

- `xs`: `Array<int>`, mode `local_, unique_, affine`. Storage: stack-allocated
  TypedArray<i64> header + 3-element buffer. **Zero allocations**.
- `xs.sum()`: takes `&xs` (auto-ref, book precedent). Borrow lifetime
  compatible. No mode promotion.
- `total`: `int`, scalar, raw native i64.
- `print(total)`: scalar argument by value.

Total RC ops: zero. Heap allocations: zero. This is the Python-feel default.
The compiler has done the work of determining this — the user wrote no
annotations.

### 3.4 RC elision rates

Survey 01 §1.4 (Lobster, ≈95%), §1.5 (Lean 4 borrow inference), §5.3
(Lobster), §6.1 (cluster of 80–95% across systems), survey 03 §5 (Roc / Koka /
Lean). Our combination of Perceus + reuse + borrow-inference + modes targets
the **upper end of the published range** (90–95% RC ops eliminated), because
modes contribute orthogonal information that Perceus alone doesn't have
(specifically: locality, which lets us elide allocation, not just RC ops).

We will publish per-benchmark RC-op counts as part of the implementation
acceptance criteria (§12).

### 3.5 Where annotations are required

Three places, none surprising:

1. **Public exported function signatures** — types, not modes (the modes are
   derived). Same effort as Rust public APIs minus lifetimes.
2. **Top-level `var` bindings** — `var` is a *user assertion* of shared
   mutability; the keyword is the annotation.
3. **Where inference fails** — the diagnostic shows the missing annotation
   with a one-click code action (LSP integration §10). Modes inference fails
   on a small explicit list of cases (e.g. mutual recursion through a
   higher-rank generic with closure arguments). User writes `clone` or adds
   `@local` / `@unique` to the binding.

---

## 4. GC strategy

### 4.1 Refcount placement

**Prefix-of-buffer**, 8-byte heap header (modeled after JEP 519 — survey 02
§1.1). One allocation per heap value:

```
[ HeapHeader (8B) | payload ... ]
```

`HeapHeader` layout (one machine word, 64-bit):

```
bits 0..32   : refcount (atomic when needed; non-atomic otherwise — see §4.2)
bits 32..40  : kind (256 total — covers all NativeKind enum values)
bits 40..48  : flags (frozen, atomic-RC, in-cycle-candidate, finalizable, ...)
bits 48..64  : reserved / future use (potentially: hash cache for strings)
```

No class pointer (Shape's slot ABI is statically typed; type is known from
the slot, not from the heap; survey 02 §8.1 §1.5 OCaml precedent). This is
**8 bytes, half of legacy JVM, identical to JEP 519** — survey 02 §7.4.

### 4.2 Atomic vs non-atomic RC

Per-allocation flag (Lean 4 precedent — survey 01 §1.3):

- Default: non-atomic. Set at allocation site. ~5× faster than atomic on
  PACT'18 measurements (survey 01 §1.5).
- Promoted to atomic when the allocation flows into a multi-threading
  primitive (`Mutex<T>`, `Atomic<T>`, `Lazy<T>`, `async let` with detached
  task). Compile-time decision; encoded in the `atomic-RC` flag bit.
- Once atomic, it stays atomic for the lifetime of the allocation.
  (No "rebias" — biased RC is overkill for our case; the static promotion
  decision is precise.)

**Multi-threading story**: structured concurrency (book §"Three Rules") plus
the compiler's mode inference means the "shared across threads" set is small
and statically known. Most allocations stay non-atomic forever.

### 4.3 Cycle handling — three escape valves

**Valve 1: Frozen-immutable RC** (default for `let`-flow data). Survey 01
§5.7. Once a deeply-immutable structure is constructed, it cannot form new
cycles; existing cycles in the *initial* construction are detected at freeze
time via a one-shot DFS. Cycle-detected structures get a special header flag
and are reclaimed via subgraph mark-and-sweep when their root RC hits 0. No
background thread needed. This handles the vast majority of `let` and
`let mut` data.

**Valve 2: Bacon-Rajan trial deletion** (default for `var`-flow data —
survey 01 §1.7, production CPython/PHP precedent). When a `var` allocation
hits a non-zero RC dec (suggesting it might be in a cycle), it joins a
bounded candidate set. A background thread runs trial deletion when the
candidate set fills above a threshold (default: 1024 candidates; tunable).
Pause times: ~1 ms expected (Bacon-Rajan ECOOP'01 measured 6 ms max for
larger candidate sets — survey 01 §1.7).

**Valve 3: `@noncyclic` annotation**. User asserts a region of code or a
type doesn't form cycles. Compiler checks at AST level (no `var` graphs, no
self-referential structs). Trial deletion is disabled at runtime for that
program; valve 1 alone handles cycles. **Hard real-time targets use this.**
Verifier is similar to Koka's `fbip` static check (survey 01 §5.1).

### 4.4 Reuse analysis

**Perceus-style reuse pairing** (survey 01 §1.1, §1.4, §5.1; survey 03 §5).
Implementation:

1. After mode inference, the MIR pass walks each `match` / reconstruct site.
2. For a pattern like `match xs { Cons(h, t) => Cons(h+1, t) }`, the dec on
   `xs` is paired with the alloc for the new Cons.
3. If at runtime the source `xs` had RC=1 at the dec point, the new Cons
   alloc reuses the source's memory directly (no malloc).
4. Frame-Limited Reuse (survey 01 §1.2) ensures the search is bounded —
   constant-factor allocation count, asymptotically optimal.

**Where reuse pays off**: functional-style array transformations (`map`,
`filter`, `fold`), pattern-match-heavy code, immutable-data transforms.
Roc's Morphic shows this is real (survey 03 §1.12). For Shape, this is the
same code path the AI-annotation chain (`@ai`) and `comptime` pipeline use,
and AI-generated code tends to be transformation-heavy — so reuse pays
disproportionately for the AI-native target.

### 4.5 Deallocation timing

- **`let` / `let mut` with `local_` mode**: stack-frame end. Same as Rust.
- **`let` / `let mut` with `unique_, global_` mode**: ASAP at last use
  (Mojo precedent — survey 01 §2.2).
- **`var`**: scope-end, unless RC drops to 0 mid-scope (Perceus-precise).
- **Mutable cycles in `var`**: collected by Bacon-Rajan background thread.

ASAP destruction is preferred over Rust-style scope-end for unique values
because (a) it shortens lifetimes, helping the borrow checker, (b) it's
predictable for AI-generated code (the compiler can show "this is freed
here" inlay hints), and (c) Mojo's production experience shows it composes
fine with finalizers.

### 4.6 Tradeoffs

- **Cost**: cycle-collection thread is a runtime moving part. We mitigate
  with three escape valves.
- **Buy**: 90–95% RC elision; non-atomic-by-default RC; predictable
  deallocation; cycles handled three different ways (user picks); 8-byte
  headers (50% smaller than legacy JVM, matching JEP 519).

---

## 5. Slot encoding & memory layout

### 5.1 Slot bits

**8 bytes per slot**, always. Same as HotSpot (survey 02 §4.1) and Wasm-GC
(survey 02 §4.7). Encoding **per-slot kind** is statically known at the slot's
program point — no per-value tags. This is the canonical answer to the
"forbidden patterns" rule in `CLAUDE.md`: there is no `ValueWord`, no NaN-box,
no low-bit tagging, because there is no dynamic type discrimination at the
value level.

### 5.2 Per-NativeKind encoding

The slot kind is determined by the compiler at every program point.
NativeKind variants and their slot encodings:

| NativeKind | Slot encoding | Heap form? |
|---|---|---|
| `Int64` | raw `i64` | No (always inline) |
| `Float64` | raw `f64` | No |
| `Int32`, `Int16`, `Int8`, `UInt*` | raw, sign/zero-extended to 8 bytes | No |
| `Bool` | raw `i8` (0/1), 7 bytes padding | No |
| `Unit` | zero-byte logical type, slot reserved as i64=0 | No |
| `String` | inline 16-byte `Str` value (split across 2 slots — see §5.4) | Heap form via prefix-RC |
| `Array<T>` | raw pointer to `TypedArray<T>` | Yes (always) |
| `TypedObject{…}` | raw pointer to `TypedObject<S>` (S = schema hash) | Yes |
| `Closure` | raw pointer to closure object | Yes |
| `Option<T>`, `Result<T,E>` | tagged enum representation (see §5.5) | Inline for simple Option<int>; heap for complex |
| `SIMD<T,N>` | raw `T x N` register-friendly | No (stays in vector regs) |

No `HeapValue` enum — that's a parallel discriminator (forbidden by ADR-005,
which §1 of CLAUDE.md cites). Heap-resident kinds are referenced by raw
typed pointers; the kind is in the slot ABI, the heap header carries the
runtime kind only as a debugging/safety check, not as a dispatch source.

### 5.3 Object header

8 bytes (§4.1). Identical for all heap-allocated objects:

```c
struct HeapHeader {
    u32 refcount;
    u8  kind;       // NativeKind enum value
    u8  flags;      // atomic_rc | frozen | finalizable | in_cycle_candidate | ...
    u16 reserved;   // future: hash cache for strings, generation count, etc.
};
```

Total: 8 bytes. Aligned to 8. JEP 519 precedent (survey 02 §1.1, §7.4).

### 5.4 String layout

`String` is a 16-byte value type, occupying **two slots** in the bytecode
(handled as a virtual single slot by the codegen):

```
[ disc:1 | len:7 | data 8 bytes ]   for SSO (≤ 7 inline, plus reuse of disc tag — actually 15)
[ disc:1 | len:7 | ptr 8 bytes ]    for heap form
```

Swift's `_StringObject` design (survey 03 §1.4) allocates the discriminator
nibble in the upper bits of the count word and packs 15 UTF-8 code units
inline. Mojo's String (survey 03 §1.8) does similar. Detail:

- bit 0 of byte 0: 0 = SSO, 1 = heap.
- SSO form: bytes 0..15 are length (in low 4 bits) plus 15 UTF-8 bytes.
- Heap form: bytes 0..7 are length+flags, bytes 8..15 are an `Arc<[u8]>` pointer
  (prefix-RC, payload contiguous after the header).

When the string code unit count exceeds 15, we allocate. Concat eagerly
allocates a new buffer and copies (no ConsString — survey 03 §1.2 has the
arguments for and against, but ConsString's complexity bought us little for
non-JS workloads, and pattern matching / parsing prefer flat strings).

### 5.5 Option / Result inline forms

For `Option<T>` where T is scalar (most common — `Option<int>`,
`Option<bool>`):
- `None` represented as a reserved bit pattern in the slot (e.g. for
  `Option<int>`, use a non-canonical i64 like `INT64_MIN+1`; safe because
  the Shape int type is i48 per `CLAUDE.md`).
- `Some(v)` represented as the raw value.

For `Option<T>` where T is a heap type:
- `None` represented as null pointer (heap pointers are never null otherwise).
- `Some(v)` represented as the raw pointer.

For complex `Option<TypedObject>` or `Result<T, E>`:
- 16-byte two-slot fat representation: `[disc:1 | payload]`.
- On the heap if it escapes; otherwise stack.

This matches Rust's niche optimization and is statically lowered by the
compiler (no runtime tag check).

### 5.6 TypedObject layout

`type Point { x: f64, y: f64 }` lowers to a TypedObject with header + flat
fields:

```
[ HeapHeader (8B) | x: f64 | y: f64 ]
```

Field offsets are computed at compile time (CLAUDE.md §"v2 native types").
`p.x` is a `load f64 [p + 8]` — single instruction. No schema lookup at
runtime. The schema hash in the heap header is for snapshot / wire / debug
purposes only — not a dispatch source.

For TypedObjects that are **`local_, unique_`**, the compiler inlines the
struct into the parent stack frame: zero allocations, zero indirection.
This is **scalar replacement of aggregates** (HotSpot precedent — survey 02
§3.3) done statically, not as a JIT optimization.

### 5.7 Array layout

`Array<T>`:

```
[ HeapHeader (8B) | len:i64 | cap:i64 | element_kind:i64 (debug) | data:T[cap] ]
```

`element_kind` is a debug-only field (kind already in HeapHeader). Total
header: 32 bytes; payload contiguous. `arr[i]` is bounds-check + `load
[ptr + 32 + i*sizeof(T)]`. SIMD-friendly for `Array<f64>`, `Array<i32>`, etc.

Multi-dim arrays carry an additional shape/strides record allocated in the
header reserved bits or as a side-table; default is contiguous 1D, multi-dim
is opt-in (NumPy precedent — survey 03 §3.2).

### 5.8 Tag-free dispatch story

All dispatch in Shape is statically resolved:

- **Method calls**: PHF tables on the receiver's static type (CLAUDE.md
  §"Method Dispatch"). No runtime kind check.
- **Trait dispatch**: monomorphized per trait. Default dispatch is virtual
  via vtable, but only when the receiver is `dyn Trait`; concrete-type
  receivers are direct calls.
- **Pattern matching**: lowered to typed jump tables / direct branches at
  MIR level.
- **`@ai` calls**: type inferred at definition, signature monomorphic.

Survey 02 §8.1: "Tag-free dispatch wins when the type system delivers it.
OCaml, the JVM verifier, Wasm, and now WASM-GC: when you can prove a slot's
type from the program's type system, you don't need per-value tags."

Shape's type system delivers it for all production code. The forbidden-
patterns rule in CLAUDE.md is an explicit project directive in this
direction.

---

## 6. VM / JIT / FFI boundary

### 6.1 Uniform slot ABI across tiers

HotSpot precedent (survey 02 §4.1, §4.4, §8.1): same frame format across
interpreter, baseline JIT, optimizing JIT. Shape adopts this:

- VM bytecode uses an 8-byte typed slot model.
- Tier-1 baseline JIT (Cranelift, no optimization) lowers each opcode to a
  direct Cranelift IR instruction sequence. No marshaling at the VM↔JIT
  boundary.
- Tier-2 optimizing JIT (Cranelift with optimizations) reads the same MIR
  the baseline JIT reads.
- OSR from VM → JIT: just patch the IP. The frame format is identical.
- Deopt from JIT → VM: same. Plus per-deopt-point metadata table tells the
  VM which slots got scalarized.

**Frame layout** (per-function):

```
[ saved FP | saved IP | locals[N] | stack[M] ]
```

Each local slot is 8 bytes, kind known statically per-PC via stack-map
(JVM precedent — survey 02 §4.1 §1.5). The VM reads slots via typed loads
parameterized by the per-PC kind table; the JIT emits typed loads directly
in machine code.

### 6.2 Cranelift codegen

Cranelift+ISLE (survey 02 §4.5, §7.1; web verification: ISLE active 2025).
We emit Shape MIR → Cranelift IR via ISLE rules.

**ISLE rules** are the lowering pattern. Examples:

```
;; Add of two i64 slots
(rule (lower (shape_add_i64 a b)) (cranelift_iadd a b))

;; Load from Array<f64> at index i
(rule (lower (shape_array_load_f64 arr i))
      (cranelift_load
        (cranelift_iadd
          (cranelift_iadd (cranelift_iconst 32) ;; header offset
                          (cranelift_imul i (cranelift_iconst 8)))
          arr)
        f64))
```

Tier-1 baseline = "lower each MIR op to a small ISLE pattern, no
optimization, generate code straight". Tier-2 optimizing = "include
inlining, escape analysis (for SROA), constant folding, range analysis for
bound-check elision, loop predication, IC-driven specialization".

**Promotion thresholds**: Tier 1 at 100 calls (HotSpot threshold —
CLAUDE.md). Tier 2 at 10k. This matches our existing design and JSC's
roughly equivalent "100 statement, 6 calls" baseline (survey 02 §4.3).

### 6.3 Speculative optimization & deopt

**Speculation source**: feedback vectors per call site, V8/JSC-style IC
state machine (Uninitialized → Monomorphic → Polymorphic ≤4 → Megamorphic —
survey 02 §6.1). Tier-2 reads the IC state and specializes.

**Guards**: emitted for any speculative assumption (e.g., "this `Option<T>`
is always `Some`"). Guard failure invokes a deopt routine that:

1. Looks up the deopt-point metadata via per-call-site offset table.
2. Reconstructs slot values from registers (esp. for scalar-replaced
   structs).
3. Materializes a VM frame in-place.
4. Resumes VM dispatch at the deopt PC.

**Deopt counters**: per-call-site, per-guard. After N=10 deopts, the
optimizing tier refuses to re-specialize that site (HotSpot precedent —
survey 02 §6.2). This prevents thrashing.

CVE-2022-1364 (V8 deopt + escape analysis bug — survey 02 §3.3, §6.2) is a
known hazard: scalar-replaced objects need careful materialization. We
mitigate by following Cranelift's PCC (Proof-Carrying Code, survey 03 §4.1)
direction — when feasible, the compiler emits a static proof of materialization
correctness.

### 6.4 `extern C` FFI

Project Panama precedent (survey 02 §5.1, §5.5, §6.2):

- `extern C fn` is one `call` instruction in the JIT path. No transition,
  no GC root scan on entry, no allocation.
- `type C` structs are flat C-ABI structs. `repr(C)` semantics in our slot
  ABI (survey 02 §5.5).
- `cview<T>` and `cmut<T>` (book-pinned syntax) are zero-copy borrow
  carriers; under the hood, they're 8-byte slot pointers. No marshaling.
- `out` parameters generate the cell alloc/read/free stub today (CLAUDE.md
  precedent, kept).

This is identical to what Rust ships and to what Project Panama achieves on
JVM (survey 02 §5.5: "the compiler doesn't need to insert a runtime
boundary"). This is the simplest part of the design.

### 6.5 Polyglot marshal layer

Each foreign runtime gets a typed bidi value bridge. The bridge is the
*only* boundary — no generic VMValue conversion, no `HeapValue` materialization.

**Python (PyO3)**: existing pattern in the codebase, kept. Shape int → Python
int (PyLong_FromLong); Shape Array<T> → numpy.ndarray (zero-copy via Arrow
C Data Interface — survey 03 §3.3, §6.1).

**TypeScript (deno_core)**: existing pattern, kept. Shape value ↔ V8
TypedArray for typed buffers (zero-copy when alignment permits).

**Mojo (future, not yet in book)**: would use MLIR-level interop directly,
via Mojo's MLIR pipeline (survey 02 §5.4). Shape's MIR is MLIR-shaped; in
principle, a Shape function and a Mojo function share a representation. Out
of scope for this ADR but worth flagging.

The marshal layer is **shallow**: each foreign call goes through one
language-specific bridge, with typed slot read on the Shape side and typed
emit on the foreign side. No reflection, no generic decode hop. CLAUDE.md
forbids "boundary translation"; this design has none — the bridges are
typed all the way through.

---

## 7. Strings

### 7.1 Representation

16-byte `String` value (§5.4). 15-byte UTF-8 SSO inline (Swift, Mojo, ecow
precedents — survey 03 §1.4, §1.8, §1.6 §7.1). Heap form is `Arc<[u8]>`
prefix-RC + CoW (survey 03 §1.6, §1.8).

**Decision: SSO threshold = 15 bytes.** Reasoning: survey 03 §8.1 reports
the production cluster at 15-23 bytes. Going to 23 (libc++ style — survey 03
§1.7) would mean a 24-byte `String` value, breaking our two-slot ceiling.
15 bytes covers most identifiers, short messages, common ASCII content;
the upper limit (23) wins ~3-5% extra inline rate at a 50% size cost. Not
worth it.

### 7.2 Heap form

Prefix-RC; payload immediately follows header:

```
[ HeapHeader (8B) | len:i64 | byte_data[len] ]
```

CoW on mutation: if RC=1, mutate in place; else clone-and-mutate. Roc
precedent (survey 03 §1.12): functional-style code with reuse-1 fast path
hits >90% in-place rate.

### 7.3 Interning

**Compile-time only.** All `"literal"` strings in source are interned at
compile time and stored once per blob. Runtime never interns. No runtime
hash table, no global pool.

Reasoning: survey 03 §2.1, §2.2. Runtime interning is a contention point in
multi-threaded programs (LuaJIT scaling issues) and a GC root in JVM
(`StringTable` tuning). Compile-time interning gives us pointer-equality for
literals (free) without runtime overhead. Crystal symbols precedent —
survey 03 §2.3.

### 7.4 Encoding

UTF-8. Survey 03 §8.1: "UTF-8 is winning over UTF-16. Swift 5 switched
(2019); Mojo is UTF-8; Rust always was; Go always was."

Indexing semantics: byte indexing, with `s.chars()` iterator for code points
and `s.graphemes()` for graphemes. Same as Rust. Book §"Strings" pins
`s[i]` returns char; we'll need to clarify this. Index lookup is
O(n) for char-indexing — consider providing both `s.byte(i)` and `s.char(i)`
as methods, with an inlay-hint suggestion at any indexing site.

### 7.5 Concat / slice semantics

**No ConsString.** Concatenation eagerly allocates a new buffer. Survey 03
§1.2 V8 precedent shows ConsString helps build-then-flatten patterns but
adds complexity for 30-50% workloads that touch the result repeatedly.
Combined with our reuse analysis, build-then-flatten patterns hit the
in-place reuse fast path (Roc precedent), which is faster than V8's lazy
flatten.

**Slicing**: produces a `Str` view (16-byte value with same SSO/heap
discrimination, but heap form holds a reference to the parent buffer plus
offset and length). Crucially, slices keep the parent buffer alive — same as
Erlang (survey 03 §1.11 §"binary leak pattern"). To avoid Erlang's leak
pattern, the compiler emits a **slice-copy hint** at any point a slice
escapes a function frame (LSP suggestion: "slice into local, consider
`.to_owned()` to release parent"). This is a *suggestion*, not a hard rule —
parsing/decoder code wants the zero-copy slice.

---

## 8. Arrays / direct memory access

### 8.1 Element-typed buffers

`Array<T>` is generic over T at the type level; the runtime representation
is **per-element-type buffer**:

- `Array<i64>` → contiguous i64 buffer.
- `Array<f64>` → contiguous f64 buffer.
- `Array<bool>` → contiguous u8 buffer.
- `Array<TypedObject>` → array of pointers (current structure) OR array of
  inline structs for `local_, unique_` typed objects (Valhalla-style flat
  layout — survey 02 §5.2).
- `Array<String>` → array of inline 16-byte String values (no extra
  indirection per element).

The `kind` byte in the heap header records the element type at runtime for
debugging only (the slot ABI knows the type from the static program point).

### 8.2 SIMD as first-class type

`SIMD<T, N>` is a first-class type (Mojo, Rust `core::simd`, Julia precedents —
survey 03 §3.4, §4.2). Examples:

- `SIMD<f64, 4>` is a 32-byte register-friendly value.
- Element-wise operations lower to vector instructions via Cranelift
  (survey 03 §4.2 mentions `i32x4`, `f64x2`, etc. as first-class IR types).
- `Array<f64>` provides a method `.as_simd_chunks()` that returns an
  iterator of `SIMD<f64, 4>` chunks plus a remainder slice (Mojo
  `vectorize` precedent — survey 03 §4.2).

This is a hard requirement for the AI-native goal: AI workloads do
matrix/tensor ops, and we need first-class SIMD to compete with NumPy /
JAX / PyTorch on inner loops.

### 8.3 Bound-check elision

Three layers (HotSpot precedent — survey 03 §4.1):

1. **Compile-time range analysis**: loops `for i in 0..arr.len()` strip
   bound checks at compile time. Cranelift PCC (Proof-Carrying Code) carries
   `PointsTo` facts on values; checked loads are statically verified.
2. **Loop predication in Tier-2**: duplicate the loop into a fast version
   (no checks) and a slow version (checks); execute the fast for proven-safe
   iterations, fall to slow at boundary.
3. **Speculative + deopt**: when the index is opaque but profiles suggest
   in-bounds, emit a guard + speculative no-check load. Deopt on guard
   failure.

### 8.4 Strided / multi-dim

Default `Array<T>` is 1D contiguous. Multi-dim is `Tensor<T>` (separate
type, opt-in) with shape + strides (NumPy precedent — survey 03 §3.2). This
keeps the common case fast and the multi-dim case full-featured.

`Tensor<T>` API:
- `tensor.view(slice...)` — zero-copy view, returns Tensor<T> with adjusted
  shape/strides.
- `tensor.transpose()` — O(1), swaps strides.
- `tensor.contiguous()` — materialize a contiguous copy if currently strided.

The transition point from `Array` to `Tensor` is opt-in: `Array` always
contiguous, `Tensor` always strided-aware. Users start with `Array`; HPC
code reaches for `Tensor`.

### 8.5 Arrow C Data Interface compatibility

**Zero-copy export to Arrow**: `Array<T>` and `Tensor<T>` of supported
element types (numeric + string) export via `arrow_export()` returning an
`ArrowArray` C struct (survey 03 §3.3, §6.1). The buffer is borrowed; the
release callback decrements our heap header refcount.

**Zero-copy import from Arrow**: `Array::from_arrow(arrow_array)` wraps an
existing Arrow buffer in a Shape Array with a borrow lifetime tied to the
Arrow array's release callback.

This is the single most important interop decision for AI / data-science
workloads. Pandas/Polars/DuckDB all speak Arrow; we get zero-copy interop
with all of them.

---

## 9. Distribution & dependency model

### 9.1 Content-addressed everything

Building on the existing content-addressed bytecode design (book §
content-addressed-bytecode.mdx). Extension:

- Every **`FunctionBlob`** has a SHA-256 hash over (instructions + constants
  + strings + dependencies + permissions + foreign-deps).
- Every **`TypeSchema`** has a SHA-256 hash over its structural shape.
- Every **`ForeignFunctionBundle`** (Python module, TypeScript module,
  native lib version) has a SHA-256 hash over its serialized contents.
- The **`ModuleManifest`** hash is over all of the above.
- The **`ProgramManifest`** hash is over the entry blob hash + the closure
  of all transitive dependencies.

Two different builds with the same source + same dependency set produce the
same `ProgramManifest` hash. Bit-identical reproducibility is the design
target.

### 9.2 Lockfile shape

`shape.lock` is a TOML file (existing convention) that pins:

```toml
# Auto-generated. Do not edit.
[program]
manifest_hash = "sha256:abcd..."
shape_compiler_version = "0.6.0"

[deps."mathlib::linalg"]
version = "0.3.1"
manifest_hash = "sha256:e4a7..."
source_hash = "sha256:f1b2..."

[native_deps."libduckdb"]
provider = "system"
version = "1.1.3"
linux_x86_64 = "libduckdb.so"
linux_x86_64_hash = "sha256:abcd..."  # hash of the .so itself
macos_aarch64 = "libduckdb.dylib"
macos_aarch64_hash = "sha256:efgh..."

[python_deps]
python_version = "3.11"
venv_hash = "sha256:1234..."  # hash of pip freeze output
[python_deps.numpy]
version = "1.26.0"
wheel_hash = "sha256:..."

[typescript_deps]
node_version = "20.0.0"
[typescript_deps.dependencies]
"@types/node" = { version = "20.0.0", hash = "sha256:..." }
```

### 9.3 Foreign-language dependencies

**Python**: maintain a project-local venv at `.shape/venv-<hash>/`. The lock
file pins:
- Python interpreter version (down to patch).
- Each Python package's version + wheel hash.
- The full `pip freeze` output, hashed.

Reproducibility: re-running `shape build` on a fresh machine produces
byte-identical venv contents (assuming the same wheels are pip-resolvable —
we record the URLs to recover from PyPI churn).

**TypeScript**: equivalent for npm packages. `.shape/node_modules-<hash>/`
populated from a pinned `package-lock.json`-equivalent. Each package's tar
hash recorded.

**Native C**: per-target hashes recorded for each platform binding (book
§native-c-interop.mdx already pins `[native-dependencies]` shape; we extend
to record the binary hash for each target).

### 9.4 Nix integration: optional

We do **not** require Nix. We provide a `shape pack-nix` subcommand that
emits a `flake.nix` capturing the locked dependencies, for users who want
Nix-level guarantees and have a Nix-using workflow. Most users don't.

Reasoning: Nix gives strong reproducibility but at high tooling cost;
content-addressed lockfiles already give us bit-identical reproducibility
for the Shape side, and we can pin foreign deps tightly via the lockfile
without making Nix a hard requirement.

### 9.5 Reproducibility guarantees

**Strong**: given a `shape.lock`, two builds on two machines (with the same
host platform) produce byte-identical `ProgramManifest` hashes.

**Caveats**:
- Native libs differ across platforms (same `manifest_hash` covers different
  per-platform binaries; the lockfile records all platforms).
- Python wheels are platform-specific; the lockfile records platform-specific
  hashes.
- Compiler version is part of the program manifest, so a compiler upgrade
  rebuilds everything.

### 9.6 Distribution units

A function, a module, a program, or a venv are each distributable units
identified by their content hash. The wire protocol (book §wire-protocol)
already pins this for functions; we extend to modules and programs.

`shape publish` uploads the manifest + all referenced blobs to a registry
(Ed25519-signed). `shape pull <hash>` retrieves a program by hash.

---

## 10. Error system & LSP integration

### 10.1 Diagnostic shape

Every compiler diagnostic is emitted in two forms:

1. **Human-readable form** (book precedent in `ownership-deep-dive.mdx`
   §"Error Output Format"): structured pretty-printed message with code
   spans, fix suggestion, and a code diff.

2. **Structured JSON form** (LLM-tuned):

```json
{
  "code": "B0001",
  "severity": "error",
  "title": "Cannot borrow `x` as mutable while shared borrow active",
  "primary": {
    "file": "src/main.shape",
    "span": { "line": 6, "col_start": 9, "col_end": 17 },
    "message": "Mutable borrow conflicts with shared borrow on line 5"
  },
  "secondary": [
    { "file": "src/main.shape", "span": { "line": 5 }, "message": "Shared borrow created here" },
    { "file": "src/main.shape", "span": { "line": 7 }, "message": "Shared borrow last used here" }
  ],
  "expected": { "borrow_kind": "any", "place": "x" },
  "found": { "borrow_kind": "exclusive", "place": "x" },
  "suggested_fixes": [
    {
      "strategy": "REORDER",
      "description": "Move the exclusive borrow after the last use of the shared borrow",
      "diff": "...",
      "verified_by_resolver": true
    }
  ],
  "see_also": [
    { "url": "https://shape-lang.org/book/fundamentals/references-borrowing/#reference-rules" }
  ]
}
```

LLM-actionable fields:

- `code` is a stable identifier (B0001, T0042, M0007 — borrow / type / module).
- `expected` and `found` are *structured*, not free-form text.
- `suggested_fixes[].diff` is a unified diff that an LLM can apply
  programmatically without parsing prose.
- `verified_by_resolver: true` indicates the fix passed re-running the borrow
  checker / type checker — not a heuristic guess (book precedent in §"Repair
  Engine").
- `see_also` points to the canonical book section.

### 10.2 Inference recovery

When an error is found mid-function, the type checker emits the error and
**continues** with a recovery type at the error site (`Type::Error`, a sentinel
that subsumes any constraint). This prevents one error from poisoning the
type environment globally. Same approach as Rust's "type error" propagation.

### 10.3 Inlay hints

Every inferred type, mode, borrow direction, and clone/move decision is
exposed via the LSP as an inlay hint. The MIR-level analysis already
computes all of this (book precedent — `ownership-deep-dive.mdx`
§"Implementation Architecture", "Single Source of Truth"). Hints surface:

- **Inferred type**: `let x = | : int | 42`.
- **Inferred mode**: `let xs = | : Array<int>, local_, unique_ | [1, 2, 3]`
  (advanced users can toggle this on; default off for noise control).
- **Borrow mode at function call**: `read(items)` → inlay shows
  `read(&items)`.
- **Move/clone decision**: `let a = b` → inlay shows "(move)" or
  "(borrow)" based on liveness.

Toggling: keyboard shortcut to cycle hint detail (none / type only / type +
mode / full).

### 10.4 LSP hover

Hover at any binding shows:
- Static type.
- Mode triple.
- Borrow state at the cursor's PC.
- List of all live borrows ("x is borrowed shared by `r` until line 12").
- "Drop here" annotation at the static drop point.

### 10.5 LSP code actions

For every error with a `suggested_fixes` array, the LSP exposes one-click
"apply fix" actions, ordered by the resolver's verification confidence. Same
mechanism as the Rust analyzer's "quick fix" code actions, but with the
rerun-borrow-checker verification step (book precedent).

### 10.6 Best-in-class error message catalog

We ship a versioned catalog of every error code, each with:

- Stable identifier (B0001, etc).
- Human-readable canonical text.
- Structured fields schema (so LLMs can reliably parse).
- Worked examples (good code → bad code → fix).
- Cross-references to book sections.

The catalog is part of the compiler distribution and is queryable from the
LSP / MCP / CLI. The error codes appear in the structured diagnostic, so an
LLM can look up the catalog entry for richer guidance.

---

## 11. Permission system

### 11.1 Two tiers

**Tier 0 (zero-cost, opcode-scan)**: the linker scans every blob's
instruction stream for permission-tagged opcodes. Every opcode has a
known capability class; the linker enumerates the union. No runtime cost.

**Tier 1 (runtime, ~5ns per call)**: stdlib functions that need
fine-grained path/host checks call a `check_permission(ctx, perm)` shim at
their entry point. Existing precedent (book §security-permissions.mdx),
kept.

### 11.2 Opcode → capability mapping

Every Shape MIR opcode falls into one of:

- **Pure**: arithmetic, comparison, control flow, struct ops, array index.
  No permission required.
- **Permissioned (compile-time)**: `OP_FS_OPEN`, `OP_NET_CONNECT`,
  `OP_PROCESS_SPAWN`, `OP_ENV_GET`, `OP_TIME_NOW`, etc. Each opcode is tagged
  with a `Permission` constant at the bytecode-emit layer.

The linker walks every blob, accumulates the permission set:

```
fn linker_permission_scan(blobs: &[FunctionBlob]) -> PermissionSet {
    blobs.iter()
        .flat_map(|b| b.instructions.iter())
        .filter_map(|op| op.required_permission())
        .collect()
}
```

This is purely instruction-class enumeration — no path data, no lifecycle.
Output is a bitset over the 16 (or expandable) permissions.

### 11.3 Tier 1 path-based checks

For permissions like `FsScoped(/tmp/**)`, the runtime check happens at the
stdlib boundary:

```rust
pub fn check_permission_path(ctx: &ModuleContext, perm: Permission, path: &Path) -> Result<()> {
    let granted = ctx.granted_permissions.as_ref().ok_or(...)?;
    let scope = &ctx.scope_constraints;

    if !granted.contains(perm) { return Err(...); }
    if let Some(scope) = scope {
        if !scope.matches_path(path) { return Err(...); }
    }
    Ok(())
}
```

5ns measured cost (book precedent). Glob matching is via a precompiled
`globset` Aho-Corasick automaton; constant-time per check.

### 11.4 Permissions in content hashes

The `FunctionBlob.required_permissions` field is part of the blob's hash
input. A function with the same code but different permissions hashes
differently. This is the existing design (book §content-addressed-bytecode.mdx),
preserved. It enables:

- **Permission-aware caching**: a cached blob bytecode is keyed by hash that
  includes permissions. No accidental run-with-wrong-permissions.
- **Permission auditing**: a third-party reviewer can verify permissions by
  checking the hash against a signed registry.
- **Permission diff**: `shape diff <hash1> <hash2>` shows the permission
  delta between two versions.

### 11.5 Permission boundaries

Permissions apply at three granularities:

- **Function**: each `FunctionBlob` has its own `required_permissions`.
- **Module**: a module's permissions are the union of its functions'.
- **Program**: the program's `total_required_permissions` is the union of
  all transitively reachable function blobs.

The user grants permissions at *load time*: `vm.load_program_with_permissions(prog, granted)`.
The load fails immediately if `granted` is missing any required permission.

### 11.6 Signing

Ed25519 over the `ProgramManifest` hash (book precedent — kept). Each module
bundle can be signed by its publisher; the registry verifies signatures on
upload. Consumers can pin a publisher's public key in their `shape.toml`
and refuse to load unsigned/wrongly-signed modules.

### 11.7 Worked example

```shape
// src/etl.shape
from std::core::io use { read_file, write_file }
from std::core::http use { get }

fn etl(input_path: string, output_path: string, api_url: string) {
    let raw = read_file(input_path)?       // requires FsRead
    let response = get(api_url)?            // requires NetConnect
    let merged = process(raw, response)
    write_file(output_path, merged)?       // requires FsWrite
}
```

Linker output: `required_permissions = {FsRead, FsWrite, NetConnect}`.
Hash: derived from the bytecode + this permission set.

User loads with: `vm.load_program_with_permissions(prog, {FsRead("/data/in/**"), FsWrite("/data/out/**"), NetConnect("api.example.com:443")})`.

Permission denial path:
- If granted set is missing `FsRead`: load fails (Tier 0).
- If granted has `FsRead` but `read_file("/etc/passwd")` is called: runtime
  permission check fails because the path doesn't match the scope (Tier 1).

---

## 12. Migration / implementation strategy

### 12.1 Greenfield rewrite, but in-tree

The supervisor's brief says "the runtime is being redesigned greenfield".
Alternative B is large enough that an *out-of-tree* greenfield rewrite is
probably the wrong shape — too much risk of "two runtimes forever." The
shape is:

- New crates live alongside existing crates (`shape-runtime-v3/`,
  `shape-vm-v3/`, `shape-jit-v3/`, `shape-modes/`, etc.).
- The compiler grows a feature flag `--runtime=v3` that selects v3 lowering.
- v2 (current) and v3 (this design) coexist for a transition period.
- The flag flips to v3 default once v3 passes the existing test suite + the
  new mode-inference benchmark suite.
- v2 is then deleted in one commit; this is the moment of no return.

**Anti-defection**: the v3 design has *no dynamic-fallback path*. There is
no `ValueWord`, no NaN-box, no `SlotKind::Dynamic`. Every type is statically
known at every program point. If something doesn't work, it's a compile
error, not a runtime fallback. This is a hard prerequisite (CLAUDE.md §
"Forbidden Patterns").

### 12.2 Effort estimate

In compiler-engineer-quarters:

| Subsystem | Effort | Notes |
|---|---|---|
| Mode inference (3-axis modal types) | 2 quarters | OCaml-modes precedent; 15-20k LoC. |
| MIR with reuse / borrow-inference | 1.5 quarters | Datafrog already in use; extend with reuse pass. |
| 8-byte heap header + slot ABI | 0.5 quarters | Mostly mechanical; touch every heap-emitting site. |
| Cranelift+ISLE codegen for v3 | 2 quarters | Replace existing `MirToIR` with ISLE rules; tier 1 + tier 2. |
| Cycle collection (frozen + Bacon-Rajan + annotation) | 1 quarter | Bacon-Rajan is well-trodden; frozen-cyclic novel. |
| Strings (SSO + heap form + reuse) | 0.5 quarters | Swift/Mojo precedent. |
| Arrays (TypedArray + Tensor + SIMD + Arrow export) | 1 quarter | Existing TypedArray; extend. |
| FFI + polyglot marshal (thin bridges) | 1 quarter | Existing PyO3/deno_core paths; tighten and optimize. |
| Distribution + lockfile (Python venv + npm + native) | 1.5 quarters | New venv resolver + npm-equivalent + native hash recording. |
| Error system + LSP + structured diagnostics catalog | 1 quarter | Build on existing LSP; add structured JSON output + catalog. |
| Permission system tier 0 opcode scan | 0.25 quarters | Existing tier 0 already; small extension. |
| Test migration + benchmarking + documentation | 1.5 quarters | Move 11.8k tests; add mode-inference benchmark suite. |
| **Total** | **≈ 14 quarters / 3.5 person-years** | At 1 senior compiler engineer FTE; 18 months at 2x parallel. |

For comparison: Mojo's ownership system + MLIR pipeline was multi-team
multi-year (Modular has ~50 engineers per public reports). Our design is
narrower scope but still substantial.

### 12.3 Order of work (which lands first)

Phased, with a stable bisection point at each phase boundary:

**Phase A — Foundations (~3 quarters)**:
1. Mode inference engine (3-axis modes; OCaml-shaped). Standalone, exposed
   via a `--print-modes` debug flag. No codegen change yet.
2. 8-byte heap header migration. Touches every heap-emit site but is
   mechanical.
3. Slot ABI v3 spec doc + per-NativeKind encoding tables, frozen.

**Phase B — Codegen (~3 quarters)**:
4. New MIR with reuse + borrow-inference passes. Outputs are debug-printable.
5. Cranelift+ISLE tier-1 baseline JIT. Function-by-function migration; v2
   path still active.
6. Cranelift+ISLE tier-2 optimizing JIT.
7. Tagless interpreter rewrite. v3 VM bytecode dispatch.

**Phase C — Memory model (~2 quarters)**:
8. Perceus reuse + RC elision (drives the test suite to validate
   mode-inference correctness).
9. Frozen-immutable cycle handling. `@noncyclic` annotation.
10. Bacon-Rajan trial deletion thread.

**Phase D — Surface features (~2 quarters)**:
11. Strings (SSO, heap form, reuse).
12. Arrays + Tensor + SIMD + Arrow export.

**Phase E — Polyglot & FFI (~1.5 quarters)**:
13. `extern C` thin bridge.
14. Python / TypeScript marshal layer optimization (typed slot direct
    read/write).

**Phase F — Distribution & Polish (~2 quarters)**:
15. Lockfile v3 (Python venv + npm + native hashes).
16. Permission system tier 0 opcode scan extension.
17. Error catalog + LSP structured output.
18. Documentation rewrite.

**Phase G — Cutover (~0.5 quarter)**:
19. Default to v3.
20. Delete v2.

### 12.4 Risk areas (highest first)

1. **Mode inference convergence on real Shape programs** — 95% case is fine
   per OCaml precedent, but Shape's combination of closures + traits + async
   + `@ai` annotations + polyglot has no exact precedent. Mitigation: stage
   on mode-inference benchmarks before codegen depends on it; explicit
   `clone` always available as user escape hatch.
2. **Cranelift+ISLE rewrite scope** — 2 quarters is optimistic. Mitigation:
   incremental — add ISLE rules per opcode; v2 fallback per opcode during
   transition.
3. **Cycle collection correctness** — Bacon-Rajan has known production
   precedent (CPython, PHP), but `frozen-immutable RC` is a 2024 ISMM paper
   with no production deployment. Mitigation: ship frozen-immutable as an
   opt-in flag first; default to Bacon-Rajan for `var` graphs.
4. **Polyglot marshal performance** — claim of "~5ns" is from book existing
   measurements; new typed-slot bridges may differ. Mitigation: benchmark
   suite at the boundary, including hot loops in mixed-language code.
5. **Python venv reproducibility on platform diversity** — wheel hashes
   differ across OS/arch/Python-version. Mitigation: per-platform hash
   recording; tooling (`shape ci-verify`) that checks all platforms in CI.
6. **Test suite migration** — 11.8k existing tests assume v2 semantics;
   moving them is mechanical but voluminous. Mitigation: dual-run mode for
   both v2 and v3 paths during phase B; v2 paths deleted only at phase G.

### 12.5 Acceptance criteria for phase boundaries

- **Phase A done**: mode inference produces correct triples on a hand-curated
  benchmark of 200 Shape functions covering closures, async, traits,
  generics. >95% inference success without explicit annotations.
- **Phase B done**: tier-1 baseline JIT compiles 100% of v2-compilable code;
  tier-2 optimizing within 90% of v2 perf on hot benchmarks.
- **Phase C done**: RC op count on functional benchmarks at most 15% of v2's
  count (90%+ elision target).
- **Phase D done**: string and array benchmarks within 5% of Mojo on
  comparable workloads (HPC kernels paper benchmarks — survey 03 §3.4).
- **Phase E done**: Python venv reproducibility verified across linux-x86_64
  + macos-aarch64 + linux-aarch64.
- **Phase F done**: lockfile-driven build is bit-identical between two clean
  machines.
- **Phase G done**: v2 deletion is clean; no test or benchmark depends on
  any v2 symbol.

---

## 13. Tradeoff summary (honest accounting)

What we gain over alternatives A and C (left as exercises since this doc is
B-only — the supervisor will compare):

- **Best ergonomics in class**: full inference of types and modes, with LSP
  inlay hints making the inference visible. Python-feel default.
- **Best perf in class**: tagless slots + Cranelift + Perceus reuse + tiered
  JIT. Mojo-shaped. Survey 02 §8.1 cluster.
- **Strongest reproducibility in class**: content-addressed everything,
  including foreign-lang deps and native libs. Reproducibility without Nix
  tax.
- **Strongest distribution in class**: hashes are trust statements.
  Permissions in hashes. Signed manifests. No surprise capabilities at
  load.

What we pay:

- **Compiler complexity**: ~30k LoC of mode/borrow/reuse analysis. Requires
  staffing a senior compiler engineer for 3-4 quarters.
- **Mode-inference learning curve for the 5% case**: when inference fails,
  the user sees a structured error with a fix; LSP inlay hints make this
  discoverable. Comparable to Rust's borrow-checker learning curve, but
  shallower because we don't have lifetime-parameter syntax.
- **Cycle-collection background thread**: only piece of the runtime that
  isn't fully predictable. Mitigated by `@noncyclic` opt-out and frozen-
  immutable handling for `let`-flow data.
- **Larger cold-start surface**: Cranelift+ISLE compile-time is fast (10×
  LLVM — survey 02 §8.1) but baseline JIT still pays a one-time per-
  function cost on first call.

What we explicitly *don't* do:

- **No global GC** (no tracing collector). Cycle handling is via frozen-
  immutable + Bacon-Rajan + annotation.
- **No NaN-boxing, no low-bit tagging, no `ValueWord`**. Tag-free slots all
  the way down. Forbidden by CLAUDE.md.
- **No dynamic fallback path**. If inference fails, it's a compile error.
  No "soft-fail counter for now, harden later" — see CLAUDE.md "Forbidden
  rationalizations."
- **No global string interning at runtime**. Compile-time only.
- **No ConsString**. Eager concat + reuse analysis.
- **No SoA intrinsic**. Mojo doesn't have one yet (survey 03 §3.1, §7.4)
  and AoS is sufficient for our target workloads. Future opt-in via
  `@layout(soa)` annotation if the data shows demand.

---

## 14. References (URLs)

- OCaml modes (ICFP'24): https://antonlorenzen.de/oxidizing-ocaml-modal-memory-management.pdf
- Mojo origins (current): https://docs.modular.com/mojo/manual/values/lifetimes/
- Mojo ownership (2024-25): https://docs.modular.com/mojo/manual/values/ownership/
- Perceus (PLDI'21): https://www.microsoft.com/en-us/research/wp-content/uploads/2021/06/perceus-pldi21.pdf
- Frame-Limited Reuse (ICFP'22): https://dl.acm.org/doi/10.1145/3547634
- FP² (ICFP'23): https://webspace.science.uu.nl/~swier004/publications/2023-icfp.pdf
- Lean 4 RC: https://arxiv.org/abs/1908.05647
- Roc Morphic / fast page: https://www.roc-lang.org/fast
- Lobster RC elision: https://aardappel.github.io/lobster/memory_management.html
- Vale generational refs: https://verdagon.dev/blog/generational-references
- Bacon-Rajan trial deletion (ECOOP'01): https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf
- Frozen-cyclic RC (ISMM'24): https://dl.acm.org/doi/10.1145/3652024.3665507
- JEP 519 (compact object headers, JDK 25): https://openjdk.org/jeps/519
- WASM-GC overview: https://github.com/WebAssembly/gc/blob/main/proposals/gc/Overview.md
- HotSpot Runtime Overview: https://openjdk.org/groups/hotspot/docs/RuntimeOverview.html
- Cranelift+ISLE (cfallin): https://cfallin.org/blog/2023/01/20/cranelift-isle/
- Cranelift exceptions (2025): https://cfallin.org/blog/2025/11/06/exceptions/
- Mojo MLIR HPC kernels (SC'25): https://arxiv.org/pdf/2509.21039
- Project Panama FFM: https://openjdk.org/jeps/454
- Project Valhalla JEP 401: https://inside.java/2025/10/31/jvmls-jep-401/
- Swift String design: https://github.com/swiftlang/swift/blob/main/stdlib/public/core/StringObject.swift
- Mojo String docs: https://docs.modular.com/mojo/std/collections/string/string/String/
- ecow Rust crate: https://crates.io/crates/ecow
- Erlang binary handling: https://www.erlang.org/doc/system/binaryhandling.html
- Apache Arrow C Data Interface: https://arrow.apache.org/docs/format/CDataInterface.html
- Mojo SIMD: https://docs.modular.com/mojo/stdlib/builtin/simd/SIMD/
- V8 hidden classes: https://v8.dev/docs/hidden-classes

---

## 15. Open questions / followups

1. **Mode-inference algorithm for `@ai` annotations**: `@ai` rewrites the
   function signature at comptime. Does mode inference run before or after
   `@ai` rewriting? Probably before — but needs validation on actual `@ai`
   bodies (which today often return `TypedObject`).
2. **Async + modes**: structured concurrency rules in the book require modes
   to interact with `async let`. Spec needs to make explicit which modes
   can cross task boundaries (probably: `unique_, local_` with move; not
   `unique_, global_`; `shared` always — book §"Three Rules" precedent).
3. **Const-generic interaction with modes**: `Tensor<f64, [N, M]>` with
   const generics — does the mode propagate through the const-generic
   parameter? OCaml-modes handle this; the spec should be precise.
4. **Macro / annotation system × modes**: annotations rewrite AST; the
   rewritten AST must be re-mode-checked. Cost is one extra pass per
   annotation. Probably fine but needs a perf budget.
5. **JIT introspection drop strategy**: VM mode keeps frame metadata,
   slot kinds, source positions. JIT can drop these for hot functions. Spec
   the drop policy: per-function flag? per-deopt-point? Per-tier (drop in
   tier 2 only)?
6. **`SlotKind::Dynamic` is forbidden by CLAUDE.md**. We don't have one in
   this design; ensure all phases of implementation maintain this invariant.
   Mechanical enforcement: the `prove_native_kind() -> Result<NativeKind,
   ProofGap>` discipline (CLAUDE.md §"Mechanical enforcement"), kept.

---

*End of Alternative B draft.*
