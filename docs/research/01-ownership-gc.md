# Research survey 01 — Ownership & garbage collection

Scope: ownership analysis, lifetime systems, garbage collection strategies, regions, and recent (2024–2026) hybrid approaches. This is a fact-and-tradeoff survey, not a recommendation.

Conventions: dates given as "current state unknown — needs verification" mean the public record I could surface here doesn't go past a given point. Citations follow each entry; URLs are in the references list at the bottom of each section.

---

## 1. Reference counting strategies

### 1.1 Perceus (Koka, Lean 4)
- **What it is**: A precise, compile-time reference-counting algorithm that emits drop/dup instructions inline at every variable's last use, producing "garbage-free" execution for cycle-free programs (only live references retained).
- **Key innovation**: Combines (a) precise per-program-point RC ops, (b) reuse analysis that converts `match` + reconstruct patterns into in-place mutation when RC=1, and (c) specialization for known-unique calling contexts. Enables FBIP (functional-but-in-place).
- **Performance**: In Koka's PLDI'21 evaluation, Perceus is competitive with or beats OCaml, Haskell (GHC), Java/G1, Swift, and even C++ on benchmarks like `rbtree`, `cfold`, `deriv`. Typical 40%+ peak-memory reduction vs. OCaml on `cfold`/`deriv`. Cycle-free pure-FP code is the regime; mutable cycles are out of scope.
- **Ergonomic cost**: Programmer-invisible by default. The `fbip`/`fip` keywords are opt-in static checks if the user wants a guarantee. Cycles must be broken manually (or via cycle-tolerating wrappers).
- **Maturity**: Production for Koka; production for Lean 4 (a variant — see §1.2). Algorithm is research-mature.
- **Cycles**: Not handled by Perceus itself — assumes acyclic data, typical of pure FP. Cyclic structures require external mechanisms (weak refs, manual breaking, or a separate cycle collector).
- **References**:
  - Reinking, Xie, de Moura, Leijen. "Perceus: Garbage Free Reference Counting with Reuse." PLDI 2021. https://www.microsoft.com/en-us/research/wp-content/uploads/2021/06/perceus-pldi21.pdf
  - Tech report MSR-TR-2020-42 (extended). https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/

### 1.2 Frame-limited reuse (Lorenzen & Leijen 2022, follow-ups 2024+)
- **What it is**: A refined version of Perceus reuse analysis where the compiler may hold memory slightly longer (bounded by a constant frame size) in exchange for a simpler, more robust reuse-token assignment algorithm — "drop-guided reuse."
- **Key innovation**: Replaces match-driven reuse with drop-driven reuse. Generalizes the linear resource calculus to a frame-limited semantics where allocator-call counts are bounded by a constant factor (so still asymptotically optimal but no longer strictly garbage-free at every program point).
- **Performance**: Authors report fewer corner-case allocations than original Perceus on real Koka benchmarks; numbers in the ICFP'22 paper. Subsequent FP² (ICFP'23) paper shows fully-in-place static guarantee using a stricter `fip` calculus on top.
- **Ergonomic cost**: Same as Perceus from the user's perspective. The `fip`/`fbip` keywords expose the static check.
- **Maturity**: Research, but implemented in the Koka compiler (released `v2.4.2+`).
- **Cycles**: Same status as Perceus.
- **References**:
  - Lorenzen, Leijen. "Reference Counting with Frame-Limited Reuse." ICFP 2022. https://dl.acm.org/doi/10.1145/3547634
  - Lorenzen, Leijen, Swierstra. "FP²: Fully in-Place Functional Programming." ICFP 2023. https://dl.acm.org/doi/10.1145/3607840

### 1.3 Lean 4 runtime (Counting Immutable Beans)
- **What it is**: Lean 4's production reference-counting runtime. Each `lean_object` has an `m_rc` header field: `> 0` = single-threaded count, `< 0` = encoded multi-threaded count, `= 0` = persistent (immortal) object — no RC needed.
- **Key innovation**: (a) Statically partitions objects into ST/MT classes so most RC ops are non-atomic; (b) borrow-annotation inference: a heuristic decides which parameters are passed as borrowed references (no inc/dec at call), eliding RC ops; (c) `lean_object*` is fixed-layout, so reuse analysis can directly write into freed memory.
- **Performance**: Lean's compiler self-bootstraps acceptably; the language is widely deployed for theorem proving with this runtime. No published end-to-end benchmark beating tracing GC by a stated margin, but the design closely follows Perceus's published advantages.
- **Ergonomic cost**: Invisible to library authors. Borrow inference is automatic; explicit `@[extern]` FFI must respect ownership conventions.
- **Maturity**: Production (Lean 4, since ~2021).
- **Cycles**: Lean's typed core is acyclic by construction (terms, types, values are inductive); cycles can only appear via reference cells (`IO.Ref`) which cycle-collect via explicit weak refs or programmer discipline.
- **References**:
  - Ullrich, de Moura. "Counting Immutable Beans: Reference Counting Optimized for Purely Functional Programming." IFL 2019. https://arxiv.org/abs/1908.05647
  - Lean reference manual. https://lean-lang.org/doc/reference/latest/Run-Time-Code/Reference-Counting/
  - `lean.h` (canonical implementation). https://github.com/leanprover/lean4/blob/master/src/include/lean/lean.h

### 1.4 Roc lang (Morphic + opportunistic in-place mutation)
- **What it is**: Roc's compile-time alias analysis (the "Morphic Solver") infers when a heap value is statically guaranteed to be unique at a use site, allowing in-place mutation; falls back to RC at runtime for the rest.
- **Key innovation**: Whole-program mutability inference via a separate research language ("Morphic") whose ideas were imported into Roc's compiler. Mutation appears in the source as if pure (pure FP semantics) but executes as in-place when safe. Defunctionalization via "lambda sets" enables monomorphizing closure types.
- **Performance**: No formally published benchmarks; project blog claims competitive with OCaml on typical workloads. Notable: when an `Str.concat` sees a unique string, it mutates in place rather than copying.
- **Ergonomic cost**: Invisible — programmer writes pure code. Cost is compile-time (whole-program analysis required).
- **Maturity**: Pre-1.0 (as of early 2026, no stable release yet); active project, large active community. Production-ready research-prototype is a fair characterization.
- **Cycles**: Roc's data model is purely functional + acyclic, so no cycle collector needed.
- **References**:
  - "Fast" page describing Morphic + RC + in-place. https://www.roc-lang.org/fast
  - Morphic-lang research. https://morphic-lang.org/
  - Master's thesis "Reference Counting with Reuse in Roc," Utrecht. https://studenttheses.uu.nl/handle/20.500.12932/44634

### 1.5 Biased Reference Counting (Choi et al. 2018)
- **What it is**: Per-object RC split into a (non-atomic) "biased" half-word for the owner thread and an atomic "shared" half-word for other threads. The owner thread updates RC without atomic ops.
- **Key innovation**: Most objects are touched primarily by one thread; bias toward that thread to skip atomics for the common case. Re-bias is possible.
- **Performance**: Implemented in the Swift runtime, the original PACT'18 paper reports each RC op >2× faster in the common case, 22.5% client-program speedup, 7.3% server-program throughput gain on the Swift benchmark suite. Inspired the proposal pattern in CPython's PEP 703 (no-GIL).
- **Ergonomic cost**: None (transparent runtime change). Cost is per-object header bytes (16-bit owner-TID + counters).
- **Maturity**: Research, implementations: Swift runtime exploration, Python no-GIL (PEP 703). A Rust crate (`biasedrc.rs`) exists but is not widely used.
- **Cycles**: Orthogonal — does not address cycles.
- **References**:
  - Choi, Shull, Torrellas. "Biased Reference Counting: Minimizing Atomic Operations in Garbage Collection." PACT 2018. https://iacoma.cs.uiuc.edu/iacoma-papers/pact18.pdf
  - PEP 703 discussion of BRC for CPython. https://peps.python.org/pep-0703/

### 1.6 Generational / deferred / buffered RC
- **What it is**: Family of optimizations that defer or batch RC updates: ignore RC ops on roots/young objects (deferred RC), buffer dec ops between collections (Bacon 2001), or piggyback on a generational copying nursery (Ulterior RC, Blackburn & McKinley 2003).
- **Key innovation**: Decouple the cost of RC update from the call site; pay it lazily in a phase that can amortize.
- **Performance**: Ulterior RC (OOPSLA'03) reported throughput comparable to fastest generational mark-sweep with bounded pause times.
- **Ergonomic cost**: None — runtime-internal.
- **Maturity**: Research mostly; some ideas absorbed into LXR (§3.2) and modern JVM hybrids. Pure-deferred-RC isn't dominant in any production runtime today.
- **Cycles**: Still requires a cycle collector or is paired with one (often Bacon-Rajan trial deletion).
- **References**:
  - Blackburn, McKinley. "Ulterior Reference Counting: Fast Garbage Collection without a Long Wait." OOPSLA 2003. https://www.cs.utexas.edu/~mckinley/papers/urc-oopsla-2003.pdf
  - Shahriyar PhD thesis, "High Performance Reference Counting and Conservative Garbage Collection," ANU 2015. https://www.steveblackburn.org/pubs/theses/shahriyar-2015.pdf

### 1.7 Bacon-Rajan trial deletion (concurrent cycle collection)
- **What it is**: A localized cycle-detection algorithm layered on top of reference counting. Candidate roots ("possibly-cyclic" objects whose RC dropped to nonzero) are queued; cycle collector walks subgraphs decrementing internal-only edges; surviving counts indicate live cycles, others are dead.
- **Key innovation**: No global heap traversal; concurrent-safe with mutator; multiprocessor implementation in IBM Jalapeño hit 6 ms max pause.
- **Performance**: Original 2001 paper: 6 ms max pause, throughput within ~10% of stop-the-world equivalents. Algorithm is the basis for CPython's `gc` module and many production RC cycle collectors.
- **Ergonomic cost**: Programmer-transparent. Runtime keeps a "candidate set" structure.
- **Maturity**: Production — CPython, PHP (Zend), and many others use a variant. Algorithm itself is canonical.
- **Cycles**: This *is* the cycle handler.
- **References**:
  - Bacon, Rajan. "Concurrent Cycle Collection in Reference Counted Systems." ECOOP 2001. https://pages.cs.wisc.edu/~cymen/misc/interests/Bacon01Concurrent.pdf
  - Ravitch, Giallorenzo et al. "Breadth-first Cycle Collection RC: Theory and a Rust Smart Pointer Implementation." SAC 2025 — modern variant ("SILB-Recycler") with `Cc<T>` smart pointer competitive with `Rc<T>`. https://www.saveriogiallorenzo.com/publications/sac2025a/sac2025a.pdf

### 1.8 Concurrent Deferred Reference Counting (Anderson et al. 2021)
- **What it is**: A C++ library combining hazard pointers with deferred RC to give wait-free constant-time RC overhead in the multi-threaded case.
- **Key innovation**: Generalized hazard pointers defer dec until no concurrent inc can race, and elide short-lived increments entirely.
- **Performance**: PLDI'21 paper claims faster than existing atomic-RC libraries (e.g. `std::shared_ptr`), competitive with manual reclamation (hazard pointers, RCU).
- **Ergonomic cost**: API is `atomic_rc_ptr<T>` — drop-in feel, transparent. Cost: per-thread hazard-pointer slots.
- **Maturity**: Open-source library (`cmuparlay/concurrent_deferred_rc`). Research artifact, not yet in production runtimes.
- **Cycles**: Does not address cycles.
- **References**:
  - Anderson, Blelloch, Wei. "Concurrent Deferred Reference Counting with Constant-Time Overhead." PLDI 2021. https://www.cs.cmu.edu/~guyb/papers/3453483.3454060.pdf
  - https://github.com/cmuparlay/concurrent_deferred_rc

---

## 2. Compile-time ownership analysis

### 2.1 Rust borrow checker (NLL, Polonius, GATs)
- **What it is**: Affine ownership + lifetime-parametric references with aliasing-XOR-mutability invariant, statically checked. NLL = non-lexical lifetimes (since Rust 2018). Polonius = next-gen location-sensitive borrow checker.
- **Key innovation**: Production-quality affine type system with inference that doesn't require user lifetime variables in most cases. GATs (generic associated types, stable since Rust 1.65) enable lending iterators, families of types parameterized by lifetimes.
- **Performance**: Compile-time only; runtime cost is zero (no allocator, no RC). Borrow check is non-trivial — Polonius accepts a 10–20% compile-time hit on its alpha goal.
- **Ergonomic cost**: Steep learning curve; well-known "fighting the borrow checker" period. Some valid programs are still rejected (NLL Problem Case 3, lending iterators with closures).
- **Maturity**: Production at scale. Polonius alpha is targeted for stabilization in 2026 per the rust-project-goals. https://rust-lang.github.io/rust-project-goals/2026/polonius.html
- **Cycles**: Not solved by borrow checker — `Rc<T>` + `Weak<T>` for shared-ownership cycles, programmer's responsibility.
- **References**:
  - Polonius Working Group. https://rust-lang.github.io/compiler-team/working-groups/polonius/
  - Niko Matsakis "Polonius revisited" series. https://smallcultfollowing.com/babysteps/blog/2023/09/22/polonius-part-1/
  - Polonius repo. https://github.com/rust-lang/polonius

### 2.2 Mojo (owned / borrowed / inout / origins)
- **What it is**: Modular's systems language with an ownership system inspired by Rust but redesigned around Python-feel ergonomics. Three argument conventions: `borrowed` (immutable ref), `inout` (mutable ref), `owned` (transfer). ASAP destruction. ARC handled by compiler with elision when ownership inference allows.
- **Key innovation**: "Origins" (renamed from "lifetimes" in Mojo 24.6) — origin is a parameter that can be derived from arguments, enabling reference-as-return without explicit lifetime annotation in many cases. Recent (2025) proposal: replace aliasing-XOR-mutability on references with the same restriction *on origin parameters*, gaining expressivity vs. Rust.
- **Performance**: AOT-compiled via MLIR; ARC overhead minimized by elision. No published apples-to-apples benchmark vs. Rust; the language is still pre-stable.
- **Ergonomic cost**: Caret transfer operator (`^`); parameter conventions are learnable; lifetime annotations less ubiquitous than Rust due to inference. Active area of design: improved ergonomics for mutable aliasing patterns common in Python/C++.
- **Maturity**: Mojo 25.4 (as of June 2025) is the latest stable; language is open-source (mostly), pre-1.0. Modular roadmap is active. Current state of the proposed "origin-XOR-mutability" model — needs verification beyond early 2026.
- **Cycles**: ARC means same cycle problem as Swift; cycle handling left to the programmer (weak refs). No cycle collector announced.
- **References**:
  - Mojo manual: ownership and lifetimes. https://docs.modular.com/mojo/manual/values/ownership/ , https://docs.modular.com/mojo/manual/values/lifetimes/
  - "Deep dive into ownership in Mojo." https://www.modular.com/blog/deep-dive-into-ownership-in-mojo
  - Nick Smith's "alternative model for lifetimes." https://gist.github.com/nmsmith/cdaa94aa74e8e0611221e65db8e41f7b

### 2.3 Lobster (van Oortmerssen)
- **What it is**: Statically-typed scripting language for indie games. Compiles down to RC + bytecode/C, but a compile-time ownership analysis elides ~95% of RC ops at compile time.
- **Key innovation**: Algorithm picks a single "owner" for each heap allocation (typically the first variable/field/element it's assigned to). Analysis is *interleaved with type checking* and operates on every AST node, not just storage locations. Each AST node has an "ownership kind" expected of children and passed to parent.
- **Performance**: Author's claim: 95% of RC ops removed. No formal benchmark vs. Rust/Swift, but Lobster ships shipped indie games (e.g. *Cube 2: Sauerbraten* descendants). Nim's `--gc:arc` is described as "effectively Lobster's algorithm."
- **Ergonomic cost**: Mostly transparent; programmer writes high-level code.
- **Maturity**: Single-author project, used for shipped games. Research-quality algorithm with practical deployment.
- **Cycles**: Cycles need explicit handling (typically weak refs). Cycle collection ideas have been discussed but not the project's focus.
- **References**:
  - "Memory Management in Lobster." https://aardappel.github.io/lobster/memory_management.html
  - "Compile time reference counting & Lifetime Analysis in Lobster" (talk). https://www.youtube.com/watch?v=WUkYIdv9B8c

### 2.4 Austral (linear types)
- **What it is**: Systems language using pure linear types (every value used exactly once) as its sole ownership primitive. Manual memory management, statically checked.
- **Key innovation**: Minimalism — linearity checker is "less than 1000 lines of OCaml." Memory + file handles + capabilities all flow through the same linear-typing discipline. No RC, no GC.
- **Performance**: Zero runtime overhead — no RC ops, no allocator interference. Performance is whatever the user codes (manual management).
- **Ergonomic cost**: Significant — every linear value must be consumed exactly once or borrowed; explicit destructors. Read-only references and mutable references reintroduce something Rust-shaped on top.
- **Maturity**: Research / hobby; small stdlib; principled. Not used in production. Author Borretti is a single primary maintainer.
- **Cycles**: Cycles are a programmer-explicit problem (linear types make them awkward).
- **References**:
  - "Introducing Austral." https://borretti.me/article/introducing-austral
  - Austral linear types tutorial. https://austral-lang.org/tutorial/linear-types
  - https://github.com/austral/austral

### 2.5 Linear Haskell
- **What it is**: GHC extension `-XLinearTypes` (since GHC 9.0) introducing multiplicity-annotated function arrows: `a %1 -> b` consumes its argument linearly, `a %Many -> b` is normal. Multiplicity polymorphism (`p`) bridges the two.
- **Key innovation**: Linearity is on the function arrow, not the type — so the same data type can be linear or unrestricted by context. Fully backwards compatible.
- **Performance**: Compile-time only. Enables in-place updates with safety, but production GHC backend doesn't yet aggressively exploit linearity for codegen optimization.
- **Ergonomic cost**: Real — multiplicity polymorphism, linear let-bindings (added 2024 in GHC 9.10), interaction with do-notation are all rough edges. `case` on linear values requires special treatment.
- **Maturity**: Production-available extension, not in default GHC2024 language edition. Used in industrial libraries like `linear-base`, `inline-java`, `safe-tensors`. Tweag (the original sponsor) and Well-Typed continue maintenance.
- **Cycles**: Haskell's lazy GC handles cycles unchanged.
- **References**:
  - Bernardy, Boespflug, Newton, Peyton Jones, Spiwack. "Linear Haskell: practical linearity in a higher-order polymorphic language." POPL 2018. https://arxiv.org/abs/1710.09756
  - GHC linear-types wiki. https://gitlab.haskell.org/ghc/ghc/-/wikis/linear-types
  - Tweag desugaring blog (2024). https://www.tweag.io/blog/2024-01-18-linear-desugaring/

### 2.6 Idris 2 (Quantitative Type Theory)
- **What it is**: Dependently-typed language whose core is Quantitative Type Theory — every binding has a multiplicity in {0, 1, ω}. 0 = compile-time only (erased), 1 = used exactly once (linear), ω = unrestricted.
- **Key innovation**: Linearity unified with erasure: 0-quantity arguments are guaranteed compile-time only, removing them at runtime is sound. Linear and dependent types coexist without phase distinction issues.
- **Performance**: Idris 2 compiles via Chez Scheme (default) or other backends; runtime perf is Scheme-tier, not C-tier. Linearity informs compilation but not yet aggressively.
- **Ergonomic cost**: QTT is the cleanest formal framework here, but writing programs against it remains research-grade. Inference of multiplicities is partial.
- **Maturity**: Used as research vehicle; pre-1.0; small community. Edwin Brady is the primary maintainer.
- **Cycles**: Backend GC (depends on backend — Chez has tracing GC).
- **References**:
  - Brady. "Idris 2: Quantitative Type Theory in Practice." ECOOP 2021. https://arxiv.org/abs/2104.00480
  - Multiplicities docs. https://idris2.readthedocs.io/en/latest/tutorial/multiplicities.html

### 2.7 Verona (Microsoft Research)
- **What it is**: Research language for concurrent ownership; partitions all program objects into a forest of regions; reference capabilities enforce isolation between regions.
- **Key innovation**: Region-as-isolation-unit + per-region memory-management policy choice (RC, tracing, arena, …) selectable per region. "Window of mutability" — at most one region is active for mutation at a time.
- **Performance**: No published end-to-end numbers; OOPSLA'23 paper "Reference Capabilities for Flexible Memory Management" describes the type system but performance evaluation is preliminary.
- **Ergonomic cost**: Programmer writes region annotations and capability annotations (`iso`, `mut`, `imm`); learning curve, partial inference. Active design changes — language is "undergoing massive refactoring" (2024–2025 GitHub status).
- **Maturity**: Research only; explicitly "not ready for use outside research" per project FAQ. Active publication stream: PLDI'25 "Dynamic Region Ownership for Concurrency Safety."
- **Cycles**: Within a region, region-local memory mgmt; across regions, isolation prevents cross-region cycles by construction.
- **References**:
  - Project Verona. https://microsoft.github.io/verona/
  - "Reference Capabilities for Flexible Memory Management." OOPSLA 2023. https://dl.acm.org/doi/10.1145/3622846
  - PLDI 2025 "Dynamic Region Ownership" — needs verification of exact details.

### 2.8 Vale (generational references + regions)
- **What it is**: Systems language using "generational references" — every object has an inline 64-bit generation counter; every pointer carries a remembered 64-bit generation; deref asserts they match. Free increments the generation, invalidating all stale pointers.
- **Key innovation**: Memory safety without borrow checking *or* tracing GC. Each object owned by exactly one place yet freely aliasable. Region borrowing layered on top to bulk-elide generation checks within an immutably-borrowed region.
- **Performance**: Author benchmarks (terrain generator) show 2–10.84% overhead vs. unsafe baseline. Region borrowing reportedly eliminates checks entirely in CA-style algorithms.
- **Ergonomic cost**: Allows observers, callbacks, graphs, dependency injection — patterns Rust struggles with. Region annotations are explicit (when used for optimization).
- **Maturity**: Pre-alpha research language; single-developer-led (Verdagon). Aimed to "complete regions by early 2024" — actual release timeline current state unknown.
- **Cycles**: Cycles are fine: deref check fires on use of stale pointers, no cycle collection needed.
- **References**:
  - Vale memory-safety strategy. https://verdagon.dev/blog/generational-references
  - "Zero-Cost Memory Safety with Vale Regions (Preview)." https://verdagon.dev/blog/zero-cost-memory-safety-regions-overview
  - https://github.com/ValeLang/Vale

### 2.9 Pony (reference capabilities + ORCA)
- **What it is**: Actor language with six reference capabilities: `iso` (unique deep-immutable transferable), `val` (deeply immutable, sendable), `ref` (mutable, single-actor), `box` (read-only view), `trn` (write-unique, allows local readers), `tag` (opaque identity).
- **Key innovation**: Capability *combinations* prevent data races at compile time — sendable iff `iso`/`val`/`tag`. Pairs with ORCA, a fully-concurrent actor-local GC: each actor reclaims its own heap independently using deferred distributed weighted RC; no STW, no read/write barriers (justified by data-race-free typing).
- **Performance**: Designed for low-latency actor systems; per-actor 256-byte overhead at 64-bit. No barriers in mutator. Hard numbers are scarce in literature; production users (e.g. WallarooLabs) reportedly run millions of actors.
- **Ergonomic cost**: Cap matrix is non-trivial; `recover` blocks for promoting capabilities; learning curve real but bounded.
- **Maturity**: Production for some shops; small community; project is alive but slow.
- **Cycles**: ORCA's distributed weighted RC handles cycles within an actor by tracing on actor death; cross-actor cycles not allowed by capability typing.
- **References**:
  - Clebsch, Drossopoulou, Blessing, McNeil. "Orca: GC and type system co-design for actor languages." OOPSLA 2017. http://janvitek.org/pubs/oopsla17a.pdf
  - Pony tutorial: reference capabilities. https://tutorial.ponylang.io/reference-capabilities/reference-capabilities.html

---

## 3. Tracing GC — frontier

### 3.1 ZGC / Generational ZGC / Shenandoah
- **What it is**: OpenJDK concurrent collectors. ZGC: region-based, colored pointers (load-value barriers). Shenandoah: Brooks-pointer indirection, concurrent compaction. Both target sub-ms pauses for multi-TB heaps.
- **Key innovation**: ZGC: load-value barriers + colored pointers obviate marking pauses entirely. Shenandoah: per-object forwarding indirection allows truly concurrent moves.
- **Performance**: ZGC on Java 25 (Sept 2025) sub-ms pauses on multi-TB heaps; ~15–30% memory overhead, ~5–10% CPU overhead vs. G1. Generational ZGC ships as the only ZGC mode in Java 25 LTS. Generational Shenandoah no longer experimental in Java 25 (~30% throughput improvement vs non-generational Shenandoah).
- **Ergonomic cost**: Tuning flags; not invisible to ops teams.
- **Maturity**: Production at very large scale (Twitter, Netflix, large fintech).
- **Cycles**: Tracing — handles cycles natively.
- **References**:
  - Beginner's guide to Shenandoah. https://developers.redhat.com/articles/2024/05/28/beginners-guide-shenandoah-garbage-collector
  - "Pauseless Garbage Collection in Java 25: ZGC Deep Dive." https://andrewbaker.ninja/2025/12/03/deep-dive-pauseless-garbage-collection-in-java-25/
  - "New in Java 25: Generational Shenandoah no longer experimental." https://theperfparlor.com/2025/09/14/new-in-java25-generational-shenandoah-gc-is-no-longer-experimental/

### 3.2 LXR (Latency-critical / Immix + Reference Counting)
- **What it is**: Hybrid GC: fast path is reference counting (with `Immix` regions); periodic SATB tracing collection handles cycles and reclaims hard cases.
- **Key innovation**: Re-establishes the case that *brief stop-the-world* RC collections beat fully-concurrent evacuation on throughput while still hitting tail-latency targets.
- **Performance**: PLDI'22 paper: 7.8× throughput, 10× better 99.99% tail latency vs. Shenandoah on Lucene under tight heap; 4% better than G1 on 17 modern workloads in moderate heaps; 43% better than Shenandoah on the same.
- **Ergonomic cost**: None (runtime-internal).
- **Maturity**: Research; implemented in OpenJDK; not the default.
- **Cycles**: SATB tracing pass cleans cycles.
- **References**:
  - Zhao, Blackburn, McKinley. "Low-Latency, High-Throughput Garbage Collection." PLDI 2022. https://www.steveblackburn.org/pubs/papers/lxr-pldi-2022.pdf
  - Extended version. https://arxiv.org/abs/2210.17175

### 3.3 Concurrent compaction (state of the art)
- **What it is**: Family of GC techniques that allow heap compaction without stopping all mutators. C4 (Azul, since ~2010), Shenandoah's Brooks-pointer scheme, ZGC's load-value barriers, and recent Jade (EuroSys'24).
- **Key innovation**: Move objects while mutators concurrently access them; correctness via read/write barriers (Brooks indirection or colored pointers) or atomic forwarding tables.
- **Performance**: Jade (EuroSys'24) reports sub-millisecond pauses under heavy workload while improving throughput vs. ZGC by avoiding lengthy pre-reclamation.
- **Ergonomic cost**: None.
- **Maturity**: Production (C4, ZGC, Shenandoah). Research frontier moves toward reducing barrier overhead and metadata cost.
- **Cycles**: Tracing, so cycles are native.
- **References**:
  - Tene, Iyengar, Wolf. "C4: The Continuously Concurrent Compacting Collector." ISMM 2011. http://paperhub.s3.amazonaws.com/d14661878f7811e5ee9c43de88414e86.pdf
  - "Jade: A High-throughput Concurrent Copying Garbage Collector." EuroSys 2024. https://dl.acm.org/doi/10.1145/3627703.3650087

### 3.4 Generational tracing — when is it still preferred over RC + reuse?
- **What it is**: Classic tracing GC with weak generational hypothesis. Still the default in JVM (G1, ZGC), .NET, V8, JavaScriptCore, OCaml, Go.
- **Key innovation (current)**: Already a mature design, ongoing work refines pause-time / throughput tradeoff (LXR section above is the cleanest counter-argument).
- **Tradeoff vs. RC+reuse**: Tracing wins on (a) heavy mutation of heap structure (high mutator pointer-write rates negate RC's amortized advantage), (b) cyclic structures without explicit weak refs, (c) workloads where peak throughput matters more than memory predictability. RC + reuse wins on (a) functional / immutable data, (b) workloads needing predictable memory release (servers, embedded), (c) when compile-time analysis can elide most RC ops.
- **Maturity**: Both are production-grade; tracing is the dominant choice for general-purpose dynamic languages.
- **References**: see §3.1, §3.2; Bacon et al. "A Unified Theory of Garbage Collection." OOPSLA 2004. https://web.eecs.umich.edu/~weimerw/2008-415/reading/bacon-garbage.pdf

---

## 4. Region-based memory management

### 4.1 ML Kit region inference (Tofte–Talpin)
- **What it is**: Static type-and-effect analysis that infers, for every allocation site in an ML program, which lexically-nested region holds the value. Region entry/exit instructions are inserted automatically; no GC runtime.
- **Key innovation**: Pure compile-time — region operations (allocate, deallocate) are constant-time. Soundness and termination proven.
- **Performance**: ML Kit benchmarks competitive with garbage-collected systems; significant peak-memory savings on programs with clear stack-shaped allocation patterns. Pathological programs ("region leaks") perform much worse than tracing GC.
- **Ergonomic cost**: Mostly transparent — but when inference fails to find tight regions, programmers need to refactor or accept large regions that retain memory longer than necessary. Has historically been fragile.
- **Maturity**: ML Kit is a maintained research compiler (Mads Tofte, Martin Elsman). Region inference itself influenced Cyclone, Rust lifetimes, Verona.
- **Cycles**: Regions are bulk-freed; cycles within a region are reclaimed with the region.
- **References**:
  - Tofte, Talpin. "Region-Based Memory Management." Information and Computation, 1997. http://ropas.snu.ac.kr/lib/dock/ToTa1997.pdf
  - Tofte, Birkedal. "A Region Inference Algorithm." TOPLAS 1998. https://elsman.com/mlkit/pdf/toplas98.pdf

### 4.2 Cyclone
- **What it is**: Type-safe C dialect (early 2000s). Introduced explicit region annotations on pointers (`int *@region`), region subtyping, integration with stack/heap/garbage-collected/dynamic regions.
- **Key innovation**: Direct ancestor of Rust's lifetime system. Showed that practical systems code can be written with explicit regions if defaults and inference are good.
- **Performance**: Comparable to C in practice; region overhead minimal.
- **Ergonomic cost**: Annotations were sometimes heavy; default-annotation system + local type inference helped.
- **Maturity**: Abandoned ~2006; project explicitly closed. Influence on Rust documented by Rust team.
- **Cycles**: Same as ML Kit — within a region.
- **References**:
  - Grossman, Morrisett, Jim, Hicks, Wang, Cheney. "Region-Based Memory Management in Cyclone." PLDI 2002. https://www.cs.umd.edu/projects/cyclone/papers/cyclone-regions.pdf

### 4.3 Vale's regions (modern revival)
- **What it is**: Distinct from ML-style region inference: Vale's regions are *mutability windows*. A region can be immutably borrowed for a scope, eliding all generational checks within. Manual but checked.
- **Key innovation**: Rust-like aliasing-XOR-mutability, but at region granularity instead of per-reference. Coexists with mutable aliasing within a region.
- **Performance**: Author claims region borrowing eliminates "every single generation check" in CA-like algorithms; preview blog only, no peer-reviewed numbers.
- **Ergonomic cost**: Programmer chooses when to invoke region borrowing. Not always required — generational refs still safe without it.
- **Maturity**: Preview / pre-1.0.
- **References**: see §2.8.

---

## 5. Hybrid / novel approaches

### 5.1 Lean 4 FBIP (Functional But In-Place)
- **What it is**: Compilation strategy where reuse analysis (Perceus-style) lets purely functional `match`/reconstruct patterns become in-place mutations when the ref count is 1. The `fbip` keyword in Koka is a *static check* that the function compiles to in-place updates without allocation in the linear case (allows non-tail calls, allows deallocation, unlike `fip`).
- **Key innovation**: Express imperative algorithms (mergesort, splay trees, finger trees) in pure-functional style with imperative-equivalent runtime characteristics. FP² (ICFP'23) tightens the static guarantee.
- **Performance**: When verified `fip`, zero allocations after warmup; constant stack space. Microbenchmarks in the FP² paper.
- **Ergonomic cost**: `fip`/`fbip` annotations require careful programming (use each linear arg exactly once; avoid sharing). Catches mistakes at compile time.
- **Maturity**: Production in Koka; the *technique* is portable. Lean 4 uses Perceus + reuse but has not officially adopted the `fip` keyword.
- **References**:
  - Lorenzen, Leijen, Swierstra. "FP²: Fully in-Place Functional Programming." ICFP 2023. https://webspace.science.uu.nl/~swier004/publications/2023-icfp.pdf
  - Microsoft blog. https://www.microsoft.com/en-us/research/blog/fp2-fully-in-place-functional-programming-provides-memory-reuse-for-pure-functional-programs/

### 5.2 Frame-limited reuse (Lorenzen & Leijen, follow-ups)
- See §1.2. Distinct from §5.1 in that this is the compilation algorithm; FP² is the linguistic guarantee built on top.

### 5.3 Linear Haskell in production
- **What it is**: See §2.5. Real industrial uses include Tweag's `linear-base`, in-place vector mutation libraries, safe FFI to C.
- **What works**: Linear FFI bindings (`Foreign.Marshal.Pure`), linear arrays for in-place algorithms, session types, resource handles where exhaustion matters.
- **What doesn't (yet)**: Library ecosystem is bifurcated — most existing libraries don't expose linear variants. Multiplicity polymorphism is verbose and doesn't compose well with do-notation. GHC backend doesn't aggressively optimize based on linearity.
- **Maturity**: Stable extension, niche use.
- **References**: See §2.5.

### 5.4 Granule (graded modal types for resources)
- **What it is**: Functional language whose type system tracks *graded* uses: type `a [n]` means "an `a` usable `n` times." Indices come from a resource semiring — naturals, intervals, security levels, …
- **Key innovation**: Subsumes linear types (n=1), affine types (n ≤ 1), unrestricted (n=ω), and security/privacy bounds in one framework.
- **Performance**: Research interpreter; perf is not the focus.
- **Ergonomic cost**: Heavy — programmer reasons about semiring elements. Not aimed at production.
- **Maturity**: Active research; small group at University of Kent (Dominic Orchard et al.).
- **References**:
  - Orchard, Liepelt, Eades. "Quantitative Program Reasoning with Graded Modal Types." ICFP 2019. https://www.cs.kent.ac.uk/people/staff/dao7/publ/granule-icfp19.pdf
  - https://granule-project.github.io/

### 5.5 Oxidizing OCaml (modal memory management, ICFP 2024)
- **What it is**: Jane Street + collaborators' design adding three independent *modes* to OCaml: `affinity` (≤1 use), `uniqueness` (no aliases), `locality` (stack-allocatable). Modes are inferred and backwards-compatible.
- **Key innovation**: Mode polymorphism allows the same code to be polymorphic over locality / uniqueness, similar to Linear Haskell's multiplicity polymorphism but with three orthogonal axes. Enables stack allocation of closures and in-place updates of unique values, alongside the existing OCaml GC.
- **Performance**: Authors report meaningful allocation reductions in Jane Street's production OCaml codebase. Specific numbers in the ICFP'24 paper and Jane Street blog series.
- **Ergonomic cost**: New mode annotations; designed to be inferable so programmers rarely write them.
- **Maturity**: In-development in OCaml fork (Jane Street's `oxcaml`); not yet upstream. Well-funded, well-engineered.
- **Cycles**: OCaml's tracing GC handles cycles unchanged; modes optimize the fast path.
- **References**:
  - Lorenzen, White, Dolan, Eisenberg, Lindley. "Oxidizing OCaml with Modal Memory Management." ICFP 2024. https://antonlorenzen.de/oxidizing-ocaml-modal-memory-management.pdf
  - Jane Street blog series. https://blog.janestreet.com/oxidizing-ocaml-locality/

### 5.6 Destination Calculus (Bagrel & Spiwack, OOPSLA 2025)
- **What it is**: Linear λ-calculus with first-class *destinations* — out-parameters that a function fills in, used to reconcile "destination-passing-style" code with pure FP semantics.
- **Key innovation**: A modal type system manages both linearity and "ages" (scope durations). Permits programs previously not expressible purely (e.g., difference lists with O(1) append).
- **Performance**: Calculus, not implementation. Influence on practical languages still emerging.
- **Ergonomic cost**: Research language; modes/ages are non-trivial.
- **Maturity**: Recent paper (March 2025 arXiv, OOPSLA'25). Experimental.
- **References**:
  - Bagrel, Spiwack. "Destination Calculus: A Linear λ-Calculus for Purely Functional Memory Writes." OOPSLA 2025. https://arxiv.org/abs/2503.07489

### 5.7 Reference Counting Deeply Immutable Data Structures with Cycles (ISMM 2024)
- **What it is**: Microsoft Research design observing that *frozen* (deeply-immutable) data, once frozen, can never form new cycles; existing cycles are themselves immutable. Therefore RC can manage frozen-cyclic data efficiently with a one-shot cycle-detection at freeze time.
- **Key innovation**: Promptness + determinism of memory release for cyclic immutable data without a tracing collector.
- **Performance**: Position paper ("intellectual abstract") — design rather than benchmarks. Slated to ground future Verona work.
- **Maturity**: Research, very recent.
- **References**:
  - Parkinson et al. "Reference Counting Deeply Immutable Data Structures with Cycles: An Intellectual Abstract." ISMM 2024. https://dl.acm.org/doi/10.1145/3652024.3665507

### 5.8 Breadth-first cycle collection (SAC 2025)
- **What it is**: New cycle-collection algorithm for refcounted systems, implemented as a Rust `Cc<T>` smart pointer ("cactusref / cycle-recycler" lineage).
- **Key innovation**: Breadth-first tracing avoids stack overflows; resilient to errors during tracing; supports finalization; no auxiliary heap.
- **Performance**: Authors report comparable to existing Rust RC alternatives, faster on cyclic workloads.
- **Maturity**: 2025 paper; published Rust crate. Single-team research.
- **References**:
  - Pais, Giallorenzo, Mezzina. "Breadth-first Cycle Collection RC: Theory and a Rust Smart Pointer Implementation." SAC 2025. https://www.saveriogiallorenzo.com/publications/sac2025a/sac2025a.pdf

### 5.9 CATALPA (low-variance GC, arXiv 2509.13429)
- **What it is**: GC for the BOSQUE language designed for very low *variance* in pause times (predictability over throughput).
- **Key innovation**: Exploits BOSQUE's all-immutable data discipline — high allocation rate but high reclamation rate.
- **Maturity**: 2025 paper (arXiv 2509.13429). Tied to the BOSQUE project, which is itself research.
- **References**: https://arxiv.org/pdf/2509.13429 — needs verification of journal/conference venue.

---

## 6. Cross-cutting findings

### 6.1 Themes appearing across multiple systems

**Theme 1 — Compile-time analysis is reducing RC overhead to "Mark":** Lean's borrow inference, Roc's Morphic, Lobster's ownership analysis, Perceus reuse, Mojo's ARC elision, and OCaml modes all converge on: `Arc<T>` everywhere is the wrong baseline; static analysis erases most ops. Numbers across projects cluster around 80–95% RC ops eliminated when the analysis is good.

**Theme 2 — Linearity has won as the formal foundation, but production languages soften it.** Pure linear (Austral) is principled but ergonomically hard. Real production designs (Linear Haskell multiplicities, Idris 2 QTT, OCaml modes, Mojo ownership conventions, Rust affine + lifetimes) all give programmers escape hatches — multiplicity polymorphism, default-unrestricted modes, borrow conventions — to keep linearity from becoming a tax.

**Theme 3 — Regions are back.** ML Kit (1990s) → Cyclone (2000s) → Verona / Vale / OCaml modes (2020s) all recover ML Kit's idea but layered on top of, not in lieu of, other memory-management strategies. The 2024–2026 design pattern: "regions are the unit at which you choose your management policy," not a replacement for the policy.

**Theme 4 — RC + occasional tracing is the modern hybrid.** LXR (PLDI'22) showed that RC fast path + brief STW tracing beats fully-concurrent tracing on tail latency *and* throughput. Pony's ORCA does per-actor RC + actor-local tracing. CPython gc and PHP Zend gc both follow Bacon-Rajan trial-deletion. The design space "pure tracing vs. pure RC" is largely closed; the live design space is *which hybrid*.

**Theme 5 — Cycles are uniformly the unsolved hard part for RC-first runtimes.** Every RC-based system either (a) requires programmer discipline (weak refs — Rust, Swift), (b) bolts on Bacon-Rajan trial deletion (Python, PHP, CactusRef in Rust), or (c) constrains the data model to acyclic (Koka, Lean, Roc — by purity). The recent ISMM'24 paper on "Reference Counting Deeply Immutable Cycles" is a notable new angle.

### 6.2 Unsolved / unconverged problems

- **Mutable aliasing without `unsafe`.** Rust's aliasing-XOR-mutability is over-restrictive for many real patterns (graphs, observers, GUI widgets, tree-with-back-edges). Mojo's 2025 origin proposal, Vale's generational refs, and Verona's regions are all attempts; none have converged on a universally accepted answer.
- **Closure capture × ownership.** Crosses repeatedly in the literature: closures capturing references are notoriously where Rust's borrow checker most often forces awkward refactoring; Mojo's origin proposal flags this; Perceus FBIP can't easily express closures over mutable state.
- **Multi-threaded RC at zero cost.** Biased RC (PACT'18), Concurrent Deferred RC (PLDI'21), and Lean 4's ST/MT split each handle the common case; the worst case is still atomic. No production runtime has fully erased the atomic-RC tax in concurrent code.
- **Closures + linear types.** GHC linear types still struggle with linear values inside closures and `do`-notation. QTT cleaner formally but production-rough. Open ergonomic problem.

### 6.3 Genuinely-novel 2024–2026 directions (flag: experimental)

- **Destination Calculus (OOPSLA'25)** — purely functional out-params, may unify difference-lists, builders, in-place updates under one framework. Implementation status unclear as of early 2026.
- **Oxidizing OCaml modes (ICFP'24)** — the cleanest *retrofit* of Rust-flavored ownership into an existing GC'd language. Three-axis modes (affinity / uniqueness / locality) appear more compositional than Linear Haskell's single multiplicity axis.
- **Verona dynamic region ownership (PLDI'25)** — region creation/transfer at runtime under typed control. Less mature than the OOPSLA'23 design.
- **Frozen-cyclic RC (ISMM'24)** — cycles in *immutable* data are tractable for RC; Verona may build this in.
- **CATALPA (2025)** — variance-as-objective rather than throughput or latency mean; novel framing for runtime-targeting languages.
- **Vale's region borrowing (preview, 2023+)** — "borrow check that you can switch on per scope." If it ships, it's a genuinely new ergonomic point; if it doesn't, it's a thought experiment.

### 6.4 Production vs. research split

| Approach | Status |
|---|---|
| Rust borrow check (NLL) | Production at scale |
| ZGC, Generational ZGC, Shenandoah | Production at scale (Java 25 LTS) |
| Bacon-Rajan trial deletion | Production (CPython, PHP, …) |
| Lean 4 RC + borrow inference | Production (Lean 4 ecosystem) |
| Perceus (Koka) | Production (Koka), portable algorithm |
| Pony ORCA | Production (small-scale: Wallaroo) |
| Linear Haskell | Production-available (extension), niche |
| Mojo ownership | Pre-1.0 production track (Modular) |
| Lobster compile-time RC | Production (single-author indie) |
| Roc + Morphic | Pre-1.0 |
| Polonius | Approaching stabilization (2026 goal) |
| Oxidizing OCaml | Pre-merge, well-funded (Jane Street) |
| LXR | Research (OpenJDK research artifact) |
| Concurrent Deferred RC | Research library |
| Vale generational refs + regions | Pre-alpha research |
| Verona | Research; "not for production use" |
| Austral | Research / hobby |
| Idris 2 / QTT | Research / theorem-proving niche |
| Granule | Research |
| Destination Calculus | Brand new (OOPSLA'25) |
| FP² / `fip` calculus | Implemented in Koka; portable |
| Frozen-cyclic RC | Position paper (2024) |

### 6.5 Where data is thin / current state needs verification

- Mojo origin-XOR-mutability proposal: **active redesign as of March 2025**; the final shipped form is unknown.
- Vale region borrowing: **preview blogs only**; no peer-reviewed performance numbers.
- Verona region system: **active refactoring**; the version of Reggio in OOPSLA'23 may not match current code.
- Polonius alpha stabilization: **targeted for 2026** but not yet shipped.
- CATALPA paper (arXiv 2509.13429): **needs verification** of journal/conference and full evaluation.
- Lean 4 FBIP: Lean uses Perceus + reuse, but hasn't formally adopted Koka's `fip`/`fbip` static checker — current state of any Lean-side equivalent **needs verification.**

### 6.6 Relevance gradient (for the question of "what to apply where")

Without recommending: the cluster *most often deployed in compile-to-native typed-functional/imperative languages with no existing GC* is **Perceus-style RC + reuse + borrow inference + an optional cycle handler**. The cluster *most often deployed when retrofitting an existing GC'd language* is **modal/qualifier types layered on top of tracing GC** (Linear Haskell, Oxidizing OCaml). The cluster *most often deployed for systems languages* is **affine ownership + lifetimes** (Rust) or **generational refs** (Vale) or **regions** (Verona). These three clusters reflect what the researcher's starting point was, not a fundamental incompatibility.
