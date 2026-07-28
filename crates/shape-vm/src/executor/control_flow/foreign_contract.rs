//! Building the declared Shape contract delivered through the extension stub
//! channel.
//!
//! ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196).
//!
//! `register_types` was designed as the stub-exchange channel and had no host
//! caller: the extensions' implementations were no-ops and nothing ever
//! produced a payload for them. This module produces the payload, from the same
//! `ForeignFunctionEntry` table the VM links against and the same schema
//! registry the marshal layer reads, so a stub cannot describe a contract the
//! program does not have.
//!
//! The division of labour is deliberate and one-way: the host classifies Shape
//! spellings into `ForeignType` (the marshaling table), the extension renders
//! `ForeignType` into its own language. Neither parses the other's types.
//!
//! One chokepoint, both tiers: `register_contract_once` is called from the VM's
//! link-now path and from the JIT's foreign bridge, so the interpreter and the
//! JIT cannot deliver different contracts (the same ffi-rebuild §4.9 invariant
//! the foreign-reentry counter follows).

use crate::bytecode::ForeignFunctionEntry;
use shape_abi_v1::foreign_types::{
    ForeignContractExport, ForeignDirection, ForeignField, ForeignFunctionContract,
    ForeignParamContract, ForeignScalar, ForeignType,
};
use shape_runtime::plugins::language_runtime::PluginLanguageRuntime;
use shape_runtime::type_schema::{FieldType, TypeSchemaRegistry};
use std::collections::HashSet;

/// Build the declared contract for one language from a program's foreign
/// function table.
///
/// Entries whose declared types fall outside the marshaling table are skipped
/// rather than guessed at: the compiler already refuses those declarations
/// ([C0933]), so reaching one here means the program was linked from bytecode
/// produced elsewhere, and a stub that invents a type would be worse than a
/// stub that omits the function.
pub fn build_contract(
    language: &str,
    entries: &[ForeignFunctionEntry],
    schemas: &TypeSchemaRegistry,
) -> ForeignContractExport {
    let mut contract = ForeignContractExport::new(language);
    let mut seen_types: HashSet<String> = HashSet::new();

    for entry in entries {
        if entry.language != language || entry.native_abi.is_some() {
            continue;
        }
        let Some(function) = classify_entry(entry) else {
            continue;
        };
        for param in &function.params {
            collect_named_types(&param.ty, schemas, &mut seen_types, &mut contract.types);
        }
        collect_named_types(
            &function.returns,
            schemas,
            &mut seen_types,
            &mut contract.types,
        );
        contract.functions.push(function);
    }

    contract
}

/// Classify one entry's declared signature, or `None` if any part of it is
/// outside the marshaling table.
fn classify_entry(entry: &ForeignFunctionEntry) -> Option<ForeignFunctionContract> {
    let mut params = Vec::with_capacity(entry.param_names.len());
    for (name, declared) in entry.param_names.iter().zip(entry.param_types.iter()) {
        let ty = ForeignType::classify(declared, ForeignDirection::Argument).ok()?;
        params.push(ForeignParamContract {
            name: name.clone(),
            ty,
        });
    }
    let returns = ForeignType::classify(
        entry.return_type.as_deref().unwrap_or("none"),
        ForeignDirection::Return,
    )
    .ok()?;
    Some(ForeignFunctionContract {
        name: entry.name.clone(),
        params,
        returns,
    })
}

/// Walk a classified type for named object types and export each one's fields
/// once, so the renderer can emit a class per named type before it is used.
fn collect_named_types(
    ty: &ForeignType,
    schemas: &TypeSchemaRegistry,
    seen: &mut HashSet<String>,
    out: &mut Vec<ForeignType>,
) {
    match ty {
        ForeignType::Optional(inner) => collect_named_types(inner, schemas, seen, out),
        ForeignType::Object {
            name: Some(name),
            fields: None,
        } => {
            if !seen.insert(name.clone()) {
                return;
            }
            let Some(schema) = schemas.get(name) else {
                // No registered schema: emit the bare name so the renderer can
                // still reference it, rather than dropping the type silently.
                out.push(ForeignType::Object {
                    name: Some(name.clone()),
                    fields: None,
                });
                return;
            };
            let mut fields = Vec::with_capacity(schema.fields.len());
            for field in &schema.fields {
                let Some(ty) = field_type_to_foreign(&field.field_type) else {
                    // A field whose type has no wire projection: the whole type
                    // is exported without fields rather than with a fabricated
                    // one. The renderer says so in the stub.
                    out.push(ForeignType::Object {
                        name: Some(name.clone()),
                        fields: None,
                    });
                    return;
                };
                // Nested named types must be declared before this one.
                collect_named_types(&ty, schemas, seen, out);
                fields.push(ForeignField {
                    name: field.wire_name().to_string(),
                    ty,
                    optional: false,
                });
            }
            out.push(ForeignType::Object {
                name: Some(name.clone()),
                fields: Some(fields),
            });
        }
        // Inline literals carry their own fields; scalars, arrays and maps have
        // no named type to declare.
        ForeignType::Object { .. }
        | ForeignType::Scalar(_)
        | ForeignType::Array(_)
        | ForeignType::Map(_) => {}
    }
}

/// Project a schema field type onto the marshaling table.
///
/// This mirrors what `foreign_marshal::typed_object_storage_to_msgpack` and
/// `build_field_slot` actually do with each field kind — a `Decimal` field
/// really does cross as a number, and a `Timestamp` really does cross as an
/// integer — so the stub describes the wire, not the Shape declaration.
fn field_type_to_foreign(field_type: &FieldType) -> Option<ForeignType> {
    match field_type {
        FieldType::I64
        | FieldType::I8
        | FieldType::U8
        | FieldType::I16
        | FieldType::U16
        | FieldType::I32
        | FieldType::U32
        | FieldType::U64
        | FieldType::Timestamp => Some(ForeignType::Scalar(ForeignScalar::Int)),
        FieldType::F64 | FieldType::Decimal => Some(ForeignType::Scalar(ForeignScalar::Number)),
        FieldType::Bool => Some(ForeignType::Scalar(ForeignScalar::Bool)),
        FieldType::String => Some(ForeignType::Scalar(ForeignScalar::String)),
        FieldType::Object(name) => Some(ForeignType::Object {
            name: Some(name.clone()),
            fields: None,
        }),
        FieldType::Array(elem) => match field_type_to_foreign(elem) {
            Some(ForeignType::Scalar(scalar)) => Some(ForeignType::Array(scalar)),
            _ => None,
        },
        _ => None,
    }
}

/// Deliver `language`'s contract to its runtime once per host, and return the
/// stub document the extension generated.
///
/// `already_registered` is the caller's per-host memo. Registration is idempotent
/// on the extension side, but the payload build walks the whole foreign table,
/// so repeating it per call would be a real cost on a hot path.
pub fn register_contract_once(
    language: &str,
    runtime: &PluginLanguageRuntime,
    entries: &[ForeignFunctionEntry],
    schemas: &TypeSchemaRegistry,
    already_registered: &mut HashSet<String>,
) -> Result<Option<String>, String> {
    if !already_registered.insert(language.to_string()) {
        return Ok(None);
    }
    let contract = build_contract(language, entries, schemas);
    runtime
        .register_contract(&contract)
        .map_err(|e| format!("foreign contract registration failed for language '{language}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::TypeSchemaBuilder;

    fn entry(name: &str, params: &[(&str, &str)], ret: &str) -> ForeignFunctionEntry {
        ForeignFunctionEntry {
            name: name.to_string(),
            language: "python".to_string(),
            body_text: String::new(),
            param_names: params.iter().map(|(n, _)| n.to_string()).collect(),
            param_types: params.iter().map(|(_, t)| t.to_string()).collect(),
            return_type: Some(ret.to_string()),
            arg_count: params.len() as u16,
            is_async: false,
            dynamic_errors: true,
            return_type_schema_id: None,
            content_hash: None,
            native_abi: None,
        }
    }

    #[test]
    fn contract_carries_the_declared_signature_classified() {
        let schemas = TypeSchemaRegistry::new();
        let entries = vec![entry("add", &[("a", "int"), ("b", "int")], "Result<int>")];
        let contract = build_contract("python", &entries, &schemas);

        assert_eq!(contract.version, 1);
        assert_eq!(contract.language, "python");
        assert_eq!(contract.functions.len(), 1);
        let f = &contract.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "a");
        assert_eq!(f.params[0].ty, ForeignType::Scalar(ForeignScalar::Int));
        // The `Result<T>` wrapper is the runtime's error channel; the foreign
        // body returns `T`, so the stub must say `T`.
        assert_eq!(f.returns, ForeignType::Scalar(ForeignScalar::Int));
    }

    #[test]
    fn other_languages_and_native_abi_entries_are_not_in_this_contract() {
        let schemas = TypeSchemaRegistry::new();
        let mut ts = entry("tadd", &[("a", "int")], "Result<int>");
        ts.language = "typescript".to_string();
        let mut native = entry("labs", &[("x", "int")], "int");
        native.native_abi = Some(crate::bytecode::NativeAbiSpec {
            abi: "C".to_string(),
            library: "c".to_string(),
            symbol: "labs".to_string(),
            signature: Default::default(),
            package_key: None,
        });
        let entries = vec![entry("padd", &[("a", "int")], "Result<int>"), ts, native];

        let contract = build_contract("python", &entries, &schemas);
        assert_eq!(contract.functions.len(), 1);
        assert_eq!(contract.functions[0].name, "padd");
    }

    #[test]
    fn named_object_types_are_exported_with_their_fields() {
        let mut schemas = TypeSchemaRegistry::new();
        TypeSchemaBuilder::new("Candle")
            .f64_field("open")
            .i64_field("volume")
            .string_field("symbol")
            .register(&mut schemas);
        let entries = vec![entry("analyze", &[("c", "Candle")], "Result<int>")];

        let contract = build_contract("python", &entries, &schemas);
        assert_eq!(contract.types.len(), 1);
        match &contract.types[0] {
            ForeignType::Object {
                name: Some(name),
                fields: Some(fields),
            } => {
                assert_eq!(name, "Candle");
                let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
                assert_eq!(names, vec!["open", "volume", "symbol"]);
                assert_eq!(fields[0].ty, ForeignType::Scalar(ForeignScalar::Number));
                assert_eq!(fields[1].ty, ForeignType::Scalar(ForeignScalar::Int));
                assert_eq!(fields[2].ty, ForeignType::Scalar(ForeignScalar::String));
            }
            other => panic!("expected an exported Candle with fields, got {other:?}"),
        }
    }

    #[test]
    fn a_named_type_is_exported_once_however_often_it_is_referenced() {
        let mut schemas = TypeSchemaRegistry::new();
        TypeSchemaBuilder::new("Candle")
            .f64_field("open")
            .register(&mut schemas);
        let entries = vec![
            entry("a", &[("c", "Candle")], "Result<Candle>"),
            entry("b", &[("c", "Candle")], "Result<int>"),
        ];

        let contract = build_contract("python", &entries, &schemas);
        assert_eq!(contract.types.len(), 1);
        assert_eq!(contract.functions.len(), 2);
    }

    #[test]
    fn a_field_type_with_no_wire_projection_leaves_the_type_field_less() {
        let mut schemas = TypeSchemaRegistry::new();
        TypeSchemaBuilder::new("Bag")
            .any_field("payload")
            .register(&mut schemas);
        let entries = vec![entry("take", &[("b", "Bag")], "Result<int>")];

        let contract = build_contract("python", &entries, &schemas);
        assert_eq!(contract.types.len(), 1);
        match &contract.types[0] {
            ForeignType::Object {
                name: Some(name),
                fields,
            } => {
                assert_eq!(name, "Bag");
                assert!(
                    fields.is_none(),
                    "a field with no projection must not be fabricated"
                );
            }
            other => panic!("expected a field-less Bag, got {other:?}"),
        }
    }

    // ── The host actually calls the channel ────────────────────────────────
    //
    // ADR-019 §1 / #196, scope item (1). `register_types` was caller-less: the
    // vtable slot existed, the extensions implemented it as a no-op, and
    // nothing in the host ever produced a payload. These tests drive the REAL
    // link-now path (`invoke_foreign_kinded`) against an in-process fake
    // extension and assert the payload arrives and the stub comes back.
    //
    // A fake rather than the built `.so` on purpose: this asserts the HOST's
    // behaviour, and must fail if the host stops calling — independently of
    // whether a Python interpreter is present.
    mod fake_extension {
        use shape_abi_v1::{ErrorModel, LanguageRuntimeVTable, STATE_MODEL_STATEFUL_OPAQUE};
        use std::ffi::{c_char, c_void};
        use std::sync::Mutex;

        /// Every `register_types` payload this fake has received.
        pub static RECEIVED: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
        /// How many times `generate_stubs` was asked.
        pub static STUB_CALLS: Mutex<u32> = Mutex::new(0);

        pub const STUB_TEXT: &str = "# fake stub document\n";

        pub fn reset() {
            RECEIVED.lock().unwrap().clear();
            *STUB_CALLS.lock().unwrap() = 0;
        }

        unsafe extern "C" fn init(_config: *const u8, _len: usize) -> *mut c_void {
            // A non-null opaque instance; this fake keeps its state in statics.
            1usize as *mut c_void
        }

        unsafe extern "C" fn register_types(
            _instance: *mut c_void,
            types: *const u8,
            types_len: usize,
        ) -> i32 {
            let bytes = unsafe { std::slice::from_raw_parts(types, types_len) }.to_vec();
            RECEIVED.lock().unwrap().push(bytes);
            0
        }

        unsafe extern "C" fn generate_stubs(
            _instance: *mut c_void,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            *STUB_CALLS.lock().unwrap() += 1;
            let mut buf = STUB_TEXT.as_bytes().to_vec();
            buf.shrink_to_fit();
            let len = buf.len();
            let ptr = buf.as_mut_ptr();
            std::mem::forget(buf);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            0
        }

        #[allow(clippy::too_many_arguments)]
        unsafe extern "C" fn compile(
            _instance: *mut c_void,
            _name: *const u8,
            _name_len: usize,
            _source: *const u8,
            _source_len: usize,
            _param_names: *const u8,
            _param_names_len: usize,
            _param_types: *const u8,
            _param_types_len: usize,
            _return_type: *const u8,
            _return_type_len: usize,
            _is_async: bool,
            _out_error: *mut *mut u8,
            _out_error_len: *mut usize,
        ) -> *mut c_void {
            1usize as *mut c_void
        }

        unsafe extern "C" fn invoke(
            _instance: *mut c_void,
            _handle: *mut c_void,
            _args: *const u8,
            _args_len: usize,
            out_ptr: *mut *mut u8,
            out_len: *mut usize,
        ) -> i32 {
            // msgpack for the integer 7.
            let mut buf = vec![7u8];
            let len = buf.len();
            let ptr = buf.as_mut_ptr();
            std::mem::forget(buf);
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            0
        }

        unsafe extern "C" fn dispose_function(_instance: *mut c_void, _handle: *mut c_void) {}

        unsafe extern "C" fn language_id(_instance: *mut c_void) -> *const c_char {
            c"python".as_ptr()
        }

        unsafe extern "C" fn free_buffer(ptr: *mut u8, len: usize) {
            if !ptr.is_null() {
                unsafe { drop(Vec::from_raw_parts(ptr, len, len)) };
            }
        }

        unsafe extern "C" fn drop_instance(_instance: *mut c_void) {}

        /// ADR-019 §5 (#202): this fake keeps its state in `Mutex` statics and
        /// its shims take no `&mut` through the instance pointer, so it declares
        /// the shared model — the same declaration the real Python runtime makes.
        unsafe extern "C" fn instance_concurrency(_instance: *mut c_void) -> u32 {
            shape_abi_v1::INSTANCE_CONCURRENCY_SHARED
        }

        pub static VTABLE: LanguageRuntimeVTable = LanguageRuntimeVTable {
            init: Some(init),
            register_types: Some(register_types),
            compile: Some(compile),
            invoke: Some(invoke),
            dispose_function: Some(dispose_function),
            language_id: Some(language_id),
            get_lsp_config: None,
            free_buffer: Some(free_buffer),
            drop: Some(drop_instance),
            error_model: ErrorModel::Dynamic,
            get_shape_source: None,
            runtime_descriptor: None,
            state_model: STATE_MODEL_STATEFUL_OPAQUE,
            generate_stubs: Some(generate_stubs),
            instance_concurrency: Some(instance_concurrency),
            reserved2: None,
            reserved3: None,
        };

        /// A vtable identical to [`VTABLE`] except that it declares no stub
        /// channel — the shape a pre-#196 extension binary has.
        pub static VTABLE_WITHOUT_STUBS: LanguageRuntimeVTable = LanguageRuntimeVTable {
            generate_stubs: None,
            init: Some(init),
            register_types: Some(register_types),
            compile: Some(compile),
            invoke: Some(invoke),
            dispose_function: Some(dispose_function),
            language_id: Some(language_id),
            get_lsp_config: None,
            free_buffer: Some(free_buffer),
            drop: Some(drop_instance),
            error_model: ErrorModel::Dynamic,
            get_shape_source: None,
            runtime_descriptor: None,
            state_model: STATE_MODEL_STATEFUL_OPAQUE,
            // A pre-#202 extension declares nothing here; the host reads that
            // as interpreter-thread-only and refuses to offload into it.
            instance_concurrency: None,
            reserved2: None,
            reserved3: None,
        };
    }

    /// Serialize this test module's access to the fake extension's statics.
    static FAKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn vm_with_fake_runtime(
        code: &str,
        vtable: &'static shape_abi_v1::LanguageRuntimeVTable,
    ) -> crate::executor::VirtualMachine {
        use crate::compiler::BytecodeCompiler;
        use crate::executor::{VMConfig, VirtualMachine};

        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile failed");

        let runtime = PluginLanguageRuntime::new(vtable, &serde_json::Value::Null)
            .expect("fake runtime initializes");
        let mut runtimes = std::collections::HashMap::new();
        runtimes.insert("python".to_string(), std::sync::Arc::new(runtime));

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.foreign_fn_handles = vec![None];
        vm.set_language_runtimes(runtimes);
        vm
    }

    const TWO_FUNCTION_PROGRAM: &str = r#"
type Candle { open: number }
fn python analyze(c: Candle) -> Result<int> {
    return 1
}
fn python add(a: int, b: int) -> Result<int> {
    return a + b
}
"#;

    /// The tripwire for scope item (1): linking a foreign function delivers the
    /// declared contract through `register_types` and collects the stub the
    /// extension generates from it.
    #[test]
    fn linking_a_foreign_function_delivers_the_contract_and_collects_the_stub() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fake_extension::reset();

        let mut vm = vm_with_fake_runtime(TWO_FUNCTION_PROGRAM, &fake_extension::VTABLE);
        vm.invoke_foreign_kinded(0, &[]).expect("foreign call runs");

        let received = fake_extension::RECEIVED.lock().unwrap().clone();
        assert_eq!(
            received.len(),
            1,
            "the host must deliver exactly one contract for the language"
        );

        let contract: ForeignContractExport =
            rmp_serde::from_slice(&received[0]).expect("the payload is a foreign contract");
        contract.check_version().expect("current wire version");
        assert_eq!(contract.language, "python");
        // The WHOLE program's contract, not just the function being linked —
        // one stub describes the file.
        let names: Vec<&str> = contract.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["analyze", "add"]);
        assert_eq!(
            contract.types.len(),
            1,
            "the named type referenced by `analyze` is exported"
        );

        assert_eq!(*fake_extension::STUB_CALLS.lock().unwrap(), 1);
        assert_eq!(
            vm.foreign_stub_document("python"),
            Some(fake_extension::STUB_TEXT),
            "the generated stub is reachable from the host"
        );
    }

    /// The contract is built once per language, not once per call: the payload
    /// walk is O(program), and repeating it per invocation would put it on the
    /// hot path.
    #[test]
    fn the_contract_is_delivered_once_however_many_calls_are_made() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fake_extension::reset();

        let mut vm = vm_with_fake_runtime(TWO_FUNCTION_PROGRAM, &fake_extension::VTABLE);
        for _ in 0..3 {
            vm.invoke_foreign_kinded(0, &[]).expect("foreign call runs");
        }
        vm.invoke_foreign_kinded(1, &[])
            .expect("second foreign function runs");

        assert_eq!(
            fake_extension::RECEIVED.lock().unwrap().len(),
            1,
            "the contract must be delivered once per (VM, language)"
        );
    }

    /// An extension built before the stub channel existed leaves the vtable slot
    /// `None`. The contract is still delivered; only the stub is unavailable —
    /// and the host says so by having no document, rather than inventing one.
    #[test]
    fn an_extension_without_the_stub_channel_still_receives_the_contract() {
        let _guard = FAKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fake_extension::reset();

        let mut vm =
            vm_with_fake_runtime(TWO_FUNCTION_PROGRAM, &fake_extension::VTABLE_WITHOUT_STUBS);
        vm.invoke_foreign_kinded(0, &[]).expect("foreign call runs");

        assert_eq!(fake_extension::RECEIVED.lock().unwrap().len(), 1);
        assert_eq!(*fake_extension::STUB_CALLS.lock().unwrap(), 0);
        assert_eq!(vm.foreign_stub_document("python"), None);
    }

    #[test]
    fn the_contract_round_trips_through_the_msgpack_wire() {
        let mut schemas = TypeSchemaRegistry::new();
        TypeSchemaBuilder::new("Candle")
            .f64_field("open")
            .register(&mut schemas);
        let entries = vec![entry(
            "analyze",
            &[("c", "Candle"), ("xs", "Array<number>")],
            "Result<int?>",
        )];
        let contract = build_contract("python", &entries, &schemas);

        let bytes = rmp_serde::to_vec_named(&contract).expect("encode");
        let decoded: ForeignContractExport = rmp_serde::from_slice(&bytes).expect("decode");
        assert_eq!(decoded, contract);
        decoded.check_version().expect("version is current");
    }
}
