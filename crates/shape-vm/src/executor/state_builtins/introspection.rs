// Capture / introspection implementations for `std::state`.
//
// Bodies read live VM state through `ctx.vm_state` and return real carriers
// where the current typed-return ABI can represent them. Unsupported public
// surfaces stay structured `Err(String)` values rather than fabricated state.

use shape_runtime::module_exports::{FrameInfo, ModuleContext};
use shape_runtime::type_schema::TypeSchema;
use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
use shape_value::heap_value::{
    HashMapData, HashMapKindedRef, HeapValue, TypedObjectPtr, TypedObjectStorage,
};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{
    ELEM_TYPE_BOOL, ELEM_TYPE_CALLABLE, ELEM_TYPE_CONTENT, ELEM_TYPE_F32, ELEM_TYPE_F64,
    ELEM_TYPE_I8, ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_STRING,
    ELEM_TYPE_TRAIT_OBJECT, ELEM_TYPE_TYPED_ARRAY, ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U8,
    ELEM_TYPE_U16, ELEM_TYPE_U32, ELEM_TYPE_UNKNOWN, TypedArray, read_elem_type, stamp_elem_type,
};
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use std::sync::Arc;

/// Surface for state bodies that cannot even read live VM state in the current
/// call context.
fn capture_surface(op: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — live VM state is unavailable \
         in this dispatch context, and this body will not synthesize state. \
         ADR-006 §2.7.4 + §2.7.5.1.",
    )
}

fn carrier_surface(op: &str, return_shape: &str, carrier_shape: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — the public {return_shape} \
         shape needs {carrier_shape} projection; this body will not \
         fabricate an `Any` container. ADR-006 §2.7.4 + §2.7.5.1.",
    )
}

fn missing_schema_surface(op: &str, schema_name: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — schema-backed state carrier \
         construction requires `{schema_name}` in ctx.schemas, but this \
         dispatch context did not register it. The body will not fabricate \
         a schema id or return an anonymous object for public state. \
         ADR-006 §2.7.4 + §2.7.5.1.",
    )
}

fn state_schema<'ctx>(
    ctx: &ModuleContext<'ctx>,
    op: &str,
    schema_name: &str,
) -> Result<&'ctx TypeSchema, String> {
    let schemas: &'ctx shape_runtime::type_schema::TypeSchemaRegistry = ctx.schemas;
    schemas
        .get(schema_name)
        .ok_or_else(|| missing_schema_surface(op, schema_name))
}

pub(crate) fn callable_content_hash(
    op: &str,
    arg: &KindedSlot,
    ctx: &ModuleContext,
) -> Result<String, String> {
    let bits = arg.slot().raw();
    let function_id = match arg.kind() {
        NativeKind::Int64 | NativeKind::UInt64 => Some(bits as u16),
        NativeKind::Ptr(HeapKind::Closure) => {
            if bits == 0 {
                None
            } else {
                // SAFETY: matches the established state.fn_hash callable path:
                // closure bits are an OwnedClosureBlock pointer.
                let ptr = bits as *const u8;
                Some(unsafe { shape_value::v2::closure_raw::typed_closure_function_id(ptr) })
            }
        }
        _ => None,
    };
    let Some(fid) = function_id else {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — argument is not a \
             function value (kind={:?}); function-handle decoding for \
             HeapKind::FunctionRef / TraitObject not yet wired. ADR-006 \
             §2.7.4.",
            arg.kind()
        ));
    };
    let Some(hashes) = ctx.function_hashes else {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — ctx.function_hashes is \
             None at this dispatch surface; content-addressed metadata not \
             propagated through invoke_module_fn_id_stub. ADR-006 §2.7.4."
        ));
    };
    let Some(maybe_hash) = hashes.get(fid as usize) else {
        return Err(format!(
            "{op}: function_id {fid} out of range (program has {} \
             functions). ADR-006 §2.7.4.",
            hashes.len()
        ));
    };
    let Some(hash_bytes) = maybe_hash else {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — function_id {fid} has no \
             content-addressed hash entry (compiled without \
             content-addressed metadata). ADR-006 §2.7.4."
        ));
    };
    Ok(hex::encode(hash_bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgsElementKind {
    Int,
    Number,
    String,
}

impl ArgsElementKind {
    fn label(self) -> &'static str {
        match self {
            ArgsElementKind::Int => "int",
            ArgsElementKind::Number => "number",
            ArgsElementKind::String => "string",
        }
    }
}

fn slot_to_i64(slot: &KindedSlot) -> Option<i64> {
    match slot.kind() {
        NativeKind::Int64 => Some(slot.raw() as i64),
        NativeKind::Int32 => Some(slot.raw() as i32 as i64),
        NativeKind::Int16 => Some(slot.raw() as i16 as i64),
        NativeKind::Int8 => Some(slot.raw() as i8 as i64),
        NativeKind::UInt64 => Some(slot.raw() as i64),
        NativeKind::UInt32 => Some((slot.raw() as u32) as i64),
        NativeKind::UInt16 => Some((slot.raw() as u16) as i64),
        NativeKind::UInt8 => Some((slot.raw() as u8) as i64),
        NativeKind::IntSize => Some(slot.raw() as isize as i64),
        NativeKind::UIntSize => Some((slot.raw() as usize) as i64),
        _ => None,
    }
}

fn slot_to_f64(slot: &KindedSlot) -> Option<f64> {
    match slot.kind() {
        NativeKind::Float64 => Some(f64::from_bits(slot.raw())),
        NativeKind::Float32 => Some(f64::from(f32::from_bits(slot.raw() as u32))),
        _ => None,
    }
}

fn slot_to_owned_string(slot: &KindedSlot) -> Result<Option<String>, String> {
    match slot.kind() {
        NativeKind::String | NativeKind::StringV2 => {
            slot.as_str().map(|s| Some(s.to_string())).ok_or_else(|| {
                format!(
                    "string slot with kind {:?} could not be borrowed as \
                     UTF-8; construction-side string carrier contract was \
                     violated. ADR-006 §2.7.5.1.",
                    slot.kind()
                )
            })
        }
        _ => Ok(None),
    }
}

fn args_element_kind(slot: &KindedSlot) -> Result<ArgsElementKind, String> {
    if slot_to_i64(slot).is_some() {
        return Ok(ArgsElementKind::Int);
    }
    if slot_to_f64(slot).is_some() {
        return Ok(ArgsElementKind::Number);
    }
    if slot_to_owned_string(slot)?.is_some() {
        return Ok(ArgsElementKind::String);
    }
    Err(format!("{:?}", slot.kind()))
}

fn args_empty_surface() -> String {
    "state.args: W17-snapshot-resume surface — current frame has no \
     arguments, and the typed return boundary cannot materialize an empty \
     Array<any> without an element kind. W17-marshal-return-arms must carry \
     Array<any> before this can return `[]` without guessing a carrier. \
     ADR-006 §2.7.4 + §2.7.5.1."
        .to_string()
}

fn args_unsupported_surface(idx: usize, kind: &str) -> String {
    format!(
        "state.args: W17-snapshot-resume surface — args[{idx}] has kind \
         {kind}; this lane only returns real homogeneous Array<int>, \
         Array<number>, or Array<string> carriers. Bool, null, heap-shaped, \
         and polymorphic Array<any> arguments need W17-marshal-return-arms; \
         no fabricated `Any` container is produced. ADR-006 §2.7.4 + \
         §2.7.5.1.",
    )
}

fn args_heterogeneous_surface(idx: usize, expected: ArgsElementKind, found: NativeKind) -> String {
    format!(
        "state.args: W17-snapshot-resume surface — current-frame args are \
         heterogeneous: args[0] projects as Array<{}>, but args[{idx}] has \
         kind {found:?}. A true heterogeneous Array<any> return needs \
         W17-marshal-return-arms at project_typed_return; no scalar is \
         coerced or boxed into a fabricated `Any` carrier. ADR-006 §2.7.4 \
         + §2.7.5.1.",
        expected.label()
    )
}

fn project_current_args(args: Vec<KindedSlot>) -> Result<TypedReturn, String> {
    let Some(first) = args.first() else {
        return Err(args_empty_surface());
    };
    let expected =
        args_element_kind(first).map_err(|kind| args_unsupported_surface(0, kind.as_str()))?;

    match expected {
        ArgsElementKind::Int => {
            let mut values = Vec::with_capacity(args.len());
            for (idx, slot) in args.iter().enumerate() {
                let Some(value) = slot_to_i64(slot) else {
                    return Err(args_heterogeneous_surface(idx, expected, slot.kind()));
                };
                values.push(value);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayI64(values)))
        }
        ArgsElementKind::Number => {
            let mut values = Vec::with_capacity(args.len());
            for (idx, slot) in args.iter().enumerate() {
                let Some(value) = slot_to_f64(slot) else {
                    return Err(args_heterogeneous_surface(idx, expected, slot.kind()));
                };
                values.push(value);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayF64(values)))
        }
        ArgsElementKind::String => {
            let mut values = Vec::with_capacity(args.len());
            for (idx, slot) in args.iter().enumerate() {
                let value = slot_to_owned_string(slot)?
                    .ok_or_else(|| args_heterogeneous_surface(idx, expected, slot.kind()))?;
                values.push(value);
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayString(values)))
        }
    }
}

fn locals_empty_surface() -> String {
    "state.locals: W17-snapshot-resume surface — current frame exposes no \
     locals with a recoverable value kind, and the typed return boundary \
     cannot materialize an empty HashMap<string, any> without a value \
     carrier. Returning an empty HashMap<string, string> would guess the \
     value type. ADR-006 §2.7.4 + §2.7.5.1."
        .to_string()
}

fn is_synthetic_current_local_name(name: &str) -> bool {
    let is_numbered_local = name
        .strip_prefix("local_")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()));
    name.starts_with("__") || is_numbered_local
}

fn project_current_locals(locals: Vec<(String, KindedSlot)>) -> Result<TypedReturn, String> {
    let locals: Vec<(String, KindedSlot)> = locals
        .into_iter()
        .filter(|(name, _)| !is_synthetic_current_local_name(name))
        .collect();

    if locals.is_empty() {
        return Err(locals_empty_surface());
    }

    let mut pairs = Vec::with_capacity(locals.len());
    for (name, slot) in locals {
        let value = slot_to_owned_string(&slot)?.ok_or_else(|| {
            format!(
                "state.locals: W17-snapshot-resume surface — local `{name}` \
                 has kind {:?}; this lane only returns real \
                 HashMap<string, string> carriers. HashMap<string, any> and \
                 scalar-valued maps need W17-marshal-return-arms plus the \
                 HashMapData kinded-value-track follow-up; no fabricated \
                 `Any` map value is produced. ADR-006 §2.7.4 + §2.7.5.1.",
                slot.kind()
            )
        })?;
        pairs.push((name, value));
    }

    Ok(TypedReturn::Concrete(ConcreteReturn::HashMapStringString(
        pairs,
    )))
}

fn typed_object_slot_for_schema(
    op: &str,
    schema: &TypeSchema,
    mut fields: Vec<(String, KindedSlot)>,
) -> Result<KindedSlot, String> {
    if schema.fields.len() != fields.len() {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — `{}` schema declares {} \
             field(s), but the state carrier builder produced {} field(s). \
             Refusing to synthesize or drop fields. ADR-006 §2.7.4.",
            schema.name,
            schema.fields.len(),
            fields.len()
        ));
    }

    let mut slots = Vec::with_capacity(schema.fields.len());
    let mut field_kinds = Vec::with_capacity(schema.fields.len());
    #[cfg(miri)]
    let mut field_provenance = Vec::with_capacity(schema.fields.len());
    let mut heap_mask = 0u64;

    for (idx, field) in schema.fields.iter().enumerate() {
        let Some(pos) = fields.iter().position(|(name, _)| name == &field.name) else {
            return Err(format!(
                "{op}: W17-snapshot-resume surface — `{}` schema field `{}` \
                 has no real value in the state carrier. Refusing to \
                 synthesize a default. ADR-006 §2.7.4.",
                schema.name, field.name
            ));
        };
        let (_, value) = fields.swap_remove(pos);
        let slot = value.slot();
        let kind = value.kind();
        #[cfg(miri)]
        field_provenance.push(value.miri_provenance());
        if kind.is_refcounted() {
            heap_mask |= 1u64 << idx;
        }
        slots.push(slot);
        field_kinds.push(kind);
        std::mem::forget(value);
    }

    let field_kinds = Arc::from(field_kinds.into_boxed_slice());
    #[cfg(miri)]
    let ptr = TypedObjectStorage::_new_with_miri_field_provenance(
        schema.id as u64,
        slots.into_boxed_slice(),
        heap_mask,
        field_kinds,
        field_provenance.into_boxed_slice(),
    );
    #[cfg(not(miri))]
    let ptr = TypedObjectStorage::_new(
        schema.id as u64,
        slots.into_boxed_slice(),
        heap_mask,
        field_kinds,
    );

    Ok(KindedSlot::from_typed_object_raw(ptr))
}

fn frame_state_slot(
    op: &str,
    frame_label: &str,
    frame_schema: &TypeSchema,
    frame: FrameInfo,
) -> Result<KindedSlot, String> {
    let Some(blob_hash) = frame.blob_hash else {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — {frame_label} `{}` has \
             no content-addressed hash entry, so a portable FrameState \
             metadata object cannot be constructed. No empty or synthetic \
             blob_hash is returned. ADR-006 §2.7.4.",
            frame.function_name
        ));
    };
    let arg_count = frame.args.len();
    let local_count = frame.locals.len();
    let upvalue_count = frame.upvalues.as_ref().map_or(0, Vec::len);
    typed_object_slot_for_schema(
        op,
        frame_schema,
        vec![
            (
                "function_name".to_string(),
                KindedSlot::from_string(frame.function_name.as_str()),
            ),
            (
                "blob_hash".to_string(),
                KindedSlot::from_string(hex::encode(blob_hash).as_str()),
            ),
            (
                "ip".to_string(),
                KindedSlot::from_int(frame.local_ip as i64),
            ),
            (
                "arg_count".to_string(),
                KindedSlot::from_int(arg_count as i64),
            ),
            (
                "local_count".to_string(),
                KindedSlot::from_int(local_count as i64),
            ),
            (
                "upvalue_count".to_string(),
                KindedSlot::from_int(upvalue_count as i64),
            ),
        ],
    )
}

fn take_typed_object_slot(
    op: &str,
    label: &str,
    slot: KindedSlot,
) -> Result<*const TypedObjectStorage, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) || slot.raw() == 0 {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — `{label}` did not produce \
             a live TypedObject carrier (kind={:?}, bits={}). Refusing to \
             reinterpret the slot. ADR-006 §2.7.4.",
            slot.kind(),
            slot.raw()
        ));
    }
    let ptr = slot.raw() as *const TypedObjectStorage;
    std::mem::forget(slot);
    Ok(ptr)
}

fn frame_state_array_slot(
    op: &str,
    frame_schema: &TypeSchema,
    frames: Vec<FrameInfo>,
) -> Result<KindedSlot, String> {
    let mut element_ptrs: Vec<*const TypedObjectStorage> = Vec::with_capacity(frames.len());
    for (idx, frame) in frames.into_iter().enumerate() {
        let frame_label = format!("frame[{idx}]");
        let frame_slot = frame_state_slot(op, frame_label.as_str(), frame_schema, frame)?;
        element_ptrs.push(take_typed_object_slot(
            op,
            frame_label.as_str(),
            frame_slot,
        )?);
    }

    let arr = TypedArray::<*const TypedObjectStorage>::from_slice(&element_ptrs);
    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
    #[cfg(miri)]
    return Ok(KindedSlot::new_with_miri_provenance(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
        shape_value::heap_value::MiriSlotProvenance::TypedArray(arr as *mut u8),
    ));
    #[cfg(not(miri))]
    Ok(KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleBindingKind {
    Int,
    Number,
    Bool,
    String,
}

impl ModuleBindingKind {
    fn label(self) -> &'static str {
        match self {
            ModuleBindingKind::Int => "int",
            ModuleBindingKind::Number => "number",
            ModuleBindingKind::Bool => "bool",
            ModuleBindingKind::String => "string",
        }
    }
}

fn module_binding_kind(slot: &KindedSlot) -> Result<ModuleBindingKind, String> {
    if slot_to_i64(slot).is_some() {
        return Ok(ModuleBindingKind::Int);
    }
    if slot_to_f64(slot).is_some() {
        return Ok(ModuleBindingKind::Number);
    }
    if slot.as_bool().is_some() {
        return Ok(ModuleBindingKind::Bool);
    }
    if slot_to_owned_string(slot)?.is_some() {
        return Ok(ModuleBindingKind::String);
    }
    Err(format!("{:?}", slot.kind()))
}

fn module_binding_unsupported_surface(op: &str, name: &str, kind: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — module binding \
         `{name}` has kind {kind}; this lane only returns homogeneous \
         scalar/string module_bindings carriers. Heap-shaped or polymorphic \
         Map<string, any> bindings need the kinded map carrier; no fabricated \
         `Any` value is produced. ADR-006 §2.7.4 + §2.7.5.1.",
    )
}

fn module_binding_heterogeneous_surface(
    op: &str,
    name: &str,
    expected: ModuleBindingKind,
    found: NativeKind,
) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — module_bindings \
         are heterogeneous: the first binding projects as HashMap<string, \
         {}>, but binding `{name}` has kind {found:?}. A true \
         Map<string, any> return needs the kinded map carrier; no scalar is \
         coerced or boxed into a fabricated `Any` value. ADR-006 §2.7.4 + \
         §2.7.5.1.",
        expected.label()
    )
}

fn is_import_namespace_binding(ctx: &ModuleContext, slot: &KindedSlot) -> bool {
    if !matches!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject)) {
        return false;
    }
    let Some(storage) = slot.as_typed_object_storage() else {
        return false;
    };
    let Ok(schema_id) = u32::try_from(storage.schema_id) else {
        return false;
    };
    ctx.schemas
        .get_by_id(schema_id)
        .is_some_and(|schema| schema.name.starts_with("__mod_"))
}

fn module_bindings_slot(
    ctx: &ModuleContext,
    op: &str,
    bindings: Vec<(String, KindedSlot)>,
) -> Result<KindedSlot, String> {
    let bindings: Vec<(String, KindedSlot)> = bindings
        .into_iter()
        .filter(|(_, slot)| !is_import_namespace_binding(ctx, slot))
        .collect();

    if bindings.is_empty() {
        return Ok(KindedSlot::from_hashmap(Arc::new(
            HashMapKindedRef::String(Arc::new(HashMapData::new())),
        )));
    }

    let expected = module_binding_kind(&bindings[0].1).map_err(|kind| {
        module_binding_unsupported_surface(op, bindings[0].0.as_str(), kind.as_str())
    })?;

    match expected {
        ModuleBindingKind::Int => {
            let mut data: HashMapData<i64> = HashMapData::new();
            for (name, slot) in bindings.iter() {
                let Some(value) = slot_to_i64(slot) else {
                    return Err(module_binding_heterogeneous_surface(
                        op,
                        name.as_str(),
                        expected,
                        slot.kind(),
                    ));
                };
                unsafe { data.insert(name.as_str(), value) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::I64(
                Arc::new(data),
            ))))
        }
        ModuleBindingKind::Number => {
            let mut data: HashMapData<f64> = HashMapData::new();
            for (name, slot) in bindings.iter() {
                let Some(value) = slot_to_f64(slot) else {
                    return Err(module_binding_heterogeneous_surface(
                        op,
                        name.as_str(),
                        expected,
                        slot.kind(),
                    ));
                };
                unsafe { data.insert(name.as_str(), value) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::F64(
                Arc::new(data),
            ))))
        }
        ModuleBindingKind::Bool => {
            let mut data: HashMapData<u8> = HashMapData::new();
            for (name, slot) in bindings.iter() {
                let Some(value) = slot.as_bool() else {
                    return Err(module_binding_heterogeneous_surface(
                        op,
                        name.as_str(),
                        expected,
                        slot.kind(),
                    ));
                };
                unsafe { data.insert(name.as_str(), u8::from(value)) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::Bool(
                Arc::new(data),
            ))))
        }
        ModuleBindingKind::String => {
            let mut data: HashMapData<*const StringObj> = HashMapData::new();
            for (name, slot) in bindings.iter() {
                let value = slot_to_owned_string(slot)?.ok_or_else(|| {
                    module_binding_heterogeneous_surface(op, name.as_str(), expected, slot.kind())
                })?;
                let value_ptr = StringObj::new(value.as_str()) as *const StringObj;
                unsafe { data.insert(name.as_str(), value_ptr) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(
                HashMapKindedRef::String(Arc::new(data)),
            )))
        }
    }
}

fn schema_hash_slot(ctx: &ModuleContext) -> Result<KindedSlot, String> {
    let mut data: HashMapData<*const StringObj> = HashMapData::new();
    let mut names: Vec<&str> = ctx.schemas.type_names().collect();
    names.sort_unstable();

    for name in names {
        let Some(schema) = ctx.schemas.get(name) else {
            return Err(format!(
                "state.capture_module: W17-snapshot-resume surface — schema \
                 name `{name}` disappeared from ctx.schemas during iteration; \
                 refusing to fabricate schema metadata. ADR-006 §2.7.4."
            ));
        };
        let bytes = bincode::serialize(schema).map_err(|e| {
            format!(
                "state.capture_module: W17-snapshot-resume surface — bincode \
                 serialization failed for schema `{name}`: {e}. ADR-006 \
                 §2.7.5.1."
            )
        })?;
        let digest = shape_runtime::hash_bytes(&bytes);
        let hash = digest.hex().to_string();
        let hash_ptr = StringObj::new(hash.as_str()) as *const StringObj;
        unsafe { data.insert(name, hash_ptr) };
    }

    Ok(KindedSlot::from_hashmap(Arc::new(
        HashMapKindedRef::String(Arc::new(data)),
    )))
}

fn call_args_elem_label(elem_type: u8) -> &'static str {
    match elem_type {
        ELEM_TYPE_I64 | ELEM_TYPE_I32 | ELEM_TYPE_I16 | ELEM_TYPE_I8 | ELEM_TYPE_U32
        | ELEM_TYPE_U16 | ELEM_TYPE_U8 => "int",
        ELEM_TYPE_F64 | ELEM_TYPE_F32 => "number",
        ELEM_TYPE_BOOL => "bool",
        ELEM_TYPE_STRING => "string",
        ELEM_TYPE_TYPED_OBJECT => "typed-object",
        ELEM_TYPE_UNKNOWN => "unknown",
        ELEM_TYPE_TYPED_ARRAY => "nested-array",
        ELEM_TYPE_TRAIT_OBJECT => "trait-object",
        ELEM_TYPE_CALLABLE => "callable",
        ELEM_TYPE_CONTENT => "content",
        _ => "unsupported",
    }
}

fn call_args_slot(op: &str, args: &KindedSlot) -> Result<KindedSlot, String> {
    if args.kind() != NativeKind::Ptr(HeapKind::TypedArray) || args.raw() == 0 {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — `args` must be a live \
             homogeneous TypedArray carrier, got kind {:?}. This lane only \
             preserves real Array<int>, Array<number>, Array<bool>, and \
             Array<string> argument carriers; it will not fabricate \
             Array<any>. ADR-006 §2.7.4 + §2.7.5.1.",
            args.kind()
        ));
    }

    let elem_type = unsafe { read_elem_type(args.raw() as *const u8) };
    match elem_type {
        ELEM_TYPE_I64 | ELEM_TYPE_I32 | ELEM_TYPE_I16 | ELEM_TYPE_I8 | ELEM_TYPE_U32
        | ELEM_TYPE_U16 | ELEM_TYPE_U8 | ELEM_TYPE_F64 | ELEM_TYPE_F32 | ELEM_TYPE_BOOL
        | ELEM_TYPE_STRING => Ok(args.clone()),
        other => Err(format!(
            "{op}: W17-snapshot-resume surface — `args` typed-array element \
             stamp {other} ({}) is not supported by the bounded CallPayload \
             carrier. This lane only preserves homogeneous scalar/string \
             Array<int>, Array<number>, Array<bool>, and Array<string> \
             carriers. Heap-shaped, nested, callable, unstamped, or \
             heterogeneous Array<any> arguments need a real Any container; \
             no argument value is boxed or coerced. ADR-006 §2.7.4 + \
             §2.7.5.1.",
            call_args_elem_label(other)
        )),
    }
}

fn opaque_typed_object_return(
    op: &str,
    label: &str,
    slot: KindedSlot,
) -> Result<TypedReturn, String> {
    let ptr = take_typed_object_slot(op, label, slot)?;
    Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
        Arc::new(HeapValue::TypedObject(TypedObjectPtr::new(ptr))),
    )))
}

fn frame_metadata_pairs(
    op: &str,
    frame: FrameInfo,
    arg_count: usize,
    local_count: usize,
) -> Result<Vec<(String, ConcreteReturn)>, String> {
    let Some(blob_hash) = frame.blob_hash else {
        return Err(format!(
            "{op}: W17-snapshot-resume surface — current frame `{}` has \
             no content-addressed hash entry, so a portable FrameState \
             metadata object cannot be constructed. No empty or synthetic \
             blob_hash is returned. ADR-006 §2.7.4.",
            frame.function_name
        ));
    };
    let upvalue_count = frame.upvalues.as_ref().map_or(0, Vec::len);
    Ok(vec![
        (
            "function_name".to_string(),
            ConcreteReturn::String(frame.function_name),
        ),
        (
            "blob_hash".to_string(),
            ConcreteReturn::String(hex::encode(blob_hash)),
        ),
        ("ip".to_string(), ConcreteReturn::I64(frame.local_ip as i64)),
        (
            "arg_count".to_string(),
            ConcreteReturn::I64(arg_count as i64),
        ),
        (
            "local_count".to_string(),
            ConcreteReturn::I64(local_count as i64),
        ),
        (
            "upvalue_count".to_string(),
            ConcreteReturn::I64(upvalue_count as i64),
        ),
    ])
}

// ===========================================================================
// Capture / introspection implementations (live VM access via ctx.vm_state)
// ===========================================================================

/// `state.capture() -> FrameState`
pub(crate) fn state_capture_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.capture"));
    };
    // Read the current frame; if no frame is on the stack the body
    // surfaces with the canonical not-in-a-function message.
    let Some(frame) = vm_state.current_frame() else {
        return Err(format!(
            "state.capture: no current frame — state.capture must be \
             called from within a function body. ADR-006 §2.7.4."
        ));
    };
    let arg_count = vm_state.current_args().len();
    let local_count = vm_state.current_locals().len();
    Ok(TypedReturn::ObjectPairs(frame_metadata_pairs(
        "state.capture",
        frame,
        arg_count,
        local_count,
    )?))
}

/// `state.capture_all() -> VmState`
pub(crate) fn state_capture_all_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.capture_all"));
    };
    let frame_schema = state_schema(ctx, "state.capture_all", "FrameState")?;
    let vm_state_schema = state_schema(ctx, "state.capture_all", "VmState")?;
    let frames = vm_state.all_frames();
    let frames_slot = frame_state_array_slot("state.capture_all", frame_schema, frames)?;
    let bindings = vm_state.module_bindings();
    let bindings_slot = module_bindings_slot(ctx, "state.capture_all", bindings)?;
    let icount = vm_state.instruction_count();
    let vm_state_slot = typed_object_slot_for_schema(
        "state.capture_all",
        vm_state_schema,
        vec![
            ("frames".to_string(), frames_slot),
            ("module_bindings".to_string(), bindings_slot),
            (
                "instruction_count".to_string(),
                KindedSlot::from_int(icount as i64),
            ),
        ],
    )?;
    opaque_typed_object_return("state.capture_all", "VmState", vm_state_slot)
}

/// `state.capture_module() -> ModuleState`
pub(crate) fn state_capture_module_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.capture_module"));
    };
    let module_state_schema = state_schema(ctx, "state.capture_module", "ModuleState")?;
    let bindings_slot =
        module_bindings_slot(ctx, "state.capture_module", vm_state.module_bindings())?;
    let schemas_slot = schema_hash_slot(ctx)?;
    let module_state_slot = typed_object_slot_for_schema(
        "state.capture_module",
        module_state_schema,
        vec![
            ("bindings".to_string(), bindings_slot),
            ("schemas".to_string(), schemas_slot),
        ],
    )?;
    opaque_typed_object_return("state.capture_module", "ModuleState", module_state_slot)
}

/// `state.capture_call(f, args) -> CallPayload`
pub(crate) fn state_capture_call_stub(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let [callable, call_args] = args else {
        return Err(carrier_surface(
            "state.capture_call",
            "CallPayload",
            "function hash plus homogeneous scalar/string args",
        ));
    };
    let call_payload_schema = state_schema(ctx, "state.capture_call", "CallPayload")?;
    let hash = callable_content_hash("state.capture_call", callable, ctx)?;
    let args_slot = call_args_slot("state.capture_call", call_args)?;
    let call_payload_slot = typed_object_slot_for_schema(
        "state.capture_call",
        call_payload_schema,
        vec![
            ("hash".to_string(), KindedSlot::from_string(hash.as_str())),
            ("args".to_string(), args_slot),
        ],
    )?;
    opaque_typed_object_return("state.capture_call", "CallPayload", call_payload_slot)
}

/// `state.resume(snapshot) -> never`
pub(crate) fn state_resume_stub(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    // Surface clean if the dispatch shell doesn't wire the
    // set_pending_resume callback (no live dispatch path — typically the
    // gate-test surface). Also covers test-only ModuleContexts where
    // every callback is None.
    let Some(set_pending_resume) = ctx.set_pending_resume else {
        return Err(format!(
            "state.resume: W17-snapshot-resume surface — public \
             `state.resume(vm: VmState)` is not wired in this dispatch \
             context. Full resume requires a live set_pending_resume \
             callback plus a schema-backed VmState typed object; this body \
             will not fabricate a VmState from metadata or counts. ADR-006 \
             §2.7.4 + §2.7.5.1."
        ));
    };
    // Per the registered schema (`state.resume(vm: VmState)`), arity = 1.
    let Some(snapshot_slot) = args.first() else {
        return Err(format!(
            "state.resume: W17-snapshot-resume surface — missing required \
             `vm: VmState` argument. ADR-006 §2.7.4."
        ));
    };
    set_pending_resume(snapshot_slot.clone());
    // The `never` return type means execution does not flow past this
    // call — the dispatch loop diverts to `apply_pending_resume` on
    // the next instruction. Returning Unit keeps the marshal happy in
    // the meantime; in practice the dispatch loop tears down the frame
    // before this value is observed.
    Ok(TypedReturn::Concrete(
        shape_runtime::typed_module_exports::ConcreteReturn::Unit,
    ))
}

/// `state.resume_frame(frame_state) -> any`
pub(crate) fn state_resume_frame_stub(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    // Test-shell / no-live-dispatch path: surface clean.
    let Some(_set_pending_frame_resume) = ctx.set_pending_frame_resume else {
        return Err(capture_surface("state.resume_frame"));
    };
    let Some(_frame_state_slot) = args.first() else {
        return Err(format!(
            "state.resume_frame: W17-snapshot-resume surface — missing \
             required `f: FrameState` argument. ADR-006 §2.7.4."
        ));
    };
    // The bounded FrameState metadata returned by state.capture() does
    // not carry resumable local slots. Surface clean rather than
    // fabricating (ip_offset=0, locals=[]), which would silently corrupt
    // resume.
    Err(format!(
        "state.resume_frame: W17-snapshot-resume surface — current \
         FrameState carries only metadata \
         {{ function_name, blob_hash, ip, arg_count, local_count, \
         upvalue_count }}. Its `ip` field is an observed metadata value, \
         not a validated resumable offset. Re-entering a frame requires \
         actual arg/local/upvalue slots plus a validated resume IP; this \
         lane refuses to fabricate slots from counts or reinterpret \
         metadata as executable state. Needs a resumable FrameState value \
         carrier/schema expansion plus typed-object field decode. ADR-006 \
         §2.7.4."
    ))
}

/// `state.caller() -> FunctionRef?`
pub(crate) fn state_caller_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.caller"));
    };
    let Some(caller) = vm_state.caller_frame() else {
        return Ok(TypedReturn::None);
    };
    let Some(hash_bytes) = caller.blob_hash else {
        return Err(format!(
            "state.caller: W17-snapshot-resume surface — caller frame `{}` \
             has no content-addressed hash entry, so a portable FunctionRef \
             cannot be constructed. ADR-006 §2.7.4.",
            caller.function_name
        ));
    };

    Ok(TypedReturn::SomeObjectPairs(vec![
        (
            "name".to_string(),
            ConcreteReturn::String(caller.function_name),
        ),
        (
            "hash".to_string(),
            ConcreteReturn::String(hex::encode(hash_bytes)),
        ),
    ]))
}

/// `state.args() -> Array<any>`
pub(crate) fn state_args_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.args"));
    };
    project_current_args(vm_state.current_args())
}

/// `state.locals() -> Map<string, any>`
pub(crate) fn state_locals_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.locals"));
    };
    project_current_locals(vm_state.current_locals())
}
