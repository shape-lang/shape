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

use crate::json_value::JsonValue;
use crate::marshal::{register_typed_fn_1, register_typed_fn_1_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::type_schema::register_predeclared_any_schema;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use shape_value::heap_value::{HashMapData, HeapValue, TypedObjectStorage};
use shape_value::v2::typed_array::TypedArray;
use shape_value::{HeapKind, NativeKind, ValueSlot};
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
        // Wave 2 Round 3b C2-joint ckpt-4 (2026-05-14): build the XML
        // attributes HashMap via the per-V mutation API on
        // `HashMapData<*const StringObj>` (V = string). Each (k, v) pair
        // becomes one fresh StringObj insert; the wrapper carries one
        // refcount share per element. ADR-006 §2.7.24 Q25.B SUPERSEDED.
        let mut attrs_data: HashMapData<*const shape_value::v2::string_obj::StringObj> =
            HashMapData::new();
        for (k, v) in &self.attributes {
            let v_obj = shape_value::v2::string_obj::StringObj::new(v.as_str())
                as *const shape_value::v2::string_obj::StringObj;
            unsafe { attrs_data.insert(k.as_str(), v_obj) };
        }
        let attrs_data: shape_value::heap_value::HashMapKindedRef =
            shape_value::heap_value::HashMapKindedRef::String(Arc::new(attrs_data));
        // Recurse: each child becomes its own TypedObject. The child raw
        // `*const TypedObjectStorage` pointers are packed into a
        // `*mut TypedArray<*const TypedObjectStorage>` flat-struct carrier
        // per V3-S5 ckpt-5-prime²c Migration shape (a) (the deleted
        // `TypedArrayData::TypedObject` enum-arm shape). The
        // `TypedObjectStorage` type impls `v2::heap_element::HeapElement`
        // (`heap_value.rs:3971`), so per-element retain/release dispatches
        // through `v2_retain` / `v2_release` on the on-header refcount.
        //
        // Each child `into_typed_object_arc()` returns an `Arc<HeapValue>`
        // wrapping `HeapValue::TypedObject(TypedObjectPtr)` — we extract
        // the inner raw pointer via `into_raw()` (transferring the
        // wrapper's one refcount share to the raw pointer, which the
        // `TypedArray` takes ownership of as an element).
        let child_ptrs: Vec<*const TypedObjectStorage> = self
            .children
            .into_iter()
            .map(|c| {
                let child_hv = c.into_typed_object_arc();
                // Extract inner TypedObjectPtr by cloning out and consuming.
                let to_ptr = match &*child_hv {
                    HeapValue::TypedObject(s) => s.clone(),
                    _ => unreachable!("into_typed_object_arc must return HeapValue::TypedObject"),
                };
                to_ptr.into_raw()
            })
            .collect();
        let children_arr: *mut TypedArray<*const TypedObjectStorage> =
            TypedArray::<*const TypedObjectStorage>::from_slice(&child_ptrs);
        // WF-2E (2026-07-05): `from_slice` does NOT stamp the element-type
        // discriminant (offset-7 header byte) — it stays `ELEM_TYPE_UNKNOWN`
        // (0). Stamp it `ELEM_TYPE_TYPED_OBJECT` so the producer-side element
        // contract is complete: any element-type-aware consumer (drop
        // dispatch, and the WF-2E object-graph marshal reader
        // `typed_array_to_json_value`) reads the children as TypedObjects.
        // Without this stamp, `xml.parse(...) |> xml.stringify` fails with
        // "element-type discriminant 0 has no JSON serialization policy".
        unsafe {
            shape_value::v2::typed_array::stamp_elem_type(
                children_arr as *mut u8,
                shape_value::v2::typed_array::ELEM_TYPE_TYPED_OBJECT,
            );
        }
        // `from_slice` copies each `*const TypedObjectStorage` bit-for-bit
        // (raw pointers are Copy). The refcount shares were transferred
        // from the source `TypedObjectPtr` wrappers into raw pointers
        // already; the source `Vec<*const _>` doesn't own any share, so
        // ordinary Drop suffices for the source Vec's heap allocation.
        // Element-share ownership now lives with the array.

        let schema_id = ensure_xml_node_schema();
        // Field-order: name(0), attributes(1), children(2), text(3).
        // Heap mask: name(String), attributes(HashMap), children(TypedArray),
        // text(String) — all 4 fields are heap-resident.
        let name_arc = Arc::new(self.name);
        let attrs_arc = Arc::new(attrs_data);
        let text_arc = Arc::new(self.text.unwrap_or_default());

        let slots: Box<[ValueSlot]> = Box::new([
            ValueSlot::from_string_arc(name_arc),
            ValueSlot::from_hashmap(attrs_arc),
            // V3-S5 ckpt-5-prime²c (2026-05-15) Migration shape (a): the
            // `ValueSlot::from_typed_array(Arc<TypedArrayData>)` constructor
            // is deleted; per-element-kind constructors aren't landed yet
            // (Round 2 follow-up). Store the raw `*mut TypedArray<T>`
            // pointer directly via `ValueSlot::from_u64` — this is the
            // canonical slot-bit shape for `NativeKind::Ptr(HeapKind::
            // TypedArray)` per `docs/runtime-v2-spec.md`. The schema's
            // field_kinds[2] = `Ptr(HeapKind::TypedArray)` controls
            // drop dispatch at slot release time.
            ValueSlot::from_u64(children_arr as u64),
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
        // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): variant signature
        // flipped to `HeapValue::TypedObject(TypedObjectPtr)`. The
        // `_new`-returned raw pointer (refcount=1) is wrapped in
        // `TypedObjectPtr`, transferring the share to the wrapper.
        let storage = TypedObjectStorage::_new(schema_id as u64, slots, heap_mask, field_kinds);
        Arc::new(HeapValue::TypedObject(
            shape_value::heap_value::TypedObjectPtr::new(storage),
        ))
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
fn parse_element(reader: &mut Reader<&[u8]>, start: &BytesStart) -> Result<ElementData, String> {
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

/// Serialize an XML node tree to an XML string.
///
/// WF-2E object-graph-marshal consumer (2026-07-05). The `value` argument
/// arrives fully walked into a `JsonValue` tree at the marshal boundary
/// (typed, kind-directed — no pointer reinterpretation), so this walker
/// only has to emit XML from the tree. A node is a `JsonValue::Object`
/// with a required `name` string field and optional `attributes`
/// (object of string→scalar), `children` (array of nodes), and `text`
/// (string) fields — the same `XmlNode` shape `xml.parse` produces.
fn write_xml_json_node(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    node: &JsonValue,
) -> Result<(), String> {
    let fields = match node {
        JsonValue::Object(fields) => fields,
        other => {
            return Err(format!(
                "xml.stringify(): expected an object node with a 'name' field, got {}",
                other.type_name()
            ));
        }
    };

    let mut name: Option<&str> = None;
    let mut attributes: Option<&Vec<(String, JsonValue)>> = None;
    let mut children: Option<&Vec<JsonValue>> = None;
    let mut text: Option<&str> = None;
    for (k, v) in fields.iter() {
        match (k.as_str(), v) {
            ("name", JsonValue::String(s)) => name = Some(s.as_str()),
            ("attributes", JsonValue::Object(attrs)) => attributes = Some(attrs),
            ("children", JsonValue::Array(arr)) => children = Some(arr),
            ("text", JsonValue::String(s)) => text = Some(s.as_str()),
            // Absent / null fields (e.g. `attributes: null`) are ignored.
            _ => {}
        }
    }

    let name =
        name.ok_or_else(|| "xml.stringify(): node missing 'name' string field".to_string())?;
    let mut elem = BytesStart::new(name);

    if let Some(attrs) = attributes {
        for (k, v) in attrs.iter() {
            let val = match v {
                JsonValue::String(s) => s.clone(),
                JsonValue::Int(i) => i.to_string(),
                JsonValue::Number(n) => n.to_string(),
                JsonValue::Bool(b) => b.to_string(),
                JsonValue::Null => String::new(),
                other => {
                    return Err(format!(
                        "xml.stringify(): attribute '{}' has non-scalar value ({})",
                        k,
                        other.type_name()
                    ));
                }
            };
            elem.push_attribute((k.as_bytes(), val.as_bytes()));
        }
    }

    let has_children = children.map(|c| !c.is_empty()).unwrap_or(false);
    let has_text = text.map(|t| !t.is_empty()).unwrap_or(false);

    if !has_children && !has_text {
        writer
            .write_event(Event::Empty(elem))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;
    } else {
        writer
            .write_event(Event::Start(elem))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;

        if let Some(t) = text {
            if !t.is_empty() {
                writer
                    .write_event(Event::Text(BytesText::new(t)))
                    .map_err(|e| format!("xml.stringify() write error: {}", e))?;
            }
        }

        if let Some(arr) = children {
            for child in arr.iter() {
                write_xml_json_node(writer, child)?;
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

    // xml.stringify(value: object) -> Result<string>
    //
    // WF-2E (2026-07-05): the `value` node is walked into a `JsonValue`
    // tree at the marshal boundary via the shared object-graph marshal
    // (`FromSlot<JsonValue>` → `slot_to_json_value`), dispatching on the
    // stamped `NativeKind`. This replaces the SIGSEGV-prone
    // `Vec<(Arc<String>, Arc<HeapValue>)>` shape whose blind `from_slot`
    // cast a `Ptr(HeapKind::TypedObject)` node (every object literal, and
    // `xml.parse`'s own output) as a `HashMapKindedRef` and segfaulted.
    register_typed_fn_1_full::<_, crate::marshal::PolymorphicArg>(
        &mut module,
        "stringify",
        "Serialize a Shape XML node object to an XML string",
        [ModuleParam {
            name: "value".to_string(),
            type_name: "object".to_string(),
            required: true,
            description: "Node value to serialize (with name, attributes, children, text? fields)"
                .to_string(),
            ..Default::default()
        }],
        ConcreteType::Result(Box::new(ConcreteType::String)),
        |value: crate::marshal::PolymorphicArg, ctx| {
            // WF-2E (2026-07-05): walk the node into a `JsonValue` tree using
            // the EXECUTION registry (`ctx.schemas`) for TypedObject field-name
            // resolution — the ambient `None` path resolves through
            // `current_registry()`, which under JIT can be the process-default
            // registry lacking the program's `XmlNode` schema (VM↔JIT divergence).
            let value: JsonValue = value.to_json_value(ctx.schemas)?;
            let mut writer = Writer::new(Cursor::new(Vec::new()));
            write_xml_json_node(&mut writer, &value)?;

            let output = String::from_utf8(writer.into_inner().into_inner())
                .map_err(|e| format!("xml.stringify(): invalid UTF-8 output: {}", e))?;

            Ok(TypedReturn::Ok(ConcreteReturn::String(output)))
        },
    );

    module
}
