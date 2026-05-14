//! Native `xml` module for XML parsing and serialization.
//!
//! Exports: xml.parse(text), xml.stringify(value)
//!
//! XML nodes are represented as Shape TypedObjects with the `XmlNode`
//! schema: `{ name: string, attributes: HashMap<string, string>,
//!            children: Array<XmlNode>, text: string }`
//!
//! W17-out-of-bundle-A-followups (2026-05-12): children rewire per the
//! C+ precedent recorded in `phase-2d-playbook.md` §3
//! ("Bundle-A checkpoint-2 amendment"). Pre-rewire, each child was an
//! `Arc<HeapValue::HashMap>` carried inside the deleted
//! `TypedArrayData::HeapValue` arm. Post-rewire, each child is an
//! `Arc<HeapValue::TypedObject>` with the registered `XmlNode` schema,
//! and the outer children array lowers to `TypedArrayData::TypedObject`
//! per ADR-006 §2.7.24 Q25.A's specialized list.
//!
//! User-visible API: `node.children[i].name` / `.attributes` / `.text`
//! continue to work via TypedObject field access (same shape as the
//! prior HashMap dispatch). The `text` field is now always present
//! (empty string when absent); the prior optional-field shape was
//! already flattened.
//!
//! Stage C HashMap-marshal P1(b) historical context (2026-05-07):
//! - `xml.parse` returns the root element as `TypedReturn::OkObjectPairs`
//!   per Cluster #4 β shape (mirrors `arrow.metadata` / http.rs precedents).
//! - `xml.stringify` takes `value: HashMap<string, *>` typed input via
//!   `Vec<(Arc<String>, Arc<HeapValue>)>` FromSlot from Step 1 P1(b)
//!   infrastructure (commit `36519f6`). Walks the recursive HeapValue
//!   tree using direct pattern matching — no marshal-boundary
//!   re-entry per element. The reader now dispatches the `children`
//!   field through `TypedArrayData::TypedObject` per the post-rewire
//!   construction shape.
//! - Attributes (`HashMap<string, string>`) carried via
//!   `ConcreteReturn::HashMapStringString` on output and read directly
//!   from `HeapValue::HashMap(d)` on input.
//!
//! Tests deleted along with the legacy ValueWord-based fixtures, mirroring
//! the csv_module migration (commit `9f6b1d3`). New typed-marshal test
//! harness arrives with the shape-vm cleanup workstream.

use crate::marshal::{register_typed_fn_1, register_typed_fn_1_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::type_schema::register_predeclared_any_schema;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use shape_value::heap_value::{HashMapData, HeapValue, TypedArrayData, TypedObjectStorage};
use shape_value::{HeapKind, NativeKind, TypedBuffer, ValueSlot};
use std::io::Cursor;
use std::sync::Arc;

/// XmlNode schema field order: matches `into_typed_object_arc` field-pair
/// order. The schema is auto-registered via
/// `register_predeclared_any_schema` on first use so the field list is the
/// single source of truth.
const XML_NODE_FIELDS: &[&str] = &["name", "attributes", "children", "text"];

/// Parsed XML element data: a recursive structure where each element has
/// a name, attribute pairs, child elements, and optional text content.
struct ElementData {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<ElementData>,
    text: Option<String>,
}

impl ElementData {
    /// Project this element into a `HeapValue::TypedObject(...)` with
    /// the `XmlNode` schema (W17-out-of-bundle-A-followups, 2026-05-12).
    /// Children are recursively projected through this method and form
    /// a `TypedArrayData::TypedObject` array — no polymorphic
    /// `Array<HashMap>` carrier. Per C+ precedent the schema is
    /// auto-registered via `register_predeclared_any_schema`.
    ///
    /// Field order matches `XML_NODE_FIELDS` (name, attributes,
    /// children, text). `text` is always present at the slot level
    /// (empty string when the source XML had no text node) so the
    /// schema is fixed-arity and the type is exhaustive — no Option
    /// indirection at the storage layer.
    fn into_typed_object_arc(self) -> Arc<HeapValue> {
        let attrs_data = HashMapData::from_pairs(
            self.attributes
                .iter()
                .map(|(k, _)| Arc::new(k.clone()))
                .collect(),
            self.attributes
                .iter()
                .map(|(_, v)| Arc::new(HeapValue::String(Arc::new(v.clone()))))
                .collect(),
        );
        // Recurse: each child becomes its own TypedObject.
        let child_objs: Vec<Arc<TypedObjectStorage>> = self
            .children
            .into_iter()
            .map(|c| {
                let child_hv = c.into_typed_object_arc();
                match &*child_hv {
                    HeapValue::TypedObject(s) => Arc::clone(s),
                    _ => unreachable!(
                        "into_typed_object_arc must return HeapValue::TypedObject"
                    ),
                }
            })
            .collect();
        let children_array_data = TypedArrayData::TypedObject(Arc::new(
            TypedBuffer::from_vec(child_objs),
        ));

        let schema_id = ensure_xml_node_schema();
        // Field-order: name(0), attributes(1), children(2), text(3).
        // Heap mask: name(String), attributes(HashMap), children(TypedArray),
        // text(String) — all 4 fields are heap-resident.
        let name_arc = Arc::new(self.name);
        let attrs_arc = Arc::new(attrs_data);
        let children_arc = Arc::new(children_array_data);
        let text_arc = Arc::new(self.text.unwrap_or_default());

        let slots: Box<[ValueSlot]> = Box::new([
            ValueSlot::from_string_arc(name_arc),
            ValueSlot::from_hashmap(attrs_arc),
            ValueSlot::from_typed_array(children_arc),
            ValueSlot::from_string_arc(text_arc),
        ]);
        let field_kinds: Arc<[NativeKind]> = Arc::from(
            vec![
                NativeKind::String,
                NativeKind::Ptr(HeapKind::HashMap),
                NativeKind::Ptr(HeapKind::TypedArray),
                NativeKind::String,
            ]
            .into_boxed_slice(),
        );
        let heap_mask: u64 = 0b1111; // all 4 fields heap-resident
        // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new` per D1 API
        // surface. `HeapValue::TypedObject` variant signature flip is
        // ckpt-final territory; the wrap below will not compile until
        // the variant signature shifts to `*const TypedObjectStorage`.
        let storage = TypedObjectStorage::_new(
            schema_id as u64,
            slots,
            heap_mask,
            field_kinds,
        );
        Arc::new(HeapValue::TypedObject(storage))
    }

    /// Project this element's TOP-LEVEL form as a `Vec<(String,
    /// ConcreteReturn)>` pair-list, suitable for `TypedReturn::OkObjectPairs`.
    /// Used only for the root element of `xml.parse`'s return value;
    /// nested elements go through `into_typed_object_arc` instead.
    fn into_root_pairs(self) -> Vec<(String, ConcreteReturn)> {
        let attrs_pairs: Vec<(String, String)> = self.attributes;
        // Each child is now an `Arc<HeapValue::TypedObject>`. The marshal
        // boundary's `ConcreteReturn::ArrayHeapValue` consumer routes
        // through `TypedArrayData::build_specialized_from_heap_arcs`,
        // which already dispatches the `HeapValue::TypedObject` arm to
        // `TypedArrayData::TypedObject` per ADR-006 §2.7.24 Q25.A. No
        // out-of-territory follow-up: the rewire is structurally
        // resolved by C+ precedent.
        let children_arc: Vec<Arc<HeapValue>> = self
            .children
            .into_iter()
            .map(ElementData::into_typed_object_arc)
            .collect();

        let mut pairs = vec![
            ("name".to_string(), ConcreteReturn::String(self.name)),
            (
                "attributes".to_string(),
                ConcreteReturn::HashMapStringString(attrs_pairs),
            ),
            (
                "children".to_string(),
                ConcreteReturn::ArrayHeapValue(children_arc),
            ),
        ];
        // `text?` follows the regex.rs precedent: emit empty string when
        // absent. Keeps the schema fixed at 4 fields when text is present
        // and 3 fields when absent — variable-length pair list per the
        // ObjectPairs contract.
        if let Some(text) = self.text {
            pairs.push(("text".to_string(), ConcreteReturn::String(text)));
        }
        pairs
    }
}

/// Register the `XmlNode` predeclared schema (auto-registered on first
/// use; subsequent calls return the cached SchemaId via the registry's
/// own deduplication). Returns the raw `u32` schema id used by
/// `TypedObjectStorage::schema_id`.
fn ensure_xml_node_schema() -> u32 {
    let owned: Vec<String> = XML_NODE_FIELDS.iter().map(|s| s.to_string()).collect();
    register_predeclared_any_schema(&owned)
}

/// Parse an XML element recursively from a quick-xml reader.
fn parse_element(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart,
) -> Result<ElementData, String> {
    let name = std::str::from_utf8(start.name().as_ref())
        .map_err(|e| format!("Invalid UTF-8 in element name: {}", e))?
        .to_string();

    let mut attributes = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("Invalid attribute: {}", e))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|e| format!("Invalid UTF-8 in attribute key: {}", e))?
            .to_string();
        let value = attr
            .unescape_value()
            .map_err(|e| format!("Invalid attribute value: {}", e))?
            .to_string();
        attributes.push((key, value));
    }

    let mut children = Vec::new();
    let mut text_parts = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let child = parse_element(reader, e)?;
                children.push(child);
            }
            Ok(Event::Empty(ref e)) => {
                let child = parse_empty_element(e)?;
                children.push(child);
            }
            Ok(Event::Text(ref e)) => {
                let t = e
                    .unescape()
                    .map_err(|err| format!("Error unescaping text: {}", err))?
                    .to_string();
                let trimmed = t.trim().to_string();
                if !trimmed.is_empty() {
                    text_parts.push(trimmed);
                }
            }
            Ok(Event::CData(ref e)) => {
                let t = std::str::from_utf8(e.as_ref())
                    .map_err(|err| format!("Invalid UTF-8 in CDATA: {}", err))?
                    .to_string();
                if !t.trim().is_empty() {
                    text_parts.push(t);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => {
                return Err("Unexpected end of XML".to_string());
            }
            Ok(_) => {} // Skip comments, PI, etc.
            Err(e) => return Err(format!("XML parse error: {}", e)),
        }
        buf.clear();
    }

    Ok(ElementData {
        name,
        attributes,
        children,
        text: if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        },
    })
}

/// Parse a self-closing XML element (e.g. `<br/>`).
fn parse_empty_element(start: &BytesStart) -> Result<ElementData, String> {
    let name = std::str::from_utf8(start.name().as_ref())
        .map_err(|e| format!("Invalid UTF-8 in element name: {}", e))?
        .to_string();

    let mut attributes = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("Invalid attribute: {}", e))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|e| format!("Invalid UTF-8 in attribute key: {}", e))?
            .to_string();
        let value = attr
            .unescape_value()
            .map_err(|e| format!("Invalid attribute value: {}", e))?
            .to_string();
        attributes.push((key, value));
    }

    Ok(ElementData {
        name,
        attributes,
        children: Vec::new(),
        text: None,
    })
}

/// Walk a top-level node — represented as a `(keys, values)` pair-list
/// from the marshal boundary — and emit the corresponding XML via the
/// writer. The top-level input from `xml.stringify` is still keyed by
/// field name (the `Vec<(Arc<String>, Arc<HeapValue>)>` FromSlot
/// shape); children recurse through `write_typed_object_node` against
/// `HeapValue::TypedObject` arms now that `into_typed_object_arc`
/// produces TypedObject per child (W17-out-of-bundle-A-followups,
/// 2026-05-12).
fn write_node_pairs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    pairs: &[(Arc<String>, Arc<HeapValue>)],
) -> Result<(), String> {
    let mut name: Option<String> = None;
    let mut attrs: Option<Arc<HashMapData>> = None;
    let mut children: Option<Arc<TypedArrayData>> = None;
    let mut text: Option<String> = None;

    for (k, v) in pairs.iter() {
        match k.as_str() {
            "name" => {
                if let HeapValue::String(s) = &**v {
                    name = Some((**s).clone());
                }
            }
            "attributes" => {
                if let HeapValue::HashMap(d) = &**v {
                    attrs = Some(Arc::clone(d));
                }
            }
            "children" => {
                if let HeapValue::TypedArray(arr) = &**v {
                    children = Some(Arc::clone(arr));
                }
            }
            "text" => {
                if let HeapValue::String(s) = &**v {
                    if !s.is_empty() {
                        text = Some((**s).clone());
                    }
                }
            }
            _ => {}
        }
    }

    write_xml_element(writer, name, attrs.as_deref(), children.as_deref(), text.as_deref())
}

/// Walk a child node — represented as an `Arc<TypedObjectStorage>` with
/// the `XmlNode` schema. Reads each field via `field_index_in_schema`
/// since the schema is auto-registered and field-order is locked to
/// `XML_NODE_FIELDS`.
///
/// W17-out-of-bundle-A-followups (2026-05-12): replaces the previous
/// `write_node_heap` HashMap-element reader. The construction side
/// (`ElementData::into_typed_object_arc`) builds TypedObjects per
/// child, so the array's elements arrive here as TypedObjects, not
/// HashMaps.
fn write_typed_object_node(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    storage: &TypedObjectStorage,
) -> Result<(), String> {
    // Match field order from `XML_NODE_FIELDS`. The construction side
    // writes slots in this exact order; the schema registration uses
    // the same field list, so positional access is sound.
    if storage.slots.len() != XML_NODE_FIELDS.len() {
        return Err(format!(
            "xml.stringify(): child TypedObject has {} slots, expected {}",
            storage.slots.len(),
            XML_NODE_FIELDS.len()
        ));
    }
    let name_slot = &storage.slots[0];
    let attrs_slot = &storage.slots[1];
    let children_slot = &storage.slots[2];
    let text_slot = &storage.slots[3];

    // SAFETY for each slot: the construction-side contract in
    // `ElementData::into_typed_object_arc` writes each slot as
    // `ValueSlot::from_string_arc` / `from_hashmap` / `from_typed_array`
    // — the bits are `Arc::into_raw::<T>` for the matching `T`. We
    // bump the strong count, recover via `Arc::from_raw`, then drop the
    // bumped share after extracting a clone of the payload (the
    // storage's own share remains intact; this is the canonical
    // 5-arm receiver-recovery shape from `3ac2f11`).
    let name: String = unsafe {
        let bits = name_slot.raw();
        if bits == 0 {
            return Err("xml.stringify(): TypedObject name slot is null".to_string());
        }
        let arc_ptr = bits as *const String;
        Arc::increment_strong_count(arc_ptr);
        let arc = Arc::from_raw(arc_ptr);
        let owned = (*arc).clone();
        // `arc` Drop here releases our bumped share; storage's share
        // is untouched.
        owned
    };
    let attrs_arc: Option<Arc<HashMapData>> = unsafe {
        let bits = attrs_slot.raw();
        if bits == 0 {
            None
        } else {
            let arc_ptr = bits as *const HashMapData;
            Arc::increment_strong_count(arc_ptr);
            Some(Arc::from_raw(arc_ptr))
        }
    };
    let children_arc: Option<Arc<TypedArrayData>> = unsafe {
        let bits = children_slot.raw();
        if bits == 0 {
            None
        } else {
            let arc_ptr = bits as *const TypedArrayData;
            Arc::increment_strong_count(arc_ptr);
            Some(Arc::from_raw(arc_ptr))
        }
    };
    let text: Option<String> = unsafe {
        let bits = text_slot.raw();
        if bits == 0 {
            None
        } else {
            let arc_ptr = bits as *const String;
            Arc::increment_strong_count(arc_ptr);
            let arc = Arc::from_raw(arc_ptr);
            let owned = (*arc).clone();
            if owned.is_empty() {
                None
            } else {
                Some(owned)
            }
        }
    };

    write_xml_element(
        writer,
        Some(name),
        attrs_arc.as_deref(),
        children_arc.as_deref(),
        text.as_deref(),
    )
}

/// Shared element-writer body — emits the XML representation of a node
/// given the four parsed XmlNode fields. Pulled out so the top-level
/// `write_node_pairs` path and the recursive
/// `write_typed_object_node` path share the same output discipline.
fn write_xml_element(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    name: Option<String>,
    attrs: Option<&HashMapData>,
    children: Option<&TypedArrayData>,
    text: Option<&str>,
) -> Result<(), String> {
    let name = name.ok_or_else(|| "xml.stringify(): node missing 'name' field".to_string())?;

    let mut elem = BytesStart::new(name.clone());

    if let Some(attrs) = attrs {
        let n = attrs.keys.data.len();
        for i in 0..n {
            let ak = &attrs.keys.data[i];
            let av = attrs.values.value_at(i);
            if let HeapValue::String(av_s) = &*av {
                elem.push_attribute((ak.as_str(), av_s.as_str()));
            }
        }
    }

    let child_objs: Option<&Arc<TypedBuffer<Arc<TypedObjectStorage>>>> = match children {
        Some(TypedArrayData::TypedObject(buf)) => Some(buf),
        _ => None,
    };
    let has_children = child_objs.map(|b| !b.data.is_empty()).unwrap_or(false);
    let has_text = text.is_some();

    if !has_children && !has_text {
        writer
            .write_event(Event::Empty(elem))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;
    } else {
        writer
            .write_event(Event::Start(elem.clone()))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;

        if let Some(text) = text {
            writer
                .write_event(Event::Text(BytesText::new(text)))
                .map_err(|e| format!("xml.stringify() write error: {}", e))?;
        }

        if let Some(buf) = child_objs {
            for child in buf.data.iter() {
                write_typed_object_node(writer, child)?;
            }
        }

        writer
            .write_event(Event::End(BytesEnd::new(name)))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;
    }

    Ok(())
}

/// Create the `xml` module with XML parsing and serialization functions.
pub fn create_xml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::xml");
    module.description = "XML parsing and serialization".to_string();

    // xml.parse(text: string) -> Result<HashMap>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "parse",
        "Parse an XML string into a Shape HashMap node",
        "text",
        "string",
        ConcreteType::Result(Box::new(ConcreteType::HashMap)),
        |text, _ctx| {
            let mut reader = Reader::from_str(text.as_str());
            reader.config_mut().trim_text(true);
            let mut buf = Vec::new();

            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref e)) => {
                        let inner = parse_element(&mut reader, e)?;
                        return Ok(TypedReturn::OkObjectPairs(inner.into_root_pairs()));
                    }
                    Ok(Event::Empty(ref e)) => {
                        let inner = parse_empty_element(e)?;
                        return Ok(TypedReturn::OkObjectPairs(inner.into_root_pairs()));
                    }
                    Ok(Event::Eof) => {
                        return Err("xml.parse(): no root element found".to_string());
                    }
                    Ok(_) => {} // Skip declaration, comments, PI
                    Err(e) => {
                        return Err(format!("xml.parse() failed: {}", e));
                    }
                }
                buf.clear();
            }
        },
    );

    // xml.stringify(value: HashMap<string, any>) -> Result<string>
    register_typed_fn_1_full::<_, Vec<(Arc<String>, Arc<HeapValue>)>>(
        &mut module,
        "stringify",
        "Serialize a Shape HashMap node to an XML string",
        [ModuleParam {
            name: "value".to_string(),
            type_name: "HashMap<string, any>".to_string(),
            required: true,
            description:
                "Node value to serialize (with name, attributes, children, text? fields)"
                    .to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |pairs: Vec<(Arc<String>, Arc<HeapValue>)>, _ctx| {
            let mut writer = Writer::new(Cursor::new(Vec::new()));
            write_node_pairs(&mut writer, &pairs)?;

            let output = String::from_utf8(writer.into_inner().into_inner())
                .map_err(|e| format!("xml.stringify(): invalid UTF-8 output: {}", e))?;

            Ok(TypedReturn::Ok(ConcreteReturn::String(output)))
        },
    );

    module
}
