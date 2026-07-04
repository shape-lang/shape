# SMOKE s4 / s5 — strict-flip regressions (READ-ONLY diagnosis)

Two canonical smoke fixtures (`tests/smokes/s4.shape`, `s5.shape`) PASS on `main`
(705cd854) and FAIL on the strict-flip branch (`a1c16d9c`). Both block smoke 5/5.

**Shared cause:** the strict-flip branch flips the default type-diagnostic mode
from `ReliableOnly` to `Strict`
(`crates/shape-vm/src/compiler/compiler_impl_initialization.rs:125`:
`type_diagnostic_mode: TypeDiagnosticMode::Strict`). Neither fixture's checker
gap is *new* on the branch — both gaps exist on `main` too, but `main`'s
`ReliableOnly` mode swallowed the type errors and let the bytecode compiler /
VM (which DO support both constructs) run. Strict mode now surfaces the
pre-existing checker holes as hard errors. Both are **FP** (the checker
over-rejects code that is valid under the ratified strict rules and runs
correctly at runtime). VM and JIT agree on the error on strict-flip (JIT
re-reports the same SEMANTIC error from bytecode compilation, no fallback).

---

## s4 — `Set()` constructor not known to the type-checker

### Fixture
```shape
let mut s = Set()
s.add("a")
s.add("b")
print(s.len())
```

### Observed
- main vm/jit: `2` (ec=0)
- strict-flip vm: `error[SEMANTIC]: Undefined function: 'Set'  --> <input>:1:1` (ec=1)
- strict-flip jit: same SEMANTIC error via "Bytecode compilation failed" (ec=1)

### Verdict: **FP**

`Set` is a genuine builtin constructor. The bytecode compiler maps `"Set" =>
BuiltinFunction::SetCtor`
(`crates/shape-vm/src/compiler/helpers.rs:4382`), the VM executes it
(`crates/shape-vm/src/executor/vm_impl/builtins.rs:680`), producing a
`HeapKind::HashSet`, and `.add` / `.len` are real PHF methods
(`crates/shape-vm/src/executor/objects/method_registry.rs:492` `SET_METHODS` —
`add`, `len`, `length`, `has`, `includes`, `delete`, `isEmpty`, `toArray`,
`union`, `intersection`, `difference`, `forEach`, `map`, `filter`). The code is
valid and ran correctly on main.

### Root cause + file:line

1. **Constructor not registered in the type env.**
   `crates/shape-runtime/src/type_system/environment/mod.rs:1006-1011`
   (`define_builtin_functions`) registers only the `HashMap` collection
   constructor (plus `Some`/`Ok`/`Err`). `Set`, `Deque`, `PriorityQueue`,
   `Channel`, `Mutex`, `Atomic`, `Lazy` are all absent — every one of them is a
   real `BuiltinFunction::*Ctor` (`helpers.rs:4382-4388`) but is invisible to
   `Env::lookup`. `infer_function_call`
   (`crates/shape-runtime/src/type_system/inference/access.rs:593-596`) does
   `self.env.lookup(name).ok_or_else(|| TypeError::UndefinedFunction(...))` →
   the `Set` call fails here. This is the surfaced error.

2. **Method-table has no `Set` entry (latent second error).** Even after the
   constructor resolves, `s.add(...)` / `s.len()` go through method-call
   inference. If `Set()` is registered as a concrete
   `Type::Concrete(TypeAnnotation::Reference("Set"))` (the HashMap pattern),
   then the method-not-found fallback in
   `crates/shape-runtime/src/type_system/inference/expressions.rs:721-739`
   pushes a `HasMethod` constraint, and the solver
   (`crates/shape-runtime/src/type_system/constraints.rs:1140-1149`) does
   `method_table.lookup(Set, "add")`. The method-table only seeds `HashMap`,
   `Vec`, `string` (`crates/shape-runtime/src/type_system/checking/method_table.rs`
   — `register_user_generic_method("HashMap"...)` / `("Vec"...)` /
   `register_user_method("string"...)`; no `Set`). So `.add` would then fail
   with `MethodNotFound { type_name: "Set", method_name: "add" }`. A complete
   fix must seed `Set` methods too.

### Minimal fix (two seams)

**Seam A — register the constructor.** In `define_builtin_functions`
(`environment/mod.rs`, right after the `HashMap` block at line 1006-1011) add a
polymorphic `Set<T>` constructor so the element type stays inferred:

```rust
// Set constructor: Set() -> Set<T>
{
    let set_t = TypeVar::new("T".to_string());
    let set_inner = Type::Variable(set_t.clone());
    let set_result = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Reference("Set".into()))),
        args: vec![set_inner],
    };
    self.define_polymorphic("Set", vec![set_t], vec![], set_result);
}
```

(A monomorphic `self.define_builtin("Set", vec![], Type::Concrete(
TypeAnnotation::Reference("Set".into())))` — exact HashMap parallel — also
clears s4; the polymorphic form is preferred so `Set<string>` element typing is
preserved for downstream `.includes`/`.toArray` element inference.)

**Seam B — seed `Set` methods in the method-table.** In
`crates/shape-runtime/src/type_system/checking/method_table.rs`, alongside the
`HashMap`/`Vec` blocks, register the `Set` methods mirroring `SET_METHODS`
(receiver param 0 = element `T`):
`add(T)->Self`, `delete(T)->Self`, `has(T)->bool`, `includes(T)->bool`,
`len()->int`, `length()->int`, `isEmpty()->bool`, `toArray()->Vec<T>`,
`union(Self)->Self`, `intersection(Self)->Self`, `difference(Self)->Self`,
`forEach((T)->void)->void`, `map<U>((T)->U)->Set<U>`,
`filter((T)->bool)->Self`. Use `register_user_generic_method("Set", ...)`.
(`add`/`delete` are `&mut self` mutators at runtime; return `SelfType` for the
checker — `s4` discards the result either way.)

### Clears
- s4 directly.
- The whole **non-HashMap collection/concurrency constructor** FP class under
  strict mode (`Deque`, `PriorityQueue`, `Channel`, `Mutex`, `Atomic`, `Lazy`)
  — same Seam-A gap. Registering each ctor (and, for the collection ones,
  seeding its method-table block) clears any smoke/corpus program using them.
  Scope to `Set` for the s4 close-gate; flag the siblings as the same root.

### Not a TP
`Set()` is in the ratified builtin surface (`SetCtor` opcode, `SET_METHODS`
PHF, `set.shape` stdlib module). No ratified rule (no-truthiness,
generic-args-required, numeric-conversion, let-gen) forbids it. The fixture
should NOT change.

---

## s5 — concrete type → `dyn Trait` coercion rejected

### Fixture
```shape
trait HasX { method x_str() -> string; }
type Bar { v: int }
impl HasX for Bar { method x_str() -> string { "x" } }
let arr: Array<dyn HasX> = [Bar { v: 1 }, Bar { v: 2 }]
print(arr[0].x_str())
```

### Observed
- main vm: `x` (ec=0). main jit: `x` (ec=0, with a benign `[jit-fallback]`
  notice — falls through to interpreter, runtime surface agrees).
- strict-flip vm/jit:
  `error[SEMANTIC]: Could not solve type constraints:`
  `  Bar is not compatible with dyn HasX` (×2, one per array element) (ec=1)

### Verdict: **FP**

`Bar` implements `HasX` (`impl HasX for Bar`), so coercing `Bar` into
`dyn HasX` is the standard trait-object upcast — valid, and it runs correctly on
main (prints `x`). The two error lines are the two `[Bar { v: 1 }, Bar { v: 2 }]`
elements each failing to unify with the declared element type `dyn HasX`.

### Root cause + file:line

`crates/shape-runtime/src/type_system/constraints.rs` —
`ConstraintSolver::unify_annotations` (`fn` at line 534). The annotated decl
`let arr: Array<dyn HasX> = [...]` decomposes the element constraint to
`Bar ~ dyn HasX`. `unify_annotations` has a `(Dyn, Dyn)` arm (line 668-671) but
**no arm for `(concrete, Dyn)`** — a named type unifying into a trait object.
So `(Reference("Bar")/Basic("Bar"), Dyn([HasX]))` falls through to the final
`_ => Ok(false)` (line 701) → unification fails → unsolved-constraint error
rendered by `format_unsolved_constraints`
(`crates/shape-runtime/src/type_system/errors.rs:160-175`,
`format_annotation` Dyn arm at `:258`).

This arm has **never existed** (confirmed via `git diff 705cd854 a1c16d9c --
constraints.rs`: the strict-flip diff touched numeric-domain + degenerate-union
+ struct-schema unification, NOT the Dyn handling). So this is a latent main
gap, not a strict-flip regression — strict-flip only unmasks it. The solver
already carries the trait-impl registry needed to decide it:
`self.has_trait_impl(trait_name, type_name)` (line 1185) over
`self.trait_impls` (populated from `env.trait_impl_keys()` →
`"TraitName::TypeName"` keys, e.g. `"HasX::Bar"`; set via
`set_trait_impls` at `inference/mod.rs:1347,1413`).

### Minimal fix (one seam)

In `unify_annotations`, add a `(concrete, Dyn)` arm **before** the
`_ => Ok(false)` fall-through (insert near line 671, after the `(Dyn, Dyn)`
arm). A named type unifies into a trait object iff it implements every trait in
the dyn set:

```rust
// Concrete nominal type coerces into a trait object iff it implements
// every trait in the dyn set (standard trait-object upcast).
(TypeAnnotation::Basic(name), TypeAnnotation::Dyn(traits))
| (TypeAnnotation::Dyn(traits), TypeAnnotation::Basic(name)) => {
    Ok(traits.iter().all(|t| self.has_trait_impl(t.as_str(), name)))
}
(TypeAnnotation::Reference(path), TypeAnnotation::Dyn(traits))
| (TypeAnnotation::Dyn(traits), TypeAnnotation::Reference(path)) => {
    Ok(traits.iter().all(|t| self.has_trait_impl(t.as_str(), path.as_str())))
}
```

Notes:
- `Dyn(Vec<TypePath>)` (`crates/shape-ast/src/ast/types.rs:46`); `TypePath`
  has `.as_str()` (`crates/shape-ast/src/ast/type_path.rs:79`).
- Keep both `Basic` and `Reference` arms — struct/enum literals may infer as
  either; the rendered `Bar` is ambiguous between the two (`format_annotation`
  treats them identically). The `Object({...})` shape is NOT what flows here
  (the error renders `Bar`, a name, not `{ v: int }`).
- `has_trait_impl` already covers numeric aliases / widening, harmless here.
- This is sound: it only succeeds when the impl is actually registered. A type
  that does NOT implement the trait still fails to unify → still correctly
  rejected.

### Clears
- s5 directly.
- The general **concrete-into-`dyn Trait` coercion** FP class (any
  `let x: dyn T = concrete`, `Array<dyn T>` element coercion, fn arg/return of
  `dyn T` fed a concrete impl).

### Not a TP
The coercion is valid (impl exists, runs on main). No ratified strict rule
forbids trait-object upcast of an implementing type. The fixture should NOT
change.

---

## Summary

| Fixture | Verdict | Root (file:line) | Seam |
|---------|---------|------------------|------|
| s4 | FP | `Set` ctor absent from `define_builtin_functions` (`environment/mod.rs:1006-1011`) + no `Set` method-table seed (`checking/method_table.rs`); surfaces at `inference/access.rs:593-596` | register `Set` ctor (env) + seed `Set` methods (method_table) |
| s5 | FP | no `(concrete, Dyn)` arm in `unify_annotations` (`constraints.rs:534`, fall-through `:701`) | add `(Basic/Reference, Dyn)` arm using `has_trait_impl` (`:1185`) |

Both surfaced (not introduced) by the `ReliableOnly → Strict` default flip
(`compiler_impl_initialization.rs:125`). Both are valid code that runs on main;
fix the checker, do not rebaseline the fixtures.
