//! Shared background async runtime for real concurrent task execution.
//!
//! # WF-2D real concurrency (Decision D1, 2026-07-05)
//!
//! The Shape VM interpreter is single-threaded and `!Sync`: Shape bytecode
//! for a task cannot be split across OS threads, and the interpreter cannot
//! suspend a mid-flight frame (coroutine-style suspension is the deferred
//! "Phase-2c snapshot-tier" work referenced throughout ADR-006 §2.7.4).
//!
//! Genuine wall-clock overlap is therefore only achievable at the point where
//! Shape work actually *waits*: an async **module** function (e.g.
//! `time::sleep`) whose body is a self-contained `Send + 'static` tokio
//! future (`register_typed_async_function` in `shape-runtime`). Those futures
//! do not borrow the VM (the `&ModuleContext` borrow "cannot cross await
//! points" — see `marshal.rs::VariadicTypedAsyncBody`), so they can be spawned
//! onto a background multi-threaded runtime and made progress concurrently
//! while the interpreter thread does other work or blocks on a completion.
//!
//! This module owns that background runtime. It is a process-global,
//! lazily-initialized multi-threaded tokio runtime. Async module futures are
//! `spawn`ed onto it (see `vm_impl/modules.rs::spawn_async_module_future`);
//! the interpreter thread collects results through a plain
//! `std::sync::mpsc` channel (blocking `recv` for `await`, non-blocking
//! `try_recv` for the `race`/`any` first-completion poll).
//!
//! ## Why a dedicated runtime rather than the ambient one
//!
//! The CLI entrypoint is `#[tokio::main(flavor = "current_thread")]`. Spawning
//! onto the ambient current-thread runtime would not overlap (a single worker
//! thread), and blocking on it (`block_on`) from inside the ambient runtime
//! panics ("Cannot start a runtime from within a runtime"). A separate
//! multi-threaded runtime sidesteps both: `spawn` merely enqueues onto its own
//! worker pool (never blocks, never nests), and completion is delivered over a
//! non-tokio `std::sync::mpsc` channel so the interpreter thread's blocking
//! `recv` never touches the ambient runtime's reactor.

use std::sync::OnceLock;
use tokio::runtime::Runtime;

use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
use shape_value::{NativeKind, VMError};

static SHARED_ASYNC_RT: OnceLock<Runtime> = OnceLock::new();

/// Return the process-global background async runtime, building it on first
/// use.
///
/// Multi-threaded with `enable_all` (timers + IO drivers) so `time::sleep`
/// and future IO-bearing async module functions make real concurrent
/// progress on the worker pool.
pub(crate) fn shared_runtime() -> &'static Runtime {
    SHARED_ASYNC_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("shape-async-worker")
            .build()
            .expect("failed to build shared Shape async runtime")
    })
}

/// WF-2D-fu real concurrency for user-defined async functions.
///
/// A user `async fn` call runs its whole body synchronously on the calling
/// interpreter thread (a bare async-fn call is *eager* — see the print-order
/// probe `1,50,150,2,3` in the WF-2D-fu diagnosis), and its inner
/// `await time::sleep(..)` blocks that thread on the module future's
/// completion channel. Two such calls launched back-to-back on one thread
/// therefore serialize (~2s for two 1s sleeps). The single-threaded `!Sync`
/// interpreter cannot suspend a mid-flight frame (the deferred Phase-2c
/// snapshot-tier, ADR-006 §2.7.4), so the only way two user async-fn bodies
/// overlap is to run each on its OWN thread.
///
/// `run_isolated_async_fn` runs one spawned async-fn task to completion on a
/// FRESH, fully-isolated `VirtualMachine` — the same isolation model the
/// distributed `run_remote_call` path already relies on. Because the fresh VM
/// is built AND consumed entirely inside the `spawn_blocking` closure (it
/// never crosses a thread boundary as a value), the VM's `!Send` raw-pointer
/// fields are irrelevant. `VirtualMachine::new` already registers every
/// stdlib module (`init.rs` — `time`, `math`, …), so `time::sleep` resolves
/// inside the isolated VM; its inner `await` blocks THIS blocking-pool worker
/// thread while the sleep future makes progress on the shared runtime's async
/// worker pool. Two `spawn_blocking` tasks → two blocking workers → the two
/// sleeps overlap → ~1s wall-clock.
///
/// ## Isolation contract (sound, but scoped)
///
/// - The task receives NO shared heap: only the (deep-cloned) immutable
///   `BytecodeProgram` and the parent's permission envelope cross in; only a
///   `TypedReturn` (owned, `Send`) crosses back. No `Arc<HeapValue>` is shared
///   across threads, so there is no cross-thread refcount or GC hazard.
/// - Module-binding initializers are NOT re-run in the isolated VM (running
///   top-level code would re-execute the whole program). A deferred async task
///   therefore sees module globals in their default (uninitialised) state.
///   The compiler only defers **zero-argument** user async-fn calls
///   (`compile_async_let`), which in practice are self-contained wrappers over
///   stdlib async work. A deferred body that reads a module global is the
///   documented boundary of this lane; arg-bearing async-fn calls keep the
///   pre-existing eager (serial) path.
/// - Only leaf scalar returns (`int` / `number` / `bool` / unit) are marshaled
///   back. A heap-typed return surfaces `VMError::NotImplemented` — NOT a
///   Bool-default — so an unsupported shape fails loudly per §Forbidden.
pub(crate) fn run_isolated_async_fn(
    program: crate::bytecode::BytecodeProgram,
    config: crate::executor::VMConfig,
    granted: Option<shape_abi_v1::PermissionSet>,
    scope: Option<shape_abi_v1::ScopeConstraints>,
    language_runtimes: std::collections::HashMap<
        String,
        std::sync::Arc<shape_runtime::plugins::language_runtime::PluginLanguageRuntime>,
    >,
    func_id: u16,
) -> Result<TypedReturn, String> {
    let mut vm = crate::executor::VirtualMachine::new(config);
    vm.set_permissions(granted, scope);
    // ADR-019 §5 / #202. The isolated task VM is built from scratch, so without
    // this it has an EMPTY language-runtime registry and any `fn python` inside
    // a spawned user `async fn` fails link-now with "no extension provides
    // language 'python'" — a live bug, and a confusing one, because the same
    // call works from the parent VM. The registry is `Arc`-shared extension
    // handles, not VM state: sharing it crosses no heap value and no `!Send`
    // field, and the extension instances behind it are the same ones the parent
    // uses, which is what makes a foreign module's state consistent across the
    // boundary.
    vm.set_language_runtimes(language_runtimes);
    vm.load_program(program);
    // Populate the stdlib module-binding objects (the `time` / `math` / …
    // TypedObjects whose fields are `ModuleFn` references) WITHOUT running the
    // program's top-level code — the same isolated-load sequence the
    // distributed `run_remote_call` path uses (remote.rs). Without this, a
    // `time::sleep` call inside the task body loads an uninitialised module
    // binding (`Null`) and fails the value-call dispatch.
    vm.populate_module_objects();
    // #202: bind module-scope slots that name a top-level function, so a call
    // reaching one through `LoadModuleBinding` + `CallValue` — which is how a
    // foreign stub is reached — finds a callable instead of an uninitialised
    // slot. Bindings initialised by top-level expressions stay uninitialised;
    // that boundary is unchanged.
    vm.seed_module_function_bindings();
    let result = vm
        .execute_function_by_id_at_host_boundary(func_id, Vec::new(), None)
        .map_err(|e| e.to_string())?;
    let bits = result.raw();
    let kind = result.kind();
    // The scalar payload is owned by value; no heap share leaves the task.
    kinded_scalar_to_typed_return(bits, kind).map_err(|e| e.to_string())
}

/// Marshal a leaf-scalar `KindedSlot` result out of the isolated task VM into
/// an owned, `Send` `TypedReturn`. Surface-and-stop on any heap kind (no
/// Bool-default, no tag decode) per ADR-006 §Forbidden.
fn kinded_scalar_to_typed_return(bits: u64, kind: NativeKind) -> Result<TypedReturn, VMError> {
    let concrete = match kind {
        NativeKind::Int64 => ConcreteReturn::I64(bits as i64),
        NativeKind::Float64 => ConcreteReturn::F64(f64::from_bits(bits)),
        NativeKind::Bool => ConcreteReturn::Bool(bits != 0),
        NativeKind::Null => ConcreteReturn::Unit,
        other => {
            return Err(VMError::NotImplemented(format!(
                "WF-2D-fu isolated async task returned a non-scalar result kind \
                 {:?}; concurrent user async-fn tasks currently marshal only \
                 leaf scalars (int/number/bool/unit) across the isolation \
                 boundary. Return a scalar or await the call eagerly.",
                other
            )));
        }
    };
    Ok(TypedReturn::Concrete(concrete))
}

#[cfg(test)]
mod isolated_task_registry_tests {
    //! #202 work item 6 — the isolated task VM inherits the parent's
    //! language-runtime registry.
    //!
    //! MEASURED before the fix: a foreign call inside a spawned user `async fn`
    //! failed link-now with `no extension provides language 'python'`, because
    //! `run_isolated_async_fn` built its VM with `VirtualMachine::new` and never
    //! set the registry. The same call from the parent VM worked, which is what
    //! made it confusing rather than merely broken.
    //!
    //! The test drives the foreign STUB function directly as the spawned task,
    //! because the stub's own body is where `CallForeign` lives — that puts
    //! link-now on the isolated VM's execution path without depending on how a
    //! caller reaches the stub.

    use super::*;
    use crate::compiler::BytecodeCompiler;
    use shape_runtime::plugins::language_runtime::PluginLanguageRuntime;

    const ZERO_ARG_FOREIGN: &str = r#"
fn python seven() -> Result<int> {
    return 7
}
"#;

    fn compiled() -> (crate::bytecode::BytecodeProgram, u16) {
        let program = shape_ast::parser::parse_program(ZERO_ARG_FOREIGN).expect("parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let func_id = bytecode
            .functions
            .iter()
            .position(|f| f.name == "seven")
            .expect("the foreign stub is registered as a function") as u16;
        (bytecode, func_id)
    }

    fn run_with(
        runtimes: std::collections::HashMap<String, std::sync::Arc<PluginLanguageRuntime>>,
    ) -> String {
        let (bytecode, func_id) = compiled();
        match run_isolated_async_fn(
            bytecode,
            crate::executor::VMConfig::default(),
            None,
            None,
            runtimes,
            func_id,
        ) {
            Ok(_) => String::new(),
            Err(e) => e,
        }
    }

    /// The regression: with an EMPTY registry the isolated VM cannot resolve the
    /// language, and says so. This is the shape of the bug — it is asserted so
    /// the next test's passing is meaningful rather than vacuous.
    #[test]
    fn an_empty_registry_is_what_the_bug_looked_like() {
        let error = run_with(std::collections::HashMap::new());
        assert!(
            error.contains("no extension provides language"),
            "without a registry the isolated VM must fail at link-now, got: {error}"
        );
    }

    /// The fix: the parent's registry crosses in, so link-now resolves the
    /// language and the call proceeds past it.
    #[test]
    fn the_parent_registry_crosses_into_the_isolated_task_vm() {
        let vtable = &crate::executor::foreign_async::tests::sleepy_extension::SHARED;
        let runtime = PluginLanguageRuntime::new(vtable, &serde_json::Value::Null)
            .expect("fake runtime initializes");
        let mut runtimes = std::collections::HashMap::new();
        runtimes.insert("python".to_string(), std::sync::Arc::new(runtime));

        let error = run_with(runtimes);
        assert!(
            !error.contains("no extension provides language"),
            "the isolated VM must resolve the language through the inherited \
             registry, got: {error}"
        );
    }
}
