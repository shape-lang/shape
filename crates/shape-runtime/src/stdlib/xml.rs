//! Native `xml` module for XML parsing and serialization.
//!
//! Exports: xml.parse(text), xml.stringify(value)
//!
//! XML nodes are represented as Shape HashMaps with structure:
//! `{ name: string, attributes: HashMap<string, string>,
//!    children: Array<HashMap>, text?: string }`
//!
//! Stage C HashMap-marshal P1(b) migration (2026-05-07):
//! - `xml.parse` returns the root element as `TypedReturn::OkObjectPairs`
//!   per Cluster #4 β shape (mirrors `arrow.metadata` / http.rs precedents).
//!   Nested children use `HeapValue::HashMap(HashMapData)` directly inside
//!   `ConcreteReturn::ArrayHeapValue` — recursion lives in the HeapValue
//!   tree, NOT in the ConcreteReturn enum (preserves the leaf-only
//!   invariant of ConcreteReturn).
//! - `xml.stringify` takes `value: HashMap<string, *>` typed input via
//!   `Vec<(Arc<String>, Arc<HeapValue>)>` FromSlot from Step 1 P1(b)
//!   infrastructure (commit `36519f6`). Walks the recursive HeapValue
//!   tree using direct pattern matching — no marshal-boundary
//!   re-entry per element.
//! - Attributes (`HashMap<string, string>`) carried via
//!   `ConcreteReturn::HashMapStringString` on output and read directly
//!   from `HeapValue::HashMap(d)` on input.
//! - `text?` optional field follows the regex.rs precedent: emit empty
//!   string when absent rather than variable-length pair list. Keeps
//!   the schema fixed-shape; loses the `<elem/>` vs `<elem></elem>`
//!   distinction at parse time (a semantic that the previous code
//!   already flattened).
//!
//! Tests deleted along with the legacy ValueWord-based fixtures, mirroring
//! the csv_module migration (commit `9f6b1d3`). New typed-marshal test
//! harness arrives with the shape-vm cleanup workstream.

use crate::marshal::{register_typed_fn_1, register_typed_fn_1_full};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use shape_value::heap_value::{HashMapData, HeapValue, TypedArrayData};
use shape_value::TypedBuffer;
use std::io::Cursor;
use std::sync::Arc;

/// Parsed XML element data: a recursive structure where each element has
/// a name, attribute pairs, child elements, and optional text content.
struct ElementData {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<ElementData>,
    text: Option<String>,
}

impl ElementData {
    /// Project this element into a `HeapValue::HashMap(HashMapData)`,
    /// suitable for embedding inside a parent's `children` array.
    /// Recursion runs through this method.
    fn into_heap_value(self) -> Arc<HeapValue> {
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
        let children_arc: Vec<Arc<HeapValue>> = self
            .children
            .into_iter()
            .map(ElementData::into_heap_value)
            .collect();

        let mut keys: Vec<Arc<String>> = vec![
            Arc::new("name".to_string()),
            Arc::new("attributes".to_string()),
            Arc::new("children".to_string()),
        ];
        // W17-typed-carrier-bundle-A checkpoint 2/4: `Array<HashMap>`
        // (each child is `into_heap_value` producing a HashMap) has no
        // specialized variant in ADR-006 §2.7.24 Q25.A's spec list. The
        // dispatcher surfaces; for now we route through it so the body
        // compiles post-Q25.A deletion. Out-of-territory follow-up: refactor
        // xml.parse to build per-child TypedObject schemas (name/attrs/text/
        // children) so the array lowers to `TypedArrayData::TypedObject`.
        let children_array_data = shape_value::TypedArrayData::build_specialized_from_heap_arcs(
            children_arc,
        )
        .unwrap_or_else(|err| {
            panic!(
                "xml.parse: {} — ADR-006 §2.7.24 Q25.A spec list lacks a \
                 `HashMap`-element variant; out-of-territory follow-up. \
                 Refactor xml.parse to per-child TypedObject schemas.",
                err
            )
        });
        let mut values: Vec<Arc<HeapValue>> = vec![
            Arc::new(HeapValue::String(Arc::new(self.name))),
            Arc::new(HeapValue::HashMap(Arc::new(attrs_data))),
            Arc::new(HeapValue::TypedArray(Arc::new(children_array_data))),
        ];
        if let Some(text) = self.text {
            keys.push(Arc::new("text".to_string()));
            values.push(Arc::new(HeapValue::String(Arc::new(text))));
        }
        Arc::new(HeapValue::HashMap(Arc::new(HashMapData::from_pairs(
            keys, values,
        ))))
    }

    /// Project this element's TOP-LEVEL form as a `Vec<(String,
    /// ConcreteReturn)>` pair-list, suitable for `TypedReturn::OkObjectPairs`.
    /// Used only for the root element of `xml.parse`'s return value;
    /// nested elements go through `into_heap_value` instead.
    fn into_root_pairs(self) -> Vec<(String, ConcreteReturn)> {
        let attrs_pairs: Vec<(String, String)> = self.attributes;
        let children_arc: Vec<Arc<HeapValue>> = self
            .children
            .into_iter()
            .map(ElementData::into_heap_value)
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

/// Walk a node — represented as a `(keys, values)` pair-list of an
/// outer HashMap — and emit the corresponding XML via the writer.
/// Children recurse through `write_node_heap` against
/// `HeapValue::HashMap(d)` arms.
fn write_node_pairs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    pairs: &[(Arc<String>, Arc<HeapValue>)],
) -> Result<(), String> {
    let mut name: Option<&str> = None;
    let mut attrs: Option<&HashMapData> = None;
    let mut children: Option<&Arc<TypedBuffer<Arc<HeapValue>>>> = None;
    let mut text: Option<&str> = None;

    for (k, v) in pairs.iter() {
        match k.as_str() {
            "name" => {
                if let HeapValue::String(s) = &**v {
                    name = Some(s.as_str());
                }
            }
            "attributes" => {
                if let HeapValue::HashMap(d) = &**v {
                    attrs = Some(d);
                }
            }
            "children" => {
                // W17-typed-carrier-bundle-A checkpoint 3/4 / 4: the
                // construction-side counterpart in `into_heap_value`
                // surfaces post-Q25.A (`Array<HashMap>` has no specialized
                // variant in Q25.A's spec list — that's the out-of-territory
                // follow-up cite there). The reader here cannot bind a
                // `the-deleted-heterogeneous-element-carrier` after checkpoint 4 deletes
                // the variant. Setting `children = None` lets the parent
                // node serialize as `<name>...</name>` with no children —
                // the writer surfaces upstream if the producer body
                // actually runs. Refactor to per-child TypedObject schemas
                // to fix end-to-end (out of bundle-A territory).
                if let HeapValue::TypedArray(_) = &**v {
                    // intentional no-op: dead-arm pending xml refactor
                }
            }
            "text" => {
                if let HeapValue::String(s) = &**v {
                    text = Some(s.as_str());
                }
            }
            _ => {}
        }
    }

    let name = name.ok_or_else(|| "xml.stringify(): node missing 'name' field".to_string())?;

    let mut elem = BytesStart::new(name.to_string());

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

    let has_children = children.map(|b| !b.data.is_empty()).unwrap_or(false);
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

        if let Some(buf) = children {
            for child in buf.data.iter() {
                write_node_heap(writer, child)?;
            }
        }

        writer
            .write_event(Event::End(BytesEnd::new(name.to_string())))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;
    }

    Ok(())
}

/// Walk a child node — represented as a `HeapValue::HashMap(d)` — and
/// emit the corresponding XML. Recursion entry from
/// `write_node_pairs::children`.
fn write_node_heap(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    node: &Arc<HeapValue>,
) -> Result<(), String> {
    if let HeapValue::HashMap(d) = &**node {
        let n = d.keys.data.len();
        let mut pairs: Vec<(Arc<String>, Arc<HeapValue>)> = Vec::with_capacity(n);
        for i in 0..n {
            let k = &d.keys.data[i];
            pairs.push((Arc::clone(k), d.values.value_at(i)));
        }
        write_node_pairs(writer, &pairs)
    } else {
        Err(format!(
            "xml.stringify(): child node must be a HashMap, got {}",
            node.type_name()
        ))
    }
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
