# Adversarial verifier finding — type-erasure class NOT cleared at root (match/pattern context)

Date: 2026-06-22. Branch strict-flip-collection-dispatch @ HEAD (rebuilt release).
Verifier: READ-ONLY (no source edits).

## Verdict: sound=false

The recurring type-erasure / heap-pointer-reinterpret class is GENERALLY cleared
for the probed dispatch-into-next-op contexts (map/get/filter/pop/index/chain/
reduce/method-into-cmp/pass-to-fn, T2 dot/norm/cumsum, T3 struct + f"{true as int}"==1,
T4 named-args all forms) — all byte-identical VM==JIT, correct values or clean
surface-and-stop (HashMap.keys/values = NotImplemented SURFACE, acceptable).

BUT a NEW dispatch context still erases type and reinterprets a heap pointer as i64:
matching an Option pattern (`Some`/`None`) against a `Result` scrutinee is NOT
type-checked, structurally matches by discriminant-slot collision, binds the payload
slot regardless of its real type, then arithmetic on that binding reads raw bits.

## Minimal repro (no array, no cast)

    let v: Result<int, string> = Ok(42)
    match v {
      Some(n) => print(n + 1),   // Some structurally matches Result::Ok
      None => print(-1),
    }

Expected: COMPILE ERROR (Option pattern vs Result scrutinee — pattern/type mismatch).
Actual:   rc=0, prints a garbage, ASLR-non-deterministic integer.
          VM=98133117351537  JIT=101999788130881  (DIFFER across modes AND across runs)
          => raw heap-pointer bits read as i64, fed into `n + 1`.

Second repro — payload is a string heap pointer (clearer reinterpret):

    let v: Result<int, string> = Err("boom")
    match v { Some(n) => print(n + 1), None => print(-1) }
    // VM=105640781291665 JIT=106342759028577 — "boom" string ptr + 1.

Reverse direction (Option matched with Ok/Err) does NOT reinterpret — it fails to
match (rc=1 "No match arm"); still wrong (should be compile error) but not a soundness
heap-reinterpret.

## Root: pattern type-checking gap
`Some`/`None` patterns are matched against the scrutinee purely structurally (variant
arity / discriminant slot) without verifying the scrutinee enum is actually Option<T>.
Fix belongs at pattern type-checking (reject Option patterns on non-Option scrutinee),
which would also fix the binding-slot/type-erasure at the root for this context.
