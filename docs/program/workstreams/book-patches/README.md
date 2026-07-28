# Book patches

Book edits produced by lanes that must not write to `shape-web`.

`shape-web` is a separate repository and is routinely dirty with unrelated
in-flight work, so a lane implementer prepares its Book edit here and the
supervisor applies it. Each patch is generated against a stated `shape-web`
revision and verified with `git apply --check` before being committed.

Apply from the `shape-web` root:

```sh
cd ../shape-web
git apply --check -p1 <patch>   # verify first
git apply -p1 <patch>
```

A patch that no longer applies means the Book moved underneath it. Re-derive
it rather than force-resolving: the surrounding prose is what makes the edit
truthful, and a fuzzy apply can land the claim next to contradicting text.

| Patch | Ticket | Applies to |
|---|---|---|
| `201-foreign-async-truthfulness.patch` | POLY-ASYNC-TRUTH (#201) | 4 pages documenting `async fn python` / `async fn typescript`, now a `[C0932]` compile error owned by #202 |
| `202-foreign-async-real.patch` | POLY-ASYNC-OFFLOAD (#202) | The REVERSAL of `201-…`: the same 4 pages, now documenting real off-thread async — futures at the call, `await` for the value, the per-language overlap model and its limits, and cancellation as discard-not-termination (generated against `cfa540e`) |
| `196-stub-channel.patch` | POLY-STUB-CHANNEL (#196) | `tooling/polyglot.mdx` + the two extension pages — the real marshaling table, the `[C0933]` declaration-site rejection, generated `.pyi` / `.d.ts` stubs, and per-declaration Python module namespacing (generated against `ca7cda8`) |
| `180-check-fix.patch` | ERGO-CONTRACT-FIXIT (#180) | `tooling/projects.mdx` — a new Checking-and-Applying-Fixes section: `shape check --fix`, the non-exhaustive-match fix as the one that ships today, the evidence-backed and revision-bound properties, and the three limits (emitter coverage, in-place rewrite, entry-file scope). The final example is `runnable` and was executed (`on` / `off`); the two intermediate states are `runnable=false` because one is a compile error and the other has an empty arm (generated against `627459a`) |
| `199-zero-copy-buffers.patch` | POLY-ZERO-COPY (#199) | `tooling/polyglot.mdx` — a new Sharing-Buffers-Without-Copying section (the `shared` / `shared mut` parameter spelling, why it is spelled rather than inferred, the three containments — call-scoped release check, CPython-enforced read-only, released-not-dangling — the shareable/not-shareable table with reasons, and the TypeScript and `async` refusals), plus a correction to Using NumPy, which said there was no zero-copy path. Both fences are `runnable=false`: they need the Python extension, like every other fence on the page (generated against `f158973`) |
| `198-declared-environments.patch` | POLY-ENV-PIN (#198) | `tooling/polyglot.mdx` — a new Declaring-the-Foreign-Environment section (`[foreign.<language>]`, the lockfile format, the environment digest and what does and does not move it, the deleted virtualenv search stated plainly as deleted, the `[C0936]` pre-entry failure, the checker pin, and TypeScript's locked module table with its three limits), plus two corrections: "any package installed in the active Python environment is available" is no longer true, and Content-Addressed Identity now says the environment digest is NOT yet in the hash. The one new fence is `runnable=false` — it needs the TypeScript extension, like every other fence on the page (generated against `ac0edde`) |
| `178-effect-rows.patch` | EFFECT-ROW-IN-TYPE (#178) | `advanced/security-permissions.mdx` — effect rows as a type component distinct from permissions, subset subsumption, `effect F` binders, and an explicit what-is-checked-today section covering the `[C0934]` declaration-position rejection owned by #143 (generated against `dd60b00`) |
