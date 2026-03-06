# Shape Language

> [!WARNING]
> Shape is currently **alpha** and **experimental**. Syntax, runtime behavior, APIs, module formats, and tooling may change between releases.

Shape is a statically typed, expression-oriented language for data transformation and simulation, with built-in support for checkpoint/resume and distributed continuation.

## What Makes Shape Different

- **Resumable + distributed workflows**: checkpoint execution with `snapshot()`, resume deterministically, and hand continuation across workers through the transport/state stack.
- **Unified metaprogramming model**: `comptime` and annotations are one system, so compile-time generation, validation, and runtime policy wrappers compose cleanly.
- **Polyglot functions in-source**: write typed inline Python (and other runtimes) directly in Shape modules with automatic marshaling at the boundary.
- **Data-native language surface**: vectors, typed objects, and tables are core language types, not bolt-on libraries.
- **Static types without verbosity**: strong inference, expression-oriented control flow, traits/generics, and explicit `Option`/`Result` error semantics.

## What Shape Looks Like

```shape
from std::core::snapshot use { Snapshot }

fn classify(values: Vec<number>) -> string {
  let avg = values.mean()
  if avg > 20.0 { "high" } else { "normal" }
}

let readings = [18.0, 22.0, 27.0, 19.0]
print(classify(readings))

match snapshot() {
  Snapshot::Hash(id) => {
    print("saved snapshot: " + id)
    exit(0)
  }
  Snapshot::Resumed => {
    print("resumed")
  }
}
```

## Language Capabilities

- **Core language**: functions, lambdas, traits, generics, enums, pattern matching, references/borrowing
- **Type and error model**: inference-first static typing, structural object typing, `Option`/`Result`, typed propagation (`?`, `!!`)
- **Expression-first semantics**: `if`, `match`, and blocks return values, enabling compact pipeline-style code
- **Comptime + annotations**: compile-time hooks, runtime hooks, generated APIs, policy wrappers, and contract validation
- **Concurrency and control**: async/await, scoped joins, await annotations for orchestration policies
- **Resumability and distribution**: snapshot/resume, continuation handoff, state serialization, transport/wire primitives
- **Modules and packaging**: modules, packages, lockfiles, content-addressed bytecode, signed distribution paths
- **Interop**: native extensions, C interop, polyglot functions, Python extension runtime

## Polyglot Example

Shape supports inline foreign-language functions via extensions.
Example (`fn python ...`) from the book syntax:

```shape
fn python std_dev(values: Vec<number>) -> number {
  import math
  mean = sum(values) / len(values)
  variance = sum((x - mean) ** 2 for x in values) / len(values)
  return math.sqrt(variance)
}

let sigma = std_dev([4.0, 7.0, 13.0, 2.0, 1.0])
print(sigma)
```

## Try It Quickly

Build workspace:

```bash
cargo build --workspace
```

Run REPL:

```bash
cargo run -p shape-cli --bin shape
```

Run a script:

```bash
cargo run -p shape-cli --bin shape -- path/to/script.shape
```

Cross-compile CLI release artifacts (for example CI on `amd64` producing `arm64` binaries):

```bash
cargo install cross --locked
cross build --release -p shape-cli --bin shape --target x86_64-unknown-linux-gnu
cross build --release -p shape-cli --bin shape --target aarch64-unknown-linux-gnu
```

## Learn Shape

The canonical reference is the Shape Book:
`https://book.shape-lang.dev`

## Monorepo Layout

- `crates/`: language, runtime, wire, tooling, and visualization crates
- `bin/`: user-facing binaries (`shape-cli`)
- `tools/`: developer tooling (`shape-lsp`, `shape-test`, `xtask`)
- `extensions/`: native/runtime extension crates
- `docs/`: design docs, audits, and architecture notes
- `tree-sitter-shape/`: parser grammar and editor integration assets

## License

Dual-licensed under `MIT OR Apache-2.0`, unless a crate states otherwise.
