//! ADR-009 C3 #14 (slice 3) — the ConstLift domain core: how declared capture
//! VALUES cross from a hook-template construction site into a specialized
//! handler as baked constants. This module is the ONE bake producer.
//!
//! # The S3 domain (C3-G5 — compositional)
//!
//! Liftable = the four scalar primitives (`int`, `number`, `bool`, `string`)
//! plus arrays, homogeneous bracket-tuples, and `Option` of liftables,
//! RECURSIVELY ([`lift_value`] over the [`LiftedConst`] domain). At the value
//! level an `Array<T>` and a homogeneous bracket-tuple are ONE v2-raw
//! `TypedArray` carrier, so both lift to [`LiftedConst::Array`]; the DECLARED
//! capture-parameter annotation distinguishes the spellings
//! ([`LiftedConst::matches_annotation`]). Never-liftable per C3-G5 / Dec 95 —
//! references, resources, capabilities, functions, provider grants, compiler
//! descriptors, secrets, runtime handles — reject with the NAMED class arm;
//! everything else out-of-domain rejects naming the kind. Every producer
//! carries the closed-domain sentence [`CONST_LIFT_DOMAIN_SENTENCE`] verbatim
//! plus a positive twin (C3-G13 string-tag message text with the #60 routing
//! posture — S3 mints NO C09xx codes; S5 owns minting from C0931+).
//!
//! `unit` is named-rejected into the domain sentence rather than
//! half-supported: no `NativeKind` unit variant exists at the value tier, so
//! no unit value can reach the capture seam at all — the sentence's own
//! conditional names unit for the declaration-site reader.
//!
//! # Staging within S3 (S3a: dark-wired domain core)
//!
//! S3a lands the domain core proven by unit pins while the `capture()`
//! builtin's VALUE entry keeps its S2 scalar cascade byte-unchanged:
//! [`lift_capture_value`] stays the production entry (scalars only; its
//! rejection sentence still points at this module's S3 domain) until S3b
//! flips it to delegate to [`lift_value`] ATOMICALLY with rule-6 identity
//! ([`structural_key_segment`] entering the specialization key/symbol) and
//! composite capture delivery. The DECLARATION-SITE domain check
//! ([`annotation_within_lift_domain`], wired at
//! `CheckedTemplateBuilder::finish()`) is LIVE from S3a — it rejects only
//! declared-but-unusable capture-parameter types, which no green pin
//! exercises.
//!
//! # Identity vs display — SEPARATE functions, never conflated
//!
//! [`structural_key_segment`] is the Dec-95 rule-6 STRUCTURAL identity
//! rendering (netstring discipline: tagged, length/count-prefixed, injective
//! — see its docs for the argument). [`LiftedConst::render`] is DISPLAY ONLY
//! (registry rows, S8 hover) and is never parsed back or used as identity.
//! The `#64`/S1-verify-1 injectivity bug class (flat non-delimited joins) is
//! refuted by unit pins with non-vacuity controls in this module's tests.
//!
//! # Delivery (no second constant store)
//!
//! [`CaptureBindingPlan::CallSiteArgs`] delivers capture values as TYPED
//! AST expressions at the wrapper's handler call sites ([`LiftedConst::
//! to_expr`]): scalars as literals, composites as array literals /
//! `Some(...)` constructor calls riding the ESTABLISHED per-function
//! constant-pool and array/Option literal emission — never a parallel
//! constant store, and the compiler never emits the host-injection-only
//! `Constant::Value(KindedConstant)` (`bytecode/core_types.rs`). This also
//! kills the legacy per-invocation-config-eval disease and its W39
//! `LoadModuleBinding` JIT poison (slice-0 report §4 / §8 item 11): the
//! specialized handler and wrapper contain the VALUE, not a config read.
//! S1's monomorphization plan-guard (b) (`cache.rs`, the CONST-PARAM GUARD)
//! stays: a template plan reaching the const-generic reroute remains a named
//! internal error.
//!
//! # Naming (c3-decisions.md §Naming — binding)
//!
//! The Rust type is module-scoped `const_lift::LiftedConst` — never a bare
//! `ConstValue` (collision with the monomorphization `call_site_consts`
//! machinery), and never built on the dead `comptime_concrete::ConstantValue`
//! (its `Opaque` variant is ValueWord-shaped — a Forbidden-Patterns
//! defection). `ComptimeConstValue` / `const_value_mono_segment` are the
//! CONST-GENERIC surface and are NOT ridden here: `ComptimeConstValue` is
//! scalar-only and `const_value_mono_segment`'s string arm is LOSSY (its
//! sanitizer maps `"a b"` and `"a_b"` to one segment — the exact injectivity
//! bug class the refuter pins below prove against).

use shape_ast::ast::{Expr, Literal, Span, TypeAnnotation};
use shape_ast::error::ShapeError;
use shape_value::heap_value::OptionData;
use shape_value::v2::string_obj::StringObj;
use shape_value::{HeapKind, KindedSlot, NativeKind};

use super::SpecializationTarget;
use crate::compiler::comptime_builtins::BoundTemplate;

/// The closed-domain sentence (C3-G5), carried VERBATIM by every ConstLift
/// rejection producer — the lift arms, the declaration-site check, and the
/// `checked_template` finish()-time wiring all embed this one constant.
pub(in crate::compiler) const CONST_LIFT_DOMAIN_SENTENCE: &str =
    "the ConstLift domain is int, number, bool, and string values, plus arrays, homogeneous \
     tuples, and Option of liftable values, recursively (unit has no literal form and is not \
     liftable; a None/null value lifts only against an Option-typed capture parameter)";

/// A capture value lifted into the C3-G5 compositional constant domain.
///
/// [`LiftedConst::Array`] covers BOTH `Array<T>` values and homogeneous
/// bracket-tuple values — at the value level both are one v2-raw `TypedArray`
/// carrier; the declared annotation distinguishes the spellings. `LiftedConst`
/// is COMPILER-TIER data (ADR-005 §1): it never becomes a runtime carrier and
/// never grows into a parallel `HeapKind` discriminator.
///
/// Equality is STRUCTURAL with `f64` compared by BIT PATTERN (`to_bits`),
/// matching [`structural_key_segment`]'s rule-6 identity exactly — one
/// equality semantic, no `==`-vs-key divergence. `-0.0` is therefore distinct
/// from `0.0` (over-distinction is sound; disclosed); NaN never reaches
/// equality because non-finite numbers reject at lift.
#[derive(Debug, Clone)]
pub(in crate::compiler) enum LiftedConst {
    /// An `int` capture value.
    Int(i64),
    /// A finite `number` capture value.
    Number(f64),
    /// A `bool` capture value.
    Bool(bool),
    /// A `string` capture value (owned copy — the thread-local store outlives
    /// the mini-VM slot that carried it).
    String(String),
    /// An array or homogeneous bracket-tuple of liftable values.
    Array(Vec<LiftedConst>),
    /// A `Some(v)` Option value with a liftable payload.
    Some(Box<LiftedConst>),
    /// A `None` Option value (also the lift of a `NativeKind::Null` slot; it
    /// matches only an Option-typed capture parameter — see
    /// [`CONST_LIFT_DOMAIN_SENTENCE`]).
    None,
}

impl PartialEq for LiftedConst {
    /// Structural equality; `Number` compares by bit pattern (see the type
    /// docs — the one equality semantic, shared with
    /// [`structural_key_segment`]).
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LiftedConst::Int(a), LiftedConst::Int(b)) => a == b,
            (LiftedConst::Number(a), LiftedConst::Number(b)) => a.to_bits() == b.to_bits(),
            (LiftedConst::Bool(a), LiftedConst::Bool(b)) => a == b,
            (LiftedConst::String(a), LiftedConst::String(b)) => a == b,
            (LiftedConst::Array(a), LiftedConst::Array(b)) => a == b,
            (LiftedConst::Some(a), LiftedConst::Some(b)) => a == b,
            (LiftedConst::None, LiftedConst::None) => true,
            _ => false,
        }
    }
}

impl Eq for LiftedConst {}

impl LiftedConst {
    /// The scalar Shape type name, when this constant is a scalar.
    fn scalar_type_name(&self) -> Option<&'static str> {
        match self {
            LiftedConst::Int(_) => Some("int"),
            LiftedConst::Number(_) => Some("number"),
            LiftedConst::Bool(_) => Some("bool"),
            LiftedConst::String(_) => Some("string"),
            _ => Option::None,
        }
    }

    /// The Shape type spelling this constant inhabits — used by the
    /// value-vs-declared-type mismatch sentences (which keep their S2 shape,
    /// now naming composite types). Scalars render their exact name; a
    /// homogeneous array renders `Array<T>`, a heterogeneous one the
    /// bracket-tuple spelling, an empty one `Array<_>`; Options render
    /// `Option<T>` / `Option<_>`.
    pub(in crate::compiler) fn shape_type_name(&self) -> String {
        match self {
            LiftedConst::Int(_) | LiftedConst::Number(_) | LiftedConst::Bool(_)
            | LiftedConst::String(_) => self
                .scalar_type_name()
                .expect("scalar arm has a scalar name")
                .to_string(),
            LiftedConst::Array(elems) => {
                if elems.is_empty() {
                    return "Array<_>".to_string();
                }
                let first = elems[0].shape_type_name();
                if elems.iter().all(|e| e.shape_type_name() == first) {
                    format!("Array<{first}>")
                } else {
                    format!(
                        "[{}]",
                        elems
                            .iter()
                            .map(|e| e.shape_type_name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            LiftedConst::Some(inner) => format!("Option<{}>", inner.shape_type_name()),
            LiftedConst::None => "Option<_>".to_string(),
        }
    }

    /// Whether a declared capture-parameter annotation matches this
    /// constant's shape, COMPOSITIONALLY:
    ///
    /// - scalars match `Basic` / single-segment unqualified `Reference`
    ///   spellings of their exact name (the same single-segment rule
    ///   `checked_template::is_bare_type_param` uses); a type alias spelled
    ///   differently is a mismatch surfaced by the named rejection
    ///   (strictness over guessing);
    /// - an [`LiftedConst::Array`] value matches `Array(inner)` /
    ///   `Generic{Array, [T]}` when EVERY element matches the element type
    ///   (an empty array matches any Array spelling), and matches
    ///   `Tuple(elems)` when the lengths are equal and elements match
    ///   positionally;
    /// - [`LiftedConst::None`] matches any `Option<T>` annotation;
    ///   [`LiftedConst::Some`] matches `Option<T>` when the payload matches
    ///   `T`;
    /// - DECIDED-AND-PINNED: a `T` value offered where `Option<T>` is
    ///   declared is a MISMATCH at this seam (strictness over guessing — the
    ///   S2 posture; the user spells `Some(x)`).
    pub(in crate::compiler) fn matches_annotation(&self, annotation: &TypeAnnotation) -> bool {
        match self {
            LiftedConst::Int(_) | LiftedConst::Number(_) | LiftedConst::Bool(_)
            | LiftedConst::String(_) => {
                let name = match annotation {
                    TypeAnnotation::Basic(name) => name.as_str(),
                    TypeAnnotation::Reference(path) if !path.is_qualified() => path.name(),
                    _ => return false,
                };
                self.scalar_type_name() == Some(name)
            }
            LiftedConst::Array(elems) => match annotation {
                TypeAnnotation::Array(inner) => {
                    elems.iter().all(|e| e.matches_annotation(inner))
                }
                TypeAnnotation::Generic { name, args }
                    if name.as_str() == "Array" && args.len() == 1 =>
                {
                    elems.iter().all(|e| e.matches_annotation(&args[0]))
                }
                TypeAnnotation::Tuple(positions) => {
                    elems.len() == positions.len()
                        && elems
                            .iter()
                            .zip(positions)
                            .all(|(e, p)| e.matches_annotation(p))
                }
                _ => false,
            },
            LiftedConst::None => annotation.option_inner().is_some(),
            LiftedConst::Some(inner) => match annotation.option_inner() {
                Some(payload_ty) => inner.matches_annotation(payload_ty),
                Option::None => false,
            },
        }
    }

    /// A display rendering for the install registry (S2b — the S8
    /// hover/query substrate): the value as a user would spell it.
    /// DISPLAY ONLY — never parsed back, never identity; the rule-6
    /// identity rendering is [`structural_key_segment`], a separate
    /// function by design.
    pub(in crate::compiler) fn render(&self) -> String {
        match self {
            LiftedConst::Int(value) => value.to_string(),
            LiftedConst::Number(value) => value.to_string(),
            LiftedConst::Bool(value) => value.to_string(),
            LiftedConst::String(value) => format!("{value:?}"),
            LiftedConst::Array(elems) => format!(
                "[{}]",
                elems.iter().map(|e| e.render()).collect::<Vec<_>>().join(", ")
            ),
            LiftedConst::Some(inner) => format!("Some({})", inner.render()),
            LiftedConst::None => "None".to_string(),
        }
    }

    /// The typed AST expression the weave passes at a handler call site
    /// ([`CaptureBindingPlan::CallSiteArgs`]). Scalars project to literals;
    /// arrays project to `Expr::Array` literals and Options to the
    /// `Some(...)` constructor call (`BuiltinFunction::SomeCtor` resolves
    /// the name) / `Literal::None` — riding the ESTABLISHED literal
    /// emission paths, never a second constant store. AST-level only — no
    /// source text ever exists.
    pub(in crate::compiler) fn to_expr(&self) -> Expr {
        let span = Span::default();
        match self {
            LiftedConst::Int(value) => Expr::Literal(Literal::Int(*value), span),
            LiftedConst::Number(value) => Expr::Literal(Literal::Number(*value), span),
            LiftedConst::Bool(value) => Expr::Literal(Literal::Bool(*value), span),
            LiftedConst::String(value) => Expr::Literal(Literal::String(value.clone()), span),
            LiftedConst::Array(elems) => {
                Expr::Array(elems.iter().map(|e| e.to_expr()).collect(), span)
            }
            LiftedConst::Some(inner) => Expr::FunctionCall {
                name: "Some".to_string(),
                const_args: Vec::new(),
                args: vec![inner.to_expr()],
                named_args: Vec::new(),
                span,
            },
            LiftedConst::None => Expr::Literal(Literal::None, span),
        }
    }
}

/// The Dec-95 rule-6 STRUCTURAL identity rendering of a lifted constant —
/// the segment S3b enters into the specialization key and symbol (under the
/// arity-pinning `::cfg#{count}` head). Netstring discipline throughout:
/// every segment is tagged, strings are byte-length-prefixed, arrays are
/// count-prefixed — NEVER a flat join, never `Tuple::mono_key`, never
/// `const_value_mono_segment`.
///
/// # Injectivity argument
///
/// 1. Every segment begins `tag:':'` and neither `':'` nor `'#'` is an
///    identifier character, so no `ConcreteType` Display / type-name
///    rendering can produce a segment-shaped or `cfg#`-shaped token —
///    segment boundaries against the S1 Sig segments are unambiguous.
/// 2. Decoding is deterministic left-to-right: the tag selects the arm;
///    `i`/`n`/`b` consume a self-delimiting scalar body; `s` consumes
///    EXACTLY `byte_len` bytes (delimiter bytes inside string content are
///    inert — the length pins them); `a` consumes exactly `len` child
///    segments; `o` consumes zero (`o:n`) or one (`o:s:`) child. By
///    structural induction over [`LiftedConst`], distinct values render
///    distinct strings.
/// 3. The S3b `::cfg#{count}` head pins top-level arity, so value
///    boundaries cannot shift across capture positions (the S1 `::a{n}`
///    discipline extended).
///
/// `Number` renders its `f64::to_bits` pattern (`{:016x}`) — finite-only by
/// lift; bit-pattern rendering IS the structural equality (`-0.0` distinct
/// from `0.0`; sound over-distinction, disclosed).
// Dark until S3b: production wiring (the specialization key/symbol suffix)
// lands atomically with value acceptance and delivery in S3b; unit-proven
// here per the S3a charter.
#[allow(dead_code)]
pub(in crate::compiler) fn structural_key_segment(value: &LiftedConst) -> String {
    match value {
        LiftedConst::Int(i) => format!("i:{i}"),
        LiftedConst::Number(n) => format!("n:{:016x}", n.to_bits()),
        LiftedConst::Bool(true) => "b:t".to_string(),
        LiftedConst::Bool(false) => "b:f".to_string(),
        LiftedConst::String(s) => format!("s:{}:{}", s.len(), s),
        LiftedConst::Array(elems) => format!(
            "a:{}:[{}]",
            elems.len(),
            elems
                .iter()
                .map(structural_key_segment)
                .collect::<Vec<_>>()
                .join("::")
        ),
        LiftedConst::Some(inner) => format!("o:s:{}", structural_key_segment(inner)),
        LiftedConst::None => "o:n".to_string(),
    }
}

/// The out-of-domain rejection (C3-G13 string-tag + positive twin; the #60
/// routing posture — uncoded until #60's coded path lands, S3 mints no
/// C09xx). Names the offending kind and the closed domain.
fn out_of_domain_message(name: &str, kind_desc: &str) -> String {
    format!(
        "capture `{name}` holds a value outside the ConstLift domain (kind {kind_desc}); \
         {CONST_LIFT_DOMAIN_SENTENCE} — pass a liftable capture value"
    )
}

/// The never-liftable rejection (C3-G5 / Dec 95): the kind identifies one of
/// the closed never-liftable classes; the sentence names the class, the full
/// Dec-95 list, and the closed domain, with the positive twin.
fn never_liftable_message(name: &str, class: &str, kind_desc: &str) -> String {
    format!(
        "capture `{name}` holds a {class} value (kind {kind_desc}), which is never liftable \
         (C3-G5 / Dec-95): references, resources, capabilities, functions, provider grants, \
         compiler descriptors, secrets, and runtime handles cannot cross the comptime->runtime \
         stage boundary; {CONST_LIFT_DOMAIN_SENTENCE} — pass a liftable capture value"
    )
}

/// The non-finite-number rejection — byte-identical to the S2 sentence in
/// [`lift_capture_value`] (recursion-inherited: it fires inside composites
/// too). S3b's delegation collapses the two producers into this one.
fn non_finite_message(name: &str, n: f64) -> String {
    format!(
        "capture `{name}` holds a non-finite number ({n}); capture values must be \
         finite so they can be delivered as typed literals — pass a finite number"
    )
}

/// Lift one declared capture VALUE off the `KindedSlot` substrate into the
/// C3-G5 COMPOSITIONAL constant domain, or a NAMED rejection with a positive
/// twin — the S3 core.
///
/// Dispatch order: the exact-kind scalar cascade first (each accessor is
/// exact on `NativeKind` per ADR-006 §2.7.6 / Q8 — never fabricated from raw
/// bits; `as_str` covers both string carriers), then kind-witnessed
/// composite arms (`Null` → `None`; `Ptr(Option)` → recurse on the
/// `OptionData` payload; `Ptr(TypedArray)` → the elem-type-stamp walk), then
/// the never-liftable class arms, then the out-of-domain arm naming the
/// kind. Non-finite numbers reject at every depth.
// Dark until S3b: `lift_capture_value` below stays the production entry for
// the `capture()` builtin until S3b flips it to delegate here atomically
// with identity + delivery; unit-proven here per the S3a charter.
#[allow(dead_code)]
pub(in crate::compiler) fn lift_value(
    name: &str,
    value: &KindedSlot,
) -> Result<LiftedConst, String> {
    if let Some(s) = value.as_str() {
        return Ok(LiftedConst::String(s.to_string()));
    }
    if let Some(i) = value.as_i64() {
        return Ok(LiftedConst::Int(i));
    }
    if let Some(n) = value.as_f64() {
        if !n.is_finite() {
            return Err(non_finite_message(name, n));
        }
        return Ok(LiftedConst::Number(n));
    }
    if let Some(b) = value.as_bool() {
        return Ok(LiftedConst::Bool(b));
    }
    let kind = value.kind();
    let bits = value.slot().raw();
    match kind {
        NativeKind::Null => Ok(LiftedConst::None),
        NativeKind::Ptr(HeapKind::Option) if bits != 0 => {
            // SAFETY: `NativeKind::Ptr(HeapKind::Option)` is the kind witness
            // that `bits` are `Arc::into_raw(Arc<OptionData>)` (ADR-006
            // §2.7.17 / Q18); `value` owns one strong-count share for the
            // borrow duration. The payload is recursed by BORROW — no share
            // is claimed or released, so the walk is refcount-balanced by
            // construction (the deep-walk precedent's retain discipline,
            // satisfied with zero retains).
            let data: &OptionData = unsafe { &*(bits as *const OptionData) };
            if data.is_some {
                Ok(LiftedConst::Some(Box::new(lift_value(name, &data.payload)?)))
            } else {
                Ok(LiftedConst::None)
            }
        }
        NativeKind::Ptr(HeapKind::TypedArray) if bits != 0 => {
            lift_typed_array(name, bits as *const u8)
        }
        // Never-liftable class arms (C3-G5 / Dec 95) the kind identifies.
        // A named-function value is a bare function-id with runtime kind
        // `NativeKind::UInt64` (`PushConst(Constant::Function)`); closures
        // and module fns carry their own kinds.
        NativeKind::Ptr(HeapKind::Closure)
        | NativeKind::Ptr(HeapKind::ModuleFn)
        | NativeKind::UInt64 => Err(never_liftable_message(
            name,
            "function",
            &format!("{kind:?}"),
        )),
        NativeKind::Ptr(HeapKind::Reference) => Err(never_liftable_message(
            name,
            "reference",
            &format!("{kind:?}"),
        )),
        NativeKind::Ptr(
            HeapKind::DataTable
            | HeapKind::IoHandle
            | HeapKind::Channel
            | HeapKind::Mutex
            | HeapKind::TaskGroup
            | HeapKind::Future
            | HeapKind::Atomic,
        ) => Err(never_liftable_message(
            name,
            "runtime handle",
            &format!("{kind:?}"),
        )),
        NativeKind::Ptr(HeapKind::TypedObject) if bits != 0 => {
            match compiler_descriptor_schema_name(value) {
                Some(schema_name) => Err(never_liftable_message(
                    name,
                    &format!("compiler descriptor ({schema_name})"),
                    &format!("{kind:?}"),
                )),
                Option::None => Err(out_of_domain_message(name, &format!("{kind:?}"))),
            }
        }
        other => Err(out_of_domain_message(name, &format!("{other:?}"))),
    }
}

/// Typed-array arm of [`lift_value`]: walk elements through the elem-type
/// stamp, modeled EXACTLY on the established deep-walk precedent
/// (`typed_array_descriptor_lift_rejection`, `compiler/comptime.rs`): the
/// caller's kind witness proves the pointer, `read_elem_type` selects the
/// monomorphized layout, `TypedArray::<T>::as_slice` reads elements.
fn lift_typed_array(name: &str, array: *const u8) -> Result<LiftedConst, String> {
    use shape_value::v2::typed_array::{
        ELEM_TYPE_BOOL, ELEM_TYPE_CALLABLE, ELEM_TYPE_F64, ELEM_TYPE_I64, ELEM_TYPE_STRING,
        ELEM_TYPE_TYPED_ARRAY, TypedArray, read_elem_type,
    };
    // SAFETY (all blocks below): the caller's `NativeKind::Ptr(HeapKind::
    // TypedArray)` kind witness (plus its non-null guard) proves `array`
    // points to a live `TypedArray<T>`; the elem-type stamp selects the
    // monomorphized element layout BEFORE any element is read. Elements are
    // read by BORROW while the caller's slot owns the array share — no
    // element share is claimed, so the deep-walk retain discipline is
    // balanced with zero retains.
    let elem_type = unsafe { read_elem_type(array) };
    match elem_type {
        ELEM_TYPE_I64 => {
            let arr = array as *const TypedArray<i64>;
            let elems = unsafe { TypedArray::as_slice(arr) };
            Ok(LiftedConst::Array(
                elems.iter().map(|&v| LiftedConst::Int(v)).collect(),
            ))
        }
        ELEM_TYPE_F64 => {
            let arr = array as *const TypedArray<f64>;
            let elems = unsafe { TypedArray::as_slice(arr) };
            let mut lifted = Vec::with_capacity(elems.len());
            for &v in elems {
                if !v.is_finite() {
                    // Recursion-inherited: the existing non-finite sentence
                    // fires INSIDE composites too.
                    return Err(non_finite_message(name, v));
                }
                lifted.push(LiftedConst::Number(v));
            }
            Ok(LiftedConst::Array(lifted))
        }
        ELEM_TYPE_BOOL => {
            let arr = array as *const TypedArray<u8>;
            let elems = unsafe { TypedArray::as_slice(arr) };
            Ok(LiftedConst::Array(
                elems.iter().map(|&v| LiftedConst::Bool(v != 0)).collect(),
            ))
        }
        ELEM_TYPE_STRING => {
            let arr = array as *const TypedArray<*const StringObj>;
            let elems = unsafe { TypedArray::as_slice(arr) };
            let mut lifted = Vec::with_capacity(elems.len());
            for &elem in elems {
                if elem.is_null() {
                    // Fail-closed: a null element has no value to lift —
                    // never silently skipped (surface-and-stop).
                    return Err(out_of_domain_message(
                        name,
                        "Ptr(TypedArray) with a null string element",
                    ));
                }
                lifted.push(LiftedConst::String(
                    unsafe { StringObj::as_str(elem) }.to_string(),
                ));
            }
            Ok(LiftedConst::Array(lifted))
        }
        ELEM_TYPE_TYPED_ARRAY => {
            let arr = array as *const TypedArray<*const u8>;
            let elems = unsafe { TypedArray::as_slice(arr) };
            let mut lifted = Vec::with_capacity(elems.len());
            for &elem in elems {
                if elem.is_null() {
                    return Err(out_of_domain_message(
                        name,
                        "Ptr(TypedArray) with a null nested-array element",
                    ));
                }
                lifted.push(lift_typed_array(name, elem)?);
            }
            Ok(LiftedConst::Array(lifted))
        }
        ELEM_TYPE_CALLABLE => Err(never_liftable_message(
            name,
            "function",
            "Ptr(TypedArray) with callable elements",
        )),
        other => Err(out_of_domain_message(
            name,
            &format!("Ptr(TypedArray) with element stamp {other}"),
        )),
    }
}

/// Classify a `TypedObject` capture value as a COMPILER DESCRIPTOR when its
/// schema identifies one: the C3 opaque index handles (`__CheckedTemplate`,
/// `__CaptureBinding`, the E2 `__CheckedItem`) by name, plus everything the
/// established comptime-reflection lift wall (`runtime_lift_rejection`,
/// `shape-runtime/src/comptime_reflection.rs` — the S0 §6 sentence-style
/// precedent) already rejects (TypeRef / FrozenType / descriptor carriers).
/// Returns the schema name for the class parenthetical, or `None` when the
/// object is an ordinary struct (which takes the out-of-domain arm).
fn compiler_descriptor_schema_name(value: &KindedSlot) -> Option<String> {
    let storage = value.as_typed_object_storage()?;
    let schema =
        shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)?;
    let is_index_handle = matches!(
        schema.name.as_str(),
        "__CheckedTemplate" | "__CaptureBinding" | "__CheckedItem"
    );
    if is_index_handle
        || shape_runtime::comptime_reflection::runtime_lift_rejection(value).is_some()
    {
        Some(schema.name)
    } else {
        Option::None
    }
}

/// Whether a declared capture-parameter TYPE annotation lies within the
/// C3-G5 ConstLift domain — the DECLARATION-SITE half of the domain (live
/// from S3a; wired at `CheckedTemplateBuilder::finish()`, the single
/// template-construction chokepoint). Accepts the scalar spellings
/// (`Basic` or single-segment unqualified `Reference`), `T[]`/`Array<T>`,
/// bracket tuples, and `Option<T>`, each RECURSIVELY. Rejects function
/// types naming "functions" and borrow types naming "references" (C3-G5 /
/// Dec-95 never-liftable classes with a declared spelling); every other
/// annotation rejects as not liftable. The `Err` payload is the REASON
/// fragment the finish()-time producer embeds in its full sentence
/// (which carries [`CONST_LIFT_DOMAIN_SENTENCE`]).
pub(in crate::compiler) fn annotation_within_lift_domain(
    annotation: &TypeAnnotation,
) -> Result<(), String> {
    if let Some(inner) = annotation.option_inner() {
        return annotation_within_lift_domain(inner);
    }
    let is_scalar_name =
        |name: &str| matches!(name, "int" | "number" | "bool" | "string");
    match annotation {
        TypeAnnotation::Basic(name) if is_scalar_name(name) => Ok(()),
        TypeAnnotation::Reference(path)
            if !path.is_qualified() && is_scalar_name(path.name()) =>
        {
            Ok(())
        }
        TypeAnnotation::Array(inner) => annotation_within_lift_domain(inner),
        TypeAnnotation::Generic { name, args }
            if name.as_str() == "Array" && args.len() == 1 =>
        {
            annotation_within_lift_domain(&args[0])
        }
        TypeAnnotation::Tuple(elems) => {
            for elem in elems {
                annotation_within_lift_domain(elem)?;
            }
            Ok(())
        }
        TypeAnnotation::Function { .. } => Err(format!(
            "`{}` is a function type, and functions are never liftable (C3-G5 / Dec-95)",
            annotation.to_type_string()
        )),
        TypeAnnotation::Borrow { .. } => Err(format!(
            "`{}` is a reference type, and references are never liftable (C3-G5 / Dec-95)",
            annotation.to_type_string()
        )),
        other => Err(format!(
            "`{}` is not a liftable type",
            other.to_type_string()
        )),
    }
}

/// Lift one declared capture VALUE off the `KindedSlot` substrate into the
/// scalar constant domain, or a NAMED rejection with a positive twin.
///
/// S2-SHAPED PRODUCTION ENTRY (byte-unchanged in S3a): the `capture()`
/// builtin's value acceptance stays scalar-only until S3b flips this fn to
/// delegate to [`lift_value`] atomically with rule-6 identity and composite
/// delivery — so its rejection sentence still names this module's S3 domain
/// as the landing point.
///
/// The kind-dispatched scalar cascade mirrors the established
/// `literal_expr_from_slot` decode (`comptime_builtins.rs`): each accessor is
/// exact on `NativeKind` (ADR-006 §2.7.6 / Q8 — never fabricated from raw
/// bits). Non-finite numbers are rejected like every literal-materialization
/// path.
pub(in crate::compiler) fn lift_capture_value(
    name: &str,
    value: &KindedSlot,
) -> Result<LiftedConst, String> {
    if let Some(s) = value.as_str() {
        return Ok(LiftedConst::String(s.to_string()));
    }
    if let Some(i) = value.as_i64() {
        return Ok(LiftedConst::Int(i));
    }
    if let Some(n) = value.as_f64() {
        if !n.is_finite() {
            return Err(format!(
                "capture `{name}` holds a non-finite number ({n}); capture values must be \
                 finite so they can be delivered as typed literals — pass a finite number"
            ));
        }
        return Ok(LiftedConst::Number(n));
    }
    if let Some(b) = value.as_bool() {
        return Ok(LiftedConst::Bool(b));
    }
    Err(format!(
        "capture `{name}` holds a value outside this slice's ConstLift domain (kind {:?}); \
         pass an int, number, bool, or string capture value — the compositional domain \
         (tuples/arrays/Option of liftables, heap-constant baking, and the never-liftable \
         named rejections) lands with S3 ConstLift",
        value.kind()
    ))
}

/// Validate each lifted capture VALUE against the body fn's declared trailing
/// capture-parameter type (by name — the bijection was already established at
/// `CheckedTemplateBuilder::finish()`). A kind/type mismatch is a NAMED
/// rejection naming BOTH sides with a positive twin. Sentences keep their S2
/// shape; [`LiftedConst::shape_type_name`] now names composite types too.
pub(in crate::compiler) fn validate_capture_value_types(
    body_fn_name: &str,
    capture_params: &[(String, TypeAnnotation)],
    values: &[(String, LiftedConst)],
) -> Result<(), String> {
    for (param_name, annotation) in capture_params {
        let Some((_, lifted)) = values.iter().find(|(name, _)| name == param_name) else {
            // Unreachable after the finish()-time bijection; fail loudly, not
            // silently, if a caller ever skips the chokepoint.
            return Err(format!(
                "internal error: capture parameter `{param_name}` of template body fn \
                 `{body_fn_name}` has no lifted capture value (the construction bijection \
                 was bypassed)"
            ));
        };
        if !lifted.matches_annotation(annotation) {
            return Err(format!(
                "capture `{param_name}` on template body fn `{body_fn_name}` holds a {} value \
                 but the matching trailing capture parameter is annotated `{}`; pass a `{}` \
                 value or annotate the parameter `{param_name}: {}`",
                lifted.shape_type_name(),
                annotation.to_type_string(),
                annotation.to_type_string(),
                lifted.shape_type_name(),
            ));
        }
    }
    Ok(())
}

/// How a specialized handler receives its capture values. One plan shape:
/// typed AST expressions at the wrapper's handler call sites (scalars as
/// literals; S3b's composite delivery rides the same plan through
/// [`LiftedConst::to_expr`] — never a second constant store).
#[derive(Debug, Clone)]
pub(in crate::compiler) enum CaptureBindingPlan {
    /// The weave passes each capture value as a TYPED expression argument at
    /// the wrapper's handler call sites, in the body fn's trailing-parameter
    /// order.
    CallSiteArgs(Vec<Expr>),
}

/// Build the capture delivery plan for one install of `template` onto
/// `target` (the S2b weave consumer; S2a proves the seam). Values are
/// ordered by the body fn's TRAILING PARAMETER order — the weave appends
/// them after the signature arguments at each handler call site.
///
/// The `target` parameter anchors error attribution (the `@application`
/// span); the plan itself is target-independent.
pub(in crate::compiler) fn bind_captures_for_install(
    bound: &BoundTemplate,
    target: &SpecializationTarget,
) -> Result<CaptureBindingPlan, ShapeError> {
    let template = &bound.template;
    let mut args = Vec::with_capacity(template.capture_params().len());
    for (param_name, _annotation) in template.capture_params() {
        let Some((_, lifted)) = bound
            .capture_values
            .iter()
            .find(|(name, _)| name == param_name)
        else {
            // Unreachable after the finish()-time bijection + the builtin's
            // value validation; internal-error-shaped (the specialize_template
            // precedent), anchored for the record at the application site.
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "internal error: bind_captures_for_install found no capture value for \
                     trailing parameter `{param_name}` of template body fn `{}` (install \
                     target `{}`); the construction bijection was bypassed",
                    template.body_fn(),
                    target.name,
                ),
                location: None,
            });
        };
        args.push(lifted.to_expr());
    }
    Ok(CaptureBindingPlan::CallSiteArgs(args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::typed_array::{
        ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I64, ELEM_TYPE_STRING, ELEM_TYPE_TYPED_ARRAY,
        TypedArray, stamp_elem_type,
    };
    use shape_value::{NativeKind, ValueSlot};
    use std::sync::Arc;

    // ── KindedSlot fixture builders (host-side v2-raw construction, the
    //    `build_arg_identity_array` / `nb_string_array` patterns: allocate,
    //    stamp the elem type, push with share transfer, then hand the ONE
    //    array share to the returned KindedSlot) ───────────────────────────

    fn int_array_raw(vals: &[i64]) -> *mut u8 {
        let arr = TypedArray::<i64>::with_capacity(vals.len() as u32);
        // SAFETY: freshly allocated array pointer; stamp-then-push mirrors
        // `build_arg_identity_array` (comptime_builtins/type_reflection.rs).
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64);
            for &v in vals {
                TypedArray::push(arr, v);
            }
        }
        arr as *mut u8
    }

    fn int_array_slot(vals: &[i64]) -> KindedSlot {
        KindedSlot::from_typed_array_raw(int_array_raw(vals))
    }

    fn number_array_slot(vals: &[f64]) -> KindedSlot {
        let arr = TypedArray::<f64>::with_capacity(vals.len() as u32);
        // SAFETY: as above, with the F64 stamp.
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64);
            for &v in vals {
                TypedArray::push(arr, v);
            }
        }
        KindedSlot::from_typed_array_raw(arr as *mut u8)
    }

    fn bool_array_slot(vals: &[bool]) -> KindedSlot {
        let arr = TypedArray::<u8>::with_capacity(vals.len() as u32);
        // SAFETY: as above; the BOOL stamp's element carrier is u8.
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_BOOL);
            for &v in vals {
                TypedArray::push(arr, u8::from(v));
            }
        }
        KindedSlot::from_typed_array_raw(arr as *mut u8)
    }

    fn string_array_slot(vals: &[&str]) -> KindedSlot {
        let arr = TypedArray::<*const StringObj>::with_capacity(vals.len() as u32);
        // SAFETY: the `nb_string_array` pattern (comptime_target.rs) — each
        // StringObj's fresh share transfers into the array.
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            for &s in vals {
                let ptr = StringObj::new(s) as *const StringObj;
                TypedArray::push(arr, ptr);
            }
        }
        KindedSlot::from_typed_array_raw(arr as *mut u8)
    }

    fn nested_int_array_slot(groups: &[&[i64]]) -> KindedSlot {
        let outer = TypedArray::<*const u8>::with_capacity(groups.len() as u32);
        // SAFETY: the ELEM_TYPE_TYPED_ARRAY carrier holds inner v2-raw
        // arrays by pointer; each freshly built inner array's ONE share
        // transfers into the outer array (release walks inner stamps).
        unsafe {
            stamp_elem_type(outer as *mut u8, ELEM_TYPE_TYPED_ARRAY);
            for &group in groups {
                let inner = int_array_raw(group);
                TypedArray::push(outer, inner as *const u8);
            }
        }
        KindedSlot::from_typed_array_raw(outer as *mut u8)
    }

    fn some_slot(payload: KindedSlot) -> KindedSlot {
        KindedSlot::from_option(Arc::new(OptionData::some(payload)))
    }

    fn none_option_slot() -> KindedSlot {
        KindedSlot::from_option(Arc::new(OptionData::none()))
    }

    fn null_slot() -> KindedSlot {
        KindedSlot::new(ValueSlot::from_raw(0), NativeKind::Null)
    }

    /// A zero-bits slot carrying a heap kind label: drop-safe (KindedSlot
    /// Drop guards bits == 0) and never dereferenced by the never-liftable
    /// arms (they read only the kind).
    fn zero_bits(kind: NativeKind) -> KindedSlot {
        KindedSlot::new(ValueSlot::from_raw(0), kind)
    }

    fn seg(v: &LiftedConst) -> String {
        structural_key_segment(v)
    }

    // ── lift_value: recursive happy paths ──────────────────────────────────

    #[test]
    fn lift_value_lifts_the_four_scalars() {
        assert_eq!(
            lift_value("c", &KindedSlot::from_int(42)).expect("int lifts"),
            LiftedConst::Int(42)
        );
        assert_eq!(
            lift_value("c", &KindedSlot::from_number(2.5)).expect("number lifts"),
            LiftedConst::Number(2.5)
        );
        assert_eq!(
            lift_value("c", &KindedSlot::from_bool(true)).expect("bool lifts"),
            LiftedConst::Bool(true)
        );
        assert_eq!(
            lift_value(
                "c",
                &KindedSlot::from_string_arc(Arc::new("hi".to_string()))
            )
            .expect("string lifts"),
            LiftedConst::String("hi".to_string())
        );
    }

    #[test]
    fn lift_value_lifts_flat_arrays_of_every_scalar() {
        assert_eq!(
            lift_value("c", &int_array_slot(&[1, 2, 3])).expect("int array lifts"),
            LiftedConst::Array(vec![
                LiftedConst::Int(1),
                LiftedConst::Int(2),
                LiftedConst::Int(3)
            ])
        );
        assert_eq!(
            lift_value("c", &number_array_slot(&[1.5, -0.0])).expect("number array lifts"),
            LiftedConst::Array(vec![
                LiftedConst::Number(1.5),
                LiftedConst::Number(-0.0)
            ])
        );
        assert_eq!(
            lift_value("c", &bool_array_slot(&[true, false])).expect("bool array lifts"),
            LiftedConst::Array(vec![LiftedConst::Bool(true), LiftedConst::Bool(false)])
        );
        assert_eq!(
            lift_value("c", &string_array_slot(&["a", "bc"])).expect("string array lifts"),
            LiftedConst::Array(vec![
                LiftedConst::String("a".to_string()),
                LiftedConst::String("bc".to_string())
            ])
        );
    }

    #[test]
    fn lift_value_lifts_nested_arrays() {
        assert_eq!(
            lift_value("c", &nested_int_array_slot(&[&[1, 2], &[3]]))
                .expect("nested array lifts"),
            LiftedConst::Array(vec![
                LiftedConst::Array(vec![LiftedConst::Int(1), LiftedConst::Int(2)]),
                LiftedConst::Array(vec![LiftedConst::Int(3)]),
            ])
        );
    }

    #[test]
    fn lift_value_lifts_empty_arrays() {
        assert_eq!(
            lift_value("c", &int_array_slot(&[])).expect("empty array lifts"),
            LiftedConst::Array(Vec::new())
        );
    }

    #[test]
    fn lift_value_lifts_some_none_and_null() {
        assert_eq!(
            lift_value("c", &some_slot(KindedSlot::from_int(5))).expect("Some lifts"),
            LiftedConst::Some(Box::new(LiftedConst::Int(5)))
        );
        assert_eq!(
            lift_value("c", &none_option_slot()).expect("None option lifts"),
            LiftedConst::None
        );
        assert_eq!(
            lift_value("c", &null_slot()).expect("Null kind lifts to None"),
            LiftedConst::None
        );
        // Nested: Some(Some(1)) recurses through payload slots.
        assert_eq!(
            lift_value("c", &some_slot(some_slot(KindedSlot::from_int(1))))
                .expect("nested Some lifts"),
            LiftedConst::Some(Box::new(LiftedConst::Some(Box::new(LiftedConst::Int(1)))))
        );
    }

    // ── lift_value: rejection sentences ────────────────────────────────────

    #[test]
    fn out_of_domain_kinds_reject_naming_the_kind_and_the_closed_domain() {
        for (slot, kind_fragment) in [
            (KindedSlot::new(ValueSlot::from_raw(7), NativeKind::Int32), "kind Int32"),
            (zero_bits(NativeKind::Char), "kind Char"),
            (zero_bits(NativeKind::Ptr(HeapKind::HashMap)), "kind Ptr(HashMap)"),
        ] {
            let err = lift_value("cfg", &slot).expect_err("out-of-domain kind rejects");
            assert_eq!(
                err,
                format!(
                    "capture `cfg` holds a value outside the ConstLift domain ({kind_fragment}); \
                     {CONST_LIFT_DOMAIN_SENTENCE} — pass a liftable capture value"
                ),
                "the out-of-domain sentence is exact: {err}"
            );
        }
    }

    #[test]
    fn function_values_are_never_liftable() {
        // A named-function value is a bare fn-id with kind UInt64
        // (`PushConst(Constant::Function)`); a closure value carries
        // Ptr(Closure); a module fn Ptr(ModuleFn).
        for (slot, kind_fragment) in [
            (
                KindedSlot::new(ValueSlot::from_raw(3), NativeKind::UInt64),
                "kind UInt64",
            ),
            (zero_bits(NativeKind::Ptr(HeapKind::Closure)), "kind Ptr(Closure)"),
            (zero_bits(NativeKind::Ptr(HeapKind::ModuleFn)), "kind Ptr(ModuleFn)"),
        ] {
            let err = lift_value("cfg", &slot).expect_err("function values reject");
            assert_eq!(
                err,
                format!(
                    "capture `cfg` holds a function value ({kind_fragment}), which is never \
                     liftable (C3-G5 / Dec-95): references, resources, capabilities, functions, \
                     provider grants, compiler descriptors, secrets, and runtime handles cannot \
                     cross the comptime->runtime stage boundary; {CONST_LIFT_DOMAIN_SENTENCE} \
                     — pass a liftable capture value"
                ),
                "the never-liftable sentence is exact: {err}"
            );
        }
    }

    #[test]
    fn reference_values_are_never_liftable() {
        let err = lift_value("cfg", &zero_bits(NativeKind::Ptr(HeapKind::Reference)))
            .expect_err("reference values reject");
        assert!(
            err.contains("holds a reference value (kind Ptr(Reference)), which is never liftable"),
            "names the reference class: {err}"
        );
        assert!(err.contains(CONST_LIFT_DOMAIN_SENTENCE), "carries the domain: {err}");
        assert!(err.contains("pass a liftable capture value"), "positive twin: {err}");
    }

    #[test]
    fn runtime_handle_values_are_never_liftable() {
        for kind in [
            NativeKind::Ptr(HeapKind::DataTable),
            NativeKind::Ptr(HeapKind::IoHandle),
            NativeKind::Ptr(HeapKind::Channel),
            NativeKind::Ptr(HeapKind::Mutex),
            NativeKind::Ptr(HeapKind::TaskGroup),
            NativeKind::Ptr(HeapKind::Future),
            NativeKind::Ptr(HeapKind::Atomic),
        ] {
            let err = lift_value("cfg", &zero_bits(kind)).expect_err("runtime handles reject");
            assert!(
                err.contains("holds a runtime handle value"),
                "names the class for {kind:?}: {err}"
            );
            assert!(
                err.contains("never liftable (C3-G5 / Dec-95)"),
                "names the ruling for {kind:?}: {err}"
            );
            assert!(err.contains(CONST_LIFT_DOMAIN_SENTENCE), "domain for {kind:?}: {err}");
        }
    }

    #[test]
    fn compiler_descriptor_handles_are_never_liftable() {
        use shape_runtime::type_schema::builtin_schemas::register_builtin_schemas;
        use shape_runtime::type_schema::registry::TypeSchemaRegistry;
        use shape_runtime::type_schema::{SyncRegistryScope, typed_object_for_named_schema};

        let mut registry = TypeSchemaRegistry::new_with_stdlib();
        let _ids = register_builtin_schemas(&mut registry);
        let _scope = SyncRegistryScope::enter(Arc::new(registry));

        let handle = typed_object_for_named_schema(
            "__CheckedTemplate",
            &[("index", KindedSlot::from_int(0))],
        );
        let err = lift_value("cfg", &handle).expect_err("descriptor handles reject");
        assert!(
            err.contains("holds a compiler descriptor (__CheckedTemplate) value"),
            "names the class and the schema: {err}"
        );
        assert!(
            err.contains("never liftable (C3-G5 / Dec-95)"),
            "names the ruling: {err}"
        );
        assert!(err.contains(CONST_LIFT_DOMAIN_SENTENCE), "carries the domain: {err}");

        let capture_handle = typed_object_for_named_schema(
            "__CaptureBinding",
            &[("index", KindedSlot::from_int(1))],
        );
        let err = lift_value("cfg", &capture_handle).expect_err("capture handles reject");
        assert!(
            err.contains("compiler descriptor (__CaptureBinding)"),
            "names the sibling handle: {err}"
        );
    }

    #[test]
    fn callable_element_arrays_are_never_liftable() {
        use shape_value::v2::typed_array::ELEM_TYPE_CALLABLE;
        // An EMPTY callable-stamped array exercises the stamp arm without
        // constructing callable descriptors (no element is read).
        let arr = TypedArray::<u64>::with_capacity(0);
        // SAFETY: freshly allocated; only the stamp byte is written.
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_CALLABLE);
        }
        let slot = KindedSlot::from_typed_array_raw(arr as *mut u8);
        let err = lift_value("cfg", &slot).expect_err("callable arrays reject");
        assert!(
            err.contains("holds a function value (kind Ptr(TypedArray) with callable elements)"),
            "names the class through the element stamp: {err}"
        );
        assert!(err.contains(CONST_LIFT_DOMAIN_SENTENCE), "carries the domain: {err}");
    }

    #[test]
    fn non_finite_numbers_reject_at_top_level_and_inside_composites() {
        let top = lift_value("cfg", &KindedSlot::from_number(f64::NAN))
            .expect_err("non-finite scalar rejects");
        assert!(top.contains("non-finite"), "names finiteness: {top}");
        assert!(top.contains("pass a finite number"), "positive twin: {top}");

        // Recursion-inherited: the SAME sentence fires from inside an array…
        let composite = lift_value("cfg", &number_array_slot(&[1.0, f64::INFINITY]))
            .expect_err("non-finite array element rejects");
        assert_eq!(
            composite,
            "capture `cfg` holds a non-finite number (inf); capture values must be finite \
             so they can be delivered as typed literals — pass a finite number",
            "the composite rejection is the exact scalar sentence: {composite}"
        );

        // …and from inside an Option payload.
        let inside_some = lift_value("cfg", &some_slot(KindedSlot::from_number(f64::NAN)))
            .expect_err("non-finite Some payload rejects");
        assert!(inside_some.contains("non-finite"), "names finiteness: {inside_some}");
    }

    #[test]
    fn unsupported_element_stamps_reject_naming_the_stamp() {
        use shape_value::v2::typed_array::ELEM_TYPE_TYPED_OBJECT;
        let arr = TypedArray::<*const shape_value::TypedObjectStorage>::with_capacity(0);
        // SAFETY: freshly allocated; only the stamp byte is written.
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);
        }
        let slot = KindedSlot::from_typed_array_raw(arr as *mut u8);
        let err = lift_value("cfg", &slot).expect_err("typed-object element arrays reject");
        assert!(
            err.contains(&format!(
                "kind Ptr(TypedArray) with element stamp {ELEM_TYPE_TYPED_OBJECT}"
            )),
            "names the element stamp: {err}"
        );
        assert!(err.contains(CONST_LIFT_DOMAIN_SENTENCE), "carries the domain: {err}");
    }

    // ── compositional matches_annotation ───────────────────────────────────

    fn int_ann() -> TypeAnnotation {
        TypeAnnotation::Basic("int".to_string())
    }

    fn string_ann() -> TypeAnnotation {
        TypeAnnotation::Basic("string".to_string())
    }

    #[test]
    fn annotation_matching_is_exact_over_scalar_spellings() {
        assert!(LiftedConst::Int(1).matches_annotation(&int_ann()));
        assert!(!LiftedConst::Int(1).matches_annotation(&string_ann()));
        assert!(LiftedConst::String("x".into()).matches_annotation(&string_ann()));
        assert!(
            !LiftedConst::Bool(true).matches_annotation(&TypeAnnotation::Array(Box::new(int_ann()))),
            "a composite annotation never matches a scalar constant"
        );
    }

    #[test]
    fn array_values_match_array_annotations_elementwise() {
        let arr = LiftedConst::Array(vec![LiftedConst::Int(1), LiftedConst::Int(2)]);
        assert!(arr.matches_annotation(&TypeAnnotation::Array(Box::new(int_ann()))));
        assert!(!arr.matches_annotation(&TypeAnnotation::Array(Box::new(string_ann()))));
        // The Generic{Array, [T]} spelling matches identically.
        assert!(arr.matches_annotation(&TypeAnnotation::Generic {
            name: shape_ast::ast::TypePath::simple("Array"),
            args: vec![int_ann()],
        }));
        // Nested element types recurse.
        let nested = LiftedConst::Array(vec![LiftedConst::Array(vec![LiftedConst::Int(1)])]);
        assert!(nested.matches_annotation(&TypeAnnotation::Array(Box::new(
            TypeAnnotation::Array(Box::new(int_ann()))
        ))));
        assert!(!nested.matches_annotation(&TypeAnnotation::Array(Box::new(int_ann()))));
    }

    #[test]
    fn empty_arrays_match_any_array_annotation_but_not_scalars() {
        let empty = LiftedConst::Array(Vec::new());
        assert!(empty.matches_annotation(&TypeAnnotation::Array(Box::new(int_ann()))));
        assert!(empty.matches_annotation(&TypeAnnotation::Array(Box::new(string_ann()))));
        assert!(!empty.matches_annotation(&int_ann()));
        // An empty array against a non-empty tuple annotation is a LENGTH
        // mismatch; against the zero-length tuple it matches.
        assert!(!empty.matches_annotation(&TypeAnnotation::Tuple(vec![int_ann()])));
        assert!(empty.matches_annotation(&TypeAnnotation::Tuple(Vec::new())));
    }

    #[test]
    fn tuple_annotations_require_length_equality_and_positional_match() {
        let pair = LiftedConst::Array(vec![
            LiftedConst::Int(1),
            LiftedConst::String("x".to_string()),
        ]);
        assert!(pair.matches_annotation(&TypeAnnotation::Tuple(vec![int_ann(), string_ann()])));
        assert!(
            !pair.matches_annotation(&TypeAnnotation::Tuple(vec![string_ann(), int_ann()])),
            "positional order matters"
        );
        assert!(
            !pair.matches_annotation(&TypeAnnotation::Tuple(vec![int_ann()])),
            "length mismatch never matches"
        );
    }

    #[test]
    fn option_annotations_match_some_and_none_compositionally() {
        let opt_int = TypeAnnotation::option(int_ann());
        assert!(LiftedConst::None.matches_annotation(&opt_int));
        assert!(
            LiftedConst::Some(Box::new(LiftedConst::Int(5))).matches_annotation(&opt_int)
        );
        assert!(
            !LiftedConst::Some(Box::new(LiftedConst::String("x".into())))
                .matches_annotation(&opt_int),
            "the Some payload must match the Option's type argument"
        );
        assert!(
            !LiftedConst::None.matches_annotation(&int_ann()),
            "None matches only Option-typed capture parameters (the domain sentence)"
        );
    }

    #[test]
    fn scalar_values_do_not_match_option_annotations() {
        // DECIDED-AND-PINNED (S3a): a T value offered where Option<T> is
        // declared is a MISMATCH at this seam — strictness over guessing
        // (the S2 posture); the user spells Some(x).
        let opt_int = TypeAnnotation::option(int_ann());
        assert!(!LiftedConst::Int(5).matches_annotation(&opt_int));
        assert!(
            !LiftedConst::Array(vec![LiftedConst::Int(5)])
                .matches_annotation(&TypeAnnotation::option(TypeAnnotation::Array(Box::new(
                    int_ann()
                )))),
            "composites are not implicitly wrapped either"
        );
    }

    // ── annotation_within_lift_domain (the declaration-site half) ──────────

    #[test]
    fn lift_domain_accepts_scalars_composites_and_options_recursively() {
        for ann in [
            int_ann(),
            TypeAnnotation::Basic("number".to_string()),
            TypeAnnotation::Basic("bool".to_string()),
            string_ann(),
            TypeAnnotation::Array(Box::new(int_ann())),
            TypeAnnotation::Array(Box::new(TypeAnnotation::Array(Box::new(string_ann())))),
            TypeAnnotation::Tuple(vec![int_ann(), string_ann()]),
            TypeAnnotation::option(int_ann()),
            TypeAnnotation::option(TypeAnnotation::Tuple(vec![
                TypeAnnotation::option(int_ann()),
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("bool".to_string()))),
            ])),
            TypeAnnotation::Generic {
                name: shape_ast::ast::TypePath::simple("Array"),
                args: vec![int_ann()],
            },
        ] {
            annotation_within_lift_domain(&ann)
                .unwrap_or_else(|err| panic!("{ann:?} is in-domain, got: {err}"));
        }
    }

    #[test]
    fn lift_domain_rejects_function_types_naming_functions() {
        let ann = TypeAnnotation::Function {
            params: Vec::new(),
            returns: Box::new(int_ann()),
        };
        let err = annotation_within_lift_domain(&ann).expect_err("function types reject");
        assert!(
            err.contains("is a function type, and functions are never liftable (C3-G5 / Dec-95)"),
            "names the function class: {err}"
        );
    }

    #[test]
    fn lift_domain_rejects_borrow_types_naming_references() {
        let ann = TypeAnnotation::Borrow {
            mutable: false,
            inner: Box::new(int_ann()),
        };
        let err = annotation_within_lift_domain(&ann).expect_err("borrow types reject");
        assert!(
            err.contains("is a reference type, and references are never liftable (C3-G5 / Dec-95)"),
            "names the reference class: {err}"
        );
    }

    #[test]
    fn lift_domain_rejects_nominal_and_other_annotations() {
        for ann in [
            TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("MyConfig")),
            TypeAnnotation::Generic {
                name: shape_ast::ast::TypePath::simple("HashMap"),
                args: vec![string_ann(), int_ann()],
            },
            TypeAnnotation::Object(Vec::new()),
        ] {
            let err = annotation_within_lift_domain(&ann)
                .expect_err("out-of-domain annotations reject");
            assert!(err.contains("is not a liftable type"), "for {ann:?}: {err}");
        }
        // The rejection recurses out of composite positions.
        let err = annotation_within_lift_domain(&TypeAnnotation::Array(Box::new(
            TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("MyConfig")),
        )))
        .expect_err("out-of-domain element types reject");
        assert!(err.contains("`MyConfig` is not a liftable type"), "{err}");
    }

    // ── expression projection + display rendering ──────────────────────────

    #[test]
    fn to_expr_projects_typed_literals() {
        match LiftedConst::Int(7).to_expr() {
            Expr::Literal(Literal::Int(7), _) => {}
            other => panic!("expected Int literal, got {other:?}"),
        }
        match LiftedConst::String("s".into()).to_expr() {
            Expr::Literal(Literal::String(s), _) if s == "s" => {}
            other => panic!("expected String literal, got {other:?}"),
        }
        match LiftedConst::Bool(false).to_expr() {
            Expr::Literal(Literal::Bool(false), _) => {}
            other => panic!("expected Bool literal, got {other:?}"),
        }
        match LiftedConst::Number(1.5).to_expr() {
            Expr::Literal(Literal::Number(n), _) if n == 1.5 => {}
            other => panic!("expected Number literal, got {other:?}"),
        }
    }

    #[test]
    fn to_expr_projects_array_literals_and_option_constructors() {
        // Array → Expr::Array with per-element projections.
        match LiftedConst::Array(vec![LiftedConst::Int(1), LiftedConst::Int(2)]).to_expr() {
            Expr::Array(elems, _) => {
                assert_eq!(elems.len(), 2);
                assert!(matches!(elems[0], Expr::Literal(Literal::Int(1), _)));
                assert!(matches!(elems[1], Expr::Literal(Literal::Int(2), _)));
            }
            other => panic!("expected Array expr, got {other:?}"),
        }
        // None → the None literal.
        match LiftedConst::None.to_expr() {
            Expr::Literal(Literal::None, _) => {}
            other => panic!("expected None literal, got {other:?}"),
        }
        // Some(v) → the Some(...) constructor call (BuiltinFunction::SomeCtor
        // resolves the bare name).
        match LiftedConst::Some(Box::new(LiftedConst::Int(5))).to_expr() {
            Expr::FunctionCall {
                name,
                const_args,
                args,
                named_args,
                ..
            } => {
                assert_eq!(name, "Some");
                assert!(const_args.is_empty());
                assert!(named_args.is_empty());
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Expr::Literal(Literal::Int(5), _)));
            }
            other => panic!("expected Some(...) call, got {other:?}"),
        }
        // Nested composites project recursively.
        match LiftedConst::Array(vec![LiftedConst::Some(Box::new(LiftedConst::Bool(true)))])
            .to_expr()
        {
            Expr::Array(elems, _) => {
                assert!(matches!(&elems[0], Expr::FunctionCall { name, .. } if name == "Some"));
            }
            other => panic!("expected Array of Some(...), got {other:?}"),
        }
    }

    #[test]
    fn render_displays_composites_as_user_spellings() {
        assert_eq!(
            LiftedConst::Array(vec![LiftedConst::Int(1), LiftedConst::Int(2)]).render(),
            "[1, 2]"
        );
        assert_eq!(
            LiftedConst::Some(Box::new(LiftedConst::Int(5))).render(),
            "Some(5)"
        );
        assert_eq!(LiftedConst::None.render(), "None");
        assert_eq!(
            LiftedConst::Array(vec![
                LiftedConst::String("a".to_string()),
                LiftedConst::None
            ])
            .render(),
            "[\"a\", None]"
        );
    }

    #[test]
    fn shape_type_name_names_composite_types() {
        assert_eq!(
            LiftedConst::Array(vec![LiftedConst::Int(1)]).shape_type_name(),
            "Array<int>"
        );
        assert_eq!(LiftedConst::Array(Vec::new()).shape_type_name(), "Array<_>");
        assert_eq!(
            LiftedConst::Array(vec![
                LiftedConst::Int(1),
                LiftedConst::String("x".to_string())
            ])
            .shape_type_name(),
            "[int, string]"
        );
        assert_eq!(
            LiftedConst::Some(Box::new(LiftedConst::Int(1))).shape_type_name(),
            "Option<int>"
        );
        assert_eq!(LiftedConst::None.shape_type_name(), "Option<_>");
    }

    // ── structural_key_segment: forms + injectivity refuters ───────────────
    //
    // Each refuter pins one collision class of the #64/S1-verify-1 family
    // (flat non-delimited identity joins) with a NON-VACUITY CONTROL proving
    // the naive flat rendering of the same pair WOULD collide (the S1
    // `colliding_flat_tuple_renderings_specialize_separately` pattern).

    #[test]
    fn segment_forms_are_tagged_and_prefixed() {
        assert_eq!(seg(&LiftedConst::Int(9)), "i:9");
        assert_eq!(seg(&LiftedConst::Number(1.5)), format!("n:{:016x}", 1.5f64.to_bits()));
        assert_eq!(seg(&LiftedConst::Bool(true)), "b:t");
        assert_eq!(seg(&LiftedConst::Bool(false)), "b:f");
        assert_eq!(seg(&LiftedConst::String("ab".to_string())), "s:2:ab");
        assert_eq!(
            seg(&LiftedConst::Array(vec![
                LiftedConst::Int(1),
                LiftedConst::Int(2)
            ])),
            "a:2:[i:1::i:2]"
        );
        assert_eq!(seg(&LiftedConst::None), "o:n");
        assert_eq!(seg(&LiftedConst::Some(Box::new(LiftedConst::Int(5)))), "o:s:i:5");
        // Byte-length prefix counts BYTES, not chars.
        assert_eq!(seg(&LiftedConst::String("日".to_string())), "s:3:日");
    }

    #[test]
    fn refuter_string_pair_redistribution() {
        // ("ab","c") vs ("a","bc") — the flat concatenation collides; the
        // netstring rendering pins byte lengths.
        let a = LiftedConst::Array(vec![
            LiftedConst::String("ab".to_string()),
            LiftedConst::String("c".to_string()),
        ]);
        let b = LiftedConst::Array(vec![
            LiftedConst::String("a".to_string()),
            LiftedConst::String("bc".to_string()),
        ]);
        // Control (non-vacuity): the flat join really collides.
        let flat = |v: &LiftedConst| -> String {
            match v {
                LiftedConst::Array(elems) => elems
                    .iter()
                    .map(|e| match e {
                        LiftedConst::String(s) => s.clone(),
                        other => seg(other),
                    })
                    .collect::<String>(),
                other => seg(other),
            }
        };
        assert_eq!(flat(&a), flat(&b), "control: the flat join must collide");
        assert_ne!(seg(&a), seg(&b), "netstring segments must distinguish the pair");
    }

    #[test]
    fn refuter_sanitizer_lossy_string_arm() {
        // ("a b") vs ("a_b") — the `const_value_mono_segment` sanitizer trap
        // (monomorphization/type_resolution.rs:503-527): its string arm maps
        // every non-alphanumeric char to '_', so "a b" and "a_b" produce ONE
        // segment. That surface is const-generic identity and is NOT ridden
        // for template captures; this control calls the REAL producer to
        // prove the trap is live, and the refuter proves the netstring
        // rendering does not inherit it.
        use crate::compiler::monomorphization::type_resolution::{
            ComptimeConstValue, const_value_mono_segment,
        };
        let spaced = "a b".to_string();
        let underscored = "a_b".to_string();
        assert_eq!(
            const_value_mono_segment(&ComptimeConstValue::String(spaced.clone())),
            const_value_mono_segment(&ComptimeConstValue::String(underscored.clone())),
            "control: the const-generic sanitizer really is lossy on this pair"
        );
        assert_ne!(
            seg(&LiftedConst::String(spaced)),
            seg(&LiftedConst::String(underscored)),
            "the rule-6 segments must not inherit the sanitizer's loss"
        );
    }

    #[test]
    fn refuter_nested_array_redistribution() {
        // [[1,2],[3]] vs [[1],[2,3]] — flat leaf joins collide; count
        // prefixes pin the nesting boundaries.
        let a = LiftedConst::Array(vec![
            LiftedConst::Array(vec![LiftedConst::Int(1), LiftedConst::Int(2)]),
            LiftedConst::Array(vec![LiftedConst::Int(3)]),
        ]);
        let b = LiftedConst::Array(vec![
            LiftedConst::Array(vec![LiftedConst::Int(1)]),
            LiftedConst::Array(vec![LiftedConst::Int(2), LiftedConst::Int(3)]),
        ]);
        // Control (non-vacuity): the flat leaf rendering (mono_key-style `_`
        // join over leaves, no count prefixes) really collides.
        fn flat_leaves(v: &LiftedConst, out: &mut Vec<String>) {
            match v {
                LiftedConst::Array(elems) => {
                    for e in elems {
                        flat_leaves(e, out);
                    }
                }
                LiftedConst::Int(i) => out.push(i.to_string()),
                other => out.push(seg(other)),
            }
        }
        let flat = |v: &LiftedConst| {
            let mut leaves = Vec::new();
            flat_leaves(v, &mut leaves);
            leaves.join("_")
        };
        assert_eq!(flat(&a), flat(&b), "control: the flat leaf join must collide");
        assert_ne!(seg(&a), seg(&b), "count-prefixed segments must distinguish the pair");
    }

    #[test]
    fn refuter_embedded_delimiter_bytes_in_string_content() {
        // A string whose BYTES spell a delimiter + int segment ("x::i:9")
        // vs the honestly-split sibling list ["x", 9]: a delimiter-join
        // without length prefixes cannot tell them apart; the byte-length
        // prefix makes delimiter bytes inside string content inert.
        let embedded = LiftedConst::Array(vec![LiftedConst::String("x::i:9".to_string())]);
        let honest = LiftedConst::Array(vec![
            LiftedConst::String("x".to_string()),
            LiftedConst::Int(9),
        ]);
        // Control (non-vacuity): the delimiter-join with RAW string bytes
        // (no length prefix) really collides.
        let flat = |v: &LiftedConst| -> String {
            match v {
                LiftedConst::Array(elems) => elems
                    .iter()
                    .map(|e| match e {
                        LiftedConst::String(s) => s.clone(),
                        other => seg(other),
                    })
                    .collect::<Vec<_>>()
                    .join("::"),
                other => seg(other),
            }
        };
        assert_eq!(flat(&embedded), flat(&honest), "control: the delimiter join must collide");
        assert_ne!(
            seg(&embedded),
            seg(&honest),
            "length-prefixed segments must distinguish embedded delimiter bytes"
        );
    }

    #[test]
    fn refuter_cross_tag_scalars() {
        // 1 / true / "1" / 1.0 — pairwise distinct segments. Control
        // (non-vacuity): the DISPLAY rendering (render()) collides on three
        // of them — which is also the display-vs-identity separation proof.
        let int1 = LiftedConst::Int(1);
        let bool_true = LiftedConst::Bool(true);
        let str1 = LiftedConst::String("1".to_string());
        let num1 = LiftedConst::Number(1.0);
        assert_eq!(int1.render(), "1");
        assert_eq!(num1.render(), "1", "control: display collides int/number");
        assert_eq!(str1.render(), "\"1\"");
        let values = [&int1, &bool_true, &str1, &num1];
        for (i, a) in values.iter().enumerate() {
            for b in values.iter().skip(i + 1) {
                assert_ne!(
                    seg(a),
                    seg(b),
                    "cross-tag segments must be pairwise distinct: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn refuter_some_vs_payload() {
        // Some(5) vs 5 — the naive payload-unwrap rendering collides; the
        // `o:s:` tag chain distinguishes.
        let some5 = LiftedConst::Some(Box::new(LiftedConst::Int(5)));
        let five = LiftedConst::Int(5);
        let naive_unwrap = |v: &LiftedConst| -> String {
            match v {
                LiftedConst::Some(inner) => seg(inner),
                other => seg(other),
            }
        };
        assert_eq!(
            naive_unwrap(&some5),
            naive_unwrap(&five),
            "control: the payload-unwrap rendering must collide"
        );
        assert_ne!(seg(&some5), seg(&five), "the Option tag must distinguish the pair");
    }

    #[test]
    fn structural_equality_is_bit_pattern_on_numbers() {
        // -0.0 vs 0.0: distinct under both `==` and the key rendering (ONE
        // equality semantic — sound over-distinction, disclosed).
        assert_ne!(LiftedConst::Number(-0.0), LiftedConst::Number(0.0));
        assert_ne!(
            seg(&LiftedConst::Number(-0.0)),
            seg(&LiftedConst::Number(0.0))
        );
        assert_eq!(LiftedConst::Number(1.5), LiftedConst::Number(1.5));
    }

    // ── lift_capture_value: the S2 scalar production entry (byte-unchanged
    //    in S3a; flips to delegate to lift_value in S3b) ────────────────────

    #[test]
    fn lifts_the_four_scalars() {
        assert_eq!(
            lift_capture_value("c", &KindedSlot::from_int(42)).expect("int lifts"),
            LiftedConst::Int(42)
        );
        assert_eq!(
            lift_capture_value("c", &KindedSlot::from_number(2.5)).expect("number lifts"),
            LiftedConst::Number(2.5)
        );
        assert_eq!(
            lift_capture_value("c", &KindedSlot::from_bool(true)).expect("bool lifts"),
            LiftedConst::Bool(true)
        );
        assert_eq!(
            lift_capture_value(
                "c",
                &KindedSlot::from_string_arc(std::sync::Arc::new("hi".to_string()))
            )
            .expect("string lifts"),
            LiftedConst::String("hi".to_string())
        );
    }

    #[test]
    fn non_scalar_value_is_a_named_rejection_pointing_at_the_s3_domain() {
        // A Null-kinded slot is outside the S2 scalar domain; the S2-shaped
        // rejection names the domain and the positive twin, and still points
        // at this module's S3 landing (until S3b flips the delegation).
        let unit = KindedSlot::new(ValueSlot::from_raw(0), NativeKind::Null);
        let err = lift_capture_value("cfg", &unit).expect_err("null is not an S2 scalar");
        assert!(
            err.contains("outside this slice's ConstLift domain"),
            "names the domain violation: {err}"
        );
        assert!(
            err.contains("pass an int, number, bool, or string"),
            "carries the positive twin: {err}"
        );
        assert!(
            err.contains("lands with S3 ConstLift"),
            "names the S3 compositional domain seam: {err}"
        );
    }

    #[test]
    fn non_finite_number_is_a_named_rejection() {
        let err = lift_capture_value("cfg", &KindedSlot::from_number(f64::NAN))
            .expect_err("non-finite numbers must reject");
        assert!(err.contains("non-finite"), "names finiteness: {err}");
        assert!(
            err.contains("pass a finite number"),
            "carries the positive twin: {err}"
        );
    }

    // ── validate_capture_value_types ────────────────────────────────────────

    fn int_param(name: &str) -> (String, TypeAnnotation) {
        (name.to_string(), TypeAnnotation::Basic("int".to_string()))
    }

    #[test]
    fn value_type_match_is_green() {
        validate_capture_value_types(
            "t",
            &[int_param("cfg")],
            &[("cfg".to_string(), LiftedConst::Int(5))],
        )
        .expect("int value against int annotation is green");
    }

    #[test]
    fn value_type_mismatch_names_both_sides_with_positive_twin() {
        let err = validate_capture_value_types(
            "t",
            &[(
                "cfg".to_string(),
                TypeAnnotation::Basic("string".to_string()),
            )],
            &[("cfg".to_string(), LiftedConst::Int(5))],
        )
        .expect_err("int value against string annotation must reject");
        assert!(err.contains("holds a int value"), "names the value: {err}");
        assert!(err.contains("annotated `string`"), "names the declared type: {err}");
        assert!(
            err.contains("pass a `string` value"),
            "carries the positive twin: {err}"
        );
    }

    #[test]
    fn composite_value_type_mismatch_names_the_composite_type() {
        // The S2 sentence SHAPE, now naming composite types (S3a).
        let err = validate_capture_value_types(
            "t",
            &[int_param("cfg")],
            &[(
                "cfg".to_string(),
                LiftedConst::Array(vec![LiftedConst::Int(1)]),
            )],
        )
        .expect_err("array value against int annotation must reject");
        assert!(
            err.contains("holds a Array<int> value"),
            "names the composite value type: {err}"
        );
        assert!(err.contains("annotated `int`"), "names the declared type: {err}");
    }

    #[test]
    fn missing_value_is_an_internal_error_never_silent() {
        let err = validate_capture_value_types("t", &[int_param("cfg")], &[])
            .expect_err("a bypassed bijection must fail loudly");
        assert!(err.contains("internal error"), "internal-error-shaped: {err}");
    }

    // ── bind_captures_for_install (the S2b weave consumer's seam) ───────────

    use crate::compiler::BytecodeCompiler;
    use crate::compiler::comptime_fragments::checked_template::{
        CheckedTemplateBuilder, TemplateHookKind,
    };
    use shape_ast::ast::{CaptureClause, CaptureEntry, CaptureMode, Item};

    fn def_of(src: &str) -> shape_ast::ast::FunctionDef {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                Item::Function(func, _) => Some(func),
                _ => Option::None,
            })
            .expect("fixture has one function")
    }

    fn move_entry(name: &str) -> CaptureEntry {
        CaptureEntry {
            mode: CaptureMode::Move,
            name: name.to_string(),
            span: shape_ast::ast::Span::default(),
            name_span: shape_ast::ast::Span::default(),
        }
    }

    fn target_of(src: &str) -> SpecializationTarget {
        let def = def_of(src);
        BytecodeCompiler::new()
            .specialization_target_from_def(&def, None, def.name_span)
            .expect("target glue builds from declared annotations")
    }

    #[test]
    fn call_site_args_deliver_in_param_order_regardless_of_binding_order() {
        // The body fn's trailing captures are (factor: int, tag: string); the
        // capture() bindings arrive in the REVERSE order — delivery follows
        // PARAMETER order (the weave contract), matching by NAME.
        let def = def_of("fn t(x: int, factor: int, tag: string) -> int { return x }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&def)
            .expect("shape passes")
            .captures(CaptureClause {
                entries: vec![move_entry("tag"), move_entry("factor")],
                span: shape_ast::ast::Span::default(),
            })
            .finish()
            .expect("template finishes");
        let bound = BoundTemplate {
            template,
            capture_values: vec![
                ("tag".to_string(), LiftedConst::String("hi".to_string())),
                ("factor".to_string(), LiftedConst::Int(3)),
            ],
        };
        let target = target_of("fn victim(a: int) -> int { return a }");

        let CaptureBindingPlan::CallSiteArgs(args) =
            bind_captures_for_install(&bound, &target).expect("plan builds");
        assert_eq!(args.len(), 2);
        match &args[0] {
            Expr::Literal(Literal::Int(3), _) => {}
            other => panic!("param order puts `factor` first, got {other:?}"),
        }
        match &args[1] {
            Expr::Literal(Literal::String(s), _) if s == "hi" => {}
            other => panic!("param order puts `tag` second, got {other:?}"),
        }
    }

    #[test]
    fn missing_capture_value_is_an_internal_error() {
        let def = def_of("fn t(x: int, factor: int) -> int { return x }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&def)
            .expect("shape passes")
            .captures(CaptureClause {
                entries: vec![move_entry("factor")],
                span: shape_ast::ast::Span::default(),
            })
            .finish()
            .expect("template finishes");
        let bound = BoundTemplate {
            template,
            capture_values: Vec::new(), // the bypassed-bijection shape
        };
        let target = target_of("fn victim(a: int) -> int { return a }");

        let err = bind_captures_for_install(&bound, &target)
            .expect_err("a bypassed bijection fails loudly");
        assert!(
            err.to_string().contains("internal error"),
            "internal-error-shaped: {err}"
        );
    }
}
