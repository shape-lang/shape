# Research survey 02 — Memory layout & runtime ABI

Survey of state-of-the-art memory layout, runtime ABI, and VM/JIT/FFI boundary
designs across production runtimes and recent (2024–2026) research, intended as
fact-finding input to a language-runtime redesign. Per-topic facts only — no
recommendations for any specific project.

---

## 1. Object layouts and headers

### 1.1 JVM / HotSpot object headers
**Layout (legacy, 64-bit HotSpot):** 16-byte header. 64-bit mark word (lock /
GC / hashcode bits) + 64-bit class word; with `-XX:+UseCompressedClassPointers`
the class word becomes 32-bit, total 12 bytes + 4 bytes alignment padding,
typically 12 bytes effective.
**Layout (Lilliput / JEP 519, JDK 25, 2025):** 8 bytes total. Class pointer is
compressed to 22 bits and folded into a 64-bit combined "compact header". 22
bits → ~4M loadable classes per process. Locking no longer overwrites the mark
word (would clobber the embedded class pointer); JDK 22 introduced the "Object
Monitor Tables" infrastructure to support that.
**Key innovation:** Subsume the class pointer into the mark word; recover the
locking metadata via an off-line monitor table, freeing 4 bytes per object.
**Performance:** SPECjbb2015 — 22% less heap, 8% faster execution. Amazon
production: up to 30% CPU reduction across hundreds of services. Backported to
JDK 17 / JDK 21.
**Recent evolution:** JEP 450 (experimental, JDK 24, March 2025) → JEP 519
(product, JDK 25, September 2025) → JEP 534 (draft) "Compact Object Headers by
Default".
**References:**
- [JEP 450: Compact Object Headers (Experimental)](https://openjdk.org/jeps/450)
- [JEP 519: Compact Object Headers](https://openjdk.org/jeps/519)
- [Java 25 Integrates Compact Object Headers — InfoQ (2025-06)](https://www.infoq.com/news/2025/06/java-25-compact-object-headers/)
- [Project Lilliput — Beyond Compact Headers (#JVMLS 2024)](https://inside.java/2024/09/06/jvmls-generics-lilliput/)
- [Save 10–20% Memory With Compact Headers — Inside Java Newscast #48](https://nipafx.dev/inside-java-newscast-48/)

### 1.2 .NET / CoreCLR object headers
**Layout:** Two words. (a) **Object header (sync block index)** sits at *negative
offset* −4 (x86) or −8 (x64) from the object pointer. Bits 0–15: thin-lock
state (10-bit thread ID + 6-bit recursion). Upper byte of header may instead be
a "fat-lock" sync block index referencing the SyncBlock table. (b) **Method
table pointer (MT)** at offset 0; 4 bytes (x86) or 8 bytes (x64). Two low bits
of MT are reused during GC for mark and pin flags. Instance fields follow MT.
**Key innovation:** Negative-offset header keeps the hot path (field access)
aligned at offset 0. Sync block is split into thin (inline) and fat
(out-of-line) representations.
**Performance:** Headers: 8 bytes (x86) / 16 bytes (x64) per reference type.
Value types have no header.
**References:**
- [Managed Object Internals, Part 1 (Microsoft Devblogs)](https://devblogs.microsoft.com/premier-developer/managed-object-internals-part-1-layout/)
- [coreclr `syncblk.h`](https://github.com/dotnet/runtime/blob/main/src/coreclr/vm/syncblk.h)
- [coreclr `methodtable.h`](https://github.com/dotnet/coreclr/blob/4895a06c/src/vm/methodtable.h)

### 1.3 V8 hidden classes & maps
**Layout:** Every JS object holds a 32/64-bit pointer to a `Map` (a.k.a. hidden
class / shape). Map encodes property names → in-object slot offsets, attribute
flags, prototype pointer, element kinds, transition pointers.
**Transitions:** Maps form a transition tree. Adding property `x` to a map
`M0` produces `M1`, persistently shared with any other object that follows the
same property-add sequence in the same order.
**Key innovation:** Lift dynamic property access to monomorphic offset access by
caching `(map, offset)` at the call site — inline cache (IC).
**Performance characteristic:** Shape-stable code path is one indirection
(load `obj.map`) + one branch (compare to cached map) + one load (offset). Inline
caches go uninitialized → monomorphic → polymorphic (≤4) → megamorphic.
**Object inlining:** Properties stored in-object up to a per-shape limit; rest
in an out-of-line "properties" backing store.
**References:**
- [V8: Maps (Hidden Classes)](https://v8.dev/docs/hidden-classes)
- [V8: Fast Properties](https://v8.dev/blog/fast-properties)
- [Mathias Bynens — JS engine fundamentals: Shapes and ICs](https://mathiasbynens.be/notes/shapes-ics)

### 1.4 Crystal / Ruby YARV value representation
**Crystal classes:** Heap-allocated. Header is a single `Int32` type ID followed
by fields. Reference-passed.
**Crystal structs:** No type ID, no header. Stack-allocated, value-passed.
A `struct Point(Int32, Int32)` is exactly 8 bytes.
**Ruby (CRuby/YARV) values:** Tagged 1-word `VALUE`. LSB tags:
- `...001` → fixnum (`(int << 1) | 1`)
- `...010` → flonum on 64-bit (compressed 62-bit double)
- distinct constants for `nil`, `true`, `false`, `Qundef`, symbols
- otherwise pointer to `RObject` (heap header + flags + klass + ivars).
**Key innovation:** Uniform `VALUE` slot with low-bit tagging keeps integers,
booleans, common floats, and small symbols off the heap.
**References:**
- [Crystal Internals (2015)](https://crystal-lang.org/2015/03/04/internals/)
- [Crystal `struct.cr`](https://github.com/crystal-lang/crystal/blob/master/src/struct.cr)
- [`ruby/include/ruby/internal/special_consts.h`](https://github.com/ruby/ruby/blob/master/include/ruby/internal/special_consts.h)

### 1.5 OCaml uniform values
**Layout:** Every value is a single machine word.
- LSB = 1 → 63-bit (on 64-bit) / 31-bit (on 32-bit) immediate integer encoded
  as `2*n + 1`.
- LSB = 0 → pointer to a heap block.
**Heap block header:** 1 word. Encodes (a) `wosize` — size of block in words,
(b) `tag` (8 bits) — distinguishes block kinds (0–245 = constructor tag,
246 = closure, 247 = lazy, 248 = object, 249 = infix, 250 = forward, 251 = abstract,
252 = string, 253 = double, 254 = double array, 255 = custom), (c) GC color
bits.
**Key innovation:** A single tag bit suffices because the type system already
constrains what the runtime can encounter; the runtime exists for GC root
identification, not per-operation type dispatch.
**Cost:** Loses 1 bit of integer precision; almost-free dispatch given pointer
alignment guarantees.
**Why it still wins:** Polymorphic code is monomorphic at the runtime level —
the runtime never branches on type to do an arithmetic op; the compiler emits
typed code already.
**References:**
- [Real World OCaml — Memory Representation of Values](https://dev.realworldocaml.org/runtime-memory-layout.html)
- [OCaml docs — Memory Representation of Values](https://ocaml.org/docs/memory-representation)
- [Jane Street — What is gained and lost with 63-bit integers?](https://blog.janestreet.com/what-is-gained-and-lost-with-63-bit-integers/)

### 1.6 CPython PyObject
**Layout:** Every Python object begins with `PyObject_HEAD`:
- `Py_ssize_t ob_refcnt` — 8 bytes (64-bit). Reference count.
- `PyTypeObject *ob_type` — 8 bytes. Type pointer.
Total: 16 bytes minimum. Variable-sized objects add `ob_size` (8 more bytes).
**GC overhead:** Objects participating in cycle GC (containers) are preceded by
a `PyGC_Head` (3 pointers = 24 bytes on 64-bit, recently 16 bytes via union
tricks) before the `PyObject` proper.
**Implication:** Even an empty Python object is ~16 B; an empty `dict` ~64 B; a
small int outside the cached range ~28 B; tuples 56 B + 8 B/slot.
**Cost driver:** Refcount is hot — every access to an object increments and
decrements `ob_refcnt`, which is the primary GIL-removal blocker (no-GIL
"free-threaded" CPython 3.13+ uses biased reference counting + deferred RC).
**References:**
- [Python C API — Common Object Structures](https://docs.python.org/3/c-api/structures.html)
- [CPython Object System Internals](https://blog.codingconfessions.com/p/cpython-object-system-internals-understanding)
- [DeepWiki — CPython Object System and Memory Management](https://deepwiki.com/python/cpython/4-object-system-and-memory-management)

---

## 2. Tagged pointers, NaN-boxing, value packing

### 2.1 NaN-boxing in JS engines (JSC, SpiderMonkey, LuaJIT)
**The trick:** IEEE 754 doubles have only 1 NaN value's worth of "real"
semantics, but the spec admits 2^52 distinct NaN bit patterns. The unused 51
bits are reused to encode pointers, integers, and constants.
**JSC encoding (pure NaN-boxing, double-biased):**
- Doubles are stored with `+ DoubleEncodeOffset = 1<<49` added to the bit pattern,
  pushing the high 16 bits into a non-NaN-prefix range.
- Pointers: high 16 bits = `0x0000`; payload = 48-bit pointer (matches x86-64
  / arm64 canonical addressing).
- Int32: high 16 bits = `0xffff`.
- Special constants: `false=0x06`, `true=0x07`, `null=0x02`, `undefined=0x0a`
  (inhabited as invalid pointers).
**LuaJIT (Mike Pall) encoding:**
- 64-bit value. If high 13 bits are all 1 → it's "tagged" (NaN exponent + sign);
  next 4 bits are itype (lightuserdata/string/table/function/etc.); low 47
  bits = pointer or 32-bit int.
- Otherwise it's a real double.
**SpiderMonkey:** historically NaN-boxed; switched between double-biased and
object-biased depending on platform-specific perf; moved away from pure
NaN-boxing on x64 to "punned" tagged pointer in 2017+.
**Key innovation:** One uniform 64-bit slot for double / int32 / pointer / 6
constants with no extra type word.
**Cost:** Every pointer dereference must mask high bits; doubles must be biased
on store and unbiased on load. Modern x86-64 / arm64 canonical addressing
limits hardware to 48-bit pointers — exactly the headroom NaN-boxing needs.
**Risk on 5-level paging (LA57, 57-bit pointers):** breaks 48-bit assumption;
JS engines either probe the OS or refuse to run.
**References:**
- [Andy Wingo — value representation in JavaScript implementations](https://wingolog.org/archives/2011/05/18/value-representation-in-javascript-implementations)
- [Annie Cherkaev — the secret life of NaN](https://anniecherkaev.com/the-secret-life-of-nan)
- [LuaJIT (luajit.org)](https://luajit.org/luajit.html)
- [Bun blog — How Bun supports V8 APIs without using V8 (Part 1)](https://bun.com/blog/how-bun-supports-v8-apis-without-using-v8-part-1)

### 2.2 Tagged pointers (SBCL, Racket, OCaml)
**SBCL (Steel Bank Common Lisp), 64-bit:**
- Low 3 bits = tag.
- `x00` (low 2 bits = 0) → fixnum (61-bit signed). Fixnum addition is just
  native add — no shift, because tag is preserved.
- `x10` → other-immediate (chars, single-floats on 64-bit, special markers).
- `001` → instance pointer.
- `011` → list (cons) pointer.
- `101` → function pointer.
- `111` → other pointer (vectors, structs, arrays).
**Object-pointer dereference:** `[ptr - tag + offset]`; the constant `-tag`
fold into addressing-mode displacement on x86-64.
**Why low-bit tags:** Pointer alignment guarantees low bits are 0 by
construction; reusing them costs nothing in addressable space if the GC always
allocates aligned.
**References:**
- [SBCL — `early-objdef.lisp`](https://github.com/sbcl/sbcl/blob/master/src/compiler/generic/early-objdef.lisp)
- [SBCL `objects-in-memory.texinfo`](https://github.com/sbcl/sbcl/blob/master/doc/internals/objects-in-memory.texinfo)

### 2.3 JavaScriptCore Structures (JSC's hidden classes)
**Vocabulary:** "Structure" in JSC == "Map" in V8 == "Shape" in SpiderMonkey.
**Layout:** Every cell holds an 8-byte `StructureID` (32-bit since pointer
compression). Structure stores property table, prototype, indexing type
(plain/array/typed-array), transition watchpoint set, transition map.
**IC strategy:** The LLInt and Baseline JIT both embed inline caches keyed on
StructureID. The DFG JIT *scrapes* the IC state to choose speculation level —
i.e. ICs do double duty as type profiles.
**Tier interaction:** ICs are intentionally simple in lower tiers so DFG can
read them quickly and decide what to speculate.
**References:**
- [WebKit — JavaScriptCore (Deep Dive)](https://docs.webkit.org/Deep%20Dive/JSC/JavaScriptCore.html)
- [Speculation in JavaScriptCore (Filip Pizlo)](https://webkit.org/blog/10308/speculation-in-javascriptcore/)
- [Filip Pizlo — DLS 2017 / VMIL 2017 slides on the JSC VM](http://www.filpizlo.com/slides/pizlo-dls2017-vmil2017-jscvm-slides.pdf)

### 2.4 Per-slot kind metadata vs per-value tags (JVM verifier-driven typing)
**Premise:** If types are statically known at every program point, you can
eliminate per-value type tags entirely; the *slot* knows its kind.
**JVM realization:** The class-file verifier proves that every JVM stack slot /
local has a fixed kind (`int`, `float`, `long`, `double`, `reference`) at every
PC. The interpreter therefore has no runtime tag check on `iadd` etc. —
opcodes are typed.
**Trade-off vs OCaml:** OCaml goes further: it tags only at the GC boundary
(scan stack, find pointers); JVM tags at the verifier boundary (scan stack-map
table). Both eliminate per-op tag checks; both still need per-frame metadata
for GC.
**HotSpot interpreter:** "Tagless" — opcodes are typed, locals/stack are raw
machine words, and a separate stack-map table (since classfile v6+ /
StackMapFrame) tells the GC which slots are oops at each safepoint.
**References:**
- [HotSpot Runtime Overview](https://openjdk.org/groups/hotspot/docs/RuntimeOverview.html)
- [HotSpot Glossary of Terms](https://openjdk.org/groups/hotspot/docs/HotSpotGlossary.html)
- [Cliff Click — C2: The JIT in HotSpot (slides)](https://assets.ctfassets.net/oxjq45e8ilak/12JQgkvXnnXcPoAGoxB6le/5481932e755600401d607e20345d81d4/100752_1543361625_Cliff_Click_The_Sea_of_Nodes_and_the_HotSpot_JIT.pdf)

---

## 3. Heap layout / data structure shapes

### 3.1 AoS vs SoA
**AoS:** `[{x,y,z}, {x,y,z}, …]`. Default for OO languages. Spatial locality
when you touch many fields of one element.
**SoA:** `{xs[], ys[], zs[]}`. Better cache + vector behaviour when you
touch one field across many elements (the SIMD-friendly case).
**Mojo:** No first-class SoA intrinsic as of late 2025; it's a tracked feature
request (modular/mojo issue #3790). Manual SoA via parameterized structs is
common in Mojo HPC kernels.
**Julia (StructArrays.jl):** `StructArray` is an `AbstractArray` whose iteration
yields struct values but whose storage is per-field arrays. Entries are
constructed on the fly. Convert from AoS copies; convert from SoA can wrap.
**Other modern attempts:** soagen (C++), dataarrays (Rust), Unity DOTS / Burst,
and Apple's accelerate.
**References:**
- [AoS and SoA — Wikipedia](https://en.wikipedia.org/wiki/AoS_and_SoA)
- [Mojo issue #3790 — Built-in SoA support](https://github.com/modularml/mojo/issues/3790)
- [StructArrays.jl docs](https://juliaarrays.github.io/StructArrays.jl/stable/)
- [Intel — Memory Layout Transformations](https://www.intel.com/content/www/us/en/developer/articles/technical/memory-layout-transformations.html)

### 3.2 Cache-line packing
**Cache line size:** 64 bytes on most x86-64 / arm64; 128 B on Apple Silicon
(M1+) — a frequent perf footgun. Power 9 has 128 B; some IBM Z 256 B.
**Zig:**
- `extern struct` — C ABI ordering, no reordering, may pad.
- `packed struct` — explicit bit-level layout (good for hardware registers, bit
  flags); not for cache control.
- For cache-line padding, idiom is `extern struct { value: T, _pad: [N]u8 }`.
**Rust:**
- `repr(C)` — C-compatible, may pad.
- `repr(packed)` — strip padding to 1-byte alignment; taking a reference to a
  field is UB if not aligned; can incur unaligned load penalty (or fault on
  some ARM).
- `repr(C, packed)` for both.
- `repr(align(N))` for padding/alignment.
**False sharing:** independent threads writing different fields that share a
cache line invalidates each other's L1 line. `crossbeam_utils::CachePadded`,
`std::hint::spin_loop`, etc.
**References:**
- [Rustonomicon — Other reprs](https://doc.rust-lang.org/nomicon/other-reprs.html)
- [Zig guide — Packed Structs](https://zig.guide/working-with-c/packed-structs/)
- [Hexops — Packed structs in Zig](https://devlog.hexops.org/2022/packed-structs-in-zig/)
- [Cache Me If You Can (false sharing)](https://cryptocode.github.io/blog/docs/falsesharing/)

### 3.3 Object inlining (escape analysis)
**Idea:** If a compiler proves a freshly allocated object never escapes a
function, it can eliminate the heap allocation: split the object into scalar
fields kept in registers/stack ("scalar replacement of aggregates"), or
replicate it inline in the parent.
**V8 (TurboFan):** Performs escape analysis; if the object's address doesn't
flow into an unknown sink (stored in the heap, returned, passed to non-inlined
callee, etc.), the object is dematerialized into SSA values. On deopt the
material object is reconstructed using deopt metadata.
**HotSpot C2:** Same approach. EA enables stack allocation, lock elision, and
scalar replacement.
**LuaJIT:** Allocation sinking — places allocations after the trace, only
materializing when the trace exits (and the object would escape).
**Stable shape:** V8 specifically tracks whether a constructor produces objects
of stable shape; ICs are healthier and EA is more profitable when shapes
don't churn.
**Counter-cases:** Deopt with materialized objects has historically been a CVE
vector (CVE-2022-1364 — inconsistent object materialization in V8).
**References:**
- [Tobias Tebbi — Escape Analysis in V8 (JFokus 2018)](https://www.jfokus.se/jfokus18/preso/Escape-Analysis-in-V8.pdf)
- [V8 — Temporarily disabling escape analysis (post-mortem)](https://v8.dev/blog/disabling-escape-analysis)
- [kipply — Escape Analysis in PyPy, LuaJIT, V8, C++, Go](https://kipp.ly/escape-analysis/)

---

## 4. Slot ABI between interpreter and JIT

### 4.1 HotSpot tagless interpreter
**Layout invariant:** Stack slots are 32-bit (long/double consume two adjacent
slots, big-endian-style on 32-bit, but logically one slot). Slots are kind-typed
by the verifier — no runtime tag.
**GC interaction:** Stack-map tables, attached to each method, encode for every
PC at which a GC can occur which slots are oops. The interpreter is a single
template-generated assembly routine per opcode; safepoint insertion points
match stack-map entries.
**Why this matters for Shape-style design:** No per-value type word and no
per-op tag check; the *position* of a value in the frame implies its kind.
**References:**
- [HotSpot Runtime Overview](https://openjdk.org/groups/hotspot/docs/RuntimeOverview.html)

### 4.2 V8 Sea of Nodes / Turbofan / Maglev / Turboshaft
**Sea of Nodes (SoN):** Nodes are operations; edges are data, control, and
effect dependencies. Free-floating non-dependent ops admit aggressive
reordering.
**Adoption story:** V8 used SoN in TurboFan since ~2014. *V8 has now
deprecated SoN* (blog post: "Land ahoy: leaving the Sea of Nodes",
2024) and the new mid-tier "Turboshaft" uses a CFG-based IR.
Both JS and Wasm backends in V8 have largely migrated to Turboshaft.
**Reasons to leave SoN:** harder to reason about; harder for engineers to debug;
empirically not worth it for the JS workload mix.
**Maglev (V8's mid-tier, between Sparkplug and Turbofan):** Splits each frame
into a *tagged* and an *untagged* region; only the split point is recorded —
not per-slot type metadata. Compromise between full type tracking (TurboFan)
and pure tagged uniformity (lower tiers).
**Pointer compression:** 32-bit slots in heap on 64-bit V8; LSB is tag (1 →
SMI / Small Integer; 0 → pointer). Doubles in `PACKED_DOUBLE_ELEMENTS` arrays
are 64-bit raw IEEE 754 with no tag.
**References:**
- [V8 — Maglev: V8's Fastest Optimizing JIT](https://v8.dev/blog/maglev)
- [V8 — Land ahoy: leaving the Sea of Nodes (2024)](https://v8.dev/blog/leaving-the-sea-of-nodes)
- [V8 — Digging into the TurboFan JIT](https://v8.dev/blog/turbofan-jit)

### 4.3 JSC's DFG and FTL tiers
**Tier ladder (4 levels):**
1. **LLInt** — Low Level Interpreter, written in offlineasm DSL, generates
   native asm at JSC build time. Includes inline caches.
2. **Baseline JIT** — template-generated machine code, also with ICs.
3. **DFG JIT** — Data Flow Graph; speculative type optimizations, IC-driven.
4. **FTL JIT** — Faster Than Light; B3 backend (replaced LLVM in 2016).
**Tier promotion:** ~100 statement executions or 6 calls → Baseline.
~1000 statements or 66 calls in Baseline → DFG. DFG → FTL is profile-driven.
**OSR exit:** Speculation guards in DFG/FTL fail → execution descends back to
Baseline (or LLInt). Implemented via `Check`-family opcodes that preserve enough
state to reconstruct the unoptimized stack.
**Same frame format across tiers:** All tiers use the same JS-VM stack frame so
OSR can swap tiers in place without conversion.
**B3 (Bare Bones Backend):** JSC's in-house low-level optimizer, replaced LLVM
to cut compile time. Used by FTL.
**References:**
- [WebKit — Introducing the FTL JIT](https://webkit.org/blog/3362/introducing-the-webkit-ftl-jit/)
- [WebKit — Introducing B3](https://webkit.org/blog/5852/introducing-the-b3-jit-compiler/)
- [Speculation in JavaScriptCore](https://webkit.org/blog/10308/speculation-in-javascriptcore/)

### 4.4 JVM interpreter ↔ C1 ↔ C2 transitions
**Tier ladder (5 levels in tiered compilation):**
- Tier 0: Interpreter.
- Tier 1: C1 with no profiling (shouldn't profile twice if going to Tier 4).
- Tier 2: C1 with method-entry-and-loop counters.
- Tier 3: C1 with full profiling.
- Tier 4: C2 with profile-driven optimizations.
**Standard path:** 0 → 3 → 4. Tier 1 / 2 used for short-lived methods that
won't reach C2.
**Frame layout:** Interpreter and compiled-method frames *differ*. The
interpreter has a fixed slot for `bcp` (bytecode pointer), `mdp` (method data
pointer), locals base, expression stack base, etc. Compiled frames are
register-rich and tightly packed.
**Switching between tiers:** Done via OSR, not in-place rewrite. The compiler
emits an OSR entry point at loop heads; deopt rebuilds an interpreter frame
from compiled-frame state via `RegisterMap` + scope desc metadata.
**Recent improvements:** JFR (Flight Recorder) sampling for low-overhead
runtime profiling; Project Loom virtual threads use small stacks (continuation
stacks) that re-park onto a carrier OS thread, requiring continuation-aware
safepoint and OSR machinery.
**References:**
- [Microsoft — How Tiered Compilation works in OpenJDK](https://devblogs.microsoft.com/java/how-tiered-compilation-works-in-openjdk/)
- [Baeldung — Tiered Compilation in JVM](https://www.baeldung.com/jvm-tiered-compilation)
- [HotSpot Glossary](https://openjdk.org/groups/hotspot/docs/HotSpotGlossary.html)

### 4.5 Cranelift JIT
**What it is:** Code generator written in Rust. Used by Wasmtime (production),
rustc_codegen_cranelift (dev-only fast Rust backend), shape-jit (this project).
*Not* used by SpiderMonkey, despite early discussions; Mozilla decided against it.
**ISLE (Instruction Selector / Lowering Engine):**
DSL based on term rewriting; `(rule (lower (iadd a b)) (x64_add a b))`-style
patterns generate Rust dispatch trees that handle multi-priority overlapping
patterns and merge into a decision tree at meta-compile time.
**Calling-convention support:** Cranelift supports `SystemV`, `WindowsFastcall`,
`Wasmtime*`, `Probestack`, `AppleAarch64`, `Tail`. Per-`Signature` calling-conv
selection.
**Pulley (2025):** Cranelift backend that emits *bytecode* for a portable
interpreter (`pulley-interpreter`), trading native-code execution for
~10× slower portable execution on platforms where Cranelift has no native
backend. Intended for cloud sandboxing and constrained environments.
**Exceptions (2025):** Cranelift gained exception support for Wasm exception
handling.
**References:**
- [Cranelift project (cranelift.dev)](https://cranelift.dev/)
- [Chris Fallin — Cranelift's ISLE: Term-Rewriting Made Practical (2023)](https://cfallin.org/blog/2023/01/20/cranelift-isle/)
- [Wasmtime/Cranelift README](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md)
- [Pulley — Wasmtime's Portable Optimizing Interpreter](https://docs.wasmtime.dev/examples-pulley.html)
- [Cranelift Exceptions (2025)](https://cfallin.org/blog/2025/11/06/exceptions/)

### 4.6 WebAssembly memory model
**Linear memory:** Single contiguous byte array, addressable starting at 0,
growable in 64 KiB pages, never shrinks. Loads/stores can be unaligned; bounds
checking via virtual-memory tricks (guard pages).
**Multi-memory (2024 draft):** Modules can declare multiple linear memories.
**Locals:** Typed (`i32`, `i64`, `f32`, `f64`, `v128`, ref types). The slot
ABI is statically typed — no per-value tags, no run-time check.
**Call stack:** Not directly accessible to Wasm code (control flow is structured;
`call`, `call_indirect`, `return`).
**No GC heap (until WASM-GC):** All structured data lives in the linear memory.
**References:**
- [WebAssembly Core Specification](https://webassembly.github.io/spec/core/bikeshed/)
- [radu-matei.com — Practical guide to WebAssembly memory](https://radu-matei.com/blog/practical-guide-to-wasm-memory/)
- [RichWasm (POPL 2024)](http://www.ccs.neu.edu/home/amal/papers/richwasm.pdf)

### 4.7 WASM-GC (2024+)
**Status:** WebAssembly/gc repo archived 2025-04 — proposal merged, considered
delivered. Shipping in Chrome (V8) by default since 2023; Firefox (SpiderMonkey)
and Safari (JSC) have shipped or are shipping.
**New types:**
- `structref` / `arrayref` — heap-allocated typed records and homogeneous arrays.
- `funcref` (already existed) — function pointers.
- `i31ref` — *unboxed* 31-bit signed integer injected into the reference
  hierarchy. Implementation note: 31 bits because that's the largest range
  guaranteed efficient on all platforms.
- `anyref`, `eqref`, `noneref` etc. form the GC ref hierarchy.
**Operations:** `struct.new`, `struct.get`, `struct.set`, `array.new`,
`ref.cast`, `ref.test`, `ref.eq`, `br_on_cast`, etc.
**Subtyping:** Nominal subtyping among declared GC types; `rec` blocks for
mutually recursive types.
**Use cases:** Kotlin/Wasm, Dart→Wasm (Flutter), Java→Wasm (`teavm`,
CheerpJ-style), Scheme→Wasm (Whiffle, Hoot).
**Performance:** Designed so engines can lay out structs flat (one indirection)
and emit one-load field access; comparable to JS hidden-class fast path but
*statically known*.
**References:**
- [WebAssembly/gc Overview](https://github.com/WebAssembly/gc/blob/main/proposals/gc/Overview.md)
- [V8 — A new way to bring GC languages to Wasm](https://v8.dev/blog/wasm-gc-porting)
- [Chrome blog — WasmGC enabled by default](https://developer.chrome.com/blog/wasmgc)
- [Wasmtime `StructRef` API](https://docs.wasmtime.dev/api/wasmtime/struct.StructRef.html)

---

## 5. FFI / language-boundary cost

### 5.1 Project Panama (JVM)
**Components:**
- **Foreign Function & Memory API (FFM)** — finalized in JDK 22 (`java.lang.foreign`).
  `MemorySegment`, `MemoryLayout`, `Arena`, `Linker`, `MethodHandle`-based
  function descriptor.
- **Vector API** — incubator; SIMD-portable abstract.
- **jextract** — header-derives Java bindings.
**Layout discipline:** `MemoryLayout.structLayout(JAVA_INT, paddingLayout(4),
ADDRESS, …)` describes a flat C-ABI struct directly; no JNI-style "GetXXXField"
helper, no Java-side wrapper, no copy. Off-heap allocation through `Arena`.
**Performance vs JNI:** Calling C functions: ~equivalent or somewhat better.
Passing Java function pointers into C: 3–4× faster than JNI's `JNIEnv*` + class
lookup path.
**Recent (2024):** FFM finalized in JDK 22 (March 2024); used by Netty,
Lucene's vector search.
**References:**
- [OpenJDK — Project Panama](https://openjdk.org/projects/panama/)
- [FOSDEM 2024 — FFM API a (quick) peek under the hood (Cimadamore)](https://archive.fosdem.org/2024/events/attachments/fosdem-2024-1714-foreign-function-memory-api/slides/22193/fosdem_2024_FtLDvIv.pdf)
- [Baeldung — Project Panama Guide](https://www.baeldung.com/java-project-panama)

### 5.2 Project Valhalla (JVM)
**Goals:** Value classes (formerly "inline classes") — identity-less,
mostly-immutable record-like types that the VM can flatten in fields, arrays,
and method bodies.
**Heap flattening (JEP 401, in progress):** A `Point[]` of value class
`Point(int x, int y)` becomes a single contiguous `int x[]; int y[]; …` —
no per-element header, no boxing, no indirection. Fields of a value class
inside a class similarly flatten in-place.
**Scalarization in JIT:** C2 already scalarizes after escape analysis; Valhalla
extends this to formal value-class semantics across method boundaries.
**Status (2026 EA):** Early-access builds available; targeting JDK 26+ for
incremental delivery.
**References:**
- [OpenJDK — Project Valhalla](https://openjdk.org/projects/valhalla/)
- [Value Classes and Objects](https://openjdk.org/projects/valhalla/value-objects)
- [JEP 401 — Heap Flattening (JVMLS 2025)](https://inside.java/2025/10/31/jvmls-jep-401/)
- [Wikipedia — Project Valhalla](https://en.wikipedia.org/wiki/Project_Valhalla_(Java_language))

### 5.3 CPython C-API churn / HPy / Faster CPython
**Why C-API is expensive:**
- Every API call dereferences `PyObject*`, touching `ob_refcnt` (causing cache
  ping-pong under multi-threading) and `ob_type` (cache-miss-prone).
- Many APIs return *borrowed* references — caller and callee disagree on lifetime
  → leaks if used wrong, not detectable statically.
- `PyArg_ParseTuple`, `PyDict_GetItemString`, etc. parse format strings at
  every call.
- API exposes implementation details (`PyObject->ob_refcnt` accessed by macro)
  preventing CPython from changing internals — the GIL is famously hard to
  remove because the C-API assumes serialized refcount manipulation.
**HPy (HPyProject):** `HPy` opaque handle replaces `PyObject*`. Implementations
(CPython / PyPy / GraalPy) provide their own opaque handle semantics —
binary-portable across CPython implementations. Adoption: NumPy started a port,
ultraJSON, and several smaller libraries.
**Faster CPython (PEP 659):** Specializing adaptive interpreter — bytecode
quickening. Hot opcodes self-rewrite from generic (`LOAD_ATTR`) to specialized
(`LOAD_ATTR_INSTANCE_VALUE_CACHED`) when shape stable. Speedups 10–60% per
release. Python 3.14 (Oct 2025) extends it; free-threaded build also benefits.
**References:**
- [HPy project](https://hpyproject.org/)
- [PEP 659 — Specializing Adaptive Interpreter](https://peps.python.org/pep-0659/)
- [Bernstein — Type information for faster Python C extensions](https://bernsteinbear.com/blog/typed-c-extensions/)
- [What's new in Python 3.14](https://docs.python.org/3/whatsnew/3.14.html)

### 5.4 Mojo's MLIR-based compilation
**Premise:** Mojo is an MLIR-first language (Chris Lattner / Jacques Pienaar /
Modular). Source → high-level Mojo dialect → progressive lowering through
custom dialects → LLVM → native code (or AI-accelerator backends).
**JIT vs AOT indistinguishability:** Because MLIR-level IR is fully typed and
shape-known, JIT and AOT produce equivalent code — there is no separate
"interpreter representation". Compile-time meta-programming uses the same
front-end interpreter that the JIT compiles.
**Value model:** Strict Rust-ish ownership: `borrowed` (immutable ref),
`inout` (mutable ref), `owned` (transfer/move). Recent (2024–25) renamed
`borrowed` → `read`, refined lifetime annotations, added `var` / `let`-style
distinctions.
**Struct layout:** Currently no SoA intrinsic (see §3.1); manual.
**References:**
- [Mojo Vision (Modular docs)](https://docs.modular.com/mojo/vision/)
- [Mojo Ownership](https://docs.modular.com/mojo/manual/values/ownership/)
- [Mojo: MLIR-Based Performance-Portable HPC Kernels (arXiv 2509.21039, 2025)](https://arxiv.org/pdf/2509.21039)
- [MLIR (Wikipedia)](https://en.wikipedia.org/wiki/MLIR_(software))

### 5.5 Rust `extern "C"` and `repr(C)` discipline
**FFI cost:** With `repr(C)` the Rust struct *is* the C struct — same field
order, same padding, same alignment. `extern "C" fn` uses the platform C ABI.
A direct `extern "C"` call is usually one `call` instruction; argument marshalling
is the same as a C-to-C call. No GC, no header, no implicit allocation, no
bounded box.
**Caveats:**
- Default `repr(Rust)` allows reordering / optimization; not stable across
  compiler versions.
- `repr(packed)` — strips padding; *taking a reference to a packed field is
  unsafe / UB on misalignment*. Reads/writes are split into byte-wise ops by
  the compiler when it can detect; otherwise faults on strict-alignment ISAs.
- `repr(C, packed)` — both, common for FFI to packed wire formats.
- Generic Rust types crossing FFI: usually wrapped in opaque handles.
**Why FFI cost stays bounded:** the compiler doesn't need to insert a *runtime
boundary* — there's no GC root scan on entry, no JNI-style transition, no
exception unwinding fixup (panics across FFI are UB).
**References:**
- [Rustonomicon — Other reprs](https://doc.rust-lang.org/nomicon/other-reprs.html)
- [Rust reference — Type layout](https://doc.rust-lang.org/reference/type-layout.html)

### 5.6 Swift resilient interfaces
**ABI stability (2019, Swift 5):** Apple's stdlib is now in the OS; user
binaries link against it without bundling. ABI-stable means generated code from
two compiler versions can interoperate.
**Resilience:** Library *implementation* changes shouldn't force recompiles of
clients — even for things like reordering fields, adding methods, adding cases
to an enum.
**Mechanism: dispatch thunks.** Calls to public APIs go through a per-API
"thunk" exported by the library. The thunk hard-codes the offset into the
witness/method table for *its* version of the library. New library versions
ship a new thunk; old client binaries still call the old thunk symbol, which is
preserved.
**Witness tables:** Swift's term for the protocol vtable. Each protocol
conformance produces a witness table mapping requirements → function pointers.
Protocol existential calls go through indirect call via witness table.
**Type metadata:** Each type has a runtime metadata record encoding fields,
size, layout. Generic code reads this rather than specializing on every type
(though the compiler can specialize if it has the source).
**Cost:** Resilient access is one extra indirection at the protocol/library
boundary; non-resilient (within a module / `@inlinable`) is direct.
**References:**
- [Swift.org — ABI Stability and More](https://www.swift.org/blog/abi-stability-and-more/)
- [Swift.org — Library Evolution](https://www.swift.org/blog/library-evolution/)
- [Swift ABI Stability Manifesto](https://github.com/apple/swift/blob/main/docs/ABIStabilityManifesto.md)
- [Faultlore — How Swift Achieved Dynamic Linking](https://faultlore.com/blah/swift-abi/)

---

## 6. Inline caching, polymorphism

### 6.1 Inline caches (V8, JSC, SpiderMonkey)
**State machine (universal):**
- **Uninitialized** — never executed; first hit triggers slow lookup, transitions
  to monomorphic.
- **Monomorphic** — has seen exactly one shape; one guard + one offset load.
- **Polymorphic** — 2–4 shapes (engine-defined cap); linear scan or jump
  table over guards.
- **Megamorphic** — > 4 shapes (V8: 4; JSC has multi-level megamorphic cache);
  falls back to a global property cache or full lookup. The optimizing tier
  typically refuses to specialize megamorphic sites.
**V8 specifics:** Each bytecode op that supports caching has a `FeedbackVector`
slot per call site (per function). Ignition reads the slot to dispatch
specialized handlers.
**JSC specifics:** ICs are inline patches in machine code (Baseline JIT)
or feedback structures (LLInt). DFG reads them.
**SpiderMonkey:** "CacheIR" — a small DSL for IC stub bodies; new ICs are
expressed as CacheIR programs and JIT-compiled to native stubs.
**References:**
- [Wikipedia — Inline caching](https://en.wikipedia.org/wiki/Inline_caching)
- [Mathias Bynens — Shapes and ICs](https://mathiasbynens.be/notes/shapes-ics)
- [Builder.io — Understanding Monomorphism](https://www.builder.io/blog/monomorphic-javascript)

### 6.2 Speculative optimization & deopt
**Mechanism:** Profile collected in lower tiers ("usually a fixnum"). Optimizing
compiler emits specialized code with *guards* (`assume X is fixnum, else exit`).
Guard failure invokes a **deopt** routine: locate the abstract interpreter
state from compiled-frame metadata, materialize an interpreter (or lower-tier)
frame, resume there.
**Guard placement strategies:** Speculate at *use* (each branch separately),
versus speculate at *check* (one guard at the entry to a region). LuaJIT
speculates per-trace; HotSpot speculates per-method.
**Deopt cost:** Frame reconstruction can require materializing escape-analyzed
objects (CVE-2022-1364 was an EA-deopt bug). Recompilation cost — repeated
deopt thrashing is a known anti-pattern; HotSpot has "deopt counters" that
disable speculation after N failures.
**Formal verification:** Barrière et al., POPL 2021, formally verified a
JIT compiler's deopt+speculate loop.
**References:**
- [Barrière et al. — Formally Verified Speculation and Deoptimization in a JIT Compiler (POPL 2021)](https://janvitek.org/pubs/popl21.pdf)
- [Project Zero — CVE-2022-1364: Inconsistent Object Materialization in V8](https://googleprojectzero.github.io/0days-in-the-wild/0day-RCAs/2022/CVE-2022-1364.html)
- [HotSpot Performance Techniques](https://wiki.openjdk.org/spaces/HotSpot/pages/11829300/PerformanceTechniques)

### 6.3 GraalVM Truffle / Sulong
**Truffle:** Java framework for AST-walking interpreters. Author writes an AST
interpreter; the framework, in collaboration with the Graal compiler, performs
*partial evaluation* — specializes the interpreter to the program. Result:
the program-specific compiled code is approximately as fast as a hand-written
JIT for that language, with roughly the effort of writing an interpreter.
**Sulong:** Truffle interpreter for LLVM bitcode. C / C++ / Rust → bitcode →
Sulong → Graal-compiled native code, *running on the JVM heap*.
**Polyglot:** Truffle defines an interop protocol; values from one language
can be read as another's via "messages" (read-member, invoke, etc.) defined
by the Truffle Library annotations. Used by GraalPy, TruffleRuby, GraalJS,
FastR, Espresso (Java-on-Truffle), Sulong.
**Trade-off:** First-call cost is high (PE compile is non-trivial); steady
state is competitive with V8/HotSpot.
**References:**
- [GraalVM Truffle Framework](https://www.graalvm.org/latest/graalvm-as-a-platform/language-implementation-framework/)
- [Sulong Slides (LLVM devmtg 2016)](https://llvm.org/devmtg/2016-01/slides/Sulong.pdf)
- [GraalVM Polyglot Programming](https://www.graalvm.org/latest/reference-manual/polyglot-programming/)

---

## 7. Novel / 2024-2026 directions

### 7.1 Pulley (2025)
Wasmtime's portable bytecode interpreter, with a Cranelift backend that emits
Pulley bytecode rather than machine code. Goal: run Wasm on platforms without
native Cranelift support (or where native code is forbidden by sandboxing
policy). ~10× slowdown vs native Cranelift; uses macro-ops and ISLE for
super-instruction fusion. Slated for Wasm phase-3 standardization in 2025.
- [Bytecode Alliance — Wasmtime portability](https://bytecodealliance.org/articles/wasmtime-portability)
- [pulley-interpreter crate](https://crates.io/crates/pulley-interpreter)

### 7.2 RichWasm (POPL 2024)
WebAssembly extension with a substructural type system (linear / unrestricted
qualifiers) for safe shared-memory concurrency in the linear-memory model.
Demonstrates that Wasm-style flat memory is compatible with sophisticated
typing.
- [RichWasm paper](http://www.ccs.neu.edu/home/amal/papers/richwasm.pdf)

### 7.3 V8 leaving Sea of Nodes (2024)
After ~10 years of SoN, V8 migrated TurboFan's mid-end to Turboshaft (CFG-based
IR). Cited reasons: SoN's free-floating nodes made debugging hard, complicated
some optimizations, and hadn't delivered the expected dramatic optimization
wins for the JS workload.
- [V8 — Land ahoy: leaving the Sea of Nodes](https://v8.dev/blog/leaving-the-sea-of-nodes)

### 7.4 JEP 519 / 534 (Compact Object Headers, 2025)
Project Lilliput's first product feature: 8-byte object header in the JVM,
saving ~10–20% heap. Class pointer compressed to 22 bits. JEP 534 (draft) would
make compact headers default-on.
- [JEP 519](https://openjdk.org/jeps/519)
- [JEP 534 draft](https://openjdk.org/jeps/534)

### 7.5 Project Valhalla (early-access 2025–26)
Value classes / heap flattening in production-track JEPs. JEP 401 implements
heap flattening; significant speedup on flat-array workloads where C2 emits
SIMD over densely-packed value class arrays.
- [Inside.java — JEP 401 (JVMLS 2025)](https://inside.java/2025/10/31/jvmls-jep-401/)

### 7.6 Faster CPython continued (Python 3.13/3.14, 2024–25)
PEP 659 specializing adaptive interpreter; Python 3.14 also enables it under
the no-GIL free-threaded build (PEP 703). Cumulative 40–50% speed-up vs Python
3.10 across the pyperformance benchmark mix. Tier 2 "uops"
(micro-operation IR) and an experimental copy-and-patch JIT (PEP 744).
- [What's new in Python 3.14](https://docs.python.org/3/whatsnew/3.14.html)
- [PEP 659](https://peps.python.org/pep-0659/)

### 7.7 WASM-GC adoption (2024–25)
Production shipped in Chrome (default since 2023), Firefox, Safari. Real
language ports: Kotlin/Wasm, Dart→Wasm (Flutter), Java front-ends, Scheme
(Hoot/Whiffle). Approach contrasts NaN-boxing: typed reference is *checked at
the type level* and engine produces flat hidden-class-style code without
runtime tag bits.
- [V8 — bringing GC languages to Wasm](https://v8.dev/blog/wasm-gc-porting)

### 7.8 Mojo evolution (2024–26)
Refined ownership keywords (`read` / `mut` / `owned`); explicit lifetime
parameters on borrowed references; first-class `origin` lifetimes in the type
system; trait inheritance reworked. SoA layout still a roadmap item. JIT and
AOT remain unified through MLIR.
- [Modular — Deep dive into ownership in Mojo](https://www.modular.com/blog/deep-dive-into-ownership-in-mojo)
- [Mojo: MLIR-Based Performance-Portable HPC Kernels (arXiv, 2025)](https://arxiv.org/pdf/2509.21039)

### 7.9 Tiered JIT for instruction-set simulation (VMIL 2024)
Chen et al. apply HLL-VM tiered-JIT techniques to RISC-V instruction-set
simulation. Suggests that the production-vetted "interpreter + light tier-1 +
aggressive tier-2 + OSR" design is a transferable architectural pattern beyond
HLL VMs.
- [Accelerate RISC-V ISS by Tiered JIT (VMIL 2024)](https://2024.splashcon.org/details/vmil-2024-papers/4/Accelerate-RISC-V-Instruction-Set-Simulation-by-Tiered-JIT-Compilation)

---

## 8. Cross-cutting findings

### 8.1 Where the production state of the art has converged

**A single uniform stack/frame format across tiers is the standard.** HotSpot
(interp ↔ C1 ↔ C2), JSC (LLInt ↔ Baseline ↔ DFG ↔ FTL), and V8 (Ignition ↔
Sparkplug ↔ Maglev ↔ Turbofan/Turboshaft) all use a single per-VM stack-frame
shape with extra metadata (stack maps, scope descriptors) for OSR. The
interpreter and the optimizing JIT do *not* convert frames at the boundary —
they share them. Deopt always re-materializes the abstract interpreter state
from compiled-frame metadata.

**Hidden classes / shapes / structures / maps are the universal property-access
mechanism for dynamic languages.** V8 maps, JSC structures, SpiderMonkey
shapes, PyPy maps, even CPython 3.11+ "type version tags" feeding
`LOAD_ATTR_INSTANCE_VALUE`. Inline-cache state machine
(uninitialized → monomorphic → polymorphic ≤4 → megamorphic) is essentially
identical across V8, JSC, and SpiderMonkey.

**Tag-free dispatch wins when the type system delivers it.** OCaml, the JVM
verifier, Wasm, and now WASM-GC: when you can prove a slot's type from the
program's type system, you don't need per-value tags. NaN-boxing and low-bit
tagging are responses to *dynamic* typing, not preferred when alternatives
exist.

**Cranelift + MLIR are eating the "build a JIT" market.** New language
implementations (shape-jit, several Wasm runtimes, parts of rustc, Mojo, MLIR
projects in general) reach for Cranelift / MLIR rather than custom backends or
LLVM. Compile time is the deciding factor — both are 10× faster than LLVM at
comparable code quality for non-extreme workloads.

**FFI ABI is converging on flat `extern "C"` + structured layout description.**
Java FFM (`MemoryLayout`), Rust `repr(C)`, Swift `@_cdecl`, Wasm component
model. Old-school marshalling FFIs (JNI, CPython C-API) are widely
acknowledged as costly and being incrementally replaced (Panama for JNI, HPy
for CPython).

**Speculative optimization with deopt is the universal optimizing-JIT
contract.** Speculate aggressively in the optimizing tier, guard, deopt back
to the lower tier on guard miss. Monomorphic IC observation drives
specialization; deopt counters prevent thrashing. The pattern is identical
in HotSpot, V8, JSC, SpiderMonkey, LuaJIT, GraalVM.

### 8.2 Where it has *not* converged

**Object header sizes vary by 4×.**
- Wasm-GC `i31`: 0 bytes (immediate). 
- Wasm-GC struct: implementation-defined, typically 4–8 B.
- OCaml block: 1 word (8 B).
- JVM Lilliput: 8 B.
- JVM legacy: 12–16 B.
- .NET reference type: 8 B (x86) / 16 B (x64).
- CPython object: 16 B + GC head.

**Slot encoding for dynamic types remains divergent.**
- V8 / SpiderMonkey: 32-bit pointer compression with low-bit SMI tag, separate
  raw double arrays.
- JSC: 64-bit pure NaN-boxing.
- LuaJIT: 64-bit NaN-tagging (sign-bit-based variant).
- OCaml / SBCL: low-bit tagging (no NaN involvement).
- Wasm-GC: typed references (no tag bit).
- HotSpot: typed slots, no tag bit.

**Optimizing-JIT IR choice is not settled.**
- HotSpot C2: Sea of Nodes (since ~1999).
- V8 Turbofan: was Sea of Nodes; Turboshaft is CFG (2024).
- JSC FTL: Sea of Nodes-ish (B3).
- LuaJIT: trace tree (linear SSA).
- Cranelift: CFG-based with extension blocks; not SoN.
- GraalVM: Sea of Nodes.

**Where *escaped* objects live is design-divergent.**
HotSpot scalar-replaces aggressively, then re-materializes on deopt. Wasm-GC
mandates GC-managed structs always live in the GC heap. Mojo / Rust use
ownership to avoid the question.

**Polymorphic-IC threshold is 4 by tradition, not principle.**
V8: 4. JSC: 4 (with multi-level megamorphic cache). SpiderMonkey: 4.
HotSpot: 2 (bimorphic only); falls to vtable thereafter.

### 8.3 2024–2026 frontier

- **8-byte JVM headers** (Lilliput / JEP 519, JDK 25, September 2025) — first
  major header shrink since compressed class pointers in 2008.
- **Heap flattening for value classes** (Valhalla / JEP 401, EA 2025) — finally
  brings Java's "everything is a reference" runtime closer to C-struct density.
- **Wasm-GC in production** — 2024 was the year browsers shipped it;
  Kotlin/Wasm, Dart/Wasm, Java/Wasm are real targets.
- **Pulley** — proves portable interpretation can share a Cranelift front-end
  with native compilation; relevant for any project wanting "JIT where allowed,
  interpret where not".
- **PyPI's pivot away from GIL** — Python 3.13 free-threaded build
  experimental, 3.14 production. Refcount remains; deferred / biased RC reduces
  contention. The `PyObject` shape is unchanged but its operations are
  rethought.
- **MLIR-everywhere** — Mojo is the proof-of-concept for a single typed IR
  spanning interpret/JIT/AOT/GPU/accelerator. CIRCT, MLIR-AI, and similar are
  expanding.
- **Verification of JIT correctness** — Barrière et al. (POPL 2021), Icarus
  (Hovav et al.), Artemis (SOSP 2023) — speculative-optimization correctness
  is becoming a formally tractable problem.
- **V8 leaving Sea of Nodes** — counter-trend; suggests SoN's reign of
  ~25 years isn't unconditional. Production engineering effort and debuggability
  beat theoretical IR elegance for V8's workload mix.
- **Tagless / type-driven dispatch on Wasm-GC** — emergent question: if you
  can target Wasm-GC, do you need NaN-boxing at all? Several Scheme/Lisp
  ports (Whiffle, Hoot) say no.
