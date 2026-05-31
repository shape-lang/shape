# Fn-Boundary Let-Generalization — Gating-Predicate Spec + Soundness Verdict

Status: implementable spec / soundness verdict. Date: 2026-05-31.
Supersedes the §4 architect framing in `docs/design/v0.3.3-let-gen-grounding.md`
on the one point where a lens found a leak (see §3).

Grounding: `docs/design/v0.3.3-let-gen-grounding.md` (read first).
Verified against `main` HEAD `787b232d` (`./target/debug/shape`, devenv toolchain,
default `TypeDiagnosticMode::ReliableOnly`). All `file:line` cites verified.

---

## 0. Verdict up front

**`predicate_sound = true` — but ONLY because this spec tightens the predicate
beyond the architect's framing to refuse the value-restriction leak the
mutable-escape lens found.** The architect's stated condition ("the body never
materializes the return var") is *not* what the proposed fix computes, and the
gate the fix edits cannot distinguish a sound pure-`None`/immutable-`let` body
from an unsound body that returns a module-level **mutable** `var`. The latter
admits a static type confusion (int bound to `string`) caught only by a *runtime*
guard — a `CLAUDE.md` no-runtime-coercion / no-dynamic-fallback violation. §3
adds the refusal that closes it; §1 states the tightened predicate as the binding
rule.

Three lens findings folded in:
- **RUNTIME-LEAK**: sound, no leak. The runtime carrier is kind-erased in `T`,
  so a non-concretized generalized `T` never reaches a `NativeKind` stamp; every
  site that would demand a concrete inner kind rejects via an *independent*
  semantic check before any typed opcode emits. **Accepted.** (Caveat folded
  into §1.4: the independence is incidental, not the predicate.)
- **value-restriction / mutable-escape**: **leak (unsound)** for the
  param-less-fn-over-module-`var` subset. **Accepted; closed by §3 refusal.**
- **INFERENCE-CONSISTENCY**: imprecise, not unsound. Forward-ref scheme asymmetry
  (pre-existing, neither fixed nor worsened) + a grounding-doc mis-location of
  the class-2 reject seam (`ensure_no_unresolved_generic_args` is callee-body-only,
  has no binding/program caller — verified at `mod.rs:956/978/984/986/1128/1137/1157`).
  **Accepted; affects §4 (the bare-application policy is a *choice*, not a
  mechanical consequence).**

---

## 1. The EXACT predicate

### 1.1 Where it fires

The fn-boundary reject seam is two co-located checks in
`crates/shape-runtime/src/type_system/inference/items.rs`:

- **Seam 1 (early, runs first)**: `infer_callable_return_type(&func.body,
  func.return_type.is_some())` at `items.rs:357`. For an unannotated fn,
  `func.return_type.is_some() == false` ⇒ `allow_unresolved_generic_args = false`
  ⇒ the single `Option<fresh>` / `Result<fresh, …>` candidate is routed into
  `ensure_no_unresolved_generic_args` at
  `crates/shape-runtime/src/type_system/inference/mod.rs:1127-1128` (single-candidate
  arm of `combine_return_types_internal`). This is the seam that rejects
  `get_none` first.
- **Seam 2 (late, belt-and-suspenders)**: `items.rs:386-401`. Fires iff
  (a) `func.return_type.is_none()`, AND
  (b) `return_vars` contains a var not in `allowed_vars` (= type-param vars
  `∪` unannotated-param vars; `items.rs:390-394`), AND
  (c) `matches!(inferred_return_type, Type::Generic { .. })` (`items.rs:395`).
  Emits `TypeError::GenericTypeError "Could not infer generic return type for
  '<fn>'"`.

Type-shape facts that make these fire for the target cases:
- `None` ⇒ `Type::Generic { base: Option, args: [fresh] }` ⇒ matches (c).
- `Err(x)` ⇒ `Type::Generic { base: Result, args: [phantom_ok, …] }` ⇒ matches (c).
- `Identifier` read of a module-level binding instantiates that binding's
  `TypeScheme`. For a `var`/`let`/`let mut` binding, the scheme is **always
  `TypeScheme::mono`** (`items.rs:1218`); a mono scheme instantiates to
  `self.ty.clone()` *unchanged* (`types/core.rs:232-234`). So reading
  `var slot = None` (type `Option<freeT>`) inside a fn body yields
  `Option<freeT>` with the **same** `freeT` — which is `Type::Generic` and whose
  `freeT` is not in the reader fn's params ⇒ gate (a)+(b)+(c) all fire,
  *identically to `get_none`*.

### 1.2 The decidable predicate (TIGHTENED — binding rule)

When seam 1/2 would fire, **quantify instead of reject** iff **all** of:

1. **Unannotated fn**: `func.return_type.is_none()` (`items.rs:386`). [unchanged]
2. **Return-position-only free var**: every var in `return_vars` that is not in
   `allowed_vars` appears in the inferred return type and **not** in any param
   type (`items.rs:387-394` already computes exactly this set). The gate (b)
   condition is the decision variable; the fix flips its *consequence* from
   reject to quantify. [unchanged]
3. **Generic-headed carrier**: `matches!(inferred_return_type, Type::Generic {
   .. })` (`items.rs:395`) — i.e. the free var sits inside an `Option<_>` /
   `Result<_,_>` / generic-struct carrier, never bare. [unchanged]
4. **NON-EXPANSIVE PROVENANCE (NEW — the tightening §3 requires)**: none of the
   to-be-quantified vars is the element/payload var of a value that the body
   obtains by **reading a mutable binding or a reference/deref**. Decidable as a
   purely-syntactic body scan (no solver, no variance lattice needed):

   - Let `Q` = the set of vars the predicate is about to quantify (the gate-(b)
     residual set).
   - The fn body is **expansive w.r.t. `Q`** if any `return`-reachable expression
     whose inferred type carries a var in `Q` is one of:
     - `Expr::Identifier(name)` where `name` resolves (via `env.lookup`) to a
       binding whose `VarKind` is `Var` or `Let` **with `mutable == true`**
       (i.e. `var` / `let mut`), OR to **any module-scope binding** (a binding
       defined at item level, not a fn-local `let`); OR
     - `Expr::Reference { .. }` / a deref / a field/index read rooted in such a
       binding (`Expr::Reference { expr } => infer_expr(inner)` collapses to the
       inner type at `expressions.rs:1117`, and `Ref/RefMut(inner) =>
       inner.to_inference_type()` at `types/core.rs:476`, so the gate is
       otherwise blind to the mutable provenance — this scan restores it).
   - If the body is expansive w.r.t. `Q`, **DO NOT quantify — REJECT** (see §3).
   - If non-expansive (the value provably traces to a freshly-constructed literal
     `None` / `Err(..)` / `Ok(..)` / `Some(..)` / generic-struct literal, or to a
     fn-local immutable `let` chain bottoming out in such a literal), **quantify**.

   This is the *non-expansiveness* check that the grounding wrongly called "moot"
   (`grounding §"no value restriction"`, lines 85-91). It is the minimal,
   syntactic value-restriction needed; it does **not** require the full
   variance/covariance subsystem the grounding feared (`grounding §4` line 178)
   because generalization here is fn-boundary-only and the only unsound source is
   shared-mutable provenance, which is a one-step syntactic property of the
   returned expression.

> Conditions 1-3 are exactly the existing gate. Condition 4 is the new refusal
> clause. A var that satisfies 1-3 but **fails 4** is a **compile error**
> (`GenericTypeError`, message extended per §3) — never a runtime fallback.

### 1.3 What "body never materializes the var" reduces to (decidable)

The architect's prose "the body never materializes the return var" was
**unverifiable** (it is not a property of any value computed by the fix; the gate
inspects only the return *type shape*, never the body's value flow). This spec
replaces it with the decidable surrogate of §1.2 condition 4: *the returned value
must not be sourced from a mutable/shared binding or a reference into one.* Under
that surrogate:
- "materializes into a typed slot needing a concrete inner kind" is handled
  downstream by the independent per-site kind floors (§1.4) — not by this
  predicate. The predicate's job is solely to bar the **shared-mutable**
  provenance that makes the type-system result unsound (§3).

### 1.4 Why the RUNTIME-LEAK lens is sound (carrier kind-erasure)

Verified: `OptionData { is_some: bool, payload: KindedSlot }` and `ResultData {
is_ok: bool, payload: KindedSlot }` (`crates/shape-value/src/heap_value.rs:2246-2267,
2207-2227`). A `None` payload is `KindedSlot::none()` — Bool-kind, zero bits,
Drop no-op (`heap_value.rs:2259-2266`). The carrier `NativeKind` is
`Ptr(HeapKind::Option)` / `Ptr(HeapKind::Result)` with the inner `T`
**not parsed** into the discriminator. `Literal::None` compiles via the
None-headed carrier path, never a fabricated concrete inner kind. So a
generalize-then-instantiate-but-unconcretized `T` changes the **type-system**
result but **not** the emitted bytecode for a *pure* `None`/`Err` — the value is
identical and needs no `T`.

Defense-in-depth (run-verified, see §5 corpus): every site that *would* demand a
concrete inner kind rejects un-concretized `T` cleanly *before* any typed opcode:
- push into a typed array → empty-array seam SemanticError "type … not statically
  known" (`crates/shape-vm/src/compiler/v2_typed_emission.rs:865-881`);
- unwrap then arithmetic → binary-op strict check "operand types are unknown";
- no provable kind → surface-and-stop `NotImplemented(SURFACE)`.

**Caveat (folded into the predicate's safety argument):** this independence is
*incidental* to the predicate, not enforced by it. The predicate is BROADER than
"safe at runtime" — it is sound for *pure* carriers because the carrier is
kind-erased, and sound for everything else only because of these per-site floors.
`prove_native_kind` (`compiler/type_tracking.rs:1238-1244`) is a Phase-2
pass-through stub (`Ok(claimed_kind)`) and is **not** today's live enforcement —
the upstream semantic checks are. This is a pre-existing codebase fact orthogonal
to this fix; do not regress those floors while implementing this predicate.

---

## 2. The plumbing

### 2.1 `allow_unresolved_generic_args` flag path

`infer_callable_return_type(&func.body, func.return_type.is_some())`
(`items.rs:357`) → `statements.rs:71-126` (return-candidate collection) →
`combine_return_types_internal(candidates, allow_unresolved_generic_args)`
(`mod.rs:1025/1110/1113`). With the flag `false` (current unannotated path) the
single-candidate arm calls `ensure_no_unresolved_generic_args` at `mod.rs:1128`
(seam 1).

**Fix:** plumb a *quantify-this-fn* decision (computed from §1.2 conditions 1-4)
so that for a non-expansive unannotated fn the single-candidate arm takes the
`allow_unresolved_generic_args == true` branch (`mod.rs:1142-1153` — push
equality constraints among unique candidates, return the representative type with
its free var intact) **instead of** `ensure_no_unresolved_generic_args`. The free
return var then survives into `inferred_return_type`, and seam 2 (`items.rs:386-401`)
must be edited to quantify-not-reject for the same non-expansive case.

> Critical: do **not** make `allow_unresolved_generic_args` unconditionally `true`
> for all unannotated fns. That would also relax the *callee-body* check for
> expansive bodies (the §3 leak) and for nested generic-arg positions. The flag
> must be gated on §1.2 condition 4 (non-expansive) at this single fn-boundary
> call site only.

### 2.2 Where `make_function_scheme` keeps the free return vars

`make_function_scheme` (`items.rs:1107-1178`): for an untyped fn (`func.type_params
== None`) it falls through to `self.env.generalize(&func_type)` (`items.rs:1176`).
`env.generalize` (`environment/mod.rs:1250-1263`) computes
`quantified = free_vars(ty).difference(environment_type_vars())`. The surviving
free return var (from §2.1) is quantified here **iff it is not also free in the
environment** (`environment_type_vars`, `mod.rs:1300-1318`).

> **This is the load-bearing soundness hinge for §3.** For `fn get_none()` the
> return var `freeT` is a body-local fresh var, not in the environment ⇒
> quantified ⇒ `∀T. () -> Option<T>`. For `fn get_slot() { return slot }` where
> `var slot: Option<freeT>` is a **module-scope** binding, `freeT` *is* free in
> the environment (it is in `slot`'s scheme.ty, and slot's scheme is mono so
> `freeT` is not in `scheme.quantified` ⇒ `environment_type_vars` includes it,
> `mod.rs:1308-1312`). Therefore `generalize` would **exclude** `freeT` and the
> scheme would be `() -> Option<freeT>` with a *dangling, un-quantified free var*
> — neither a clean monomorphic type nor a proper `∀`-scheme. The §1.2-cond-4
> refusal must catch this case **before** `generalize`, because leaving a
> dangling free var in a fn scheme is itself a soundness hazard (each call site
> re-reads the same live module cell against a fresh-looking but actually-shared
> var; see §3). Verified: `instantiate` of a mono scheme is identity
> (`core.rs:232-234`); two call sites of `get_slot` therefore both alias the same
> live `slot` cell.

### 2.3 Predeclare / infer scheme consistency

`predeclare_function_signature` (`items.rs:38-65`) builds `∀a. () -> a` via
`make_function_scheme → env.generalize` (untyped fn, no `type_params` ⇒ else arm
at `items.rs:1176`) and `env.define`s it. Pass-2 `infer_item` (`items.rs:121-126`)
re-infers and **overwrites** via a second independent `make_function_scheme →
env.generalize` + `env.define`. The INFERENCE-CONSISTENCY lens verified: no
double-generalize / quantifier-arity blowup — `quantified` is rebuilt from
scratch each call (`mod.rs:1255`), and the self-scheme's own quantified var is
excluded from `environment_type_vars` (`mod.rs:1308-1312`). Two independent
`TypeScheme` objects; the second wins. **Accepted as consistent.**

Pre-existing imprecision (NOT introduced by this fix): pass-2 is source-order
(`mod.rs:1252`) with no post-solve env-scheme rewrite. A call textually *before*
the def instantiates the predeclare `∀a.()->a` (bare-var return); a call *after*
sees the inferred `∀b.()->Option<b>`. This forward-ref asymmetry exists today
(post-reject the env stays at predeclare `()->a` for all sites); the fix improves
backward refs without worsening forward refs. **Imprecise, not unsound — out of
scope for this fix; track separately.**

---

## 3. Lens-found unsoundness and the required refusal

### 3.1 The leak (value-restriction / mutable-escape lens — ACCEPTED)

A param-less fn returning a module-level **mutable** `var` reaches the *identical*
gate as `get_none()` and, under the architect's framing, would be generalized
identically. The gate (`items.rs:386-401`) inspects only type **shape**; it
cannot tell a vacuous-in-`T` body apart from one returning a shared mutable.
There is no `is_expansive` / `is_syntactic_value` / variance machinery anywhere in
`type_system/` (confirmed: `generalize` at `environment/mod.rs:1250` takes no
value-form parameter).

Run-verified on `main` HEAD `787b232d` (`./target/debug/shape`, ReliableOnly —
the GenericTypeError is filtered today; the strict-flip and this fix both reach
this gate):

- **T20** (`--mode vm`): `var slot = None; fn get_slot() { return slot };
  fn put_int(){ slot = Some(1) }; fn put_str(){ slot = Some("a") };
  put_int(); let x: int = get_slot() ?? 0; put_str(); let y: string =
  get_slot() ?? ""` — **COMPILES + RUNS**, prints `Some(1)` then `Some("a")`.
  Both `put_int` and `put_str` type-check against the *same* slot ⇒ slot's
  element var is genuinely unconstrained-free (a mono-int slot would reject
  `Some("a")`). int and string both written to / read from one shared cell.
- **T18**: `var slot = None; fn get_slot(){ return slot }; slot = Some(5);
  let r = get_slot(); match r { Some(s) => { let z: string = s;
  print(z.length) } None => {} }` — binds int `5` to `z: string`. The **static
  system was silent**; only a **runtime guard** caught it:
  `Error: Runtime error: TypeError: expected string, got int (line 6)`.
- **T17**: `... slot = Some(5); let b: string = get_slot()!; print(b.length)` —
  `b = Some(5)` bound to `string`; **runtime** `Error: ... heap value without
  length semantics (line 5)`. Static system silent.
- **T21 control** (immutable `let slot = None`): `get_slot()` returns pure `None`,
  reads give defaults `0` / `""` — **EXIT 0**. *This is the only case the
  grounding analyzed* (`grounding` lines 90-91, 176-179).

This is precisely a `CLAUDE.md` **no-runtime-coercion / no-dynamic-fallback**
violation: a generalized-`T`-derived value reinterpreted at a runtime type check
instead of being rejected at compile time. The static system must reject T17/T18/
T20 at **compile time**.

Aggravators (verified):
- `Expr::Reference { expr } => infer_expr(inner)` (`expressions.rs:1117`) and
  `Ref/RefMut(inner) => inner.to_inference_type()` (`types/core.rs:476`):
  references collapse to inner type, so a fn returning `&mut`/ref of polymorphic
  data is inferred as the bare var; the gate is blind to mutability.
- B0003 `ReferenceEscapeIntoModuleBinding` / B0005 `UseAfterMove`
  (`crates/shape-vm/src/mir/analysis.rs:155/157/240/246`) block **local**
  `let mut` capture-and-read, but do **not** block `return slot` of a
  module-level binding (T20's `return slot` runs) — the exact escape route the
  fix would otherwise generalize is left open.

### 3.2 The refusal the predicate MUST encode

**The predicate REFUSES the shape "an unannotated fn whose to-be-quantified
return var is the payload/element var of a value sourced from a mutable (`var` /
`let mut`) binding, a module-scope binding, or a reference/deref into one."**
Mechanically: §1.2 condition 4. A fn that fails condition 4 is a **compile error**
— extend the `GenericTypeError` message at `items.rs:397-400` to, e.g.:

> "Cannot infer a polymorphic return type for '<fn>': its result is read from the
> mutable/shared binding '<name>', whose element type is not fixed. Annotate the
> binding (`let <name>: Option<ConcreteT> = …`) or the function's return type
> (`fn <fn>() -> Option<ConcreteT>`)."

This keeps generalization restricted to vars whose returned value provably traces
to a freshly-constructed carrier (pure `None` / `Err` / `Ok` / `Some` / struct
literal) or a fn-local immutable `let` chain — i.e. it builds exactly the minimal
syntactic non-expansiveness check the grounding claimed was unnecessary. With
condition 4 in place, the fix is **sound** (T17/T18/T20 now reject at compile
time; T21 and `get_none` still quantify).

### 3.3 RUNTIME-LEAK precision caveat (no leak, but folded)

The RUNTIME-LEAK lens flagged that the architect's "body never materializes the
return var" is not the gate's computed condition and the fix is broader than the
stated predicate. §1.3 resolves this by replacing the unverifiable prose with the
decidable §1.2-cond-4 surrogate. Soundness for the *pure* subset survives via
carrier kind-erasure (§1.4); soundness for the *mutable* subset is now secured by
the §3.2 refusal rather than by hoping a downstream per-site floor fires.

---

## 4. The §5 bare-application sub-decision (for USER ratify)

**Restated:** Fn-boundary let-gen (with the §3 tightening) clears the class-(1)
fn-def residuals where a call site or annotation pins `T`. **One** residual
remains undecided: the bare-application `let x = get_none()` (test
`functions/stress_recursion.rs:393`) where nothing downstream constrains `T`.

**Recommended: Option A — annotation-required.** `let x: Option<T> = get_none()`,
or use the result at a site that pins `T`. Cheapest correct rule; keeps `let`
always-mono (`items.rs:1218`); mirrors the shipped empty-array remedy
(`let a: Array<T> = []`). Amend the one recursion test to pin `T` (it tests
recursion, not unconstrained-bare-call inference). **Do NOT** adopt Option B
(let-binding generalization) or Option C (relaxed value restriction) — both
presuppose wholesale let-generalization Shape does not have, out of proportion to
one test.

**Honesty caveat the user must weigh (INFERENCE-CONSISTENCY LEAK 2 — verified):**
the grounding (lines 104, 160) claimed `let x = get_none()` "re-trips
`ensure_no_unresolved_generic_args` at the binding/program level." **This is
false.** `ensure_no_unresolved_generic_args` has **no** binding/program caller —
it is invoked only inside `combine_return_types_internal` (callee-body) and its
own recursion (`mod.rs:956/978/984/986/1128/1137/1157`). `infer_variable_decl`
(`items.rs:1181-1224`) binds mono with no such check. Run-verified: `fn get_none()
{ return None }; let x = get_none()` **EXIT 0 today** (case A1, §5).

Consequence the user must ratify: **the fix as-described does not *mechanically*
enforce Option A.** Once the callee-body reject is removed (the fix), the bare
application `let x = get_none()` simply succeeds — instantiates a fresh unpinned
`T`, binds it mono into `let`, with no binding-level re-check. Option A is then a
*policy* requiring a **new** binding/program-level check (reject a `let` whose
final inferred type still carries an un-pinnable free generic arg, with the
annotation remedy) — it is not free. The user must choose:
- **A-enforced**: add the new binding-level reject (small, mirrors the
  empty-array seam) so `let x = get_none()` *errors* demanding an annotation.
  Bytecode side is unaffected: `inferred_type_to_hint_name(Option<free-var>)`
  returns `None` (`compiler/compiler_impl_reference_model.rs:1131/1135`).
- **A-relaxed (de-facto B-lite)**: accept that `let x = get_none()` compiles and
  the unpinned `T` is harmless because the value is a pure kind-erased `None`
  (§1.4) — no runtime hazard, but `let` now silently tolerates an un-pinned
  generic arg, eroding the "always-concrete `let`" contract.

**Recommendation stands at Option A-enforced** (cheapest *correct*; preserves the
concrete-`let` contract), but the user must ratify that it requires the small new
binding-level check, because the grounding mis-stated it as already-present.

---

## 5. Regression test corpus (must-cover)

All cases below run-verified on `main` HEAD `787b232d`. After the fix +
§3-tightening, the **ACCEPT** cases must compile (and remain EXIT 0); the
**REJECT** cases must become **compile errors** (today they are filtered/runtime
errors — proving no generalized-`T` leaks to runtime). Add as unit tests
(`#[cfg(test)]`), not standalone files.

### 5.1 ACCEPT — the 5 class-(1) fn-def cases that should now COMPILE

| # | Shape | Expectation | Cite |
|---|-------|-------------|------|
| A1 | `fn get_none() { return None }` + `let x = get_none()` | compile + EXIT 0 (`ok`) — bare-app; §4 user decision governs whether this *errors* under A-enforced | grounding §"#1"; run-verified EXIT 0 |
| A2 | `fn get_val() { return None }` + `let v = get_val() ?? 42` | compile + EXIT 0, prints `42`; `?? 42` pins inner=int | grounding §"#2"; run-verified |
| A3 | `fn step1() { return Err("boom") }` + `let y = step1()` | compile + EXIT 0 (`ok`) — Result carrier, kind-erased | grounding §"#1/#6"; run-verified |
| A4 | `fn find_user() { None }` consumed via `(find_user() !! "…")?` in a `-> Result<number>` fn | compile + EXIT 0; call site pins `T` | grounding edge_cases.rs:161-177 |
| A5 | `fn get_opt() { None }` via `!!?` + `Ok(v + 5)` in `-> Result<number>` | compile + EXIT 0; pinned by `Ok(v+5)` | grounding propagation.rs:413-430 |

Plus two predicate-mechanism checks:
- **A_rec** (no-op): `fn rec(n) { if n <= 0 { return None } return rec(n-1) }` +
  `let r = rec(3)` — EXIT 0. Mixed None/recursive return takes the union branch
  (`statements.rs:100-118`), yielding a bare var **not** `Type::Generic` ⇒ gate
  (c) false ⇒ fix's path not taken. Run-verified EXIT 0. (Guards against
  polymorphic-recursion regression.)
- **A_pure_local** (§1.2-cond-4 non-expansive accept): `fn get_none() { let inner
  = None; return inner }` where `inner` is a fn-local **immutable** `let` bottoming
  out in a literal `None` — must quantify (compile + EXIT 0), proving cond-4
  permits fn-local immutable chains.

> NOTE: do **not** include a "generic-struct return" accept case modeled on the
> INFERENCE-CONSISTENCY lens's r5. Verified counter-result: both
> `type Cell { value: Option<int> }` and `type Cell<T> { value: Option<T> }` with
> `fn make() { return Cell { value: None } }` currently **fail** with a
> bytecode-compiler semantic error (`cannot construct field 'value' … with
> Option<any> literal`), not EXIT 0. The r5 EXIT-0 claim is **not reproducible**;
> exclude it from the corpus.

### 5.2 MUST-REJECT — no generalized-`T` leaks to runtime (the §3 leak repros)

These must become **compile errors** under the fix + §3 tightening. Today
(filtered ReliableOnly) they compile and fail only at *runtime* — that runtime
backstop is exactly what the predicate must replace with a compile-time reject.

| # | Shape | Today (main, ReliableOnly) | Required after fix |
|---|-------|----------------------------|--------------------|
| R1 (=T17) | `var slot = None; fn get_slot(){ return slot }; slot = Some(5); let b: string = get_slot()!; print(b.length)` | RUNTIME `TypeError … heap value without length semantics (line 5)` | **COMPILE ERROR** (§3.2 message): `get_slot` reads mutable `slot` ⇒ cond-4 fails ⇒ reject |
| R2 (=T18) | `var slot = None; fn get_slot(){ return slot }; slot = Some(5); let r = get_slot(); match r { Some(s) => { let z: string = s; print(z.length) } None => {} }` | RUNTIME `TypeError: expected string, got int (line 6)` | **COMPILE ERROR** (same) |
| R3 (=T20) | `var slot = None; fn get_slot(){ return slot }; fn put_int(){ slot = Some(1) }; fn put_str(){ slot = Some("a") }; put_int(); let x: int = get_slot() ?? 0; put_str(); let y: string = get_slot() ?? ""` | COMPILES + RUNS (prints `Some(1)`, `Some("a")`) | **COMPILE ERROR** (same) — one shared cell typed both int and string is the unsoundness |
| R4 (ref) | `var slot = Some(0); fn get_ref(){ return &slot }` consumed at conflicting types | gate blind via `Reference => infer_expr(inner)` (`expressions.rs:1117`) | **COMPILE ERROR** — cond-4 treats reference-into-mutable as expansive |

Control (must STILL ACCEPT, proving the refusal is not over-broad):
- **R-ctl (=T21)**: `let slot = None; fn get_slot(){ return slot }; let x: int =
  get_slot() ?? 0; let y: string = get_slot() ?? ""` — immutable `let slot`;
  EXIT 0 today, must stay EXIT 0 (pure None, non-expansive ⇒ quantify). Confirms
  cond-4 distinguishes immutable-`let` (accept) from `var`/`let mut`/module-`var`
  (reject).

### 5.3 Empty-array boundary (orthogonal, must remain rejected on its own seam)

- `let a = []; a.length` — bytecode-compiler semantic error
  (`v2_typed_emission.rs:870-929`), Concrete-Array not `Type::Generic`, never
  reaches the fn-seam. Remedy unchanged: `let a: Array<T> = []`. Include one such
  case to assert the fix does **not** alter the empty-array seam.

---

## 6. CLAUDE.md compliance statement

- **No runtime coercion / no dynamic fallback**: a generalized var that cannot be
  concretized is a **compile error** (seam 1 `ensure_no_unresolved_generic_args`
  for expansive bodies via §3.2; seam 2 `GenericTypeError` retained for cond-4
  failures). The fix never emits a coercion opcode and never relies on the
  runtime `TypeError` guard (R1/R2) as the type-soundness mechanism — that guard
  is demoted from "the only check" to "unreachable, because the compile error
  fires first."
- **No new modal-types subsystem**: §1.2 condition 4 is a syntactic body scan
  reusing existing `env.lookup` / `VarKind` / `Expr::Reference` shapes, not a new
  variance lattice.
- **`prove_native_kind` stub untouched**: the fix adds no fabricated kind; it
  bars un-concretizable `T` upstream of any `NativeKind` stamp (§1.4).
