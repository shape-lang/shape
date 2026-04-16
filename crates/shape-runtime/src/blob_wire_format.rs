//! Cross-language wire format for FunctionBlob serialization.
//!
//! This module provides a stable, versioned binary format for FunctionBlobs
//! that can be implemented in any language. The format is designed for:
//! - Deterministic serialization (same blob = same bytes)
//! - Forward compatibility (new fields are appendable)
//! - Language independence (no Rust-specific constructs)
//!
//! ## Wire Format Layout
//!
//! ```text
//! +-------------------+
//! | BlobHeader (50 B) |
//! +-------------------+
//! | Section Table     |  (section_count * 18 bytes)
//! +-------------------+
//! | Section 0 data    |
//! +-------------------+
//! | Section 1 data    |
//! +-------------------+
//! | ...               |
//! +-------------------+
//! ```
//!
//! The encoder accepts pre-serialized section payloads via [`EncodableBlob`],
//! keeping this module free of `shape-vm` dependencies (which would create a
//! circular crate dependency). Higher-level code (in `shape-vm`) provides a
//! `From<&FunctionBlob>` conversion into `EncodableBlob`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Wire format version.
pub const WIRE_FORMAT_VERSION: u32 = 1;

/// Magic bytes identifying a Shape blob file.
pub const BLOB_MAGIC: [u8; 4] = [0x53, 0x48, 0x42, 0x4C]; // "SHBL"

/// Header size: 4 (magic) + 4 (version) + 32 (hash) + 8 (total_size) + 2 (section_count) = 50
const HEADER_SIZE: usize = 50;

/// Each section table entry: 2 (type) + 8 (offset) + 8 (length) = 18 bytes
const SECTION_ENTRY_SIZE: usize = 18;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during wire format encoding/decoding.
#[derive(Debug, Clone, thiserror::Error)]
pub enum WireFormatError {
    #[error("invalid magic bytes: expected SHBL")]
    InvalidMagic,
    #[error("unsupported wire format version {0} (expected {WIRE_FORMAT_VERSION})")]
    UnsupportedVersion(u32),
    #[error("content hash mismatch")]
    HashMismatch,
    #[error("data too short: need {needed} bytes, got {got}")]
    TooShort { needed: usize, got: usize },
    #[error("section offset {offset} + length {length} exceeds total size {total}")]
    SectionOutOfBounds {
        offset: u64,
        length: u64,
        total: u64,
    },
    #[error("unknown section type 0x{0:04x}")]
    UnknownSectionType(u16),
    #[error("missing required section: {0}")]
    MissingSection(&'static str),
    #[error("msgpack decode error: {0}")]
    MsgpackDecode(String),
    #[error("msgpack encode error: {0}")]
    MsgpackEncode(String),
    #[error("duplicate section type 0x{0:04x}")]
    DuplicateSection(u16),
}

// ---------------------------------------------------------------------------
// Section types
// ---------------------------------------------------------------------------

/// Section types in the wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum SectionType {
    Metadata = 0x0001,
    Instructions = 0x0002,
    Constants = 0x0003,
    Strings = 0x0004,
    Dependencies = 0x0005,
    TypeSchemas = 0x0006,
    SourceMap = 0x0007,
    Permissions = 0x0008,
}

impl SectionType {
    fn from_u16(v: u16) -> Result<Self, WireFormatError> {
        match v {
            0x0001 => Ok(Self::Metadata),
            0x0002 => Ok(Self::Instructions),
            0x0003 => Ok(Self::Constants),
            0x0004 => Ok(Self::Strings),
            0x0005 => Ok(Self::Dependencies),
            0x0006 => Ok(Self::TypeSchemas),
            0x0007 => Ok(Self::SourceMap),
            0x0008 => Ok(Self::Permissions),
            other => Err(WireFormatError::UnknownSectionType(other)),
        }
    }
}

// ---------------------------------------------------------------------------
// Header / section table (on-disk structs)
// ---------------------------------------------------------------------------

/// Wire format header for a serialized blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub content_hash: [u8; 32],
    pub total_size: u64,
    pub section_count: u16,
}

/// A section table entry in the wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobSectionEntry {
    pub section_type: SectionType,
    pub offset: u64,
    pub length: u64,
}

// ---------------------------------------------------------------------------
// Section payloads (msgpack-serialized)
// ---------------------------------------------------------------------------

/// Metadata section content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobMetadata {
    pub name: String,
    pub arity: u16,
    pub locals_count: u16,
    pub is_async: bool,
    pub is_closure: bool,
    pub captures_count: u16,
    pub param_names: Vec<String>,
    pub ref_params: Vec<bool>,
    pub ref_mutates: Vec<bool>,
    pub mutable_captures: Vec<bool>,
}

/// Type mapping specification for cross-language interop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeMapping {
    pub shape_type: String,
    pub schema_hash: [u8; 32],
    pub fields: Vec<TypeFieldMapping>,
}

/// A field within a type mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeFieldMapping {
    pub name: String,
    pub field_type: WireType,
    pub offset: u32,
}

/// Language-independent type descriptors for cross-language interop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireType {
    Int64,
    Float64,
    Bool,
    String,
    Bytes,
    Array(Box<WireType>),
    Map(Box<WireType>, Box<WireType>),
    Optional(Box<WireType>),
    /// Reference to another type by schema hash.
    Ref([u8; 32]),
}

// ---------------------------------------------------------------------------
// EncodableBlob - VM-independent input for the encoder
// ---------------------------------------------------------------------------

/// A blob prepared for wire-format encoding.
///
/// This struct holds pre-serialized section payloads so that the encoder does
/// not need to depend on `shape-vm` types.  Higher-level code (typically in
/// `shape-vm`) converts a `FunctionBlob` into an `EncodableBlob`.
#[derive(Debug, Clone)]
pub struct EncodableBlob {
    /// Msgpack-encoded [`BlobMetadata`].
    pub metadata_bytes: Vec<u8>,
    /// Msgpack-encoded instructions (opaque to this module).
    pub instructions_bytes: Vec<u8>,
    /// Msgpack-encoded constants (opaque to this module).
    pub constants_bytes: Vec<u8>,
    /// Msgpack-encoded string pool.
    pub strings_bytes: Vec<u8>,
    /// Flat concatenation of 32-byte dependency hashes.
    pub dependencies_bytes: Vec<u8>,
    /// Msgpack-encoded type schema names.
    pub type_schemas_bytes: Vec<u8>,
    /// Msgpack-encoded source map entries.
    pub source_map_bytes: Vec<u8>,
    /// Msgpack-encoded permission names.
    pub permissions_bytes: Vec<u8>,
}

impl EncodableBlob {
    /// Build an `EncodableBlob` from individual fields.
    ///
    /// This is the convenience constructor when you already have the decoded
    /// data available. Each field is msgpack-serialized here.
    pub fn from_parts(
        metadata: &BlobMetadata,
        instructions: &impl Serialize,
        constants: &impl Serialize,
        strings: &[String],
        dependency_hashes: &[[u8; 32]],
        type_schemas: &[String],
        source_map: &[(usize, u32, u32)],
        permission_names: &[&str],
    ) -> Result<Self, WireFormatError> {
        fn mp<T: Serialize + ?Sized>(v: &T) -> Result<Vec<u8>, WireFormatError> {
            rmp_serde::encode::to_vec(v).map_err(|e| WireFormatError::MsgpackEncode(e.to_string()))
        }

        let mut deps_buf = Vec::with_capacity(dependency_hashes.len() * 32);
        for h in dependency_hashes {
            deps_buf.extend_from_slice(h);
        }

        Ok(Self {
            metadata_bytes: mp(metadata)?,
            instructions_bytes: mp(instructions)?,
            constants_bytes: mp(constants)?,
            strings_bytes: mp(strings)?,
            dependencies_bytes: deps_buf,
            type_schemas_bytes: mp(type_schemas)?,
            source_map_bytes: mp(source_map)?,
            permissions_bytes: mp(&permission_names)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Decoded blob (in-memory representation after decode)
// ---------------------------------------------------------------------------

/// A fully decoded blob from wire format.
#[derive(Debug, Clone)]
pub struct DecodedBlob {
    pub content_hash: [u8; 32],
    pub metadata: BlobMetadata,
    /// Raw msgpack bytes for the instructions section.
    pub instructions_bytes: Vec<u8>,
    /// Raw msgpack bytes for the constants section.
    pub constants_bytes: Vec<u8>,
    /// Decoded string pool.
    pub strings: Vec<String>,
    /// Dependency hashes (each 32 bytes).
    pub dependencies: Vec<[u8; 32]>,
    /// Type schema names referenced by this blob.
    pub type_schemas: Vec<String>,
    /// Source map entries: (local_offset, file_id, line).
    pub source_map: Vec<(usize, u32, u32)>,
    /// Permission names required by this function.
    pub permissions: Vec<String>,
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Encodes an [`EncodableBlob`] into the cross-language wire format.
pub struct BlobEncoder<'a> {
    blob: &'a EncodableBlob,
}

impl<'a> BlobEncoder<'a> {
    pub fn new(blob: &'a EncodableBlob) -> Self {
        Self { blob }
    }

    /// Serialize the blob into wire format bytes.
    pub fn encode_to_bytes(&self) -> Result<Vec<u8>, WireFormatError> {
        let sections: Vec<(SectionType, &[u8])> = vec![
            (SectionType::Metadata, &self.blob.metadata_bytes),
            (SectionType::Instructions, &self.blob.instructions_bytes),
            (SectionType::Constants, &self.blob.constants_bytes),
            (SectionType::Strings, &self.blob.strings_bytes),
            (SectionType::Dependencies, &self.blob.dependencies_bytes),
            (SectionType::TypeSchemas, &self.blob.type_schemas_bytes),
            (SectionType::SourceMap, &self.blob.source_map_bytes),
            (SectionType::Permissions, &self.blob.permissions_bytes),
        ];

        let section_count = sections.len() as u16;
        let section_table_size = sections.len() * SECTION_ENTRY_SIZE;
        let data_start = HEADER_SIZE + section_table_size;

        // Compute section offsets.
        let mut section_entries = Vec::with_capacity(sections.len());
        let mut current_offset = data_start as u64;
        for (st, payload) in &sections {
            section_entries.push(BlobSectionEntry {
                section_type: *st,
                offset: current_offset,
                length: payload.len() as u64,
            });
            current_offset += payload.len() as u64;
        }
        let total_size = current_offset;

        // Assemble the final buffer.
        let mut buf = Vec::with_capacity(total_size as usize);

        // -- header (50 bytes) --
        buf.extend_from_slice(&BLOB_MAGIC);
        buf.extend_from_slice(&WIRE_FORMAT_VERSION.to_le_bytes());
        // Placeholder for content hash (filled after computing over body).
        let hash_pos = buf.len();
        buf.extend_from_slice(&[0u8; 32]);
        buf.extend_from_slice(&total_size.to_le_bytes());
        buf.extend_from_slice(&section_count.to_le_bytes());

        // -- section table --
        for entry in &section_entries {
            buf.extend_from_slice(&(entry.section_type as u16).to_le_bytes());
            buf.extend_from_slice(&entry.offset.to_le_bytes());
            buf.extend_from_slice(&entry.length.to_le_bytes());
        }

        // -- section data --
        for (_, payload) in &sections {
            buf.extend_from_slice(payload);
        }

        // Compute content hash over everything after the header
        // (section table + section data).
        let digest = Sha256::digest(&buf[HEADER_SIZE..]);
        buf[hash_pos..hash_pos + 32].copy_from_slice(&digest);

        Ok(buf)
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Decodes wire-format bytes into a [`DecodedBlob`].
pub struct BlobDecoder;

impl BlobDecoder {
    /// Decode a wire-format byte slice into a [`DecodedBlob`].
    ///
    /// This validates the magic bytes and version but does **not** verify the
    /// content hash. Call [`validate_blob`] for full verification.
    pub fn decode_from_bytes(data: &[u8]) -> Result<DecodedBlob, WireFormatError> {
        if data.len() < HEADER_SIZE {
            return Err(WireFormatError::TooShort {
                needed: HEADER_SIZE,
                got: data.len(),
            });
        }

        // Parse header.
        let magic: [u8; 4] = data[0..4].try_into().unwrap();
        if magic != BLOB_MAGIC {
            return Err(WireFormatError::InvalidMagic);
        }

        let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
        if version != WIRE_FORMAT_VERSION {
            return Err(WireFormatError::UnsupportedVersion(version));
        }

        let content_hash: [u8; 32] = data[8..40].try_into().unwrap();
        let total_size = u64::from_le_bytes(data[40..48].try_into().unwrap());
        let section_count = u16::from_le_bytes(data[48..50].try_into().unwrap());

        let needed = HEADER_SIZE + (section_count as usize) * SECTION_ENTRY_SIZE;
        if data.len() < needed {
            return Err(WireFormatError::TooShort {
                needed,
                got: data.len(),
            });
        }

        // Parse section table.
        let mut sections = Vec::with_capacity(section_count as usize);
        let mut offset = HEADER_SIZE;
        for _ in 0..section_count {
            let st = u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap());
            let sec_offset = u64::from_le_bytes(data[offset + 2..offset + 10].try_into().unwrap());
            let sec_length = u64::from_le_bytes(data[offset + 10..offset + 18].try_into().unwrap());

            let section_type = SectionType::from_u16(st)?;

            if sec_offset + sec_length > total_size {
                return Err(WireFormatError::SectionOutOfBounds {
                    offset: sec_offset,
                    length: sec_length,
                    total: total_size,
                });
            }

            sections.push((section_type, sec_offset as usize, sec_length as usize));
            offset += SECTION_ENTRY_SIZE;
        }

        // Helper to extract a section's bytes.
        let get_section = |st: SectionType| -> Option<&[u8]> {
            sections
                .iter()
                .find(|(t, _, _)| *t == st)
                .map(|(_, o, l)| &data[*o..*o + *l])
        };

        let require_section =
            |st: SectionType, label: &'static str| -> Result<&[u8], WireFormatError> {
                get_section(st).ok_or(WireFormatError::MissingSection(label))
            };

        // Decode each required section.
        let metadata: BlobMetadata = {
            let bytes = require_section(SectionType::Metadata, "Metadata")?;
            rmp_serde::decode::from_slice(bytes)
                .map_err(|e| WireFormatError::MsgpackDecode(e.to_string()))?
        };

        let instructions_bytes =
            require_section(SectionType::Instructions, "Instructions")?.to_vec();

        let constants_bytes = require_section(SectionType::Constants, "Constants")?.to_vec();

        let strings: Vec<String> = {
            let bytes = require_section(SectionType::Strings, "Strings")?;
            rmp_serde::decode::from_slice(bytes)
                .map_err(|e| WireFormatError::MsgpackDecode(e.to_string()))?
        };

        let dependencies: Vec<[u8; 32]> = {
            let bytes = require_section(SectionType::Dependencies, "Dependencies")?;
            if bytes.len() % 32 != 0 {
                return Err(WireFormatError::MsgpackDecode(
                    "dependency section length not a multiple of 32".into(),
                ));
            }
            bytes
                .chunks_exact(32)
                .map(|chunk| {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(chunk);
                    arr
                })
                .collect()
        };

        let type_schemas: Vec<String> = {
            let bytes = require_section(SectionType::TypeSchemas, "TypeSchemas")?;
            rmp_serde::decode::from_slice(bytes)
                .map_err(|e| WireFormatError::MsgpackDecode(e.to_string()))?
        };

        let source_map: Vec<(usize, u32, u32)> = {
            let bytes = require_section(SectionType::SourceMap, "SourceMap")?;
            rmp_serde::decode::from_slice(bytes)
                .map_err(|e| WireFormatError::MsgpackDecode(e.to_string()))?
        };

        let permissions: Vec<String> = {
            let bytes = require_section(SectionType::Permissions, "Permissions")?;
            rmp_serde::decode::from_slice(bytes)
                .map_err(|e| WireFormatError::MsgpackDecode(e.to_string()))?
        };

        Ok(DecodedBlob {
            content_hash,
            metadata,
            instructions_bytes,
            constants_bytes,
            strings,
            dependencies,
            type_schemas,
            source_map,
            permissions,
        })
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Verify magic, version, and content hash of a wire-format blob.
///
/// Returns `Ok(true)` if valid, or an error describing the first problem found.
pub fn validate_blob(data: &[u8]) -> Result<bool, WireFormatError> {
    if data.len() < HEADER_SIZE {
        return Err(WireFormatError::TooShort {
            needed: HEADER_SIZE,
            got: data.len(),
        });
    }

    // Magic.
    let magic: [u8; 4] = data[0..4].try_into().unwrap();
    if magic != BLOB_MAGIC {
        return Err(WireFormatError::InvalidMagic);
    }

    // Version.
    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version != WIRE_FORMAT_VERSION {
        return Err(WireFormatError::UnsupportedVersion(version));
    }

    // Content hash.
    let stored_hash: [u8; 32] = data[8..40].try_into().unwrap();
    let computed = Sha256::digest(&data[HEADER_SIZE..]);
    let mut computed_arr = [0u8; 32];
    computed_arr.copy_from_slice(&computed);

    if stored_hash != computed_arr {
        return Err(WireFormatError::HashMismatch);
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// TypeMappingRegistry
// ---------------------------------------------------------------------------

/// Registry for cross-language type mappings, keyed by schema hash.
#[derive(Debug, Clone, Default)]
pub struct TypeMappingRegistry {
    by_hash: HashMap<[u8; 32], TypeMapping>,
    by_name: HashMap<String, [u8; 32]>,
}

impl TypeMappingRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type mapping. Overwrites any existing mapping for the same hash.
    pub fn register(&mut self, mapping: TypeMapping) {
        let hash = mapping.schema_hash;
        self.by_name.insert(mapping.shape_type.clone(), hash);
        self.by_hash.insert(hash, mapping);
    }

    /// Look up a type mapping by its schema hash.
    pub fn get_by_hash(&self, hash: &[u8; 32]) -> Option<&TypeMapping> {
        self.by_hash.get(hash)
    }

    /// Look up a type mapping by its Shape type name.
    pub fn get_by_name(&self, name: &str) -> Option<&TypeMapping> {
        self.by_name.get(name).and_then(|h| self.by_hash.get(h))
    }

    /// Number of registered type mappings.
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Iterate over all registered type mappings.
    pub fn iter(&self) -> impl Iterator<Item = &TypeMapping> {
        self.by_hash.values()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_metadata() -> BlobMetadata {
        BlobMetadata {
            name: "test_fn".to_string(),
            arity: 2,
            locals_count: 3,
            is_async: false,
            is_closure: false,
            captures_count: 0,
            param_names: vec!["a".to_string(), "b".to_string()],
            ref_params: vec![false, false],
            ref_mutates: vec![false, false],
            mutable_captures: vec![],
        }
    }

    fn make_test_encodable() -> EncodableBlob {
        let meta = make_test_metadata();
        // Use simple placeholder data for instructions/constants.
        let instructions: Vec<u8> = vec![1, 2, 3, 4]; // opaque payload
        let constants: Vec<i64> = vec![42, 100];
        let strings: Vec<String> = vec!["a".into(), "b".into()];
        let deps: Vec<[u8; 32]> = vec![];
        let type_schemas: Vec<String> = vec!["MyType".into()];
        let source_map: Vec<(usize, u32, u32)> = vec![(0, 0, 1), (2, 0, 2)];
        let perms: Vec<&str> = vec![];

        EncodableBlob::from_parts(
            &meta,
            &instructions,
            &constants,
            &strings,
            &deps,
            &type_schemas,
            &source_map,
            &perms,
        )
        .unwrap()
    }

    #[test]
    fn roundtrip_encode_decode() {
        let encodable = make_test_encodable();
        let encoder = BlobEncoder::new(&encodable);
        let wire_bytes = encoder.encode_to_bytes().unwrap();

        // Validate.
        assert!(validate_blob(&wire_bytes).unwrap());

        // Decode.
        let decoded = BlobDecoder::decode_from_bytes(&wire_bytes).unwrap();

        assert_eq!(decoded.metadata.name, "test_fn");
        assert_eq!(decoded.metadata.arity, 2);
        assert_eq!(decoded.metadata.locals_count, 3);
        assert!(!decoded.metadata.is_async);
        assert!(!decoded.metadata.is_closure);
        assert_eq!(decoded.metadata.captures_count, 0);
        assert_eq!(
            decoded.metadata.param_names,
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(decoded.strings, vec!["a".to_string(), "b".to_string()]);
        assert!(decoded.dependencies.is_empty());
        assert_eq!(decoded.type_schemas, vec!["MyType".to_string()]);
        assert_eq!(decoded.source_map, vec![(0, 0, 1), (2, 0, 2)]);
        assert!(decoded.permissions.is_empty());
    }

    #[test]
    fn validates_magic() {
        let encodable = make_test_encodable();
        let mut wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();

        // Corrupt magic.
        wire_bytes[0] = 0xFF;
        assert!(matches!(
            validate_blob(&wire_bytes),
            Err(WireFormatError::InvalidMagic)
        ));
    }

    #[test]
    fn validates_version() {
        let encodable = make_test_encodable();
        let mut wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();

        // Set version to 99.
        wire_bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        assert!(matches!(
            validate_blob(&wire_bytes),
            Err(WireFormatError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn detects_hash_mismatch() {
        let encodable = make_test_encodable();
        let mut wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();

        // Corrupt a byte in the data section.
        let last = wire_bytes.len() - 1;
        wire_bytes[last] ^= 0xFF;
        assert!(matches!(
            validate_blob(&wire_bytes),
            Err(WireFormatError::HashMismatch)
        ));
    }

    #[test]
    fn too_short_data() {
        assert!(matches!(
            validate_blob(&[0u8; 10]),
            Err(WireFormatError::TooShort { .. })
        ));
        assert!(matches!(
            BlobDecoder::decode_from_bytes(&[0u8; 10]),
            Err(WireFormatError::TooShort { .. })
        ));
    }

    #[test]
    fn content_hash_is_nonzero_and_validates() {
        let encodable = make_test_encodable();
        let wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();
        let decoded = BlobDecoder::decode_from_bytes(&wire_bytes).unwrap();

        assert_ne!(decoded.content_hash, [0u8; 32]);
        assert!(validate_blob(&wire_bytes).unwrap());
    }

    #[test]
    fn encode_decode_with_dependencies() {
        let meta = BlobMetadata {
            name: "caller".to_string(),
            arity: 0,
            locals_count: 1,
            is_async: true,
            is_closure: false,
            captures_count: 0,
            param_names: vec![],
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
        };

        let dep_hash = [0xAAu8; 32];
        let encodable = EncodableBlob::from_parts(
            &meta,
            &Vec::<u8>::new(),
            &Vec::<i64>::new(),
            &Vec::<String>::new(),
            &[dep_hash],
            &Vec::<String>::new(),
            &Vec::<(usize, u32, u32)>::new(),
            &Vec::<&str>::new(),
        )
        .unwrap();

        let wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();
        assert!(validate_blob(&wire_bytes).unwrap());

        let decoded = BlobDecoder::decode_from_bytes(&wire_bytes).unwrap();
        assert_eq!(decoded.dependencies.len(), 1);
        assert_eq!(decoded.dependencies[0], [0xAA; 32]);
        assert!(decoded.metadata.is_async);
    }

    #[test]
    fn type_mapping_registry_operations() {
        let mut registry = TypeMappingRegistry::new();
        assert!(registry.is_empty());

        let hash = [42u8; 32];
        let mapping = TypeMapping {
            shape_type: "Point".to_string(),
            schema_hash: hash,
            fields: vec![
                TypeFieldMapping {
                    name: "x".to_string(),
                    field_type: WireType::Float64,
                    offset: 0,
                },
                TypeFieldMapping {
                    name: "y".to_string(),
                    field_type: WireType::Float64,
                    offset: 8,
                },
            ],
        };

        registry.register(mapping);
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());

        let found = registry.get_by_hash(&hash).unwrap();
        assert_eq!(found.shape_type, "Point");
        assert_eq!(found.fields.len(), 2);

        let found_by_name = registry.get_by_name("Point").unwrap();
        assert_eq!(found_by_name.schema_hash, hash);

        assert!(registry.get_by_name("NonExistent").is_none());
        assert!(registry.get_by_hash(&[0u8; 32]).is_none());
    }

    #[test]
    fn type_mapping_registry_overwrite() {
        let mut registry = TypeMappingRegistry::new();
        let hash = [1u8; 32];

        registry.register(TypeMapping {
            shape_type: "Foo".into(),
            schema_hash: hash,
            fields: vec![],
        });
        assert_eq!(registry.len(), 1);

        // Overwrite same hash.
        registry.register(TypeMapping {
            shape_type: "Foo".into(),
            schema_hash: hash,
            fields: vec![TypeFieldMapping {
                name: "x".into(),
                field_type: WireType::Int64,
                offset: 0,
            }],
        });
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get_by_hash(&hash).unwrap().fields.len(), 1);
    }

    #[test]
    fn wire_type_complex_nesting() {
        let complex = WireType::Map(
            Box::new(WireType::String),
            Box::new(WireType::Optional(Box::new(WireType::Array(Box::new(
                WireType::Ref([0xAB; 32]),
            ))))),
        );

        let bytes = rmp_serde::encode::to_vec(&complex).unwrap();
        let decoded: WireType = rmp_serde::decode::from_slice(&bytes).unwrap();

        // Verify the round-trip by re-encoding.
        let bytes2 = rmp_serde::encode::to_vec(&decoded).unwrap();
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn decode_rejects_truncated_section_table() {
        let encodable = make_test_encodable();
        let wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();

        // Truncate right after the header so the section table is incomplete.
        let truncated = &wire_bytes[..HEADER_SIZE + 5];
        assert!(matches!(
            BlobDecoder::decode_from_bytes(truncated),
            Err(WireFormatError::TooShort { .. })
        ));
    }

    #[test]
    fn encode_with_permissions() {
        let meta = BlobMetadata {
            name: "needs_io".into(),
            arity: 0,
            locals_count: 0,
            is_async: false,
            is_closure: false,
            captures_count: 0,
            param_names: vec![],
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
        };

        let perms: Vec<&str> = vec!["io.read", "io.write"];
        let encodable = EncodableBlob::from_parts(
            &meta,
            &Vec::<u8>::new(),
            &Vec::<i64>::new(),
            &Vec::<String>::new(),
            &[],
            &Vec::<String>::new(),
            &Vec::<(usize, u32, u32)>::new(),
            &perms,
        )
        .unwrap();

        let wire_bytes = BlobEncoder::new(&encodable).encode_to_bytes().unwrap();
        assert!(validate_blob(&wire_bytes).unwrap());

        let decoded = BlobDecoder::decode_from_bytes(&wire_bytes).unwrap();
        assert_eq!(decoded.permissions, vec!["io.read", "io.write"]);
    }
}
