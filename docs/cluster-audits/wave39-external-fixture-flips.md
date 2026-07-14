# Wave 39D External Fixture Flip Scout

Date: 2026-07-10
Scope: the 41 rows classified as `External/manual/fixture-only` by
`wave38-disabled-current-triage.md`.

## Baseline and Decision Rule

The current sibling manifest is
`/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`:
707 total snippets, 559 runnable, 148 disabled, and 41 in this category.
The current harness has three fixture classes:
`serve`, `serve-snapshot-resume`, and `local-snapshot-resume`. `serve` only
does one isolated loopback TCP receiver, address substitution, and ordinary
VM/JIT client execution. It does not load extensions, create projects, serve
HTTP, provide raw transport peers, or configure host permissions.

The ranking below counts a row only when the fixture can prove the documented
behavior with fixed inputs and no dependency on DNS, the public internet, a
wall-clock value, or an undeclared host capability. The reductions are additive
because the lanes own disjoint rows.

## Ranked Lanes

### 1. Loopback Remote Annotation (`serve`): 3 rows

**Rows**

- `stdlib/core/remote.mdx:42`
- `stdlib/core/remote.mdx:77`
- `advanced/annotations.mdx:480`

**Why now:** These are pure Shape functions over an integer, array, and string
value. The only blocker stated by the pages is a live receiver. The existing
`fixture=serve` already allocates a loopback port, substitutes
`__BOOK_SERVE_ADDR__`, isolates client/server config directories, and kills the
receiver. The low-level remote examples already use this exact contract.

**Fixture contract:** Mark each fence `fixture=serve
serve-sandbox=none`. Replace every hard-coded worker address with
`__BOOK_SERVE_ADDR__`. Run both VM and JIT against one fresh receiver per mode;
retain the assertion-only rows or add stable printed markers before adding an
`expected=` value. The fixture must reject non-loopback substitutions and must
not silently fall back to local execution.

**Owned files:**
`shape-web/book/book-site/src/content/docs/stdlib/core/remote.mdx`,
`shape-web/book/book-site/src/content/docs/advanced/annotations.mdx`.
The existing implementation in
`shape-web/book/book-site/scripts/run-book-truth-gate.mjs` and its focused
coverage in `scripts/serve-fixture.test.mjs` are the proof surface; no new
fixture class is required.

**Prerequisites and constraints:** Only the release `shape` binary and
loopback TCP are required. Use temporary `XDG_DATA_HOME` and
`SHAPE_CONFIG_DIR`; do not use DNS, TLS, external addresses, or a shared
snapshot store. A receiver log or a deliberately non-local control should be
part of the focused fixture proof so a local-success fallback cannot pass.

**Expected reduction:** 3 disabled rows, from 41 to 38 in this category and
from 148 to 145 overall.

### 2. Local Foreign Scalar/Object/Error Examples (`extension-*`): 8 rows

**Rows**

- `fundamentals/functions.mdx:413`
- `tooling/polyglot.mdx:14`
- `tooling/python-extension.mdx:68`, `:117`, `:184`
- `tooling/typescript-extension.mdx:74`, `:134`, `:163`

**Why now:** These calls have fixed, local bodies. They do not need a server,
HTTP, NumPy, `aiohttp`, a project file, or a database. Both required artifacts
are present in this workspace at `extensions/libshape_ext_python.so` and
`extensions/libshape_ext_typescript.so`; the existing distributed composition
tests also locate these artifacts and explicitly opt languages in on receivers.

**Fixture contract:** Add two manifest fixture names, for example
`fixture=extension-python` and `fixture=extension-typescript`. The harness must
resolve only the checked-in/pinned artifact (or an explicitly configured CI
artifact directory), pass the selected `.so` with the CLI's extension loading
option, run both VM and JIT in isolated temporary config/data directories, and
fail loudly when the artifact is absent. It must not self-skip. Add stable
`expected=` output where the snippet prints values; for error examples assert
that the foreign body ran and the returned error is a marshal error, rather
than matching a platform-specific stack trace.

**Owned files:** The exact MDX rows above; parser/dispatch changes in
`shape-web/book/book-site/scripts/extract-shape-snippets.mjs` and
`run-book-truth-gate.mjs`; metadata and subprocess tests in
`scripts/serve-fixture.test.mjs` and `scripts/run-book-truth-gate.test.mjs`.
Reuse the extension lookup convention in
`bin/shape-cli/tests/support/distributed_snapshot_polyglot.rs` when specifying
the CI artifact path, but do not make the book gate depend on a developer's
`target/` directory.

**Prerequisites and constraints:** CI must publish the two exact ABI-compatible
`.so` artifacts for the book gate, with a required check rather than a skip.
The bodies use Python `statistics` or pure arithmetic and TypeScript literals;
they must not import network packages or read the host filesystem. Do not mark
these runs as deterministic sandbox runs: the current runtime deliberately
refuses foreign code under the deterministic permission marker. Limit the
foreign language to the fixture's declared language and keep all other
permissions closed.

**Expected reduction:** 8 disabled rows, conditional on the pinned artifacts;
the category reaches 30 and the overall disabled count reaches 137. Without
those artifacts, this lane is not a valid book flip and must remain disabled.

### 3. Python Snapshot/Resume With a Foreign Call (`local-snapshot-resume`): 1 row

**Row**

- `advanced/polyglot-distributed.mdx:149`

**Why now:** This row calls a fixed Python function before and after a snapshot;
the current integration matrix already proves the same foreign-call,
snapshot-store, and resume composition. It does not require a remote server or
network service. The current local snapshot fixture already owns the two-pass
VM protocol; it only lacks extension loading and a foreign-language selection.

**Fixture contract:** Extend the local snapshot fixture with a declared
`foreign-language=python` option (or a dedicated
`fixture=local-snapshot-resume-extension-python`). Resolve the pinned Python
`.so`, pass it on both first and resume invocations, require a stable
`LOCAL_SNAPSHOT=HASH:<hex>` marker, and compare the resumed signal to an exact
expected string. Change the row's marker from the generic `HASH:` to the
fixture marker if necessary. The snapshot must be taken between foreign calls;
the fixture must prove resume re-links the extension rather than recomputing
from source.

**Owned files:** `shape-web/book/book-site/src/content/docs/advanced/polyglot-
distributed.mdx`; `scripts/run-book-truth-gate.mjs` for extension-aware local
resume; `scripts/serve-fixture.test.mjs` and
`scripts/run-book-truth-gate.test.mjs` for marker, artifact, and cleanup
coverage. The existing `bin/shape-cli/tests/distributed_dynamic_snapshot_e2e.rs`
and `distributed_composition_e2e.rs` are reference contracts only.

**Prerequisites and constraints:** The pinned Python extension artifact is
required. No public network, DNS, TLS, live foreign frame, or shared snapshot
store is permitted. Run VM-authoritative resume as the existing fixture does;
do not claim JIT resume coverage. The first-pass output may contain a hash, but
the gate must compare only fixed semantic markers, never the hash itself.

**Expected reduction:** 1 disabled row, conditional on the Python artifact;
category 29 and overall 136 after lanes 1-3. This lane is not a flip if the
artifact is missing.

### 4. Isolated Two-File Module Project (`project-modules`): 1 row

**Row**

- `fundamentals/modules.mdx:191`

**Why now:** The page already supplies the complete `mylib/linalg.shape` and
`mylib/stats.shape` sources immediately above the disabled main program. The
runtime has filesystem project/module resolution; no service or native library
is involved. The fixture can turn the page's illustrative three-file example
into an isolated temporary project without changing language semantics.

**Fixture contract:** Add `fixture=project-modules`. The harness writes a
temporary `shape.toml`, `main.shape` from the extracted row, and the two
adjacent module sources under `mylib/`; runs from that temporary project root
under VM and JIT; and removes the tree afterward. The sidecar source map must
be keyed by the manifest ID, not inferred from arbitrary neighboring fences.
Require the import paths to resolve from the temporary project and compare the
two modes' normalized output. Do not inject an `input` binding or rewrite the
program body.

**Owned files:** `shape-web/book/book-site/src/content/docs/fundamentals/modules.mdx`;
`scripts/extract-shape-snippets.mjs` and `run-book-truth-gate.mjs` for fixture
metadata and project setup; a new focused
`scripts/project-fixture.test.mjs` for sidecar creation, cwd isolation, and
cleanup. No production module-loader edits are implied.

**Prerequisites and constraints:** Only the release binary and ordinary local
filesystem are needed. The fixture must use a fresh temp directory, reject
absolute/module escape paths, and avoid reading the checked-out book or source
tree at execution time. This row is distinct from package distribution: it
does not prove `.shapec` bundles, dependency locks, or registry resolution.

**Expected reduction:** 1 disabled row, category 28 and overall 135 if all
four lanes land. This is the smallest lane and should follow the remote and
foreign fixture contracts.

## Explicit Rejections

These rows should remain disabled; changing only book metadata would produce a
false truth gate.

- `examples/web-request.mdx:22` still documents a public HTTPS API and also
  states that the typed HTTP return path is an outstanding runtime gap.
- `fundamentals/datetime.mdx:19` depends on the host clock/timezone and the
  page records a current JIT DateTime receiver gap.
- `fundamentals/modules.mdx:80` uses an undefined `input` and is not a
  self-contained module-resolution proof.
- `fundamentals/resource-management.mdx:365`, `:387` require an undeclared
  database, streams, async-scope/`for await`, and task semantics.
- `fundamentals/variables.mdx:168` uses an undeclared `fs` binding and a
  host-relative `config.txt`; it is not the shipped `std::core::file` API.
- `advanced/content-addressed-bytecode.mdx:515`,
  `advanced/wire-protocol.mdx:90`, and `stdlib/core/transport.mdx:61`, `:95`
  contain undefined payloads and require a raw framed-transport peer. The
  `serve` fixture is not a raw transport test peer.
- `advanced/module-distribution.mdx:563` needs an unavailable package/module
  fixture and undefined arguments; it is not covered by the two-file project
  lane.
- `stdlib/core/remote.mdx:107` needs a live receiver plus Python/NumPy and the
  GPU-worker premise; `stdlib/core/remote.mdx:220` needs a deterministic
  refused-port fixture and is not self-contained as extracted.
- `advanced/security-permissions.mdx:441` describes host-configured
  permission enforcement, not a Shape-level API. It needs a permission-aware
  host fixture and a stable diagnostic contract.
- `advanced/annotations.mdx:508`,
  `advanced/comptime-annotations-cookbook.mdx:183`, `:329` depend on
  extension-provided routing, undefined application values, or unsupported
  conceptual syntax.
- `advanced/native-c-interop.mdx:139`, `:155`, `:286`,
  `tooling/extensions.mdx:120`, and `tooling/polyglot.mdx:186` require DuckDB,
  Arrow, NumPy, or other native libraries not guaranteed by the book gate.
- `tooling/polyglot.mdx:126`, `tooling/python-extension.mdx:142`, `:163`,
  `:197`, `tooling/typescript-extension.mdx:180` require HTTP services or
  network packages. `tooling/typescript-extension.mdx:238` additionally needs
  an external `helpers.ts` module. These need a separate HTTP/project fixture
  and should not be folded into the scalar extension lane.

## Recommendation

Dispatch the first worker on **Lane 1**. It is the only lane requiring no new
harness contract, no external artifact, and no change to runtime behavior; its
three rows are a direct extension of the already truth-gated remote examples.
Then dispatch Lane 2 only with a required CI artifact check, followed by the
foreign snapshot lane. Leave the rejected rows disabled until their stated
service, host API, native dependency, or language gap is independently closed.

Static inspection only; no cargo, build, test, extraction, or book-truth gate
was run.
