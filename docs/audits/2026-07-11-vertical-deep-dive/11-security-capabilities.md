# Vertical Deep-Dive Audit 11 — Security, Capabilities & Sandboxing

**Auditor:** 11 of 19
**Date:** 2026-07-11
**Scope:** `crates/shape-abi-v1` (Permission / PermissionSet / ScopeConstraints), `crates/shape-runtime/src/stdlib/capability_tags.rs`, `check_permission` call sites across stdlib, `crates/shape-vm/src/resource_limits.rs` + `crates/shape-value/src/v2/alloc_budget.rs`, `crates/shape-runtime/src/stdlib/deterministic.rs`, Ed25519 signing (`crypto/signing.rs` + `crypto/keychain.rs` + `module_manifest.rs`), permission-into-content-hash baking (`bytecode/content_addressed.rs`), linker permission union (`linker.rs`), the `shape run` / `shape serve` security wiring in `bin/shape-cli`.
**Method:** working-tree read of the DIRTY tree + empirical execution against the prebuilt debug binary `target/debug/shape`. Every runtime claim below carries a run transcript or a `file:line` cite.

---

## 0. Executive Summary

### 0.1 Overall health verdict

The security/capability subsystem is in **materially better shape than the stale 2026-07-04 "DEAD STUB" audit suggests** — but it is **not** the "16 fine-grained permissions, three-tier security" system the book and CLAUDE.md advertise. The honest picture is a **two-speed reality**:

- **What genuinely works end-to-end (empirically verified this audit):** the compile-time capability-derivation → content-hash → linker-union → load-time subset gate is real and fires; the runtime `check_permission` backstop in gated stdlib I/O is real and fires; **7 of 17** permissions (`FsRead`, `FsWrite`, `NetConnect`, `NetListen`, `Process`, `Env`, `Random`) have a live runtime enforcement site; `Ffi` and `Deterministic` have live load-time gates; filesystem path-scope and network host-scope narrowing work at runtime; all four resource limits (`--max-instructions`, `--max-time-ms`, `--max-output-bytes`, `--max-memory-bytes`) are enforced by the interpreter and fail closed; the `shape serve` sandbox model fails closed on strict + non-loopback binds; the Ed25519 signing primitive is correct and well-tested.
- **What is decorative, dead, or dangerously mis-wired:** **5 of 17** permissions (`Vfs`, `Capture`, `MemLimited`, `TimeLimited`, `OutputLimited`) have **zero enforcement references** — pure decoration; the `DeterministicRuntime` seeded-clock/PRNG module is **compiled but wired nowhere** — `sandbox.deterministic=true` does NOT make `time.millis()`/random reproducible (it only refuses foreign code); the `shape.toml [permissions]` surface is a **least-privilege-inverting footgun** — an empty `[permissions]` section grants near-full trust, unset fields default `true`, and (worst) the natural `fs.write = false` TOML **dotted-key silently binds to a different struct field and is ignored with no warning**, leaving the write permission OPEN; the Ed25519 keychain is **never installed by `shape run`/`shape serve`**, so module signature verification never runs in the normal execution path; `--max-memory-bytes` is a **per-single-TypedArray-buffer ceiling, not a cumulative heap cap** (and only `TypedArray` growth is instrumented — strings/objects/hashmaps are not).

The subsystem's core mechanism (derive → hash → union → subset + runtime backstop) is sound and non-bypassable in the paths that are wired. The risk concentrates in (a) the config surface's fail-open ergonomics, (b) five decorative permission variants that imply guarantees the runtime does not deliver, and (c) doc claims well ahead of wiring.

### 0.2 Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | **P0** | `shape.toml` `[permissions]` dotted-key `fs.write = false` (the natural form) is **silently ignored** — TOML parses it as `[permissions.fs]` table (which has no `write` field), and no `deny_unknown_fields` → `fs_write` stays `None` → defaults `true`. Write stays OPEN with no warning. | §9.1; transcript: `fs.write=false` → "WRITE OK - PERMISSION BYPASS"; only `"fs.write" = false` (quoted) denies. `permissions.rs:24`, no `deny_unknown_fields` grep-empty |
| 2 | **P1** | Empty/partial `[permissions]` section is **fail-open by field**: unset booleans default `true` (`unwrap_or(true)`), so declaring `[permissions]` with only the fields you want to *deny* still grants everything else. An empty `[permissions]` grants near-full trust (all but `Ffi`). | §9.2; transcript "EMPTY-SECTION WRITE OK"; `permissions.rs:150-176` |
| 3 | **P1** | 5 of 17 permissions are **decorative** — `Vfs`, `Capture`, `MemLimited`, `TimeLimited`, `OutputLimited` have zero enforcement references outside `abi-v1/src/lib.rs` metadata. Granting/withholding them changes nothing. | §2.4; grep for `Permission::{Vfs,Capture,MemLimited,TimeLimited,OutputLimited}` returns only enum/`name()`/`all_variants()` |
| 4 | **P1** | `DeterministicRuntime` (seeded ChaCha8 PRNG + virtual clock) is **compiled but referenced nowhere** in the execution path. `sandbox.deterministic=true` does NOT route `time.millis()`/random through it — `time.millis()` reads real `SystemTime` unconditionally. The module doc-comment claim is false. | §2.3, §9.3; `stdlib_time.rs:208` ignores `_ctx`; grep `DeterministicRuntime` outside `deterministic.rs` = empty |
| 5 | **P1** | Ed25519 module-signature verification is **never invoked by `shape run`/`shape serve`** — no keychain is installed on those paths (`self.keychain` stays `None`), so `verify_module` is dead in normal execution. Signing is a correct standalone primitive only. | §2.5, §9.4; `grep set_keychain` in `bin/` returns 0 hits on run/serve |
| 6 | **P1** | `--max-memory-bytes` is a **per-single-buffer ceiling, not a cumulative budget**, and only `TypedArray::grow` is instrumented (`check_size` has exactly one caller). String / TypedObject / HashMap allocation is uninstrumented; many separate live buffers each under the ceiling are not bounded. `ResourceUsage::record_allocation` (the cumulative tracker) is dead code. | §2.2, §9.5; `alloc_budget.rs:14-25` self-documents; `check_size` single caller `typed_array.rs:297`; `record_allocation` 0 real callers |
| 7 | **P2** | Runtime permission denial is a **fatal uncatchable VM error**, not a Shape-level `Result::Err`, contradicting the book (`security-permissions.mdx:449` shows `→ Err(...)`). A program cannot recover from a scoped-path denial. | §9.6; transcript: out-of-scope write aborts with "Runtime error: Scope constraint denied", `Err` arm not taken |
| 8 | **P2** | `Time` permission has **no runtime backstop** — only load-time derivation gates it (`time.millis` body ignores `_ctx`). Any path that reaches `time.millis` without the compiler deriving `Time` is ungated. `FsScoped`/`NetScoped` are also load-derivation-only labels; scope narrowing keys off `scope_constraints` presence, not the permission. | §2.4; `stdlib_time.rs:208`; `module_exports.rs:244-308` |
| 9 | **P2** | Scope-path matching is **prefix-only, not real glob** — `/tmp/*/x` mid-pattern globs silently never match; only trailing `*`/`**` are handled (trimmed to a prefix + `Path::starts_with`). Documented as "glob-style" in `module_exports.rs:255`. | §2.1, §9.7; `module_exports.rs:254-259` |
| 10 | **P2** | The book (`security-permissions.mdx:20`) states the **entire permission model is host-side-only and unavailable from Shape in v0.3.3** ("planned for v0.4"), yet the working `shape.toml [permissions]` mechanism exists and enforces. Doc is both stale and wrong about availability; the working surface is undocumented. | §8; `security-permissions.mdx:20` vs. working transcripts in §9 |

**Two further findings surfaced during deeper empirical testing (§9.14, §9.15), both meriting the top band:**

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 11 | **P1** | Network host-scope wildcard `*.example.com` **matches `evilexample.com`** — the boundary check is `ends_with(suffix) && len > suffix.len()` with `suffix = "example.com"` (the leading dot stripped), so any host merely *ending in* the domain string is allowed, not just true subdomains. A scope-narrowing bypass. | §9.14; `module_exports.rs:292-293` |
| 12 | **P1** | Any **unknown/typo'd key** under `[permissions]` (e.g. `totally_bogus_key = false`) is silently accepted (no `deny_unknown_fields`) and leaves all real permissions at their default-`true`, and the documented `permissions = "readonly"` shorthand is **unreachable at top level** (the field is `Option<PermissionsSection>` table-only, not `Option<PermissionPreset>`) — both fail open. | §9.15; `project_config.rs:64`, transcript "WRITE OK" |

### 0.2b Methodology note

Findings are graded on evidence type: **empirical** (a run transcript against `target/debug/shape`, reproduced this audit — Findings #1, #2, #3-via-grep, #4, #6, #12, and all §9 supporting transcripts), **code-reasoning** (a pure function whose behavior I traced but did not drive live — Finding #11's wildcard bypass, verified by evaluating the boolean expression on concrete inputs), and **absence-of-wiring** (a grep proving a symbol has no consumer — Findings #5 keychain, #4 DeterministicRuntime, #6 record_allocation). Every P0/P1 carries at least one of these, cited inline. Where I could not stand up infrastructure within the cargo/time budget (a two-node cluster; a live child-process spawn), I say so explicitly and fall back to code-cite (§2.7, §9.16) rather than assert.

### 0.3 Feature-completeness score: **58 / 100**

Justification: the core capability pipeline (derive → hash → union → load gate → runtime backstop) plus resource limits and the serve sandbox are real and verified working (this is the bulk of the value and pulls the score up). But nearly a third of the advertised permission surface is decorative, the determinism runtime is unwired, signature verification is unreachable in the run path, and the primary user-facing config surface (`shape.toml [permissions]`) fails open through two independent footguns. "Code exists" is high; "works end-to-end as advertised" is moderate.

### 0.4 Code-quality score: **72 / 100**

Justification: the `abi-v1` permission types, `PermissionSet` algebra, `alloc_budget`, `signing`, and `keychain` modules are clean, idiomatic, well-commented Rust with honest doc-comments and good unit tests. `capability_tags.rs` is a readable static table with excellent test coverage. The deductions are for: dead code carried as if live (`record_allocation`, `DeterministicRuntime`), the split-brain between the `capability_tags` env function list and the actual `env` module exports, the config-parse footguns (missing `deny_unknown_fields`), and doc-comments that assert behavior the code does not implement (`deterministic.rs`).

### 0.5 Biggest-risk paragraph

The single biggest risk is **the `shape.toml [permissions]` config surface fails open in the exact ways a security-conscious user would trip over**. A developer who writes the obvious `fs.write = false` gets **no enforcement and no warning** — the write permission stays granted because the dotted key silently lands on the wrong serde field (Finding #1). A developer who writes a minimal `[permissions]` block to "lock things down" gets **near-full trust** because every field they did not explicitly set to `false` defaults to `true` (Finding #2). These are not exotic edge cases; they are the first two things a user will type. Combined with the fact that five permission variants are decorative (Finding #3), the determinism runtime is a no-op (Finding #4), and signatures are never checked (Finding #5), the subsystem projects far more security assurance than it delivers. Nothing here is *unsound in the wired paths* — the load gate and runtime backstop cannot be bypassed by Shape code once a correct permission set is installed — but the path from "user intent" to "correct permission set installed" is riddled with silent fail-open transitions. The security story is a well-engineered engine bolted to a footgun-shaped steering wheel.

---

## 1. Architecture & Code-Structure Map

### 1.1 Module inventory (LOC measured with `wc -l`, working tree)

| Module | Path | LOC | Responsibility |
|--------|------|----:|----------------|
| ABI permission types | `crates/shape-abi-v1/src/lib.rs` | 2531 | `Permission` (17 variants), `PermissionCategory`, `PermissionSet` (BTreeSet-backed, set algebra), `ScopeConstraints`, `PermissionGrant`; the stable C-ABI surface. Permission section is `lib.rs:1021-1520`. |
| Capability tag table | `crates/shape-runtime/src/stdlib/capability_tags.rs` | 346 | Static `module::function → PermissionSet` map consumed at compile time to derive `required_permissions`. |
| Permission gate + scope | `crates/shape-runtime/src/module_exports.rs` | 752 | `check_permission`, `check_fs_permission`, `check_net_permission`, `ModuleContext` (carries `granted_permissions`, `scope_constraints`). The runtime enforcement primitives. |
| Resource limits (struct) | `crates/shape-vm/src/resource_limits.rs` | 184 | `ResourceLimits`, `ResourceUsage`, `ResourceLimitExceeded`; instruction/wall-time/output tick logic. |
| Alloc budget (memory) | `crates/shape-value/src/v2/alloc_budget.rs` | 220 | Thread-local per-buffer byte ceiling; `check_size`, `record_breach`, `take_breach`, `BudgetGuard`. The actual memory enforcement. |
| Deterministic runtime | `crates/shape-runtime/src/stdlib/deterministic.rs` | 190 | Seeded ChaCha8 PRNG + virtual clock. **Unwired** (see §2.3). |
| Virtual FS | `crates/shape-runtime/src/stdlib/virtual_fs.rs` | 546 | In-memory `FileSystemProvider` with per-file size limits. Not gated by the `Vfs` permission. |
| Ed25519 signing | `crates/shape-runtime/src/crypto/signing.rs` | 156 | `ModuleSignatureData::{sign,verify}`, `sign_manifest_hash`, `generate_keypair`. Correct primitive. |
| Keychain / trust | `crates/shape-runtime/src/crypto/keychain.rs` | 289 | `Keychain`, `TrustLevel` (Full/Scoped/Pinned), `TrustedAuthor`, `verify_module`, `require_signatures`. |
| Module manifest | `crates/shape-runtime/src/module_manifest.rs` | 290 | `ModuleManifest`, `verify_integrity`, `verify_signature`. |
| Permission config (toml) | `crates/shape-runtime/src/project/permissions.rs` | 275 | `PermissionsSection`, `to_permission_set`, `to_scope_constraints`, shorthands (pure/readonly/full). The `shape.toml [permissions]` surface. |
| Content-addressed blob | `crates/shape-vm/src/bytecode/content_addressed.rs` | 653 | `FunctionBlob.required_permissions`, `compute_hash` (folds sorted permission names), `Program`, `LinkedProgram.total_required_permissions`. |
| Linker | `crates/shape-vm/src/linker.rs` | 807 | Transitive permission union across blobs (`linker.rs:406-408`). |
| Serve sandbox wiring | `bin/shape-cli/src/commands/serve_cmd.rs` | 2970 | `SandboxLevel` (strict/permissive/none) → `PermissionSet` + `ScopeConstraints` + `ResourceLimits`; loopback fail-closed. |
| Run security wiring | `bin/shape-cli/src/commands/script_cmd.rs` | (partial) | `RunSecurity`, `derive_run_security`, `apply_run_security`, `downgrade_mode_for_limits`. |

Territory total (the 14 rows above): **~10,209 LOC**, dominated by the 2531-line `abi-v1` (most of which is non-security ABI surface) and the 2970-line `serve_cmd`.

### 1.2 Data flow — the capability pipeline

The end-to-end flow for a permission decision has two independently-firing tiers plus a config front-end:

```
                 shape.toml [permissions] / frontmatter / --sandbox
                                     │
                 derive_run_security │ (script_cmd.rs:36)
                                     ▼
                effective_permission_set()  ──►  granted: Option<PermissionSet>
                  (project_config.rs:195)        scope:   Option<ScopeConstraints>
                                     │
     ┌───────────────────────────────┴───────────────────────────────┐
     │ COMPILE TIME (per stdlib call site)                            │
     │   capability_tags::required_permissions(module, fn)           │
     │        → FunctionBlob.required_permissions                    │
     │        → compute_hash() folds SORTED permission NAMES         │  (content_addressed.rs:157)
     │        → linker union: total_required_permissions            │  (linker.rs:406)
     └───────────────────────────────┬───────────────────────────────┘
                                     ▼
     LOAD TIME  load_program_with_permissions(ca, &granted)          (program.rs:383)
        deterministic_foreign_gate(required, granted)                (program.rs:10)
        if !total_required.is_subset(granted) → PermissionError      (program.rs:391)
                                     │  (skipped entirely when granted == None)
                                     ▼
     RUN TIME   gated stdlib fn calls check_permission(ctx, P)       (module_exports.rs:222)
        check_fs_permission / check_net_permission add scope check   (module_exports.rs:244/278)
                                     │  (fail-OPEN when granted == None)
                                     ▼
                            allow / Err(String) → VMError
```

Two facts govern everything downstream:

1. **`granted == None` ⇒ allow-all.** Both the load-time subset gate (`execution.rs:270`, only runs `if granted.is_some()`) and the runtime `check_permission` (`module_exports.rs:226`, `if let Some(ref granted)`) short-circuit to "allow" when no permission set is installed. For a local `shape run` with no `[permissions]` section, `granted` is `None` (`script_cmd.rs:49-57`). This is intentional ("trusted local"), but it means the *default* posture of the binary is zero enforcement.
2. **The two tiers are redundant by design but not identical in coverage.** The load-time gate is derived from static call-site analysis; the runtime gate fires at the stdlib FFI boundary. Seven permissions have both; `Time`/`Ffi`/`FsScoped`/`NetScoped` have only one; five have neither (§2.4).

### 1.3 Key types

- `Permission` (`abi-v1/src/lib.rs:1063`): `Copy` C-style enum, 17 variants, `Ffi` pinned last (ordinal-stable for content hashing, `lib.rs:1106-1117`). Carries `name()` (dotted machine name, e.g. `"fs.write"`), `description()`, `category()`, `all_variants()`.
- `PermissionSet` (`lib.rs:1222`): `BTreeSet<Permission>` wrapper — deterministic iteration (matters for hash stability). Full set-algebra API: `union`, `intersection`, `difference`, `is_subset`, `is_superset`. Constructors `pure()`, `readonly()`, `full()`.
- `ScopeConstraints` (`lib.rs:1396`): `allowed_paths`, `allowed_hosts`, `max_memory_bytes`, `max_time_ms`, `max_output_bytes`, plus `ffi_languages`/`ffi_libraries`/`ffi_symbols`. Note the resource caps here are **separate** from `ResourceLimits` and appear unused by the run path.
- `ResourceLimits` / `ResourceUsage` (`resource_limits.rs:11/52`): four optional caps; usage tracker with amortized wall-time checks (every 1024 instructions, `resource_limits.rs:102`).
- `ModuleContext` (`module_exports.rs`): threads `granted_permissions` + `scope_constraints` into every gated stdlib call.
- `Keychain` / `TrustLevel` (`keychain.rs:30/85`): trust policy — `Full`, `Scoped(prefixes)`, `Pinned(hash)`.

### 1.4 Entry points

- **CLI:** `shape run` (`script_cmd.rs::run_script`), `shape serve` (`serve_cmd.rs`), `shape keys {generate,trust,list,sign,verify}` (`keys_cmd.rs`).
- **Run-path security install:** `derive_run_security` (`script_cmd.rs:36`) → `apply_run_security` (`script_cmd.rs:110`) → `executor.set_granted_permissions(...)` + `executor.set_resource_limits(...)`.
- **VM install:** `VirtualMachine::set_permissions` (`vm_impl/init.rs:126`), `with_resource_limits` (`vm_impl/init.rs:114`).
- **Load gate:** `load_program_with_permissions` (`vm_impl/program.rs:383`).
- **Runtime gate:** `check_permission` and friends (`module_exports.rs:222/244/278`), called from `stdlib_io/*`, `stdlib/env.rs`, `stdlib/crypto.rs`, `executor/builtins/{transport,remote}_builtins.rs`.

### 1.5 `ModuleContext` — the runtime enforcement carrier

Every gated stdlib function receives a `ModuleContext` (`module_exports.rs`) that carries `granted_permissions: Option<PermissionSet>` (`:193`) and `scope_constraints: Option<ScopeConstraints>` (`:197`) alongside the type-schema registry and callable invoker. The context is assembled per-dispatch in `vm_impl/modules.rs:845-909`: the VM clones its `granted_permissions` (`modules.rs:849`) into the context *before* parking `&mut self`, so the gated function sees the run's envelope. This is the single thread through which both the permission set and the scope constraints reach the enforcement primitives. Two consequences:

- **The `Option` is the fail-open switch.** `check_permission` (`:226`) and `check_fs_permission`/`check_net_permission` only enforce inside `if let Some(ref granted)`/`if let Some(ref constraints)`. A `None` on either field is silent allow. The VM's default (`init.rs:76`, `granted_permissions: None`) is therefore allow-all until `set_permissions` installs a set.
- **Scope enforcement is orthogonal to the permission variant.** `check_fs_permission` first calls `check_permission(ctx, FsRead|FsWrite)` then, *independently*, checks `scope_constraints.allowed_paths` if present (`:251-267`). The `FsScoped` permission variant is NOT consulted in this path — the narrowing keys entirely off `scope_constraints` being `Some` with a non-empty list. This is why `FsScoped`/`NetScoped` are "labels" in §2.4: the scope machinery works, but it does not depend on the scoped-permission being granted.

### 1.6 The distributed / remote enforcement path

The most security-sensitive entry point is `remote.rs::handle_remote_call` (~`:1000-1044`), which executes code transferred from another node. Its design is "receiver-owned enforcement, never trust the sender":

1. Recompute every received blob's content hash from bytes and reject mismatches (`remote.rs:~1031`).
2. Recompute the linker permission union from the *verified* blobs (not the sender's self-declared claim).
3. Install the *receiver's* granted set + scope via `set_permissions(Some(...), ...)` — `None` is explicitly forbidden here (`remote.rs:1029`: "`None` is forbidden ... because `check_permission` is fail-OPEN when `None`").
4. Enforce `--ffi-languages` as a strict opt-in allow-list for dynamic foreign languages (`remote.rs:1038-1044`).

This path is the correct model and the one place where the fail-open-when-`None` default is explicitly closed by construction. It is a good template for what the *local* run path's config surface should aspire to (fail-closed by default).

### 1.7 `derive_run_security` — the local-run precedence logic

`derive_run_security` (`script_cmd.rs:36-107`) assembles the local `RunSecurity` from four sources with a specific precedence. Understanding it is essential to the fail-open findings:

1. **Permissions.** `granted = Some(effective_permission_set())` ONLY when `cfg.permissions.is_some()` (`script_cmd.rs:49-51`); otherwise `granted = None` (allow-all). So the *presence* of a `[permissions]` table is the switch between "trusted" and "gated." This is why an empty `[permissions]` (Finding #2) is dangerous: it flips the switch to "gated" but with a default-open set — arguably worse than `None` because the user believes they gated it.
2. **Scope.** `scope = Some(section.to_scope_constraints())` in the same branch (`script_cmd.rs:54`).
3. **Deterministic special-case.** If `[sandbox] deterministic = true`, `Permission::Deterministic` is *added* to the granted set, and if there was no `[permissions]` section the set is bootstrapped from `PermissionSet::full()` (`script_cmd.rs:73-79`). So `deterministic = true` alone (no `[permissions]`) yields `granted = Some(full + Deterministic)` — full trust plus the foreign-refusal marker. This is additive and documented, but note it silently *creates* a granted set where none existed, changing `None` (allow-all, no load gate) into `Some(full)` (allow-all, WITH load gate + foreign refusal). Subtle but correct.
4. **Resource limits.** CLI `--max-*` flags take precedence, falling back to `[sandbox]` caps (`script_cmd.rs:80-91`); the whole `ResourceLimits` collapses to `None` if every field is unset (`script_cmd.rs:92-100`).

The precedence is coherent and the deterministic special-case is thoughtfully additive. The problem is entirely upstream: `effective_permission_set()` → `to_permission_set()` is where the default-open + unknown-key-ignore footguns live (§9.1, §9.2, §9.15). The wiring around it is sound; the set it produces is the hazard.

### 1.8 Content-addressed permission fields — storage detail

`FunctionBlob.required_permissions: PermissionSet` (`content_addressed.rs:88-91`) uses `#[serde(default = "default_permission_set")]` so older blobs without the field deserialize to `pure()` (empty) — a safe default (a blob claiming no permissions cannot escalate; the linker union just won't add anything). `LinkedProgram.total_required_permissions` (`content_addressed.rs:576`) carries the same serde default. The serialization is deterministic (BTreeSet iteration + sorted names in the hash input), which is what makes the content hash reproducible across nodes — a prerequisite for the distributed model. The `default_permission_set` fallback is the right fail-safe direction (empty, not full).

---

## 2. Feature Completeness

Legend: **WORKS** = code exists AND verified working end-to-end this audit; **PARTIAL** = works in some paths / with caveats; **STUB** = code exists but not wired; **DECORATIVE** = symbol exists, no behavior.

### 2.1 The capability pipeline (compile → hash → union → load gate → runtime gate) — **WORKS**

Every stage of the pipeline is present and I verified each stage produces observable behavior:

- **Static derivation** — `capability_tags::required_permissions("std::core::file","write_text")` returns `{FsWrite}` (`capability_tags.rs:74-75`), unit-tested at `capability_tags.rs:131-136`.
- **Baked into content hash** — `FunctionBlob::compute_hash` folds the sorted permission names into the SHA-256 input (`content_addressed.rs:158-159,178`). Proven load-bearing by the test `ffi_reservation_leaves_existing_content_hash_unchanged` (`linker_tests.rs:585-606`): adding `Ffi` to an otherwise-identical blob changes the hash (`assert_ne!` at `linker_tests.rs:605`). This directly satisfies the audit's ask — "two functions differing only in permissions hash differently."
- **Linker union** — `linker.rs:406-408`: `blobs.iter().fold(PermissionSet::pure(), |acc, blob| acc.union(&blob.required_permissions))` → `total_required_permissions`.
- **Load-time subset gate** — `load_program_with_permissions` (`vm_impl/program.rs:383-408`): `deterministic_foreign_gate` then `if !total_required.is_subset(granted) → PermissionError::PermissionsNotGranted`.
- **Runtime backstop** — `check_permission` (`module_exports.rs:222-236`) at the stdlib FFI boundary.

**Empirical proof (load gate):** with `shape.toml [permissions] "fs.write" = false`, a `file::write_text` program is refused before executing:

```
$ shape run proj/src/main.shape   # [permissions] "fs.write" = false
Error: Semantic error: Permission denied at load: program requires permissions not granted: fs.write
```
(full transcript §9.1). Env and net gate identically (§9.8, §9.9).

**Path-scope narrowing — WORKS but prefix-only (Finding #9).** `check_fs_permission` (`module_exports.rs:244-270`) checks base perm then, if `scope_constraints.allowed_paths` non-empty, requires `target.starts_with(prefix)` where `prefix` is the pattern with trailing `*`/`**`/`/` trimmed (`module_exports.rs:254-259`). Uses `Path::starts_with`, which is component-aware (so `/tmp/shape_allowed` does NOT match `/tmp/shape_allowed_evil` — good). But mid-pattern globs (`/tmp/*/x`) are not handled — the `*` is left literal and `Path::starts_with(Path::new("/tmp/*/x"))` never matches a real path. Verified working for the common trailing-glob case (§9.7).

### 2.2 Resource limits — **WORKS (with a memory-model caveat, Finding #6)**

All four caps verified firing under the interpreter:

| Cap | Flag | Mechanism | Verified |
|-----|------|-----------|----------|
| Instructions | `--max-instructions` | `ResourceUsage::tick_instruction` per dispatch cycle (`resource_limits.rs:115`; dispatch calls at `dispatch.rs:177,383,539,599`) | §9.10: "Instruction limit exceeded: 1000000 >= 1000000" |
| Wall time | `--max-time-ms` | amortized check every 1024 instructions (`resource_limits.rs:102,129`) | §9.10: "Wall time limit exceeded: 200.03ms >= 200ms" |
| Output | `--max-output-bytes` | `ResourceUsage::record_output` charged at `builtin_print` (`vm_impl/builtins.rs:1707`) | §9.11: truncated at 20 lines, "Output limit exceeded: 1020 >= 1000" |
| Memory | `--max-memory-bytes` | thread-local per-buffer ceiling `alloc_budget::check_size` (`alloc_budget.rs:84`) consulted in `TypedArray::grow` (`typed_array.rs:297`) | §9.5: "a single heap buffer reached 16777216 bytes, over the 10000000-byte ceiling" |

**Caveat (Finding #6):** the memory cap is honestly documented in `alloc_budget.rs:14-25` as "a ceiling on any single buffer, not a cumulative budget." `check_size` has exactly ONE caller — `TypedArray::grow` (`typed_array.rs:297`). Strings, TypedObjects, HashMaps, closures allocate through paths that never call `check_size`. And `ResourceUsage::record_allocation` (`resource_limits.rs:138`), the cumulative tracker in the *other* limits module, has zero real callers (only a comment mention at `execution.rs:703`). So `--max-memory-bytes N` means "no single `TypedArray` buffer may exceed N bytes," not "total live heap ≤ N." A program retaining many separate sub-ceiling buffers (or many strings/objects) is not bounded. In practice array-shaped retention trips the container array's own buffer first (§9.5 shows the outer accumulator tripping), which partially mitigates exploitability, but the guarantee is weaker than "256 MB memory limit" implies.

**JIT bypass — correctly handled.** `downgrade_mode_for_limits` (`script_cmd.rs:125`) forces the interpreter whenever any limit is set, emitting a one-line note, "because JIT-native execution has no per-instruction budget." Verified: every limited run in §9.10/§9.11 printed `[shape] resource limits set — running under the bytecode interpreter`. This is the right call — it fails closed rather than silently bypassing caps.

### 2.3 Deterministic runtime — **STUB (Finding #4)**

`DeterministicRuntime` (`deterministic.rs:21`) is a correct, tested seeded-ChaCha8 PRNG + virtual clock. Its module doc-comment claims (`deterministic.rs:6-7`): *"When `sandbox.deterministic = true`, the VM routes `time.millis()` and random functions through this module instead of real system sources."*

**This is false.** `grep DeterministicRuntime` across `crates/` + `bin/` + `tools/` returns hits ONLY inside `deterministic.rs` itself (and its tests). The type is never instantiated by the executor, never consulted by `time.millis`, never consulted by any random path. `time.millis`'s body (`stdlib_time.rs:208-215`) reads `std::time::SystemTime::now()` unconditionally and ignores its `_ctx` argument entirely. Verified empirically (§9.3): under a determinism-flagged run, `time.millis()` returns a live wall-clock epoch value.

What `sandbox.deterministic = true` *actually* does (`script_cmd.rs:73-79`): inserts `Permission::Deterministic` into the granted set, whose sole effect is `deterministic_foreign_gate` (`vm_impl/program.rs:10-20`) refusing any program that derives `Ffi`. So the flag is a foreign-code kill-switch mislabeled as a reproducibility runtime. The seeded-clock/PRNG half is entirely unbuilt-into-the-loop.

### 2.4 The 16 (+1) permission enforcement map — **7 WORKS, 2 load-only, 3 indirect, 5 DECORATIVE**

Cross-referencing every `Permission::X` reference against actual enforcement sites (`grep` of `check_permission`/`check_*_permission`/`deterministic_foreign_gate`/`is_subset` call sites):

| Permission | Load gate (derived) | Runtime `check_permission` | Verdict |
|-----------|:---:|:---:|---------|
| `FsRead` | yes (capability_tags) | yes (`file_ops.rs:150,186,231,281`; `file.rs:45`; `csv`) | **WORKS** |
| `FsWrite` | yes | yes (`file_ops.rs:260,295`; `file.rs:70`) | **WORKS** |
| `NetConnect` | yes | yes (`network_ops.rs:135,168,197,244,288`; `http.rs`; `remote_builtins.rs:1757+`; `transport_builtins.rs`) | **WORKS** |
| `NetListen` | yes | yes (`network_ops.rs:87`) | **WORKS** |
| `Process` | yes | yes (`process_ops.rs` ×13) | **WORKS** |
| `Env` | yes | yes (`env.rs:40,54,69,83`) | **WORKS** |
| `Random` | (not in capability_tags) | yes (`crypto.rs:195,215`) | **WORKS** (runtime-only) |
| `Ffi` | yes (`functions_foreign.rs:310`) + `deterministic_foreign_gate` | phase-1 opt-in check (`control_flow/mod.rs:1116`) | **WORKS** (load + dynamic-dispatch) |
| `Deterministic` | yes (foreign gate `program.rs:14`) | — | **WORKS** (foreign-refusal only) |
| `Time` | yes (`capability_tags.rs:106`) | **NO** (`stdlib_time.rs:208` ignores `_ctx`) | **PARTIAL** — load-only, no runtime backstop |
| `FsScoped` | added when `fs.allowed` present (`permissions.rs:188`) | indirect — scope check keys off `scope_constraints`, not this variant | **PARTIAL** — label; narrowing works via scope |
| `NetScoped` | added when `net.allowed_hosts` present (`permissions.rs:195`) | indirect (same) | **PARTIAL** — label |
| `Vfs` | — | — | **DECORATIVE** |
| `Capture` | — | — | **DECORATIVE** |
| `MemLimited` | — | — | **DECORATIVE** (memory capped by `ResourceLimits`, not this) |
| `TimeLimited` | — | — | **DECORATIVE** (time capped by `ResourceLimits`, not this) |
| `OutputLimited` | — | — | **DECORATIVE** (output capped by `ResourceLimits`, not this) |

The five DECORATIVE variants (Finding #3) are referenced NOWHERE outside `abi-v1/src/lib.rs`'s `name()`/`description()`/`category()`/`all_variants()` metadata — confirmed by `grep 'Permission::{Vfs,Capture,MemLimited,TimeLimited,OutputLimited}'` returning empty across the runtime/vm/cli crates. The three sandbox resource-limit variants are especially misleading: the actual mem/time/output enforcement is `ResourceLimits` (§2.2), which is completely disjoint from these permission variants — a caller granting `MemLimited` but not setting `--max-memory-bytes` gets no cap, and vice versa.

### 2.5 Ed25519 signing & trust — **PARTIAL (primitive WORKS, wiring STUB — Finding #5)**

- **Signing primitive — WORKS.** `ModuleSignatureData::{sign,verify}` (`signing.rs:29-55`) is a clean wrapper over `ed25519_dalek` with 6 unit tests covering sign/verify, wrong-hash, corrupt-sig, wrong-key, timestamp, serde round-trip (`signing.rs:99-155`). All correct.
- **Keychain trust policy — WORKS (logic).** `Keychain::verify_module` (`keychain.rs:100-127`) correctly checks: no-sig → `Unsigned`/`Rejected` per `require_signatures`; invalid sig → `Rejected`; untrusted key → `Rejected`; else `Trusted`. `TrustLevel::{Full,Scoped,Pinned}` implemented (`keychain.rs:85-91`). 10 unit tests.
- **Manifest verification wiring — PARTIAL.** `module_loader/mod.rs:870-906` verifies integrity (always) then signature *"when configured"* — `if let Some(keychain) = &self.keychain`.
- **Keychain install — STUB in the run path.** `grep set_keychain` in `bin/` shows it is called nowhere for `shape run` or `shape serve` (§9.4). `ShapeEngine::new` sets `keychain: None` (`lib.rs:246`). So in normal execution `self.keychain` is always `None` → signature verification is skipped entirely; unsigned or wrongly-signed modules load without objection. The `shape keys trust` CLI writes a keychain file (`keys_cmd.rs`), but no run path reads it back into the engine. The signing flow is thus a functional standalone tool (`shape keys sign`/`verify`) whose output is never checked during execution.

### 2.6 Serve sandbox model — **WORKS (fail-closed)**

`serve_cmd.rs:110-180` maps `--sandbox {strict,permissive,none}` + bind class → envelope:

- `strict` (default): `PermissionSet::pure()` (grants nothing) + `ResourceLimits::sandboxed()` (`serve_cmd.rs:115`).
- `permissive`: FsRead+Env+Time+Random+NetConnect + sandboxed caps (`serve_cmd.rs:116-123`).
- `none`/`off`: `full()` minus `Deterministic`, unlimited (`serve_cmd.rs:138-142`).
- **Non-loopback bind fails closed:** `granted.intersection(&PermissionSet::pure())` unless loopback (`serve_cmd.rs:146-150`) — a public bind gets nothing regardless of `--sandbox`.
- **Foreign-language opt-in:** `--ffi-languages` defaults empty; a transferred `fn python` is refused unless the operator opts the language in (`serve_cmd.rs:152-179`), and even then only on loopback.
- **Receiver-owned enforcement:** `remote.rs:1024-1044` recomputes the permission union from *verified* received blobs and gates against the receiver's set — "never trust the sender," and `None` is explicitly forbidden here (`remote.rs:1029`).

This is the strongest part of the subsystem: the network-facing path is fail-closed by construction.

### 2.7 Distributed receiver-owned enforcement — **WORKS**

The `remote.rs` path (§1.6) is covered by serve-level integration tests: `test_remote_permission_refusal_over_wire` (`serve_cmd.rs:2508`) and `test_remote_mutable_capture_refused_over_tcp` (`serve_cmd.rs:2158`) exercise a real TCP round-trip and assert refusal. Combined with the code review (hash-recompute → union-recompute → receiver-set install → `None`-forbidden), the distributed permission story is the most complete tier — the sender cannot escalate beyond the receiver's grant, and cannot smuggle a foreign-language call past the `--ffi-languages` allow-list. I did not stand up a two-node cluster this audit (time budget), but the code path and its tests are consistent and reviewed.

### 2.9 `shape run` has NO permission CLI flag — **GAP**

The only way to install a permission envelope for a local `shape run` is a `shape.toml [permissions]` section or script frontmatter. `shape run --help` exposes `--max-instructions/-memory-bytes/-time-ms/-output-bytes` (resource limits) but **no `--sandbox`, `--permissions`, `--allow`, `--deny`, or `--untrusted` flag** (verified: the grep returns empty). `shape serve` has `--sandbox {strict,permissive,none}`, but the local runner does not. Consequences:

- You cannot run an untrusted third-party `.shape` file sandboxed from the CLI — you must first wrap it in a project with a hand-written `[permissions]` section, navigating the two fail-open footguns (§9.1, §9.15) to do so.
- Resource limits ARE reachable via CLI flags (and fail closed), but capability limits are not. So a CLI user can bound *how much* a script consumes but not *what* it may touch.

This is a meaningful ergonomics/security gap: the safest local posture (deny-by-default) is unreachable without authoring config, and the config surface itself fails open. A `shape run --sandbox strict script.shape` mirroring the serve semantics would close this.

### 2.8 Runtime-only backstop (Random / Process) — **WORKS**

Two permissions are enforced *only* at runtime (not derived into the load gate): `Random` (`crypto.rs:195,215`) and — for the `io::spawn`/`io::exec` builtins — `Process` (`process_ops.rs`). Verified empirically that the runtime `check_permission` fires as a fatal error independent of the load gate:

```
$ shape run pure/src/rand.shape          # [permissions] random = false
start
Error: Runtime error: Permission denied: Access random number generation (sys.random) (line 3)
```
This proves the runtime backstop is a genuinely separate enforcement point from the load gate (the program compiled and *started* — printed "start" — then was denied at the `crypto::random_bytes` call site). See §9.13.

---

## 3. Code Quality

### 3.1 Idiom & naming

The territory is, on the whole, idiomatic modern Rust. Highlights:

- `PermissionSet` (`abi-v1/src/lib.rs:1222-1346`) is a textbook newtype over `BTreeSet` with a complete, orthogonal set-algebra API (`union`/`intersection`/`difference`/`is_subset`/`is_superset`) and `FromIterator`/`IntoIterator`/`From<[Permission; N]>` impls. The `BTreeSet` choice is deliberate and correct — deterministic iteration is load-bearing for content-hash stability (documented at `lib.rs:1218`).
- `Permission::name()` returns stable dotted machine names decoupled from the Rust variant names (`lib.rs:1122-1144`); this is the right call for a wire/hash-stable identifier.
- `alloc_budget.rs` is exemplary: the module doc-comment (lines 1-25) states the model, its limits, and *why* it is a per-buffer ceiling rather than a cumulative budget — honest about what it does NOT guarantee. The `BudgetGuard` RAII pattern with prior-ceiling restore (`alloc_budget.rs:121-136`) is clean.
- `check_permission`/`check_fs_permission`/`check_net_permission` (`module_exports.rs:222-308`) are small, single-responsibility, and read top-to-bottom.

Naming nits: `ResourceLimits` sub-fields on `ScopeConstraints` (`max_memory_bytes`/`max_time_ms`/`max_output_bytes`, `lib.rs:1411-1430`) shadow the `ResourceLimits` struct's fields with no wiring between them — two structs that look like they cooperate but do not (see §5.4).

### 3.2 Error handling

- The runtime gate returns `Result<(), String>` (`module_exports.rs:222`). Stringly-typed but adequate at the FFI boundary; the string is surfaced verbatim to the user ("Permission denied: Write, create... (fs.write)").
- The load gate uses a structured `PermissionError` enum (`executor/mod.rs:74-104`) with `PermissionsNotGranted { granted, required, missing }` and `DeterministicForeignRefused`. Good — structured where it matters (the security boundary), strings where it is user-facing text.
- **Panic-audit:** the security-relevant modules (`signing.rs`, `keychain.rs`, `permissions.rs` non-test, `resource_limits.rs`, `alloc_budget.rs`, `module_exports.rs`) contain **no non-test `unwrap`/`expect`/`panic!`**. The only `expect` on a hot path is `content_addressed.rs:185` / `module_manifest.rs` (`rmp_serde::encode::to_vec(...).expect("...serialization should not fail")`) — these are genuinely infallible (serializing owned primitive structs) and acceptable. `signing.rs:35` uses `unwrap_or_default()` on the `SystemTime` duration — correct defensive choice (a pre-epoch clock yields `signed_at = 0` rather than panicking).
- One notable **non-panicking correctness choice**: `TypedArray::grow` records a memory breach via `record_breach` and refuses to grow rather than `panic!`-ing (`typed_array.rs:297-298`), so a memory-limit breach on a serve node surfaces as a clean `VMError` and the worker survives (`resource_limit_enforcement.rs:11-16`). This was a real fixed defect (process-abort exit-101 → clean error).

### 3.3 `unsafe` usage

- **Permission logic: zero `unsafe`.** The `Permission`/`PermissionSet`/`ScopeConstraints` region (`abi-v1/src/lib.rs:1000-1520`) contains no `unsafe` blocks. `capability_tags.rs`, `resource_limits.rs`, `alloc_budget.rs`, `signing.rs`, `keychain.rs`, `permissions.rs` are entirely safe code.
- `module_exports.rs` has 3 `unsafe` occurrences — all in the C-ABI trait-object marshalling (`RawCallableInvoker`), not in the permission-check functions. Justified by the stable-ABI requirement (ADR-006 §2.7.5) and out-of-scope for permission soundness.
- `abi-v1/src/lib.rs` has ~91 `unsafe` occurrences total — these are the C-ABI vtable/FFI surface (`LanguageRuntimeVTable`, extension entry points), NOT the permission enum. The security-critical types are safe.

Verdict: **no unjustified `unsafe` in the security path.**

### 3.3b Resource-limit enforcement precision

Reading `resource_limits.rs` and `alloc_budget.rs` for the *precision* of each cap (how tightly the observed breach tracks the configured limit):

- **Instructions (`--max-instructions`)** — exact. `tick_instruction` (`resource_limits.rs:115-125`) increments then checks `>= limit` every single dispatch cycle. The breach fires on the instruction that reaches the limit (verified: "1000000 >= 1000000", §9.10). No slack.
- **Wall time (`--max-time-ms`)** — amortized, bounded slack. Checked every 1024 instructions (`resource_limits.rs:102,129`). A tight loop could overrun the deadline by up to ~1024 instructions of wall time before the next check. Verified overrun was tiny (200.03ms vs 200ms, §9.10) — the amortization is a reasonable cost/precision trade, but a pathological single-instruction-that-blocks (e.g. a slow FFI call, a `sleep`) would not be interrupted mid-instruction. This is the classic cooperative-safepoint limitation; acceptable for CPU-bound runaways, weaker for I/O-bound ones.
- **Output (`--max-output-bytes`)** — near-exact with truncation. Charged at the `print` sink (`vm_impl/builtins.rs:1707`); the breach fires when cumulative output crosses the cap, and output is truncated at the boundary line (verified: 1020 bytes / 20 lines for a 1000-byte cap, §9.11). Small overshoot (one line) is expected.
- **Memory (`--max-memory-bytes`)** — per-buffer, imprecise as a total-heap bound (§2.2/#6). Fires when any *single* `TypedArray` growth would exceed the ceiling; the observed breach size is the *would-be* buffer size (e.g. 16 MiB for a 10 MB ceiling — the doubling `realloc` target), not the cumulative heap. So the *reported* number can substantially exceed the configured cap, and cumulative heap across buffers is unbounded.

Net: three of four caps are precise-to-tight; memory is the outlier both in precision and in model.

### 3.3c Amortized wall-time check — a subtle correctness note

The wall-time check only runs inside `tick_instruction` (`resource_limits.rs:128-132`), i.e. only while the dispatch loop is executing bytecode. A program that enters a long-blocking native call (a slow `extern C` function, a large synchronous `http::get`, `time::sleep_sync`) does not tick instructions during the block, so the wall-time cap does not fire until control returns to the interpreter. For a serve node this means `--max-time-ms` bounds *Shape-level compute* but not *blocked-on-native* time. Not a defect per se (there is no portable way to preempt a blocking native call cooperatively), but a limit-semantics caveat worth documenting: the wall clock cap is a compute deadline, not a hard wall-time SLA.

### 3.4 Complexity hotspots

- `serve_cmd.rs` (2970 LOC) is the largest file, but the security logic within — `derive_serve_security` (`serve_cmd.rs:105`) — is a readable ~90-line mapping function. The bulk of the file is the server/transport machinery and an extensive test suite (tests start ~`serve_cmd.rs:1628`, including `strict_sandbox_refuses_file_write`, `none_sandbox_allows_file_write`, `derive_serve_security_maps_levels_and_bind_class`).
- `abi-v1/src/lib.rs` (2531 LOC) is large but mostly declarative enum/impl boilerplate; no deep control flow.
- No single security function exceeds ~90 lines. Cyclomatic complexity is low throughout the territory.

### 3.5 Dead / decorative code in-territory

- `ResourceUsage::record_allocation` (`resource_limits.rs:138-149`) — zero real callers (§2.2). Dead.
- `ResourceUsage.memory_bytes_allocated` field — written only by the dead `record_allocation`. Effectively dead.
- `DeterministicRuntime` (whole module, `deterministic.rs`) — compiled, tested, referenced nowhere in the execution loop (§2.3). Dead-but-tested.
- 5 decorative `Permission` variants (§2.4) — carried in every set/hash but drive no behavior.
- `ScopeConstraints::{max_memory_bytes,max_time_ms,max_output_bytes}` (`lib.rs:1411-1430`) — serialized, never read by the run/serve path (the caps come from `ResourceLimits` via CLI/`[sandbox]`, not from scope constraints). Effectively decorative in the current wiring.

This is the report's most consistent quality theme: **capable primitives carried as if wired, but not wired** — a maintenance hazard because a future reader will reasonably assume `record_allocation` bounds memory or that granting `MemLimited` caps memory.

---

## 4. Duplication & DRY Violations

### 4.1 Two permission-serialization formats for hashing (dangerous drift risk)

- `FunctionBlob::compute_hash` folds **sorted permission NAMES** (`Vec<&str>`) into its SHA-256 input (`content_addressed.rs:158-159`).
- `ModuleManifest::verify_integrity` folds `required_permission_bits` — a **bitfield** — into its hash (`module_manifest.rs`, `ManifestHashInput.required_permission_bits`).

Two independent encodings of "which permissions" feed two different content hashes. They are not obviously kept in sync: a change to the `Permission` enum ordinal affects the bitfield but not the name-string encoding (that is *intentional* for the blob hash — see the `Ffi`-reservation note at `lib.rs:1106-1117` — but the manifest bitfield encoding has the *opposite* sensitivity). If a future variant is inserted mid-enum, blob hashes stay stable (names unchanged) while manifest hashes shift (bits shift) — a silent divergence between the two "content addresses" of the same code. Not currently a live bug (all variants append), but a latent trap.

### 4.2 Permission→bool mapping duplicated three ways

The mapping between a `Permission` and its coarse config toggle appears in:
1. `PermissionsSection::to_permission_set` (`permissions.rs:150-198`) — struct-field → `Permission`.
2. `PermissionsSection::from_shorthand` (`permissions.rs:92-145`) — three hardcoded full sets (pure/readonly/full).
3. `serve_cmd::derive_serve_security` (`serve_cmd.rs:113-142`) — sandbox-level → hand-built set.

Three hand-maintained lists of "which permissions." `PermissionSet::readonly()` (`abi-v1/lib.rs:1241`) is a *fourth* definition of "read-only" (FsRead+Env+Time) that does NOT match `from_shorthand("readonly")` (which is FsRead+Env+Time, matching — but the `serve` permissive level adds Random+NetConnect, a fifth distinct "moderate" set). These will drift when a new permission is added: nothing forces all five call sites to be updated together.

### 4.3 `required_permissions` vs `module_permissions` in `capability_tags`

`capability_tags.rs` maintains BOTH a per-function map (`required_permissions`, line 14) and a per-module union (`module_permissions`, line 32), hand-written separately. The test `function_perms_subset_of_module_perms` (`capability_tags.rs:302-345`) exists precisely because these can drift — a good defensive test, but the duplication is real: `module_permissions` could be *derived* from `required_permissions` rather than hand-maintained.

### 4.4b Concrete "readonly" divergence

"Read-only" is defined in at least two places that already disagree in intent:

- `PermissionSet::readonly()` (`abi-v1/lib.rs:1241-1247`) = `{FsRead, Env, Time}`.
- `PermissionsSection::from_shorthand("readonly")` (`permissions.rs:110-125`) = `{FsRead, Env, Time}` (fs_read+env+time true, all else false) — matches.
- `serve permissive` (`serve_cmd.rs:116-123`) = `{FsRead, Env, Time, Random, NetConnect}` — a *different* "moderate read-ish" set.

So there are two "read-only-ish" definitions and one "permissive" that a reader could reasonably conflate. Adding a permission (say a hypothetical `Clipboard`) requires deciding, at each of these sites, whether "readonly" includes it — with nothing forcing the decision to be consistent. Today they happen to align on the two `readonly` definitions; the drift risk is structural, not yet realized.

### 4.4 Scope-glob trim logic

The trailing-glob prefix trim appears once for paths (`module_exports.rs:257`) and the host-wildcard logic once for hosts (`module_exports.rs:292`). Not duplicated across files, but both are ad-hoc string ops rather than a shared glob utility — and neither is a real glob engine (§9.7).

---

## 5. Split-Brain Analysis

### 5.1 `capability_tags` env function list vs. the actual `env` module exports — **REAL DRIFT**

`capability_tags::env_permissions` (`capability_tags.rs:87-92`) maps `get`, `has`, `all`, `args`, `cwd` → `Env`. But the *native* `env` module (`stdlib/env.rs`) only registers `has`, `cwd`, `os`, `arch` (verified by grep). So:

- `get`, `all`, `args` appear in the capability table but are **not exported by the native module** — verified: `env::get("HOME")` fails with `module 'env' has no export 'get'` (§9.8). (They exist in the *source* stdlib `stdlib-src/core/env.shape`, implying a native-vs-source env module split.)
- `os`, `arch` are exported by the native module but are **absent from the capability table** → they derive NO permission. `env::os()`/`env::arch()` reveal host OS/architecture (a fingerprinting surface) yet require no `Env` permission. Minor info-leak gap.

This is a classic split-brain: the permission table and the module registration are two hand-maintained lists of "what functions exist," and they have already drifted.

### 5.2 Two memory-limit mechanisms (`ResourceUsage` vs `alloc_budget`) — **DRIFT**

`ResourceLimits.max_memory_bytes` is consumed by TWO disjoint mechanisms: the dead `ResourceUsage::record_allocation` cumulative tracker (`resource_limits.rs:138`) and the live thread-local `alloc_budget` per-buffer ceiling (`alloc_budget.rs`). Only the latter fires. A reader seeing `ResourceUsage::memory_bytes_allocated` would reasonably assume cumulative accounting exists — it does not. The two implement contradictory models (cumulative vs per-buffer) of the same config field.

### 5.3 Permission-set definitions across four+ files (§4.2) — **DRIFT RISK**

Covered in §4.2 — `readonly` alone has ≥2 definitions that already differ from `serve permissive`. No single source of truth for "what does preset X grant."

### 5.4 `ScopeConstraints` resource caps vs `ResourceLimits` — **PARALLEL-CONFIG, unwired**

`ScopeConstraints` carries `max_memory_bytes`/`max_time_ms`/`max_output_bytes` (`lib.rs:1411-1430`) AND `ResourceLimits` carries the same three concepts (`resource_limits.rs:11-20`). The run/serve path builds `ResourceLimits` from CLI flags + `[sandbox]` (`script_cmd.rs:80-91`) and never reads the `ScopeConstraints` caps. Two config surfaces for the same limits; only one is wired. If a future contributor sets scope caps expecting enforcement, they get silence.

### 5.5 VM-vs-JIT — **NO split-brain here (correctly avoided)**

Permission enforcement lives at the stdlib FFI boundary (runtime `check_permission`) and at load time — both mode-independent. The JIT does not re-implement permission checks; stdlib calls route through the same runtime gate in both modes. Resource limits are the exception, and rather than duplicate them into JIT-native code, the CLI *forces the interpreter* when limits are set (`script_cmd.rs:125`). This is the right anti-split-brain move: one enforcement implementation, mode selection handles the gap. Verified: every limited run downgrades to interpreter (§9.10).

---

## 6. ADR & Spec Conformance

There is **no dedicated ADR for the capability/permission model** in `docs/adr/` — only ADR-005 (typed-slot construction) and ADR-006 (value & memory model) bind this territory, and only where the permission-check ABI touches value representation. The permission model itself is governed informally by CLAUDE.md's "Security Model (Three Tiers)" prose and by inline `ffi-rebuild §4.8.x` / `distributed §4.x` / `WF-1D`/`WF-2A`/`WF-2F` references scattered through the code comments. **This is itself a conformance gap (Finding, P2): a 17-permission security model with no architecture-decision record.** The "three tiers" description in CLAUDE.md is the closest thing to a spec, and it is partially inaccurate (§8).

### 6.1 ADR-005 — single-discriminator / typed-slot construction

| Rule | Conforms? | Evidence |
|------|:---:|----------|
| `HeapValue` is the canonical discriminator; no parallel sum types projecting 1:1 to `HeapKind` | **YES** | The permission types (`Permission`, `PermissionSet`, `ScopeConstraints`) are not heap values and introduce no parallel discriminator. `module_exports.rs` dispatches via `KindedSlot`→`HeapValue` per ADR (`module_exports.rs:6-29`). |
| Slot storage typed; no `Box<HeapValue>` wrappers | **YES** | No slot construction in the permission path. |

The permission subsystem is orthogonal to the value model; ADR-005 conformance is trivially satisfied (no discriminators introduced).

### 6.2 ADR-006 — value & memory model (§2.7.5 cross-crate ABI, §2.7.10 method ABI)

| Rule | Conforms? | Evidence |
|------|:---:|----------|
| §2.7.5 — internal Rust trait objects (ModuleFn) migrate to `KindedSlot`; raw-bits only for stable extension contracts | **YES** | `module_exports.rs:6-29` explicitly documents and enforces the split; `use shape_value::KindedSlot` (`module_exports.rs:29`); the raw-bits `RawCallableInvoker::invoke` stays on raw bits by design. Marker comments present. |
| §2.7.7 — parallel `Vec<NativeKind>` stack track; no `ValueWord` | **YES** | No `ValueWord`/tag-decode in the permission path (grep-confirmed empty). |
| §2.7 — refcount discipline via `KindedSlot::Drop`/`Clone` | **YES** | `module_exports.rs:24-25` documents the retain/release via `KindedSlot`. |
| Cell-storage parallel-kind (§2.7.8) — no Bool-default for `Load*Ptr` | **N/A** | Not touched by permission code. |

### 6.3 Forbidden-Patterns conformance (CLAUDE.md)

Grepped the territory (`capability_tags.rs`, `resource_limits.rs`, `crypto/`, `module_exports.rs`, `abi-v1/src/lib.rs`) for every forbidden symbol/rename family: `ValueWord`, `synthesize_value_word_from_raw`, `is_tagged`, `SlotKind::Dynamic`/`Unknown`, `exec_*_dynamic_fallback`, `call_value_legacy`, `MethodFnV2 bridge`, and the broader `(decode|tag|kind|dispatch|...) (bridge|probe|helper|hop|translator|adapter|shim)` regex.

**Result: ZERO hits.** The security territory is clean of all forbidden dynamic-dispatch patterns. No P0 forbidden-pattern finding. (This is expected — the permission subsystem never touched the value-word migration.)

### 6.4 Content-hash ordinal-stability rule (self-imposed spec, `lib.rs:1106-1117`)

The code imposes its own binding rule: `Ffi` MUST stay at the highest ordinal so that reserving it does not perturb existing content hashes, and hashes fold *sorted names* (not ordinals) for exactly this reason. **Conforms:** `Permission::Ffi` is last in the enum (`lib.rs:1117`), last in `all_variants()` (`lib.rs:1204-1205`), and the stability is pinned by the regression test `ffi_reservation_leaves_existing_content_hash_unchanged` (`linker_tests.rs:585`). The one ratified hash-rebaseline (2eb6e818→971ebf5f) is documented (`linker_tests.rs:578-583`) and the pinned value is asserted. This is a well-governed micro-spec.

**Latent conflict (§4.1):** the manifest hash uses `required_permission_bits` (ordinal-sensitive), contradicting the blob hash's ordinal-insensitive design. Not a live violation (all variants append) but an unrecorded tension.

### 6.5 Content-addressed blob permission fields (spec conformance to CLAUDE.md "Content-Addressed Bytecode")

CLAUDE.md's architecture spec states: "FunctionBlob: Self-contained bytecode unit with content_hash (SHA-256), required_permissions... Permissions baked into hash... Linker: Computes transitive union of all blobs' required_permissions at link time." Rule-by-rule:

| Spec rule | Conforms? | Evidence |
|-----------|:---:|----------|
| `FunctionBlob` carries `required_permissions` | **YES** | `content_addressed.rs:88-91` (`#[serde(default = "default_permission_set")] pub required_permissions: PermissionSet`). |
| Permissions folded into `content_hash` (SHA-256) | **YES** | `compute_hash` (`content_addressed.rs:157-190`) folds sorted names; digest is SHA-256 (`Sha256::digest`, `:186`). |
| Two functions with identical code + different permissions → different hashes | **YES** | `linker_tests.rs:597-605`. |
| Linker computes transitive union | **YES** | `linker.rs:406-408`, stored as `LinkedProgram.total_required_permissions` (`content_addressed.rs:576`). |
| Union checked at load | **YES** | `program.rs:391` (`is_subset`). |

Full conformance to the documented content-addressing security model. This is the part of CLAUDE.md that matches reality exactly.

### 6.6 Distributed spec (`distributed §4.6` "never trust the sender") conformance

The in-code `distributed §4.6` references (there is no `docs/adr/` file, only comments) require receiver-owned enforcement. `remote.rs:1024-1044` conforms: recompute-from-verified-blobs + receiver-set install + `None`-forbidden + per-language FFI opt-in. This is the strongest-conforming path in the territory. The absence of a formal ADR (§6 preamble) means these rules live only in comments and the `serve_cmd.rs` tests — fragile against a future refactor that does not read the comments.

---

## 7. Test Coverage In-Territory

### 7.1 Counts (measured `grep -c '#\[test\]'`)

| File | Tests | Quality |
|------|------:|---------|
| `capability_tags.rs` | 20 | Excellent — every module/function mapping asserted, plus the `function_perms_subset_of_module_perms` invariant (`:302`) and negative cases (unknown module/fn → empty). |
| `abi-v1/src/lib.rs` | 34 | Good — `PermissionSet` algebra, serde round-trip of `ScopeConstraints` (`:2416`), category grouping. |
| `crypto/signing.rs` | 6 | Strong — sign/verify, wrong-hash, corrupt-sig, wrong-key, timestamp, serde round-trip. |
| `crypto/keychain.rs` | 10 | Strong — trust levels, require-signatures, unsigned handling. |
| `module_manifest.rs` | 9 | Good — integrity + signature verification paths. |
| `project/permissions.rs` | 4 (ffi_tests) | Adequate for FFI defaults; **gaps below**. |
| `alloc_budget.rs` | 7 | Strong — ceiling bounds, transient non-accumulation, breach roundtrip, cross-run leak prevention. |
| `resource_limit_enforcement.rs` | 4 | Good — output clean-error, memory clean-error-not-panic, serve-worker-survives-breach. |
| `serve_cmd.rs` (sandbox) | ~8 | Strong — `strict_sandbox_refuses_file_write` (`:1926`), `none_sandbox_allows_file_write` (`:1982`), `derive_serve_security_maps_levels_and_bind_class` (`:1786`), `test_remote_permission_refusal_over_wire` (`:2508`), `test_remote_mutable_capture_refused_over_tcp` (`:2158`). |
| `compiler/statements.rs` (perm checks) | 6 | Good — `test_permission_check_blocks_file_import_under_pure` (`:8745`), `test_permission_check_no_permission_set_allows_everything` (`:8778`), namespace-import blocked/allowed. |
| `linker_tests.rs` (perm hash) | 1 | Critical — the hash-stability + Ffi-load-bearing test (`:585`). |

Territory total: **~100+ unit tests** directly on security logic. Density is high on the primitives.

### 7.2 Assertion quality

Assertions are precise, not smoke. Examples:
- `capability_tags.rs:125` asserts BOTH `contains(&FsRead)` AND `len() == 1` — catches over-granting.
- `linker_tests.rs:591-605` pins an exact hash string AND asserts the Ffi delta with `assert_ne!` — catches both accidental invalidation and silent ignoring.
- `resource_limit_enforcement.rs:119` (`serve_worker_survives_breach_then_next_request_succeeds`) asserts a SECOND execution on the same thread succeeds after a memory breach — a real serve-resilience property, not just "error fired."

### 7.3 Gaps (untested behavior that this audit exercised or found)

The following are **not covered by any test** I could find, and several are the report's findings:

1. **The dotted-key `fs.write = false` silent-ignore (Finding #1)** — no test asserts that `fs.write = false` in TOML actually denies. The 4 `permissions.rs` tests only cover the `ffi` field and use flat/quoted keys. A test parsing the *natural* dotted form would have caught this P0.
2. **Empty `[permissions]` → near-full trust (Finding #2)** — untested. No assertion pins the default-open behavior (arguably because it is a footgun no one wanted to bless).
3. **`Time` runtime backstop absence (Finding #8)** — no test asserts `time.millis` under a granted set lacking `Time`; the load gate is tested but not the runtime path (which does not exist).
4. **Deterministic reproducibility (Finding #4)** — no test asserts `sandbox.deterministic` makes `time.millis`/random reproducible (because it does not; the `deterministic.rs` tests only exercise the unwired `DeterministicRuntime` in isolation).
5. **Cumulative memory (Finding #6)** — `alloc_budget.rs:156` (`transient_buffers_do_not_accumulate`) tests that transient buffers don't false-trip, but nothing tests that many *retained* small buffers are bounded (they are not).
6. **Decorative permissions (Finding #3)** — no test asserts `Vfs`/`Capture`/etc. do anything, which is consistent (they don't), but also means their inertness is undocumented.
7. **End-to-end `shape run` permission enforcement** — the compiler-level `test_permission_check_*` tests use in-process compilation; there is no integration test driving the actual `shape run <file>` binary against a `shape.toml [permissions]` section. This audit's §9 transcripts are, as far as I can tell, the first end-to-end verification of the CLI surface.

### 7.4b Test-gap → finding mapping

Each report finding mapped to the test that *would* have caught it, had it existed:

| Finding | Missing test | Where it should live |
|---------|--------------|----------------------|
| #1 dotted-key ignore | parse `[permissions]\nfs.write = false` → assert `!granted.contains(FsWrite)` | `permissions.rs` tests |
| #2 default-open | parse empty `[permissions]` → assert the exact granted set (and decide if that's intended) | `permissions.rs` tests |
| #11 net wildcard | `check_net_permission("evilexample.com", ["*.example.com"])` → assert `Err` | `module_exports.rs` tests (none exist for `check_net_permission`) |
| #12 unknown key | parse `[permissions]\nbogus = false` → assert parse error | `permissions.rs` tests + `deny_unknown_fields` |
| #8 Time no runtime backstop | granted set without `Time`, call `time.millis` directly → assert denied | `stdlib_time.rs` tests |
| #4 determinism | `[sandbox] deterministic`, call `time.millis` twice → assert equal (currently unequal) | integration |
| #6 cumulative memory | retain N sub-ceiling non-array buffers → assert total bounded | `alloc_budget.rs` tests |

The pattern: the *primitives* are well-tested in isolation; the *config-to-enforcement* and *scope-boundary* seams — exactly where the fail-open findings live — have the coverage gaps. `check_net_permission` in particular has **zero** unit tests despite containing a security bug (#11).

### 7.4 Ignored tests

No `#[ignore]` tests found in the core security modules. The territory does not carry ignored-test debt (unlike the JIT/typed-array territories noted in CLAUDE.md). Clean.

---

## 8. Book / Docs vs. Reality

Source: `shape-web/book/book-site/src/content/docs/advanced/security-permissions.mdx` (526 lines) + CLAUDE.md "Security Model (Three Tiers)".

| Claim | Source | Reality | Verdict |
|-------|--------|---------|---------|
| "16 permissions organized into four categories" | book:222; CLAUDE.md | There are **17** — `Ffi` (`Foreign` category) exists (`lib.rs:1117`) but is absent from the book's Permission Enum table (book:222-263 lists only 16, no `Foreign` category). | **STALE** — book pre-dates the `Ffi` reservation |
| "the permission model is configured from the host... NOT available as Shape symbols in v0.3.3... planned for v0.4. Every code block... is non-runnable." | book:20 | The `shape.toml [permissions]` / frontmatter surface EXISTS and ENFORCES today (§9.1, §9.8, §9.9). It is a config-file surface, not a Shape-symbol surface, so the literal claim ("not available as Shape symbols") is technically true — but the blanket "configured from the host only" is misleading: a project author configures it via `shape.toml`, no host embedding required. The working surface is **entirely undocumented**. | **MISLEADING + UNDOCUMENTED FEATURE** |
| "two functions with different permission requirements always produce different hashes" | book:31 | TRUE — verified by `ffi_reservation_leaves_existing_content_hash_unchanged` (`linker_tests.rs:585`, `assert_ne!` at :605). | **ACCURATE** |
| `record_allocation(bytes)` "called when the VM allocates heap memory. Checks `max_memory_bytes`." | book:~230 | FALSE — `record_allocation` has zero real callers (§2.2). Memory is enforced by the disjoint per-buffer `alloc_budget::check_size` instead. | **WRONG** |
| `MemoryLimit` message "Memory limit exceeded: {allocated} bytes >= {limit} bytes" | book table | The actual surfaced message is "Shape memory limit exceeded: a single heap buffer reached {N} bytes, over the {L}-byte per-execution ceiling" (`alloc_budget.rs:111`). Different message, different semantics (per-buffer, not cumulative-allocated). | **WRONG** |
| `sandboxed()` = "Memory 256 MB" | book:192 | The 256 MB is a per-single-buffer ceiling, not a 256 MB total-heap cap (§2.2, Finding #6). | **OVERSELLS** |
| `Deterministic` = "Run in a deterministic runtime (fixed time, seeded RNG)" | book:259; CLAUDE.md | FALSE — `DeterministicRuntime` is unwired; the flag only refuses foreign code (§2.3, Finding #4). `time.millis` reads real `SystemTime` (§9.3). | **WRONG** |
| `MemLimited`/`TimeLimited`/`OutputLimited` = "Memory/Execution/Output ... is limited/capped" | book:261-263 | These permission variants are decorative; the actual caps come from `ResourceLimits`, disjoint from the permissions (§2.4, Finding #3). | **WRONG (implies enforcement)** |
| `Vfs` = "Operate against a virtual filesystem" | book:258 | The VFS machinery exists (`virtual_fs.rs`) but is not gated by the `Vfs` permission; granting/withholding `Vfs` does nothing. | **WRONG (implies gating)** |
| CLAUDE.md: "Runtime permission gating: Every stdlib I/O call guarded by check_permission (~5ns per call). 16 permissions" | CLAUDE.md | Mostly true for the 7 wired permissions; `Time` has no runtime check (load-only, §2.4/#8); count is 17. | **PARTIALLY ACCURATE** |
| CLAUDE.md: "Package signing: Ed25519 signatures on module manifests via ModuleSignatureData" | CLAUDE.md | The primitive exists and is correct, but signatures are **never verified** in `shape run`/`serve` (no keychain installed, §2.5/#5). | **OVERSELLS (primitive-only)** |
| Book Tier 1/2/3 "three-tier security model" | book:7-13 | Tiers 1 (load gate) and 2 (runtime gate) are real for wired permissions; Tier 3 (resource sandboxing) is real. The framing is accurate for the wired subset but omits that a third of permissions and the determinism runtime are inert. | **ACCURATE-BUT-INCOMPLETE** |

**Net:** the book is honest about the *shape* of the system (three tiers, content-hash embedding, the permission taxonomy) but wrong or misleading about (a) which permissions actually enforce, (b) how memory is limited, (c) whether determinism works, and (d) the availability of the working `shape.toml` surface. It also predates `Ffi`. The book's own "v0.4 preview" caution is, ironically, *more* pessimistic than reality on availability while *more* optimistic on determinism/memory.

---

## 9. Bugs & Correctness Risks (with repro transcripts)

Scratch programs live under `.../scratchpad/verticals/security-capabilities/`. Extension-load warnings (`libshape_ext_*`, "Loaded module: python/typescript") are filtered from transcripts per audit instructions.

### 9.1 P0 — `fs.write = false` (dotted TOML key) silently ignored; write stays OPEN

**Root cause:** `PermissionsSection` fields use `#[serde(rename = "fs.write")]` (`permissions.rs:24`), expecting a literal flat key named `"fs.write"`. But in TOML, `fs.write = false` is *dotted-key syntax* — it constructs a nested table `[permissions.fs] { write = false }`. The struct DOES have an `fs: Option<FsPermissions>` field (`permissions.rs:61`), and `FsPermissions` (`permissions.rs:68-76`) has fields `allowed`/`read_only` — NOT `write`. With no `#[serde(deny_unknown_fields)]` anywhere (grep-confirmed empty), the unknown `write` key is silently dropped, `fs_write` stays `None`, and `to_permission_set` defaults it to `true` (`permissions.rs:154`).

**Repro:**
```
# shape.toml
[permissions]
fs.read = true
fs.write = false     # <-- dotted key: silently parsed as [permissions.fs], ignored

# src/main.shape (top-level)
use std::core::file
print("start")
match file::write_text("/tmp/shape_audit_denied.txt", "should be denied") {
  Ok(_) => print("WRITE OK - PERMISSION BYPASS")
  Err(e) => print(f"WRITE DENIED: {e}")
}
```
```
$ shape run proj/src/main.shape
start
WRITE OK - PERMISSION BYPASS
$ ls -la /tmp/shape_audit_denied.txt
-rw-r--r-- 1 dev users 16 Jul 11 11:55 /tmp/shape_audit_denied.txt
```
The write succeeded despite `fs.write = false`. **Contrast** — the quoted-key form enforces correctly:
```
[permissions]
"fs.read" = true
"fs.write" = false
```
```
$ shape run proj/src/main.shape
Error: Semantic error: Permission denied at load: program requires permissions not granted: fs.write
```

**Severity P0:** a user writing the intuitive TOML gets a silent fail-open on the single most dangerous permission (filesystem write), with no warning, no error, no diagnostic. This is the exact anti-pattern a capability system must never exhibit. The fix is small (add `deny_unknown_fields` + normalize the key convention or add a config-lint), but the current behavior is a genuine security hole in the config surface.

### 9.2 P1 — Empty / partial `[permissions]` grants near-full trust (fail-open by field)

**Root cause:** `to_permission_set` uses `unwrap_or(true)` for every coarse boolean (`permissions.rs:153-176`). A field not present defaults to GRANTED. Only `ffi` defaults closed (`unwrap_or(false)`, `permissions.rs:181`).

**Repro (empty section):**
```
# shape.toml
[project]
name = "emptytest"
version = "0.1.0"
[permissions]
```
```
$ shape run empty/src/main.shape    # file::write_text(...)
start
EMPTY-SECTION WRITE OK (near-full trust)
$ ls -la /tmp/shape_empty_perm.txt
-rw-r--r-- 1 dev users 31 Jul 11 12:03 /tmp/shape_empty_perm.txt
```
An empty `[permissions]` section grants `{FsRead, FsWrite, NetConnect, NetListen, Process, Env, Time, Random}` — everything except `Ffi`. A user who adds `[permissions]` intending to *restrict* gets an almost-fully-open set unless they enumerate every `= false`. This inverts least-privilege: the safe default (deny) requires exhaustive opt-out, and any forgotten field is a grant.

**Repro (partial — deny only some):** the `pure` project in §9.8 sets `env=false, process=false, "net.connect"=false, "fs.read"=false, "fs.write"=false` but omits `time`/`random`; `time::millis()` is consequently allowed (§9.3 transcript). The fields you forget are the fields you grant.

**Severity P1:** combined with #9.1, the config surface has two independent fail-open paths reachable by ordinary use. Mitigation exists (`from_shorthand("pure")` sets everything false, `permissions.rs:94-109`) but requires knowing to use the shorthand.

### 9.3 P1 — `sandbox.deterministic = true` does not make time/random reproducible

**Root cause:** §2.3 — `DeterministicRuntime` unwired; `time.millis` (`stdlib_time.rs:208`) reads real `SystemTime`, ignores `_ctx`.

**Repro:** under the `pure` project (which has no `time` key → `Time` granted, §9.2), `time.millis()` returns a live epoch value:
```
$ shape run pure/src/time.shape
start
time 1783764199979.0
```
The value is the real wall clock (2026-07-11 epoch millis), not a virtual clock. Adding `[sandbox] deterministic = true` would insert `Permission::Deterministic` — whose only effect is refusing foreign code — and would NOT alter this output. The book's promise "fixed time, seeded RNG" is unimplemented in the loop.

**Severity P1:** determinism is a load-bearing feature for the "resumable distributed execution" and "reproducible sandbox" pitches (project memory `project_priority_verticals.md`); a determinism flag that silently does not deterministic-ize time/random is a correctness trap for anyone relying on it.

### 9.4 P1 — Module signature verification never runs in `shape run`/`serve`

**Root cause:** §2.5 — no `set_keychain` call on the run/serve paths; `self.keychain` stays `None` (`lib.rs:246`), so `module_loader/mod.rs:882` (`if let Some(keychain) = &self.keychain`) is never entered.

**Evidence:**
```
$ grep -rn "set_keychain" bin/shape-cli/src/commands/script_cmd.rs bin/shape-cli/src/commands/serve_cmd.rs
   (no output)
```
The `shape keys trust` command writes a keychain file (`keys_cmd.rs`), but nothing loads it into the engine before execution. Signed modules and unsigned modules load identically; a tampered module fails only the *integrity* hash check (`verify_integrity`, always run) — which catches corruption but NOT a validly-hashed module from an untrusted author. Author-trust enforcement is dead in the run path.

**Severity P1:** the "Ed25519 package signing" security claim (CLAUDE.md, book) is a primitive with no consumer in the execution path. Integrity is checked; *authenticity/trust* is not.

### 9.5 P1 — `--max-memory-bytes` is per-buffer, not cumulative; only TypedArray instrumented

**Root cause:** §2.2/§5.2 — `check_size` single caller `typed_array.rs:297`; `record_allocation` dead.

**Repro (the per-buffer ceiling DOES fire for a single growing buffer):**
```
$ shape run --max-memory-bytes 10000000 mem.shape   # arr.push in a loop
[shape] resource limits set — running under the bytecode interpreter ...
Error: Runtime error: Shape memory limit exceeded: a single heap buffer reached 16777216 bytes, over the 10000000-byte per-execution ceiling (likely an unbounded allocating loop)
```
**The gap:** a program allocating many *separate* live buffers each under the ceiling, or many strings/objects (uninstrumented), is not bounded to the configured total. In §9.5's array-of-arrays test the *outer* accumulator array trips its own buffer ceiling first (partial mitigation), but for non-array retention (deeply nested objects, many retained strings) there is no cumulative accounting. The `alloc_budget.rs:14-25` doc-comment is honest about this; the CLI help ("Cap heap growth") and book ("256 MB") are not.

**Severity P1** for a serve node running untrusted transferred code: the memory DoS surface is only partially closed (single-buffer runaways, the common case, are caught; distributed-allocation runaways are not).

### 9.6 P2 — Runtime permission/scope denial is a fatal uncatchable error, not `Result::Err`

**Root cause:** gated stdlib fns return `Err(String)` from `check_permission` (`module_exports.rs:227`), which propagates as a `VMError` aborting the program rather than materializing as a Shape-level `Result::Err` the program can `match`.

**Repro (scope denial):**
```
# [permissions] "fs.write" = true, [permissions.fs] allowed = ["/tmp/shape_allowed"]
match file::write_text("/tmp/shape_allowed/ok.txt", "in scope") { Ok(_) => print("IN-SCOPE WRITE OK") Err(e) => ... }
match file::write_text("/tmp/shape_outside.txt", "out")        { Ok(_) => ... Err(e) => print(f"OUT-SCOPE DENIED: {e}") }
```
```
$ shape run scoped/src/in.shape
start
IN-SCOPE WRITE OK
Error: Runtime error: Scope constraint denied: path '/tmp/shape_outside.txt' is not in allowed paths (line 7)
```
The `Err(e)` arm was **not** taken — the program aborted. The book (`security-permissions.mdx:449`) documents `→ Err("Permission denied: ...")`, implying a catchable `Result`. Reality: uncatchable fatal error. Note this *also proves scope narrowing works* (in-scope succeeded, out-of-scope blocked) — it is the recovery semantics that mismatch the docs.

**Severity P2:** correct enforcement, wrong (undocumented) failure mode. A sandbox host that wants "deny this call, let the program handle it" cannot get that behavior; every denial is fatal.

### 9.7 P2 — Scope path matching is prefix-only, not glob

**Root cause:** `module_exports.rs:257` trims trailing `*`/`**`/`/` to a prefix and uses `Path::starts_with`. Mid-pattern globs are not interpreted.

**Repro (working case — trailing dir prefix):** §9.6 shows `/tmp/shape_allowed` correctly gating. `Path::starts_with` is component-aware, so `/tmp/shape_allowed` does not match a sibling `/tmp/shape_allowed_evil` — good (no string-prefix hole). But a pattern like `/tmp/*/logs` never matches any real path (the literal `*` component). The doc-comment calls it "glob-style" (`module_exports.rs:255`), overstating capability.

**Severity P2:** not a security hole (fails closed — an unmatched pattern denies), but a usability trap and a documentation inaccuracy. A user writing `/data/*/public` expecting glob semantics gets a deny-all.

### 9.8 (supporting) Env load-gate enforcement — WORKS

```
# pure project: [permissions] with env=false
use std::core::env
let h = env::has("HOME")
```
```
$ shape run pure/src/env.shape          # env = false
Error: Semantic error: Permission denied at load: program requires permissions not granted: sys.env
$ shape run pure/src/env.shape          # env = true
start
ENV has HOME: true
```
Confirms load-time env gating. Also surfaced the split-brain (§5.1): `env::get` fails with "module 'env' has no export 'get'" — the native module lacks `get`/`all`/`args` that `capability_tags` maps.

### 9.9 (supporting) Net load-gate enforcement — WORKS

```
use std::core::http
match http::get("http://example.com") { ... }
```
```
$ shape run pure/src/net.shape          # "net.connect" = false
Error: Semantic error: Permission denied at load: program requires permissions not granted: net.connect
```

### 9.10 (supporting) Instruction + wall-time limits — WORK

```
$ shape run --max-instructions 1000000 loop.shape
[shape] resource limits set — running under the bytecode interpreter ...
Error: Runtime error: Instruction limit exceeded: 1000000 >= 1000000
$ shape run --max-time-ms 200 loop.shape
[shape] resource limits set — running under the bytecode interpreter ...
Error: Runtime error: Wall time limit exceeded: 200.032503ms >= 200ms
```
The `[shape] ... running under the bytecode interpreter` note confirms the JIT-bypass mitigation (`downgrade_mode_for_limits`) fires.

### 9.11 (supporting) Output limit — WORKS (with truncation)

```
$ shape run --max-output-bytes 1000 output.shape   # loop of 50-char prints
AAAA... (20 lines) ...
Error: Runtime error: Output limit exceeded: 1020 bytes >= 1000 bytes (line 3)
```
Output truncated at the cap (20 lines ≈ 1020 bytes) then a clean error — matches `resource_limit_enforcement.rs:6-10`'s "Defect A" fix.

### 9.12 (supporting) Ffi load-gate enforcement — WORKS

```
fn python add(a: int, b: int) -> Result<int, string> { return a + b }
```
```
$ shape run ffi/src/main.shape          # [permissions] present, ffi unset (defaults false)
Error: Semantic error: Permission denied at load: program requires permissions not granted: ffi.call
```
Confirms `Ffi` is derived (`functions_foreign.rs:310`) and load-gated. `ffi = true` opts in.

### 9.13 (supporting) Random runtime gate — WORKS (fatal)

```
use std::core::crypto
let b = crypto::random_bytes(16)
```
```
$ shape run pure/src/rand.shape          # random = false
start
Error: Runtime error: Permission denied: Access random number generation (sys.random) (line 3)
```
The program printed "start" then aborted at the call — confirming a runtime (not load) denial. `random = true` (or omitting the key, which defaults `true`) allows it: "got bytes".

### 9.14 P1 — Network scope wildcard `*.example.com` matches `evilexample.com`

**Root cause:** `check_net_permission` (`module_exports.rs:292-293`):
```rust
if let Some(suffix) = pattern_host.strip_prefix("*.") {
    target_host.ends_with(suffix) && target_host.len() > suffix.len()
}
```
For pattern `*.example.com`, `strip_prefix("*.")` yields `suffix = "example.com"` — **the boundary dot is stripped**. The match then admits any `target_host` that (a) ends with the string `example.com` and (b) is strictly longer. That includes:
- `sub.example.com` ✓ (intended)
- `evilexample.com` ✗ **(unintended — no subdomain boundary)**
- `notexample.com`, `myexample.com`, `xexample.com` ✗ (all admitted)

An operator scoping outbound network to `*.example.com` (intending "subdomains of example.com only") inadvertently permits an attacker-registered `evilexample.com`. The `len() > suffix.len()` guard only rejects the exact apex `example.com`; it does not require the character before the suffix to be a `.`.

**Reasoning-level repro** (the check is a pure function; live HTTP is unreliable in-sandbox so I verify by the logic): `check_net_permission(ctx, NetConnect, "evilexample.com:80")` with `allowed_hosts = ["*.example.com"]` → `target_host = "evilexample.com"` → `"evilexample.com".ends_with("example.com")` = `true`, `15 > 11` = `true` → **`allowed = true`** → `Ok(())`. The connection is permitted.

**Correct fix:** strip only `"*"` (retain the dot) → `suffix = ".example.com"` and check `ends_with(".example.com")`, or verify `target_host[.. len-suffix.len()]` ends in `.`. **Severity P1:** a network egress allow-list that admits look-alike domains is a real exfiltration/SSRF-scope-escape surface for a serve node or scoped run. (This bug is present but untested — no unit test exercises `check_net_permission` wildcard boundaries; §7.3.)

### 9.15 P1 — Unknown `[permissions]` keys and the string shorthand both fail open

**Two sub-cases, same fail-open root (no `deny_unknown_fields`, table-only field type):**

**(a) Unknown/typo key silently ignored:**
```
[permissions]
totally_bogus_key = false
```
```
$ shape run short/src/main.shape      # file::write_text(...)
WRITE OK
$ ls /tmp/short_test.txt              # file created
/tmp/short_test.txt
```
The bogus key is dropped; every real permission stays at default-`true`; the write succeeds. Any typo (`fs_wrte`, `fswrite`, `write`) is a silent grant. This generalizes Finding #1: it is not just the dotted-key collision — *any* key the struct does not recognize is ignored without warning.

**(b) Documented shorthand unreachable at top level:** `PermissionsSection::from_shorthand("pure"|"readonly"|"full")` (`permissions.rs:92`) exists and is unit-tested, but the top-level `project_config.rs:64` field is `permissions: Option<PermissionsSection>` (table only) — NOT `Option<PermissionPreset>` (the untagged string-or-table enum, which lives only on `dependency_spec.rs:29`). So `permissions = "readonly"` in a top-level `shape.toml`:
```
[project]
name = "short"
version = "0.1.0"
permissions = "readonly"
```
```
$ shape run short/src/main.shape
WRITE OK
```
The write succeeded — `readonly` did NOT deny `fs.write`. The string form is not parsed into a `PermissionsSection`, so the run falls back to `granted = None` → allow-all. A user reaching for the documented `readonly` preset at the project level gets the *opposite* of least privilege: full trust, silently.

**Severity P1:** both sub-cases turn a user's restriction attempt into a no-op grant. Fix is `deny_unknown_fields` + accepting `PermissionPreset` at the top level (or erroring on a string).

### 9.16 (supporting) Process spawn gate — code cite

`io::spawn`/`io::exec` route through `check_permission(ctx, Process)` (`process_ops.rs`, 13 sites incl. `:91,132,158,189`). Under a `[permissions] process = false` envelope the load gate also denies (`io::spawn` derives `Process` via `capability_tags.rs:67`). Both tiers cover `Process`; I did not spawn a live child (sandbox hygiene) but the enforcement sites are dense and dual-tier, matching the FsRead/FsWrite pattern verified in §9.1.

---

## 10. What Is Done Well

1. **The content-hash permission-baking is genuinely elegant and correct.** Folding *sorted permission names* (not ordinals) into the SHA-256 blob hash (`content_addressed.rs:158`) makes the hash sensitive to *what* a function may do while remaining stable under enum reordering — and the team pinned this with a regression test that asserts BOTH stability (exact hash) and load-bearing-ness (`assert_ne!` on the Ffi delta, `linker_tests.rs:591-605`). The `Ffi`-at-highest-ordinal reservation discipline (`lib.rs:1106-1117`) shows the authors understood the failure mode and engineered around it deliberately. This is the strongest single design in the territory.

2. **Two-tier enforcement (load gate + runtime backstop) with a clean redundancy story.** The load-time subset check fails closed *before any instruction runs* (`program.rs:391`), and the runtime `check_permission` catches anything the static derivation missed. For the 7 wired permissions this is defense-in-depth done right, and both tiers key off the same `granted_permissions` set — no chance of the two tiers disagreeing about what was granted.

3. **The JIT-bypass mitigation is the correct anti-split-brain move.** Rather than re-implement resource limits in Cranelift-native code (which would inevitably drift), `downgrade_mode_for_limits` (`script_cmd.rs:125`) forces the interpreter when any limit is set and prints a one-line note. One enforcement implementation, mode selection closes the gap. Verified firing (§9.10).

4. **Serve/remote is fail-closed by construction.** `PermissionSet::pure()` as the strict default, `intersection(&pure())` on non-loopback binds (`serve_cmd.rs:149`), receiver-recomputes-the-union-from-verified-blobs ("never trust the sender," `remote.rs:1024`), and the explicit prohibition of `None` on the remote path (`remote.rs:1029`). The network-facing surface — where it matters most — is the most conservative part of the system.

5. **`alloc_budget`'s honest self-documentation.** The module doc-comment (`alloc_budget.rs:14-25`) explicitly states it is a per-buffer ceiling, NOT a cumulative budget, and explains *why* (no credit-on-free needed, no false-positives on transient loops). Even though the *outward* docs oversell it (§8), the *code's* doc-comment is scrupulously honest. The non-panicking `record_breach`/`take_breach` design (surfacing at a safepoint instead of aborting the process) is exactly right for a serve worker (`resource_limit_enforcement.rs:11-16`).

6. **The Ed25519 primitive is textbook.** `signing.rs` is a minimal, correct wrapper with 6 focused tests including the three that matter (wrong-hash, corrupt-sig, wrong-key). `unwrap_or_default()` on the timestamp (`signing.rs:35`) is the right defensive choice. The `Keychain` trust-level model (`Full`/`Scoped`/`Pinned`) is a thoughtful policy taxonomy. The only problem is that nothing calls it in the run path — the code itself is high quality.

7. **`PermissionSet` set-algebra completeness.** Full, orthogonal `union`/`intersection`/`difference`/`is_subset`/`is_superset` over a deterministic `BTreeSet`. This made the linker union (`linker.rs:407`) and the subset gate (`program.rs:391`) one-liners. Good abstraction boundary.

8. **Capability-table test discipline.** `capability_tags.rs` has 20 tests including the `function_perms_subset_of_module_perms` invariant (`:302`) — the authors anticipated the per-function/per-module drift risk and guarded it, even if the underlying duplication remains (§4.3).

9. **`Ffi` foreign-code posture is carefully reasoned.** The asymmetry — local `shape run` grants `Ffi` unscoped by default (hello-world works) but an explicit `[permissions]` section fails closed on `Ffi` (`permissions.rs:177-183`), and serve requires per-language opt-in even under `--sandbox off` (`serve_cmd.rs:152-179`) — reflects real threat-modeling (foreign code = process authority). The `deterministic_foreign_gate` (refuse foreign under determinism, even when Ffi granted) is a subtle, correct rule.

---

## 11. What Is Done Poorly / Tech Debt

1. **The config surface fails open twice (Findings #1, #2).** No `deny_unknown_fields`, dotted-key/serde-rename collision, and `unwrap_or(true)` defaults combine so that the two most natural user inputs (`fs.write = false`; a minimal `[permissions]` block) both silently grant more than intended. This is the highest-priority debt: a capability system whose config fails open is worse than no config, because it projects false assurance.

2. **Decorative permissions (Finding #3).** Carrying `Vfs`/`Capture`/`MemLimited`/`TimeLimited`/`OutputLimited` as first-class enum variants — folded into every hash, listed in the book with enforcement-implying descriptions — while they drive zero behavior is actively misleading. Either wire them or mark them reserved/hidden. The `MemLimited`/`TimeLimited`/`OutputLimited` trio is especially bad because it *looks* like it controls the resource limits that are actually controlled by the disjoint `ResourceLimits`.

3. **Dead code carried as if live.** `ResourceUsage::record_allocation` + `memory_bytes_allocated` (a whole cumulative-memory mechanism that no one calls) and the entire `DeterministicRuntime` module. A maintainer will reasonably assume these work. Both should be either wired or deleted. The book even documents `record_allocation` as the memory mechanism (§8) — the dead code has leaked into the spec.

4. **`DeterministicRuntime` unwired but doc-claimed (Finding #4).** The module asserts in its own doc-comment that "the VM routes `time.millis()` ... through this module," which is false. This is the one place the *code's* documentation lies (contrast `alloc_budget`'s honesty).

5. **Signature verification unreachable (Finding #5).** A complete signing/keychain/trust stack with no consumer in the execution path. `shape keys trust` writes state that nothing reads back.

6. **No ADR for the capability model.** A 17-permission security-critical subsystem governed only by scattered `WF-*`/`ffi-rebuild §4.8` code comments and a partially-wrong book chapter. The design decisions (why fail-open-per-field? why per-buffer memory? why are 5 permissions reserved-but-inert?) are undocumented. Given CLAUDE.md's heavy ADR culture elsewhere, the security model's absence from `docs/adr/` is conspicuous.

7. **Permission-set definitions duplicated across ≥5 sites (§4.2).** `readonly` alone has multiple non-matching definitions. Adding a permission requires touching `to_permission_set`, `from_shorthand`, `serve derive_serve_security`, `PermissionSet::readonly`, and `capability_tags` — with nothing forcing consistency.

8. **Split-brain between `capability_tags` and the native `env` module (§5.1).** The table maps functions that don't exist (`get`/`all`/`args`) and omits ones that do (`os`/`arch`, which leak host info without an `Env` requirement).

9. **Two hash encodings of "which permissions" (§4.1).** Blob hash uses sorted names (ordinal-insensitive by design); manifest hash uses `required_permission_bits` (ordinal-sensitive). Latent divergence on any mid-enum insertion.

10. **`ScopeConstraints` resource caps unwired (§5.4).** Three fields (`max_memory_bytes`/`max_time_ms`/`max_output_bytes`) that duplicate `ResourceLimits` and are never read by the run/serve path.

11. **Uncatchable denial semantics undocumented (Finding #7).** Permission denials abort the program; the book implies recoverable `Result::Err`.

12. **Network wildcard boundary bug (Finding #11, §9.14).** `*.example.com` admits `evilexample.com` — a scope-narrowing bypass in security-critical egress-filtering code, present without a unit test.

13. **Unknown-key silent-accept + unreachable shorthand (Finding #12, §9.15).** Any typo'd `[permissions]` key is silently ignored (default-open), and the documented `pure`/`readonly`/`full` shorthands cannot be used at the top level. The config surface has no fewer than *three* distinct fail-open paths (dotted-key #1, empty/partial #2, unknown-key/shorthand #12).

14. **No CLI sandbox for local runs (§2.9).** `shape run` cannot install a permission envelope without authoring config; the deny-by-default posture is CLI-unreachable.

---

## 12. Prioritized Recommendations

### P0 (do before any "secure sandbox" claim ships)

1. **Close the config fail-open (Findings #1, #2).** Add `#[serde(deny_unknown_fields)]` to `PermissionsSection` and `FsPermissions`/`NetPermissions` so `fs.write = false` (which lands on the `fs` sub-table) is a hard parse error instead of a silent ignore. Additionally, decide the least-privilege default: either flip unset-field defaults to `false` when a `[permissions]` section is present (breaking but correct), or emit a warning enumerating every implicitly-granted permission. Add integration tests driving the actual `shape run` binary against both the dotted and quoted TOML forms. **Effort: ~0.5–1 day** (small code change; the test harness is the bulk).

2. **Add an end-to-end permission-enforcement integration test suite** driving `target/debug/shape run` against `shape.toml [permissions]` fixtures for all 7 wired permissions + scope narrowing + all 4 resource limits. This audit's §9 transcripts are currently the only end-to-end evidence. **Effort: ~1 day.**

2b. **Fix the network wildcard boundary bug (Finding #11, §9.14).** Change `module_exports.rs:292-293` to require a dot boundary: strip `"*"` (not `"*."`) so `suffix = ".example.com"` and check `target_host.ends_with(suffix)`. Add unit tests for `*.example.com` vs `evilexample.com`/`example.com`/`sub.example.com`. This is a genuine egress-scope bypass; treat as P0-adjacent for any deployment using `net.scoped`. **Effort: ~1 hour.**

### P1 (correctness / assurance)

3. **Either wire or delete the decorative permissions and dead code (Findings #3, #4).** For `Vfs`/`Capture`/`MemLimited`/`TimeLimited`/`OutputLimited`: if they will not be enforced soon, mark them `#[doc(hidden)]` / "reserved" and remove them from the book's enforcement-implying table. Delete `ResourceUsage::record_allocation` + `memory_bytes_allocated` (or wire cumulative accounting). For `DeterministicRuntime`: either wire it into `time.millis`/random dispatch under `Permission::Deterministic`, or delete it and correct the book's "fixed time, seeded RNG" claim. **Effort: ~1 day to delete/mark; ~3–5 days to actually wire determinism.**

4. **Wire keychain into the run/serve paths (Finding #5).** Load the user keychain (`~/.shape/keys`) into the engine before execution and honor `require_signatures`. At minimum, a `shape run --require-signatures` flag. **Effort: ~1–2 days.**

5. **Make the memory cap cumulative or rename it (Finding #6).** Either instrument all heap-allocating paths (strings/objects/hashmaps) with a running total (restoring `record_allocation` semantics), or rename the flag/docs to `--max-buffer-bytes` and drop the "256 MB memory limit" framing. For a serve node running untrusted code, cumulative accounting is the correct target. **Effort: ~3–5 days for true cumulative; ~1 hour to rename.**

6. **Write an ADR for the capability model.** Record: the 17 permissions and their enforcement status, the fail-open-when-`None` posture, the per-buffer memory model, the two-hash-encoding decision, and the `Ffi`/`Deterministic` special rules. This subsystem is too security-critical to live only in code comments. **Effort: ~1 day.**

### P2 (polish / hardening)

7. **Add a `Time` runtime backstop** (`check_permission(ctx, Time)` in `time.millis`, `stdlib_time.rs:208`) so it matches the other System permissions and is not load-derivation-only (Finding #8). Add `Env` requirement to `env::os`/`env::arch` (§5.1). **Effort: ~1 hour.**

8. **Reconcile `capability_tags` with the native `env` module (§5.1)** — remove `get`/`all`/`args` or add the exports; add `os`/`arch`. Consider deriving `module_permissions` from `required_permissions` to kill the duplication (§4.3). **Effort: ~2 hours.**

9. **Replace the prefix-only scope matcher with a real glob** (`globset` crate) or narrow the doc-comment to "prefix match" (Finding #9). **Effort: ~2–4 hours.**

10. **Document (or change) the uncatchable-denial semantics (Finding #7)** — decide whether permission denials should be fatal or `Result::Err`, and make the book match. **Effort: ~2 hours doc; larger if changing semantics.**

11. **Unify the ≥5 permission-set definitions (§4.2)** behind a single source of truth (e.g. a `preset → PermissionSet` table consumed by `from_shorthand`, `serve`, and `readonly`). **Effort: ~half day.**

12. **Add a config-lint / `shape check --permissions`** that prints the effective granted set and warns on suspicious patterns (empty section, unknown keys, dotted-key mistakes). This turns the fail-open footguns into visible diagnostics. **Effort: ~1 day.**

13. **Accept `PermissionPreset` at top level (Finding #12b)** so `permissions = "readonly"` works as documented, and error (not ignore) a string that is neither a known preset nor a table. **Effort: ~2 hours.**

14. **Add `shape run --sandbox {strict,permissive,none}`** mirroring the serve semantics (§2.9), so an untrusted local script can be run deny-by-default without authoring config. **Effort: ~half day** (the `derive_serve_security` mapping already exists to reuse).

---

## 13. Threat-Model & Attack-Surface Summary

To make the findings actionable, here is the subsystem viewed as an attacker would see it, per deployment posture.

### 13.1 Posture A — local `shape run`, no `[permissions]` (the default)

- **Granted set:** `None` → allow-all. Both tiers short-circuit to "allow."
- **Attacker capability:** full — any FS/net/process/env/random operation the host user can do. `Ffi` unscoped (extern C, python, typescript with process authority).
- **Only real bound:** resource limits, and only if the user passes `--max-*` flags (default none).
- **Verdict:** this is "trusted local execution" by design, and it is genuinely trust-everything. Fine for running your own code; catastrophic for running someone else's, and there is no CLI flag to lock it down (§2.9).

### 13.2 Posture B — local `shape run` with `[permissions]`

- **Granted set:** derived from `shape.toml`/frontmatter. **This is where the footguns live.** An attacker who *authors* the config is not the threat; the threat is a *defender* who authors it wrong and believes they are protected. Findings #1, #2, #11, #12 all make "I tried to restrict X" silently equal "X is allowed."
- **What actually holds once the set is correct:** the load gate + runtime backstop are non-bypassable from Shape code (no reflection, no dynamic dispatch escape — §6.3 forbidden-patterns clean). Scope narrowing works (modulo the wildcard bug #11).
- **Verdict:** enforcement is sound; *reaching* a correct enforcement config is the hazard.

### 13.3 Posture C — `shape serve` (network-facing, untrusted transferred code)

- **Granted set:** `strict`→`pure()` default; non-loopback binds intersect to `pure()`; receiver recomputes union from verified blobs; `None` forbidden.
- **Attacker capability (strict, default):** nothing — no FS, net, process, env, random, or FFI. Resource-capped (`sandboxed()`).
- **Residual risks:** (a) the memory cap is per-buffer (#6) — a distributed-allocation runaway of many sub-ceiling buffers is not bounded, a partial DoS surface; (b) the network wildcard bug (#11) applies if the operator uses `net.scoped`; (c) no signature/author-trust check (#5) — but integrity is verified and permissions are receiver-owned, so a tampered blob cannot escalate, only mis-attribute authorship.
- **Verdict:** the strongest posture; fail-closed by construction. The residual risks are real but bounded (DoS-shaped, not privilege-escalation-shaped).

### 13.4 What an attacker CANNOT do (the system's genuine guarantees)

- Cannot bypass a *correctly-installed* permission set from Shape code — there is no dynamic-dispatch escape hatch (forbidden patterns absent, §6.3), no reflection, no ungated intrinsic path to I/O (intrinsics are gated per CLAUDE.md's Builtins rules, orthogonal to this audit but relevant).
- Cannot forge a content hash to smuggle different permissions — the hash folds the permission names (§6.4).
- Cannot, as a distributed sender, escalate beyond the receiver's grant (§13.3).
- Cannot exhaust the host via a single unbounded `arr.push` loop under `--max-memory-bytes` — that specific runaway IS caught (§9.5).

The security model's *core* is real. Its *perimeter* (config ergonomics, decorative permissions, unwired determinism/signing, the wildcard boundary) is where the gaps concentrate — and those gaps are exactly the ones most likely to give a defender false confidence.

---

### Appendix A — Enforcement-status one-glance table

| Permission | Wired? | Where |
|-----------|--------|-------|
| FsRead | runtime + load | `file_ops.rs`, `file.rs:45`, `capability_tags` |
| FsWrite | runtime + load | `file_ops.rs:260`, `file.rs:70` |
| NetConnect | runtime + load | `network_ops.rs`, `http.rs`, `remote/transport_builtins.rs` |
| NetListen | runtime + load | `network_ops.rs:87` |
| Process | runtime + load | `process_ops.rs` ×13 |
| Env | runtime + load | `env.rs:40+` |
| Random | runtime | `crypto.rs:195,215` |
| Ffi | load + dynamic-dispatch | `functions_foreign.rs:310`, `control_flow/mod.rs:1116`, `program.rs:14` |
| Deterministic | load (foreign-refuse) | `program.rs:10-20` |
| Time | load only | `capability_tags.rs:106` (no runtime check) |
| FsScoped | label + scope narrowing | `permissions.rs:188`, `module_exports.rs:244` |
| NetScoped | label + scope narrowing | `permissions.rs:195`, `module_exports.rs:278` |
| Vfs | **none** | decorative |
| Capture | **none** | decorative |
| MemLimited | **none** | decorative (mem via `ResourceLimits`) |
| TimeLimited | **none** | decorative (time via `ResourceLimits`) |
| OutputLimited | **none** | decorative (output via `ResourceLimits`) |

### Appendix B — Files read (evidence trail)

`abi-v1/src/lib.rs` (§1021-1520), `capability_tags.rs` (full), `resource_limits.rs` (full), `alloc_budget.rs` (full), `signing.rs` (full), `keychain.rs` (§76-133), `module_exports.rs` (§180-320), `module_manifest.rs` (verify_integrity), `permissions.rs` (full), `content_addressed.rs` (§140-209), `linker.rs` (§400-408), `program.rs` (§1-45), `execution.rs` (§700-740), `remote.rs` (§1020-1044), `script_cmd.rs` (§1-260), `serve_cmd.rs` (§40-180), `cli_args.rs` (§300-450), `deterministic.rs` (§1-40), `virtual_fs.rs` (head), `stdlib_time.rs` (§201-230), `env.rs` (function names), `module_loader/mod.rs` (§855-934), `security-permissions.mdx` (§1-270, key lines to 526).

### Appendix C — Empirical test programs (scratchpad)

All under `.../scratchpad/verticals/security-capabilities/`:

| Program / project | Tests | Result |
|-------------------|-------|--------|
| `t_write.shape` (no project) | default FS write | WRITE OK (fail-open, §9 preamble) |
| `proj/` (`"fs.write" = false`) | quoted-key deny | Denied at load (§9.1) |
| `proj/` (`fs.write = false`) | dotted-key deny | **BYPASS — write OK** (§9.1, P0) |
| `empty/` (`[permissions]` empty) | default-open | WRITE OK (§9.2, P1) |
| `pure/` (env/net/fs false) | env/net/time/random gates | env+net denied, time+random allowed by default (§9.3, §9.8, §9.9, §9.13) |
| `scoped/` (fs allowed=[/tmp/shape_allowed]) | path scope narrowing | in-scope OK, out-of-scope fatal (§9.6) |
| `hostscope/` (`*.example.com`) | net host wildcard | logic reviewed → boundary bug (§9.14, P1) |
| `short/` (`permissions="readonly"` / bogus key) | shorthand + unknown key | both fail open — WRITE OK (§9.15, P1) |
| `ffi/` (`fn python`, ffi unset) | Ffi load gate | Denied at load (§9.12) |
| `loop.shape` `--max-instructions`/`--max-time-ms` | instruction/time caps | both fire (§9.10) |
| `output.shape` `--max-output-bytes` | output cap | fires + truncates (§9.11) |
| `mem.shape` / `manymem.shape` `--max-memory-bytes` | memory cap | per-buffer fires (§9.5) |

### Appendix D — One-paragraph verdict per the audit's focus notes

The 2026-07-04 "DEAD STUB" characterization of security enforcement is **outdated and now incorrect**: the capability pipeline (derive→hash→union→load-gate) and the runtime `check_permission` backstop are live and empirically enforce for 7 permissions plus `Ffi`/`Deterministic`; scope narrowing and all four resource limits fire; the serve sandbox and distributed receiver-owned enforcement fail closed. What remains stub-like: 5 decorative permissions, the unwired determinism runtime, and unreachable signature verification. The permission-into-content-hash claim is fully verified (test-pinned). Resource-limit precision is tight for instructions/time/output and per-buffer-only for memory. Ed25519 signing is a correct primitive with no run-path consumer. The dominant *new* risk this audit surfaces is not "nothing is enforced" (false) but "the config surface fails open in three independent, easily-triggered ways, and one network-scope wildcard admits look-alike domains" — enforcement is real; the path to *configuring* it safely is not.

*End of audit 11 — Security, Capabilities & Sandboxing.*
