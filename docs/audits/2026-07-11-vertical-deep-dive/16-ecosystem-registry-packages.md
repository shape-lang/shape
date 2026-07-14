# Vertical Deep-Dive 16 — Ecosystem: Registry, Packages, MCP, Apps, Infra

**Auditor:** 16 of 19
**Date:** 2026-07-11
**Territory:** `shape-registry/` (Axum package registry, Ed25519 verification), `shape-app/` (playground + notebook server), `shape-mcp/` (MCP server teaching LLMs Shape), `packages/` (duckdb, xgboost pure-Shape packages), `shape-infra/` (NixOS/Debian deployment)
**Method:** Full read of every in-territory Rust/SQL/TOML/Nix source file, cross-reference against the language HEAD workspace (`shape/`) and the book (`shape-web/book/`), plus empirical runs of the prebuilt `shape` binary and a from-source rebuild of `shape-mcp` driven over JSON-RPC.

> Scope note: the sibling directory `/home/dev/dev/shape-lang/` holds ~250 stale worktrees. Nothing in this report reads or cites those. Every citation is inside the five named territory roots plus the main `shape/` workspace and `shape-web/book/` where the ecosystem consumes them.

---

## 0. Executive summary

The ecosystem tier is a **collection of five independently-versioned satellite projects** orbiting the language core. Each is individually well-built in isolation — the registry is a clean, layered Axum service with real password hashing and rate limiting; the MCP server has a genuinely sophisticated retrieval pipeline; shape-app has systemd-hardened deployment — but the tier as a whole suffers from **drift**: the satellites were each pinned or snapshotted at different points in the language's evolution, and the connective tissue between them (doc IDs, bundle formats, syntax examples, sandbox posture) has silently fallen out of sync with HEAD.

The two highest-impact findings are both **empirically demonstrated**, not inferred:

1. **The MCP server — whose entire reason to exist is teaching LLMs current Shape — has broken retrieval for core constructs and the entire stdlib API surface.** After the book was reorganized (stdlib split into `core/`/`native/`/`domain/`/`math/` subdirectories; `traits`/`async`/`modules` moved from `advanced/` to `fundamentals/`), the MCP's hardcoded construct→doc-ID maps were never updated. A from-source rebuild against the current book returns `Unknown module: 'math'` for **all six** advertised `get_shape_api` modules and `Unknown construct: 'traits'` for `traits`, `async`, `modules`, `packages`, `projects` — while the tool's own help text still lists those exact names as valid inputs. Nine of twenty-one advertised MCP resources 404. This is a silent regression: the committed eval artifact shows these tools passing when the book was flat.

2. **The shape-app playground/notebook executes arbitrary user-submitted Shape code with no capability sandbox and no instruction cap.** The execution engine is built with `granted_permissions = None`, which `check_permission` treats as "all operations allowed", and `ResourceLimits::default()` sets `max_instructions: None`. The only guard is a `tokio::time::timeout`, which cannot preempt a synchronous CPU-bound VM loop. I demonstrated arbitrary filesystem read (`file::read_text("/etc/hostname")` → `atlas-dev`) through the same default engine path. Production systemd hardening (`ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`) contains file *writes* and hides `/home`, but `FsRead` of world-readable paths, `NetConnect` (SSRF), and CPU/memory DoS remain open.

The other material findings: the **xgboost sample package no longer compiles** against HEAD (`string as number` is now rejected under the numeric-conversion rules); the **registry's "Ed25519 verification" has no trust root** (keys are self-asserted and auto-bound on first use, signatures are optional, and the README overstates verification as a publish gate); a **native-blob path-traversal** write primitive; and a **widened markdown sanitizer** on the registry frontend that renders user READMEs with `<style>` and arbitrary `style=` attributes.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | **P1** | MCP `get_shape_api` broken for all 6 advertised modules; `get_shape_syntax` broken for traits/async/modules/packages/projects; 9/21 resources 404 — book-reorg drift in hardcoded ID maps | Empirical rebuild transcript §9.1; `shape-mcp/src/tools.rs:34,689`, `resources.rs:148,180`; book tree §8 |
| 2 | **P1** | shape-app executes untrusted code unsandboxed: `granted_permissions=None` (=all allowed) + `ResourceLimits` default `None` (no instr cap) + ineffective async timeout | `execute.rs:93-97,207`, `stdlib_cache.rs:57-67`, `module_exports.rs:219-226`, `resource_limits.rs:22-27`; file-read transcript §9.2 |
| 3 | **P1** | `xgboost` sample package fails to compile against HEAD — `string as number` no longer valid | `packages/xgboost/index.shape:45`; transcript §9.3 |
| 4 | **P1** | Registry "signature verification" has no trust root: self-asserted keys, TOFU auto-bind, signatures **optional** (unsigned bundles publish) | `services/publish.rs:196-250`; `crypto/signing.rs:46-55`; migration `user_keys` §6 |
| 5 | **P2** | Native-blob `target` unvalidated → authenticated path-traversal file write outside `blob_dir` | `routes/publish.rs:33-37`, `services/blob.rs:67-73`, `services/publish.rs:64-84` |
| 6 | **P2** | Registry frontend renders user READMEs through a widened sanitizer allowing `<style>`, `style=*`, `svg *`, `foreignObject` → CSS injection | `frontend/src/lib/markdown/render.ts` |
| 7 | **P2** | Version skew: registry pins crates.io `shape-runtime =0.1.6`; workspace HEAD is `0.3.2` — `PackageBundle` format-drift risk between publisher CLI and parser | `shape-registry/Cargo.toml:12`; `shape-runtime` workspace `0.3.2` §6 |
| 8 | **P2** | `version_info`/`package_info` hardcode `has_source:false`, `native_targets:[]` — the multi-blob feature is half-wired on the read path | `routes/package_info.rs:66-67,107-108` |
| 9 | **P2** | `llms.txt` teaches retired/wrong syntax (`c"..."` retired W18.3; `int` as i48 not i64; `method(self): T` trait form) | `shape-mcp/llms.txt`; transcripts §9.5 |
| 10 | **P2** | MCP `run_shape_code` description says "temp file + CLI" but implementation uses managed `shape serve` wire protocol; MCP hand-mirrors `shape-vm::WireMessage` (drift risk) | `tools.rs:118`, `executor.rs:307-317` |

### Feature-completeness score: **68 / 100**

The registry implements a genuinely complete crates.io-style surface (search, publish, yank, docs, sparse index, download stats, categories, dependents, native/source blobs, auth) and the CLI↔registry HTTP contract is consistent. But the read path leaves the multi-blob feature half-wired, the two shipped sample packages are non-functional against HEAD (one broken, one unverifiable), and the MCP's flagship retrieval tools are silently returning errors for the most-requested constructs. The playground/notebook works end-to-end for benign code. Points lost primarily for the MCP retrieval regression (a shipped, deployed tool that no longer does its one job) and the broken sample packages.

### Code-quality score: **74 / 100**

Individually the code is clean: idiomatic Axum/tokio, zero real `unsafe`, zero TODO/FIXME markers, parameterized SQL throughout, argon2 + token hashing, constant-time secret comparison. The MCP retrieval scorer is well-factored and tested. Points lost for: near-zero registry test coverage (1 test function for a security-sensitive publish/auth service), the hardcoded parallel ID maps that drifted, the hand-mirrored wire enum, committed junk files (`one,n`, `two,n`), and the default-permissive execution posture that a fresh reader would not expect to be the sandbox.

### Biggest risk

The biggest risk is **not any single bug — it is that the ecosystem has no mechanism to notice when it drifts from the core.** The MCP breakage is the canonical example: the book restructured, `build.rs` faithfully regenerated the embedded content with new IDs, the hardcoded lookup tables kept pointing at the old IDs, every affected tool started returning "Unknown …", and nothing failed the build, nothing failed a test, and the committed eval artifact still reads "PASS" because it was run before the reorg. The same class of latent drift sits under the registry's `shape-runtime =0.1.6` pin (a bundle produced by the HEAD CLI may not deserialize), under `llms.txt` (teaching syntax the compiler now rejects), and under the two sample packages (idioms the type system now forbids). Each satellite is a snapshot of "Shape as it was when I was last touched", and there is no CI gate that re-exercises the real cross-boundary path. The registry's unsandboxed-by-default sibling (shape-app) is a security instance of the same theme: the careful sandbox envelope built into `shape serve` is simply not on the path the playground actually uses.

---

## 1. Architecture & code structure map

The territory is **five separate git repositories / workspaces**, none of which is a member of the main `shape/` Cargo workspace. This is a deliberate isolation boundary (confirmed by CLAUDE.md: "`shape-app` and `shape-server` are NOT workspace members") and it means each ecosystem piece has its own `Cargo.lock`, its own `target/`, and its own dependency-version choices.

```
shape-lang/
├── shape/            ← language core (HEAD 0.3.2)  — the thing being orbited
├── shape-registry/   ← Axum package registry + Postgres + SvelteKit frontend
├── shape-app/        ← shape-server (playground/notebook HTTP API) + shape-notebook (Svelte UI)
├── shape-mcp/        ← MCP server (JSON-RPC over stdio) teaching LLMs Shape
├── packages/         ← duckdb, xgboost  (pure-Shape / native-binding sample packages)
└── shape-infra/      ← NixOS + Debian deployment configs, Cloudflare tunnel
```

### 1.1 shape-registry (2,619 LoC Rust across `src/`)

A layered Axum 0.8 service backed by PostgreSQL 16 (via sqlx 0.8) with a SvelteKit static-adapter frontend. Content-addressed `.shapec` bundles are stored on disk under `BLOB_DIR`; metadata lives in Postgres.

| Module | LoC | Responsibility |
|--------|-----|----------------|
| `services/publish.rs` | 541 | The 12-step publish pipeline: deserialize `PackageBundle`, validate name/semver, ownership check, manifest integrity, signature verify, key bind, checksum, extract docs + native deps, write blob, DB transaction |
| `rate_limit.rs` | 225 | `governor`-based keyed per-IP rate limiting (read 600 rpm / write 60 rpm), `cf-connecting-ip` → `ConnectInfo` → `x-forwarded-for` → `x-real-ip` client-key resolution |
| `routes/auth_routes.rs` | 217 | register / login / create_token / validate; argon2 password hashing; in-process login-attempt lockout |
| `dto.rs` | 183 | Serde request/response types mirroring the CLI's `PackageInfo`/`VersionInfo`/`RegistryIndexFile` |
| `main.rs` | 119 | Router assembly, CORS predicate, 50 MB body limit, compression, migrations-on-boot |
| `routes/download.rs` | 117 | bundle / source / native-blob download with fire-and-forget download-counter increment |
| `routes/stats.rs` | 114 | trending (7-day window), registry-wide totals, per-package download time-series |
| `routes/mod.rs` | 114 | Route table: read routes (rate-limited), write routes (rate-limited), health |
| `services/search.rs` | 111 | pg_trgm similarity + FTS combined ranking with sort modes |
| `routes/package_info.rs` | 110 | package info (versions + owners) / version info |
| `routes/docs.rs` | 81 | latest / specific-version doc retrieval (DocItem JSON per module) |
| `config.rs` | 77 | Env-driven config; SHA-256 admin-secret hash; constant-time compare |
| `auth.rs` | 75 | `AuthUser` extractor; Bearer-token → SHA-256 hash → DB lookup; 90-day token expiry |
| `services/blob.rs` | 74 | Disk blob read/write helpers (`{blob_dir}/{name}/{version}.shapec` and typed sub-blobs) |
| `error.rs` | 73 | `AppError` with status + message; `IntoResponse`; `From<sqlx::Error>` |
| `routes/yank.rs` | 63 | yank / unyank (owner-gated) |
| `routes/publish.rs` | 62 | multipart publish + legacy octet-stream publish handlers |
| `services/index_gen.rs` | 61 | Sparse-index TOML generation for the CLI dependency resolver |
| smaller | — | `dependents.rs` (39), `search.rs` (36), `categories.rs` (33), `health.rs` (21), `models/*`, `state.rs`, `services/mod.rs`, `models/mod.rs` |

**Data flow (publish):** CLI POSTs multipart (`shapec` + optional `source` + `native:<target>` fields) → `routes/publish.rs::publish` → `services/publish::publish_multipart` → `publish_bundle_inner` (12 steps) → `run_publish_transaction` (atomic upsert of package/version/deps/docs/blobs). Entry point: `main::main` binds `LISTEN_ADDR` (default `0.0.0.0:3000`).

**Migrations:** three SQL files run automatically on boot (`sqlx::migrate!`). `001_initial.sql` (139 lines) defines users, user_keys, api_tokens, packages, package_owners, versions, version_dependencies, version_docs, download_stats, categories, package_categories, package_tags + pg_trgm/FTS GIN indexes + 10 seed categories. `002_native_deps.sql` adds native-dep columns to versions. `003_multi_blob.sql` adds the `version_blobs` table.

### 1.2 shape-app / shape-server (Rust) + shape-notebook (Svelte)

`shape-server` is the HTTP API behind `play.shape-lang.dev` and `notebook.shape-lang.dev`. It links the language core via **local path dependencies** (`shape-runtime`, `shape-vm`, `shape-wire`, `shape-lsp` from `../shape/crates/...` at `=0.3.2`) — so it tracks HEAD, unlike the registry.

| Module | LoC | Responsibility |
|--------|-----|----------------|
| `static/playground.html` | 1,925 | Self-contained playground SPA (Monaco-ish editor + result panel) |
| `routes/notebook.rs` | 1,335 | Notebook CRUD (create/get/add-cell/update/delete/reorder) + per-cell and execute-all evaluation via `execute_repl` |
| `routes/execute.rs` | 505 | `/v1/api/execute` — one-shot code execution, chart-spec detection, error/HTML rendering |
| `static/notebook.html` | 474 | Notebook SPA |
| `rate_limit.rs` | 265 | Per-client editor/execution/LSP rate limiters |
| `main.rs` | 205 | Router, CORS `Any`, warmup, auth wiring |
| `routes/lsp.rs` | 201 | LSP-over-WebSocket proxy (spawns `shape-lsp`) |
| `auth.rs` | 127 | Optional Bearer auth proxied to the registry's `/v1/api/auth/validate` with a 5-min token cache |
| `routes/inlay_hints.rs` / `completions.rs` / `hover.rs` | 118/83/66 | Editor-assist endpoints |
| `routes/stdlib_cache.rs` | 92 | `once_cell` cached stdlib bootstrap; `create_engine_with_cached_stdlib` |
| `routes/playground.rs` / `health.rs` / `mod.rs` / `chart_detect.rs` | 33/25/11/6 | Legacy sessions, health, chart detection |

**Data flow (execute):** browser POSTs `{code, timeout_ms}` → `execute::execute_code` → `tokio::time::timeout(execute_internal)` → `create_engine_with_cached_stdlib()` (fresh engine + cached stdlib snapshot) → `engine.execute()` → WireValue rendered to JSON/display/chart-spec. Entry point: `main::main` binds `0.0.0.0:9091`.

**shape-notebook** (Svelte/TS, ~900 LoC) is the richer notebook UI with an ECharts chart adapter (`chart-spec-to-echarts.ts`, 361 LoC), a reactive notebook store, and an LSP client. It is a separate Vite build.

### 1.3 shape-mcp (2,896 LoC Rust)

A JSON-RPC-2.0-over-stdio MCP server. Its distinguishing design choice: **all documentation content is sourced from the Shape book at build time** via a `build.rs` that walks `../shape-web/book/book-site/src/content/docs`, discovers every `.mdx`, and generates a `BOOK_ENTRIES: &[(&str,&str)]` table of `include_str!`ed pages. There is no separate hand-maintained content corpus.

| Module | LoC | Responsibility |
|--------|-----|----------------|
| `content/loader.rs` | 833 | Content index: frontmatter parsing, LLM-enrichment extraction, synonym expansion, TF-IDF + trigram fuzzy search, token-budgeted retrieval |
| `tools.rs` | 712 | 10 MCP tools: search_docs, get_syntax, get_examples, run_code, run_file, run_project, validate, reload, get_api, search_packages |
| `executor.rs` | 518 | Managed `shape serve` child process + MessagePack-over-TCP wire client (mirrors a subset of `shape-vm::WireMessage`) |
| `main.rs` | 316 | JSON-RPC dispatch loop over stdin/stdout; initialize/tools/resources/prompts/ping |
| `prompts.rs` | 235 | MCP prompt templates |
| `resources.rs` | 187 | 21 `shape://` resource definitions + URI→doc-ID resolution |
| `content/tokenizer.rs` | 62 | Token estimation + budget truncation |
| `logging.rs` | 31 | Structured JSON analytics to stderr |

**Data flow (tools/call):** stdin JSON-RPC → `main` dispatch → `tools::call_tool` → either a content-index lookup (`get_syntax`/`get_api`/`search_docs`/`get_examples` — no subprocess) or a wire round-trip to the managed `shape serve` (`run_code`/`run_file`/`run_project`/`validate`). Entry point: `main::main`; the executor spawns `shape serve --sandbox permissive` on a random loopback port at startup.

### 1.4 packages/ (pure-Shape sample packages)

Two sample packages, each its own git repo:

- **duckdb** (`index.shape`, 182 LoC): type-safe DuckDB bindings via `extern C fn ... out out_db: ptr` declarations and comptime `DESCRIBE`-based schema inference. Declares a `[native-dependencies]` entry requiring `libduckdb`. Uses `from std::core::native use { ptr_new_cell, ... }`.
- **xgboost** (`index.shape`, 145 LoC): pure-Shape XGBoost JSON-model tree-walking inference, no native deps. Uses `use std::core::file` / `use std::core::json`. Has a `shape.lock`.

### 1.5 shape-infra (NixOS + Debian)

- `nixos/shape-prod.nix`: the production NixOS host — a systemd-hardened `shape-server` unit, an nginx origin router (127.0.0.1:8080/8081/8091/8092), and a Cloudflare tunnel mapping `shape-lang.dev` / `book.` / `play.` / `notebook.` to loopback ports.
- `debian/pull-deploy/`: an alternate pull-based deploy (systemd timers + a `shape-deploy-app.sh` that fetches a GitHub release asset, verifies SHA-256, and symlinks the new binary).
- `flake.nix` + `flake.lock`: portable flake check.
- `.github/workflows/{ci,deploy}.yml`.

### 1.5b Deployment topology (registry vs app)

The two internet-facing services are deployed with different reverse proxies but a consistent "app-port-closed, proxy-fronted" pattern:

**Registry** (`shape-registry/deploy/`): `docker-compose.prod.yml` runs the Axum server bound to `127.0.0.1:3000` (note: *loopback-only* published port, so Docker does not expose it to the LAN) plus a Postgres 16 with a healthcheck gate. Caddy (`deploy/Caddyfile`) terminates TLS for `pkg.shape-lang.dev`, reverse-proxies `/v1/*` to `localhost:3000`, and serves the SPA from `/srv/shape/web/registry` with SPA fallback. The bare-metal alternative (`deploy/shape-registry.service`) is a systemd unit running as user `shape-registry` with `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome=true`, `ReadWritePaths=/srv/shape/registry/data`, `PrivateTmp=true`, and `EnvironmentFile=/srv/shape/registry/.env`. Note the container itself binds `0.0.0.0:3000` (`LISTEN_ADDR`) but the *published* port is loopback-only, and the app-layer 50 MB body limit + CORS predicate + rate limits provide the request-level guards.

**App** (`shape-infra/nixos/shape-prod.nix`): nginx origin router on loopback ports 8091 (play) / 8092 (notebook) proxies to shape-server on `127.0.0.1:9091`; a Cloudflare tunnel maps `play.shape-lang.dev` / `notebook.shape-lang.dev` to those loopback ports. The firewall (`networking.firewall.allowedTCPPorts = [ 22 ]`) closes everything except SSH — 9091 is never directly reachable. The systemd hardening on `shape-server` is the strongest in the tier (§10.7).

Both services therefore keep their app port off the public interface and front it with a proxy + tunnel. The difference that matters is *what runs behind the proxy*: the registry behind its proxy is a hardened metadata service; the app behind its proxy is an unsandboxed code executor (§9.2).

### 1.6 Cross-boundary contracts (the connective tissue)

Three contracts stitch the tier together, and each is a drift surface analyzed later:

1. **HTTP API** — CLI `registry_client.rs` ↔ registry `routes/`. Consistent (see §5, §10).
2. **`PackageBundle` msgpack format** — CLI (produced with HEAD `shape-runtime`) ↔ registry (parsed with crates.io `shape-runtime =0.1.6`). Version-skewed (see §5, §6).
3. **Book doc IDs** — book `.mdx` tree ↔ MCP hardcoded ID maps. **Drifted / broken** (see §8, §9.1).

---

## 2. Feature completeness

Legend: ✅ works end-to-end (verified) · 🟡 code exists, partial / unverified end-to-end · 🔴 stubbed or broken · ⬜ missing.

### 2.1 shape-registry

| Feature | State | Evidence |
|---------|-------|----------|
| Package search (pg_trgm + FTS, sortable) | 🟡 | `services/search.rs:6-111` — well-formed SQL, but no runtime test (needs Postgres); logic reviewed, ranking formula sound |
| Package info + versions + owners | ✅ code / 🟡 runtime | `routes/package_info.rs:8-78`; but see `has_source`/`native_targets` gap below |
| Version info | 🟡 | `routes/package_info.rs:80-110` — hardcodes `has_source:false`, `native_targets:Vec::new()` (partial) |
| Publish (multipart: shapec + source + native) | 🟡 | `services/publish.rs:25-105`; full pipeline present; blob-cleanup on failure implemented |
| Publish (legacy octet-stream) | ✅ code | `routes/publish.rs:50-62`, `services/publish.rs:114-122` |
| Ownership / first-publish auto-claim | ✅ code | `services/publish.rs:153-183, 398-414` |
| Manifest integrity verify | ✅ code | `services/publish.rs:186-193` calls `manifest.verify_integrity()` |
| Ed25519 signature verify | 🟡 | `services/publish.rs:199-226` — verifies **iff** a signature is present; signatures are **optional** (see §9.4) |
| Key→user binding (TOFU auto-bind) | 🟡 | `services/publish.rs:232-250` — auto-binds on first use; no external trust root |
| Yank / unyank (owner-gated) | ✅ code | `routes/yank.rs:23-63` |
| Sparse index (TOML) for resolver | ✅ code | `services/index_gen.rs:8-61` |
| Download (bundle / source / native) | 🟡 | `routes/download.rs` — yanked-version filter on main bundle; source/native gated by `version_blobs` |
| Download counters + daily stats | ✅ code | `routes/download.rs:31-48` fire-and-forget |
| Trending / stats / dependents / categories / downloads-timeseries | ✅ code | `routes/stats.rs`, `dependents.rs`, `categories.rs` |
| Native-dependency metadata extraction | ✅ code | `services/publish.rs:259-322` |
| Auth: register (admin-gated) / login / token / validate | ✅ code | `routes/auth_routes.rs:75-217` |
| Argon2 password hashing | ✅ | `routes/auth_routes.rs:104-108` |
| Login rate-limit + lockout | ✅ code | `routes/auth_routes.rs:34-66` |
| Per-IP API rate limiting | ✅ (tested) | `rate_limit.rs:184-224` — the one unit test in the crate |
| Token expiry (90d) + cleanup | ✅ code | `auth.rs:52-63` |
| Frontend SPA (SvelteKit) | 🟡 | `frontend/` present; README points to wrong location (§8) |

**Publish pipeline correctness walkthrough** (`services/publish.rs:124-364`). The 12 steps, each assessed:

1. *Body size* — enforced by the tower-http `RequestBodyLimitLayer(50 MB)` (`main.rs:70`). ✅ Correct layer-level guard.
2. *Deserialize* `PackageBundle::from_bytes` — errors mapped to 400. ✅ (but see version-skew §5.6).
3. *Validate name* `^[a-z][a-z0-9_-]{0,63}$` (`:494-513`) — first char must be lowercase letter; prevents leading digit/dash and, crucially, path-traversal via `name` (no `.`/`/`). ✅ Security-relevant validator, correct.
4. *Validate semver* via `semver::Version::parse` — prevents traversal via `version` in the *write* path too. ✅.
5. *Duplicate/ownership* — existing package → owner check → duplicate-version conflict (`:147-183`). ✅ Enforces immutability ("version cannot be overwritten") and authz.
6. *Manifest integrity* `verify_integrity()` per manifest (`:186-193`). ✅ Content-hash self-consistency.
7. *Signature verify* — **conditional** on signature presence (`:199-226`); enforces "all manifests same key". 🟡 Optional (§9.4).
8. *Key bind* — TOFU auto-bind (`:232-250`). 🟡 (§9.4).
9. *Checksum* SHA-256 over raw body (`:253`). ✅.
10. *Docs + native-dep extraction* (`:255-322`). ✅ Reshapes into DB rows.
11. *Write blob* before the transaction (`:325`). ✅ With cleanup-on-failure (§10.3).
12. *Atomic transaction* upsert package/version/deps/docs/blobs (`:328-345, 367-483`). ✅ Real DB transaction with commit.

The pipeline ordering is correct (validate-then-write-then-transact, with compensating cleanup). The one semantic wrinkle is step 12's package-update branch, which sets `description`/`license`/`repository` to `bundle.metadata.name`/`None`/`None` (`:388-396`) — i.e., on re-publish it *overwrites* the package description with the package **name** and nulls license/repository. That looks like a copy-paste bug: `UPDATE packages SET description = $1` binds `&bundle.metadata.name` (the name, not a description) and hardcodes `None` for license/repository. So publishing a second version of an existing package clobbers its description with its own name and wipes license/repository. Filed as a latent correctness bug (P2) — the *create* branch (`:398-414`) doesn't set description at all (defaults to `''`), so descriptions are effectively never populated correctly on this path.

**Verdict:** the registry is *feature-broad* — it covers the full crates.io-analog surface — but *shallowly verified*. Exactly one unit test exists (rate limiting); every SQL path is untested against a live database. The multi-blob (source/native) feature is asymmetric: it is written on the *publish* and *download* paths but the *info/read* path hardcodes `has_source:false` and `native_targets:[]`, so a client asking "does this version have source?" always gets "no" even when a source blob was published.

Evidence for the read-path gap (`routes/package_info.rs:64-68`):

```rust
                        native_deps,
                        has_source: false,          // ← always false
                        native_targets: Vec::new(), // ← always empty
```

Both `version_info` (line 107-108) and `package_info` (line 66-67) hardcode these. The `version_blobs` table that the download endpoints query for `blob_type='source'`/`'native'` is never consulted by the info endpoints. So the frontend and CLI cannot discover published source/native artifacts through the metadata API — they would have to blind-probe the download endpoints.

### 2.2 shape-app / shape-server

| Feature | State | Evidence |
|---------|-------|----------|
| One-shot code execution (`/v1/api/execute`) | ✅ (tested) | `execute.rs:83-192`; unit tests `execute_print_returns_display_output` etc. |
| Print-output capture | ✅ | `execute.rs:210-233` via `SharedCaptureAdapter` (the round-2 eval "print not captured" flag is fixed here) |
| Structured value + display + chart-spec | ✅ | `execute.rs:100-143`, `detect_chart_spec:246-258` |
| Chart-spec from Content / Arrow table | 🟡 | `execute.rs:246-277`; image rendering explicitly disabled (`render_chart_if_applicable` returns `None`, line 237-240) |
| Notebook CRUD + per-cell execution | ✅ code | `routes/notebook.rs` (1,335 LoC); `execute_repl` at `:366` |
| LSP-over-WebSocket proxy | 🟡 | `routes/lsp.rs`; spawns `shape-lsp` binary |
| Editor assists (hover/completions/inlay) | ✅ code | `routes/{hover,completions,inlay_hints}.rs` |
| Optional registry-backed auth | ✅ code | `auth.rs:15-71`; **off by default** (`SHAPE_REQUIRE_AUTH` unset) |
| Per-client rate limiting | ✅ code | `rate_limit.rs` |
| Execution timeout | 🔴 (ineffective for CPU loops) | `execute.rs:93-97`; see §9.2 |
| Capability sandbox on user code | 🔴 **absent** | `stdlib_cache.rs:57-67` never sets `granted_permissions`; §9.2 |
| Resource caps (instructions/memory) on user code | 🔴 **absent** | `ResourceLimits::default()` → `max_instructions:None` |

**Verdict:** the playground/notebook works end-to-end for benign programs (I ran equivalent code through the same engine path). The *product* is functional. The *sandbox*, which the focus note explicitly asks about ("it executes arbitrary Shape — with what limits?"), is effectively **none at the application layer** — the only containment is the deployment-time systemd sandbox (§9.2, §6).

### 2.3 shape-mcp

| Tool / feature | State | Evidence |
|----------------|-------|----------|
| `search_shape_docs` (TF-IDF + trigram + synonyms) | ✅ | `loader.rs:548-605`; rebuild transcript: relevance scoring works |
| `get_shape_syntax` — functions/types/variables/strings/… | ✅ (subset) | 16/21 constructs resolve; transcript §9.1 |
| `get_shape_syntax` — traits/async/modules/packages/projects | 🔴 **broken** | `tools.rs:34,37,50,51,52` map to non-existent `advanced/*` IDs; §9.1 |
| `get_shape_api` — math/http/json/io/time/state | 🔴 **all broken** | `tools.rs:689` builds `stdlib/{module}`; book has `stdlib/{core,native}/…`; §9.1 |
| `get_shape_examples` | ✅ | `tools.rs:342-404`; `examples/` prefix present in book |
| `run_shape_code` | 🟡 | works via managed serve; description stale (§9.5) |
| `run_shape_file` / `run_shape_project` / `validate_shape` | 🟡 code | `executor.rs:127-167`; require a working `shape` on PATH |
| `reload_shape_server` | ✅ code | `executor.rs:107-114` |
| `search_shape_packages` | 🟡 | `tools.rs:595-674`; hits live `pkg.shape-lang.dev`; eval saw empty registry |
| Resources: 12 grammar + 6 stdlib + overview | 🔴 9/21 broken | `resources.rs:142-187`; §9.1 |
| Prompts | 🟡 code | `prompts.rs` (not exercised here) |
| Content sourced from book at build time | ✅ | `build.rs:11-71`; `loader.rs:27,417` |

**Verdict:** the *infrastructure* (JSON-RPC loop, content pipeline, search scorer, managed executor) is complete and good. The *retrieval surface* is partially broken by doc-ID drift: the two most valuable "teach me the language" tools (`get_shape_syntax` for traits/async/modules, `get_shape_api` for every stdlib module) return "Unknown …" against the current book. This is a shipped tool silently failing its core purpose.

**Prompts survive the drift.** The three MCP prompt templates — `write-shape-function`, `write-shape-type`, `debug-shape-error` (`prompts.rs:9-70`) — are *static* templates parameterized by name/description/params, with no dependency on the book doc-ID maps. They are unaffected by the reorg and still work. So the drift is localized to the *content-retrieval* tools that resolve names to book IDs (`get_shape_syntax`, `get_shape_api`, `resources/read`), not to `search_shape_docs` (which fuzzy-matches over all `BOOK_ENTRIES` and therefore *does* find content), nor to prompts, nor to the execution tools. This localization is exactly why the fix is to route the exact-lookup tools through the same fuzzy path that already works.

### 2.4 packages/

| Package | State | Evidence |
|---------|-------|----------|
| xgboost (pure Shape) | 🔴 **does not compile vs HEAD** | `string as number` rejected at `index.shape:45`; §9.3 |
| duckdb (native binding) | ⬜ unverifiable | native-dep preflight fails (no `libduckdb.so` in env); Shape compile not reached; §9.3 |

**Verdict:** neither shipped sample package is demonstrably runnable against HEAD in this environment. xgboost is *definitively broken* by a language rule change. duckdb is blocked earlier, at native-dependency preflight, so its Shape source cannot even be type-checked here.

### 2.5 shape-infra

| Feature | State | Evidence |
|---------|-------|----------|
| NixOS prod host (systemd + nginx + cloudflared) | ✅ code | `nixos/shape-prod.nix` |
| systemd hardening for shape-server | ✅ | `shape-prod.nix` serviceConfig (§6) |
| Debian pull-deploy (release fetch + SHA verify) | ✅ code | `debian/pull-deploy/shape-deploy-app.sh` |
| CI / deploy workflows | 🟡 code | `.github/workflows/` (not executed here) |

**Verdict:** deployment configs are complete and notably security-conscious (systemd hardening, SHA-256 asset verification, firewall-closed app port behind a tunnel). This is the healthiest sub-territory.

---

## 3. Code quality

### 3.1 Idiom and naming

The Rust is consistently idiomatic and reads like it was written by someone fluent in the axum/tokio/sqlx idiom:

- Registry error handling funnels through a single `AppError` type with named constructors (`bad_request`, `forbidden`, `not_found`, `conflict`, `too_many_requests`, `internal`) and a `From<sqlx::Error>` that logs the real error and returns a generic "internal database error" to the client (`error.rs:68-73`) — correct information-hiding.
- Extractors are used properly: `AuthUser` implements `FromRequestParts` (`auth.rs:20-35`) so auth is compositional at the handler-signature level.
- sqlx is used with bound parameters everywhere (`$1`, `.bind(...)`), so there is **no SQL injection surface** even in the dynamically-assembled search/order queries — the `format!`-built SQL only interpolates a fixed `order` string chosen from a match arm (`services/search.rs:18-23,61-66`), never user input.
- Naming is clear and consistent across the three Rust projects; module boundaries are sensible.

### 3.2 Error handling

Registry: excellent. Every fallible path returns `Result<_, AppError>`; blob writes are cleaned up on transaction failure (`services/publish.rs:90-102, 347-363`). The one soft spot is that fire-and-forget download-counter updates (`routes/download.rs:31-48`) swallow all errors with `let _ =`, which is acceptable for a best-effort counter.

shape-app: the execute path degrades gracefully — parse/analysis/runtime errors are mapped to structured `ErrorInfo` with line/column extraction and an HTML render of the runtime error payload (`execute.rs:334-382`). Good.

shape-mcp: consistent `anyhow::Result` with contextual `map_err`. The JSON-RPC layer distinguishes notifications (no `id`) from requests and silently ignores unknown notifications per spec (`main.rs:268-279`). Wire framing has a sane 256 MB response cap (`executor.rs:199, 243`).

### 3.3 `unsafe` usage

**Zero real `unsafe` blocks in the entire territory.** The single grep hit in `shape-registry/src` is the string literal `"unsafe_memory"` — a permission *name* in `permission_bits_to_names` (`services/publish.rs:532`), not an `unsafe` block. `shape-app/shape-server/src` and `shape-mcp/src` have zero occurrences. For code that handles untrusted network input and executes untrusted user programs, the absence of hand-rolled `unsafe` is a genuine quality signal — all the memory-safety-critical work is delegated to the language core.

### 3.4 Complexity hotspots

- **`services/publish.rs::publish_bundle_inner`** (240 lines, `:124-364`) is the longest and most branchy function in the registry. It is a documented 12-step pipeline and reads linearly, but it mixes concerns: bundle validation, native-dep metadata reshaping (steps 10b, `:259-322`), blob I/O, and transaction orchestration. `run_publish_transaction` has 14 parameters (`#[allow(clippy::too_many_arguments)]`, `:366-383`) — a sign that a `PublishContext` struct is overdue.
- **`routes/notebook.rs`** at 1,335 lines is by far the largest module in shape-app and warrants a split (CRUD vs execution vs serialization).
- **`content/loader.rs::score_document`** (`:489-542`) is dense with five scoring tiers but is well-commented and unit-tested; complexity here is inherent to the retrieval algorithm and justified.
- **`static/playground.html`** at 1,925 lines is a single inlined SPA; not Rust, but a maintenance liability (no build step, hand-edited HTML/JS/CSS in one file).

### 3.5 Dead code / housekeeping

- `dto.rs::DependencyEntry` is `#[allow(dead_code)]` (`:64-68`) — an unused response type.
- `auth.rs::AuthUser` is `#[allow(dead_code)]` on the `username` field (`:14`) — actually used by `validate`, so the allow is stale but harmless.
- **Committed junk files:** `shape-app/one,n` and `shape-app/two,n` — both 0 bytes, both tracked in git (`git ls-files` confirms). Almost certainly the residue of a shell redirection typo (`> one,n`). They should be removed.
- `shape-app/test_playground.sh` is a tracked ad-hoc test script.
- `shape-mcp/llms-full.txt` is literally a committed placeholder: *"(This is a placeholder. In production, this file would contain the concatenation…)"*.
- **Zero TODO/FIXME/XXX/HACK markers** across all in-territory Rust — either genuinely clean or the markers were scrubbed. Given the absence of `unimplemented!()`/`todo!()` too, this reads as genuine.

### 3.6 Test-to-code ratio

| Project | `#[test]`/`#[tokio::test]` fns |
|---------|-------------------------------|
| shape-registry/src | **1** (rate-limit only) |
| shape-app/shape-server/src | 29 |
| shape-mcp/src | 26 |

The registry — the most security-sensitive component (it verifies signatures, hashes passwords, gates publishing) — has effectively no automated tests. shape-app and shape-mcp are reasonably covered at the unit level (though not at the cross-boundary level that would have caught the MCP drift). See §7.

---

## 4. Duplication & DRY violations

### 4.1 Wire-message types re-declared in shape-mcp (dangerous drift)

`shape-mcp/src/executor.rs:307-389` hand-declares a subset of `shape-vm::remote::WireMessage`, `ExecuteRequest`, `ExecuteResponse`, `ValidateResponse`, `WireDiagnostic`, `ExecutionMetrics`, plus the framing constants `COMPRESSION_THRESHOLD`/`FLAG_COMPRESSED`/`ZSTD_LEVEL` (`:395-397`). The comment is explicit about the risk:

```rust
/// Mirrors `shape_vm::remote::WireMessage` — only the variants MCP needs.
/// Must be serde-compatible with the server's enum.
```

This is a copy of a contract that lives authoritatively in the language core. Because `rmp_serde::to_vec_named` serializes enum variants and struct fields **by name**, adding/reordering variants is tolerated, but a *rename* of a variant or a field on the server side — or a change to the framing constants — would silently break MCP↔serve communication with a "MessagePack decode error" and no compile-time warning. The mitigation (which the code partly relies on) is `to_vec_named` + `#[serde(default)]` on optional fields, but the framing constants (`COMPRESSION_THRESHOLD = 256`, `ZSTD_LEVEL = 3`) are hard-duplicated magic numbers that must match `shape_wire::transport::framing` exactly. shape-mcp already depends on `shape-wire` as a path dependency (`Cargo.toml:26`) — it could import `WireValue` and the framing helpers directly (it imports `shape_wire::WireValue` already) rather than re-deriving them. The `WireMessage` enum lives in `shape-vm`, which shape-mcp does *not* depend on, hence the copy. Divergence risk: **medium** (name-stable serde protects the common case; framing-constant drift is the sharp edge).

### 4.2 Permission-bit → name mapping duplicated from the ABI enum

`shape-registry/src/services/publish.rs:515-541` hardcodes a 16-element array mapping permission bit positions to names:

```rust
    let names = [
        "filesystem_read", "filesystem_write", "network", "env_read",
        "env_write", "process_spawn", "ffi", "stdin", "stdout", "stderr",
        "random", "time", "signal", "ipc", "gpu", "unsafe_memory",
    ];
```

This is a parallel copy of the `Permission` enum's bit layout, which lives authoritatively in `shape-abi-v1` (CLAUDE.md: "Permission enum (16 permissions) — `crates/shape-abi-v1/src/lib.rs:996`"). The registry pins `shape-runtime =0.1.6`, so it *has* access to the ABI enum transitively, but it chose to hardcode the names as strings instead of iterating the enum. If the language ever reorders or renames a permission (the ABI names in the enum are things like `FsRead`, `FsWrite`, `NetConnect` — note these already differ from the registry's `filesystem_read`, `network`), the registry's index/API would report stale permission names. In fact the names *already differ* from the ABI's own `Permission::name()` output, which is a latent inconsistency: a bundle's `required_permissions` as stored by the registry uses `filesystem_read`, while the CLI/runtime speak `FsRead`. Divergence risk: **low-medium** (bit *layout* rarely changes; but the two naming conventions already disagree).

### 4.3 Sort-order match arms duplicated in search

`services/search.rs` duplicates the sort-order match (`downloads`/`recent`/`name`/default) twice — once for the empty-query branch (`:18-23`) and once for the search branch (`:61-66`), with the only difference being the default (`total_downloads DESC` vs `rank DESC, total_downloads DESC`). Minor; a shared helper returning the ORDER BY fragment would remove it.

### 4.4 Wire framing helpers duplicated

`shape-mcp/src/executor.rs:399-434` (`encode_framed`/`decode_framed`) reimplements the length-prefix + zstd framing that `shape_wire::transport::framing` already provides. See §4.1 — same root cause (shape-mcp mirrors rather than imports).

### 4.5 `is_owner` ownership check duplicated

The `SELECT EXISTS(... FROM package_owners WHERE package_id=$1 AND user_id=$2)` ownership check appears verbatim in `services/publish.rs:154-162` and `routes/yank.rs:36-43`. Low risk, but a `services::ownership::is_owner(db, pkg_id, user_id)` helper would centralize the authz predicate — worthwhile precisely *because* it is an authorization check that must not drift.

### 4.6 Client-IP extraction and rate-limit scaffolding

Both `shape-registry/src/rate_limit.rs` and `shape-app/shape-server/src/rate_limit.rs` implement near-identical `governor`-based keyed limiters with `cf-connecting-ip`/`x-forwarded-for` client-key resolution. These are separate repos so sharing is awkward, but the two implementations will drift independently (e.g. one may gain proxy-trust hardening the other lacks). Documented as a cross-repo duplication, not actionable within one repo.

---

## 5. Split-brain analysis

This tier has several "same concept implemented twice" hazards. The most consequential is a **security** split-brain.

### 5.1 Two code-execution surfaces with opposite sandbox postures (security split-brain)

The language exposes user-code execution through two entirely separate paths with **incompatible default security postures**:

- **`shape serve`** (used by the MCP executor): `bin/shape-cli/src/commands/serve_cmd.rs:110-146` maps a `--sandbox` level to a real `PermissionSet` + `ResourceLimits`:
  - `strict` → `PermissionSet::pure()` (nothing) + `ResourceLimits::sandboxed()`
  - `permissive`/`moderate` → `pure` + `FsRead`+`Env`+`Time`+`Random`+`NetConnect` + `sandboxed()` caps
  - `off` → `PermissionSet::full()` + `ResourceLimits::unlimited()`
  - and further narrows/loosens by bind class (`is_loopback`, `:146`).
- **shape-app engine-direct** (used by `/v1/api/execute` and the notebook): `shape-app/shape-server/src/routes/stdlib_cache.rs:57-67` builds the engine with `ShapeEngine::new()` + `apply_bootstrap_state()` and **never attaches a `PermissionSet` or `ResourceLimits`**. Per `shape-runtime/src/module_exports.rs:219-226`, a `None` permission set means *all operations allowed*; per `shape-vm/src/resource_limits.rs:22-27`, the default `ResourceLimits` has `max_instructions: None`.

So the careful sandbox envelope that the CLI author built for `shape serve` is **not on the path the public playground actually uses.** The playground is strictly *more* permissive than even `shape serve --sandbox off` (both grant all permissions, but the playground also has no resource caps and no deterministic-mode exclusion). Two implementations of "run untrusted Shape", one hardened, one wide open, and the wide-open one is the internet-facing product. This is the textbook split-brain: the safe implementation exists, but the dangerous sibling is what ships. Drift evidence: the serve path has evolved a rich sandbox model; the shape-app path has not tracked it at all. (See §9.2 for full impact and deployment mitigation.)

### 5.2 Doc-ID maps vs book directory layout (doc-vs-code split-brain — already drifted)

The MCP maintains **three** hardcoded maps from user-facing names to book doc IDs:
- `tools.rs::resolve_construct` (`:27-56`) — for `get_shape_syntax`
- `tools.rs::get_api`'s `format!("stdlib/{module}")` (`:689`) — for `get_shape_api`
- `resources.rs::grammar_to_doc_id` (`:142-160`) + the stdlib-URI passthrough (`:180-184`) — for `resources/read`

All three are parallel tables that must stay in lockstep with the book's `.mdx` directory structure, which `build.rs` auto-discovers. When the book restructured (`stdlib/*.mdx` → `stdlib/{core,native,domain,math}/*.mdx`; `advanced/{traits,async,modules}` → `fundamentals/{traits,async,modules}`), `build.rs` regenerated the embedded IDs but the three maps did not follow. **This split-brain has already drifted and is broken** (§9.1). It is the concrete instantiation of the tier's central risk.

### 5.3 `PackageBundle` producer vs consumer version skew (format split-brain)

The `.shapec` bundle is produced by the CLI (built from HEAD `shape-runtime`, version `0.3.2` per the workspace) and consumed by the registry's `PackageBundle::from_bytes` (built from crates.io `shape-runtime =0.1.6`, `shape-registry/Cargo.toml:12`). The bundle's serde/msgpack layout is defined once in `shape-runtime` but the two ends were compiled from different versions of that definition. If any field of `PackageBundle`, `ModuleManifest`, `ModuleSignatureData`, `DocItem`, or `NativeDependencySpec` changed shape between 0.1.6 and 0.3.2, a bundle the current CLI produces may fail to deserialize on the registry, or (worse) deserialize with silently-dropped fields. This cannot be fully verified without standing up the registry against a live publish, but the version gap is a real, unmonitored drift surface.

### 5.4 Native-platform naming (config duplication)

`services/publish.rs:263-291` reshapes `NativeDependencySpec` into a registry-local `NativeDepEntry` with a hardcoded `["linux","macos","windows"]` platform universe (`:264, 288, 306`). The same platform triple appears three times in one function. If the language adds a target (e.g. `wasm`), this local universe silently excludes it. Low risk, contained to one function, but it is a config value duplicated inline.

### 5.6 Version-skew matrix (ecosystem ↔ language HEAD)

The focus notes ask for an explicit version-skew matrix. Here is how each ecosystem piece links to the language core, and the resulting drift risk:

| Piece | How it links to core | Effective core version | Drift risk |
|-------|----------------------|------------------------|------------|
| **shape-registry** | crates.io `shape-runtime = "=0.1.6"` (`Cargo.toml:12`) | **0.1.6** (published crate) | **High** — parses `PackageBundle`/`DocItem`/`ModuleSignatureData` with a version different from the HEAD (0.3.2) CLI that produces them. Format-drift risk on every publish. |
| **shape-app** | path `../shape/crates/{shape-runtime,shape-vm,shape-wire,shape-lsp}` at `=0.3.2` (`Cargo.toml:26-29`) | **HEAD (0.3.2, dirty tree)** | Low for format; but tracks the *dirty working tree* including in-progress GC work, so behavior matches whatever is on disk. |
| **shape-mcp** | path `../shape/crates/shape-wire`; book via `build.rs` include_str! | `shape-wire` HEAD; **book content = whatever the book is at build time** | **High for content** — the embedded book snapshot drifts from the ID maps (§9.1); the prebuilt binary (Mar 15) embeds a *different* book than a rebuild today. |
| **packages/xgboost** | `use std::core::{file,json}` resolved at run time by whatever `shape` is on PATH | HEAD (via the running binary) | **Broken** — HEAD's numeric rules reject the package's idioms (§9.3). |
| **packages/duckdb** | `extern C` + `[native-dependencies]`, `from std::core::native` | HEAD | Unverifiable (native-dep-gated); uses `out`-param + comptime features that may have evolved. |
| **shape-infra** | deploys prebuilt release binaries (SHA-verified) | whatever release tag is deployed | Decoupled by design (deploys artifacts, not source). |
| **CLI (producer of bundles)** | workspace member | HEAD (0.3.2) | — (reference point) |

**The critical cell** is shape-registry's `=0.1.6` pin against a HEAD CLI at `0.3.2`. The registry and the CLI are the two ends of the `PackageBundle` serde contract, and they are compiled from *different definitions of that contract*. Whether this actually breaks depends on whether `PackageBundle`/`ModuleManifest`/`ModuleSignatureData`/`DocItem`/`NativeDependencySpec` changed shape between 0.1.6 and 0.3.2 — which I could not verify without standing up the registry against a live publish. But it is precisely the kind of unmonitored skew that ships a "invalid bundle" error to the first user who publishes after a format change. shape-app avoids this by using path deps; the registry should too, or the crate should be re-published and the pin bumped per release (§12 rec 8).

Note the crate-version numbering itself is confusing: `shape-runtime`'s crates.io version (0.1.6) is decoupled from the *language* release version (0.3.2 workspace). A reader cannot tell from "0.1.6" whether the registry is one release behind or many — the crate semver and the language semver are independent axes.

### 5.5 CORS origin policy divergence

The registry uses a *predicate* CORS policy allowing only `shape-lang.dev`/`pkg.shape-lang.dev`/`localhost:*` (`main.rs:86-119`), while shape-app uses `CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any)` (`main.rs:99-102`). Two services in the same trust boundary with opposite CORS strictness. The registry's is correct and tight; shape-app's is wide open. Because shape-app's execute endpoint is (by default) unauthenticated and unsandboxed, `allow_origin(Any)` means *any website a victim visits can POST code to a reachable shape-server*. See §9.2.

---

## 6. ADR & spec conformance

The ADRs binding the language core (ADR-005 single-discriminator, ADR-006 value/memory model, the runtime-v2 spec, and the Forbidden-Patterns catalogue) govern `shape/crates/*`. The ecosystem tier is **downstream of** these ADRs — it consumes the runtime as a library and does not implement the value model, slot ABI, or dispatch machinery. So the conformance question for this territory is narrower: *does ecosystem code introduce any of the Forbidden Patterns, and does it correctly respect the interfaces the ADRs define?*

### 6.1 Forbidden Patterns (CLAUDE.md §Forbidden) — clean

I grepped the territory for the forbidden-symbol families (`ValueWord`, `synthesize_value_word_from_raw`, `is_tagged`, `normalize_persisted_for_slot`, `SlotKind::Dynamic`/`Unknown`, `exec_*_dynamic_fallback`, generic un-suffixed opcodes, the `(decode|tag|kind|dispatch|…) (bridge|probe|helper|…)` regex, `MethodFnV2`/`call_value_legacy`/`call_value_raw_u64`).

**No Forbidden Pattern appears in the ecosystem code.** The territory never touches slots, `NativeKind`, `HeapValue`, opcodes, or dispatch — it works entirely at the `WireValue`/`PackageBundle`/`DocItem`/`ShapeEngine::execute` interface. The one place that comes near the value model is `shape-app/shape-server/src/routes/execute.rs::format_wire_value` (`:280-331`), which exhaustively matches `WireValue` variants — this is the *public wire type*, not the internal `HeapValue` discriminator, and exhaustive matching on it is exactly correct (no wildcard fallthrough, no tag synthesis). ✅ **Conformant.**

### 6.2 ADR-005 (single discriminator) — not applicable, correctly respected

ADR-005 forbids sum types that project 1:1 to `HeapKind`. The ecosystem introduces no such type. `WireValue` (the serialization type the tier uses) is the sanctioned wire projection, not a parallel heap discriminator. `execute.rs` dispatches on `WireValue` and the MCP dispatches on `WireValue` for rendering — both consuming the wire type, not re-deriving a heap discriminator. ✅ **No violation.**

### 6.3 ADR-006 (value & memory model) — consumed, not implemented

ADR-006's rules (`KindedSlot`, parallel `NativeKind` tracks, cell-storage extensions, method/value-call ABIs) all live below the `ShapeEngine`/`WireValue` boundary. The ecosystem never constructs a slot, never sees a `KindedSlot`, never calls a method-dispatch ABI. The `shape-app` engine path (`stdlib_cache.rs`) uses the public `ShapeEngine::new()` / `apply_bootstrap_state()` / `execute()` surface. ✅ **No touchpoint; nothing to violate.**

There are **no `// ADR-005` or `// ADR-006` marker comments** anywhere in the territory — appropriate, since those markers flag code that participates in the value model, and none of this code does.

### 6.4 Security-model spec (CLAUDE.md §Security Model) — partially respected, one gap

CLAUDE.md specifies three security tiers: compile-time capability checking, runtime permission gating (`check_permission`, "every stdlib I/O call guarded"), and resource sandboxing (`ResourceLimits` presets `unlimited()`/`sandboxed()`).

- **`shape serve` (MCP path):** correctly maps sandbox levels to `PermissionSet` + `ResourceLimits` (`serve_cmd.rs:110-146`). ✅ **Conformant** — this is exactly the model the spec describes.
- **shape-app engine-direct path:** **does not apply either tier 2 or tier 3.** The spec says runtime permission gating guards every I/O call — but `check_permission` is a no-op when `granted_permissions` is `None` (`module_exports.rs:219-226`: *"If `granted_permissions` is `None`, all operations are allowed (backwards compatible…)"*), and the shape-app engine never sets it. The spec's `ResourceLimits` presets exist but shape-app uses the `Default` (no caps). 🔴 **Non-conformant on the shape-app execution surface.**

The `None`-means-all-allowed backward-compat escape hatch in `check_permission` is itself worth flagging against the spec's intent: the spec frames permission gating as always-on ("~5ns per call"), but the implementation makes it opt-in per execution context, and the ecosystem's most exposed consumer opts out by omission. This is the security equivalent of a "documented FFI-boundary fallback" — a compat escape hatch that becomes the default posture for anyone who forgets to set the field. (This is a *runtime-core* design issue surfaced by an ecosystem consumer; the fix could be either "make shape-app set `sandboxed()`" or "make the default deny".)

### 6.5 Ed25519 signing model vs the spec's "trust root" implication

CLAUDE.md describes "Package signing: Ed25519 signatures on module manifests via `ModuleSignatureData`". The implementation (`shape-runtime/src/crypto/signing.rs:46-55`) verifies a signature against the public key *embedded in the signature itself*:

```rust
    pub fn verify(&self, manifest_hash: &[u8; 32]) -> bool {
        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.author_key) else { return false; };
        // ... verifies signature over manifest_hash using self.author_key
    }
```

This proves **integrity** (the manifest hash was signed by *whoever holds the private key for `self.author_key`*) but establishes **no authenticity against any trust root** — the key is self-asserted. The registry's only binding is trust-on-first-use (`services/publish.rs:232-250`: auto-bind the key to the publishing user on first use). There is no key registration/verification ceremony, no key revocation, no CA, no keyserver. This is a *defensible* design (it matches cargo's "the registry is the trust root, signatures are integrity-only" posture) but the spec language ("package signing") implies more than the code delivers, and the registry README overstates it (§8). Marked as a conformance *nuance*, not a violation — the code does what it does correctly; the documentation implies a stronger guarantee. Detail and severity in §9.4.

### 6.6 Version pinning vs "content-addressed bytecode" spec

The spec emphasizes content-addressed bytecode with permissions baked into the hash. The registry stores `bundle_sha256` and `required_permissions` per version and computes the checksum over the raw body (`services/publish.rs:253`), which is consistent with the content-addressing model. The permission list is derived from `manifest.required_permission_bits` (`:200`). ✅ **Conformant** with the content-addressing surface, modulo the version-skew caveat that the *bit→name* mapping is a local hardcode (§4.2).

---

## 7. Test coverage in-territory

### 7.1 Counts and character

| Project | test fns | Character |
|---------|----------|-----------|
| shape-registry | **1** | `rate_limit::tests::read_rate_limit_returns_429_and_retry_after` — a real integration-style test using `tower::ServiceExt::oneshot`, asserting 429 + `Retry-After` header after quota. Good test; but it's the *only* one. |
| shape-app/shape-server | 29 | `execute.rs` has 6 (print capture, expression value, combined display, uncaught-exception HTML, error-info location precedence). The rest are spread across routes. Assertions are specific (`response.display.as_deref() == Some("3")`, `value["Integer"] == json!(3)`). |
| shape-mcp | 26 | `content/loader.rs` has 20 (trigram math, synonym expansion, IDF weights, typo tolerance, budget respect, real searches like "struct"→types, "macro"→comptime). `executor.rs` has 5 (msgpack roundtrips, framing). Assertions are meaningful. |

### 7.2 Registry coverage gap (the important one)

The registry has **no test** for:
- The publish pipeline (`publish_bundle_inner`) — the 12-step security-critical path, including signature verification, ownership checks, and the duplicate-version conflict.
- `validate_package_name` — a security-relevant validator (it prevents path traversal via the package name).
- Signature verification integration (only the `signing.rs` unit tests in `shape-runtime` cover the crypto itself).
- The auth flow (register/login/token/validate), including the argon2 hashing and the login-lockout logic.
- Any SQL query (all are untested against a live schema; a column rename or a query typo ships silently — indeed migration `003` shipped with a *"syntax error"* that a later commit `7497206` had to fix, per git log).

For a service whose job is to gate who can publish what and to verify signatures, one test is a serious gap. The `governor` rate-limit test even hardcodes a Postgres DSN in `test_state` (`rate_limit.rs:165-169`), which means it only runs where a DB is reachable — but since the tested handler is a trivial `StatusCode::OK` stub, the DB isn't actually touched (the pool is lazy). So even the "integration" test is effectively a unit test.

### 7.3 shape-mcp: unit-tested but not cross-boundary-tested

The 20 loader tests exercise the *search algorithm* thoroughly (`test_search_finds_types_for_struct_query`, `test_search_finds_comptime_for_macro`, etc.). Critically, they test *search* (fuzzy, forgiving) but **not the exact-ID lookups** that `get_shape_syntax`/`get_shape_api`/`resources::read_resource` depend on. There is no test asserting `resolve_construct("traits")` resolves to an ID that actually exists in `BOOK_ENTRIES`. Had there been a single test like:

```rust
for construct in ALL_CONSTRUCTS { assert!(index.get(resolve_construct(construct)[0]).is_some()); }
```

…the book-reorg drift (§9.1) would have failed the build the moment the book restructured. The existing tests pass *because* search is fuzzy and finds *something* for every query, masking the exact-lookup breakage. This is a textbook case of "the tests test the forgiving path, not the strict path."

### 7.4 shape-app: good unit coverage, no sandbox test

The `execute.rs` tests are solid for functional behavior. But there is **no test asserting that user code cannot read files / cannot spawn processes / is capped in instructions** — i.e., no test encodes the sandbox contract, because there is no sandbox contract (§9.2). A regression test like `assert!(execute("use std::core::file\nfile::read_text(\"/etc/passwd\")").is_err())` would both document the intended posture and fail today (it would *succeed* at reading the file). The absence of such a test is consistent with the absence of the sandbox.

### 7.5 Ignored tests

There are **no `#[ignore]` tests** in the territory (grep clean). So there is no "these are known-broken and parked" backlog here — the coverage is simply thin (registry) or narrow (mcp exact-lookup, shape-app sandbox), not deliberately disabled.

### 7.6 The MCP eval harness (a different kind of "test")

`shape-mcp/eval/` contains manual eval transcripts (`round-1`, `round-2`) run by an LLM through the live MCP tools. `round-2/summary.md` is a genuinely useful artifact — it caught real issues (print-capture, a json-import doc contradiction). **But it is a snapshot, not a gate.** It records `2.6 All 6 stdlib modules … math, http, json, io, time, state all work` and `2.5 Syntax spot-check … all return rich content` — both **PASS** — because it was run when the book was flat. It is now *stale evidence of correctness*: the same tests fail today (§9.1). An eval artifact that is never re-run becomes a liability — it asserts health that no longer holds. This is the audit's cautionary tale in miniature: a "PASS" with no re-run cadence is indistinguishable from a "was PASS once".

---

## 8. Book / docs vs reality for this vertical

### 8.1 Registry README overstates signature verification and mislocates the frontend

`shape-registry/README.md` describes the publish pipeline step 7 as an unconditional gate:

> 7. Verify Ed25519 signatures (`ModuleSignatureData::verify()`)

But the code makes signature verification **conditional on a signature being present** (`services/publish.rs:202` — `if let Some(sig) = &manifest.signature`). An unsigned bundle sails through step 7 entirely. The README's numbered list reads as "signatures are required and verified"; the reality is "signatures are verified iff supplied, and supplying one is optional." **Doc overstates the guarantee.**

The README also states:

> The registry web UI lives in `shape-web/registry/` (SvelteKit SPA).

`shape-web/registry/` **does not exist** (verified: `ls` → No such file or directory). The frontend actually lives in `shape-registry/frontend/` (a SvelteKit static-adapter app, confirmed by `frontend/package.json` and the `.svelte-kit/` build output). **Doc points at a nonexistent path.**

### 8.2 The book (tooling/packages.mdx) is *more accurate* than the registry README

Interestingly, the book's package docs are honest about the exact model the code implements:

- `tooling/packages.mdx:183`: *"On first use of a package from an unknown author, you'll see a Trust-on-First-Use (TOFU) prompt showing the author's signing key."* — matches the auto-bind/TOFU code (`services/publish.rs:232-250`) exactly.
- `:209`: *"Author-bound — signing keys are tied to authenticated registry accounts"* — matches the `user_keys` binding.
- `:155`: *"Every published version is immutable — once published, a version cannot be overwritten"* — matches the duplicate-version conflict (`services/publish.rs:169-182`).

So the *book* correctly describes the trust model as TOFU (integrity + first-use binding, not a CA-rooted authenticity guarantee). The *registry README* is the doc that overstates. When these disagree, trust the book here.

### 8.3 The book's MCP doc describes a different surface than the shipped MCP

`tooling/mcp-server.mdx` describes a **REST API** with endpoints returning `{"modules":{"std:io":[...],"std:time":[...]}}` (`:70-76`). The actual `shape-mcp` is a **stdio JSON-RPC MCP server** with tools named `get_shape_api`, `get_shape_syntax`, etc. (`tools.rs`). The book doc appears to describe an older HTTP-based design or a conflation with `shape serve`'s HTTP surface. A reader following the book to integrate the MCP would look for REST endpoints that the stdio MCP does not expose. **Doc describes a stale/wrong interface shape.** (Lower severity — the MCP is typically wired via an MCP client config, not by reading this doc, but it is inaccurate.)

### 8.4 llms.txt teaches retired and wrong syntax

`shape-mcp/llms.txt` is a hand-written "LLM cheat sheet". It is stale in several concrete, verifiable ways:

| llms.txt claim | Reality | Evidence |
|----------------|---------|----------|
| `int (i48)` | `int` is **i64** | `9000000000000000` (> 2⁴⁷) runs fine — §9.5 transcript; CLAUDE.md "int (i64)" |
| Strings: `c"styled {text:bold}"` | `c"..."` **retired in W18.3** | `c"..."` → *Undefined variable: 'c'* — §9.5; CLAUDE.md "legacy c\"…\" syntax was retired in W18.3" |
| Traits: `trait Name { method(self): T }` | Trait methods use `fn m() -> T` / `method m() -> T` | `greet(self): string` → parse error E0001 — §9.5 |
| `impl Trait for Type { method m() { } }` | `impl Trait for Type { method m() -> T { } }` | book `fundamentals/traits.mdx:79` uses `method display() -> string` |
| Collections: `Vec<T>, HashMap<K,V>` | Canonical is `Array<T>`; `Vec<T>` is accepted as an alias | both `Vec<int>` and `Array<int>` compile — §9.5 |

The good news: `llms.txt` appears to be **vestigial** — it is not referenced anywhere in `shape-mcp/src` (the content pipeline sources everything from the book via `build.rs`). So the MCP does not *serve* `llms.txt` to LLMs. But it is a committed file that will mislead anyone (human or crawler) who reads it, and `llms-full.txt` is an outright placeholder stub. If these files are published at a `/llms.txt` URL (the conventional location), they actively teach wrong syntax.

### 8.5 The book itself is current on trait/method syntax (a positive)

By contrast, the book's `fundamentals/traits.mdx` correctly uses the *current* `method display() -> string { self.name }` syntax and even documents that `method foo(...)` desugars to `fn foo(self, ...)` (`:62-63`). It honestly marks snippets `runnable=false` where the trait-method JIT dispatch gap applies (`:122-124, 152`). So the book — the MCP's actual content source — is up to date on this. The staleness is entirely in the hand-maintained `llms.txt` and the MCP's hardcoded ID maps, **not** in the book content. This matters: it means fixing the MCP is a matter of correcting the ID maps (mechanical), not rewriting docs.

### 8.6 CLAUDE.md trait-method example is itself stale (noted, out of territory)

While verifying, I found that CLAUDE.md's own "Traits" example (`trait Name { fn method(self) -> ReturnType; }` and `impl Trait for Type { ... }`) is stale relative to the compiler, which now rejects explicit `self` in impl methods (*"method receivers are implicit. Use `method greet(...)` without `self`"* — §9.5 transcript). This is outside my territory to fix but is relevant context: the drift between docs and current method syntax is broader than the ecosystem, and the book is actually the most-current artifact.

---

## 9. Bugs & correctness risks found

### 9.1 [P1] MCP retrieval broken for core constructs and all stdlib modules (empirically proven)

**Root cause:** the book was reorganized; the MCP's three hardcoded doc-ID maps were not. `build.rs` auto-discovers `.mdx` files and generates IDs as the path relative to the docs root with `.mdx` stripped (`build.rs:36-47`). The current book yields IDs like `stdlib/native/math`, `stdlib/core/state`, `fundamentals/traits`, `fundamentals/async`, `fundamentals/modules`. But:

- `tools.rs::resolve_construct` maps `"traits" => vec!["advanced/traits"]`, `"async" => vec!["advanced/async"]`, `"modules" => vec!["advanced/modules"]`, `"packages" => vec!["advanced/packages"]`, `"projects" => vec!["advanced/projects"]` (`:34,37,50,51,52`) — none of those IDs exist.
- `tools.rs::get_api` builds `format!("stdlib/{}", module)` (`:689`) — `stdlib/math`, `stdlib/http`, `stdlib/json`, `stdlib/io`, `stdlib/time`, `stdlib/state` — none exist (they are under `stdlib/{core,native}/…`).
- `resources.rs::grammar_to_doc_id` has the same broken `advanced/{traits,async,modules}` maps (`:148,150,157`), and the stdlib resource URIs pass through to `index.get("stdlib/math")` etc. (`:180-184`) — all miss.

**Empirical proof.** I rebuilt `shape-mcp` from source against the current book (`cargo build --release --bin shape-mcp`, 45s — regenerated `book_content.rs` and recompiled `shape-wire`) and drove it over JSON-RPC:

```
$ SHAPE_BIN=…/shape  shape-mcp  < calls.jsonl
id=1: INIT ok
id=2 [get_shape_api math]:    Unknown module: 'math'. Available modules: core/collections, core/distributions, core/log, core/math, core/monte_carlo, core/ode, core/prope…
id=3 [get_shape_api http]:    Unknown module: 'http'. Available modules: core/collections, …
id=4 [get_shape_api json]:    Unknown module: 'json'. Available modules: core/collections, …
id=5 [get_shape_syntax traits]:   Unknown construct: 'traits'. Available constructs: functions, types, enums, traits, pattern-matching, async, comptime, …
id=6 [get_shape_syntax async]:    Unknown construct: 'async'. Available constructs: functions, types, enums, traits, pattern-matching, async, …
id=7 [get_shape_syntax modules]:  Unknown construct: 'modules'. Available constructs: functions, types, enums, traits, pattern-matching, async, comptime, …
id=8 [get_shape_syntax functions]: **Summary:** Functions use `fn name(param: Type) -> ReturnType { body }`. …   ← works
id=9 [resource shape://stdlib/json]:  ERROR Resource not found: shape://stdlib/json
id=10 [resource shape://grammar/traits]: ERROR Resource not found: shape://grammar/traits
```

Note the **self-contradiction**: `get_shape_syntax("traits")` returns *"Unknown construct: 'traits'. Available constructs: functions, types, enums, **traits**, …"* — it lists `traits` as valid in the very error that rejects it. An LLM has no recovery path: the tool advertises `traits` and rejects `traits`. For `get_shape_api`, the error at least lists the *real* IDs (`core/math`, `native/http`), so a persistent LLM could retry with `get_shape_api("core/math")` — but the tool's *description* still says "Available modules: math, http, json, io, time, state", so the LLM would only discover the real names by first failing.

**Confirmation it is a regression, not always-broken:** running the *prebuilt* (Mar 15) binary — whose embedded book was flat — returns real content (`get_shape_api math` → "# stdlib: math", `get_shape_syntax traits` → "# Traits"). And the committed `eval/results/round-2/summary.md` records these tools as **PASS**. So the tools worked when the book was flat and silently broke when it restructured.

**Impact:** the MCP's flagship purpose — teaching an LLM current Shape — is defeated for the most-requested topics (traits, async, modules) and the entire stdlib API surface (math, http, json, io, time, state). An LLM using this MCP to learn Shape gets "Unknown construct/module" for the exact things it most needs. **P1 — a shipped, deployed tool silently fails its core function.**

**Fix:** either (a) update the three ID maps to the current book layout, or better (b) delete the hardcoded maps and resolve by *search over `BOOK_ENTRIES`* (fuzzy match the construct/module name against doc IDs), so future book reorgs cannot break it; and (c) add the exact-lookup test from §7.3 so it fails the build on drift.

### 9.2 [P1] shape-app executes untrusted code with no capability sandbox and no resource cap

**Chain of evidence:**

1. `/v1/api/execute` builds the engine via `create_engine_with_cached_stdlib()` (`execute.rs:207`), which is `ShapeEngine::new()` + `apply_bootstrap_state()` (`stdlib_cache.rs:63-66`) — **no `PermissionSet`, no `ResourceLimits` set.**
2. `check_permission` treats a `None` permission set as all-allowed: *"If `granted_permissions` is `None`, all operations are allowed"* (`shape-runtime/src/module_exports.rs:219-226`).
3. `ResourceLimits::default()` sets `max_instructions: None` (`shape-vm/src/resource_limits.rs:22-27`) — no instruction cap. `BytecodeExecutor::new()` (the executor shape-app uses) carries default limits.
4. The only guard is `tokio::time::timeout(timeout_ms, execute_internal(&code))` (`execute.rs:93-97`). But `execute_internal` calls `engine.execute()` **synchronously** — there is no `.await` inside the VM dispatch loop. A `tokio` timeout can only fire when the awaited future *yields*; a synchronous CPU-bound `loop {}` never yields, so the timeout branch is never polled and the worker thread hangs. This is the well-known tokio blocking-future footgun. Combined with (3)'s absent instruction cap, an infinite loop runs **uncapped and un-timed-out**.

**Empirical proof of capability access** (through the same default-engine path, via the CLI which shares the default-permissive posture):

```
$ cat t_fs.shape
use std::core::file
let t = file::read_text("/etc/hostname")?
print(t)

$ shape run t_fs.shape
atlas-dev          ← arbitrary file read succeeded, no permission prompt/denial
```

The engine read `/etc/hostname` with no permission check. On the shape-app path, `granted_permissions` is `None`, so the same `file::read_text` — and `NetConnect`, `Process`, `Env` — are all permitted for remote-submitted code.

**Deployment mitigation (partial).** `shape-infra/nixos/shape-prod.nix` runs `shape-server` as a hardened systemd unit: `User=shape` (unprivileged), `ProtectSystem=strict` (whole FS read-only except `/var/lib/shape`), `ProtectHome=true` (`/home`, `/root` inaccessible), `PrivateTmp`, `NoNewPrivileges`, `MemoryDenyWriteExecute`, and the firewall exposes only port 22 (the app port 9091 is reached only via nginx@loopback → Cloudflare tunnel). This meaningfully contains the damage:
- Arbitrary file **write** is confined to `/var/lib/shape` (ProtectSystem=strict).
- `/home` and `/root` are hidden (ProtectHome).

**Residual risk after mitigation:**
- **Arbitrary file read** of world-readable paths outside `/home` remains — `/etc/*`, `/nix/store/*` (all package sources/secrets baked into the store), the process's own environment. `ProtectSystem=strict` makes these *read-only*, not *unreadable*.
- **SSRF** — `NetConnect` is permitted, and there is no egress firewall in the unit. Remote-submitted code can reach internal services and, on a cloud host, the metadata endpoint (`169.254.169.254`) → potential credential theft. The Cloudflare-tunnel topology reduces but does not eliminate this (the VM still has outbound network).
- **CPU/memory DoS** — no `MemoryMax=`/`CPUQuota=` in the unit and no VM instruction cap; a `loop {}` pegs a core indefinitely (timeout ineffective), and concurrent requests multiply it. `MemoryDenyWriteExecute` does not bound heap growth.

**Severity: P1.** It is not P0 only because (a) the systemd hardening blocks the worst outcomes (privilege escalation, file write outside state dir, `/home` exfiltration) and (b) `SHAPE_REQUIRE_AUTH` *can* be enabled. But by default the endpoint is unauthenticated (`main.rs:82-84` → `require_auth=false`), CORS is `Any`, and the application-layer sandbox the spec describes is simply absent. The correct posture is for shape-app to build its engine with `PermissionSet::pure()` (or a tightly-scoped set) + `ResourceLimits::sandboxed()` — i.e., use the same envelope `shape serve --sandbox strict` already implements, rather than the default-permissive engine.

### 9.3 [P1] xgboost sample package does not compile against HEAD

```
$ shape run packages/xgboost/index.shape
error[RUNTIME]: Bytecode compilation failed: Semantic error: Cannot assert type 'string' as 'number'
$ echo $?   → 1
```

The offending idiom is at `packages/xgboost/index.shape:45`:

```shape
    let base_score_str = params.get("base_score") as string
    let base_score = base_score_str as number     // ← 'string as number' now rejected
```

Under the current numeric-conversion rules (per project memory: *"numeric conversion implicit ONLY if truly lossless; … string→… needs explicit"* — a string is not numerically convertible via `as` at all), `string as number` is a compile error. Minimal repro:

```
$ printf 'let s: string = "3.14"\nlet n = s as number\nprint(n)\n' | shape run /dev/stdin
error: Cannot assert type 'string' as 'number'
```

The package predates this rule. It uses the same idiom again at `:47` (`num_features_str as int`). The correct modern form would be a parse function (e.g. `number::parse(...)` / `int::parse(...)`), not an `as` cast. **The shipped sample package is broken against the current language.** (Bonus paper-cut: the diagnostic's `-->` location points at line 1, the module doc-comment, not line 45 — a diagnostic-location bug in the core, noted for completeness.)

### 9.3b [P2/unverifiable] duckdb package blocked at native preflight

```
$ shape run packages/duckdb/index.shape
Error: native dependency preflight failed for target 'linux-x86_64':
package 'duckdb@0.1.0':
  - alias 'duckdb' (system) failed to load from 'libduckdb.so': … No such file or directory
```

The native-dependency preflight (a good feature — it fails fast before compiling) blocks execution because `libduckdb.so` is not installed in this environment. This means duckdb's Shape source **cannot be type-checked here**, so I cannot confirm whether it, too, has drifted (it uses `extern C fn … out out_db: ptr`, `from std::core::native use {...}`, and comptime `DESCRIBE` — several features that may have evolved). Marked unverifiable, not confirmed-broken. It is a risk that a package requiring an uninstalled system library is one of only two shipped samples.

### 9.4 [P1] Registry signature model: no trust root, signatures optional, README overstates

The registry's "Ed25519 verification" provides **integrity, not authenticity against any trust root**, and even the integrity check is optional:

1. **Self-asserted keys.** `ModuleSignatureData::verify` (`shape-runtime/src/crypto/signing.rs:46-55`) verifies the signature against the public key embedded *in the signature*. Anyone can `generate_keypair()`, sign their own bundle, and the check passes. There is no keyserver, CA, or external identity binding.
2. **TOFU auto-bind.** On first publish with a given key, the registry binds it to the publishing account (`services/publish.rs:243-248`, `INSERT … ON CONFLICT DO NOTHING`). The `user_keys.key_hex` column is `UNIQUE` (migration `001:16`), so a key can bind to at most one account — which does give a useful property (a second account cannot claim an already-bound key). But the *first* binder of any key is whoever uses it first; there is no proof the key belongs to a real-world identity.
3. **Signatures optional.** The verification loop only runs `if let Some(sig) = &manifest.signature` (`:202`). A bundle with **no** signature passes step 7 unconditionally and publishes with `author_key_hex = NULL`. So "signed packages" is opt-in, and an attacker can simply publish unsigned.
4. **README overstates.** `README.md` lists step 7 as "Verify Ed25519 signatures" with no "if present" qualifier, implying a gate.

This is a **defensible** design (it matches cargo: the registry account + immutability is the real trust root; signatures add tamper-evidence). The book documents it honestly as TOFU (§8.2). The bug is the **gap between what the registry README claims and what the code enforces**, plus the fact that the strongest-sounding security feature (Ed25519 signing) is optional and unrooted. Severity **P1** for the security-posture clarity (a registry operator reading the README would believe signatures are enforced); the underlying design is acceptable if documented accurately and if unsigned publishes are either disallowed or clearly flagged.

### 9.5 [P2] Path traversal via native-blob `target` (authenticated arbitrary file write)

The multipart publish handler extracts the native-blob target from the field name with no validation:

```rust
// routes/publish.rs:33-37
} else if let Some(target) = field_name.strip_prefix("native:") {
    payload.native_blobs.push((target.to_string(), data));   // target unvalidated
}
```

That `target` flows into `blob::blob_filename("native", Some(target))` = `format!("native-{}.tar.gz", target)` (`services/blob.rs:71`), which is joined onto `{blob_dir}/{name}/{version}/` (`services/publish.rs:67`, `blob::write_typed_blob`). Package `name` is validated (`^[a-z][a-z0-9_-]{0,63}$`) and `version` is semver-validated, but `target` is not. A target like `../../../../var/lib/shape/evil` yields a filename `native-../../../../var/lib/shape/evil.tar.gz`; `Path::join` resolves the `..` components and the write escapes `{name}/{version}/` (each `..` climbs one directory, minus one consumed cancelling the `native-..` leading component). An authenticated package **owner** can thus write attacker-controlled bytes to paths outside the intended blob directory.

**Constraints that reduce severity to P2:** (a) the attacker must be an authenticated, registered user who is the owner of *some* package (registration is admin-gated — §10); (b) the filename always carries a `native-` prefix on its leading component and a `.tar.gz` suffix; (c) under the production systemd unit, `ProtectSystem=strict` confines writes to `/var/lib/shape` and `ReadWritePaths=[/var/lib/shape]`, so the blast radius is the registry's own state directory (where it could overwrite *another* package's blobs or the DB-adjacent files). Still, unvalidated path components from request data reaching `fs::write` is a defect. **Fix:** validate `target` against an allowlist regex (e.g. `^[a-z0-9_.-]+-[a-z0-9_.-]+$` for `os-arch` triples) and reject `..`/`/`.

### 9.6 [P2] Registry frontend renders user READMEs through a widened sanitizer (CSS injection)

`shape-registry/frontend/src/lib/markdown/render.ts` renders package READMEs (user-controlled, from `bundle.metadata.readme`) through `remark`→`rehypeRaw` (`allowDangerousHtml`)→`rehypeSanitize`. Applying sanitize *last* is correct, but the schema is heavily widened beyond `defaultSchema`:

- `schema.tagNames` adds `'style'` and `'foreignObject'` (and full SVG).
- `schema.attributes['style'] = ['*']` and `schema.attributes['span'] = [..., 'style', ...]`, `schema.attributes['pre'] = [..., 'style', ...]`.
- `schema.attributes['svg'] = ['*']` (and `path`/`g`/`use`/`foreignObject`/… = `['*']`).

Allowing `<style>` elements and arbitrary `style=` attribute values on user content is a **CSS-injection** surface: a malicious README can inject `position:fixed` overlays (phishing/clickjacking), `display:none` defacement, or CSS that loads external resources (tracking/exfil via `background:url(...)`). Direct JS XSS is *likely* blocked (no event-handler attributes are allowlisted, `script` is not in `tagNames`, and the default `href` protocol allowlist — not overridden — strips `javascript:`), so this is CSS-injection rather than script execution. But `foreignObject` + `svg *` is historically a sanitizer-bypass hotspot and combined with `<style>` warrants tightening. **Fix:** drop `'style'` from `tagNames`, remove the `['*']` on `style`/svg attributes, and constrain SVG to a KaTeX/mermaid-specific attribute allowlist rather than `['*']`.

### 9.7 [P2] `version_info`/`package_info` never report source/native availability

As detailed in §2.1, both info endpoints hardcode `has_source: false` and `native_targets: Vec::new()` (`routes/package_info.rs:66-67, 107-108`) rather than querying `version_blobs`. A version published with a source tarball or native blobs reports having neither through the metadata API. Functional-completeness bug, not a security issue.

### 9.8 [P2] `download` counter uses `tokio::spawn` with a cloned pool per download

`routes/download.rs:31-48` spawns a detached task per download to increment three counters. Under load this creates unbounded detached tasks each holding a pooled connection; the three UPDATEs are also not transactional with each other (a crash between them leaves counters inconsistent). Low severity (best-effort analytics), but at scale the spawn-per-request pattern can exhaust the 20-connection pool (`main.rs:35`). Consider a batched/async-channel counter.

### 9.9 [P3] MCP `run_shape_code` description contradicts implementation

`tools.rs:118` describes the tool as *"Execute Shape code by writing it to a temp file and running it via the `shape` CLI"* — but the current implementation (`executor.rs:117-124`) sends the code over the managed `shape serve` wire protocol; there is no temp file and no CLI shell-out (that was the *old* executor, still present in the prebuilt Mar-15 binary). Stale description; harmless but misleading.

### 9.10 [P3] Diagnostic location bug surfaced via ecosystem code

The xgboost failure (§9.3) reports the error at `<input>:1:1` (the module doc-comment) instead of line 45 where the `as number` cast is. This is a core diagnostic-location bug (the `as`-assertion error loses its span), surfaced here because a real package tripped it. Out of territory to fix; logged because it degrades the debuggability of exactly the kind of package-compilation failure the ecosystem produces.

### 9.11 [P2] Notebook cell execution has *no* timeout at all (worse than one-shot execute)

The one-shot `/v1/api/execute` at least wraps execution in `tokio::time::timeout` (ineffective for CPU loops, but a guard for I/O-bound hangs). The **notebook** execution path has *no timeout whatsoever*. In `notebook.rs`, cells are executed in a loop with a direct blocking call:

```rust
// notebook.rs:363-368
let start = Instant::now();
// execute_repl is async; block_on it from the blocking thread
let exec_result = handle.block_on(engine.execute_repl(&mut executor, content));
let total_ms = start.elapsed().as_millis() as u64;
```

There is no `timeout(...)` wrapper anywhere in the notebook execute path (grep for `timeout` in `notebook.rs` → the only hit is the struct-field name `metrics`, not a real timeout). So a notebook cell containing `loop {}` — submitted by a remote (by default unauthenticated) user — runs forever on a `block_on`-pinned blocking thread, with no permission sandbox and no instruction cap (§9.2). This is the same unsandboxed engine as §9.2, minus even the (ineffective) timeout. **P2** on top of the P1 sandbox gap — it broadens the DoS surface.

Additionally, the notebook **replays every prior cell on each execution** to rebuild REPL state (the module doc-comment states: *"Each execution request creates a fresh engine and replays all cells to rebuild state"*, `notebook.rs:4-5`; the loop runs from cell 0 and only *reports* results from `from_index`, `:312-370`). This is O(n²) work across a notebook session — executing cell *n* re-runs cells 0..n. For a session of many cells this is a quadratic compute amplifier that a user can trivially trigger, compounding the DoS.

### 9.12 [P2] Notebook `SESSIONS` is an unbounded in-memory map with no eviction (memory leak / DoS)

Notebook sessions live in a process-global map:

```rust
// notebook.rs:24-25
static SESSIONS: Lazy<Arc<Mutex<HashMap<String, NotebookSession>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));
```

`create_notebook` inserts a session (`:469`) but **nothing ever removes a session** — there is no TTL, no LRU cap, no eviction. `delete_cell` removes a *cell within* a session, never a session itself. Every notebook a client creates (by default, unauthenticated) accumulates in memory for the lifetime of the process. A trivial loop of `POST /v1/api/notebook` grows RSS without bound until OOM. Combined with the fact that `SESSIONS` holds `ServerCell`s (which retain each cell's `content` and last `output` JSON), the per-session footprint is non-trivial. **P2 memory-leak / DoS.** Fix: bounded LRU + TTL eviction, and count sessions against the per-client rate limiter.

### 9.13 [P2] Re-publish clobbers package description with the package name and nulls license/repository

In the update branch of `run_publish_transaction` (`services/publish.rs:388-396`):

```rust
        sqlx::query(
            "UPDATE packages SET description = $1, license = $2, repository = $3, updated_at = now() WHERE id = $4"
        )
        .bind(&bundle.metadata.name) // Use metadata description if available   ← binds the NAME
        .bind(Option::<&str>::None)  // license → NULL
        .bind(Option::<&str>::None)  // repository → NULL
        .bind(pkg_id)
```

The comment says "Use metadata description if available" but the code binds `bundle.metadata.name` to `description`, and hardcodes `NULL` for `license` and `repository`. So publishing a **second** version of an existing package: (a) overwrites the package's `description` with its own **name**, and (b) wipes any previously-set `license` and `repository`. The `create` branch (`:398-414`) doesn't set `description` at all, so it defaults to `''`. Net result: package descriptions are never correctly populated on the publish path, and re-publishing actively degrades the metadata (name-as-description, nulled license/repo). This is why `package_info` returns `description: None` unless the empty string is stored (`routes/package_info.rs:37` treats `''` as `None`). **P2 correctness bug** — user-visible metadata corruption on every re-publish. Fix: bind the actual `bundle.metadata.description`/`license`/`repository` fields (if `PackageBundle::metadata` exposes them), or leave the columns untouched on update.

---

## 10. What is done well

Specific, named engineering decisions worth preserving:

### 10.1 Registry auth is genuinely well-built

- **Argon2 password hashing** with per-user salt (`auth_routes.rs:104-108`) — the correct choice, not a fast hash.
- **API tokens stored as SHA-256 hashes**, never plaintext (`auth.rs:38-39`, migration comment `001:21`). A DB compromise does not leak usable tokens.
- **Constant-time admin-secret comparison** (`config.rs:38-58`, `constant_time_eq`) — the author correctly recognized the timing side-channel on secret comparison and hand-rolled a branch-free compare. The admin secret is also stored only as a SHA-256 hash (`config.rs:19-21`).
- **Login lockout**: 5 failed attempts → 15-minute lockout, checked *before* doing any work (`auth_routes.rs:34-51, 148`), with success clearing the counter. This is a real brute-force defense, not decoration.
- **Registration is admin-gated** (`auth_routes.rs:80-93`): with no `ADMIN_SECRET` set, registration is *disabled entirely* (returns "registration is disabled"); with one set, it requires the `X-Admin-Secret` header. So the registry is invite-only by default — a sound posture for a young registry (it prevents spam/typosquatting land-grabs).
- **Token expiry with cleanup** (`auth.rs:52-63`): 90-day tokens, and expired tokens are deleted on use.

### 10.2 Parameterized SQL everywhere — no injection surface

Every query binds parameters (`.bind(...)`); the only `format!`-assembled SQL (`services/search.rs`) interpolates a fixed `ORDER BY` fragment selected from a closed match, never user input. For a service with rich search/sort/pagination, keeping the injection surface at zero is a real discipline.

### 10.3 Blob cleanup on transaction failure

The publish pipeline writes the blob to disk *before* the DB transaction, then cleans it up if the transaction fails (`services/publish.rs:90-102` for extra blobs, `:347-363` for the main blob). This avoids orphaned blobs from failed publishes — a detail that is easy to skip and that the author got right, including logging the cleanup failure rather than silently ignoring it.

### 10.4 The MCP content pipeline (single source of truth)

`build.rs` auto-discovers the book's `.mdx` files and embeds them, so the MCP's documentation corpus *is* the book — there is no second copy to maintain (`build.rs:11-71`, `loader.rs:27,417`). This is exactly the right architecture: one source of truth, mechanically embedded. The retrieval scorer (`loader.rs:489-542`) is genuinely sophisticated — five tiers (curated keywords 50pts, title 20pts, section weight multiplier, TF-IDF with log-dampening, trigram-fuzzy typo tolerance), synonym expansion mapping other-language terms to Shape concepts (`struct`→types, `lambda`→functions, `try/catch`→error-handling), all unit-tested. When the IDs match, this is a high-quality retrieval system. *The pipeline is right; only the hardcoded ID maps that bypass it are wrong — which is precisely why the fix is to route everything through search.*

### 10.5 `shape serve` sandbox mapping (the model to copy)

`serve_cmd.rs:110-146` is the *correct* implementation of the security spec: it maps `--sandbox {strict,moderate,off}` to real `PermissionSet` + `ResourceLimits`, narrows by bind class (loopback vs public), and defaults foreign FFI to strict-empty. This is the envelope the shape-app playground *should* be using. It exists, it is careful, it just is not on the shape-app path.

### 10.6 CLI ↔ registry HTTP contract consistency

The CLI's `registry_client.rs` and the registry's `routes/mod.rs` agree on every endpoint: `/v1/api/auth/{register,login,token,validate}`, `/v1/api/packages` (search), `/v1/api/packages/{name}`, `…/{version}/download`, `/v1/index/{name}`, `/v1/api/packages/new` (multipart publish). The `DEFAULT_REGISTRY = "https://pkg.shape-lang.dev"` in the CLI (`config/mod.rs:15`) matches the registry's CORS allowlist (`main.rs:94`) and the MCP's `REGISTRY_URL` (`tools.rs:593`). The *HTTP* contract is coherent across all three consumers — the drift is in the *data* format (bundle serde version), not the endpoint surface.

### 10.7 Deployment security posture (shape-infra)

`nixos/shape-prod.nix` is the healthiest artifact in the territory:
- Comprehensive systemd hardening on the code-executing service (`ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `NoNewPrivileges`, `MemoryDenyWriteExecute`, `RestrictSUIDSGID`, `LockPersonality`, `ProtectKernelTunables`, `ProtectControlGroups`).
- Firewall closed to everything but SSH; app ports reached only via loopback nginx → Cloudflare tunnel.
- Unprivileged system user with a dedicated state dir.
- The Debian pull-deploy verifies release-asset SHA-256 before swapping the binary (`shape-deploy-app.sh` — `sha256sum -c`), and requires a `REVISION` marker.

This hardening is what downgrades the shape-app sandbox gap from P0 to P1. Whoever wrote the infra understood that the app executes untrusted code and defended at the OS layer.

The registry gets the same treatment on both deployment paths: the Docker compose publishes only `127.0.0.1:3000` (not LAN-exposed) behind Caddy, and the bare-metal systemd unit (`deploy/shape-registry.service`) carries `NoNewPrivileges`/`ProtectSystem=strict`/`ProtectHome`/`ReadWritePaths=…/data`/`PrivateTmp`. A nice hygiene detail: commit `e0f9b17` "Remove ADMIN_SECRET placeholder from service file" — the operator deliberately moved the admin secret out of the checked-in unit into the `EnvironmentFile`, avoiding a secret-in-git leak. And the registration gate itself (admin-secret-required, else disabled — §10.1) means the registry ships closed rather than open.

### 10.8 Rate limiting done properly

Both the registry and shape-app use `governor` keyed rate limiters with a sensible client-IP resolution chain (`cf-connecting-ip` first, honoring the actual proxy topology, then `ConnectInfo`, then `x-forwarded-for`/`x-real-ip`). The registry separates read (600 rpm) from write (60 rpm) quotas and returns proper `429` + `Retry-After` (`rate_limit.rs:86-103`). The one registry unit test covers exactly this. Correct and well-scoped.

---

## 11. What is done poorly / tech debt

### 11.1 No drift detection between satellites and core (the meta-debt)

The single largest piece of tech debt is structural: **nothing re-exercises the cross-boundary contracts.** The MCP ID maps drifted and broke with zero signal; the sample packages rot against language changes with zero signal; `llms.txt` teaches retired syntax with zero signal; the registry parses bundles with a 0.1.6 `shape-runtime` while the CLI produces them with 0.3.2 with zero signal. Every one of these is a latent break waiting for someone to notice manually. There is no CI job that (a) rebuilds the MCP and asserts every advertised construct/module resolves, (b) compiles the sample packages against HEAD, or (c) round-trips a HEAD-produced bundle through the registry's parser. The eval artifact (§7.6) is a snapshot masquerading as coverage.

### 11.2 Registry test coverage is near-zero

One test for a service that verifies signatures, hashes passwords, gates publishing, and runs raw SQL. Migration `003` shipped with a SQL syntax error (git log `7497206` "Fix migration 3 syntax error") — exactly the class of bug a single migration-smoke test would catch. The publish pipeline, the name validator, the auth flow, and every query are untested.

### 11.3 Default-permissive execution posture

`check_permission`'s "`None` means all allowed" backward-compat rule (`module_exports.rs:219-226`) makes the *insecure* configuration the *default* one. Any engine consumer that forgets to attach a permission set gets full capabilities. shape-app is the consumer that forgot (or never added it). This is a footgun in the core surfaced by an ecosystem consumer — the safe default would be deny, with an explicit `unlimited()` opt-in for trusted callers (the CLI). As written, forgetting to sandbox is silent and total.

### 11.4 Hardcoded parallel tables (ID maps, permission names, platform triples)

Three hardcoded maps in the MCP (§5.2) and the permission-bit-name array + platform triple in the registry (§4.2, §5.4) are all parallel copies of definitions that live elsewhere. Each is a drift surface. The MCP maps have *already* drifted. The permission names *already* disagree with the ABI's own `name()`.

### 11.5 The hand-mirrored wire protocol

shape-mcp copies `shape-vm::WireMessage` + framing constants (§4.1) rather than importing them, despite already depending on `shape-wire`. A rename on the server side breaks it at runtime with no compile error.

### 11.6 The version-pin mismatch

The registry pinning crates.io `shape-runtime =0.1.6` while the workspace is at `0.3.2` (and shape-app tracks HEAD via path) means the three consumers of the bundle format are built from *three different* versions of the format definition. The registry is the odd one out (crates.io vs path). Either the registry should also use the path dependency (tracking HEAD like shape-app), or the crate should be re-published and the pin bumped in lockstep with releases.

### 11.7 Committed junk and placeholders

`shape-app/one,n`, `shape-app/two,n` (0-byte, tracked), `shape-mcp/llms-full.txt` (a literal placeholder), and `shape-mcp/llms.txt` (stale wrong syntax). These are small but they are *committed* — they signal an absent "is this file real?" review step.

### 11.8 Monolithic hand-edited SPAs

`shape-app/shape-server/static/playground.html` (1,925 lines) and `notebook.html` (474 lines) are single inlined HTML/JS/CSS files with no build step, alongside the *separate* `shape-notebook/` Svelte app that appears to be the intended replacement. Two generations of the UI coexist; the legacy static SPAs are large hand-edited liabilities.

### 11.9 `run_publish_transaction`'s 14 parameters

`#[allow(clippy::too_many_arguments)]` on a 14-parameter function (`services/publish.rs:366-383`) is a smell; a `PublishContext` struct would make the pipeline readable and testable.

### 11.10b Frontend DTO models a subset of the backend response (dead fields on both ends)

The frontend's `VersionInfo` TypeScript interface (`frontend/src/lib/api/types.ts:21-33`) omits `has_source` and `native_targets` entirely — it does not model them. The Rust `VersionInfoResponse` DTO *serializes* them but with `skip_serializing_if` and always-`false`/always-empty values (§9.7), so they never appear in the JSON and the frontend never misses them. The net effect is a feature that is dead on both ends of the read path: the backend computes nothing, serializes nothing, and the frontend models nothing. This is benign today but is a latent inconsistency — if someone later fixes the backend to populate `has_source`, the frontend still would not render it. The two type definitions (Rust DTO and TS interface) are a manual parallel that has already partially diverged (`dto.rs` has `native_deps`, `has_source`, `native_targets`; the TS has `native_deps` but not the latter two).

### 11.10c Registry search ranking is reasonable but untested and Postgres-locale-coupled

The combined trigram+FTS ranking (`services/search.rs:67-92`) computes `GREATEST(similarity(name,q), similarity(description,q)*0.5) + (0.3 if FTS matches else 0)`. This is a sensible hand-tuned formula (name matches beat description matches, FTS presence adds a fixed boost), but: (a) it is entirely untested — there is no fixture asserting that searching "json" ranks a package named `json` above one merely mentioning json; (b) it hard-codes `to_tsvector('english', …)` / `plainto_tsquery('english', …)`, coupling relevance to the English text-search config — a package with a non-English description ranks purely on trigram similarity; (c) the `%` trigram operator depends on `pg_trgm`'s `similarity_threshold` GUC (default 0.3), which is not set explicitly, so relevance silently depends on server configuration. None of these is a bug, but the combination (untested + config-coupled) means search quality can drift with a Postgres upgrade or config change and nobody would notice.

### 11.10 shape-infra is a single "Initial commit"

`shape-infra` has exactly one commit ("Initial commit: Shape infra baseline"). The prod config has a `REPLACE_WITH_CLOUDFLARE_TUNNEL_ID` placeholder (`shape-prod.nix:4`), meaning the checked-in config is a template, not the live config. That is fine for a template, but there is no history/currency signal — no way to tell if it reflects what is actually deployed.

---

## 12. Prioritized recommendations

### P0 — none

No unsound-value-model, wrong-results-in-core, or remote-privilege-escalation defect was found *within the territory*. The shape-app sandbox gap (§9.2) is P1 rather than P0 only because deployment-time systemd hardening blocks the catastrophic outcomes.

### P1 — fix before the next release

1. **Fix the MCP retrieval drift (§9.1).** *Effort: S (½–1 day).* Immediate: update `resolve_construct`, `get_api`'s `stdlib/{module}` pattern, and `grammar_to_doc_id` + stdlib resource URIs to the current book layout. Durable: delete the hardcoded maps and resolve construct/module names by fuzzy search over `BOOK_ENTRIES` so future reorgs can't break them. Add an exact-lookup test (`for c in ALL_CONSTRUCTS: assert index.get(resolve(c)).is_some()`) to fail the build on drift. Re-run the eval and refresh `round-2/summary.md`.

2. **Sandbox the shape-app execution engine (§9.2).** *Effort: S–M (1–2 days).* Build the playground/notebook engine with `PermissionSet::pure()` (or a tightly scoped read-only set matching `serve --sandbox strict`) + `ResourceLimits::sandboxed()` (instruction + memory + wall-time caps). This closes the file-read/SSRF/DoS surface at the application layer, independent of deployment. Run CPU-bound execution on a `spawn_blocking` worker with a hard-kill watchdog so the timeout actually bites. Add `MemoryMax=`/`CPUQuota=` to the systemd unit as defense-in-depth. Add a regression test asserting user code cannot read `/etc/passwd`.

3. **Fix (or retire) the xgboost sample package (§9.3).** *Effort: S.* Replace `string as number`/`as int` with the current parse idiom. Add a CI job that compiles every package in `packages/` against HEAD so sample-package rot fails the build. Decide whether duckdb (native-dep-gated) belongs as a shipped sample or a docs example.

4. **Reconcile the registry signature model with its docs (§9.4).** *Effort: S for docs, M for enforcement.* Either (a) correct the README to state signatures are optional/integrity-only + TOFU (match the book), or (b) make signatures required (reject unsigned publishes) and document the TOFU trust boundary explicitly. At minimum, stop the README implying an unconditional Ed25519 gate.

### P2 — fix soon

5. **Validate the native-blob `target` (§9.5).** *Effort: XS.* Reject `target` containing `/`, `..`, or not matching `^[a-z0-9._-]+$`. A three-line guard in `routes/publish.rs`.

6. **Tighten the registry frontend markdown sanitizer (§9.6).** *Effort: S.* Drop `<style>` from `tagNames`, remove `['*']` on `style`/svg attributes, constrain SVG to a KaTeX/mermaid allowlist. Test with a hostile README fixture.

7. **Wire `has_source`/`native_targets` from `version_blobs` (§9.7).** *Effort: S.* One extra query in the info endpoints; remove the hardcoded `false`/`[]`.

8. **Resolve the `shape-runtime` version pin (§11.6).** *Effort: S.* Switch the registry to the path dependency (tracking HEAD like shape-app), or re-publish `shape-runtime` and bump the pin per release. Add a bundle round-trip test (produce with HEAD CLI, parse with registry) to catch format skew.

9. **Give the registry real tests (§11.2, §7.2).** *Effort: M.* At minimum: a migration-smoke test (apply all three against an ephemeral Postgres), `validate_package_name` unit tests, a publish-pipeline test with a signed and an unsigned fixture bundle, and an auth-flow test. `sqlx::test` + a testcontainer makes this tractable.

10. **Import the wire protocol instead of mirroring it (§4.1, §11.5).** *Effort: S.* Have shape-mcp depend on the crate that defines `WireMessage`/framing (it already uses `shape-wire`) and delete the hand-copied types + framing constants.

### P3 — housekeeping

11. **Correct stale docs:** fix `run_shape_code`'s description (§9.9); either delete `llms.txt`/`llms-full.txt` or regenerate them from the book (§8.4); fix the registry README's frontend location (§8.1); reconcile the book's MCP doc's REST description with the stdio reality (§8.3). *Effort: XS each.*

12. **Delete committed junk:** `shape-app/one,n`, `shape-app/two,n` (§3.5). *Effort: XS.*

13. **Refactor `run_publish_transaction` to a `PublishContext` struct (§11.9)** and extract the `is_owner` authz helper (§4.5). *Effort: S.*

14. **Decide the fate of the legacy static SPAs (§11.8)** vs the shape-notebook Svelte app; retire one. *Effort: M.*

### Sequencing note

Recommendations 1, 2, and 3 are the ones that restore *functional truth* to the ecosystem — after them, the MCP teaches current Shape, the playground is safe, and the sample packages run. Everything else is hardening and hygiene. The unifying fix behind 1, 3, 8, and 11.1 is a single new CI stage: **"exercise the cross-boundary contracts against HEAD"** — rebuild+probe the MCP, compile the packages, round-trip a bundle. That one gate would have caught four of this report's findings before they shipped.

---

## Appendix A — Empirical transcripts (reproduction commands)

All runs used the prebuilt working-tree binary `…/shape/target/debug/shape` (extension-load warnings elided) and a from-source rebuild of `shape-mcp` (release).

**A.1 — c-string retired / int is i64 / Vec≡Array / trait syntax** (§8.4, §9.5)

```
$ printf 'let x = c"hello {name:bold}"\nprint(x)\n' | shape run  → error: Undefined variable: 'c'
$ printf 'let x: int = 9000000000000000\nprint(x)\n'  | shape run → 9000000000000000   (> 2^47 ⇒ i64)
$ printf 'let xs: Vec<int> = [1,2,3]\nprint(xs.len())\n'   | shape run → 3
$ printf 'let xs: Array<int> = [1,2,3]\nprint(xs.len())\n' | shape run → 3
$ printf 'trait Greet { greet(self): string }\n' | shape run → parse error E0001 (unexpected identifier `greet`)
$ printf 'impl Greet for P { fn greet(self)->string {"hi"} }' → error: method receivers are implicit. Use `method greet(...)`
```

**A.2 — default-permissive file read** (§9.2)

```
$ printf 'use std::core::file\nlet t=file::read_text("/etc/hostname")?\nprint(t)\n' | shape run  → atlas-dev
```

**A.3 — xgboost broken / duckdb native-preflight** (§9.3)

```
$ shape run packages/xgboost/index.shape  → Semantic error: Cannot assert type 'string' as 'number'  (exit 1)
$ shape run packages/duckdb/index.shape   → native dependency preflight failed … libduckdb.so … No such file
```

**A.4 — MCP prebuilt (Mar-15, flat book) vs rebuilt (current book)** (§9.1)

```
# prebuilt release binary (embedded flat book):
get_shape_api math      → "# stdlib: math …"        (works)
get_shape_syntax traits → "# Traits …"              (works)

# rebuilt from source against current book:
get_shape_api math      → "Unknown module: 'math'. Available modules: core/collections, core/math, …"
get_shape_api http/json → "Unknown module …"
get_shape_syntax traits → "Unknown construct: 'traits'. Available constructs: …, traits, …"  (self-contradiction)
get_shape_syntax async  → "Unknown construct: 'async' …"
get_shape_syntax modules→ "Unknown construct: 'modules' …"
get_shape_syntax functions → "**Summary:** Functions use `fn name(...)` …"  (works)
resources/read shape://stdlib/json    → ERROR Resource not found
resources/read shape://grammar/traits → ERROR Resource not found
```

## Appendix B — File inventory cited

Registry: `main.rs`, `config.rs`, `auth.rs`, `error.rs`, `state.rs`, `dto.rs`, `rate_limit.rs`, `routes/{mod,publish,download,package_info,docs,stats,yank,search,index,dependents,categories,health}.rs`, `services/{publish,blob,search,index_gen}.rs`, `migrations/00{1,2,3}_*.sql`, `Cargo.toml`, `build.rs`, `README.md`, `Dockerfile`, `deploy/*`, `frontend/src/lib/markdown/render.ts`.
shape-app: `shape-server/src/{main,auth,rate_limit}.rs`, `routes/{execute,stdlib_cache,notebook}.rs`, `Cargo.toml`, `shape-server/Cargo.toml`.
shape-mcp: `src/{main,tools,executor,resources,logging}.rs`, `content/loader.rs`, `build.rs`, `Cargo.toml`, `llms.txt`, `llms-full.txt`, `eval/results/round-2/summary.md`.
packages: `duckdb/{index.shape,shape.toml}`, `xgboost/{index.shape,shape.toml,shape.lock}`.
shape-infra: `nixos/shape-prod.nix`, `debian/pull-deploy/shape-deploy-app.sh`, `README.md`.
Core (referenced): `shape-runtime/src/crypto/signing.rs`, `shape-runtime/src/module_exports.rs`, `shape-vm/src/resource_limits.rs`, `bin/shape-cli/src/commands/serve_cmd.rs`, `bin/shape-cli/src/{registry_client,config/mod}.rs`; book `shape-web/book/book-site/src/content/docs/**`.

## Appendix C — Methodology & confidence levels

To let a reader weight each finding, here is how each was established:

| Finding | How established | Confidence |
|---------|-----------------|------------|
| §9.1 MCP retrieval broken | **Empirical** — from-source rebuild + JSON-RPC transcript, cross-checked against the book `.mdx` tree and the prebuilt binary | **Certain** |
| §9.2 shape-app unsandboxed | **Empirical** (file-read transcript) + code trace through `check_permission`/`ResourceLimits`/`stdlib_cache` | **Certain** on posture; **High** on DoS mechanism (timeout reasoning is standard tokio semantics) |
| §9.3 xgboost broken | **Empirical** — `shape run` transcript + minimal repro | **Certain** |
| §9.3b duckdb | **Empirical** — preflight-fail transcript; Shape compile not reached | **Confirmed-blocked, compile unverified** |
| §9.4 signature model | **Code read** — `signing.rs::verify`, publish.rs signature loop, migration `user_keys` | **Certain** |
| §9.5 native-blob traversal | **Code trace** — `routes/publish.rs` → `blob_filename` → `write_typed_blob`; path-arithmetic reasoned, not executed | **High** (not run against a live registry) |
| §9.6 sanitizer widening | **Code read** — `render.ts` schema; XSS-vs-CSS-injection distinction reasoned from allowlist | **High** for CSS-injection; **Medium** that JS-XSS is fully blocked (would need a live fixture) |
| §9.7 has_source/native_targets | **Code read** — hardcoded literals + frontend TS interface | **Certain** |
| §9.11/§9.12 notebook DoS/leak | **Code read** — no timeout wrapper; `SESSIONS` insert-only | **Certain** on structure; **High** on exploitability |
| §9.13 re-publish clobber | **Code read** — the `UPDATE` binds `name` to `description` | **Certain** on the binding; **High** on user-visible impact (depends on `PackageBundle::metadata` fields) |
| §5.6 version skew | **Code read** — Cargo.toml pins vs workspace version | **Certain** on the pin; **Medium** on whether the format actually broke |

What I did **not** do (and why): I did not stand up a live Postgres + registry (would have consumed the cargo/DB budget and is a multi-service bring-up), so all SQL-path and end-to-end-publish claims are code-level. I did not build shape-app's server (heavy; the engine-path posture is provable via the shared CLI default). I made exactly two cargo invocations, both against shape-mcp's independent target dir: the incremental rebuild that produced the §9.1 proof, and (implicitly) its dependency compile. Everything else used the prebuilt working-tree `shape` binary.

**Cross-cutting theme restated for the record:** every high-severity finding in this report is a *drift* between a satellite and the core — the MCP's ID maps vs the book, shape-app's engine vs the `serve` sandbox, xgboost's idioms vs the numeric rules, the registry's `0.1.6` parser vs the `0.3.2` producer, `llms.txt` vs the current syntax. None is a bug in isolation; each is a snapshot that the core moved out from under. The single highest-leverage investment is a CI stage that re-exercises the three cross-boundary contracts (§1.6) against HEAD on every change. That gate converts four of these silent breaks into loud build failures.

*End of report.*
