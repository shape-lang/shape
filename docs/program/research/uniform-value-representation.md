# Uniform Value Representation for a Strictly-Typed VM + JIT Runtime

**Status**: research report, 2026-07-29. Untracked pending supervisor review.
**Question**: what is the best and most performant value representation for a
strictly-typed language runtime, uniform across the bytecode VM and the
Cranelift JIT — same representation in both tiers, native compatibility, zero
conversion boundaries?

Claims are marked **[MEASURED]** (cited benchmark or measured artifact),
**[SOURCE]** (documented design fact in a primary source), or **[DESIGN]**
(argument from evidence, not itself measured). Shape facts cite repo paths.

---

## Executive summary

**Recommendation: no runtime value tagging anywhere in either execution tier.
Every value is a raw native machine value whose type lives entirely in static
metadata (opcode / signature / schema), with exactly four statically-shaped
encodings for the cases the JIT currently NaN-boxes:**

1. **Null** — per-type niches, never a universal sentinel word: `T?` for heap
   `T` is a nullable pointer (`None` = 0), following Rust's guaranteed
   null-pointer optimization; nullable scalars get per-width encodings defined
   once in `shape-value` (see §Q2 — two of them need owner rulings).
2. **Bool** — `0`/`1` in an integer slot/register (Cranelift has no boolean
   type; `icmp` produces integers).
3. **Unit** — *no value at all*: unit-returning functions are void (zero
   return values, which Cranelift's multi-value signatures express directly).
   `TAG_UNIT` exists only because the JIT forces every call to produce one u64.
4. **Function values** — one carrier: a pointer to a closure record
   (code ptr + captures behind one refcounted header), where **zero-capture
   closures point to a statically-allocated immortal record** — zero
   allocation, one slot, no `fn_id` sentinel. (Ideal-state alternative: a
   two-word fat pair, §Q3.)

The strongest precedents for this are MLton (whole-program monomorphization,
no uniform representation at all) and Rust/.NET value types — and the
strongest *negative* evidence is that the two ecosystems built on uniform
tagged/boxed words (OCaml, JVM) are each spending multi-year flagship projects
(Jane Street's unboxed types / OxCaml layouts; Project Valhalla, a decade to a
JDK 28 preview) buying back exactly the representation Shape can have for free
because kinds are proven at compile time. V8-style hedging (NaN-boxing, Smi
tags) is designed to price dynamic uncertainty; under strict typing that
uncertainty is zero, so every tag is pure deadweight — and Shape's own
2026-07-28 bug family (#219 TAG_NULL leaks, #188 dual function carrier, #189
zero-capture closures as raw UInt64) is the measured cost of keeping a private
tagged dialect in one tier.

**What "uniform across tiers" must mean** (the honest line, from HotSpot /
Pulley / .NET evidence, §Q4): heap object layout and the *bit-level encoding of
every value* must be identical in both tiers; the *location* of values (operand
stack slots vs registers, frame layout) may differ, and relocating bits between
locations at a tier transition is not a conversion boundary — **re-encoding
bits is**. The unified ABI therefore covers: (a) one `HeapHeader` (refcount +
GC color/buffered bits + trace kind) for all heap objects in both tiers —
`UnifiedValue<T>`/`JitAlloc<T>` merge into it; (b) one encoding table
`NativeKind → bit pattern` owned by `shape-value`, included by VM, JIT, and
snapshot; (c) one typed call signature per function used by both interpreter
entry and JIT entry (Cranelift-convention-shaped), with tier adapters allowed
to move values, never re-encode them; (d) snapshot/wire = slots + static kinds
(already ADR-006 §2.7.7 — the VM-side model is validated by this research;
the JIT-side NaN-box residue is the violation to delete).

---

## Q1 — Fully unboxed monomorphized vs uniform-word-with-boxing

### The uniform-word camp exists to serve polymorphism-by-erasure

**OCaml** keeps every value in one machine word — low-bit-tagged immediate or
pointer to a header-carrying block — explicitly so that polymorphic functions
compile once for all types **[SOURCE:
[OCaml docs, Memory Representation of Values](https://ocaml.org/docs/memory-representation),
[Real World OCaml, runtime memory layout](https://dev.realworldocaml.org/runtime-memory-layout.html)]**.
The costs are structural: 63-bit ints, boxed floats (and a special-cased float
array representation), an allocation per float in generic positions.
Jane Street's **unboxed types project (OxCaml)** is a multi-year effort to
escape this: it introduces *layouts* (`value`, `float64`, `bits32`, `bits64`,
`word`, `vec128`, …) as kinds, and polymorphic functions become polymorphic
*within a single layout only* — "for basically any `t`, you cannot write
`float# t`"; abstraction over layout must be resolved at compile time
**[SOURCE: [OxCaml unboxed types intro](https://oxcaml.org/documentation/unboxed-types/01-intro/),
[OxCaml kinds intro](https://oxcaml.org/documentation/kinds/intro/),
[Jane Street tech talk, Unboxed Types for OCaml](https://www.janestreet.com/tech-talks/unboxed-types-for-ocaml/)]**.

**GHC** reached the same conclusion earlier and more formally: *Levity
Polymorphism* (Eisenberg & Peyton Jones, PLDI 2017) treats **kinds as calling
conventions** — a type's kind carries its `RuntimeRep`, and code may only be
representation-polymorphic where the representation is statically resolvable;
otherwise it must be instantiated (specialized) at compile time **[SOURCE:
[Levity Polymorphism, PLDI 2017](https://richarde.dev/papers/2017/levity/levity.pdf),
[ACM DL](https://dl.acm.org/doi/10.1145/3062341.3062357)]**. GHC's unboxed
types carry the matching restrictions (no unboxed values in polymorphic-`value`
positions, no top-level unboxed bindings) **[SOURCE:
[GHC User's Guide §Unboxed types](https://ghc.gitlab.haskell.org/ghc/doc/users_guide/exts/primitives.html)]**.

**Takeaway [DESIGN]**: both mature uniform-word systems converged on "the
representation of every runtime value must be statically known; polymorphism
over representation is a compile-time phenomenon." That is Shape's existing
contract (`docs/runtime-v2-spec.md:38` — "Every value at runtime has a
compile-time-determined type … Generics are monomorphized"). A uniform tagged
word buys nothing Shape needs.

### The unboxed camp: what it costs

**MLton** is the closest whole-system precedent: it defunctorizes,
**monomorphizes every polymorphic datatype and function at every instantiated
type**, defunctionalizes higher-order functions, and then chooses native data
representations per type (unboxed integers/reals, flattened tuples/refs,
per-datatype representation selection) **[SOURCE:
[mlton.org WholeProgramOptimization](http://mlton.org/WholeProgramOptimization),
Weeks, "Whole-Program Compilation in MLton", ML Workshop 2006,
[slides](http://www.mlton.org/References.attachments/060916-mlton.pdf),
[ACM DL](https://dl.acm.org/doi/10.1145/1159876.1159877)]**. The documented
price is whole-program compilation (no separate compilation, longer builds)
and code growth from duplication. Shape already pays that price by design:
content-addressed whole-program linking and monomorphized generics.

**.NET** (Kennedy & Syme, PLDI 2001) is the hybrid precedent: generics are
implemented with **lazy specialization per instantiation, but code is shared
between all reference-type instantiations** (which have identical
pointer-sized representation), while each value-type instantiation gets its
own code with fully unboxed, flattened struct layout; exact runtime types are
preserved **[SOURCE:
[Kennedy & Syme, Design and Implementation of Generics for the .NET CLR](https://www.microsoft.com/en-us/research/publication/design-and-implementation-of-generics-for-the-net-common-language-runtime/)]**.
This is the standard answer to monomorphization code-size worry: monomorphize
over *layout classes*, not over types — all `Ptr(HeapKind)`-kinded
instantiations can share one body. Twenty-five years of .NET struct/`Span<T>`
evolution kept this layout story and built zero-copy APIs on it [DESIGN].

**JVM / Project Valhalla** is the negative-space evidence: a uniform
"everything is an object reference" model retrofitting value flattening. JEP
401 (Value Classes and Objects) reached an early-access build fully
implementing the preview in late 2025 and targets **JDK 28 (March 2027) as a
preview, disabled by default** — roughly a decade of work — because heap
flattening must be reconciled with identity, **nullability (null channels),
and atomicity/tearing** of multi-word values; non-atomic flattening was
explicitly deferred as "too challenging … for a first release" **[SOURCE:
[JEP 401](https://openjdk.org/jeps/401),
[inside.java on the JEP 401 EA build](https://inside.java/2025/10/27/try-jep-401-value-classes/),
[inside.java JVMLS heap-flattening talk](https://inside.java/2025/10/31/jvmls-jep-401/)]**.
Reported EA results show ~3x speedups from flat arrays when flattening
applies **[MEASURED, vendor-reported: inside.java above]**. Lesson for Shape
[DESIGN]: bake flattening in from day one (done — `TypedArray<T>`,
`TypedStruct`); the residual hard problem is not representation but *atomicity
of multi-word values under shared mutation*, which Shape should confine to the
`SharedAtomic`/`SharedAtomicMut` storage classes (ADR-006 §3.2) rather than
solve in the value encoding.

**Swift** shows the other hybrid axis: within a module (or `@frozen`), layout
is static and fully unboxed; across resilient ABI boundaries, layout is opaque
and manipulated through value witness tables, and unspecialized generics take
metadata/witness-table parameters — i.e., dictionary passing — with
specialization as the optimizer's job. Existential (protocol) values live in a
three-word inline buffer + metadata + witness pointers **[SOURCE:
[Swift ABIStabilityManifesto](https://github.com/apple/swift/blob/main/docs/ABIStabilityManifesto.md),
[Swift TypeLayout docs](https://github.com/apple/swift/blob/main/docs/ABI/TypeLayout.rst)]**.
Swift pays for witness tables because it must ship stable dynamic libraries.
Shape is greenfield, whole-program, no ABI-stability constraint (standing
owner ruling: no users, compat zero weight) — so dictionary passing is
unjustified anywhere except trait objects, where a vtable is the point [DESIGN].

**Conclusion for Q1 [DESIGN]**: fully-unboxed monomorphized native layout,
MLton-style, with the .NET pointer-sharing trick available as a code-size
valve. Uniform-word designs are justified only by (a) polymorphism-by-erasure,
(b) separate compilation / stable ABIs, or (c) a uniform GC that must parse
any word — Shape has none of the three (its RC+trace GC is header-driven, not
word-driven; see Q5).

---

## Q2 — Null, Option, and unit

### What the sources guarantee

Rust **guarantees** `Option<T>` is same-size, same-ABI as `T` — with
`None` = all-zero bits — for `&T`, `&mut T`, `Box<T>`, `fn` pointers,
`NonNull<T>`, `NonZero*`, and `#[repr(transparent)]` wrappers of these
**[SOURCE: [std::option docs, representation section](https://doc.rust-lang.org/std/option/)]**.
The general mechanism is niche-filling / discriminant elision: the compiler
stores the discriminant in bit patterns the payload type can never inhabit
**[SOURCE: [rustc niche docs / nonnull_optimization_guaranteed](https://doc.rust-lang.org/nightly/nightly-rustc/rustc_lint/types/fn.nonnull_optimization_guaranteed.html)]**.
Valhalla independently arrived at nullability-as-representation-problem: flat
values that must also encode null need an explicit null channel, and that
channel is one of the two things (with atomicity) that gate heap flattening
**[SOURCE: inside.java JVMLS JEP 401 talk, above]**.

### Mapping to Shape [DESIGN]

Shape's `T?` is statically distinct from `T`, so *the consumer always knows
it is looking at a nullable slot* — no dynamic "is this null-ish?" question
ever arises. That licenses per-type null encodings chosen once, at
compile time, in one place:

| Shape type | Encoding | Cost | Notes |
|---|---|---|---|
| `T?`, `T` heap (incl. `string?`, closures) | null pointer (0) | zero — Rust-guaranteed shape | replaces `TAG_NULL` for all pointer kinds |
| `number?` | NaN sentinel **or** 2-slot `{presence, f64}` | zero vs +1 slot | **needs owner ruling** (below) |
| `i8?/i16?/i32?/u8?/u16?/u32?` | widen into the 8-byte slot; sentinel outside the value range (a true niche, e.g. `1<<32` for `u32?`) | zero — slot is 8 bytes anyway | sound: the niche is uninhabited by the payload type |
| `int?` (`i64?`), `u64?` | 2-slot `{presence: i8, value}` **or** a blessed stolen sentinel (e.g. `i64::MIN`) | +1 slot vs losing one value | **needs owner ruling** (below) |
| `bool?` | byte niche: 0/1 = value, 2 = null | zero | never reuse 0 — Shape already shipped and fixed exactly this collision (`(0, Bool)` as null vs `false`), see the R5b-2 disposition note in `crates/shape-value/src/native_kind.rs` (Bool variant doc) |
| `Option<T>` the enum | same encodings as `T?` after monomorphization | — | `None` for heap payload = null ptr, per Rust guarantee shape |

Shape's spec already sketches this (`docs/runtime-v2-spec.md:155-161` proposes
nullable pointer + NaN sentinel + `{tag, value}` for sized ints), and the VM
already carries the static machinery: `NativeKind` has `Nullable*` variants
per width and `NullableFloat64` documented as NaN-sentinel
(`crates/shape-value/src/native_kind.rs`). What is missing is a **single
normative encoding table** — today the null encodings live implicitly in VM
handlers while the JIT uses `TAG_NULL` (`crates/shape-jit/src/ffi/value_ffi.rs:112`),
which is precisely bug #219's mechanism.

**Unit**: represent by absence. A unit-typed function has zero return values;
a unit-typed expression pushes nothing. Cranelift signatures natively express
zero-return functions (§Q6). `TAG_UNIT` (`value_ffi.rs:121`) exists only
because the JIT ABI forces one u64 out of every call. Shape's `NativeKind::Null`
merged unit/null sentinel remains fine as a *static* kind for
generic-carrier metadata; it must never again be a runtime bit-pattern.

### The two rulings this needs (also in Open Questions)

- **`Some(NaN)`**: a NaN sentinel makes `number?` zero-cost but conflates
  every NaN payload with null (OCaml's `or_null` hits the identical
  one-value-two-meanings wall: `t or_null or_null` is rejected because 0
  would mean two things **[SOURCE: [OxCaml or_null](https://oxcaml.org/documentation/unboxed-types/02-or-null/)]**).
  Options: (a) canonicalize — arithmetic NaN results *become* null (the
  current `NullableFloat64` doc note "NaN + x = NaN, so null propagates
  automatically" already commits to this semantics); or (b) pay 2 slots.
- **Full-range `int?`**: there is no niche in `i64`. Steal a sentinel
  (documented, user-visible: `i64::MIN` is not storable in `int?`) or pay the
  presence slot. Rust pays the extra word (`Option<i64>` is 16 bytes);
  correctness argues for the presence slot [DESIGN].

---

## Q3 — Function values: ending the dual carrier

Today Shape has two carriers for one type: VM-side `Arc<Closure…>` heap values
vs JIT-side `box_function(fn_id)` NaN-box sentinels
(`crates/shape-jit/src/ffi/value_ffi.rs:299`) plus `HK_JIT_FUNCTION`
allocations (`ffi/jit_kinds.rs:29`) — the direct cause of #188, and of #189
where a zero-capture closure crossed the boundary as raw `UInt64`.

What the sources do:

- **Rust**: every closure is its own struct type (captures = fields);
  statically-dispatched closures are monomorphized away entirely; a
  function-typed *value* is either a bare code pointer (`fn`, and zero-capture
  closures coerce to it) or a two-word fat pointer `(data, vtable)` for
  `dyn Fn` **[SOURCE: [Rust Reference, closure types](https://doc.rust-lang.org/reference/types/closure.html);
  std guarantee that `Option<fn>` is pointer-shaped, std::option docs above]**.
- **Swift**: a "thick" function is `{function pointer, context pointer}`; the
  context is a refcounted box passed in a dedicated register, and because that
  register is callee-saved, "partial application [is] free as well as
  converting thin closures to thick" — a thin function just carries a null
  context **[SOURCE: [ABIStabilityManifesto](https://github.com/apple/swift/blob/main/docs/ABIStabilityManifesto.md)]**.
- **OCaml**: a closure is one heap block containing the code pointer(s) and
  environment — one word to carry, one indirection to call **[SOURCE: Real
  World OCaml, runtime memory layout, above]**.
- **MLton**: defunctionalization — closures become datatype values dispatched
  by first-order apply functions, chosen per higher-order call site via
  whole-program control-flow analysis **[SOURCE: mlton.org
  WholeProgramOptimization, above]**.

**Recommendation [DESIGN]** — three statically-selected cases, one value shape:

1. **Callee statically known** (the overwhelmingly common case in a strict
   language): direct call. No function *value* exists at all. This is
   MLton/Rust monomorphization and is already Shape's `CallDirect` path.
2. **Function-typed values** (stored, passed, returned): **one carrier — a
   pointer to a closure record** `{HeapHeader, code_ptr, captures…}` with the
   standard unified header. **Zero-capture closures point to a per-function
   static immortal record emitted at link time**: zero allocation, no RC
   traffic (immortal refcount, GC-exempt flag bit), one 8-byte slot, and the
   same encoding in both tiers. This deletes `box_function`, `HK_JIT_FUNCTION`,
   and the fn_id sentinel family outright.
3. **Trait-object methods**: vtable fat pointer or vtable-in-header — separate
   concern; the point here is only that closures do not ride on it.

The ideal-state alternative is the Rust/Swift **two-word fat pair**
`(code_ptr, env_ptr)` held inline (env null when zero-capture): it saves one
load on every indirect call and makes "call" a plain
`call_indirect code_ptr(env_ptr, args…)`. It costs multi-slot values (a
function value occupies 2 slots), which is a slot-model change (see Open
Questions). The closure-record pointer is the minimal-distance form and is
what OCaml ships; the fat pair is what Rust/Swift ship. Both end the dual
carrier; neither involves a tag.

---

## Q4 — Uniformity across tiers: what must actually be shared

Evidence from systems that run one program in two engines:

- **HotSpot** shares the heap object model completely, but interpreter and
  compiled code use *different calling conventions*, bridged by generated
  i2c/c2i adapters; deoptimization rebuilds interpreter frames from compiled
  frames (one native frame may become several interpreter frames after
  inlining) **[SOURCE:
  [openjdk sharedRuntime.cpp adapter comments](https://github.com/openjdk/jdk/blob/master/src/hotspot/share/runtime/sharedRuntime.cpp),
  [deoptimization.cpp](https://github.com/openjdk/jdk/blob/master/src/hotspot/share/runtime/deoptimization.cpp)]**.
  Crucially the adapters *relocate* argument values between the interpreter's
  expression-stack layout and the compiled register convention — they never
  re-encode a value's bits; an oop is the same oop in both tiers.
- **.NET's new CoreCLR interpreter** (landed for .NET 10-era runtime) is
  built as "a JIT that emits IR bytecode": it shares the runtime type system
  and object layout, enters via ordinary method stubs, and promotes to jitted
  code through tiered compilation with a transition frame for arguments
  **[SOURCE: [dotnet/runtime PR #112202](https://github.com/dotnet/runtime/pull/112202),
  [issue #112748](https://github.com/dotnet/runtime/issues/112748)]**.
- **Wasmtime Pulley** is the strongest precedent for total sharing: the
  interpreter's bytecode is emitted *by Cranelift itself* — same CLIF, same
  mid-end optimizations, a Pulley backend instead of a machine backend — so
  interpreter and native code agree on everything by construction (expected
  ~10x interpretation slowdown vs native) **[SOURCE:
  [Bytecode Alliance, Wasmtime portability article](https://bytecodealliance.org/articles/wasmtime-portability),
  [Pulley README](https://github.com/bytecodealliance/wasmtime/blob/main/pulley/README.md),
  [Wasmtime Pulley docs](https://docs.wasmtime.dev/examples-pulley.html)]**.
- **Graal/Truffle** shares by a different construction: compiled code *is* the
  partial evaluation of the interpreter, so a representation mismatch is
  impossible **[SOURCE: Würthinger et al., "One VM to Rule Them All",
  Onward! 2013, [ACM DL](https://dl.acm.org/doi/10.1145/2509578.2509581)]**.
- **Julia** shares one heap object model (type tag in the word *before* the
  object pointer; `jl_value_t` is opaque) while type-specialized native code
  keeps `isbits` values fully unboxed in registers, boxing **on demand** only
  when a generic `jl_value_t*` is required **[SOURCE:
  [Julia devdocs, Memory layout of Julia Objects](https://docs.julialang.org/en/v1/devdocs/object/)]**.
  Instructive contrast: Julia's "box on demand at dynamic boundaries" is
  exactly the move Shape's strict typing forbids — Shape has no dynamic
  boundary at which a box could be demanded.

**The honest line [DESIGN]**: distinguish three layers.

| Layer | Must be shared? | Evidence |
|---|---|---|
| Heap object layout (headers, field offsets, RC/GC bits) | **Yes, bit-identical** | universal across HotSpot/.NET/Julia/Pulley |
| Value encoding (bit pattern of each type: null repr, bool repr, closure repr) | **Yes, bit-identical** — this is where Shape's tiers diverge today | HotSpot oops, Pulley-by-construction; Shape's #219/#188/#189 are the cost of divergence |
| Value *location* (operand-stack slot vs register, frame shape) | **No** — may differ per tier; adapters may move values | HotSpot i2c/c2i; .NET TransitionFrame |

A tier crossing that *moves* identical bits is not a conversion boundary. A
tier crossing that *re-encodes* bits (today: `unmarshal_jit_result` /
synthesis in `crates/shape-vm/src/executor/control_flow/jit_abi.rs`, NaN-box →
kinded slot) is one, and is the thing to delete. ADR-005 §4 already states
the target ("a slot's bit pattern is interpreted identically in VM and JIT …
No conversion happens at the VM↔JIT boundary, including OSR entries and deopt
exits", `docs/adr/005-typed-slot-construction.md:152-167`); the JIT's private
sentinel dialect (552 tag-family references across 54 files in
`crates/shape-jit/src`, measured by grep 2026-07-29 **[MEASURED]**) is the
outstanding violation, not the spec.

---

## Q5 — RC + cycle-collector interplay with representation

- **Perceus** (Koka; Reinking, Xie, de Moura, Leijen, PLDI 2021,
  distinguished paper) shows precise ownership-based RC insertion makes
  cycle-free programs garbage-free and enables reuse (in-place functional
  update); it is competitive with tracing GCs **[MEASURED: paper benchmarks;
  [PDF](https://xnning.github.io/papers/perceus.pdf),
  [ACM DL](https://dl.acm.org/doi/10.1145/3453483.3454032)]**. Its lesson for
  representation: RC cost scales with the number of *refcounted words that
  move*; unboxed scalars are RC-free, so "RC only on heap pointers, scalars
  raw" minimizes RC traffic by construction [DESIGN].
- **Lobster** reports ~95% of RC operations eliminated by compile-time
  ownership analysis **[MEASURED, author-reported:
  [Memory Management in Lobster](https://aardappel.github.io/lobster/memory_management.html)]** —
  orthogonal to encoding, but it requires statically knowing which slots are
  refcounted, which Shape's per-slot static kinds provide.
- **Nim ORC** = ARC + a trial-deletion cycle collector (Lins lineage), where
  the *compiler's type analysis* decides whether a type can participate in
  cycles; provably-acyclic types (annotatable `{.acyclic.}`) never register as
  cycle candidates and skip collector overhead entirely **[SOURCE:
  [Introducing ORC](https://nim-lang.org/blog/2020/12/08/introducing-orc.html),
  [Nim memory management docs](https://nim-lang.github.io/Nim/mm.html)]**.
  This transfers directly: `TypedArray<f64>`, `string`, `decimal` can never
  close a cycle; Shape's compiler can statically exempt them from
  Bacon-Rajan candidate buffering [DESIGN].
- **Bacon–Rajan** (ECOOP 2001) requires, per candidate object: a color field
  (black/gray/white/purple in the synchronous algorithm) and a buffered flag,
  plus the ability to enumerate an object's outgoing pointers **[SOURCE:
  [Bacon & Rajan, Concurrent Cycle Collection in Reference Counted Systems, ECOOP 2001](https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf)]**.
  Representation constraints on Shape's `HeapHeader`: 3 bits in `flags: u8`
  (2 color + 1 buffered) suffice; and the `kind: u16` must index a
  **trace table** (kind → offsets of pointer fields) since flattened typed
  layouts can't be scanned generically. Both fit the existing 8-byte header
  (`docs/runtime-v2-spec.md:74-83`) without growing it. Monomorphized layouts
  make trace functions compile-time-generated per type — cheaper and more
  precise than uniform-word scanning [DESIGN].
- **Swift ARC** confirms the hybrid at scale: only class instances are
  refcounted; value types carry no RC unless they contain references
  **[SOURCE: [Swift docs, ARC](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/automaticreferencecounting/)]**.

**Conclusion [DESIGN]**: unboxed-scalars + RC-only-on-heap is strictly better
for RC traffic and cycle-collection candidacy than any boxed/uniform scheme,
and Shape's static kinds enable the Nim-style acyclic exemption for free. The
one representation obligation is the trace table keyed by header kind — which
the spec already reserves the `kind` field for ("GC traversal, serialization,
debug — never hot-path dispatch").

---

## Q6 — Cranelift realities

From current Cranelift IR docs (wasmtime main) **[SOURCE:
[cranelift/docs/ir.md](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md)]**:

- Scalar types are `i8/i16/i32/i64/i128` and `f32/f64` (+ SIMD vectors).
  **There is no boolean type** — the old `b1/b8/…` family is gone; `icmp`
  produces integer results consumed by `brif`. A "bool" is an integer with
  values 0/1, which matches Shape's `bool = u8` spec directly. Anything
  NaN-box-shaped must be hand-built from `band/bor/icmp` on i64 — every tag
  check is 2-3 extra ALU ops that the strict compiler already knows the
  answer to.
- `i32`/`i64` are the most exercised widths ("most heavily-tested because of
  their use by Wasmtime"); `i8/i16/i128` may lack full backend coverage —
  favor 8-byte slots with sub-width ops on load/store, which Shape already
  does (zero-extended sub-width slots, `runtime-v2-spec.md:194-203`).
- **Multi-value returns are first-class** in signatures; beyond available
  return registers they lower via a return-area pointer (`sret`), and `sarg`
  covers struct-by-value args. This is what makes "unit = zero returns" and
  "Option-as-two-values / fat function pairs" ABI-expressible without heap
  traffic **[SOURCE: ir.md above;
  [Bytecode Alliance, Multi-Value All The Wasm](https://bytecodealliance.org/articles/multi-value-all-the-wasm)]**.
- Calling conventions: `fast`, `cold`, `system_v`, `windows_fastcall`, plus
  the tail-call-capable convention; `fast` is the not-ABI-stable
  performance convention appropriate for intra-runtime calls.
- Keeping `f64` raw (not NaN-boxed) keeps floats in FP registers end-to-end;
  a NaN-boxed float must round-trip through GPR bitmask ops at every
  produce/consume point [DESIGN].

Nothing in Cranelift favors tagging; everything favors exactly the
`{i64, f64, i32, i8-as-int, ptr}` + multi-value shape Shape's spec already
names (`runtime-v2-spec.md:51-66` maps Shape types to Cranelift types 1:1).

---

## Q7 — Verdict and the unified ABI

**Is any runtime value tagging defensible anywhere?** In the execution tiers:
no. Every candidate consumer of a tag has a static answer: opcodes know their
operand kinds (VM), signatures know theirs (JIT), schemas know field kinds
(heap). The defensible *residue* of "kind next to bits" is metadata at
reflective boundaries — snapshot/wire serialization, REPL printing, polyglot
marshal, comptime interleaving, GC trace — and in every one of those the kind
comes from static tables (`Vec<NativeKind>` tracks, schemas, signatures),
never from probing the bits. That is exactly ADR-006's `KindedSlot`/parallel-
kind-track architecture: this research **validates the VM-side model as
built** and indicts only the JIT's private NaN-box dialect. The heap header's
`kind: u16` survives as GC/serialization metadata (a heap-resident struct
field, not a value tag) — the same distinction `jit_kinds.rs` already argues,
minus the sentinel words.

### The unified ABI, named

1. **Heap headers**: one `HeapHeader` (`refcount: AtomicU32`, `kind: u16`
   trace-table index, `flags: u8` carrying Bacon-Rajan color+buffered +
   immortal/static bit) for every heap object in both tiers.
   `UnifiedValue<T>`/`JitAlloc<T>` (`crates/shape-jit/src/ffi/jit_kinds.rs:67,90`)
   are near-miss parallel implementations of it and merge into it; the
   Tier-2 JIT-private `HK_*` ordinal space (`value_ffi.rs:171-231`)
   dissolves with them.
2. **Value encodings**: one normative table, owned by `shape-value`,
   `NativeKind → bit-level encoding` (including each `Nullable*` null
   encoding, bool 0/1, closure-record pointer). VM handlers, JIT emitters,
   and snapshot all include it; a sentinel constant defined anywhere else is
   a verify-merge violation.
3. **Call convention**: one typed Cranelift-shaped signature per function
   (params/returns in native types; unit = zero returns; multi-value allowed),
   used by JIT-compiled entry and by the interpreter's call setup. Tier
   adapters (interp→jit, jit→interp, OSR, deopt) relocate values between
   frame layouts; they may not re-encode bits (HotSpot i2c/c2i is the
   precedent and the boundary of what's allowed).
4. **Immediates**: null/bool/unit/function-ref per §Q2/§Q3 — statically
   shaped, no sentinel words, `TAG_*` constants deleted.
5. **Snapshot/wire**: parallel `Vec<u64>` + `Vec<NativeKind>` per ADR-006
   §2.7.7 — unchanged; encodings in (2) make JIT-produced and VM-produced
   snapshots bit-identical by construction.

### Minimal-distance migration (from the current NativeKind/KindedSlot model)

The VM tier stays as-is. The work is JIT-side deletion plus encoding
unification, roughly in dependency order:

1. Define the normative encoding table in `shape-value` (mostly transcription
   of what VM handlers already do; the `Nullable*` scalar encodings need the
   two owner rulings below).
2. Unit → zero-return signatures in MirToIR + VM call convention; delete
   `TAG_UNIT`.
3. Null → per-kind encodings at JIT emit sites; delete `TAG_NULL`/`TAG_NONE`
   (directly retires the #219 family).
4. Bool → bare 0/1 i64/i8 values in JIT code; delete `TAG_BOOL_*`.
5. Function values → closure-record pointer with static records for
   zero-capture; delete `box_function`/`HK_JIT_FUNCTION`/`is_inline_function`
   (retires #188/#189 family).
6. Merge `UnifiedValue`/`JitAlloc` into `HeapHeader`; collapse the
   `jit_abi.rs` re-encode path into a pure relocation adapter.
7. Extend `just check-no-dynamic` with the retired sentinel names.

Each step is independently testable against the differential (VM-vs-JIT
bit-equality on returned slots — the test the current representation split
makes impossible to state).

### Ideal-ignoring-distance target

Everything above, plus: (a) **multi-slot inline values** — fat function
pairs, `Option<scalar>` as `{presence, value}`, small tuples/enums inline via
multi-value/`sret` instead of heap cells; (b) **Pulley-style single pipeline**
— the interpreter's bytecode emitted from the same MIR lowering that feeds
Cranelift, making tier divergence structurally impossible rather than
test-enforced; (c) Nim-style static acyclic exemption wired into Bacon-Rajan
candidate buffering; (d) .NET-style code sharing across `Ptr(HeapKind)`
monomorphic instantiations if code size ever measures as a problem.

---

## Open questions needing owner rulings

1. **`Some(NaN)` semantics for `number?`** — NaN-sentinel (zero-cost,
   `Some(NaN)` unrepresentable / NaN-propagates-to-null, as
   `NullableFloat64` currently documents) vs 2-slot presence encoding
   (full fidelity, +1 slot). Q2.
2. **Full-range `int?`/`u64?`** — blessed documented sentinel (`i64::MIN`
   unstorable) vs 2-slot presence encoding. Q2.
3. **Multi-slot inline values** — adopt now (changes the 1-value-=-1-slot
   model; enables fat function pairs and inline Option/tuples) or keep
   single-slot with heap/static-record fallbacks (minimal distance). Q3/Q7.
4. **Interpreter architecture** — keep the hand-written typed-opcode
   interpreter with a shared encoding table, or move toward a Pulley-style
   backend-of-the-same-compiler. The latter is a large architectural change
   with the strongest possible uniformity guarantee. Q4.
5. **Atomicity of multi-word shared values** — confine tearing concerns to
   `SharedAtomic*` storage classes (Valhalla's lesson) and document that
   non-shared multi-slot values are tear-free by construction. Q1.

## Source index

Primary sources: [OCaml memory representation](https://ocaml.org/docs/memory-representation) ·
[Real World OCaml runtime layout](https://dev.realworldocaml.org/runtime-memory-layout.html) ·
[OxCaml unboxed types](https://oxcaml.org/documentation/unboxed-types/01-intro/) ·
[OxCaml or_null](https://oxcaml.org/documentation/unboxed-types/02-or-null/) ·
[OxCaml kinds](https://oxcaml.org/documentation/kinds/intro/) ·
[Levity Polymorphism, PLDI 2017 (PDF)](https://richarde.dev/papers/2017/levity/levity.pdf) ·
[GHC User's Guide, unboxed types](https://ghc.gitlab.haskell.org/ghc/doc/users_guide/exts/primitives.html) ·
[MLton whole-program optimization](http://mlton.org/WholeProgramOptimization) ·
[Weeks, Whole-Program Compilation in MLton (2006)](https://dl.acm.org/doi/10.1145/1159876.1159877) ·
[Kennedy & Syme, .NET generics, PLDI 2001](https://www.microsoft.com/en-us/research/publication/design-and-implementation-of-generics-for-the-net-common-language-runtime/) ·
[JEP 401](https://openjdk.org/jeps/401) ·
[inside.java JEP 401 EA](https://inside.java/2025/10/27/try-jep-401-value-classes/) ·
[inside.java JVMLS heap flattening](https://inside.java/2025/10/31/jvmls-jep-401/) ·
[Swift ABI Stability Manifesto](https://github.com/apple/swift/blob/main/docs/ABIStabilityManifesto.md) ·
[Swift TypeLayout](https://github.com/apple/swift/blob/main/docs/ABI/TypeLayout.rst) ·
[Rust std::option representation guarantees](https://doc.rust-lang.org/std/option/) ·
[Rust Reference, closure types](https://doc.rust-lang.org/reference/types/closure.html) ·
[HotSpot sharedRuntime.cpp](https://github.com/openjdk/jdk/blob/master/src/hotspot/share/runtime/sharedRuntime.cpp) ·
[HotSpot deoptimization.cpp](https://github.com/openjdk/jdk/blob/master/src/hotspot/share/runtime/deoptimization.cpp) ·
[dotnet/runtime interpreter PR #112202](https://github.com/dotnet/runtime/pull/112202) ·
[Wasmtime portability / Pulley](https://bytecodealliance.org/articles/wasmtime-portability) ·
[Pulley README](https://github.com/bytecodealliance/wasmtime/blob/main/pulley/README.md) ·
[Würthinger et al., One VM to Rule Them All, Onward! 2013](https://dl.acm.org/doi/10.1145/2509578.2509581) ·
[Julia devdocs, object layout](https://docs.julialang.org/en/v1/devdocs/object/) ·
[Perceus, PLDI 2021 (PDF)](https://xnning.github.io/papers/perceus.pdf) ·
[Nim ORC announcement](https://nim-lang.org/blog/2020/12/08/introducing-orc.html) ·
[Nim memory management](https://nim-lang.github.io/Nim/mm.html) ·
[Bacon & Rajan, ECOOP 2001 (PDF)](https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf) ·
[Lobster memory management](https://aardappel.github.io/lobster/memory_management.html) ·
[Swift ARC](https://docs.swift.org/swift-book/documentation/the-swift-programming-language/automaticreferencecounting/) ·
[Cranelift ir.md](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md) ·
[Multi-Value All The Wasm](https://bytecodealliance.org/articles/multi-value-all-the-wasm)

Shape facts cited from: `docs/runtime-v2-spec.md`,
`docs/adr/005-typed-slot-construction.md`, `docs/adr/006-value-and-memory-model.md`,
`crates/shape-value/src/native_kind.rs`, `crates/shape-jit/src/ffi/value_ffi.rs`,
`crates/shape-jit/src/ffi/jit_kinds.rs`,
`crates/shape-vm/src/executor/control_flow/jit_abi.rs`,
`crates/shape-vm/src/executor/mod.rs` (parallel kind track).

---

## OWNER RULINGS — 2026-07-29 (Daniel Amesberger, via supervisor)

The five open questions are ruled. These are binding for the representation
program:

1. **`number?` = NaN-sentinel with canonicalization-at-construction**
   *(AMENDED 2026-07-29, user, superseding the same-day collapse-to-null
   ruling)*: one slot; `None` = one reserved sentinel NaN bit pattern;
   at every statically-known `number` → `number?` construction site, a
   computed NaN is canonicalized to a DIFFERENT fixed quiet-NaN pattern
   (`if x != x { x = CANON_NAN }`, one compare + cmov, only at those
   sites). `Some(NaN)` is therefore fully representable; `x == null` is a
   single 64-bit integer compare; reads are free. Documented loss: NaN
   PAYLOAD bits are not preserved through nullable positions (all user
   NaNs become the canonical Some-NaN) — semantically invisible unless
   float bit introspection is ever exposed; the ADR must note it.
   Rationale: variant identity (Some vs None) is runtime information and
   must be bit-distinguishable, but construction sites ARE static in a
   strictly-typed program, so disjointness can be enforced exactly there.
   The same trick does NOT apply to `int?` (no spare bit pattern exists);
   ruling 2 stands unchanged.
2. **`int?` = blessed sentinel.** `i64::MIN` is reserved as null and
   unstorable as a `Some` value (checked at construction); the language
   documents int's range as `[i64::MIN+1, i64::MAX]`.
3. **Multi-slot inline values = ADOPT NOW.** The 1-value-=-1-slot invariant
   is retired across VM slots, JIT ABI, and snapshot format while the formats
   are still free to change (greenfield ruling). Enables fat function pairs,
   inline small tuples/enums. Note: rulings 1–2 still choose single-slot
   sentinels for nullable scalars — multi-slot is for shapes with NO niche,
   not a license to spend slots where a free encoding exists.
4. **Interpreter architecture = Pulley-style single pipeline.** The
   interpreter's bytecode is to be emitted from the same MIR lowering that
   feeds Cranelift, making tier divergence structurally impossible.
   Sequencing (supervisor): the encoding-table + JIT-deletion steps (§minimal
   distance 1–7) land FIRST against the hand-written interpreter — the
   normative encodings must exist and be differential-proven before any
   pipeline emits them — then the Pulley-style rebuild is its own phase.
5. **Multi-word atomicity = confined to `SharedAtomic*`** (supervisor default
   per report recommendation, Valhalla's lesson): non-shared multi-slot
   values are tear-free by construction; document in the ADR.
