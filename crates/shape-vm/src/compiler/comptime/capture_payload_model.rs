//! Typed comptime payload model for generated closure captures.

use shape_ast::ast::{
    CaptureMode, EnumDef, EnumMember, EnumMemberKind, Item, Span, StructField, StructTypeDef,
    TypeAnnotation,
};
use shape_runtime::comptime_reflection::{
    CAPTURE_DESCRIPTOR_SCHEMA_NAME, CAPTURE_MODE_SCHEMA_NAME,
};

pub(super) fn capture_mode_enum_item() -> Item {
    Item::Enum(
        EnumDef {
            name: CAPTURE_MODE_SCHEMA_NAME.to_string(),
            doc_comment: None,
            type_params: None,
            members: CaptureMode::ALL
                .into_iter()
                .map(|mode| EnumMember {
                    name: mode.variant_name().to_string(),
                    kind: EnumMemberKind::Unit { value: None },
                    span: Span::DUMMY,
                    doc_comment: None,
                })
                .collect(),
            annotations: Vec::new(),
        },
        Span::DUMMY,
    )
}

pub(super) fn capture_descriptor_struct_item() -> Item {
    let integer = || TypeAnnotation::Basic("int".to_string());
    let field = |name: &str, type_annotation: TypeAnnotation| StructField {
        annotations: Vec::new(),
        is_comptime: false,
        name: name.to_string(),
        span: Span::DUMMY,
        doc_comment: None,
        type_annotation,
        default_value: None,
    };
    Item::StructType(
        StructTypeDef {
            name: CAPTURE_DESCRIPTOR_SCHEMA_NAME.to_string(),
            doc_comment: None,
            type_params: None,
            fields: vec![
                field("signature_identity_high", integer()),
                field("signature_identity_low", integer()),
                field("index", integer()),
                field("type_identity_high", integer()),
                field("type_identity_low", integer()),
                field(
                    "mode",
                    TypeAnnotation::Basic(CAPTURE_MODE_SCHEMA_NAME.to_string()),
                ),
            ],
            methods: Vec::new(),
            annotations: Vec::new(),
            native_layout: None,
        },
        Span::DUMMY,
    )
}
