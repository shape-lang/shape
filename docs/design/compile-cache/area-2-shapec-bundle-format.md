# Feasibility Area 2 — `.shapec` bundle format + extension point + content-hash invalidation

Read-only investigation. Every claim cites `file:line` against workspace HEAD
(`/home/dev/dev/shape-lang/shape`).

**VERDICT: clean.** There is a clear extension point (add a field to
`PackageBundle` / `ModuleManifest` and/or a new typed field on `FunctionBlob`)
and a real format-version mechanism already exists (`FORMAT_VERSION`,
magic + LE-u32 header, `MIN_FORMAT_VERSION` floor, `#[serde(default)]`
backward-compat). Adding a `resolved_interface` field is a v3→v4 bump that
old loaders already tolerate via `#[serde(default)]`; no blocker. One real
GAP for the *invalidation half* of the question: there is **no
source-hash → load-or-rebuild gate today** for either the `.shapec` bundle
or `core_stdlib.msgpack`. `source_hash` is write-only metadata; the embedded
stdlib is an unconditional `include_bytes!`. The hook point exists and is
clean, but the logic must be written.

---

## TL;DR table

| Question | Answer |
|----------|--------|
| (a) bundle struct | `PackageBundle` — `crates/shape-runtime/src/package_bundle.rs:72`. Per-module bytecode is `BundledModule.bytecode_bytes: Vec<u8>` (MessagePack `BytecodeProgram`), `package_bundle.rs:59-68`. The content-addressed unit (the struct the prompt enumerates: bytecode/constants/strings/deps/permissions/content_hash) is `FunctionBlob` — `crates/shape-vm/src/bytecode/content_addressed.rs:33`. |
| (b) serialization | MessagePack via `rmp_serde` over serde derives. Bundle: `package_bundle.rs:112`. Blob: `content_addressed.rs:147`. Stdlib: `stdlib_gen.rs:15`. |
| (c) extension point | Add `resolved_interface` to `PackageBundle` (`package_bundle.rs:72`) or `ModuleManifest` (`module_manifest.rs:13`) — both already `#[serde(default)]`-friendly. Version field EXISTS: `FORMAT_VERSION: u32 = 3` (`package_bundle.rs:15`). Needs a `3→4` bump (mechanical). |
| (d) content_hash | `FunctionBlob::compute_hash` (`content_addressed.rs:122`) = SHA-256 of `rmp_serde`-encoded `FunctionBlobHashInput` (`content_addressed.rs:96-117`). Source-hash invalidation would hook at `BundleCompiler::compile` (`bundle_compiler.rs:24`) producer + `ModuleLoader::set_dependency_paths`/`load_bundle` consumer (`module_loader/mod.rs:324,372`) and `stdlib.rs:load_core_modules_best_effort` (`stdlib.rs:56`). |
| (e) shared format? | **Distinct.** `core_stdlib.msgpack` = bare MessagePack `BytecodeProgram`, no header (`stdlib_gen.rs:14-15`). `.shapec` = `SHAPEPKG` magic + version header wrapping a `PackageBundle`, whose per-module payload is itself a MessagePack `BytecodeProgram` (`package_bundle.rs:6,59-68,111-120`). They share the inner `BytecodeProgram` serde shape but NOT the container format. |

---

## (a) What is serialized today

### Two layers exist. Don't conflate them.

The prompt's enumerated field list (bytecode, constants, strings, dependency
hashes, required_permissions, content_hash) is the **content-addressed
function unit**, not the bundle container. That unit is:

**`FunctionBlob`** — `crates/shape-vm/src/bytecode/content_addressed.rs:33`

Serialized fields (`#[derive(Debug, Clone, Serialize, Deserialize)]`,
`content_addressed.rs:32`):
- `content_hash: FunctionHash` (`[u8;32]`) — `content_addressed.rs:35`
- metadata: `name`, `arity`, `param_names`, `locals_count`, `is_closure`,
  `captures_count`, `is_async`, `ref_params`, `ref_mutates`,
  `mutable_captures`, `frame_descriptor` — `content_addressed.rs:38-53`
- `instructions: Vec<Instruction>` — `content_addressed.rs:57`
- `constants: Vec<Constant>` — `content_addressed.rs:59`
- `strings: Vec<String>` — `content_addressed.rs:61`
- `required_permissions: PermissionSet` — `content_addressed.rs:66`
- `dependencies: Vec<FunctionHash>` (content hashes of referenced functions)
  — `content_addressed.rs:71`
- `callee_names: Vec<String>` — **NOT serialized** (`#[serde(skip)]`,
  `content_addressed.rs:75`)
- `type_schemas: Vec<String>` — `content_addressed.rs:80`
- `foreign_dependencies: Vec<[u8;32]>` — `content_addressed.rs:86`
- `source_map: Vec<(usize,u32,u32)>` — `content_addressed.rs:91`

The blobs live in a content-addressed `Program` container
(`HashMap<FunctionHash, FunctionBlob>`) at `content_addressed.rs:166-362`
(field `function_store`, `content_addressed.rs:171`). Note: `Program` carries
~15 `#[serde(skip, default)]` JIT/ConcreteType conduit side-tables
(`content_addressed.rs:205,219,231,243,256,270,303,316,329,341,360`) that are
deliberately NOT on the wire — relevant precedent for adding a field that the
format may or may not serialize.

### The actual `.shapec` container

**`PackageBundle`** — `crates/shape-runtime/src/package_bundle.rs:72`,
`#[derive(Serialize, Deserialize)]` at `package_bundle.rs:71`. Fields:
- `metadata: BundleMetadata` — `package_bundle.rs:74` (struct at
  `package_bundle.rs:25`; contains `name`, `version`, `compiler_version`,
  **`source_hash: String`** (`package_bundle.rs:33`), `bundle_kind`,
  `build_host`, `native_portable`, `entry_module`, `built_at`, `readme`)
- `modules: Vec<BundledModule>` — `package_bundle.rs:76`
- `dependencies: HashMap<String,String>` — `package_bundle.rs:78`
- `blob_store: HashMap<[u8;32], Vec<u8>>` — `package_bundle.rs:82`
  (content-addressed store of `rmp_serde`-encoded `FunctionBlob` bytes;
  populated at `bundle_compiler.rs:207-211`)
- `manifests: Vec<ModuleManifest>` — `package_bundle.rs:86`
- `native_dependency_scopes: Vec<BundledNativeDependencyScope>` —
  `package_bundle.rs:90`
- `docs: HashMap<String, Vec<DocItem>>` — `package_bundle.rs:93`

**`BundledModule`** — `package_bundle.rs:59`:
- `module_path: String` — `package_bundle.rs:61`
- `bytecode_bytes: Vec<u8>` — `package_bundle.rs:63` — "MessagePack-serialized
  `BytecodeProgram` as raw bytes" (set at `bundle_compiler.rs:123-138`)
- `export_names: Vec<String>` — `package_bundle.rs:65`
- `source_hash: String` (per-file SHA-256) — `package_bundle.rs:67`
  (computed `bundle_compiler.rs:78-80`)

So a `.shapec` carries the **same module twice**: once legacy as
`bytecode_bytes` (whole `BytecodeProgram`) and once content-addressed via
`manifests` + `blob_store` (per-function `FunctionBlob`s). The loader prefers
manifests when present, then also registers the legacy form
(`module_loader/mod.rs:377-426`).

---

## (b) Serialization mechanism

MessagePack via `rmp_serde` over serde derives, throughout:
- Bundle container: `PackageBundle::to_bytes` → `rmp_serde::to_vec(self)`,
  `package_bundle.rs:112`; `from_bytes` → `rmp_serde::from_slice`,
  `package_bundle.rs:147`.
- Per-module bytecode: `rmp_serde::to_vec(&bytecode)`, `bundle_compiler.rs:123`.
- Per-function blob into `blob_store`: `rmp_serde::to_vec(blob)`,
  `bundle_compiler.rs:208`.
- Content hash input: `rmp_serde::encode::to_vec(&input)`,
  `content_addressed.rs:147` (comment notes the struct-as-array encoding is
  order-preserving/deterministic, `content_addressed.rs:144-146`).
- Manifest hash input: `rmp_serde::encode::to_vec(&input)`,
  `module_manifest.rs:97,123`.
- `core_stdlib.msgpack`: `rmp_serde::to_vec(&program)`, `stdlib_gen.rs:15`;
  loaded `rmp_serde::from_slice(bytes)`, `stdlib.rs:83`.

No bincode, no JSON, no custom codec on the load path. (`serde_json` appears
only in a `module_manifest.rs:225` unit test.)

---

## (c) Clean extension point for `resolved_interface`

### A version field already exists.

`crates/shape-runtime/src/package_bundle.rs:15`:
```rust
const FORMAT_VERSION: u32 = 3;
const MIN_FORMAT_VERSION: u32 = 1;   // package_bundle.rs:17
const MAGIC: &[u8; 8] = b"SHAPEPKG"; // package_bundle.rs:14
```
On-disk header (documented `package_bundle.rs:6`): `[8 bytes "SHAPEPKG"]
[4 bytes format_version LE] [MessagePack payload]`. Written at
`package_bundle.rs:115-118`; validated at `package_bundle.rs:127-145`
(range check `MIN_FORMAT_VERSION..=FORMAT_VERSION`, `package_bundle.rs:140`).
Version history is in-code: v1 lacked blob_store/manifests, v2 added them,
v3 added docs (`package_bundle.rs:16,123-124`).

### Three candidate insertion points (all clean):

1. **`PackageBundle` bundle-level** (`package_bundle.rs:72`) — add
   `#[serde(default)] pub resolved_interface: ...`. Most natural if the
   interface is per-bundle or keyed by module path. New fields with
   `#[serde(default)]` are exactly how v2/v3 fields were added
   (`blob_store`/`manifests`/`docs` carry `#[serde(default)]` at
   `package_bundle.rs:81,85,92`). Bump `FORMAT_VERSION` to `4`
   (`package_bundle.rs:15`).

2. **`ModuleManifest`** (`module_manifest.rs:13`) — add the resolved
   interface alongside `exports`/`type_schemas`. This is the most
   *semantically* correct home: a manifest already maps export names →
   content hashes (`module_manifest.rs:17`), so it is the per-module
   "what does this module expose" record. Caveat: the manifest has its own
   integrity hash `manifest_hash` (`module_manifest.rs:26`) computed over
   `ManifestHashInput` (`module_manifest.rs:43-51`, `finalize` at
   `module_manifest.rs:78`). Adding an interface field that should be
   integrity-covered means extending `ManifestHashInput` + `finalize` +
   `verify_integrity` (`module_manifest.rs:104`); if it should NOT be
   covered, just add the field and leave the hash input alone. Either is
   mechanical.

3. **`FunctionBlob` per-function** (`content_addressed.rs:33`) — add a typed
   field carrying the function's resolved signature/annotation interface.
   This is the right home if the interface is *per-function* (e.g.
   annotation-level interface attached to a specific exported function).
   It would change `content_hash` ONLY if added to `FunctionBlobHashInput`
   (`content_addressed.rs:96-117`); leaving it out of that struct keeps
   existing hashes stable (precedent: `source_map`, `callee_names`,
   `foreign_dependencies` handling — `foreign_dependencies` IS in the hash
   input at `content_addressed.rs:116`, `source_map` is NOT).

### Does it need a format-version bump?

For options 1 and 2: a `FORMAT_VERSION 3→4` bump is the correct, clean move,
and it is purely additive — **old loaders still load new bundles** because
`from_bytes` accepts any version in `MIN..=FORMAT_VERSION` and serde fills
missing fields via `#[serde(default)]`. Conversely a v3 loader reading a v4
bundle would be rejected by the range check (`package_bundle.rs:140`) UNLESS
you keep the field `#[serde(default)]` and do not raise the floor — but
raising `FORMAT_VERSION` is the honest signal. **Forward-compat caveat:**
the current check rejects `version > FORMAT_VERSION` (`package_bundle.rs:140`),
so a v4 bundle fails on a v3 binary. If silent forward-tolerance is desired,
that comparison would need relaxing — but that is a policy choice, not a
format limitation.

For option 3: no header bump needed at all (`FunctionBlob` is serialized
inside `blob_store` values without its own version header); a new
`#[serde(default)]` field round-trips against old blobs transparently.

### Can the format carry annotation-level interface? (not blocked)

Yes. The payload is arbitrary serde over MessagePack, so any
`Serialize+Deserialize` type — including an annotation/interface descriptor
with nested structs, maps, enums — serializes cleanly. The only constraints
observed are: (i) MessagePack does not natively support `[u8;64]`
(`module_manifest.rs:35-36` uses `Vec<u8>` for Ed25519 sigs as a worked
example), and (ii) several conduit side-tables in `Program`/`LinkedProgram`
are `#[serde(skip)]` precisely because they carry "opaque registry IDs that
aren't a stable wire shape" (e.g. `content_addressed.rs:202-206,
243-245`). So: a `resolved_interface` must be expressed in *stable, registry-
ID-free* terms (names/strings/structural types), not raw `ConcreteType` /
`StructLayoutId` / `Arc<VTable>` handles. With that discipline it is fully
representable. **Not blocked.**

---

## (d) How `content_hash` is computed, and where invalidation hooks in

### content_hash today

`FunctionBlob::compute_hash` — `content_addressed.rs:122-153`:
1. Convert `required_permissions` → sorted permission name strings
   (`content_addressed.rs:124`).
2. Build `FunctionBlobHashInput` (`content_addressed.rs:96-117,125-143`) —
   the explicit, ordered subset of identity fields: `name`, `arity`,
   `param_names`, `locals_count`, `is_closure`, `captures_count`, `is_async`,
   `ref_params`, `ref_mutates`, `mutable_captures`, `instructions`,
   `constants`, `strings`, `dependencies`, `type_schemas`,
   `required_permission_names`, `foreign_dependencies`.
3. `rmp_serde::encode::to_vec(&input)` → `Sha256::digest` → `[u8;32]`
   (`content_addressed.rs:147-152`).

Note what is **excluded** from the hash: `content_hash` itself, `callee_names`
(skip), `frame_descriptor`, `source_map`. So content_hash = SHA-256 of
(metadata-identity + bytecode + constants + strings + dependency-hashes +
sorted-permission-names + foreign-dep-hashes), MessagePack-encoded. Two
functions with identical code but different permissions hash differently
(permission names are in the input) — matches CLAUDE.md's stated invariant.

The `ModuleManifest.manifest_hash` is a second, independent SHA-256 over
`ManifestHashInput` (name/version/exports/type_schemas/perm_bits/dep_closure),
`module_manifest.rs:43-51,78-101`.

### Source-hash → load-or-rebuild invalidation: GAP (logic absent; hook clean)

There is **no source-hash-driven load-or-rebuild gate today**:

- `BundleMetadata.source_hash` (`package_bundle.rs:33`) and
  `BundledModule.source_hash` (`package_bundle.rs:67`) are **write-only**.
  Producer computes them (`bundle_compiler.rs:78-80` per-file,
  `bundle_compiler.rs:142-144` combined). A workspace grep for read-consumers
  of `.source_hash` that drive a rebuild/skip decision returns **none** — the
  only consumers are test fixtures and the write sites. The doc comment claims
  "freshness checks" (`package_bundle.rs:4`) but no loader path reads it.

- `core_stdlib.msgpack` is loaded by **unconditional `include_bytes!`** baked
  into the binary at compile time (`stdlib.rs:26`), then
  `load_core_modules_best_effort` (`stdlib.rs:56-79`) tries the embedded bytes
  and falls back to source compilation only on *deserialize failure*
  (`stdlib.rs:63-78`) or the `SHAPE_FORCE_SOURCE_STDLIB` env override
  (`stdlib.rs:58`). There is no source-hash check — staleness is caught only
  by the offline `stdlib_gen --verify` semantic count/name comparison
  (`stdlib_gen.rs:27-82`), which is a CI/dev tool, not a runtime gate.

- The **only** real load-or-rebuild-by-hash precedent in the tree is the
  vendored-native-library cache in `script_cmd.rs:818-858`: it hashes the
  source path (`PackageLock::hash_path`, `script_cmd.rs:818`), derives a
  cache key, and re-copies only when the cached file's hash differs
  (`script_cmd.rs:842-849`). This is the pattern a compile-cache
  invalidation would mirror.

**Where the hook belongs (clean insertion points):**
- *Producer / write side:* `BundleCompiler::compile` already computes both
  per-file and combined source hashes (`bundle_compiler.rs:78-80,142-144`) and
  stamps them into metadata (`bundle_compiler.rs:182-198`). A cache layer
  would compare the prospective source hash against an existing on-disk
  `.shapec`'s `metadata.source_hash` before recompiling.
- *Consumer / load side:* `ModuleLoader::set_dependency_paths`
  (`module_loader/mod.rs:324`) → `load_bundle` (`module_loader/mod.rs:372`) is
  where a `.shapec` is read and registered; a freshness gate comparing the
  bundle's `source_hash` against the live source tree (if present) belongs
  here, or in a thin cache wrapper around `PackageBundle::read_from_file`
  (`package_bundle.rs:159`).
- *Stdlib prelude side:* `load_core_modules_best_effort` (`stdlib.rs:56`) is
  the single chokepoint; a source-hash compare (embedded-artifact hash vs.
  hash of `stdlib/core/` sources) would slot in next to the existing
  env-override + deserialize-fallback branches.

All three are clean, single-function hook points. The verdict for (d) is:
the *hook* is clean, the *invalidation logic does not exist yet* and must be
written (this is the GAP, scoped and non-blocking).

---

## (e) Is the format shared between `core_stdlib.msgpack` and `.shapec`?

**Distinct container formats; shared inner `BytecodeProgram` serde shape.**

- `core_stdlib.msgpack`: a **bare MessagePack `BytecodeProgram`**, no magic, no
  version header. Written `rmp_serde::to_vec(&program)` directly to the file
  (`stdlib_gen.rs:14-15,87`); loaded `rmp_serde::from_slice::<BytecodeProgram>`
  (`stdlib.rs:82-83`, and the verify path `stdlib_gen.rs:39`). Embedded via
  `include_bytes!` (`stdlib.rs:26`).

- `.shapec`: a **`SHAPEPKG`-magic + LE-u32-version header** wrapping a
  MessagePack `PackageBundle` (`package_bundle.rs:6,111-120`). Each
  `BundledModule.bytecode_bytes` is *itself* a MessagePack `BytecodeProgram`
  (`package_bundle.rs:63`, produced `bundle_compiler.rs:123`). So the inner
  per-module payload IS the same serde shape as `core_stdlib.msgpack`, but the
  outer container, header, and the content-addressed `blob_store`/`manifests`
  layer are bundle-only.

**Implication for "the compile-cache must serve BOTH the prelude cache and
per-package builds":** a single unified cache CAN serve both, because both
ultimately deserialize a `BytecodeProgram` (stdlib directly; package via
`bytecode_bytes`). But they do NOT share a container today — the stdlib path
is header-less and `include_bytes!`-baked, while the package path is
header-versioned and file/dependency-loaded. A unified compile-cache would
need to either (i) wrap the stdlib in the same `SHAPEPKG`-versioned container
(treating the prelude as a degenerate bundle), or (ii) keep two thin readers
sharing the inner `BytecodeProgram` codec and the source-hash invalidation
helper. Neither is blocked; both are additive.

---

## Net verdict: CLEAN (with one scoped GAP in invalidation)

- Extension point: **clear** — `PackageBundle` / `ModuleManifest` /
  `FunctionBlob`, all `#[serde(default)]`-friendly additive fields.
- Version mechanism: **exists** — `FORMAT_VERSION=3` + magic + LE-u32 header +
  `MIN_FORMAT_VERSION` floor + serde-default backward compat
  (`package_bundle.rs:14-17,111-149`). `resolved_interface` is a clean
  v3→v4 additive bump (or no bump if placed on `FunctionBlob`).
- Format can carry annotation-level interface: **yes**, provided it is
  expressed in registry-ID-free, structurally-stable terms (the `#[serde(skip)]`
  conduit side-tables in `Program`/`LinkedProgram` are the cautionary
  precedent — raw `ConcreteType`/`StructLayoutId`/`Arc<VTable>` are NOT wire-
  stable).
- **GAP (non-blocking, scoped):** no source-hash → load-or-rebuild gate exists
  today for either artifact; `source_hash` is write-only metadata
  (`package_bundle.rs:33,67`; zero read-consumers) and the stdlib is an
  unconditional `include_bytes!` (`stdlib.rs:26`). The hook points are clean
  (`bundle_compiler.rs:24`, `module_loader/mod.rs:324/372`, `stdlib.rs:56`)
  and there is a working hash-cache precedent to copy (`script_cmd.rs:818-858`),
  but the invalidation logic must be written.
