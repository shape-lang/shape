//! Opaque inference-variable identities and their authenticated annotation carrier.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use shape_ast::ast::TypeAnnotation;

use crate::type_system::semantic::TypeVarId;

/// Unspellable annotation carrier for a deferred inference variable.
///
/// The text following this prefix is authenticated with a process-local key.
/// Parsing a similarly shaped string therefore cannot mint a type-variable
/// capability; only [`tyvar_to_annotation`] can issue a valid carrier.
/// Carriers are transient: the key is never serialized, so a carrier restored
/// after a process restart fails closed instead of recreating authority.
pub const TYVAR_ANNOTATION_PREFIX: &str = "\u{1}tyvar:";

const CARRIER_VERSION: &str = "v1";
const REDACTED_TYPE_VAR_NAME: &str = "<invalid type variable>";
type CarrierMac = Hmac<Sha256>;

/// Non-cloneable generator for per-inference holes and declared capabilities.
///
/// ```compile_fail
/// # use shape_runtime::type_system::TypeVarGen;
/// let _ = TypeVarGen::new().clone();
/// ```
pub struct TypeVarGen {
    next_id: u32,
    inference_owner: u64,
    next_declared_owner: u32,
}

impl fmt::Debug for TypeVarGen {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TypeVarGen").finish_non_exhaustive()
    }
}

static NEXT_INFERENCE_OWNER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl TypeVarGen {
    pub fn new() -> Self {
        let inference_owner = NEXT_INFERENCE_OWNER
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |owner| owner.checked_add(1),
            )
            .expect("TypeVar inference-owner overflow");
        Self {
            next_id: 0,
            inference_owner,
            next_declared_owner: 0,
        }
    }

    pub fn fresh_var(&mut self) -> TypeVar {
        let ordinal = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("TypeVarGen overflow");
        TypeVar::inference_hole(self.inference_owner, ordinal)
    }

    pub fn fresh_type(&mut self) -> super::core::Type {
        super::core::Type::Variable(self.fresh_var())
    }

    /// Mint the opaque owner capability for one generic declaration.
    pub(crate) fn fresh_declared_owner(&mut self) -> DeclaredTypeVarOwner {
        let declaration = self.next_declared_owner;
        self.next_declared_owner = declaration
            .checked_add(1)
            .expect("declared TypeVar owner overflow");
        DeclaredTypeVarOwner {
            inference: self.inference_owner,
            declaration,
        }
    }
}

impl Default for TypeVarGen {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque identity for the declaration that owns a generic parameter list.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclaredTypeVarOwner {
    inference: u64,
    declaration: u32,
}

impl fmt::Debug for DeclaredTypeVarOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeclaredTypeVarOwner(..)")
    }
}

/// Typed, non-stringly provenance for a declared generic parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclaredTypeVarProvenance<'a> {
    owner: DeclaredTypeVarOwner,
    ordinal: u32,
    source_name: &'a str,
}

impl<'a> DeclaredTypeVarProvenance<'a> {
    pub fn owner(self) -> DeclaredTypeVarOwner {
        self.owner
    }

    pub fn ordinal(self) -> u32 {
        self.ordinal
    }

    pub fn source_name(self) -> &'a str {
        self.source_name
    }
}

#[derive(Clone)]
enum TypeVarIdentity {
    Legacy(String),
    InferenceHole {
        inference_owner: u64,
        ordinal: u32,
    },
    Declared {
        owner: DeclaredTypeVarOwner,
        ordinal: u32,
        source_name: String,
    },
}

/// A type-variable capability.
///
/// [`TypeVar::new`] intentionally creates only legacy, name-based variables.
/// Declared provenance is available exclusively through the inference-owned
/// minting path, and the representation is private so raw strings cannot
/// impersonate that authority.
#[derive(Clone)]
pub struct TypeVar {
    identity: TypeVarIdentity,
}

impl TypeVar {
    pub fn new(name: String) -> Self {
        Self {
            identity: TypeVarIdentity::Legacy(name),
        }
    }

    fn inference_hole(inference_owner: u64, ordinal: u32) -> Self {
        Self {
            identity: TypeVarIdentity::InferenceHole {
                inference_owner,
                ordinal,
            },
        }
    }

    pub(crate) fn declared(
        owner: DeclaredTypeVarOwner,
        ordinal: u32,
        source_name: impl AsRef<str>,
    ) -> Self {
        Self {
            identity: TypeVarIdentity::Declared {
                owner,
                ordinal,
                source_name: source_name.as_ref().to_string(),
            },
        }
    }

    pub fn declared_provenance(&self) -> Option<DeclaredTypeVarProvenance<'_>> {
        let TypeVarIdentity::Declared {
            owner,
            ordinal,
            source_name,
        } = &self.identity
        else {
            return None;
        };
        Some(DeclaredTypeVarProvenance {
            owner: *owner,
            ordinal: *ordinal,
            source_name,
        })
    }

    pub fn presentation_name(&self) -> Cow<'_, str> {
        match &self.identity {
            TypeVarIdentity::Legacy(name) => safe_presentation_name(name),
            TypeVarIdentity::InferenceHole { ordinal, .. } => Cow::Owned(format!("T{ordinal}")),
            TypeVarIdentity::Declared { source_name, .. } => safe_presentation_name(source_name),
        }
    }

    pub(crate) fn is_legacy_named(&self, expected: &str) -> bool {
        matches!(&self.identity, TypeVarIdentity::Legacy(name) if name == expected)
    }

    pub(super) fn legacy_semantic_id(&self) -> Option<TypeVarId> {
        let TypeVarIdentity::Legacy(name) = &self.identity else {
            return None;
        };
        name.strip_prefix('T')?.parse().ok().map(TypeVarId)
    }

    fn carrier_payload(&self) -> String {
        // Authentication covers this complete payload: version, variant tag,
        // typed identity fields, and presentation-only source name.
        match &self.identity {
            TypeVarIdentity::Legacy(name) => {
                format!("{CARRIER_VERSION}:l:{}", hex::encode(name.as_bytes()))
            }
            TypeVarIdentity::InferenceHole {
                inference_owner,
                ordinal,
            } => format!("{CARRIER_VERSION}:h:{inference_owner:016x}:{ordinal:08x}"),
            TypeVarIdentity::Declared {
                owner,
                ordinal,
                source_name,
            } => format!(
                "{CARRIER_VERSION}:d:{:016x}:{:08x}:{ordinal:08x}:{}",
                owner.inference,
                owner.declaration,
                hex::encode(source_name.as_bytes())
            ),
        }
    }

    fn from_authenticated_payload(payload: &str) -> Option<Self> {
        let mut fields = payload.split(':');
        if fields.next()? != CARRIER_VERSION {
            return None;
        }
        let identity = match fields.next()? {
            "l" => TypeVarIdentity::Legacy(decode_text(fields.next()?)?),
            "h" => TypeVarIdentity::InferenceHole {
                inference_owner: decode_u64(fields.next()?)?,
                ordinal: decode_u32(fields.next()?)?,
            },
            "d" => TypeVarIdentity::Declared {
                owner: DeclaredTypeVarOwner {
                    inference: decode_u64(fields.next()?)?,
                    declaration: decode_u32(fields.next()?)?,
                },
                ordinal: decode_u32(fields.next()?)?,
                source_name: decode_text(fields.next()?)?,
            },
            _ => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        Some(Self { identity })
    }
}

impl PartialEq for TypeVar {
    fn eq(&self, other: &Self) -> bool {
        match (&self.identity, &other.identity) {
            (TypeVarIdentity::Legacy(left), TypeVarIdentity::Legacy(right)) => left == right,
            (
                TypeVarIdentity::InferenceHole {
                    inference_owner: left_owner,
                    ordinal: left_ordinal,
                },
                TypeVarIdentity::InferenceHole {
                    inference_owner: right_owner,
                    ordinal: right_ordinal,
                },
            ) => left_owner == right_owner && left_ordinal == right_ordinal,
            (
                TypeVarIdentity::Declared {
                    owner: left_owner,
                    ordinal: left_ordinal,
                    ..
                },
                TypeVarIdentity::Declared {
                    owner: right_owner,
                    ordinal: right_ordinal,
                    ..
                },
            ) => left_owner == right_owner && left_ordinal == right_ordinal,
            _ => false,
        }
    }
}

impl Eq for TypeVar {}

impl Hash for TypeVar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.identity {
            TypeVarIdentity::Legacy(name) => {
                0_u8.hash(state);
                name.hash(state);
            }
            TypeVarIdentity::InferenceHole {
                inference_owner,
                ordinal,
            } => {
                1_u8.hash(state);
                inference_owner.hash(state);
                ordinal.hash(state);
            }
            TypeVarIdentity::Declared { owner, ordinal, .. } => {
                2_u8.hash(state);
                owner.hash(state);
                ordinal.hash(state);
            }
        }
    }
}

impl fmt::Debug for TypeVar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TypeVar")
            .field(&self.presentation_name())
            .finish()
    }
}

pub fn tyvar_to_annotation(variable: &TypeVar) -> TypeAnnotation {
    let payload = variable.carrier_payload();
    TypeAnnotation::Basic(format!(
        "{TYVAR_ANNOTATION_PREFIX}{payload}:{}",
        authenticate(&payload)
    ))
}

pub fn annotation_as_tyvar(annotation: &TypeAnnotation) -> Option<TypeVar> {
    let TypeAnnotation::Basic(name) = annotation else {
        return None;
    };
    let carrier = name.strip_prefix(TYVAR_ANNOTATION_PREFIX)?;
    let (payload, authentication) = carrier.rsplit_once(':')?;
    verify_authentication(payload, authentication)?;
    TypeVar::from_authenticated_payload(payload)
}

/// Treat every reserved carrier prefix as unresolved at the semantic boundary.
///
/// This deliberately does not authenticate or recover a [`TypeVar`]. Invalid
/// carriers are rejected fail-closed instead of falling through as user type
/// names; [`annotation_as_tyvar`] remains the sole authority-recovery path.
pub fn annotation_contains_reserved_type_var_carrier(annotation: &TypeAnnotation) -> bool {
    let contains = annotation_contains_reserved_type_var_carrier;
    match annotation {
        TypeAnnotation::Basic(name) if name.starts_with(TYVAR_ANNOTATION_PREFIX) => true,
        TypeAnnotation::Array(inner)
        | TypeAnnotation::Borrow { inner, .. }
        | TypeAnnotation::Existential { inner, .. } => contains(inner),
        TypeAnnotation::Tuple(items)
        | TypeAnnotation::Union(items)
        | TypeAnnotation::Intersection(items) => items.iter().any(contains),
        TypeAnnotation::Object(fields) => {
            fields.iter().any(|field| contains(&field.type_annotation))
        }
        TypeAnnotation::Function { params, returns } => {
            params
                .iter()
                .any(|parameter| contains(&parameter.type_annotation))
                || contains(returns)
        }
        TypeAnnotation::Generic { args, .. } => args.iter().any(contains),
        TypeAnnotation::Basic(_)
        | TypeAnnotation::Reference(_)
        | TypeAnnotation::Void
        | TypeAnnotation::Never
        | TypeAnnotation::Null
        | TypeAnnotation::Undefined
        | TypeAnnotation::Dyn(_) => false,
    }
}

fn carrier_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut key = [0_u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        key
    })
}

fn carrier_mac(payload: &str) -> CarrierMac {
    let mut mac = CarrierMac::new_from_slice(carrier_key())
        .expect("the fixed type-variable carrier key length is valid");
    mac.update(payload.as_bytes());
    mac
}

fn authenticate(payload: &str) -> String {
    hex::encode(carrier_mac(payload).finalize().into_bytes())
}

fn verify_authentication(payload: &str, encoded: &str) -> Option<()> {
    let authentication = hex::decode(encoded).ok()?;
    carrier_mac(payload).verify_slice(&authentication).ok()
}

fn decode_u64(encoded: &str) -> Option<u64> {
    u64::from_str_radix(encoded, 16).ok()
}

fn decode_u32(encoded: &str) -> Option<u32> {
    u32::from_str_radix(encoded, 16).ok()
}

fn decode_text(encoded: &str) -> Option<String> {
    String::from_utf8(hex::decode(encoded).ok()?).ok()
}

fn safe_presentation_name(name: &str) -> Cow<'_, str> {
    if name.chars().any(char::is_control) {
        Cow::Borrowed(REDACTED_TYPE_VAR_NAME)
    } else {
        Cow::Borrowed(name)
    }
}

#[cfg(test)]
mod tests;
