//! The foreign marshaling table — the canonical set of Shape declared types
//! that can cross a language-runtime boundary.
//!
//! ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196).
//!
//! # Why this lives in the ABI crate
//!
//! Three parties must agree on exactly one set of crossable types:
//!
//! 1. **the compiler**, which rejects a declaration whose parameter or return
//!    type has no mapping ([`ForeignFunctionDef::unmapped_foreign_types`] in
//!    `shape-ast`) — the diagnostic lands at the declaration, never at a
//!    marshal-time `NotImplemented`;
//! 2. **the marshal layer**
//!    (`shape-vm/src/executor/control_flow/foreign_marshal.rs`), which projects
//!    values onto the msgpack wire per the classified type;
//! 3. **the extensions**, which render `.pyi` / `.d.ts` stubs for the declared
//!    contract.
//!
//! `shape-abi-v1` is the only crate all three can see (the extensions link
//! nothing else of ours), so the table lives here and the extensions never
//! parse a Shape type spelling — the host classifies, the extension renders.
//!
//! # Mechanical coverage
//!
//! [`ForeignTypeShape`] is the constructor witness for [`ForeignType`]. A new
//! marshal shape forces:
//!
//! - a compile error in every renderer that matches on `ForeignType`
//!   exhaustively (the two stub generators), and
//! - a failure of the `marshal_table_covers_every_shape` assertions, because
//!   [`marshal_table`] is checked against [`ForeignTypeShape::ALL`].
//!
//! Adding a scalar to [`ForeignScalar::ALL`] likewise widens the generated
//! table in every constructor that ranges over scalars.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use std::string::String;
use std::string::ToString;
use std::vec::Vec;

/// A scalar Shape type with a direct wire projection.
///
/// This is the closed set the marshal layer projects without a schema:
/// `foreign_marshal::kinded_slot_to_msgpack` on the way out and
/// `foreign_marshal::msgpack_to_kinded_slot` on the way back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ForeignScalar {
    /// `int` — msgpack integer.
    Int,
    /// `number` (alias `float`) — msgpack float.
    Number,
    /// `bool` — msgpack boolean.
    Bool,
    /// `string` — msgpack string.
    String,
    /// `none` / `()` — msgpack nil. Return position only in practice; a
    /// parameter of this type carries no information but is not refused here.
    Unit,
}

impl ForeignScalar {
    /// Every scalar in the table. Ranged over by [`marshal_table`], so a new
    /// scalar widens the per-type assertions automatically.
    pub const ALL: &'static [ForeignScalar] = &[
        ForeignScalar::Int,
        ForeignScalar::Number,
        ForeignScalar::Bool,
        ForeignScalar::String,
        ForeignScalar::Unit,
    ];

    /// The canonical Shape spelling. Round-trips through
    /// [`ForeignType::classify`].
    pub fn shape_spelling(self) -> &'static str {
        match self {
            ForeignScalar::Int => "int",
            ForeignScalar::Number => "number",
            ForeignScalar::Bool => "bool",
            ForeignScalar::String => "string",
            ForeignScalar::Unit => "none",
        }
    }

    /// The `BUFFER_ELEM_*` code for an `Array<Self>` that can be exported as a
    /// zero-copy view, or `None` for a scalar whose array cannot be shared
    /// (ADR-019 §2 / #199).
    ///
    /// The set is `int` and `number` and stops there, and the two exclusions are
    /// soundness, not effort:
    ///
    /// - `bool` is stored one byte per element with only `0` and `1` valid. A
    ///   mutable view would let foreign code write `7` into a slot the compiler
    ///   has proven holds a Shape `bool`, producing a value that is neither
    ///   `true` nor `false`. There is no read-only-only mode worth the
    ///   asymmetry, so the element type is out entirely.
    /// - `string` arrays hold `*const StringObj` — host pointers with host
    ///   refcounts. Exporting the buffer would export the heap, and every
    ///   pointer in it would outlive the call's pin.
    pub fn buffer_elem(self) -> Option<u32> {
        match self {
            ForeignScalar::Int => Some(crate::BUFFER_ELEM_INT64),
            ForeignScalar::Number => Some(crate::BUFFER_ELEM_FLOAT64),
            ForeignScalar::Bool | ForeignScalar::String | ForeignScalar::Unit => None,
        }
    }
}

/// How one declared parameter's backing buffer crosses a foreign call
/// (ADR-019 §2 / #199).
///
/// Written by the author in the Shape declaration, never inferred: sharing moves
/// the memory-safety boundary from a handful of trusted extension crates to
/// anyone who writes an inline fence, so ADR-019 §2 requires the widening to be
/// visible in the source. That is why there is a spelling at all.
///
/// # Why this lives here and not in the AST
///
/// The three parties named at the top of this module all have to agree on it:
/// the compiler decides which declarations may carry it, the marshal layer
/// decides whether to build a view or walk the elements, and the stub renderers
/// decide whether the foreign signature says `list[float]` or a buffer type. A
/// second copy of the enum in the AST would be a parallel discriminator over the
/// same three values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum BufferShare {
    /// The ordinary boundary: the value is deep-copied onto the MessagePack
    /// wire. The default, and the only mode legal outside a `fn <language>`
    /// declaration.
    #[default]
    Copied,
    /// `shared x` — an immutable shared-borrow view over the host buffer,
    /// valid for the duration of one call.
    Shared,
    /// `shared mut x` — an exclusive mutable-borrow view over the host buffer,
    /// valid for the duration of one call.
    SharedMut,
}

impl BufferShare {
    /// Every mode. A value-level witness, so a new mode widens the per-mode
    /// assertions instead of slipping past them.
    pub const ALL: &'static [BufferShare] = &[
        BufferShare::Copied,
        BufferShare::Shared,
        BufferShare::SharedMut,
    ];

    /// Whether this mode exports a view at all.
    pub fn is_shared(self) -> bool {
        !matches!(self, BufferShare::Copied)
    }

    /// Whether foreign code may write through the view.
    pub fn is_mutable(self) -> bool {
        matches!(self, BufferShare::SharedMut)
    }

    /// The source spelling that selects this mode. `None` for the default,
    /// which has no spelling because it is what writing nothing means.
    pub fn spelling(self) -> Option<&'static str> {
        match self {
            BufferShare::Copied => None,
            BufferShare::Shared => Some("shared"),
            BufferShare::SharedMut => Some("shared mut"),
        }
    }

    /// The `BUFFER_MODE_*` bit this mode asks an extension for; `None` for the
    /// copied default, which asks for nothing.
    pub fn abi_mode(self) -> Option<u32> {
        match self {
            BufferShare::Copied => None,
            BufferShare::Shared => Some(crate::BUFFER_MODE_SHARED),
            BufferShare::SharedMut => Some(crate::BUFFER_MODE_SHARED_MUT),
        }
    }
}

/// A field of an object-shaped foreign type, carried so a stub renderer can
/// emit a class / interface without seeing the Shape schema registry.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ForeignField {
    /// Field name as the foreign side sees it (the `@alias` wire name when one
    /// was declared, otherwise the Shape field name).
    pub name: String,
    /// The field's own crossable type.
    pub ty: ForeignType,
    /// Whether the field may be absent.
    pub optional: bool,
}

/// A Shape type that can cross the foreign boundary.
///
/// Constructed only by [`ForeignType::classify`] on the host side; the
/// extensions receive it already classified through the stub channel.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ForeignType {
    /// A scalar: `int`, `number`, `bool`, `string`, `none`.
    Scalar(ForeignScalar),
    /// `Array<T>` / `Vec<T>` with a scalar element.
    ///
    /// Non-scalar elements are deliberately absent: `typed_array_to_msgpack`
    /// and `build_scalar_typed_array` both monomorphize on the declared scalar
    /// element, and there is no `Array<Struct>` projection.
    Array(ForeignScalar),
    /// `HashMap<string, V>` with a scalar `V`.
    ///
    /// **Argument position only** — `hashmap_to_msgpack` projects one out, but
    /// there is no inverse; see [`ForeignType::supports`].
    Map(ForeignScalar),
    /// `Option<T>` / `T?`.
    Optional(Box<ForeignType>),
    /// A named `type` declaration or an inline object literal.
    Object {
        /// Declared type name, or `None` for an inline `{ a: int }` literal.
        name: Option<String>,
        /// Fields, when the host knew them at classification time. Spelling-only
        /// classification leaves this `None`; the contract export enriches it
        /// from the schema registry.
        fields: Option<Vec<ForeignField>>,
    },
}

/// The constructor witness for [`ForeignType`].
///
/// Exists so [`marshal_table`] can be asserted complete against a value-level
/// enumeration: adding a `ForeignType` variant without extending the table is
/// a test failure, not a silent gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignTypeShape {
    Scalar,
    Array,
    Map,
    Optional,
    Object,
}

impl ForeignTypeShape {
    /// Every constructor of [`ForeignType`].
    pub const ALL: &'static [ForeignTypeShape] = &[
        ForeignTypeShape::Scalar,
        ForeignTypeShape::Array,
        ForeignTypeShape::Map,
        ForeignTypeShape::Optional,
        ForeignTypeShape::Object,
    ];
}

/// Which side of a call a type appears on.
///
/// The table is not symmetric: `HashMap<string, V>` has an outbound projection
/// and no inbound one, so it is legal as a parameter and refused as a return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignDirection {
    /// Shape value → foreign value (a parameter).
    Argument,
    /// Foreign value → Shape value (the return).
    Return,
}

/// Why a declared type cannot cross the boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct UnmappedForeignType {
    /// The exact Shape spelling that has no mapping. For a compound type this
    /// is the offending *inner* spelling (`Measurement` in
    /// `Array<Measurement>`), so the diagnostic names the real problem.
    pub spelling: String,
    /// The full declared spelling the classification started from.
    pub declared: String,
    /// Machine-independent explanation, phrased for a declaration-site
    /// diagnostic.
    pub reason: UnmappedReason,
}

/// The distinct ways a declared type falls outside the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmappedReason {
    /// A builtin Shape type with no wire projection at all (`DataTable`,
    /// `decimal`, `DateTime`, …).
    NoWireProjection,
    /// `Array<T>` / `HashMap<string, V>` whose element/value type is not a
    /// scalar.
    NonScalarElement,
    /// `HashMap<K, V>` whose key type is not `string`.
    NonStringMapKey,
    /// A type constructor that has no foreign meaning (tuple, function type,
    /// union, borrow, `dyn Trait`, …).
    UnsupportedConstructor,
    /// The type is mapped, but not in this direction.
    WrongDirection,
    /// A generic type used with the wrong arity (`Option<A, B>`).
    BadArity,
}

impl UnmappedReason {
    /// A sentence fragment completing "… because {}".
    pub fn explain(self) -> &'static str {
        match self {
            UnmappedReason::NoWireProjection => {
                "it has no foreign wire projection — the marshaling table covers \
                 int / number / bool / string / none, arrays and maps over those, \
                 optionals, and object types"
            }
            UnmappedReason::NonScalarElement => {
                "only scalar elements cross inside a container: \
                 Array<int>, Array<number>, Array<bool>, Array<string>"
            }
            UnmappedReason::NonStringMapKey => "a foreign map key must be `string`",
            UnmappedReason::UnsupportedConstructor => {
                "this type constructor has no foreign representation"
            }
            UnmappedReason::WrongDirection => {
                "`HashMap<string, V>` can be passed into a foreign function but \
                 cannot be returned from one — there is no inbound projection"
            }
            UnmappedReason::BadArity => "the generic type was given the wrong number of arguments",
        }
    }
}

/// Builtin Shape types that exist but have no foreign wire projection.
///
/// Kept explicit so the diagnostic can distinguish "this Shape type cannot
/// cross" from "I do not know this name" (a user type, which classifies as an
/// object and is checked by ordinary type resolution).
///
/// Every entry here is a type the marshal layer answers with
/// `VMError::NotImplemented`; the list is the declaration-time mirror of those
/// arms.
const NO_WIRE_PROJECTION: &[&str] = &[
    "decimal",
    "Decimal",
    "bigint",
    "BigInt",
    "char",
    "Char",
    "DateTime",
    "Date",
    "Duration",
    "Instant",
    "DataTable",
    "Table",
    "TableView",
    "Matrix",
    "Set",
    "HashSet",
    "Deque",
    "Channel",
    "Iterator",
    "Range",
    "Future",
    "Content",
    "ptr",
    "Ptr",
    "void",
    "never",
    "undefined",
    "any",
];

impl ForeignType {
    /// Classify a declared Shape type spelling.
    ///
    /// `declared` is the output of `TypeAnnotation::to_type_string()` — the same
    /// string the compiler stores in `ForeignFunctionEntry::param_types` and the
    /// marshal layer dispatches on, so all three consumers classify identically.
    ///
    /// In [`ForeignDirection::Return`] position a single `Result<T>` /
    /// `Result<T, E>` wrapper is stripped first (dynamic runtimes mandate it),
    /// matching `foreign_marshal::strip_result_wrapper`.
    pub fn classify(
        declared: &str,
        direction: ForeignDirection,
    ) -> Result<ForeignType, UnmappedForeignType> {
        let trimmed = declared.trim();
        let inner = if direction == ForeignDirection::Return {
            strip_result_wrapper(trimmed)
        } else {
            trimmed
        };
        classify_inner(inner, trimmed, direction, true)
    }

    /// The constructor witness for this value.
    pub fn shape(&self) -> ForeignTypeShape {
        match self {
            ForeignType::Scalar(_) => ForeignTypeShape::Scalar,
            ForeignType::Array(_) => ForeignTypeShape::Array,
            ForeignType::Map(_) => ForeignTypeShape::Map,
            ForeignType::Optional(_) => ForeignTypeShape::Optional,
            ForeignType::Object { .. } => ForeignTypeShape::Object,
        }
    }

    /// Whether this type crosses in `direction`.
    pub fn supports(&self, direction: ForeignDirection) -> bool {
        match self {
            ForeignType::Map(_) => direction == ForeignDirection::Argument,
            ForeignType::Optional(inner) => inner.supports(direction),
            _ => true,
        }
    }

    /// The canonical Shape spelling for this type.
    ///
    /// [`ForeignType::classify`] on this string yields an equal value — the
    /// round-trip the per-type stub assertions ride on.
    pub fn shape_spelling(&self) -> String {
        match self {
            ForeignType::Scalar(s) => s.shape_spelling().to_string(),
            ForeignType::Array(s) => alloc_format(&["Array<", s.shape_spelling(), ">"]),
            ForeignType::Map(s) => alloc_format(&["HashMap<string, ", s.shape_spelling(), ">"]),
            ForeignType::Optional(inner) => {
                alloc_format(&["Option<", &inner.shape_spelling(), ">"])
            }
            ForeignType::Object { name, fields } => match name {
                Some(n) => n.clone(),
                None => match fields {
                    Some(fs) => {
                        let mut out = String::from("{");
                        for (i, f) in fs.iter().enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            out.push_str(&f.name);
                            if f.optional {
                                out.push('?');
                            }
                            out.push_str(": ");
                            out.push_str(&f.ty.shape_spelling());
                        }
                        out.push('}');
                        out
                    }
                    None => String::from("{}"),
                },
            },
        }
    }
}

fn alloc_format(parts: &[&str]) -> String {
    let mut out = String::new();
    for p in parts {
        out.push_str(p);
    }
    out
}

/// Strip one `Result<...>` wrapper. Mirrors
/// `foreign_marshal::strip_result_wrapper`.
fn strip_result_wrapper(s: &str) -> &str {
    if let Some(inner) = s
        .strip_prefix("Result<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        // `Result<T, E>` — the error arm is the host's string channel and
        // never crosses; classify `T`.
        return match split_top_level(inner) {
            Some((first, _)) => first.trim(),
            None => inner.trim(),
        };
    }
    s
}

/// Split `A, B` at the first top-level comma (nesting-aware).
fn split_top_level(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '<' | '{' | '[' | '(' => depth += 1,
            '>' | '}' | ']' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

fn scalar_from_spelling(s: &str) -> Option<ForeignScalar> {
    match s {
        "int" | "Int" | "i64" => Some(ForeignScalar::Int),
        "number" | "Number" | "float" | "Float" | "f64" => Some(ForeignScalar::Number),
        "bool" | "Bool" => Some(ForeignScalar::Bool),
        "string" | "String" | "str" => Some(ForeignScalar::String),
        "none" | "Unit" | "()" | "null" => Some(ForeignScalar::Unit),
        _ => None,
    }
}

fn unmapped(
    spelling: &str,
    declared: &str,
    reason: UnmappedReason,
) -> Result<ForeignType, UnmappedForeignType> {
    Err(UnmappedForeignType {
        spelling: spelling.to_string(),
        declared: declared.to_string(),
        reason,
    })
}

fn classify_inner(
    s: &str,
    declared: &str,
    direction: ForeignDirection,
    top_level: bool,
) -> Result<ForeignType, UnmappedForeignType> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(ForeignType::Scalar(ForeignScalar::Unit));
    }

    // `T?` optional sugar. Checked before scalars so `int?` does not read as a
    // name. Guarded against `??` exactly as `strip_option_inner` is.
    if let Some(head) = s
        .strip_suffix('?')
        .filter(|head| !head.is_empty() && !head.ends_with('?'))
    {
        let inner = classify_inner(head, declared, direction, false)?;
        return Ok(ForeignType::Optional(Box::new(inner)));
    }

    if let Some(scalar) = scalar_from_spelling(s) {
        return Ok(ForeignType::Scalar(scalar));
    }

    // Generic forms.
    if let Some((head, args)) = split_generic(s) {
        return match head {
            "Option" => match args.len() {
                1 => {
                    let inner = classify_inner(args[0], declared, direction, false)?;
                    Ok(ForeignType::Optional(Box::new(inner)))
                }
                _ => unmapped(s, declared, UnmappedReason::BadArity),
            },
            "Array" | "Vec" => match args.len() {
                1 => match scalar_from_spelling(args[0].trim()) {
                    Some(elem) => Ok(ForeignType::Array(elem)),
                    None => unmapped(args[0].trim(), declared, UnmappedReason::NonScalarElement),
                },
                _ => unmapped(s, declared, UnmappedReason::BadArity),
            },
            "HashMap" | "Map" => match args.len() {
                2 => {
                    if scalar_from_spelling(args[0].trim()) != Some(ForeignScalar::String) {
                        return unmapped(args[0].trim(), declared, UnmappedReason::NonStringMapKey);
                    }
                    let value = match scalar_from_spelling(args[1].trim()) {
                        Some(v) if v != ForeignScalar::Unit => v,
                        _ => {
                            return unmapped(
                                args[1].trim(),
                                declared,
                                UnmappedReason::NonScalarElement,
                            );
                        }
                    };
                    if direction == ForeignDirection::Return {
                        return unmapped(s, declared, UnmappedReason::WrongDirection);
                    }
                    Ok(ForeignType::Map(value))
                }
                _ => unmapped(s, declared, UnmappedReason::BadArity),
            },
            "Result" if top_level => {
                // A nested `Result` (or one in argument position) has no
                // projection: the outer wrapper is the runtime's error channel
                // and is stripped by `classify`, never here.
                unmapped(s, declared, UnmappedReason::UnsupportedConstructor)
            }
            _ => unmapped(s, declared, UnmappedReason::UnsupportedConstructor),
        };
    }

    // Inline object literal `{ a: int, b?: string }`.
    if s.starts_with('{') && s.ends_with('}') {
        let fields = parse_object_fields(&s[1..s.len() - 1], declared, direction)?;
        return Ok(ForeignType::Object {
            name: None,
            fields: Some(fields),
        });
    }

    // Constructors with no foreign meaning.
    if s.starts_with('[')
        || s.starts_with('&')
        || s.starts_with("dyn ")
        || s.contains("=>")
        || s.contains('|')
        || s.contains('+')
    {
        return unmapped(s, declared, UnmappedReason::UnsupportedConstructor);
    }

    if NO_WIRE_PROJECTION.contains(&s) {
        return unmapped(s, declared, UnmappedReason::NoWireProjection);
    }

    // A plain name: a user `type` declaration. It crosses as an object; whether
    // the name resolves at all is ordinary type resolution's job, not ours.
    if s.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
    {
        return Ok(ForeignType::Object {
            name: Some(s.to_string()),
            fields: None,
        });
    }

    unmapped(s, declared, UnmappedReason::UnsupportedConstructor)
}

/// Split `Name<A, B>` into `("Name", ["A", "B"])`.
fn split_generic(s: &str) -> Option<(&str, Vec<&str>)> {
    let open = s.find('<')?;
    if !s.ends_with('>') {
        return None;
    }
    let head = &s[..open];
    let body = &s[open + 1..s.len() - 1];
    let mut args = Vec::new();
    let mut rest = body;
    while let Some((first, tail)) = split_top_level(rest) {
        args.push(first.trim());
        rest = tail;
    }
    args.push(rest.trim());
    Some((head.trim(), args))
}

/// Parse the body of an inline object-literal type string.
///
/// Field spellings come from `TypeAnnotation::to_type_string()`, which renders
/// `@"wire_name" field?: Type`. The alias, when present, is the name the
/// foreign side sees, so it wins.
fn parse_object_fields(
    body: &str,
    declared: &str,
    direction: ForeignDirection,
) -> Result<Vec<ForeignField>, UnmappedForeignType> {
    let mut fields = Vec::new();
    let mut rest = body.trim();
    if rest.is_empty() {
        return Ok(fields);
    }
    loop {
        let (chunk, tail) = match split_top_level(rest) {
            Some((c, t)) => (c, Some(t)),
            None => (rest, None),
        };
        let chunk = chunk.trim();
        if !chunk.is_empty() {
            let (name_part, type_part) = match split_field(chunk) {
                Some(pair) => pair,
                None => {
                    return Err(UnmappedForeignType {
                        spelling: chunk.to_string(),
                        declared: declared.to_string(),
                        reason: UnmappedReason::UnsupportedConstructor,
                    });
                }
            };
            let mut name = name_part.trim();
            // `@"alias" field` — the alias is the wire name.
            if let Some(after_at) = name.strip_prefix('@') {
                let after_at = after_at.trim_start();
                if let Some((quoted, _)) = after_at
                    .strip_prefix('"')
                    .and_then(|rest| rest.split_once('"'))
                {
                    name = quoted;
                }
            }
            let optional = name.ends_with('?');
            let name = name.trim_end_matches('?').trim();
            let ty = classify_inner(type_part.trim(), declared, direction, false)?;
            fields.push(ForeignField {
                name: name.to_string(),
                ty,
                optional,
            });
        }
        match tail {
            Some(t) => rest = t,
            None => break,
        }
    }
    Ok(fields)
}

/// Split `name: Type` at the first top-level colon.
fn split_field(chunk: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let bytes = chunk.as_bytes();
    for (i, c) in chunk.char_indices() {
        match c {
            '<' | '{' | '[' | '(' => depth += 1,
            '>' | '}' | ']' | ')' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => {
                // Skip `::` path separators.
                if bytes.get(i + 1) == Some(&b':') {
                    continue;
                }
                if i > 0 && bytes[i - 1] == b':' {
                    continue;
                }
                return Some((&chunk[..i], &chunk[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

// ============================================================================
// The stub channel payload
// ============================================================================

/// Wire version of [`ForeignContractExport`].
///
/// Bumped when the payload shape changes. An extension that reads a version it
/// does not know must refuse rather than guess — a misread contract produces a
/// wrong stub, which is worse than no stub.
pub const FOREIGN_CONTRACT_VERSION: u32 = 1;

/// One declared parameter of a foreign function.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ForeignParamContract {
    /// Parameter name, as the foreign body binds it.
    pub name: String,
    /// The parameter's classified type.
    pub ty: ForeignType,
    /// How the value crosses (ADR-019 §2 / #199).
    ///
    /// The renderer must honour it: a `shared` `Array<number>` reaches the body
    /// as a buffer view, not as a list, and a stub that says `list[float]` for
    /// it would document a type the body never receives.
    #[cfg_attr(feature = "serde", serde(default))]
    pub share: BufferShare,
}

/// One declared foreign function, classified for stub rendering.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ForeignFunctionContract {
    /// Function name as declared in Shape.
    pub name: String,
    /// Parameters in call order.
    pub params: Vec<ForeignParamContract>,
    /// The return type, with the runtime's `Result<T>` wrapper already
    /// stripped — the foreign body returns `T`, and failure is the runtime's
    /// own exception channel.
    pub returns: ForeignType,
}

/// The declared Shape contract for one language, delivered to an extension
/// through `LanguageRuntimeVTable::register_types`.
///
/// ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196). Everything the extension
/// needs to render a `.pyi` / `.d.ts` is here in classified form: the extension
/// never parses a Shape type spelling, and the host never renders a foreign
/// one.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ForeignContractExport {
    /// [`FOREIGN_CONTRACT_VERSION`] at the time of writing.
    pub version: u32,
    /// The language this contract is for (`"python"`, `"typescript"`, …).
    pub language: String,
    /// Every foreign function declared for this language in the program.
    pub functions: Vec<ForeignFunctionContract>,
    /// Named object types referenced by the functions above, in declaration
    /// order, so a renderer can emit each class once before it is used.
    pub types: Vec<ForeignType>,
}

impl ForeignContractExport {
    /// A contract for `language` at the current wire version.
    pub fn new(language: impl Into<String>) -> Self {
        ForeignContractExport {
            version: FOREIGN_CONTRACT_VERSION,
            language: language.into(),
            functions: Vec::new(),
            types: Vec::new(),
        }
    }

    /// Refuse a payload written by a host the extension does not understand.
    pub fn check_version(&self) -> Result<(), String> {
        if self.version == FOREIGN_CONTRACT_VERSION {
            Ok(())
        } else {
            Err(alloc_format(&[
                "foreign contract version ",
                &self.version.to_string(),
                " is not supported (this extension speaks version ",
                &FOREIGN_CONTRACT_VERSION.to_string(),
                ")",
            ]))
        }
    }
}

/// The marshaling table: one representative [`ForeignType`] per constructor,
/// ranged over every scalar where a constructor takes one.
///
/// This is what the per-type stub assertions iterate. It is generated rather
/// than hand-listed so a new [`ForeignScalar`] widens it automatically, and it
/// is checked against [`ForeignTypeShape::ALL`] so a new constructor cannot be
/// added without extending it.
pub fn marshal_table() -> Vec<ForeignType> {
    let mut table = Vec::new();
    for &s in ForeignScalar::ALL {
        table.push(ForeignType::Scalar(s));
    }
    for &s in ForeignScalar::ALL {
        if s == ForeignScalar::Unit {
            continue; // `Array<none>` has no element projection.
        }
        table.push(ForeignType::Array(s));
    }
    for &s in ForeignScalar::ALL {
        if s == ForeignScalar::Unit {
            continue; // `HashMap<string, none>` carries no value.
        }
        table.push(ForeignType::Map(s));
    }
    for &s in ForeignScalar::ALL {
        if s == ForeignScalar::Unit {
            continue; // `Option<none>` is `none`.
        }
        table.push(ForeignType::Optional(Box::new(ForeignType::Scalar(s))));
    }
    table.push(ForeignType::Optional(Box::new(ForeignType::Array(
        ForeignScalar::Int,
    ))));
    table.push(ForeignType::Object {
        name: Some(String::from("Candle")),
        fields: Some(vec![
            ForeignField {
                name: String::from("open"),
                ty: ForeignType::Scalar(ForeignScalar::Number),
                optional: false,
            },
            ForeignField {
                name: String::from("label"),
                ty: ForeignType::Scalar(ForeignScalar::String),
                optional: true,
            },
        ]),
    });
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshal_table_covers_every_shape() {
        let table = marshal_table();
        for shape in ForeignTypeShape::ALL {
            assert!(
                table.iter().any(|t| t.shape() == *shape),
                "marshaling table has no entry for constructor {:?} — add one to \
                 `marshal_table()` so the per-type stub assertions cover it",
                shape
            );
        }
    }

    #[test]
    fn marshal_table_covers_every_scalar() {
        let table = marshal_table();
        for scalar in ForeignScalar::ALL {
            assert!(
                table
                    .iter()
                    .any(|t| matches!(t, ForeignType::Scalar(s) if s == scalar)),
                "marshaling table has no scalar entry for {:?}",
                scalar
            );
        }
    }

    #[test]
    fn every_table_entry_round_trips_through_its_shape_spelling() {
        for entry in marshal_table() {
            let direction = if entry.supports(ForeignDirection::Return) {
                ForeignDirection::Return
            } else {
                ForeignDirection::Argument
            };
            let spelling = entry.shape_spelling();
            let reparsed = ForeignType::classify(&spelling, direction)
                .unwrap_or_else(|e| panic!("`{}` failed to re-classify: {:?}", spelling, e));
            // Object field detail is not recoverable from a bare name; compare
            // the constructor and the spelling instead.
            assert_eq!(
                reparsed.shape(),
                entry.shape(),
                "`{}` re-classified to a different constructor",
                spelling
            );
            assert_eq!(
                reparsed.shape_spelling(),
                spelling,
                "`{}` did not round-trip",
                spelling
            );
        }
    }

    #[test]
    fn classifies_the_primitive_spellings_the_marshal_layer_accepts() {
        for (spelling, expected) in [
            ("int", ForeignScalar::Int),
            ("Int", ForeignScalar::Int),
            ("number", ForeignScalar::Number),
            ("float", ForeignScalar::Number),
            ("Number", ForeignScalar::Number),
            ("Float", ForeignScalar::Number),
            ("bool", ForeignScalar::Bool),
            ("Bool", ForeignScalar::Bool),
            ("string", ForeignScalar::String),
            ("String", ForeignScalar::String),
            ("none", ForeignScalar::Unit),
            ("()", ForeignScalar::Unit),
            ("Unit", ForeignScalar::Unit),
        ] {
            assert_eq!(
                ForeignType::classify(spelling, ForeignDirection::Argument),
                Ok(ForeignType::Scalar(expected)),
                "`{}` should classify as {:?}",
                spelling,
                expected
            );
        }
    }

    #[test]
    fn strips_one_result_wrapper_in_return_position_only() {
        assert_eq!(
            ForeignType::classify("Result<int>", ForeignDirection::Return),
            Ok(ForeignType::Scalar(ForeignScalar::Int))
        );
        assert_eq!(
            ForeignType::classify("Result<int, string>", ForeignDirection::Return),
            Ok(ForeignType::Scalar(ForeignScalar::Int))
        );
        assert_eq!(
            ForeignType::classify("Result<int>", ForeignDirection::Argument)
                .unwrap_err()
                .reason,
            UnmappedReason::UnsupportedConstructor
        );
    }

    #[test]
    fn optional_sugar_and_generic_form_agree() {
        let sugar = ForeignType::classify("int?", ForeignDirection::Return).unwrap();
        let generic = ForeignType::classify("Option<int>", ForeignDirection::Return).unwrap();
        assert_eq!(sugar, generic);
        assert_eq!(
            sugar,
            ForeignType::Optional(Box::new(ForeignType::Scalar(ForeignScalar::Int)))
        );
    }

    #[test]
    fn array_of_struct_is_unmapped_at_the_element() {
        let err = ForeignType::classify("Array<Measurement>", ForeignDirection::Argument)
            .expect_err("Array<Struct> has no element projection");
        assert_eq!(err.spelling, "Measurement");
        assert_eq!(err.reason, UnmappedReason::NonScalarElement);
    }

    #[test]
    fn hashmap_is_argument_only() {
        let arg = ForeignType::classify("HashMap<string, int>", ForeignDirection::Argument);
        assert_eq!(arg, Ok(ForeignType::Map(ForeignScalar::Int)));
        let ret = ForeignType::classify("HashMap<string, int>", ForeignDirection::Return)
            .expect_err("HashMap has no inbound projection");
        assert_eq!(ret.reason, UnmappedReason::WrongDirection);
        assert!(!ForeignType::Map(ForeignScalar::Int).supports(ForeignDirection::Return));
    }

    #[test]
    fn builtin_types_without_a_projection_are_named_as_such() {
        for spelling in ["DataTable", "decimal", "DateTime", "bigint", "char"] {
            match ForeignType::classify(spelling, ForeignDirection::Argument) {
                Ok(mapped) => panic!("`{}` classified as mapped: {:?}", spelling, mapped),
                Err(err) => {
                    assert_eq!(err.reason, UnmappedReason::NoWireProjection);
                    assert_eq!(err.spelling, spelling);
                }
            }
        }
    }

    #[test]
    fn user_named_types_classify_as_objects() {
        assert_eq!(
            ForeignType::classify("Measurement", ForeignDirection::Return),
            Ok(ForeignType::Object {
                name: Some(String::from("Measurement")),
                fields: None
            })
        );
    }

    #[test]
    fn inline_object_literals_carry_their_fields() {
        let t = ForeignType::classify("{open: number, label?: string}", ForeignDirection::Return)
            .expect("object literal classifies");
        match t {
            ForeignType::Object { name, fields } => {
                assert!(name.is_none());
                let fields = fields.expect("literal carries fields");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "open");
                assert_eq!(fields[0].ty, ForeignType::Scalar(ForeignScalar::Number));
                assert!(!fields[0].optional);
                assert_eq!(fields[1].name, "label");
                assert!(fields[1].optional);
            }
            other => panic!("expected an object, got {:?}", other),
        }
    }

    #[test]
    fn object_literal_alias_becomes_the_wire_name() {
        let t = ForeignType::classify("{@\"wire_name\" field: int}", ForeignDirection::Return)
            .expect("aliased literal classifies");
        match t {
            ForeignType::Object { fields, .. } => {
                let fields = fields.expect("literal carries fields");
                assert_eq!(fields[0].name, "wire_name");
            }
            other => panic!("expected an object, got {:?}", other),
        }
    }

    #[test]
    fn unsupported_constructors_are_refused() {
        for spelling in ["[int, string]", "&int", "dyn Shape", "int | string"] {
            let err = ForeignType::classify(spelling, ForeignDirection::Argument)
                .expect_err("constructor has no foreign representation");
            assert_eq!(err.reason, UnmappedReason::UnsupportedConstructor);
        }
    }
}
