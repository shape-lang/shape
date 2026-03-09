//! Type system definitions for Shape AST

use super::DocComment;
use super::functions::Annotation;
use super::span::Span;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeAnnotation {
    /// Basic types: number, string, bool, row, pattern, etc.
    Basic(String),
    /// Vector type: T[] or Vec<T>
    Array(Box<TypeAnnotation>),
    /// Tuple type: [T1, T2, T3]
    Tuple(Vec<TypeAnnotation>),
    /// Object type: { field1: T1, field2?: T2 }
    Object(Vec<ObjectTypeField>),
    /// Function type: (T1, T2) => T3
    Function {
        params: Vec<FunctionParam>,
        returns: Box<TypeAnnotation>,
    },
    /// Union type: T1 | T2 | T3 (discriminated union - value is ONE of the types)
    Union(Vec<TypeAnnotation>),
    /// Intersection type: T1 + T2 (structural merge - value has ALL fields from both types)
    /// Only valid for object/interface types. Field collisions are compile-time errors.
    Intersection(Vec<TypeAnnotation>),
    /// Generic type: Map<K, V>
    Generic {
        name: String,
        args: Vec<TypeAnnotation>,
    },
    /// Type reference (custom type or type alias)
    Reference(String),
    /// Void type
    Void,
    /// Never type
    Never,
    /// Null type
    Null,
    /// Undefined type
    Undefined,
    /// Trait object type: dyn Trait1 + Trait2
    /// Represents a type-erased value that implements the given traits
    Dyn(Vec<String>),
}

impl TypeAnnotation {
    pub fn option(inner: TypeAnnotation) -> Self {
        TypeAnnotation::Generic {
            name: "Option".to_string(),
            args: vec![inner],
        }
    }

    pub fn option_inner(&self) -> Option<&TypeAnnotation> {
        match self {
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                args.first()
            }
            _ => None,
        }
    }

    pub fn into_option_inner(self) -> Option<TypeAnnotation> {
        match self {
            TypeAnnotation::Generic { name, mut args } if name == "Option" && args.len() == 1 => {
                Some(args.remove(0))
            }
            _ => None,
        }
    }

    pub fn is_option(&self) -> bool {
        self.option_inner().is_some()
    }

    /// Extract a simple type name if this is a Reference or Basic type
    ///
    /// Returns `Some(type_name)` for:
    /// - `TypeAnnotation::Reference(name)` - e.g., `Currency`, `MyType`
    /// - `TypeAnnotation::Basic(name)` - e.g., `number`, `string`
    ///
    /// Returns `None` for complex types like arrays, tuples, functions, etc.
    pub fn as_simple_name(&self) -> Option<&str> {
        match self {
            TypeAnnotation::Reference(name) => Some(name.as_str()),
            TypeAnnotation::Basic(name) => Some(name.as_str()),
            _ => None,
        }
    }

    /// Convert a type annotation to its full string representation.
    pub fn to_type_string(&self) -> String {
        match self {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
            TypeAnnotation::Array(inner) => format!("Array<{}>", inner.to_type_string()),
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                format!("{}?", args[0].to_type_string())
            }
            TypeAnnotation::Generic { name, args } => {
                let args_str: Vec<String> = args.iter().map(|a| a.to_type_string()).collect();
                format!("{}<{}>", name, args_str.join(", "))
            }
            TypeAnnotation::Tuple(items) => {
                let items_str: Vec<String> = items.iter().map(|t| t.to_type_string()).collect();
                format!("[{}]", items_str.join(", "))
            }
            TypeAnnotation::Union(items) => {
                let items_str: Vec<String> = items.iter().map(|t| t.to_type_string()).collect();
                items_str.join(" | ")
            }
            TypeAnnotation::Void => "void".to_string(),
            TypeAnnotation::Never => "never".to_string(),
            TypeAnnotation::Null => "null".to_string(),
            TypeAnnotation::Undefined => "undefined".to_string(),
            TypeAnnotation::Object(fields) => {
                let fields_str: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let opt = if f.optional { "?" } else { "" };
                        // Include @alias in the type string so the Python extension
                        // can use the wire name when looking up dict keys.
                        let alias = f
                            .annotations
                            .iter()
                            .find(|a| a.name == "alias")
                            .and_then(|a| a.args.first())
                            .and_then(|arg| match arg {
                                super::expressions::Expr::Literal(
                                    super::literals::Literal::String(s),
                                    _,
                                ) => Some(s.as_str()),
                                _ => None,
                            });
                        let alias_str = alias.map(|a| format!("@\"{}\" ", a)).unwrap_or_default();
                        format!(
                            "{}{}{}: {}",
                            alias_str,
                            f.name,
                            opt,
                            f.type_annotation.to_type_string()
                        )
                    })
                    .collect();
                format!("{{{}}}", fields_str.join(", "))
            }
            _ => "any".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectTypeField {
    pub name: String,
    pub optional: bool,
    pub type_annotation: TypeAnnotation,
    /// Field annotations (e.g. `@alias("wire name")`)
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionParam {
    pub name: Option<String>,
    pub optional: bool,
    pub type_annotation: TypeAnnotation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeParam {
    pub name: String,
    #[serde(default)]
    pub span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    /// Default type argument: `T = int`
    pub default_type: Option<TypeAnnotation>,
    /// Trait bounds: `T: Comparable + Displayable`
    #[serde(default)]
    pub trait_bounds: Vec<String>,
}

/// A predicate in a where clause: `T: Comparable + Display`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WherePredicate {
    pub type_name: String,
    pub bounds: Vec<String>,
}

impl PartialEq for TypeParam {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.doc_comment == other.doc_comment
            && self.default_type == other.default_type
            && self.trait_bounds == other.trait_bounds
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeAliasDef {
    pub name: String,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub type_annotation: TypeAnnotation,
    /// Meta parameter overrides: type Percent4 = Percent { decimals: 4 }
    pub meta_param_overrides: Option<std::collections::HashMap<String, super::expressions::Expr>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceDef {
    pub name: String,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub members: Vec<InterfaceMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InterfaceMember {
    /// Property signature
    Property {
        name: String,
        optional: bool,
        type_annotation: TypeAnnotation,
        #[serde(default)]
        span: Span,
        #[serde(default)]
        doc_comment: Option<DocComment>,
    },
    /// Method signature
    Method {
        name: String,
        optional: bool,
        params: Vec<FunctionParam>,
        return_type: TypeAnnotation,
        /// Whether this is an async method
        is_async: bool,
        #[serde(default)]
        span: Span,
        #[serde(default)]
        doc_comment: Option<DocComment>,
    },
    /// Index signature
    IndexSignature {
        param_name: String,
        param_type: String, // "string" or "number"
        return_type: TypeAnnotation,
        #[serde(default)]
        span: Span,
        #[serde(default)]
        doc_comment: Option<DocComment>,
    },
}

impl InterfaceMember {
    pub fn span(&self) -> Span {
        match self {
            InterfaceMember::Property { span, .. }
            | InterfaceMember::Method { span, .. }
            | InterfaceMember::IndexSignature { span, .. } => *span,
        }
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        match self {
            InterfaceMember::Property { doc_comment, .. }
            | InterfaceMember::Method { doc_comment, .. }
            | InterfaceMember::IndexSignature { doc_comment, .. } => doc_comment.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDef {
    pub name: String,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub members: Vec<EnumMember>,
    /// Annotations applied to the enum (e.g., `@with_label() enum Color { ... }`)
    #[serde(default)]
    pub annotations: Vec<super::Annotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumMember {
    pub name: String,
    pub kind: EnumMemberKind,
    #[serde(default)]
    pub span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnumMemberKind {
    /// Unit variant: Variant or Variant = 1
    Unit { value: Option<EnumValue> },
    /// Tuple variant: Variant(Type, Type)
    Tuple(Vec<TypeAnnotation>),
    /// Struct variant: Variant { field: Type, ... }
    Struct(Vec<ObjectTypeField>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EnumValue {
    String(String),
    Number(f64),
}

/// A member of a trait definition: either required (signature only) or default (with body)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraitMember {
    /// Required method — implementors must provide this
    Required(InterfaceMember),
    /// Default method — used if implementor does not override
    Default(MethodDef),
    /// Associated type declaration: `type Item;` or `type Item: Comparable;`
    AssociatedType {
        name: String,
        bounds: Vec<TypeAnnotation>,
        #[serde(default)]
        span: Span,
        #[serde(default)]
        doc_comment: Option<DocComment>,
    },
}

impl TraitMember {
    pub fn span(&self) -> Span {
        match self {
            TraitMember::Required(member) => member.span(),
            TraitMember::Default(method) => method.span,
            TraitMember::AssociatedType { span, .. } => *span,
        }
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        match self {
            TraitMember::Required(member) => member.doc_comment(),
            TraitMember::Default(method) => method.doc_comment.as_ref(),
            TraitMember::AssociatedType { doc_comment, .. } => doc_comment.as_ref(),
        }
    }
}

/// A concrete binding for an associated type inside an `impl` block:
/// `type Item = number;`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedTypeBinding {
    pub name: String,
    pub concrete_type: TypeAnnotation,
}

/// Trait definition — like interface but with `trait` keyword, supporting default methods
///
/// ```shape
/// trait Queryable<T> {
///     filter(predicate: (T) => bool): Self    // required
///     method execute() -> Result<Table<T>> {   // default
///         return Ok(self.filter(|_| true))
///     }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitDef {
    pub name: String,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub members: Vec<TraitMember>,
    /// Annotations applied to the trait (e.g., `@documented("...") trait Foo { ... }`)
    #[serde(default)]
    pub annotations: Vec<super::Annotation>,
}

/// Impl block — implements a trait for a type
///
/// ```shape
/// impl Queryable<T> for Table<T> {
///     method filter(predicate) { /* ... */ }
///     method execute() { Ok(self) }
/// }
/// ```
///
/// Under the hood, compiles identically to an extend block (UFCS desugaring)
/// plus trait validation (all required methods present with correct arities).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplBlock {
    /// The trait being implemented (e.g., Queryable<T>)
    pub trait_name: TypeName,
    /// The type implementing the trait (e.g., Table<T>)
    pub target_type: TypeName,
    /// Optional named implementation selector:
    /// `impl Display for User as JsonDisplay { ... }`
    pub impl_name: Option<String>,
    /// Method implementations
    pub methods: Vec<MethodDef>,
    /// Associated type bindings: `type Item = number;`
    pub associated_type_bindings: Vec<AssociatedTypeBinding>,
    /// Where clause: `where T: Display + Comparable`
    pub where_clause: Option<Vec<WherePredicate>>,
}

/// Type extension for adding methods to existing types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtendStatement {
    /// The type being extended (e.g., "Vec")
    pub type_name: TypeName,
    /// Methods being added to the type
    pub methods: Vec<MethodDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodDef {
    /// Method name
    pub name: String,
    #[serde(default)]
    pub span: Span,
    /// Declaring module path for compiler/runtime provenance checks.
    ///
    /// This is injected by the module loader for loaded modules and is not part
    /// of user-authored source syntax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaring_module_path: Option<String>,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    /// Annotations applied to this method (e.g., `@traced`)
    #[serde(default)]
    pub annotations: Vec<super::functions::Annotation>,
    /// Method parameters
    pub params: Vec<super::functions::FunctionParameter>,
    /// Optional when clause for conditional method definitions
    pub when_clause: Option<Box<super::expressions::Expr>>,
    /// Optional return type annotation
    pub return_type: Option<TypeAnnotation>,
    /// Method body
    pub body: Vec<super::statements::Statement>,
    /// Whether this is an async method
    pub is_async: bool,
}

impl PartialEq for MethodDef {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.doc_comment == other.doc_comment
            && self.annotations == other.annotations
            && self.params == other.params
            && self.when_clause == other.when_clause
            && self.return_type == other.return_type
            && self.body == other.body
            && self.is_async == other.is_async
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeName {
    /// Simple type name (e.g., "Vec", "Table")
    Simple(String),
    /// Generic type name (e.g., "Table<Row>")
    Generic {
        name: String,
        type_args: Vec<TypeAnnotation>,
    },
}

// ============================================================================
// Struct Type Definitions
// ============================================================================

/// Struct type definition — pure data with named fields
///
/// ```shape
/// type Point { x: number, y: number }
/// type DataVec<V, K = Timestamp> { index: Vec<K>, data: Vec<V> }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructTypeDef {
    pub name: String,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub fields: Vec<StructField>,
    /// Inline method definitions inside the type body
    #[serde(default)]
    pub methods: Vec<MethodDef>,
    /// Annotations applied to the struct (e.g., `@derive_debug type Foo { ... }`)
    pub annotations: Vec<Annotation>,
    /// Optional native layout metadata for `type C`.
    #[serde(default)]
    pub native_layout: Option<NativeLayoutBinding>,
}

/// Native layout binding metadata for `type C` declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeLayoutBinding {
    /// ABI name (currently `"C"`).
    pub abi: String,
}

/// A field in a struct type definition
///
/// Comptime fields are type-level constants baked at compile time.
/// They occupy zero runtime slots (no ValueSlot in TypedObject).
/// Access resolves to a constant push at compile time.
///
/// ```shape
/// type Currency {
///     comptime symbol: string = "$",
///     comptime decimals: number = 2,
///     amount: number,
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructField {
    pub annotations: Vec<Annotation>,
    pub is_comptime: bool,
    pub name: String,
    #[serde(default)]
    pub span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_annotation: TypeAnnotation,
    pub default_value: Option<super::expressions::Expr>,
}

impl PartialEq for StructField {
    fn eq(&self, other: &Self) -> bool {
        self.annotations == other.annotations
            && self.is_comptime == other.is_comptime
            && self.name == other.name
            && self.doc_comment == other.doc_comment
            && self.type_annotation == other.type_annotation
            && self.default_value == other.default_value
    }
}
