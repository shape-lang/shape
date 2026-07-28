//! `NativeExecutionWitness` — machine-readable evidence that a named function
//! actually executed as native code (#117, ruling R15, ADR-016 §5).
//!
//! R15 forbids relabelling interpreter fallback as native execution. A native
//! claim must bind, per exact function:
//!
//! 1. the **verified artifact** — what was compiled (identity + digest);
//! 2. the **installation** event — native code was finalized and linked in;
//! 3. a **later native dispatch** event — the installed code actually RAN;
//! 4. **covered fallback / deoptimization** — if the function did not run
//!    native, the witness says so and why.
//!
//! (3) is the anti-vacuity core. Installation is a compile-time fact and can
//! be true of code that never executes; a witness that can be produced without
//! a native dispatch is exactly the interpreter-fallback lie R15 names. The
//! dispatch counter here is therefore incremented from *inside the emitted
//! native body* (a callback the JIT emits into each compiled function's entry
//! block — see `crates/shape-jit/src/compiler/witness_emit.rs`), never inferred
//! from "we compiled it and the program produced the right answer".
//!
//! R15 also requires the witness to come "from the runtime/JIT authority, not
//! parsed log prose". Nothing in this module reads a log line: installation is
//! recorded at the `get_finalized_function` site, fallback at each refusal
//! site, and dispatch from emitted code.
//!
//! ## Session state
//!
//! The session is **thread-local** and inert unless activated. The whole-program
//! JIT path compiles, installs, and executes on one thread, so a thread-local
//! session sees every event on that path. A dispatch that happened on another
//! thread is simply not counted — which *under*-reports nativity and can only
//! make a native claim fail, never succeed spuriously. That direction is the
//! safe one and is the reason this is not a process-global with atomics.
//!
//! ## Determinism
//!
//! The record carries no timing, address, duration, or thread field. Function
//! rows are sorted by `(function_identity, function_index)`. Serialization is
//! `serde_json` over ordered structs, so two runs of the same program over the
//! same binary produce byte-identical JSON.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bytecode::BytecodeProgram;

/// Witness schema major. Bumped when a consumer would have to change to keep
/// reading the record (R15 requires the witness schema version to be part of
/// every native claim).
pub const WITNESS_SCHEMA_VERSION: u32 = 1;

/// The identity used for the top-level / `main` compilation unit, which is not
/// a member of `BytecodeProgram::functions`.
pub const TOP_LEVEL_IDENTITY: &str = "__main__";

/// Which execution tier produced this witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WitnessMode {
    /// `--mode vm`: the bytecode interpreter. No native execution is possible,
    /// so no native claim can be made from a witness in this mode.
    Vm,
    /// `--mode jit`: whole-program ahead-of-time Cranelift compilation
    /// (`JITExecutor::execute_program`).
    JitWholeProgram,
}

/// Whether a fallback removed the whole program from native execution or only
/// one function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackScope {
    /// Every function in the program ran on the bytecode interpreter.
    Program,
    /// This function ran on the bytecode interpreter; others may be native.
    Function,
}

/// Stable machine-readable classification of *why* native execution did not
/// happen. Consumers match on this; `detail` carries the prose.
///
/// Every variant corresponds to a real refusal site. Adding a refusal site to
/// the JIT without adding its class here leaves the witness unable to explain a
/// fallback, which is itself a witness defect — `Unclassified` exists so that
/// gap is *visible* rather than silently rendered as "not reached".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackReasonClass {
    /// Ran under `--mode vm`. Not a defect; native execution was never asked for.
    ModeVm,
    /// REPL cross-cell persistence routes cells through the interpreter.
    ReplPersistence,
    /// Top-level `comptime { }` — deopt before compile to keep comptime
    /// side-effects exactly-once.
    TopLevelComptime,
    /// The source declares a `trait` or `impl` block (Wave-20A surface).
    UserTraitOrImpl,
    /// V2 typed-opcode verification failed (R8 W7 G.5).
    V2VerifierUnverified,
    /// Imported `pub const` inlined at use (R8 W8 Cluster A).
    ImportedConstInline,
    /// W17 marshal-return residual (R8 W9 B1).
    W17MarshalResidual,
    /// `?` operator residual (c4-4B).
    TryUnwrapResidual,
    /// Reference-escape `PromotedCell` deref (ADR-006 §2.7.30).
    ReferenceEscapePromotion,
    /// `??` null-coalesce residual.
    NullCoalesceResidual,
    /// Wave-17 scalar-move-lift exposed surface.
    ScalarMoveLift,
    /// The program registers a user `impl Drop for T` (R8 W9 B3).
    UserDropImpl,
    /// A function body accesses a module binding (W39 F1).
    ModuleBindingFunctionBody,
    /// A generic free function specialized on a struct type argument (WS-6).
    GenericStructSpecialization,
    /// Top-level (non-function) code contains a construct the JIT does not lower.
    MainCodeUnsupportedConstruct,
    /// Top-level MIR missing or failing MirToIR preflight.
    TopLevelMirPreflight,
    /// The Cranelift compiler could not be constructed.
    JitCompilerInit,
    /// Foreign-function linking for the JIT failed.
    ForeignLinkFailed,
    /// `compile_program_selective` returned an error.
    JitCompileError,
    /// `compile_program_selective` panicked.
    JitCompilePanic,
    /// A JIT-FFI return reached the host boundary without a stamped
    /// `NativeKind` (ADR-006 §2.7.5 kind-source gap).
    ReturnKindGap,
    /// The function's bytecode contains an opcode the JIT deliberately does not
    /// lower (`vm_only_opcode_reason`) — e.g. `CallForeign`, `as` casts.
    VmOnlyOpcode,
    /// The function calls a builtin the JIT translator does not lower.
    UnsupportedBuiltin,
    /// The function has neither a bytecode body nor MIR data to compile.
    NoCompilableBody,
    /// The function passed preflight but its Cranelift codegen failed; it was
    /// demoted to interpreted.
    FunctionCodegenFailed,
    /// A refusal site exists that this taxonomy does not name yet.
    Unclassified,
}

/// One covered fallback / deoptimization event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackRecord {
    pub scope: FallbackScope,
    pub reason_class: FallbackReasonClass,
    /// Human-readable detail from the refusal site. Never parsed by consumers —
    /// `reason_class` is the machine-readable half.
    pub detail: String,
}

/// What the runtime observed for one function. Derived, never asserted: see
/// `FunctionRecord::disposition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    /// Native code was installed AND a later native dispatch was observed.
    /// This is the only disposition that supports a native claim.
    NativeDispatched,
    /// Native code was installed but no native dispatch was observed — the
    /// function was compiled and never called. Not a native claim.
    InstalledNotDispatched,
    /// The function ran, on the bytecode interpreter.
    InterpreterFallback,
    /// The function was neither installed natively nor observed running.
    NotReached,
}

/// Per-function evidence: artifact, installation, dispatch, fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionWitness {
    /// The name a native claim cites. This is the string an ADR-016
    /// `native-execution` expectation puts in `function_identity`, so a Book
    /// manifest row can reference it without translation.
    pub function_identity: String,
    /// Position in `BytecodeProgram::functions`; `functions.len()` for the
    /// top-level unit. Present because identities are not guaranteed unique
    /// (generated closures share names).
    pub function_index: usize,
    /// SHA-256 over the exact compilation unit (name, arity, opcode/operand
    /// sequence). This is the "verified artifact" half of R15's binding: it
    /// changes when what was compiled changes.
    pub artifact_digest: String,
    /// Native code was finalized and entered the function table.
    pub native_installed: bool,
    /// Times the *emitted native body* announced entry. Not inferred.
    pub native_dispatches: u64,
    /// Times this function was dispatched to the bytecode interpreter from a
    /// native frame (the JIT trampoline).
    pub interpreter_dispatches: u64,
    pub disposition: Disposition,
    pub fallback: Option<FallbackRecord>,
}

/// The full record for one program execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeExecutionWitness {
    pub schema_version: u32,
    pub shape_version: String,
    pub mode: WitnessMode,
    /// Code generator that produced the native artifacts, or `none` under `vm`.
    pub backend: String,
    /// How native dispatch was observed. Recorded because it is part of what
    /// makes the claim checkable: `native-entry-callback` means the count came
    /// from inside the emitted body.
    pub instrumentation: String,
    /// SHA-256 over every unit digest, in index order.
    pub program_digest: String,
    pub function_count: usize,
    /// Set when the whole program was deoptimized to the interpreter.
    pub program_fallback: Option<FallbackRecord>,
    /// Sorted by `(function_identity, function_index)`.
    pub functions: Vec<FunctionWitness>,
    /// Identities carried by more than one unit. A native claim naming one of
    /// these is ambiguous and the assertion helpers refuse it.
    pub ambiguous_identities: Vec<String>,
}

impl NativeExecutionWitness {
    /// Canonical JSON. Deterministic: ordered structs, sorted rows, no timing.
    pub fn to_canonical_json(&self) -> String {
        // `expect` is sound: every field is a plain serde-derived struct with no
        // map keys that can fail to serialize.
        serde_json::to_string_pretty(self).expect("witness serialization is infallible")
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// All rows carrying `identity`. Empty when the identity is unknown.
    pub fn lookup(&self, identity: &str) -> Vec<&FunctionWitness> {
        self.functions
            .iter()
            .filter(|f| f.function_identity == identity)
            .collect()
    }
}

/// Why an assertion over a witness failed. Consumers render this; the variants
/// are distinct so a test can tell "no such function" from "it fell back".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessAssertion {
    /// No unit in the witness carries this identity.
    UnknownFunction { identity: String },
    /// More than one unit carries this identity; the claim cannot name one.
    AmbiguousFunction { identity: String, count: usize },
    /// The whole program deoptimized, so no function ran native.
    ProgramFellBack { record: FallbackRecord },
    /// Native code was never installed for this function.
    NotInstalled {
        identity: String,
        disposition: Disposition,
        fallback: Option<FallbackRecord>,
    },
    /// Installed, but the emitted body never announced entry. This is the
    /// vacuity guard: compiled is not executed.
    InstalledButNeverDispatched { identity: String },
    /// A fallback was expected but the function ran native.
    ExpectedFallbackButWasNative {
        identity: String,
        native_dispatches: u64,
    },
    /// A fallback was expected but none was recorded.
    ExpectedFallbackButNoneRecorded {
        identity: String,
        disposition: Disposition,
    },
    /// A fallback was recorded, with a different class than expected.
    FallbackClassMismatch {
        identity: String,
        expected: FallbackReasonClass,
        actual: FallbackReasonClass,
    },
}

impl std::fmt::Display for WitnessAssertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFunction { identity } => {
                write!(f, "no compilation unit named `{identity}` in the witness")
            }
            Self::AmbiguousFunction { identity, count } => write!(
                f,
                "`{identity}` names {count} compilation units; a native claim must \
                 identify exactly one"
            ),
            Self::ProgramFellBack { record } => write!(
                f,
                "the whole program deoptimized to the interpreter ({:?}): {}",
                record.reason_class, record.detail
            ),
            Self::NotInstalled {
                identity,
                disposition,
                fallback,
            } => match fallback {
                Some(r) => write!(
                    f,
                    "`{identity}` has no native installation ({disposition:?}); \
                     covered fallback {:?}: {}",
                    r.reason_class, r.detail
                ),
                None => write!(
                    f,
                    "`{identity}` has no native installation ({disposition:?}) and no \
                     recorded fallback reason"
                ),
            },
            Self::InstalledButNeverDispatched { identity } => write!(
                f,
                "`{identity}` was installed natively but its emitted body never ran; \
                 installation alone is not a native execution claim (R15)"
            ),
            Self::ExpectedFallbackButWasNative {
                identity,
                native_dispatches,
            } => write!(
                f,
                "expected `{identity}` to fall back, but it dispatched natively \
                 {native_dispatches} time(s)"
            ),
            Self::ExpectedFallbackButNoneRecorded {
                identity,
                disposition,
            } => write!(
                f,
                "expected a covered fallback for `{identity}`, but none was recorded \
                 ({disposition:?})"
            ),
            Self::FallbackClassMismatch {
                identity,
                expected,
                actual,
            } => write!(
                f,
                "`{identity}` fell back with {actual:?}, expected {expected:?}"
            ),
        }
    }
}

impl std::error::Error for WitnessAssertion {}

/// Resolve `identity` to exactly one unit, refusing unknown and ambiguous names.
fn resolve<'w>(
    witness: &'w NativeExecutionWitness,
    identity: &str,
) -> Result<&'w FunctionWitness, WitnessAssertion> {
    let matches = witness.lookup(identity);
    match matches.len() {
        0 => Err(WitnessAssertion::UnknownFunction {
            identity: identity.to_string(),
        }),
        1 => Ok(matches[0]),
        n => Err(WitnessAssertion::AmbiguousFunction {
            identity: identity.to_string(),
            count: n,
        }),
    }
}

/// The consumer entry point for a native claim (#187, #188, #146, #97).
///
/// Succeeds only when the named function was installed natively AND its emitted
/// body announced entry at least once. Installation without dispatch fails, a
/// whole-program deopt fails, and an ambiguous name fails.
pub fn assert_native_dispatch<'w>(
    witness: &'w NativeExecutionWitness,
    identity: &str,
) -> Result<&'w FunctionWitness, WitnessAssertion> {
    if let Some(record) = &witness.program_fallback {
        return Err(WitnessAssertion::ProgramFellBack {
            record: record.clone(),
        });
    }
    let unit = resolve(witness, identity)?;
    if !unit.native_installed {
        return Err(WitnessAssertion::NotInstalled {
            identity: identity.to_string(),
            disposition: unit.disposition,
            fallback: unit.fallback.clone(),
        });
    }
    if unit.native_dispatches == 0 {
        return Err(WitnessAssertion::InstalledButNeverDispatched {
            identity: identity.to_string(),
        });
    }
    Ok(unit)
}

/// The consumer entry point for a covered-fallback claim.
///
/// A fallback is a truthful record, not a missing witness: this succeeds when
/// the named function has a recorded fallback of the expected class, whether
/// the fallback was program-scoped or function-scoped.
pub fn assert_fallback<'w>(
    witness: &'w NativeExecutionWitness,
    identity: &str,
    expected: FallbackReasonClass,
) -> Result<&'w FunctionWitness, WitnessAssertion> {
    let unit = resolve(witness, identity)?;
    if unit.native_dispatches > 0 {
        return Err(WitnessAssertion::ExpectedFallbackButWasNative {
            identity: identity.to_string(),
            native_dispatches: unit.native_dispatches,
        });
    }
    let record = unit
        .fallback
        .as_ref()
        .or(witness.program_fallback.as_ref())
        .ok_or_else(|| WitnessAssertion::ExpectedFallbackButNoneRecorded {
            identity: identity.to_string(),
            disposition: unit.disposition,
        })?;
    if record.reason_class != expected {
        return Err(WitnessAssertion::FallbackClassMismatch {
            identity: identity.to_string(),
            expected,
            actual: record.reason_class,
        });
    }
    Ok(unit)
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

/// One unit the compiler was asked to produce native code for.
#[derive(Debug, Clone)]
struct FunctionRecord {
    identity: String,
    index: usize,
    artifact_digest: String,
    installed: bool,
    native_dispatches: u64,
    interpreter_dispatches: u64,
    fallback: Option<FallbackRecord>,
}

#[derive(Debug)]
struct SessionState {
    mode: WitnessMode,
    backend: String,
    units: Vec<FunctionRecord>,
    program_digest: String,
    program_fallback: Option<FallbackRecord>,
}

thread_local! {
    static SESSION: RefCell<Option<SessionState>> = const { RefCell::new(None) };
}

/// True when a witness session is collecting on this thread. The JIT reads this
/// to decide whether to emit the native-entry callback, so it must be set
/// before compilation starts.
pub fn is_active() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}

/// Start collecting. Discards any previous session on this thread.
pub fn activate(mode: WitnessMode) {
    let backend = match mode {
        WitnessMode::Vm => "none".to_string(),
        WitnessMode::JitWholeProgram => "cranelift".to_string(),
    };
    SESSION.with(|s| {
        *s.borrow_mut() = Some(SessionState {
            mode,
            backend,
            units: Vec::new(),
            program_digest: String::new(),
            program_fallback: None,
        });
    });
}

/// Stop collecting and drop any partial record.
pub fn deactivate() {
    SESSION.with(|s| *s.borrow_mut() = None);
}

fn digest_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// SHA-256 over the exact compilation unit: name, arity, and the opcode/operand
/// sequence. Two builds of the same source produce the same digest; a changed
/// body produces a different one.
fn unit_digest(name: &str, arity: u16, instructions: &[crate::bytecode::Instruction]) -> String {
    let mut buf = String::with_capacity(instructions.len() * 24 + name.len());
    buf.push_str(name);
    buf.push('\u{1}');
    buf.push_str(&arity.to_string());
    for instr in instructions {
        buf.push('\u{1}');
        buf.push_str(&format!("{:?}|{:?}", instr.opcode, instr.operand));
    }
    digest_hex(buf.as_bytes())
}

/// Register the program's compilation units, before compilation, with the exact
/// bytecode the tier is about to consume.
///
/// The top-level unit is appended at index `functions.len()` under
/// [`TOP_LEVEL_IDENTITY`], because it is not a member of `functions` but is a
/// compilation unit the JIT installs and dispatches like any other.
///
/// **Idempotent.** The `--mode jit` fall-through runs the same bytecode on the
/// interpreter, and the interpreter registers units too; re-registering would
/// wipe the installation, dispatch, and refusal records the JIT already made and
/// silently turn a fallback witness into an empty one. First registration wins.
pub fn begin_program(bytecode: &BytecodeProgram) {
    SESSION.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(state) = guard.as_mut() else {
            return;
        };
        if !state.units.is_empty() {
            return;
        }
        let mut units = Vec::with_capacity(bytecode.functions.len() + 1);
        for (index, func) in bytecode.functions.iter().enumerate() {
            let end = func.entry_point + func.body_length;
            let body = bytecode
                .instructions
                .get(func.entry_point..end)
                .unwrap_or(&[]);
            units.push(FunctionRecord {
                identity: func.name.clone(),
                index,
                artifact_digest: unit_digest(&func.name, func.arity, body),
                installed: false,
                native_dispatches: 0,
                interpreter_dispatches: 0,
                fallback: None,
            });
        }
        // The top-level unit's body is every instruction not owned by a
        // function; digesting the whole instruction stream is a superset that
        // is equally deterministic and avoids duplicating skip-range logic.
        units.push(FunctionRecord {
            identity: TOP_LEVEL_IDENTITY.to_string(),
            index: bytecode.functions.len(),
            artifact_digest: unit_digest(TOP_LEVEL_IDENTITY, 0, &bytecode.instructions),
            installed: false,
            native_dispatches: 0,
            interpreter_dispatches: 0,
            fallback: None,
        });

        let combined: String = units
            .iter()
            .map(|u| u.artifact_digest.as_str())
            .collect::<Vec<_>>()
            .join("\u{1}");
        state.program_digest = digest_hex(combined.as_bytes());
        state.units = units;
    });
}

/// The index the top-level unit occupies, if a program has been registered.
pub fn top_level_index() -> Option<usize> {
    SESSION.with(|s| {
        s.borrow()
            .as_ref()
            .and_then(|state| state.units.last().map(|u| u.index))
    })
}

fn with_unit<F: FnOnce(&mut FunctionRecord)>(index: usize, f: F) {
    SESSION.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(state) = guard.as_mut() else {
            return;
        };
        if let Some(unit) = state.units.iter_mut().find(|u| u.index == index) {
            f(unit);
        }
    });
}

/// Record that native code for `index` was finalized and linked into the
/// function table. Called from the installation site, not inferred.
pub fn record_installation(index: usize) {
    with_unit(index, |unit| unit.installed = true);
}

/// Record that `index` did not get native code, and why.
pub fn record_function_fallback(
    index: usize,
    class: FallbackReasonClass,
    detail: impl Into<String>,
) {
    with_unit(index, |unit| {
        // First reason wins: preflight refusal is the cause, a later codegen
        // demotion of the same unit is a consequence.
        if unit.fallback.is_none() {
            unit.fallback = Some(FallbackRecord {
                scope: FallbackScope::Function,
                reason_class: class,
                detail: detail.into(),
            });
        }
    });
}

/// Record that the whole program deoptimized to the interpreter, and why.
pub fn record_program_fallback(class: FallbackReasonClass, detail: impl Into<String>) {
    SESSION.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(state) = guard.as_mut() else {
            return;
        };
        if state.program_fallback.is_none() {
            state.program_fallback = Some(FallbackRecord {
                scope: FallbackScope::Program,
                reason_class: class,
                detail: detail.into(),
            });
        }
    });
}

/// Announce that the emitted native body for `index` is executing.
///
/// This is the one call that turns "installed" into "dispatched". It is invoked
/// from inside JIT-emitted machine code (see the shape-jit `witness_emit`
/// module); nothing else may call it, because nothing else can honestly claim
/// the native body ran.
pub fn record_native_dispatch(index: usize) {
    with_unit(index, |unit| {
        unit.native_dispatches = unit.native_dispatches.saturating_add(1);
    });
}

/// Record that `index` was dispatched to the bytecode interpreter from a native
/// frame (the JIT's trampoline VM).
pub fn record_interpreter_dispatch(index: usize) {
    with_unit(index, |unit| {
        unit.interpreter_dispatches = unit.interpreter_dispatches.saturating_add(1);
    });
}

/// Close the session and produce the record. Returns `None` when no session was
/// active.
///
/// Dispositions are **derived here**, never asserted by a caller:
/// `NativeDispatched` requires installation and a nonzero dispatch count, so no
/// sequence of recording calls short of a real native entry can produce it.
pub fn finish() -> Option<NativeExecutionWitness> {
    let state = SESSION.with(|s| s.borrow_mut().take())?;

    let mut functions: Vec<FunctionWitness> = state
        .units
        .iter()
        .map(|unit| {
            // #188: interpreter dispatches outrank installation. A function
            // can be compiled and installed and still have every call routed
            // to the interpreter — a closure reached only through
            // `jit_call_value`'s trampoline is exactly that shape. Ordering
            // `installed` ahead of `interpreter_dispatches > 0` labelled such
            // a function `installed-not-dispatched`, which reads as "never
            // called" while it was called 200 times, none of them native.
            // The ordering below can only ever move a unit AWAY from a native
            // reading, so it cannot manufacture an R15 claim: the
            // `NativeDispatched` arm is unchanged and still requires
            // installation plus a nonzero native count.
            let disposition = if unit.installed && unit.native_dispatches > 0 {
                Disposition::NativeDispatched
            } else if unit.interpreter_dispatches > 0 {
                Disposition::InterpreterFallback
            } else if unit.installed {
                Disposition::InstalledNotDispatched
            } else {
                Disposition::NotReached
            };
            FunctionWitness {
                function_identity: unit.identity.clone(),
                function_index: unit.index,
                artifact_digest: unit.artifact_digest.clone(),
                native_installed: unit.installed,
                native_dispatches: unit.native_dispatches,
                interpreter_dispatches: unit.interpreter_dispatches,
                disposition,
                fallback: unit.fallback.clone(),
            }
        })
        .collect();
    functions.sort_by(|a, b| {
        a.function_identity
            .cmp(&b.function_identity)
            .then(a.function_index.cmp(&b.function_index))
    });

    let mut ambiguous_identities: Vec<String> = Vec::new();
    for window in functions.windows(2) {
        if window[0].function_identity == window[1].function_identity
            && ambiguous_identities.last() != Some(&window[0].function_identity)
        {
            ambiguous_identities.push(window[0].function_identity.clone());
        }
    }

    Some(NativeExecutionWitness {
        schema_version: WITNESS_SCHEMA_VERSION,
        shape_version: env!("CARGO_PKG_VERSION").to_string(),
        mode: state.mode,
        backend: state.backend,
        instrumentation: match state.mode {
            WitnessMode::Vm => "none".to_string(),
            WitnessMode::JitWholeProgram => "native-entry-callback".to_string(),
        },
        program_digest: state.program_digest,
        function_count: functions.len(),
        program_fallback: state.program_fallback,
        functions,
        ambiguous_identities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a witness directly from recorded events, the way a real run does.
    fn record_two_units(f: impl FnOnce()) -> NativeExecutionWitness {
        activate(WitnessMode::JitWholeProgram);
        SESSION.with(|s| {
            let mut guard = s.borrow_mut();
            let state = guard.as_mut().unwrap();
            state.program_digest = "d0".to_string();
            state.units = vec![
                FunctionRecord {
                    identity: "hot".to_string(),
                    index: 0,
                    artifact_digest: "a0".to_string(),
                    installed: false,
                    native_dispatches: 0,
                    interpreter_dispatches: 0,
                    fallback: None,
                },
                FunctionRecord {
                    identity: "cold".to_string(),
                    index: 1,
                    artifact_digest: "a1".to_string(),
                    installed: false,
                    native_dispatches: 0,
                    interpreter_dispatches: 0,
                    fallback: None,
                },
            ];
        });
        f();
        finish().unwrap()
    }

    #[test]
    fn installation_without_dispatch_is_not_a_native_claim() {
        // The anti-vacuity core: everything a compiler can know at compile time
        // is recorded, and the claim still fails.
        let witness = record_two_units(|| {
            record_installation(0);
        });
        let unit = &witness.lookup("hot")[0];
        assert!(unit.native_installed);
        assert_eq!(unit.disposition, Disposition::InstalledNotDispatched);
        assert_eq!(
            assert_native_dispatch(&witness, "hot"),
            Err(WitnessAssertion::InstalledButNeverDispatched {
                identity: "hot".to_string()
            })
        );
    }

    #[test]
    fn dispatch_after_installation_is_a_native_claim() {
        let witness = record_two_units(|| {
            record_installation(0);
            record_native_dispatch(0);
        });
        let unit = assert_native_dispatch(&witness, "hot").expect("native claim holds");
        assert_eq!(unit.disposition, Disposition::NativeDispatched);
        assert_eq!(unit.native_dispatches, 1);
    }

    #[test]
    fn a_fallback_function_can_never_produce_a_native_claim() {
        let witness = record_two_units(|| {
            record_installation(0);
            record_native_dispatch(0);
            record_function_fallback(1, FallbackReasonClass::VmOnlyOpcode, "ConvertToInt");
            record_interpreter_dispatch(1);
        });
        let err = assert_native_dispatch(&witness, "cold").unwrap_err();
        assert!(
            matches!(err, WitnessAssertion::NotInstalled { .. }),
            "expected NotInstalled, got {err:?}"
        );
        // And the truthful record IS available for the same function.
        let unit = assert_fallback(&witness, "cold", FallbackReasonClass::VmOnlyOpcode)
            .expect("fallback claim holds");
        assert_eq!(unit.disposition, Disposition::InterpreterFallback);
        assert_eq!(unit.interpreter_dispatches, 1);
    }

    #[test]
    fn assert_fallback_refuses_a_natively_dispatched_function() {
        // The non-vacuity control for the fallback helper: it must not accept a
        // function that actually ran native.
        let witness = record_two_units(|| {
            record_installation(0);
            record_native_dispatch(0);
        });
        assert_eq!(
            assert_fallback(&witness, "hot", FallbackReasonClass::VmOnlyOpcode),
            Err(WitnessAssertion::ExpectedFallbackButWasNative {
                identity: "hot".to_string(),
                native_dispatches: 1,
            })
        );
    }

    #[test]
    fn fallback_class_mismatch_is_reported() {
        let witness = record_two_units(|| {
            record_function_fallback(1, FallbackReasonClass::VmOnlyOpcode, "ConvertToInt");
        });
        assert_eq!(
            assert_fallback(&witness, "cold", FallbackReasonClass::UserDropImpl),
            Err(WitnessAssertion::FallbackClassMismatch {
                identity: "cold".to_string(),
                expected: FallbackReasonClass::UserDropImpl,
                actual: FallbackReasonClass::VmOnlyOpcode,
            })
        );
    }

    #[test]
    fn whole_program_fallback_denies_every_native_claim() {
        let witness = record_two_units(|| {
            // Even with an installation and a dispatch recorded, a program-scoped
            // deopt means no native claim survives.
            record_installation(0);
            record_native_dispatch(0);
            record_program_fallback(FallbackReasonClass::UserDropImpl, "impl Drop for T");
        });
        assert!(matches!(
            assert_native_dispatch(&witness, "hot"),
            Err(WitnessAssertion::ProgramFellBack { .. })
        ));
        // Program-scoped fallback answers for a function with no own record.
        assert_fallback(&witness, "cold", FallbackReasonClass::UserDropImpl)
            .expect("program fallback covers the function");
    }

    #[test]
    fn unknown_and_ambiguous_identities_are_refused() {
        let witness = record_two_units(|| {});
        assert_eq!(
            assert_native_dispatch(&witness, "nope"),
            Err(WitnessAssertion::UnknownFunction {
                identity: "nope".to_string()
            })
        );

        activate(WitnessMode::JitWholeProgram);
        SESSION.with(|s| {
            let mut guard = s.borrow_mut();
            let state = guard.as_mut().unwrap();
            state.units = vec![
                FunctionRecord {
                    identity: "__closure_0".to_string(),
                    index: 0,
                    artifact_digest: "a".to_string(),
                    installed: true,
                    native_dispatches: 3,
                    interpreter_dispatches: 0,
                    fallback: None,
                },
                FunctionRecord {
                    identity: "__closure_0".to_string(),
                    index: 1,
                    artifact_digest: "b".to_string(),
                    installed: true,
                    native_dispatches: 4,
                    interpreter_dispatches: 0,
                    fallback: None,
                },
            ];
        });
        let dup = finish().unwrap();
        assert_eq!(dup.ambiguous_identities, vec!["__closure_0".to_string()]);
        assert_eq!(
            assert_native_dispatch(&dup, "__closure_0"),
            Err(WitnessAssertion::AmbiguousFunction {
                identity: "__closure_0".to_string(),
                count: 2,
            })
        );
    }

    #[test]
    fn re_registering_a_program_cannot_erase_recorded_evidence() {
        // The `--mode jit` fall-through re-enters the interpreter with the same
        // bytecode. If registration were not idempotent, the JIT's refusal
        // reason and any recorded dispatch would vanish and the witness would
        // read as "nothing happened" instead of "it fell back, for this reason".
        let bytecode = BytecodeProgram::default();
        let witness = record_two_units(|| {
            record_installation(0);
            record_native_dispatch(0);
            record_function_fallback(1, FallbackReasonClass::VmOnlyOpcode, "ConvertToInt");
            begin_program(&bytecode);
        });
        assert_eq!(
            witness.functions.len(),
            2,
            "units must not be re-registered"
        );
        assert_native_dispatch(&witness, "hot").expect("dispatch record survives");
        assert_fallback(&witness, "cold", FallbackReasonClass::VmOnlyOpcode)
            .expect("fallback record survives");
    }

    #[test]
    fn inactive_session_records_nothing_and_finishes_none() {
        deactivate();
        assert!(!is_active());
        record_installation(0);
        record_native_dispatch(0);
        record_program_fallback(FallbackReasonClass::ModeVm, "inert");
        assert_eq!(finish(), None);
    }

    #[test]
    fn serialization_is_stable_and_round_trips() {
        let build = || {
            record_two_units(|| {
                record_installation(0);
                record_native_dispatch(0);
                record_native_dispatch(0);
                record_function_fallback(1, FallbackReasonClass::VmOnlyOpcode, "ConvertToInt");
                record_interpreter_dispatch(1);
            })
        };
        let first = build().to_canonical_json();
        let second = build().to_canonical_json();
        assert_eq!(first, second, "witness JSON must be byte-identical");
        assert_eq!(
            NativeExecutionWitness::from_json(&first).unwrap(),
            build(),
            "witness must round-trip through JSON"
        );
        // Determinism guard: no timing or address fields may leak in.
        for banned in ["elapsed", "duration", "_ms", "timestamp", "address", "ptr"] {
            assert!(
                !first.contains(banned),
                "witness JSON must not carry `{banned}` (nondeterministic)"
            );
        }
    }

    #[test]
    fn rows_are_sorted_by_identity_then_index() {
        activate(WitnessMode::JitWholeProgram);
        SESSION.with(|s| {
            let mut guard = s.borrow_mut();
            guard.as_mut().unwrap().units = vec!["zeta", "alpha", "mid"]
                .into_iter()
                .enumerate()
                .map(|(index, identity)| FunctionRecord {
                    identity: identity.to_string(),
                    index,
                    artifact_digest: String::new(),
                    installed: false,
                    native_dispatches: 0,
                    interpreter_dispatches: 0,
                    fallback: None,
                })
                .collect();
        });
        let witness = finish().unwrap();
        let order: Vec<&str> = witness
            .functions
            .iter()
            .map(|f| f.function_identity.as_str())
            .collect();
        assert_eq!(order, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn unit_digest_tracks_the_compiled_body() {
        use crate::bytecode::{Instruction, OpCode, Operand};
        let body_a = vec![Instruction {
            opcode: OpCode::Return,
            operand: None,
        }];
        let body_b = vec![Instruction {
            opcode: OpCode::Return,
            operand: Some(Operand::Count(1)),
        }];
        assert_eq!(
            unit_digest("f", 0, &body_a),
            unit_digest("f", 0, &body_a),
            "the digest must be stable for the same unit"
        );
        assert_ne!(
            unit_digest("f", 0, &body_a),
            unit_digest("f", 0, &body_b),
            "a changed body must change the artifact digest"
        );
        assert_ne!(
            unit_digest("f", 0, &body_a),
            unit_digest("g", 0, &body_a),
            "a different function must have a different artifact digest"
        );
    }
}
