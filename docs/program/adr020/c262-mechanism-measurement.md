# #262 — the mechanism, measured

**Status:** measurement complete, nothing designed and nothing wired. Branch
`c-real-inference`, based on `origin/main@f99dc36c`.

**Headline:** the ticket's hypothesised mechanism is **refuted**. There is not
one mechanism — there are at least **three**, and the largest is not a drop at
all. Three of the four measured cases do not match the hypothesis in any part.

The ticket recorded, explicitly as unmeasured: *"unannotated closure parameters,
passed to a generic method inside a generic `extend` body, are left free and
dropped by `finalize_expr_type_table`."* That describes at most one of the four.

---

## 1. Method

The decisive question was whether a residual span is **never recorded** by the
inference walk, or **recorded and then dropped** as still-free. Those imply
completely different fixes, so both sides were dumped rather than one inferred
from the other:

- `finalize_expr_type_table` (`inference/mod.rs:471`) instrumented to print every
  KEPT and DROPPED entry with its span, pre-substitution and post-substitution
  type.
- `infer_extend_method_bodies` (`inference/extend_methods.rs:172`) instrumented
  to print its `should_infer_body` gate and each disjunct.

Both gated on `C262_TRACE`, both reverted. The **exact** ticket sources were
used, never approximations — per the warning carried from #240, where four
approximated shapes all came back clean and read as "no compiler change needed".

Command:

```
C262_TRACE=1 direnv exec /home/dev/dev/shape-lang cargo test -p shape-vm --lib --jobs 4 \
  -- --nocapture <probe>
```

---

## 2. What was measured

### Case 5 — `extend Vec<number>`, span 96..101 (`a + b`). **Never recorded.**

```
C262 EXTEND type_name="Vec" should_infer_body=false (struct=false bare_collection=false scalar=false)
```

`infer_extend_method_bodies` **early-returns**, so the method body is never
walked and no span inside it is ever recorded. The span table for this program
holds only spans 139..156 — the trailing `[1, 2, 3].sum_all()`.

This is a **coverage gate**, not a free-variable drop. The gate
(`extend_methods.rs:172-176`) admits a body only when the receiver is a known
struct, a *bare* single-param collection, or a scalar. `Vec<number>` — a
collection with a CONCRETE type argument — matches none of them.
`bare_single_param_collection_extend` recognises the bare/parametric form only.

### Case 3 — `extend Number { method tripled() { self * 3.0 } }`, span 166..177. **Recorded, dropped.**

```
C262 EXTEND type_name="Number"            should_infer_body=true  (scalar=true)
C262 EXTEND type_name="generated::Number" should_infer_body=false (all false)
C262 DROPPED span=166..177 pre=Variable(TypeVar("N")) post=Variable(TypeVar("N"))
```

The body IS walked and `self * 3.0` IS recorded — then dropped because `self`'s
receiver type variable `N` never binds. Not a closure, and no closure parameter
involved.

**Second finding in the same trace:** the module-qualified `generated::Number`
extend gets `should_infer_body=false` while the bare `Number` gets `true`. The
gate is defeated by a module-qualified receiver name — a distinct instance of the
Case-5 coverage gate.

### Case 2 — nested returned closures, span 82..120. **Recorded, dropped, partially resolved.**

```
C262 DROPPED span=82..120
  pre =Function { params: [T10], returns: T9 }
  post=Function { params: [T5],  returns: Concrete(Basic("int")) }
```

The closure's RETURN resolves to `int`; its PARAMETER stays free. The entry is
dropped because `type_is_fully_resolved` requires the whole structure. **No
`extend` anywhere in this source.** This is the only case that resembles the
hypothesis, and it resembles the "closure parameter stays free" half only.

### Case 1 — `extend User { method is_active() }`. **The extend body is fine; the drop is elsewhere.**

```
C262 EXTEND type_name="User" should_infer_body=true (struct=true)
C262 KEPT span=203..249 ty=Concrete(Basic("bool"))       <- the extend body, resolved
C262 DROPPED span=318..356 pre=Variable(TypeVar("T12")) post=Variable(TypeVar("T6"))
```

The `extend` method body resolves completely and is kept. Every dropped span
(318..356) is inside `fn count_active(users) { users.filter(|u| u.is_active()).length }`
— an ordinary function with an **unannotated parameter**, so `u` cannot bind.
Nothing to do with `extend`.

---

## 3. The refutation, stated precisely

| Case | Ticket's hypothesis holds? | Actual mechanism |
|---|---|---|
| 5 | **No** — not a drop at all | coverage gate; body never walked |
| 3 | **No** — no closure, no closure param | unresolved receiver type var `N`; plus a second gate miss on module-qualified names |
| 1 | **No** — `extend` body resolves fine | unannotated ordinary-fn parameter |
| 2 | **Partly** | closure param free while return resolves; no `extend` involved |

The framing "closure parameters inside generic `extend` bodies" is wrong on both
halves for most cases: two of four have no closure parameter in the failing
expression, and two of four have no `extend` in the failing path.

**What actually unifies them** is much weaker and much more useful: *an
expression's type is unavailable to the descriptor classifier*, for three
unrelated upstream reasons. They should be sized and fixed separately, because
a single fix cannot address a coverage gate and a constraint-binding gap at once.

---

## 4. Consequences for the ticket

- **Acceptance criterion "the mechanism is measured before the fix" is met**, and
  the answer is that the ticket describes a mechanism that mostly is not there.
  #262 as written scopes one root; the evidence shows three.
- **The `Queryable<T>` question is answerable in the affirmative for Case 5 and
  the `generated::Number` gate miss, and only those.** Both are receiver-type
  recognition failing on a parameterised or qualified name — the same family as
  "a generic `impl` parses but type inference erases the type arguments back to
  simple names". Cases 1 and 2 are unrelated to it.
- **Suggested split.** Sub-root A (coverage gate: concrete-arg collection
  receivers, module-qualified receivers) is self-contained, is the one with a
  clear `Queryable<T>` relationship, and covers the ticket's own reproducer.
  Sub-roots B and C are constraint-binding questions in the ordinary inference
  path with no `extend` involvement, and belong with whatever ticket owns
  unannotated-parameter inference.

No fix is proposed here. The measurement was the chartered deliverable and the
result changes the ticket's shape enough that scoping should be re-decided
before any design.

---

## 5. Note on residual identity

Per the check added to this ticket: when these close, verify each residual
**disappears** rather than **moves**. Case 3's trace already shows why — it
carries two independent problems (the dropped `self * 3.0` and the
module-qualified gate miss) in one source, so fixing either alone will leave a
residual at a different span and the count will still fall by one.
