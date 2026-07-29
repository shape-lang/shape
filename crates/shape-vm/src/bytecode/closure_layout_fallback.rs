use std::sync::Arc;

use thiserror::Error;

use super::{FunctionBlob, FunctionHash, Program};
use crate::type_tracking::NativeKind;
use shape_value::HeapKind;
use shape_value::v2::ConcreteType;
use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};

/// Why a transferred closure blob cannot be materialized without compiler-only
/// layout side tables.
#[derive(Debug, Error)]
pub enum TransferredClosureLayoutError {
    #[error("function blob is not marked as a closure")]
    NotClosure,
    #[error("declares {declared} capture(s) but carries {kinds} capture kind(s)")]
    CaptureKindCountMismatch { declared: usize, kinds: usize },
    #[error("declares {declared} capture(s) but carries {flags} mutable-capture flag(s)")]
    MutableCaptureCountMismatch { declared: usize, flags: usize },
    #[error("declares {captures} capture(s), exceeding the 64-capture layout limit")]
    CaptureLimitExceeded { captures: usize },
    #[error("declares captures but has no frame descriptor")]
    MissingFrameDescriptor,
    #[error("frame descriptor has {slots} slot(s) but needs a {captures}-slot capture prefix")]
    FrameDescriptorTooShort { slots: usize, captures: usize },
    #[error("frame descriptor capture slot {index} is {descriptor:?}, not {capture:?}")]
    FrameDescriptorCaptureMismatch {
        index: usize,
        descriptor: NativeKind,
        capture: NativeKind,
    },
    #[error("capture {index} is mutable")]
    MutableCapture { index: usize },
    #[error("capture {index} uses unsupported kind {kind:?}")]
    UnsupportedCaptureKind { index: usize, kind: NativeKind },
}

/// A closure-layout reconstruction failure associated with the received blob.
#[derive(Debug)]
pub struct TransferredClosureLayoutLinkError {
    pub blob: FunctionHash,
    pub source: TransferredClosureLayoutError,
}

/// Remap compiler-owned layouts when available, otherwise reconstruct the
/// supported immutable transferred-closure layout from blob metadata.
pub fn remap_closure_function_layouts(
    program: &Program,
    blobs: &[&FunctionBlob],
) -> Result<Vec<Option<Arc<ClosureLayout>>>, TransferredClosureLayoutLinkError> {
    blobs
        .iter()
        .map(|blob| {
            if let Some(layout) = program
                .closure_function_layouts_by_name
                .get(&blob.name)
                .cloned()
            {
                return Ok(Some(layout));
            }
            if !blob.is_closure {
                return Ok(None);
            }
            reconstruct_transferred_closure_layout(blob)
                .map(Arc::new)
                .map(Some)
                .map_err(|source| TransferredClosureLayoutLinkError {
                    blob: blob.content_hash,
                    source,
                })
        })
        .collect()
}

/// Reconstruct the layout for an immutable closure transferred without the
/// compiler-only layout side table.
pub fn reconstruct_transferred_closure_layout(
    blob: &FunctionBlob,
) -> Result<ClosureLayout, TransferredClosureLayoutError> {
    if !blob.is_closure {
        return Err(TransferredClosureLayoutError::NotClosure);
    }

    let captures = usize::from(blob.captures_count);
    if blob.capture_kinds.len() != captures {
        return Err(TransferredClosureLayoutError::CaptureKindCountMismatch {
            declared: captures,
            kinds: blob.capture_kinds.len(),
        });
    }
    if blob.mutable_captures.len() != captures {
        return Err(TransferredClosureLayoutError::MutableCaptureCountMismatch {
            declared: captures,
            flags: blob.mutable_captures.len(),
        });
    }
    if captures > 64 {
        return Err(TransferredClosureLayoutError::CaptureLimitExceeded { captures });
    }

    match &blob.frame_descriptor {
        Some(descriptor) if descriptor.slots.len() < captures => {
            return Err(TransferredClosureLayoutError::FrameDescriptorTooShort {
                slots: descriptor.slots.len(),
                captures,
            });
        }
        Some(descriptor) => {
            for (index, (descriptor_kind, capture_kind)) in descriptor
                .slots
                .iter()
                .zip(&blob.capture_kinds)
                .take(captures)
                .enumerate()
            {
                if descriptor_kind != capture_kind {
                    return Err(
                        TransferredClosureLayoutError::FrameDescriptorCaptureMismatch {
                            index,
                            descriptor: *descriptor_kind,
                            capture: *capture_kind,
                        },
                    );
                }
            }
        }
        None if captures != 0 => {
            return Err(TransferredClosureLayoutError::MissingFrameDescriptor);
        }
        None => {}
    }

    let mut capture_types = Vec::with_capacity(captures);
    for (index, kind) in blob.capture_kinds.iter().copied().enumerate() {
        if blob.mutable_captures[index] {
            return Err(TransferredClosureLayoutError::MutableCapture { index });
        }
        capture_types.push(representative_capture_type(index, kind)?);
    }

    let capture_kinds = vec![CaptureKind::Immutable; captures];
    Ok(ClosureLayout::from_capture_types_with_native_kinds(
        &capture_types,
        &capture_kinds,
        &blob.capture_kinds,
    ))
}

fn representative_capture_type(
    index: usize,
    kind: NativeKind,
) -> Result<ConcreteType, TransferredClosureLayoutError> {
    let ty = match kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => ConcreteType::F64,
        NativeKind::Float32 => ConcreteType::F32,
        NativeKind::Char => ConcreteType::Char,
        NativeKind::Int8 | NativeKind::NullableInt8 => ConcreteType::I8,
        NativeKind::UInt8 | NativeKind::NullableUInt8 => ConcreteType::U8,
        NativeKind::Int16 | NativeKind::NullableInt16 => ConcreteType::I16,
        NativeKind::UInt16 | NativeKind::NullableUInt16 => ConcreteType::U16,
        NativeKind::Int32 | NativeKind::NullableInt32 => ConcreteType::I32,
        NativeKind::UInt32 | NativeKind::NullableUInt32 => ConcreteType::U32,
        NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::IntSize
        | NativeKind::NullableIntSize => ConcreteType::I64,
        NativeKind::UInt64
        | NativeKind::NullableUInt64
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => ConcreteType::U64,
        NativeKind::Bool => ConcreteType::Bool,
        NativeKind::String => ConcreteType::String,
        NativeKind::StringV2 | NativeKind::DecimalV2 => {
            ConcreteType::Pointer(Box::new(ConcreteType::Void))
        }
        NativeKind::Ptr(heap_kind) => match heap_kind {
            HeapKind::Closure
            | HeapKind::Reference
            | HeapKind::SharedCell
            | HeapKind::IoHandle
            | HeapKind::Future
            | HeapKind::TaskGroup => {
                return Err(TransferredClosureLayoutError::UnsupportedCaptureKind { index, kind });
            }
            HeapKind::String
            | HeapKind::TypedObject
            | HeapKind::Decimal
            | HeapKind::BigInt
            | HeapKind::DataTable
            | HeapKind::TypedArray
            | HeapKind::Temporal
            | HeapKind::TableView
            | HeapKind::Content
            | HeapKind::Instant
            | HeapKind::NativeScalar
            | HeapKind::NativeView
            | HeapKind::Char
            | HeapKind::HashMap
            | HeapKind::FilterExpr
            | HeapKind::ForeignRef
            | HeapKind::HashSet
            | HeapKind::Iterator
            | HeapKind::Deque
            | HeapKind::Channel
            | HeapKind::PriorityQueue
            | HeapKind::Range
            | HeapKind::Result
            | HeapKind::Option
            | HeapKind::TraitObject
            | HeapKind::Mutex
            | HeapKind::Atomic
            | HeapKind::Lazy
            | HeapKind::ModuleFn
            | HeapKind::Matrix
            | HeapKind::MatrixSlice => ConcreteType::Pointer(Box::new(ConcreteType::Void)),
        },
        NativeKind::Null => {
            return Err(TransferredClosureLayoutError::UnsupportedCaptureKind { index, kind });
        }
    };
    Ok(ty)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use shape_abi_v1::PermissionSet;
    use shape_value::v2::struct_layout::FieldKind;

    use super::*;
    use crate::bytecode::{Constant, DebugInfo, FunctionHash, Instruction, SourceMap};
    use crate::type_tracking::FrameDescriptor;

    fn closure_blob(capture_kinds: Vec<NativeKind>) -> FunctionBlob {
        let captures_count = capture_kinds.len() as u16;
        let mut blob = FunctionBlob {
            content_hash: FunctionHash::ZERO,
            name: "nested".into(),
            arity: 0,
            param_names: vec![],
            locals_count: captures_count,
            is_closure: true,
            captures_count,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![false; usize::from(captures_count)],
            frame_descriptor: (captures_count != 0).then(|| {
                FrameDescriptor::from_slots(
                    capture_kinds.clone(),
                    crate::type_tracking::FrameReturnWrapper::Plain,
                )
            }),
            capture_kinds,
            capture_names: vec![],
            instructions: Vec::<Instruction>::new(),
            constants: Vec::<Constant>::new(),
            strings: vec![],
            required_permissions: PermissionSet::pure(),
            dependencies: vec![],
            callee_names: vec![],
            type_schemas: vec![],
            foreign_dependencies: vec![],
            source_map: vec![],
        };
        blob.finalize();
        blob
    }

    fn program_with(blob: &FunctionBlob) -> Program {
        let mut function_store = HashMap::new();
        function_store.insert(blob.content_hash, blob.clone());
        Program {
            entry: blob.content_hash,
            function_store,
            top_level_locals_count: 0,
            top_level_local_storage_hints: vec![],
            module_binding_names: vec![],
            module_binding_storage_hints: vec![],
            function_local_storage_hints: vec![],
            top_level_frame: None,
            top_level_local_concrete_types: vec![],
            function_local_concrete_types: vec![],
            function_return_concrete_types: vec![],
            monomorphized_method_call_sites: HashMap::new(),
            value_call_return_concrete_types: HashMap::new(),
            operator_trait_dispatch_sites: HashMap::new(),
            data_schema: None,
            type_schema_registry: Default::default(),
            trait_method_symbols: HashMap::new(),
            foreign_functions: vec![],
            native_struct_layouts: vec![],
            debug_info: DebugInfo {
                source_map: SourceMap {
                    files: vec![],
                    source_texts: vec![],
                },
                line_numbers: vec![],
                variable_names: vec![],
                source_text: String::new(),
            },
            closure_function_layouts_by_name: HashMap::new(),
            trait_vtables: HashMap::new(),
            has_imported_const_inline: false,
            has_w17_marshal_residual: false,
            has_try_unwrap_residual: false,
            has_reference_escape_promotion: false,
            has_null_coalesce_residual: false,
        }
    }

    #[test]
    fn zero_capture_closure_needs_no_frame_descriptor() {
        let blob = closure_blob(vec![]);

        let layout = reconstruct_transferred_closure_layout(&blob).unwrap();

        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.owned_mutable_capture_mask, 0);
        assert_eq!(layout.shared_capture_mask, 0);
    }

    #[test]
    fn compiler_layout_is_authoritative_over_transferred_metadata() {
        let blob = closure_blob(vec![NativeKind::Int64]);
        let mut program = program_with(&blob);
        let compiler_layout = Arc::new(ClosureLayout::from_capture_types(
            &[ConcreteType::F64],
            &[CaptureKind::Immutable],
        ));
        program
            .closure_function_layouts_by_name
            .insert(blob.name.clone(), compiler_layout.clone());

        let layouts = remap_closure_function_layouts(&program, &[&blob]).unwrap();

        assert!(Arc::ptr_eq(layouts[0].as_ref().unwrap(), &compiler_layout));
    }

    #[test]
    fn scalar_and_pointer_captures_preserve_native_kinds() {
        let blob = closure_blob(vec![
            NativeKind::Int32,
            NativeKind::Ptr(HeapKind::TypedObject),
        ]);

        let layout = reconstruct_transferred_closure_layout(&blob).unwrap();

        assert_eq!(layout.capture_native_kinds, blob.capture_kinds);
        assert_eq!(layout.capture_kind(0), FieldKind::I32);
        assert_eq!(layout.capture_kind(1), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 1 << 1);
    }

    #[test]
    fn malformed_capture_metadata_is_rejected() {
        let mut blob = closure_blob(vec![NativeKind::Int64]);
        blob.capture_kinds.clear();
        assert!(matches!(
            reconstruct_transferred_closure_layout(&blob),
            Err(TransferredClosureLayoutError::CaptureKindCountMismatch { .. })
        ));

        let mut blob = closure_blob(vec![NativeKind::Int64]);
        blob.mutable_captures.clear();
        assert!(matches!(
            reconstruct_transferred_closure_layout(&blob),
            Err(TransferredClosureLayoutError::MutableCaptureCountMismatch { .. })
        ));

        let mut blob = closure_blob(vec![NativeKind::Int64]);
        blob.frame_descriptor = None;
        assert!(matches!(
            reconstruct_transferred_closure_layout(&blob),
            Err(TransferredClosureLayoutError::MissingFrameDescriptor)
        ));

        let mut blob = closure_blob(vec![NativeKind::Int64]);
        blob.frame_descriptor = Some(FrameDescriptor::from_slots(
            vec![NativeKind::Bool],
            crate::type_tracking::FrameReturnWrapper::Plain,
        ));
        assert!(matches!(
            reconstruct_transferred_closure_layout(&blob),
            Err(TransferredClosureLayoutError::FrameDescriptorCaptureMismatch { .. })
        ));
    }

    #[test]
    fn mutable_captures_are_rejected() {
        let mut blob = closure_blob(vec![NativeKind::Int64]);
        blob.mutable_captures = vec![true];

        assert!(matches!(
            reconstruct_transferred_closure_layout(&blob),
            Err(TransferredClosureLayoutError::MutableCapture { index: 0 })
        ));
    }

    #[test]
    fn forbidden_capture_kinds_are_rejected() {
        for kind in [
            HeapKind::Closure,
            HeapKind::Reference,
            HeapKind::SharedCell,
            HeapKind::IoHandle,
            HeapKind::Future,
            HeapKind::TaskGroup,
        ] {
            let blob = closure_blob(vec![NativeKind::Ptr(kind)]);
            assert!(matches!(
                reconstruct_transferred_closure_layout(&blob),
                Err(TransferredClosureLayoutError::UnsupportedCaptureKind { .. })
            ));
        }
    }

    #[test]
    fn layout_metadata_is_hash_covered() {
        let blob = closure_blob(vec![NativeKind::Int64]);
        let hash = blob.compute_hash();

        let mut mutated = blob.clone();
        mutated.is_closure = false;
        assert_ne!(mutated.compute_hash(), hash);

        let mut mutated = blob.clone();
        mutated.capture_kinds[0] = NativeKind::Float64;
        assert_ne!(mutated.compute_hash(), hash);

        let mut mutated = blob.clone();
        mutated.mutable_captures[0] = true;
        assert_ne!(mutated.compute_hash(), hash);

        let mut mutated = blob.clone();
        mutated.frame_descriptor = Some(FrameDescriptor::from_slots(
            vec![NativeKind::Bool],
            crate::type_tracking::FrameReturnWrapper::Plain,
        ));
        assert_ne!(mutated.compute_hash(), hash);
    }
}
