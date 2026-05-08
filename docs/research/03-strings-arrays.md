# Research survey 03 — Strings, arrays, direct memory access

A structured technical survey of state-of-the-art string and array representations,
direct-memory access patterns, reuse analysis, zero-copy interop, and 2024-2026
frontier work. Facts and tradeoffs only — no recommendations for Shape's specific
use case.

---

## 1. String representations

### 1.1 CPython PEP 393 (Flexible String Representation)

**Layout**: Three concrete C structs - `PyASCIIObject` (the base header with
`length`, `hash`, `state` flags including `interned`/`kind`/`compact`/`ascii`/`ready`,
and `wstr`), `PyCompactUnicodeObject` (adds `utf8_length`, `utf8`, `wstr_length`),
and `PyUnicodeObject` (adds a union of `Py_UCS1*`/`Py_UCS2*`/`Py_UCS4*`). Header is
~56 bytes on 64-bit; payload follows immediately after the struct (single
allocation since 3.3).
**Key innovation**: Encoding is chosen at creation by scanning for the maximum
code point: ASCII if `<128`, Latin-1 (UCS-1) if `<256`, UCS-2 if `<65536`,
otherwise UCS-4. All operations remain O(1) indexing because the width is fixed
per-string. There are no narrow/wide builds anymore.
**Performance**: 1-byte storage for the overwhelming majority of real-world
strings (English/Latin/source code). Fast path encodes detection during creation;
random-access stays constant-time. Header overhead (~56B) is heavy for very
short strings.
**Why this wins / loses**: Wins for memory at scale (Latin-only payloads halve
or quarter), wins for C-API compatibility (each width has a contiguous buffer),
loses on header-to-payload ratio for 1-3 character strings.
**References**:
- [PEP 393](https://peps.python.org/pep-0393/)
- [Python behind the scenes #9 (tenthousandmeters)](https://tenthousandmeters.com/blog/python-behind-the-scenes-9-how-python-strings-work/)
- [CPython Unicode C API](https://docs.python.org/3/c-api/unicode.html)

### 1.2 V8 string types

**Layout**: A class hierarchy: `SeqOneByteString` / `SeqTwoByteString` (actual
Latin-1 or UTF-16 storage), `ConsString` (two child pointers + length, lazy
concat), `SlicedString` (parent pointer + offset + length, lazy substring),
`ThinString` (forwards to an internalized sibling), `ExternalString` (pointer
to off-heap C++ resource).
**Key innovation**: Lazy concat (`ConsString`) makes `s = a + b` allocate only
a small node containing two pointers; the bytes are never materialized until
something reads them. Lazy substring (`SlicedString`) does the same for `slice`.
**Performance**: Concatenation is O(1). String access does an opportunistic
flatten via `String::Flatten`, which walks the cons tree and writes a contiguous
sequential string. `kMinLength = 13` characters: shorter cons would just be
flattened immediately. ThinString avoids breaking pointer-equality after
internalization.
**Why this wins / loses**: Brilliant for code that builds long strings via `+=`
and a single later read. Loses on workloads that do build-then-touch repeatedly
(repeated `Flatten`) or on memory in pathological cons trees. External strings
let an embedder hand V8 a borrowed buffer (zero-copy bridge).
**References**:
- [iliazeus: V8 string optimizations](https://iliazeus.lol/articles/js-string-optimizations-en/)
- [danbev/learning-v8 string notes](https://github.com/danbev/learning-v8/blob/master/notes/string.md)
- [v8/src/objects/string.h](https://github.com/v8/v8/blob/master/src/objects/string.h)
- ["Moving V8 to only flat strings" (design doc)](https://docs.google.com/document/u/0/d/1mgeH9Kii0K09so4EReZUn6ua9efqRNA2ZaUfBQ-0z7c/mobilebasic)

### 1.3 JSC strings (JavaScriptCore / WebKit)

**Layout**: `JSString` is a fixed 16 bytes (header + single `m_fiber` pointer to
a `WTF::StringImpl`). `StringImpl` carries length, ref count, and an `is8Bit`
flag. `JSRopeString` is 48 bytes and packs three fibers into 12 bytes plus
length and `is8Bit` bits in the LSBs of fibers; `isRope` is encoded in the
first fiber's LSB.
**Key innovation**: Length and width flags are encoded into pointer LSBs for
ropes — saves a separate header word. JSC keeps the rope structure narrower
than V8's typical layout (3-fiber ropes resolved as one shape).
**Performance**: Like V8, lazy concatenation via ropes; resolution is
incremental and includes optimized 3-fiber paths. 8-bit ropes can in some
historical bugs contain 16-bit children — width is checked on resolution.
**Why this wins / loses**: Tighter `JSString` size than V8's typical large
string-shape variants; the price is more complex pointer-arithmetic and a
narrower set of layouts.
**References**:
- [JSC JSString.h (WebKit)](https://github.com/WebKit/WebKit/blob/main/Source/JavaScriptCore/runtime/JSString.h)
- [WebKit Bug 205323 — 8Bit JSRopeString can contain 16Bit string](https://bugs.webkit.org/show_bug.cgi?id=205323)
- [WebKit JSC fiber rope handling commit](https://www.mail-archive.com/webkit-changes@lists.webkit.org/msg224281.html)

### 1.4 Swift String

**Layout**: `String` is a 16-byte value (`_StringObject`: two words on 64-bit).
A discriminator nibble in the upper bits chooses small vs. large form. Small
form packs up to **15 UTF-8 code units** inline. Large form has a `CountAndFlags`
word (lower 48 bits = length, upper 16 bits = flags `isASCII`/`isNFC`/
`isNativelyStored`/`isTailAllocated`/`isForeignUTF8`) plus an "object" word
pointing to tail-allocated UTF-8 bytes, an `NSString` bridge, or constant-section
literal storage.
**Key innovation**: SSO with cached perf flags. Native storage is tail-allocated
immediately after the storage object header (one allocation). Switched to UTF-8
in Swift 5 (was UTF-16 before).
**Performance**: 15-byte SSO covers most identifiers and short text. ASCII flag
enables fast paths. `Character` iteration is grapheme-cluster-correct (sees
"é" with combining mark as one element), which is semantically-correct but
costs more than indexing code units.
**Why this wins / loses**: Wins on correctness (default iteration matches user
intuition for human-readable text). Loses on raw indexing perf — `String.Index`
isn't an `Int`; you pay grapheme-cluster scanning unless you drop to `.utf8`/
`.unicodeScalars`.
**References**:
- [swift/stdlib/public/core/StringObject.swift](https://github.com/swiftlang/swift/blob/main/stdlib/public/core/StringObject.swift)
- ["State of String: ABI, Performance, Ergonomics, and You!" (Swift Forums)](https://forums.swift.org/t/state-of-string-abi-performance-ergonomics-and-you/7397)
- [mikeash: Why is Swift's String API So Hard?](https://www.mikeash.com/pyblog/friday-qa-2015-11-06-why-is-swifts-string-api-so-hard.html)

### 1.5 Java String — JEP 254 Compact Strings

**Layout**: `String` carries `byte[] value` plus a `byte coder` flag (0=Latin-1,
1=UTF-16). Pre-Java 9, `String` was always `char[]` (UTF-16). Each method
branches on `coder`. Indexing is O(1) given the `coder`.
**Key innovation**: Heap savings without behavioral change. Internal char[]
fields accept the cost of a flag check at every operation in exchange for ~50%
shrink for Latin-1-only data, which is the common case based on field telemetry
that motivated the JEP.
**Performance**: Free for Latin-1-only programs (and saves heap+GC time).
Programs that are always UTF-16 see a small slowdown from the per-op coder
branch (~5-10% reported in some workloads).
**Why this wins / loses**: Wins on broad heap reduction; loses on workloads
genuinely all-non-Latin-1. Interning is opt-in (`String.intern()`) because the
pool is a JVM-side `StringTable` (GC root, can stall pauses if oversized) and
interned references compete with the regular GC scheme.
**References**:
- [JEP 254: Compact Strings](https://openjdk.org/jeps/254)
- [Ionut Balosin: Compact Strings can slow UTF-16 apps](https://ionutbalosin.com/2018/06/compact-strings-feature-might-slow-down-predominant-utf-16-strings-applications/)
- [Shipilev: JVM Anatomy Quark #10 String.intern()](https://shipilev.net/jvm/anatomy-quarks/10-string-intern/)

### 1.6 Rust String / &str — and ecosystem crates

**Layout**: `String` is `Vec<u8>` (24 bytes: ptr + len + cap), guaranteed UTF-8.
`&str` is a 2-word borrow (ptr + len). No SSO in std; no implicit refcount.
Indexing is `O(1)` byte-level but **byte indices, not char indices** — char-
level indexing is `O(n)` because UTF-8 is variable-width. Std deliberately
distinguishes byte (`s.as_bytes()[i]`), `char` iteration, and grapheme handling
(in 3rd-party `unicode-segmentation`).
**Key innovation**: Borrowed `&str` (and lifetimes) make zero-copy substring/
slicing free in safe code at the type level. Composition via crates rather
than baked-in.
**Performance** & **ecosystem crates** (per `string-rosetta-rs` benchmarks):

| crate | size | inline SSO | clone | notes |
|---|---|---|---|---|
| `String` (std) | 24 B | none | O(n) | baseline |
| `compact_str` | 24 B | 24 B (uses niche) | O(n) for heap | always-on SSO; immutable when small? no, mutable |
| `smol_str` | 24 B | 22-23 B | O(1) heap (Arc) | immutable; deprecated in favor of `ecow` |
| `arcstr` | 8 B | none | O(1) | static-friendly; just an `Arc<str>` improvement |
| `smartstring` | 24 B | 23 B | O(n) | drop-in for `String` API |
| `kstring` | 24 B | 15 B | varies | optimized for "key" strings |
| `hipstr` | 24 B | 23 B | O(1) | borrowed/shared/inline tri-mode |
| `ecow` | 16 B | 15 B | O(1) | refcounted CoW; `smol_str` successor |

**Why this wins / loses**: No SSO in std means `String` and `&str` reflect
exactly what's in memory — no hidden niches, no hidden refcounts, predictable
codegen. Loses on ergonomics for "short, frequently-cloned" workloads (parsers,
ASTs, dictionary keys), which is exactly why the ecosystem fills the gap.
**References**:
- [string-rosetta-rs comparison](https://github.com/rosetta-rs/string-rosetta-rs)
- [Swatinem: Choosing a more optimal String type](https://swatinem.de/blog/optimized-strings/)
- [fasterthanli.me: Small strings in Rust](https://fasterthanli.me/articles/small-strings-in-rust)
- [docs.rs/compact_str](https://docs.rs/compact_str)

### 1.7 C++ std::string SSO

**Layout — libc++ (Clang/LLVM)**: 24-byte struct. Short mode uses the entire
24 bytes for inline data; first byte stores length (low bits also tag short
vs. long). Inline capacity = **22 chars** + null terminator (advertised "23
characters" — 23 + size byte fits in 24).
**Layout — libstdc++ (GCC)**: 32-byte struct. Always carries `ptr/len/cap` (16
bytes) plus a 16-byte inline buffer; inline capacity = **15 chars**.
**Layout — MSVC STL**: 32-byte struct, ~15 char inline.
**Key innovation**: SSO is folded into the same value type — no `std::optional`-
like tag. The "is short" is hidden in unused high bits of size (libc++) or in
whether `ptr` aliases the inline buffer (libstdc++).
**Performance**: libc++ wins on inline capacity but loses 8 bytes per *long*
string vs. libstdc++ (no, actually libc++ at 24B wins both). libstdc++'s 32B
gives slightly more padding for long strings (cap is in-place). MSVC and GCC
agree on size 32; LLVM 24.
**Why this wins / loses**: Universally adopted; standard reference for SSO
design. Loses when strings cluster around 16-25 bytes (boundary cliff: GCC
heap-allocates, LLVM stays inline) — workload-dependent.
**References**:
- [tigercosmos: SSO in C++](https://tigercosmos.xyz/en/post/2022/06/c++/sso/)
- [TastyHedge: Memory layout of std::string](https://tastyhedge.com/blog/memory-layout-of-std-string/)
- [elliotgoodrich/SSO-23 (memory-optimal SSO)](https://github.com/elliotgoodrich/SSO-23)

### 1.8 Mojo String

**Layout** (as of 2026): `String` is a value type with a three-mode strategy —
(a) static-pointer mode for string literals (free, immortal), (b) SSO inline
for ≤23 bytes (no allocation), (c) refcounted heap for larger payloads. Has
copy-on-write semantics on assignment so the actual byte-copy is deferred until
mutation.
**Key innovation**: Combines the libc++-style 23-byte inline form with copy-
on-write *and* with Mojo's owned/borrowed parameter convention — `String` has
move semantics by default. Literals are baked into the binary and aliased by
zero-cost `String` values.
**Performance**: SSO inline, free for literals, refcount + COW for longer.
Move-by-default avoids most copies.
**Why this wins / loses**: Wins on a range that's wider than any other single
language type — literals to large mutable buffers all in one type. Mojo's
ownership rules prevent the typical CoW pitfalls (e.g., shared inadvertent
mutation). Still maturing — owned semantics shifted in 2025 toward stricter
checks.
**References**:
- [Mojo String docs](https://docs.modular.com/mojo/std/collections/string/string/String/)
- [Mojo Ownership manual](https://docs.modular.com/mojo/manual/values/ownership/)
- [Mojo Value semantics manual](https://docs.modular.com/mojo/manual/values/value-semantics/)

### 1.9 OCaml strings

**Layout**: A single OCaml block tagged `String_tag = 252` (≥ `No_scan_tag = 251`,
so the GC treats payload as opaque bytes). Header has size in machine words.
Payload is the raw bytes followed by padding bytes and a final length byte
encoding `(words*sizeof(word) - last_byte - 1)` so the byte length is recoverable
without scanning. No SSO. `string` is immutable; `bytes` is the mutable variant
with the same layout.
**Key innovation**: Reuse of the standard block layout for strings (one tag
suffices). Length recoverable from header + last byte (clever 1-byte trick).
**Performance**: Always boxed (heap-allocated, even short strings). Boxed-only
keeps the GC simpler. No SSO ever.
**Why this wins / loses**: OCaml uses GC and pointer-tagged `int`/`pointer`
discrimination for everything; strings would not fit naturally into a tagged
1-word value. Loses on small-string density (every 1-char string is a boxed
allocation) but is fine because OCaml programs typically hold few strings
relative to their other data.
**References**:
- [Real World OCaml: Memory Representation of Values](https://dev.realworldocaml.org/runtime-memory-layout.html)
- [ocaml.org: Memory Representation](https://ocaml.org/docs/memory-representation)

### 1.10 LuaJIT strings

**Layout**: All strings are `GCstr` heap objects with `(reserved, hashalg, sid,
hash, len)` plus payload. A global hash table (`global_State::strhash`) holds
*every* string in the VM — strings are interned at creation.
**Key innovation**: Pointer-equality `==` is a single integer compare. Hash is
cached. String identity propagates through closures/tables; lookups in tables
keyed by interned strings are basically pointer-key lookups.
**Performance**: Creating a string costs a hash-table probe; identical strings
collapse. `==`, table lookup by string key, and method dispatch are very fast.
The cost lives at *string creation*, not at use.
**Why this wins / loses**: Wins for programs with high string-keyed table use
(typical Lua) — table lookup is essentially pointer-compare. Loses on
parser/serializer workloads that produce many transient unique strings (every
one hits the global table). Has known scaling issues with very large hash
tables (see issue trackers).
**References**:
- [luajit.io: LuaJIT String Interning](http://luajit.io/posts/luajit-string-interning/)
- [LuaJIT lj_obj.h](https://github.com/LuaJIT/LuaJIT/blob/v2.1/src/lj_obj.h)
- [LuaJIT issue #168: hash collisions](https://github.com/LuaJIT/LuaJIT/issues/168)

### 1.11 Erlang binaries

**Layout**: Three flavors. `HeapBinary` (≤64 bytes, lives in process heap, GC'd
normally). `RefcBinary` (>64 bytes, lives in shared binary heap, refcounted via
a `ProcBin` cell on each owning process heap). `SubBinary` (a 4-tuple-ish
descriptor with parent pointer + offset + size + bit-offset; references a slice
into a refc or heap binary, never into another sub-binary).
**Key innovation**: Sub-binaries make pattern-matching a refc binary into many
sub-pieces a zero-copy operation — only descriptor allocations. The runtime
maintains binary identity through process boundaries via refcount.
**Performance**: Binary parsers (HTTP, protocol decoders) are extremely fast
on Erlang/Elixir because slicing is a descriptor allocation, not a copy.
Refcount means the underlying buffer survives until the last sub-binary dies.
**Why this wins / loses**: Wins enormously for binary protocol code. Loses on
the indirection cost (every read goes through descriptor + parent), and refc
binaries can hold large allocations alive long after most sub-references die
(the "binary leak" pattern is well-known).
**References**:
- [Erlang Efficiency Guide: Binary handling](https://www.erlang.org/doc/system/binaryhandling.html)
- [Mentel: A short guide to refc binaries](https://medium.com/@mentels/a-short-guide-to-refc-binaries-f13f9029f6e2)

### 1.12 Roc strings

**Layout**: A `Str` is a refcounted `Vec<u8>` (UTF-8). Backing buffer has a
prefix-stored refcount.
**Key innovation**: Reuse analysis (see §5.2 Morphic): when the compiler/runtime
can prove a `Str` has refcount = 1 at a mutation site, the buffer is mutated
in place. Most Roc string-building operations therefore allocate once and grow
in place. Slicing creates "seamless slices" that share the parent buffer
(extra ref) but those slices and their parents are then ineligible for in-place
mutation.
**Performance**: Refcount-1 mutation gives apparent purely-functional code
near-Vec-mutation speed. Slicing is cheap.
**Why this wins / loses**: Wins for functional-style code that builds and
discards strings — the runtime opportunistically does the imperative thing.
Loses on shared / cached strings — those force genuine copies.
**References**:
- [Opportunistic Mutation in Roc (HN discussion)](https://news.ycombinator.com/item?id=45741156)
- [Roc Functional](https://www.roc-lang.org/functional)
- [Reference Counting with Reuse in Roc (thesis)](https://studenttheses.uu.nl/handle/20.500.12932/44634)

---

## 2. String interning

### 2.1 JVM string pool (`String.intern()` and `StringTable`)

**What it is**: A native hash table inside the JVM, separate from the heap's
normal object graph but in a Java-language sense holds canonical `String`
references. `String.intern()` is a native call that adds-if-absent and returns
the canonical entry.
**Layout**: `StringTable` is a hash table; size is tunable via
`-XX:StringTableSize`. Entries are weak references — strings can be GC'd if
nothing else holds them. JEP 192 / JDK 8u20 introduced **string deduplication**
in G1 (transparent, GC-driven; not the same as interning) under
`-XX:+UseStringDeduplication`.
**Performance**: `intern()` is slow (native-call + hash-table) and was a
historical scaling pain point; smaller default `StringTableSize` was a footgun.
String dedup in G1 catches duplicates without the explicit-call cost but only
during GC, and only deduplicates the underlying `byte[]`.
**Why opt-in**: Pool entries are quasi-permanent (until GC determines no live
refs), the pool is a GC root, and forcing canonicalization isn't free — making
it default would penalize programs that don't need pointer-equality.
**References**:
- [Shipilev: JVM Anatomy Quark #10](https://shipilev.net/jvm/anatomy-quarks/10-string-intern/)
- [java-performance.info: String Intern in 6/7/8](https://java-performance.info/string-intern-in-java-6-7-8/)

### 2.2 LuaJIT global interning (revisited)

Already covered in §1.10. Key points: every string is interned at construction;
no opt-in. Cost is at creation, win is at use. Trade depends entirely on the
read/create ratio. LuaJIT's strhash has known size and resize concerns under
very-large workloads.

### 2.3 Static / compile-time interning (Crystal symbols, OCaml polymorphic
variants, Lisp symbols)

**Crystal `Symbol`**: Symbols are compile-time constants represented internally
as `Int32`. There is no `Symbol.new` or string-to-symbol coercion; you cannot
create them dynamically. Each unique `:identifier` literal in source is
assigned a unique integer at compile time. Comparison is integer-equality.
**OCaml polymorphic variants** (` ``Foo``): tag is a hash of the constructor
name, computed at compile time; equality on variants without payload is
integer-equality. Not the same as full interning but fills the same niche for
small enumerated sets.
**Common Lisp symbols**: a symbol is a heap object containing name + package +
value/function/property bindings. Looking up `'foo` in a package is the
pointer-equal canonical symbol. Symbols may be uninterned (`#:foo`) but the
default in `read` is to intern in current package.
**Why this wins / loses**: Wins on cost (zero runtime work) and on guaranteeing
identity. Loses on flexibility — no dynamic symbols means string→symbol bridges
always have to go through some other mechanism (look-up table, perfect hash,
etc).
**References**:
- [Crystal: Symbol](https://crystal-lang.org/api/master/Symbol.html)
- [Crystal Compile-time flags](https://crystal-lang.org/reference/1.20/syntax_and_semantics/compile_time_flags.html)
- [Common Lisp Cookbook: Strings & Symbols](https://lispcookbook.github.io/cl-cookbook/type.html)

### 2.4 Rope vs string

**What a rope is**: A binary-tree structure where leaves are short strings and
internal nodes carry the cumulative length of their subtree. Concatenation is
O(log n) (link two nodes). Substring is O(log n). Random index is O(log n).
**Where ropes win**: Very long, frequently-edited strings — text editors,
versioned documents, append-heavy buffers. Cedar (Xerox PARC, 1982) used ropes
extensively. The Boehm/Atkinson/Plass 1995 paper is the canonical reference.
**Where ropes lose**: O(log n) random access vs. O(1) for flat strings; pointer
chasing destroys cache friendliness; iteration has bad ILP. For typical short
program strings (filenames, identifiers, short messages), ropes are pure
overhead.
**Modern flavor**: Most production "ropes" today are *piece tables* (used in
VS Code's TextBuffer) or *gap buffers* — different structures with similar
goals. V8's ConsString is a degenerate rope (tree with concat-only edges,
flattened on access) — not really a rope in the editor sense.
**Why most languages don't bother**: At the language-level string type, almost
all strings are short and read-many-times. The only languages whose default
strings are rope-like are those with extremely fast concatenation requirements
(Erlang's iolist, V8's ConsString) — and even those are only "lazy concat
trees", not full ropes.
**References**:
- [Boehm/Atkinson/Plass: Ropes are Better than Strings](https://www.bitsavers.org/pdf/xerox/parc/techReports/CSL-94-10_Ropes_Are_Better_Than_Strings.pdf)
- [Wikipedia: Rope (data structure)](https://en.wikipedia.org/wiki/Rope_(data_structure))

---

## 3. Array / vector representations

### 3.1 Julia `Vector{T}` and `Array{T,N}`

**Layout**: A header (length + dimensions + flags) plus a contiguous, flat
payload of `T`-sized elements. For `isbits` element types (primitives, simple
structs of primitives), elements are stored directly (unboxed). For `Any` /
boxed types, elements are pointers. Multi-dimensional arrays are column-major
(Fortran order) for compatibility with BLAS/LAPACK.
**Key innovation**: Type parameter `{T,N}` on the type itself enables full
specialization — the compiler generates a loop with a known stride and a known
element type. Array views (`view(A, ranges...)`) produce zero-copy windows
into a parent.
**Performance**: For `isbits T` and unit-stride access, the loop pipeline
through LLVM produces SIMD code automatically; LoopVectorization.jl
(`@turbo`) is widely used to push this further. Strided / non-unit access
defeats vectorization.
**Why scientific devs love it**: Fortran-quality numeric code from a high-level
language with REPL ergonomics. Multi-dim indexing is first-class. BLAS
interop is one allocation away.
**References**:
- [Julia Performance Tips](https://docs.julialang.org/en/v1/manual/performance-tips/)
- [SIMD.jl](https://github.com/eschnett/SIMD.jl)
- [LoopVectorization.jl](https://github.com/JuliaSIMD/LoopVectorization.jl)
- [Demystifying Auto-vectorization in Julia](https://www.juliabloggers.com/demystifying-auto-vectorization-in-julia/)

### 3.2 NumPy `ndarray`

**Layout**: A Python object header plus `(data_ptr, dtype, shape, strides,
flags)`. `data_ptr` points to a contiguous C-allocated buffer (or borrowed
memory). `strides` is a per-dim byte offset; multi-dim indexing is
`sum(idx[i] * strides[i])`. `flags` includes C-contiguous, F-contiguous,
writeable, owns-data, aligned.
**Key innovation**: Strided view model. Slicing produces a new ndarray that
shares the same buffer with adjusted shape/strides. Transpose is O(1) (swap
strides). Broadcast operates without materialization via stride-0 dims.
**Performance**: SIMD/auto-vectorization works on contiguous arrays; strided
access drops to scalar loops in NumPy's C kernels but BLAS (matmul, etc.)
absorbs strides directly. Universal-functions (`ufunc`s) are typed loops that
vectorize over contiguous chunks.
**C-API contract**: This is the part that drives the entire scientific Python
stack — anyone can produce a buffer that satisfies the ndarray contract
(`__array_interface__`, the Python buffer protocol, `__array__()`). Pandas,
SciPy, scikit-learn, PyTorch and JAX all interoperate by exposing this layout.
**References**:
- [NumPy: The N-dimensional array](https://numpy.org/doc/stable/reference/arrays.ndarray.html)
- [ajcr: An Illustrated Guide to Shape and Strides](https://ajcr.net/stride-guide-part-1/)
- [NumPy C-API strided loop](https://runebook.dev/en/docs/numpy/reference/c-api/array/c.NPY_METH_unaligned_strided_loop)

### 3.3 Apache Arrow

**What it is**: A language-neutral columnar memory format spec + a family of
implementations (C++, Java, Rust, Python, Go, JS, R, Julia, Ruby, etc.).
**Layout**: For each column: a validity bitmap (1 bit per row, 0 = NULL), a
values buffer (typed primitive or offsets-into-bytes for variable-width), and
optional child arrays (for nested types). All buffers are 64-byte aligned for
SIMD friendliness. Variable-width strings are an offsets array + a single byte
buffer; null entries get a duplicate offset (zero-length).
**Key innovation**: Spec is the contract — any language can produce or consume
Arrow buffers identically, enabling true zero-copy crossing of FFI boundaries.
The C Data Interface defines `ArrowArray`, `ArrowSchema`, `ArrowArrayStream`
structs in plain C with a release-callback memory-management protocol.
**Performance**: Columnar layout = perfect for SIMD scans, predicate
vectorization, dictionary encoding. Most analytic engines (DuckDB, Polars,
Spark via DataSource v2 + Arrow, Pandas 2 with PyArrow backend) now use Arrow
internally or as their interchange.
**Ecosystem maturity**: State of the art for cross-language analytics as of
2025-2026. Arrow Flight = network protocol for high-throughput Arrow IPC.
Arrow-DataFusion = a Rust SQL engine over Arrow.
**References**:
- [Arrow Columnar Format spec](https://arrow.apache.org/docs/format/Columnar.html)
- [Arrow C Data Interface](https://arrow.apache.org/docs/format/CDataInterface.html)
- [Arrow C Data Interface intro blog](https://arrow.apache.org/blog/2020/05/03/introducing-arrow-c-data-interface/)

### 3.4 Mojo `Tensor` / `SIMD` / `List`

**Tensor**: Shape-typed multidimensional array with MLIR-backed operations.
Generates kernels via Mojo's KGEN (kernel generator) framework on top of MLIR.
Both CPU SIMD and GPU paths (NVIDIA Hopper/Ampere + AMD MI300 since June 2025).
**SIMD**: Mojo's `SIMD[T, width]` is a first-class type. All numeric scalars
*are* `SIMD[T, 1]`. Element-wise comparisons return `SIMD[Bool, width]`.
**v0.25.6 (Sept 2025)**: comparison operators now return single `Bool` (not
mask) by default; explicit `eq()`/`le()` for masks; SIMD became
`EqualityComparable` so it can key dicts/sets.
**List**: Dynamic growable container; in 2025 became stricter about implicit
copies (warning → hard error in next release after v0.25.6).
**InlineArray**: Compile-time fixed-size; lives on the stack; pure value type.
**Performance**: Auto-vectorizes via `vectorize` higher-order — maps a kernel
across `[0, size)` in `simd_width` chunks with a remainder loop.
**Evolution since 2024**: Major shift toward GPU support, ownership-strict
collections, and MLIR-native HPC kernels (SC '25 workshop paper).
**References**:
- [Mojo SIMD docs](https://docs.modular.com/mojo/stdlib/builtin/simd/SIMD/)
- [Mojo InlineArray](https://docs.modular.com/mojo/std/collections/inline_array/InlineArray/)
- [v0.25.6 changelog](https://docs.modular.com/mojo/changelog/v0.25.6/)
- [Mojo: MLIR-Based HPC Science Kernels (arxiv:2509.21039)](https://arxiv.org/pdf/2509.21039)

### 3.5 Roc lists

**Layout**: A refcounted `Vec<T>` with prefix refcount. Variable-length, owned.
**Reuse**: Same Morphic-driven analysis as strings. Functional-style
`List.map` / `List.set` / `List.append` mutate in place when input is unique.
Seamless slices share buffer with parent (parent and slice both ineligible for
in-place mutation).
**References**: same as §1.12.

### 3.6 Vale arrays

**Layout**: Either heap or stack-allocated; supports inline structs in arrays
(no boxing). Refs are *generational references* — 8-byte pointer + 8-byte
remembered-generation; the object header has a current-generation counter that
increments on free. Each dereference checks the two generations match.
**Region allocation**: Arrays can live in regions; immutable region borrowing
elides generation checks within the borrowing scope. Arrays support partial-fill
state.
**Performance**: ~10.84% overhead in benchmarks (Vale's terrain generator)
versus unsafe code — about half the cost of naive RC.
**Why interesting**: Generational references are an alternative to both GC and
borrow-checking that gives memory safety with predictable cost. Regions add
zero-overhead "I promise not to mutate this" guarantees.
**References**:
- [verdagon.dev: Generational References](https://verdagon.dev/blog/generational-references)
- [verdagon.dev: Zero-Cost Borrowing with Vale Regions, Part 1](https://verdagon.dev/blog/zero-cost-borrowing-regions-part-1-immutable-borrowing)
- [Vale Linear-Aliasing Model](https://vale.dev/linear-aliasing-model)

### 3.7 Swift `Array` / `ArraySlice`

**Layout**: `Array<T>` is a struct holding a single reference to a heap buffer
(class-typed `_ContiguousArrayBuffer`). The buffer holds the elements
contiguously plus a header with count/capacity.
**CoW semantics**: On any mutation, `isKnownUniquelyReferenced(&buffer)` is
checked; if refcount is 1, mutate in place; otherwise allocate-and-copy. This
gives value semantics with zero copies in the common case (single owner).
**`ArraySlice<T>`**: A view into a parent's buffer with offset + count. Mutations
on a slice with refcount 1 stay in place; the slice prolongs the parent buffer's
lifetime even past the parent variable's death. Mutating-slice CoW had known
subtleties (Swift Forums has multiple threads on it).
**References**:
- [swift/docs/Arrays.md](https://github.com/swiftlang/swift/blob/main/docs/Arrays.md)
- [Jared Khan: Swift's CoW Optimisation](https://jaredkhan.com/blog/swift-copy-on-write)
- [Solving the mutating slice CoW problem (Swift Forums)](https://forums.swift.org/t/solving-the-mutating-slice-cow-problem/35297)

### 3.8 Rust `Vec<T>`

**Layout**: 24 bytes (ptr + len + cap). Owned, growable. `&[T]` and `&mut [T]`
are 2-word borrows.
**No CoW**: Rust deliberately doesn't have CoW on `Vec` — assignment moves
ownership, `clone()` copies. Sharing across threads = `Arc<Vec<T>>` (immutable
sharing) or `Arc<Mutex<Vec<T>>>` (shared mutable). `Arc<[T]>` is a refcounted
slice (just length + payload after the ref count, single allocation).
**`Cow<T>`** (the type) exists for "borrowed-or-owned" scenarios but it's
explicit — a user opts in.
**Why this design**: Ownership + borrowing already eliminates the need for CoW
in most cases — you almost always know whether you have unique ownership at
compile time. `Arc<[T]>` covers the "shared, immutable, possibly-from-anywhere"
case that Swift CoW handles by refcount.
**References**:
- [Rust docs: Arc](https://doc.rust-lang.org/std/sync/struct.Arc.html)
- [Rust RFC 1845: shared-from-slice](https://rust-lang.github.io/rfcs/1845-shared-from-slice.html)

### 3.9 OCaml arrays

**Layout**: A standard OCaml block. Elements are 1-word values (tagged ints
unboxed; pointers for everything else). Header carries length + tag.
**Float-array specialization**: When the array's static type is exactly
`float array` and the context isn't polymorphic, OCaml uses
`Double_array_tag = 254`. Floats are stored *unboxed* directly in the block —
8 bytes per element instead of 24 (boxed float = 3 words = ptr + header + data).
3-4x speedup for numeric code.
**Performance**: Generic 1-word-per-element model is slow for compact numeric
data. The float-array trick is the workaround. OCaml 5 + the Jane Street
"unboxed types" project is extending unboxed-element arrays to more types.
**References**:
- [LexiFi: About unboxed float arrays](https://www.lexifi.com/blog/ocaml/about-unboxed-float-arrays/)
- [Jane Street: Unboxed Types for OCaml](https://www.janestreet.com/tech-talks/unboxed-types-for-ocaml/)

---

## 4. Direct memory access patterns

### 4.1 JIT bound-check elision

**V8 (TurboFan / Maglev)**: Range-analysis-driven elimination. If the
TurboFan/Maglev range type for the index proves it's in `[0, length)`, the
check is removed. Loop-invariant code motion hoists the `length` load and the
check itself out of loops where possible.
**HotSpot JVM**: *Loop predication* — duplicates the loop into a fast version
without checks plus a slow version with checks; runs the fast version for
iterations the analysis proved safe and falls into the slow loop only for
boundary iterations. Failing speculation deoptimizes back to the interpreter.
For counted loops with trivially-bounded indices, the checks are entirely
hoisted out.
**Cranelift**: Has a basic bounds-check elimination pass for WebAssembly heap
accesses; uses Spectre-mitigation conditional moves for the remaining checks
(speculative-execution-safe). Proof-Carrying-Code (PCC, an experimental
feature) lets a frontend embed `PointsTo` facts on values; checked loads/stores
are then statically verified.
**Big picture**: The same problem at three levels of sophistication —
TurboFan/HotSpot have decades of range-analysis work; Cranelift is younger but
has the unusual angle of formal verification (PCC) at compile time.
**References**:
- [Red Hat: Range check elimination in HotSpot](https://developers.redhat.com/articles/2022/03/16/range-check-elimination-loops-openjdks-hotspot-jvm)
- [Cranelift docs: ir.md](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md)
- [Cranelift Progress 2022 (BCA)](https://bytecodealliance.org/articles/cranelift-progress-2022)

### 4.2 SIMD intrinsics

**Rust `core::simd` / `std::simd`**: Portable SIMD (`Simd<T, N>`) — nightly-only
as of 2025-2026 but widely used. `wide` and `pulp` are the stable-friendly
alternatives. Most arch-specific intrinsics in `core::arch::*` became safe in
1.87.
**Mojo SIMD**: First-class — `SIMD[T, N]`; scalars *are* `SIMD[T, 1]`. The
`vectorize` higher-order generates SIMD loops with remainder.
**Julia broadcasting**: `.+` / `.*` etc. fuse element-wise operations through
LLVM and SIMD-vectorize automatically for `isbits`/contiguous arrays. Explicit
`@simd` annotation, plus the SIMD.jl + LoopVectorization.jl packages.
**Cranelift**: SIMD types are first-class IR (`i8x16`, `i32x4`, `f64x2` etc.);
ISLE rules lower them to SSE/AVX/NEON/etc. Wasm SIMD is fully supported on
x86-64 and aarch64.
**WebAssembly**: 128-bit fixed SIMD (released; widely supported); "Relaxed
SIMD" extension finalizes some platform-dependent operations; "Wasm SIMD2"
(384/512-bit) and Flexible-Vector proposals in flight.
**References**:
- [Rust portable-simd](https://github.com/rust-lang/portable-simd)
- [Shnatsel: State of SIMD in Rust 2025](https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d)
- [Cranelift IR docs (SIMD)](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/docs/ir.md)
- [Mojo vectorize](https://docs.modular.com/mojo/stdlib/algorithm/functional/vectorize/)

### 4.3 Cache-friendly traversal — prefetching, blocking, tiling

**Tiling/blocking**: Partition an iteration space into tile-sized chunks so a
working set fits in L1/L2. Crucial for matmul, stencils, image filters.
**Compiler responsibility**: Polyhedral compilers (Polly, MLIR's affine
dialect, ISL) can do automatic tiling. LLVM's loop-nest optimizer handles
simple cases. Most production code still gets hand-tuned tile sizes.
**Programmer responsibility**: Choosing tile sizes (cache-aware vs. cache-
oblivious algorithms — the latter avoids tile-size choice entirely by recursive
decomposition).
**Prefetching**: Hardware prefetchers handle linear / stride-1 well; complex
patterns benefit from `__builtin_prefetch` / `_mm_prefetch`. Compilers rarely
auto-insert software prefetch.
**Where the work lives**: For numeric code, BLAS libraries (OpenBLAS, MKL,
BLIS) are still the reference — not auto-vectorized C; they're written in
hand-tuned assembly with fine cache and ILP control. MLIR (and Mojo via
MLIR) is the closest thing to "compile high-level kernels into BLAS-quality
code" in practice.
**References**:
- [Wikipedia: Loop nest optimization](https://en.wikipedia.org/wiki/Loop_nest_optimization)
- [Wikipedia: Cache-oblivious algorithm](https://en.wikipedia.org/wiki/Cache-oblivious_algorithm)
- [Codee: Loop tiling glossary](https://open-catalog.codee.com/Glossary/Loop-tiling/)

### 4.4 Strided vs contiguous

**Contiguous wins**: SIMD vectorization (one wide load), hardware prefetch
prediction, BLAS Level-3 routines, kernel auto-vectorization.
**Strided wins**: Zero-copy views (transpose, slice, reshape), columnar
analytics on row-major source data, virtual reshape without realloc.
**How multi-dim arrays choose**: NumPy / Julia / Fortran / R / MATLAB all
support strided. But operations on contiguous fast-path; strided falls into a
slower kernel. Most libraries materialize a contiguous copy when chained
operations would otherwise repeatedly pay stride cost (`np.ascontiguousarray`).
**Measured**: NumPy column-mean over column-major matrix can be 6× faster
than over row-major for the column-aligned case (and vice versa) — direct
consequence of stride alignment to access pattern.
**References**:
- [Wikipedia: Row- and column-major order](https://en.wikipedia.org/wiki/Row-_and_column-major_order)
- [Modular: Row-major vs. Column-major in Mojo and NumPy](https://www.modular.com/blog/row-major-vs-column-major-matrices-a-performance-analysis-in-mojo-and-numpy)
- [Eli Bendersky: Memory layout of multi-dim arrays](https://eli.thegreenplace.net/2015/memory-layout-of-multi-dimensional-arrays)

---

## 5. Reuse analysis (in-place updates)

### 5.1 Perceus / FBIP / FIP — Koka and Lean 4

**Perceus** (PLDI 2021, Reinking/Xie/de Moura/Leijen): A precise reference-
counting algorithm for languages with explicit control flow that emits exactly
the inc/dec instructions needed. Key trick: pair each `dec` with a *reuse* slot
of the same shape; if the dec drops to 0, recycle the memory in place.
**FBIP** (Functional But In-Place): Coding style enabled by Perceus where pure
functional programs perform in-place updates whenever inputs are unique. Like
TCO for memory. Used in Koka.
**FP² / FIP** (ICFP 2023, Lorenzen/Leijen/Swierstra/Lindley): A *fully* in-place
calculus where, given a discipline (no allocation, constant stack), pure
functional programs are guaranteed to use no extra memory at all. Algorithms
expressible: splay trees, finger trees, merge sort, quicksort.
**Lean 4**: Uses Perceus-style RC. Pure code performs destructive updates on
unshared values transparently.
**Production status**: Koka is research; Lean 4 is in production use (theorem
proving). The technique is real and shipping.
**References**:
- [Perceus paper (PLDI 2021)](https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/)
- [FP²: Fully in-Place Functional Programming (ICFP 2023)](https://dl.acm.org/doi/10.1145/3607840)
- [FP² PDF](https://www.microsoft.com/en-us/research/wp-content/uploads/2023/05/fbip.pdf)
- [Trends in FP: Benchmarking a Baseline FIP Compiler](https://trendsfp.github.io/papers/tfp26-paper-12.pdf)

### 5.2 Roc's Morphic

**What it is**: A whole-program type-level analysis (defunctionalizing closures
and tracking "modes" — borrowed vs. owned vs. moved usage). Output is fed into
the back-end so the runtime can choose in-place mutation when an argument is
provably unique.
**Production state**: Roc is in active development (2026); Morphic is integral
to the compiler. Combined with Roc's Perceus-influenced RC scheme. Has known
implementation challenges around type inference and lambda-set specialization
(Roc issue #5969).
**Outcome**: Functional code with apparent value-semantics gets mutation
performance on the unique-input fast path, with seamless slices flagged as
sharing-introducing.
**References**:
- [Roc Issue #5969 (Lambda Sets / Morphic)](https://github.com/roc-lang/roc/issues/5969)
- [Reference Counting with Reuse in Roc (thesis)](https://studenttheses.uu.nl/handle/20.500.12932/44634)
- [Better Defunctionalization through Lambda Set Specialization (paper)](https://www.roc-lang.org/) (linked from Roc compiler docs)

### 5.3 Lobster's compile-time RC elision

**What it is**: A static-analysis pass that picks an "owner" for each
allocation (typically the first variable/field/element it's assigned to);
thereafter every use is a "borrow" with no RC ops. Combined with Lobster's
type system to specialize over ownership.
**Production state**: Used in real game-development workloads. Author Wouter
van Oortmerssen reports ~95% of RC operations removed at compile time across a
corpus of dozens of game prototypes; only minor source changes needed when
moving from runtime-RC to compile-time RC.
**References**:
- [aardappel.github.io: Memory Management in Lobster](https://aardappel.github.io/lobster/memory_management.html)
- [Lobster Issue #169: RC with less than half the overhead](https://github.com/aardappel/lobster/issues/169)
- [Compile time RC & Lifetime Analysis in Lobster (talk)](https://www.youtube.com/watch?v=WUkYIdv9B8c)

### 5.4 Linear Haskell + arrays

**What it is**: GHC's `-XLinearTypes` extension (released with GHC 9.0)
introduces multiplicity annotations: `f :: A %1 -> B` means `f` consumes
exactly one `A`. The `linear-base` library uses this to expose pure linear
APIs over mutation: `Data.Array.Mutable.Linear`, `Data.HashMap.Mutable.Linear`,
linear file I/O, linear sockets.
**Pattern**: Caller `alloc`s an `Array a`, runs a linear computation
`Array a %1 -> Ur b`, and gets back an `Ur b` (unrestricted result). The
linear discipline statically guarantees no extra references — so the runtime
can mutate in place without any check.
**Production state**: Used in industry (Tweag, MangoIV, etc.); not as
mainstream as `Maybe` but well-tested. Performance approaches imperative
languages for the kernels expressible in linear style.
**References**:
- [Data.Array.Mutable.Linear (Hackage)](https://hackage.haskell.org/package/linear-base/docs/Data-Array-Mutable-Linear.html)
- [tweag: linear-base makes Linear Haskell easy](https://www.tweag.io/blog/2021-02-10-linear-base/)
- [Linear Haskell paper (ICFP 2018)](https://arxiv.org/pdf/1710.09756)

---

## 6. Zero-copy and FFI

### 6.1 Apache Arrow as FFI

**The C Data Interface**: A small set of plain C structs (`ArrowArray`,
`ArrowSchema`, `ArrowArrayStream`) with a release-callback memory-management
protocol. Buffers are passed by pointer; ownership transfers via the callback.
**No build-time coupling**: Any language with a C FFI (Python via ctypes,
Julia via `@ccall`, Rust via `extern "C"`, Go via cgo, R via `.Call`) can
participate without linking to Arrow itself.
**Used in practice**: Pandas ↔ DuckDB, Polars ↔ DuckDB, R Arrow ↔ Python
Arrow, Spark ↔ Pandas UDFs (via Arrow IPC), Snowflake ↔ Python connector.
**Device variants**: An Arrow C Device Data Interface adds a device-id field
so GPU/accelerator buffers can be passed across the same interface.
**References**:
- [Arrow C Data Interface intro](https://arrow.apache.org/blog/2020/05/03/introducing-arrow-c-data-interface/)
- [Arrow C Device Data Interface](https://arrow.apache.org/docs/format/CDeviceDataInterface.html)

### 6.2 Project Panama Foreign Memory API (JEP 454)

**JEP timeline**: 424 (preview, JDK 19) → 434 (preview, JDK 20) → 442
(third preview, JDK 21) → **454 (final, JDK 22)**.
**API shape**: `MemorySegment` (address + size + scope), `Arena` (allocates
segments and bounds their lifetime), `ValueLayout` (typed view of native
memory), `Linker` (calls native functions). Off-heap memory is bounded by
`Arena.close()` — guaranteed cleanup.
**vs. JNI**: No boilerplate native-side bridge code; method handles are JIT-
inlined; safety enforced via `Scope`/`Arena` (use-after-free detected at
runtime via segment validity checks).
**Performance**: Comparable to or better than JNI in many benchmarks;
specifically wins on small-call overhead.
**References**:
- [JEP 454: Foreign Function & Memory API](https://openjdk.org/jeps/454)
- [JEP 442 (Third Preview)](https://openjdk.org/jeps/442)
- [InfoQ: FFM API to bridge Java/native](https://www.infoq.com/news/2023/10/foreign-function-and-memory-api/)

### 6.3 WASM memory model

**Linear memory (MVP)**: One contiguous `i32`-addressed byte array per module
(or multiple, with the multi-memory proposal). All loads/stores go through it
explicitly. No inherent boxing or GC. Compiled languages place their entire
heap inside.
**Reference types**: Adds `funcref` and `externref` — opaque host references
not stored in linear memory; lifetimes managed by the host runtime.
**Wasm-GC** (proposal, in Wasm 3.0 as of 2025): Adds `struct` and `array` heap
types with typed references (`(ref $T)`). VM manages lifetimes (host GC).
Avoids the cost of bringing a custom GC compiled into linear memory (typical
size cost: 1-2 MB per module before GC).
**Typed arrays**: Wasm-GC has `array` heap type (mutable typed arrays of
elements). Element types include packed (`i8`, `i16`) and non-packed
(`i32`/`i64`/`f32`/`f64`/`anyref`). Indexing is bounds-checked; the VM picks
the heap layout.
**Status (May 2026)**: Wasm 3.0 finalized in Sept 2025 includes Wasm-GC.
Chrome ships it by default; SpiderMonkey supports it; full toolchain support
in Kotlin/Wasm and Dart. Not yet ideal for realtime graphics (HN reports).
**References**:
- [WebAssembly GC proposal Overview](https://github.com/WebAssembly/gc/blob/main/proposals/gc/Overview.md)
- [V8: WasmGC porting blog](https://v8.dev/blog/wasm-gc-porting)
- [Chrome: WasmGC enabled by default](https://developer.chrome.com/blog/wasmgc)
- [Wasm 3.0 Completed](https://webassembly.org/news/2025-09-17-wasm-3.0/)

---

## 7. Novel / 2024-2026 frontier work

### 7.1 Small string / string-pool design

- **`compact_str` 0.8+ niches** (Rust ecosystem): inline form discriminated by
  the high bit of length, allowing the entire 24-byte struct to be inline
  (24-byte SSO with no extra discriminator). Standard reference for "modern"
  SSO design.
- **`ecow`** (Rust, succeeded `smol_str`): 16-byte type with 15-byte SSO and
  refcounted CoW heap mode; one of the smallest "everything" types.
- **`hipstr`** (Rust): Tri-mode (inline / refcounted / borrowed-`'static`)
  string in 24 bytes; very general but heavier branching per op.
- **CPython 3.13+** PEP 756 (`PyUnicode_Export` / `PyUnicode_Import`): A
  stable, string-rep-agnostic export API — third-party libs can hand a
  utf8/utf16/utf32 buffer to CPython without knowing internal representation.
**References**:
- [PEP 756 (PyUnicode_Export/Import)](https://github.com/python/cpython/issues/119609)
- [docs.rs/compact_str](https://docs.rs/compact_str)
- [crates.io/ecow](https://crates.io/crates/ecow) (search/HN)

### 7.2 Automatic SoA/AoS — compiler-driven layout

- **Annotation-guided AoS-to-SoA in C++** (Radtke et al., PPAM 2024 →
  Concurrency & Computation 2025, arxiv:2502.16517): A C++ attribute / Clang
  plugin that converts AoS structures to SoA views over kernel scopes,
  including GPU offloading. The compiler does the conversion transparently;
  user marks which struct members the kernel reads.
- **arxiv:2512.05516 (2026)**: "Compiler-supported reduced precision and AoS-
  SoA transformations for heterogeneous hardware" — extends the above to mixed-
  precision and accelerator-targeted layouts.
- **Mojo / MLIR**: Layout transformations are first-class IR ops in MLIR's
  `linalg` and `tensor` dialects; AoS↔SoA is just a layout attribute on a
  tensor type. Mojo benefits from this through KGEN.
**References**:
- [arxiv:2502.16517 — Annotation-guided AoS-to-SoA](https://arxiv.org/html/2502.16517v1)
- [arxiv:2512.05516 — Compiler-supported precision/layout](https://arxiv.org/html/2512.05516v1)

### 7.3 Region-aware arrays / regions in modern languages

- **Vale regions** (2024 onward): "Zero-Cost Borrowing with Vale Regions" blog
  series; immutable region borrowing reaches RC-comparable safety with no
  per-deref overhead within the borrow scope.
- **Modal OCaml** (Jane Street, ICFP 2024): "Oxidizing OCaml with Modal Memory
  Management" — adds `local_` / `unique_` / `once_` modes. Local arrays live
  on a region (stack-like); unique arrays support in-place mutation (Linear-
  Haskell-flavored).
- **Koka** continues to refine the FBIP discipline with FIP (Fully In-Place)
  guarantees; benchmarking infrastructure published at TFP 2026.
**References**:
- [verdagon.dev: Zero-Cost Borrowing Regions Part 1](https://verdagon.dev/blog/zero-cost-borrowing-regions-part-1-immutable-borrowing)
- ["Oxidizing OCaml with Modal Memory Management" (ICFP 2024)](https://dl.acm.org/doi/10.1145/3674642) — Jane Street modal-types work
- [Trends in FP 2026: FIP Compiler Benchmarking](https://trendsfp.github.io/papers/tfp26-paper-12.pdf)

### 7.4 Mojo array evolution since 2024

- **Pre-2024**: `Tensor` was the headline type but heavyweight; `DynamicVector`
  / `InlinedFixedVector` / `UnsafeFixedVector` proliferated.
- **2024**: Consolidation around `List`, `InlineArray`, `Tensor`. SIMD scalar-
  unification ("scalars are SIMD[T,1]") becomes the default.
- **June 2025**: AMD MI300 GPU support added — same Mojo source compiles to
  NVIDIA + AMD via MLIR; vendor-agnostic GPU portability is a Mojo
  differentiator.
- **Sept 2025 (v0.25.6)**: SIMD comparison operators returned to producing
  `Bool` instead of mask (with explicit `eq`/`le` for masks); SIMD now
  `EqualityComparable`. `List`/`Dict` implicit copies become warnings →
  hard errors next release.
- **Nov 2025 (SC '25 workshop)**: HPC science kernels paper demonstrating
  Mojo's MLIR pipeline narrows the Python-vs-C++ gap in production HPC.
**References**:
- [Mojo v0.25.6 changelog](https://docs.modular.com/mojo/changelog/v0.25.6/)
- [arxiv:2509.21039 — Mojo HPC Science Kernels](https://arxiv.org/pdf/2509.21039)
- [Mojo + AMD MI300 announcement (DeepEngineering)](https://deepengineering.substack.com/p/deep-engineering-9-unpacking-mlir)

---

## 8. Cross-cutting findings

### 8.1 Convergence: where most production runtimes agree

- **SSO is the default** for value-typed strings. The only widely-used types
  *without* SSO are `std::string` historically (yes — copy-on-write was banned
  by C++11; SSO is now ubiquitous in libc++, libstdc++, MSVC) and Rust's
  `String` (intentional — niche-filled by ecosystem crates). Inline capacity
  varies: 15 (libstdc++, Swift, Java pre-Compact, ecow, kstring), 22-23
  (libc++, smol_str/ecow successor, smartstring, Mojo), 24 (compact_str's
  niche-using design).
- **Latin-1/ASCII fast path**: Java, CPython, JSC, V8 all distinguish 1-byte
  vs. 2/4-byte storage. The "most strings are short and ASCII" assumption
  drives memory savings of 50%+ in real heaps.
- **Lazy concat for high-throughput JS**: V8 ConsString and JSC JSRopeString
  both lazy-concat. Threshold ~13 chars to avoid trivial-cons overhead.
- **UTF-8 is winning over UTF-16**: Swift 5 switched to UTF-8 (2019); Mojo is
  UTF-8; Rust always was; Go always was. Java + JS remain UTF-16-flavored
  internally for legacy reasons. The trend is clear.
- **Columnar + strided + zero-copy** is the unanimous answer for analytical
  arrays. NumPy's strided model + Arrow's columnar layout + buffer-protocol
  FFI is the lingua franca.
- **CoW for value-typed arrays** (Swift `Array`, Roc lists, Mojo `String`)
  is *de facto* standard for high-level languages. Rust is the holdout
  (ownership eliminates the need).
- **SIMD as first-class type** (Mojo `SIMD[T,N]`, Rust `Simd<T,N>`, Julia
  via SIMD.jl + LLVM) is the modern design. Older intrinsic-only APIs
  (`__m256`, NEON intrinsics) survive but are wrapped.
- **Reuse analysis is moving from research to production**: Lean 4, Roc,
  Koka, Lobster, GHC + linear-base. The technique works; the question is
  ergonomics.

### 8.2 Divergence: open questions

- **Interning yes/no**: LuaJIT interns everything globally; JVM intern is
  opt-in; Crystal interns symbols but not strings; Rust interns nothing in
  std. The right answer is workload-dependent, and language designers
  consistently disagree.
- **SoA vs. AoS**: Static (Rust struct-of-arrays libraries like `soa_derive`),
  hybrid (Mojo `@parameter`), automatic (Clang annotation-guided AoS-to-SoA),
  or compiler-driven (MLIR layouts). No consensus; each ecosystem's answer
  ties to its lowering pipeline.
- **Grapheme-cluster vs. code-unit indexing**: Swift's default-grapheme
  iteration is correct-but-expensive; Rust's byte-indexing is fast-but-
  surprising; Python's code-point indexing is a middle ground (still wrong
  for graphemes, but at least Unicode-aware).
- **String length cost**: O(1) (length stored explicitly — the universal
  choice) vs. O(n) (C-style null termination — only C and C++ at the
  language level; even there, `std::string` caches length).
- **Refcount placement**: Prefix-of-buffer (Swift, Roc, Mojo refcounted form,
  Arc<T>) vs. side-table (some research GCs) vs. inside the value
  (generational refs, Vale). Convergence on prefix.
- **In-place mutation in functional languages**: Linear types (Linear Haskell,
  modal OCaml), uniqueness types (Clean, Idris2), reuse analysis (Roc,
  Koka, Lean 4), or runtime RC checks (Swift CoW). All work; ergonomic
  tradeoffs differ.
- **GC in Wasm**: linear memory + custom GC (legacy) vs. Wasm-GC (typed
  references, host GC). Wasm-GC just shipped (Sept 2025) — adoption is
  still mid-transition.

### 8.3 2024-2026 frontier work

- **FP² / FIP** (Lorenzen et al., ICFP 2023, with 2026 benchmarking work):
  proves *fully* in-place is achievable at the type-system level for purely
  functional code; merge sort, quicksort, splay trees all expressible.
- **Modal OCaml** (Jane Street, ICFP 2024): brings Rust-like
  ownership/uniqueness modes to OCaml's type system without breaking the
  existing runtime — production-aimed.
- **Annotation-guided AoS↔SoA** (PPAM 2024 / CCPE 2025 / arxiv 2502.16517):
  C++/Clang plugin makes layout transformation a per-kernel choice with
  zero source disturbance.
- **Mixed-precision + layout transformations** (arxiv 2512.05516, 2026):
  unified compiler support for precision and AoS-SoA conversion across CPU
  and accelerators.
- **Wasm 3.0 / Wasm-GC** (Sept 2025): typed `array`/`struct` references in
  Wasm are now a baseline browser feature.
- **Vale regions, Mojo MLIR HPC** (SC Workshops 2025): production-leaning
  region-aware arrays and MLIR-driven HPC kernels narrow the gap to C++ +
  BLAS.
- **PEP 756** (CPython): stable C-API string export/import
  representation-agnostic.
- **Mojo SIMD-as-scalar** + `Bool`-returning comparisons (v0.25.6): smaller
  surface area, more trait-friendly for collections.

---

## Appendix: quick reference table

| Topic | Default in language | Inline cap (bytes) | Lazy concat | Refcount | Grapheme-aware |
|---|---|---|---|---|---|
| CPython 3.3+ | PEP 393 (1/2/4-byte fixed) | none (header ≥56 B) | no | no | no (code-point) |
| V8 SeqString + ConsString | UTF-16 (or 1-byte) | none | yes (≥13) | no (atom intern) | no |
| JSC JSString | StringImpl (1/2-byte) | none (16 B JSString) | yes (rope) | yes | no |
| Swift String | UTF-8 + flags | 15 | no | yes (CoW heap form) | yes (default) |
| Java 9+ | byte[] + coder | none | no | no (intern opt-in) | no |
| Rust String | Vec<u8> UTF-8 | none | no | no | no (byte) |
| compact_str | UTF-8 niche-disc | 24 | no | no | no |
| smol_str | UTF-8 + Arc | 22 | no | yes (heap) | no |
| ecow | UTF-8 + RC CoW | 15 | no | yes | no |
| C++ libc++ | CharT* | 22 | no | no | no |
| C++ libstdc++ | CharT* | 15 | no | no | no |
| Mojo String | UTF-8 + RC CoW | 23 | no | yes | no |
| OCaml string | bytes block | none | no | no | no |
| LuaJIT GCstr | UTF-? heap | none | no | global intern | no |
| Erlang binary | refc + sub | n/a | yes (subbinary) | yes | no |
| Roc Str | Vec<u8> + RC | none | no | yes | no |

(Inline-cap is for the value type itself; languages that always heap-allocate
are noted "none". RC = reference count.)

---
