# E5 CLOSE report — typed comptime type-reconstruction complete, `.source` fallback gone

**Branch:** `adr009/e5`. E5 turned the `__ComptimeTypeRef` type carrier from a
string-`.source`-reparse surface into a producer-STAMPED, consumer-IDENTITY-ONLY
surface, then DELETED the reparse fallback. The stamped identity route is now the
SOLE resolution path; an unstamped/unresolvable ref is a NAMED surface-and-stop;
the stamped->reparse walk-back (the Forbidden-Patterns dynamic-reparse shape) is
structurally impossible.

## Checkpoints

| CKPT | Landed | What |
|------|--------|------|
| CKPT-1 | `e57a8acd` | applied-generic + bare-nominal SPELLING reconstruction (design §1a); stamp-gate auto-widen (`stamp_for = reconstruct(...).is_ok()`) |
| CKPT-2 | `1d54eb67` | applied-generic DESCRIPTOR substitution (A8-OUT) |
| CKPT-3 | `f5b46958` | record field-name preservation + record spelling |
| CKPT-4 | `f5c51332` | migrate A/B/D producers; rule INVALID identity LOUD (design class C/A5); class-E (bare-string arm) BLOCKED/SURFACED → #88 |
| CKPT-5 | *(this)* | **DELETE the `.source` reparse fallback** (field + emit + arm); PRESERVE the item_fn parser (#88); walk-back now structurally impossible |

## End state — invariants

- **Every reconstructable type STAMPS.** Leaves (primitive synonym families),
  applied generics (`Array<int>`, `Option<T>`, `HashMap<K,V>`, `Result<T,E>`,
  applied user structs/enums), tuples, references, callables, records (field-name
  preserved), and bare user nominals all reconstruct off the shared frozen memo via
  `reconstruct_type_annotation` — the one total inverse of the freeze composite
  algebra. The producer stamp-gate admits an identity iff that inverse is `Ok`, so
  producer and consumer share ONE code path (E1-D7(b)).
- **Unstamped/unresolvable ⇒ NAMED surface-and-stop.** Unresolved returns
  (`kind:"Unresolved"`), synthetic members, scoped generic parameters, un-applied
  generic heads carry INVALID `{-1,-1}` and the consumer rejects LOUD. There is NO
  `.source` field and NO reparse arm — the fallback is DELETED.
- **`__ComptimeTypeRef` fields:** `name`, `kind` (spell/reflect-only, derived from
  the type spelling at build time — the U02 corpus reads `type_ref.kind`),
  `identity_high`, `identity_low` (the producer-stamped `FrozenTypeIdentity`). No
  `.source`.

## What CKPT-5 deleted vs preserved

- **DELETED:** the `.source` schema field (`builtin_schemas.rs`), its producer emit
  (`comptime_target.rs build_type_ref_descriptor`), and the consumer reparse arm
  (`comptime_builtins.rs`). Grep-proven 0 hits.
- **PRESERVED (#88):** `parse_type_annotation_payload` + `fn __type_probe` + the
  bare-string type-payload arm — the sanctioned `item_fn`/`extend_method` carrier
  (`return_type: string | TypeRef`), which has no non-parse path from a spelling
  string to an AST. Pinned by the pin-(g) over-deletion tripwire (parses `"int"`
  AND `"Array<int>"`).

## Absence enforcement

`no_json_comptime_protocol.rs` grows two needles — `no_source_field_on_comptime_
type_ref_schema` (the `.string_field("source")` field) and
`no_reparse_from_type_ref_source_field` (the `&schema, "source")` read) — both 0
across `crates/ bin/ tools/ extensions/`. Precise: they do NOT ban the preserved
item_fn parser. The route-proof pins are rewritten so a re-introduced arm makes
them fail (`e1_s5_stamped_unresolvable_ref_errs_through_full_consumer_…`).

## Gates (post-CKPT-5)

`e1_s5` 31/0 · `no_json` 4/0 · `comptime` 271/3 (3 named pre-existing) ·
`annotations_comptime` 117/10 · `just check-clean` exit 0 ·
`just check-no-dynamic` success · `just verify-merge` 15/0. `shape-vm --lib`
comptime blast radius fully green; the lone route_tests `nested_exact_calls` delta
is a documented pre-existing non-deterministic flap (see the CKPT-5 report).

## Residuals (out of E5, tracked)

- **#87** — as filed; unchanged by CKPT-5.
- **#88** — item_fn/extend_method typed-carrier migration. Until it lands, the
  bare-string arm + `parse_type_annotation_payload`/`__type_probe` remain the
  sanctioned carrier. Closing #88 is the prerequisite for ruling the bare-string
  arm LOUD (design class E) with an additive migration.

## Verdict

E5 COMPLETE. All type reconstruction landed; the `.source` reparse fallback is
gone; the walk-back is structurally impossible.
