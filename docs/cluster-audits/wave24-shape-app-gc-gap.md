# Wave 24 Shape App GC Gap Check

Date: 2026-07-09

Scope: verify the sibling `../shape-app` playground/server embedding path that
was outside this repo's GC-default build graph.

## Finding

The app does embed `shape-vm`, and the server dependency now enables GC
explicitly:

- `../shape-app/shape-server/Cargo.toml` depends on
  `shape-vm = { version = "=0.3.2", path = "../../shape/crates/shape-vm",
  default-features = false, features = ["gc"] }`.
- `rg` found the VM embedding path in `shape-server/src/routes/stdlib_cache.rs`
  via `shape_vm::BytecodeExecutor`.
- No other non-lockfile `shape-vm` dependency was found under `../shape-app`.

The workspace root also points Shape crates at the sibling source checkout:

- `shape-wire = { path = "../shape/crates/shape-wire", default-features = false }`
- `shape-runtime = { path = "../shape/crates/shape-runtime", default-features = false }`
- `shape-lsp = { path = "../shape/tools/shape-lsp", default-features = false }`

These `../shape-app` files were already dirty before this note was written; this
audit did not modify them.

## Status

Source configuration closes the original concern that `shape-app` pinned
`shape-vm` with `default-features = false` and failed to opt into GC. The server
now opts in with `features = ["gc"]`, so the playground/notebook VM path should
receive the same GC safety story as the shipped binary.

## Verification

Static checks only:

- `rg -n "shape_vm|shape-vm|BytecodeExecutor|VirtualMachine|gc" ../shape-app --glob '!target/**' --glob '!Cargo.lock'`
- `git -C ../shape-app diff --check -- Cargo.toml shape-server/Cargo.toml Cargo.lock`

No `cargo` build/test was run for `../shape-app` in this supervisor turn. A
follow-up build gate should verify the dirty sibling app workspace under the
same cgroup resource policy before treating this as release evidence.
