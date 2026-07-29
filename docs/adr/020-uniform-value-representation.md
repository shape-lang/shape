# ADR-020: Uniform Value Representation — No Tags, One Encoding, Both Tiers

**Status**: Accepted 2026-07-29 (owner rulings recorded in
`docs/program/research/uniform-value-representation.md` §OWNER RULINGS,
commits `de1e761a` + `329a9a80`).
**Supersedes**: the JIT-internal NaN-box dialect
(`crates/shape-jit/src/ffi/value_ffi.rs` tag family, `unified_box`,
`box_function`) in its entirety. Amends ADR-006 (which this ADR validates on
the VM side and extends across tiers). ADR-018 §2's tiering language is
superseded by §7 here (see #217).
**Evidence base**: the cited research report above; the 2026-07-28
boundary-bug family (#219, #188, #189) as the measured cost of a private
tagged dialect in one tier.

## 1. The rule

**No runtime value tagging exists anywhere, in either execution tier.**
Every value is a raw native machine value. Its type lives exclusively in
static metadata: opcode, signature, schema, `NativeKind` tables. This is
already the VM-tier contract (ADR-006, `docs/runtime-v2-spec.md`); this ADR
extends it to JIT-emitted code, which today maintains a private NaN-box
dialect (~692 tag-family references across 54 files, measured 2026-07-29).

**Uniformity means bit-level encoding identity, not location identity.**
Heap object layout and the bit encoding of every value are identical in both
tiers. The *location* of a value (interpreter operand slot, register, spill
slot, frame) may differ per tier. A tier adapter may **relocate** bits; it
must never **re-encode** them. A re-encoding adapter is the conversion
boundary this ADR abolishes (the `jit_abi.rs` synthesis path is the named
instance to delete).

## 2. The unified ABI (normative surface)

1. **One encoding table.** `shape-value` owns the single normative
   `NativeKind → bit encoding` table. VM handlers, JIT emit sites, and
   snapshot/wire serialization include it; none may define a private
   encoding. The table is data + doc-comments in one module with a
   compile-time exhaustiveness match over `NativeKind`.
2. **One `HeapHeader`.** All heap objects in both tiers carry the ADR-006
   8-byte header (refcount, kind, flags incl. GC color/buffered bits).
   `UnifiedValue<T>` / `JitAlloc<T>` merge into it.
3. **One call signature per function.** Each Shape function has one typed
   signature (Cranelift-convention-shaped) used by interpreter entry and
   native entry alike. Unit-returning functions have **zero return values**.
4. **Snapshot/wire = slots + static kinds** (ADR-006 §2.7.7 unchanged),
   extended for multi-slot values (§5).

## 3. The four encodings that replace the tag dialect

1. **Null** — per-type niches, never a universal sentinel word:
   - Heap `T?`: null pointer (`None` = 0), per Rust's guaranteed
     null-pointer optimization shape.
   - `number?`: **NaN-sentinel with canonicalization-at-construction**
     (ruling 1, amended 2026-07-29): `None` = one reserved sentinel NaN bit
     pattern; at every statically-known `number` → `number?` construction
     site a computed NaN is canonicalized to a distinct fixed quiet-NaN
     pattern (`if x != x { x = CANON_NAN }`, cmp + cmov, only at those
     sites). `Some(NaN)` is representable; `x == null` is one 64-bit
     integer compare. Documented loss: NaN payload bits are not preserved
     through nullable positions.
   - 64-bit integers (`int?`/`i64?`, `u64?`, `usize?`): **2-slot presence
     pair** `{presence: 0|1, payload}` — *(ruling 2 as AMENDED 2026-07-29,
     second amendment)*: no blessed integer sentinel ever; `i64::MIN` and
     the full u64 range remain representable. Rust-confirmed policy
     (`Option<i64>` is 16 bytes; niches only where genuine). The
     presence-pair machinery lands in Phase 1 with this encoding — "no
     work on code that is obsolete later" (owner).
   - Narrower scalars (`i32?`, `i16?`, `i8?`, `u32?`, `u16?`, `u8?`,
     `bool?`, `char?`): widen within the 8-byte slot; the encoding table
     defines one out-of-range `None` pattern per width (`1 << width`) —
     a genuine niche, nothing representable is excluded.
   - The niche/sentinel distinction is normative: an encoding may use
     only patterns that are INVALID for the payload type (NaN payload
     space with canonicalization, out-of-range widened patterns, null
     pointers). Excluding a representable value is forbidden.
2. **Bool** — `0`/`1` in an integer slot/register. No `TAG_BOOL_*`.
3. **Unit** — no value. Unit calls are void (zero-return signatures).
   No `TAG_UNIT`.
4. **Function values** — **one carrier**: pointer to a closure record
   (code ptr + captures behind one refcounted `HeapHeader`). Zero-capture
   closures and named-function references point to **statically-allocated
   immortal records** (refcount ops are no-ops on immortals): zero
   allocation, one slot, no `box_function`, no `fn_id` sentinel, no dual
   carrier. (Fat two-word pairs are the §5 multi-slot upgrade path if
   measurement later justifies them.)

## 4. Nullable-scalar semantics (public, book-gated)

Rulings 1–2 are public language semantics, not internal encodings:
`Some(NaN)` survives nullable positions with a canonical payload; `int`'s
range excludes `i64::MIN`. Both take the standing Book gate — the book pages
for `number`, `int`, and `Option`/`?` types must state them, with runnable
fences, in the same slice that lands each encoding.

## 5. Multi-slot inline values (ruling 3)

The 1-value-=-1-slot invariant is retired. A value may occupy k ≥ 1
consecutive slots with a statically-known k per type: fat function pairs (if
adopted per §3.4), inline small tuples/enums, `sret`/multi-value returns.
The FIRST multi-slot citizens are the §3.1 64-bit presence pairs, which land
in **Phase 1** (#225) together with the minimal 2-slot machinery (kinds
track, call ABI, snapshot for a 2-slot NativeKind); the general k-slot
system remains Phase 3.
Kinds tables, snapshot format, and the call ABI carry per-type slot counts.
Sentinel encodings (rulings 1–2) remain single-slot — multi-slot is for
shapes with **no niche**, not a license to spend slots where a free encoding
exists. **Atomicity** (ruling 5): tearing concerns are confined to
`SharedAtomic*` storage classes; non-shared multi-slot values are tear-free
by construction. The borrow solver / storage planner already serialize
access to non-shared storage; the ADR records this as the invariant that
makes multi-slot safe.

## 6. Forbidden patterns (extends CLAUDE.md §Forbidden)

- Any runtime tag test in either tier (`is_tagged`, `get_tag`, tag-mask
  compares) outside the two sentinel compares defined by rulings 1–2.
- Any universal null/none/unit word (`TAG_NULL`, `TAG_NONE`, `TAG_UNIT`,
  `TAG_BOOL_*` and successors under any rename).
- Any second function-value carrier (`box_function`, `HK_JIT_FUNCTION`,
  fn-id sentinels) beside the closure-record pointer.
- Any tier adapter that re-encodes bits (relocation is the only legal
  adapter operation).
- Any private per-crate encoding of a `NativeKind` (the shape-value table
  is the sole source).
- `just check-no-dynamic` grows the retired names as each deletion lands
  (shrink-only ratchet, same discipline as CHECK 21).

## 7. Interpreter end-state (ruling 4)

The interpreter's end-state is a **Pulley-style single pipeline**: its
bytecode is emitted from the same MIR lowering that feeds Cranelift, making
tier divergence structurally impossible. Sequencing is binding: the encoding
table and the §3 deletions land FIRST against the hand-written interpreter
(encodings must exist and be differential-proven before a pipeline emits
them); the pipeline rebuild is its own later phase with its own design
review. ADR-018 §2's tiered-default language and the #217 rebuild question
are superseded by this end-state.

## 8. Phases and gates

- **Phase 1** (steps in dependency order): encoding table → unit/void →
  null niches (+ canonicalization) → bool → function carrier. Each step is
  gated by the **VM-vs-JIT bit-equality differential on returned slots** —
  newly stateable once encodings are shared — plus the standing per-group
  corpus differential and verify-merge.
- **Phase 2**: HeapHeader merge; `jit_abi.rs` → relocation-only;
  check-no-dynamic ratchet extension.
- **Phase 3**: multi-slot ABI + snapshot/wire format bump.
- **Phase 4**: Pulley-style pipeline (design-first, own program).

Retires on landing: #219 (null step), the #188/#189 carrier residue
(function step), #217 (superseded by §7).
