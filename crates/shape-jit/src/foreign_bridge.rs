use shape_runtime::context::ExecutionContext;
use shape_runtime::module_exports::RawCallableInvoker;
use shape_runtime::plugins::language_runtime::{CompiledForeignFunction, PluginLanguageRuntime};
use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::{ValueWord, ValueWordExt};
use shape_vm::bytecode::BytecodeProgram;
use shape_vm::executor::{
    foreign_marshal,
    native_abi::{self, NativeLinkedFunction},
};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) enum LinkedForeignHandle {
    Runtime {
        runtime: Arc<PluginLanguageRuntime>,
        compiled: CompiledForeignFunction,
    },
    Native(Arc<NativeLinkedFunction>),
}

pub(crate) struct LinkedForeignEntry {
    pub(crate) name: String,
    pub(crate) return_type: Option<String>,
    pub(crate) return_type_schema_id: Option<u32>,
    pub(crate) dynamic_errors: bool,
    pub(crate) handle: LinkedForeignHandle,
}

pub(crate) struct JitForeignBridgeState {
    pub(crate) entries: Vec<LinkedForeignEntry>,
    pub(crate) schemas: Arc<TypeSchemaRegistry>,
}

impl Drop for JitForeignBridgeState {
    fn drop(&mut self) {
        for entry in &self.entries {
            if let LinkedForeignHandle::Runtime { runtime, compiled } = &entry.handle {
                runtime.dispose_function(compiled);
            }
        }
    }
}

pub(crate) fn link_foreign_functions_for_jit(
    program: &BytecodeProgram,
    exec_ctx: Option<&ExecutionContext>,
) -> Result<Option<Box<JitForeignBridgeState>>, String> {
    if program.foreign_functions.is_empty() {
        return Ok(None);
    }

    let mut entries = Vec::with_capacity(program.foreign_functions.len());
    let mut native_library_cache: HashMap<String, Arc<libloading::Library>> = HashMap::new();

    for entry in &program.foreign_functions {
        if let Some(native_spec) = &entry.native_abi {
            let linked = native_abi::link_native_function(
                native_spec,
                &program.native_struct_layouts,
                &mut native_library_cache,
            )
            .map_err(|e| format!("Failed to link native function '{}': {}", entry.name, e))?;

            entries.push(LinkedForeignEntry {
                name: entry.name.clone(),
                return_type: entry.return_type.clone(),
                return_type_schema_id: entry.return_type_schema_id,
                dynamic_errors: false,
                handle: LinkedForeignHandle::Native(Arc::new(linked)),
            });
            continue;
        }

        let Some(ctx) = exec_ctx else {
            return Err(format!(
                "No runtime context available to link foreign function '{}'",
                entry.name
            ));
        };

        let Some(runtime) = ctx.get_language_runtime(&entry.language) else {
            return Err(format!(
                "No language runtime registered for '{}'. Install the {} extension to use `fn {} ...` blocks.",
                entry.language, entry.language, entry.language
            ));
        };

        let dynamic_errors = runtime.has_dynamic_errors();
        let compiled = runtime
            .compile(
                &entry.name,
                &entry.body_text,
                &entry.param_names,
                &entry.param_types,
                entry.return_type.as_deref(),
                entry.is_async,
            )
            .map_err(|e| e.to_string())?;

        entries.push(LinkedForeignEntry {
            name: entry.name.clone(),
            return_type: entry.return_type.clone(),
            return_type_schema_id: entry.return_type_schema_id,
            dynamic_errors,
            handle: LinkedForeignHandle::Runtime { runtime, compiled },
        });
    }

    Ok(Some(Box::new(JitForeignBridgeState {
        entries,
        schemas: Arc::new(program.type_schema_registry.clone()),
    })))
}

impl JitForeignBridgeState {
    fn entry(&self, foreign_idx: usize) -> Result<&LinkedForeignEntry, String> {
        self.entries.get(foreign_idx).ok_or_else(|| {
            format!(
                "Foreign function index {} out of bounds ({} entries)",
                foreign_idx,
                self.entries.len()
            )
        })
    }

    fn invoke_runtime_entry(
        &self,
        entry: &LinkedForeignEntry,
        args: &[ValueWord],
    ) -> Result<ValueWord, String> {
        let LinkedForeignHandle::Runtime { runtime, compiled } = &entry.handle else {
            return Err(format!(
                "Foreign function '{}' is not a runtime foreign function",
                entry.name
            ));
        };

        let args_msgpack = foreign_marshal::marshal_args(args, &self.schemas)
            .map_err(|e| format!("Foreign function '{}': {}", entry.name, e))?;

        let return_type = entry.return_type.as_ref().ok_or_else(|| {
            format!(
                "Foreign function '{}' is missing an explicit return type",
                entry.name
            )
        })?;

        if entry.dynamic_errors {
            match runtime.invoke(compiled, &args_msgpack) {
                Ok(result_msgpack) => match foreign_marshal::unmarshal_result(
                    &result_msgpack,
                    return_type,
                    entry.return_type_schema_id,
                    &self.schemas,
                ) {
                    Ok(value) => Ok(ValueWord::from_ok(value)),
                    Err(err) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
                        format!("Foreign function '{}': {}", entry.name, err),
                    )))),
                },
                Err(err) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
                    format!("Foreign function '{}': {}", entry.name, err),
                )))),
            }
        } else {
            let result_msgpack = runtime
                .invoke(compiled, &args_msgpack)
                .map_err(|e| format!("Foreign function '{}' error: {}", entry.name, e))?;
            foreign_marshal::unmarshal_result(
                &result_msgpack,
                return_type,
                entry.return_type_schema_id,
                &self.schemas,
            )
            .map_err(|e| format!("Foreign function '{}': {}", entry.name, e))
        }
    }

    fn invoke_native_entry(
        &self,
        entry: &LinkedForeignEntry,
        args: &[ValueWord],
        raw_invoker: Option<RawCallableInvoker>,
    ) -> Result<ValueWord, String> {
        let LinkedForeignHandle::Native(linked) = &entry.handle else {
            return Err(format!(
                "Foreign function '{}' is not a native ABI foreign function",
                entry.name
            ));
        };
        native_abi::invoke_linked_function(linked, args, raw_invoker, None)
            .map_err(|e| format!("Native function '{}' error: {}", entry.name, e))
    }

    pub(crate) fn invoke_dynamic(
        &self,
        foreign_idx: usize,
        args: &[ValueWord],
    ) -> Result<ValueWord, String> {
        let entry = self.entry(foreign_idx)?;
        self.invoke_runtime_entry(entry, args)
    }

    pub(crate) fn invoke_native(
        &self,
        foreign_idx: usize,
        args: &[ValueWord],
        raw_invoker: Option<RawCallableInvoker>,
    ) -> Result<ValueWord, String> {
        let entry = self.entry(foreign_idx)?;
        self.invoke_native_entry(entry, args, raw_invoker)
    }

    pub(crate) fn invoke(
        &self,
        foreign_idx: usize,
        args: &[ValueWord],
        raw_invoker: Option<RawCallableInvoker>,
    ) -> Result<ValueWord, String> {
        let entry = self.entry(foreign_idx)?;

        match &entry.handle {
            LinkedForeignHandle::Runtime { .. } => self.invoke_runtime_entry(entry, args),
            LinkedForeignHandle::Native(_) => self.invoke_native_entry(entry, args, raw_invoker),
        }
    }
}
