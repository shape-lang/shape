# Architecture Decision Records

Current system decisions:

| ADR | Status | Decision |
|---|---|---|
| [001](001-value-model.md) | Superseded by ADR-006 | Historical NaN-boxed value model |
| [002](002-abi-unification.md) | Superseded by ADR-006 | Historical NaN-boxed VM/JIT ABI |
| [003](003-method-registry.md) | Accepted; clarified by ADR-010/011 | Resolved method descriptors and local dispatch handles |
| [004](004-native-c-interop.md) | Accepted; clarified by ADR-006/011 | Native C interop and resolved foreign contracts |
| [005](005-typed-slot-construction.md) | Partially superseded by ADR-006 | Single-discriminator and typed-slot discipline |
| [006](006-value-and-memory-model.md) | Accepted; clarified by ADR-003/011/012; constrained by proposed ADR-018/019 | Value, memory, method, FFI, and dynamic-adapter model |
| [007](007-arbitrary-precision-decimal.md) | Accepted | Arbitrary-precision exact decimal |
| [008](008-sequence-segment-ownership.md) | Accepted | Sequence segment ownership |
| [009](009-strictly-typed-comptime-and-annotations.md) | Accepted; clarified by ADR-011/012/013 | Typed comptime and annotations |
| [010](010-verified-region-teardown-and-callable-lifecycle.md) | Accepted; clarified by ADR-011/012/014/015; §4 consumed by proposed ADR-018 | Late lifecycle/teardown verification and execution authority |
| [011](011-resolved-semantic-identity-and-typed-elaboration.md) | Accepted; clarified by ADR-013/014; §6 consumed by proposed ADR-017/019 | Resolved semantic identity, typed elaboration, and no compiler magic |
| [012](012-verified-annotation-elaboration-and-callable-transforms.md) | Accepted; clarified by ADR-013/014/015; §5 cell amendment proposed 2026-07-27 (ADR-019 §5) | Two-stage annotation-elaboration seam and typed callable transforms |
| [013](013-incremental-semantic-queries-and-tracked-comptime.md) | Accepted | Incremental semantic queries and tracked comptime inputs |
| [014](014-closed-effects-and-static-capability-ownership.md) | Accepted; §8 amendment proposed 2026-07-27 | Closed effect algebra and static affine/linear capability ownership; function-type effect rows (§8) |
| [015](015-recovery-episodes-and-durable-obligation-journal.md) | Accepted; §10 amendment proposed 2026-07-27 | Cleanup-ordered retry episodes and durable uncertain-obligation ownership; obligation batches (§10) |
| [016](016-executable-public-feature-documentation.md) | Accepted; §10 amendment proposed 2026-07-27 | Complete executable Book coverage for every public feature; script tier and ceremony budgets (§10) |
| [017](017-ergonomic-parity-and-progressive-disclosure.md) | Proposed 2026-07-27 (pending ratification) | Sugar parity, script tier, quasiquote, structured fixes, supervisor scopes |
| [018](018-performance-charter-region-arenas-and-rc-elision.md) | Proposed 2026-07-27 (pending ratification) | Measured performance charter, deopt granularity, RC elision, BCE, region arenas |
| [019](019-polyglot-depth-and-foreign-toolchain-integration.md) | Proposed 2026-07-27 (pending ratification) | Compile-checked foreign bodies, zero-copy buffers, foreign refs, locked environments |

Where a dated implementation report conflicts with an accepted ADR, the ADR is
the authority. Historical reports remain evidence of shipped behavior and test
results; supersession notes identify which mechanisms must be replaced.
