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
| `196-stub-channel.patch` | POLY-STUB-CHANNEL (#196) | `tooling/polyglot.mdx` + the two extension pages — the real marshaling table, the `[C0933]` declaration-site rejection, generated `.pyi` / `.d.ts` stubs, and per-declaration Python module namespacing (generated against `ca7cda8`) |
