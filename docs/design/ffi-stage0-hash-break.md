# FFI Stage-0 Hash Break (WF-2A stage 0)

**Status:** landed on `wave2/ffi-stage0`.
**Scope:** the single, ratified, one-time content-hash / blob-format break that
the Q1 design mandates land **before** any persisted store
(`SnapshotStore`, `RemoteBlobCache`) exists. It gates WF-2A stages 1–7,
WF-2B, and WF-2C.

Pre-1.0, there are no persisted stores in the wild, so exactly one hash
invalidation event is acceptable. After this workflow **all content hashes are
FINAL** — see [Finality](#finality).

This document is the durable record of *what changed in the hash inputs* and the
*old → new hashes* of representative samples, computed with the workspace's exact
dependency versions (`rmp-serde 1.3.1`, `sha2 0.10.9`, `serde 1.0.228`).
A self-contained reproducer lives at `scratchpad/hashrepr/` (two binaries:
`hashrepr` for the `FunctionBlob` msgpack construction, `fentry` for the
`ForeignFunctionEntry` manual construction).

---

## 1. Two independent hash constructions

Shape has two content-hash constructions, both broken (deliberately) by this
workflow:

| Construction | Where | Encoding |
|---|---|---|
| `FunctionBlob::compute_hash` | `crates/shape-vm/src/bytecode/content_addressed.rs` | `rmp_serde::encode::to_vec(FunctionBlobHashInput)` (struct-as-array) → SHA-256 |
| `ForeignFunctionEntry::compute_content_hash` | `crates/shape-vm/src/bytecode/core_types.rs` | manual `\0`-separated field feed → SHA-256 |

---

## 2. What changed

Four break sources, plus one hash-neutral change recorded for completeness.

### BREAK A — `FrameDescriptor` + `capture_kinds` into `FunctionBlobHashInput` (distributed §4.8 / OQ-6)

`FunctionBlobHashInput` grew from **17 → 19** serialized fields. Two typed
fields were inserted **after `mutable_captures`**, matching the `FunctionBlob`
field order:

- `frame_descriptor: &Option<FrameDescriptor>` — the typed frame layout
  (per-slot `NativeKind` + ABI return kind) that the remote-marshal path
  trusts. Hash-covered so a tampered/divergent descriptor cannot hide behind a
  matching hash.
- `capture_kinds: &[NativeKind]` — per-capture proven `NativeKind`, in
  declaration order. Capture layout is call-ABI identity exactly like param
  kinds: a closure that reads capture 0 as `Float64` is a *different function*
  from one that reads it as `Ptr(TypedObject)`.

Both carry **typed** `NativeKind` / `FrameDescriptor` data — never raw bits or a
Bool-default (CLAUDE.md §Forbidden Patterns). Because these fields are present
in **every** blob (as `None` / empty for non-closures), BREAK A alone changes
**every** blob content hash.

### BREAK B — A6: blob-local `CallForeign` ordinals + first-use-deduped `foreign_dependencies` (integration §4.2.0)

Fixes soundness hole C10 (program-level foreign index leaking into blob
identity) and enables cross-program blob dedup:

- `foreign_dependencies` is now **ordered, first-use-deduped** (was
  `sort()` + `dedup()`).
- Every `CallForeign` operand in a blob's instruction stream is rewritten from
  the program-level foreign index to the **blob-local ordinal** (position of
  that entry's content hash in this blob's `foreign_dependencies`). The linker
  (`crates/shape-vm/src/linker.rs`) inverts ordinal → hash → assembled-table
  index at every consuming node, with structured `LinkError`s
  (`ForeignOrdinalOutOfRange` / `MissingForeignEntry`) replacing index panics on
  a self-consistent-but-malformed received blob.

Both the hashed instruction stream and the hashed `foreign_dependencies`
sequence change for any blob that contains a `CallForeign`. Blobs without
foreign calls are unaffected by BREAK B (they are still moved by BREAK A).

### BREAK C — `is_async` + `param_names` into `ForeignFunctionEntry::compute_content_hash` (ffi §4.7)

Param names are visible to Python/TS bodies as binding names and `is_async`
changes the invoke integration — both are semantics-affecting foreign-function
identity and were omitted from the hash pre-stage-0. A domain-separated block is
appended **after the `param_types` run and before `return_type`**:

```
… \0names\0 (param_name \0)…  \0async\0 [is_async as u8] …
```

The `\0names\0` / `\0async\0` domain separators guarantee this block can never
be confused with the preceding `param_types` run or the following `native`
tail. BREAK C changes **every** foreign-entry content hash.

### BREAK D — A7: `NativeAbiSpec.library` stores the declared alias (integration §4.1 / §4.4.3)

The compile side no longer resolves the native-library alias into a
compile-host `soname`. It stores the **declared alias** verbatim (`"c"`, not
`"libc.so.6"`); resolution to a concrete path/soname is deferred wholly to the
executing host (link-now locally, load-verify on remote/resume). Identical
declarations now hash identically across compile hosts. The resolution chain is
retained (dead-code-gated) in `functions_foreign.rs` as the template the
executing host reuses in WF-2A stage 1+.

### A5(ii) — hash-derived foreign-return schema name (hash-neutral)

The anonymous foreign-return schema is now registered under a **hash-derived**
name `__ffi_h{hex16}_return` (first 16 hex chars of the entry's content hash)
instead of `__ffi_{fn_name}_return`. This makes the receiver's
`return_type_schema_id` re-stamp immune to name dedup / tampering and shares one
return schema between two aliases of one body. **Hash-neutral:**
`return_type_schema_id` is a registry-local numeric id excluded from
`compute_content_hash`, so the schema name reaches no hashed payload.

---

## 3. Old → new hashes (exact reproductions)

All values below were regenerated with the workspace's exact dependency
versions. The "before" values reproduce the pre-stage-0 constructions
byte-for-byte; the "after" values are what workspace `HEAD` now produces.

### 3a. `FunctionBlob` — TEST-VERIFIED real blob (authoritative)

The regression anchor `blob_with_perms({FsRead, FsWrite})` in
`crates/shape-vm/src/linker_tests.rs`
(`ffi_reservation_leaves_existing_content_hash_unchanged`) — a real
production-code blob: `name="wf1d_anchor"`, arity 1, one param `x`, one local, a
single `Halt` instruction, permissions `["fs.read", "fs.write"]`, no foreign
calls. Computed by the actual `FunctionBlob::compute_hash`, both before and
after, and pinned by a passing test:

| | content hash |
|---|---|
| **before** (main) | `2eb6e818552bfaf68df6ba02b43a6e29a2ab20e3601a2fc8bdd4fd800a5313e9` |
| **after** (this workflow) | `971ebf5f849c5a28a16a40a25d2b94e77f816fc70f4ed8bb9baccad60d15b56c` |

The delta is purely BREAK A (this blob has no `CallForeign`, so BREAK B does not
apply): the msgpack struct-array went 17 → 19 elements, and even this blob's
`None` `frame_descriptor` and empty `capture_kinds` shift the digest.

### 3b. `FunctionBlob` — minimal synthetic samples (`scratchpad/hashrepr/`, bin `hashrepr`)

Zero-arg, empty-body function. Empty vectors serialize element-type-agnostically
and a `None` option serializes to msgpack nil regardless of inner type, so these
are byte-exact reproductions of a real blob with the same field values.

| sample | before (17 fields) | after (19 fields) |
|---|---|---|
| `name="repr"` | `7db5e74fed9ed1dbc83d41a417f4ff426c6dbc9cf0d4edf07a1df405a3754804` (24 B) | `e81229647c321749b048d57b557d998f58ab32332328bb50c2be52b8cdf06c72` (26 B) |
| `name=""` | `144bd679f944e39ebbc718445d30bd1ebc0865be200f5667146a3ce467e8812c` (20 B) | `ce62cc5fbdc50dc18a79fbd0366aa70d73913825a3cf6becce75d2a25987f97c` (22 B) |
| `name="repr"` + perm `ffi.call` | `7a57e177a3a167931974ba4234f199f686631ef03f3090ae6a982079daea430e` (33 B) | `feac2125980835573e09a05b99a6c32d3c5c0896d622e4f58b8b2874b4ce9a62` (35 B) |

The +2 bytes per sample are exactly the inserted nil (`frame_descriptor: None`)
and array-0 (`capture_kinds: []`).

### 3c. `ForeignFunctionEntry` (`scratchpad/hashrepr/`, bin `fentry`)

**BREAK C** — python entry `fn python add(a: int, b: int) -> Result<int> { return a + b }`:

| | content hash |
|---|---|
| before (no names/async) | `1e2dcde95df05debfe9611bd3d418429661a3a27e2615eb4d0100720e6fe9b29` |
| after (`names=[a,b]`, `async=false`) | `5e967fa7b7d9d2f0b486f29fc88767796520307776bf7cf6456d1a66adf3f818` |

**BREAK D / A7** — extern-C `labs` entry, isolated so the delta is purely the
`library` string (construction held constant):

| | content hash |
|---|---|
| before (resolved soname `libc.so.6`) | `f5774b65c04ac7b24f55855d2e5d70fe010df90e1bcb57a2b646ef866ff6d46c` |
| after (declared alias `c`) | `3d8009b6b187353d3c22e85af63e327b463e9340fbcc24b302f9dc74adc464fa` |

For reference, the **full** post-stage-0 hash of a real
`extern C fn labs(x: int) -> int` in library `"c"` (BREAK C + BREAK D combined,
`names=[x]`, `async=false`) is
`f50e09c8726fd45be5ae296c9fb3bdf4dbeb8a801ef7a209fc4de91994ddf973`.

---

## 4. ABI + vtable (not a hash input, but part of the same landing)

- `ABI_VERSION` bumped **3 → 4** (`crates/shape-abi-v1/src/lib.rs`). The plugin
  loader refuses version-mismatched extensions, so a v4 host never dereferences
  a v3 vtable's shorter layout.
- `LanguageRuntimeVTable` grew a **strictly additive tail** appended after every
  v3 field: `runtime_descriptor` (fn ptr, `None` for now),
  `state_model: u32` (`STATE_MODEL_STATELESS_COMPILE_CACHE` = 0 /
  `STATE_MODEL_STATEFUL_OPAQUE` = 1), and four reserved fn-ptr padding slots for
  future additive vtable functions without another ABI bump. No existing field
  was reordered or removed.
- Extension-side **panic containment** (`catch_unwind` shells around every
  exported vtable entry) moved into the `language_runtime_plugin!` macro, which
  also stamps the conservative `STATE_MODEL_STATEFUL_OPAQUE` for the
  interpreter-backed runtimes it generates.
- `Permission::Ffi` (17th ordinal, `ffi.call`) was **already** reserved in
  WF-1D and is untouched here.

Both Python and TypeScript extensions build clean at ABI v4
(`just build-extensions`).

---

## 5. Every content hash is invalidated by this workflow

- **Every** `FunctionBlob` hash changes (BREAK A touches all blobs).
- **Every** `ForeignFunctionEntry` hash changes (BREAK C touches all foreign
  entries; BREAK D additionally moves native-ABI entries and makes them
  host-stable).

This is the intended single invalidation event: the whole content-address space
is renewed exactly once, here.

---

## Finality

After WF-2A stage 0, the **blob format** (`FunctionBlob` fields +
`FunctionBlobHashInput` field set + order) and **both hash constructions**
(`FunctionBlob::compute_hash`, `ForeignFunctionEntry::compute_content_hash`) are
**FINAL**.

- WF-2A **stages 1–7** wire *enforcement* (link-now / load-verify / capability
  gating) onto these already-final inputs — no further hash-input or
  blob-format change.
- **WF-2B** (snapshot/resume) and **WF-2C** (distributed transfer /
  de-panicking) persist and transport these hashes; they add **no** hash-input
  or blob-format change.
- The additive ABI-v4 vtable tail reserves padding precisely so later polyglot
  vtable growth (e.g. the ffi-rebuild §7 `request_cancel` hook) needs **no**
  further ABI bump.

The precondition for the persisted stores (`SnapshotStore`, `RemoteBlobCache`)
introduced downstream — that a content hash computed today equals the one
computed after every remaining WF-2A/2B/2C stage — **holds as of this
workflow.** No further break is expected or permitted without an explicit
re-ratification.
