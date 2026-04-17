//! Native `xml` module for XML parsing and serialization.
//!
//! Exports: xml.parse(text), xml.stringify(value)
//!
//! XML nodes are represented as Shape HashMaps with structure:
//! `{ name: string, attributes: HashMap, children: Array, text?: string }`

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use shape_value::{ValueWord, ValueWordExt};
use std::io::Cursor;
use std::sync::Arc;

/// Parse an XML element recursively from a quick-xml reader.
/// Returns a ValueWord HashMap: { name, attributes, children, text? }
fn parse_element(reader: &mut Reader<&[u8]>, start: &BytesStart) -> Result<ValueWord, String> {
    let name = std::str::from_utf8(start.name().as_ref())
        .map_err(|e| format!("Invalid UTF-8 in element name: {}", e))?
        .to_string();

    // Parse attributes
    let mut attr_keys = Vec::new();
    let mut attr_values = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("Invalid attribute: {}", e))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|e| format!("Invalid UTF-8 in attribute key: {}", e))?
            .to_string();
        let value = attr
            .unescape_value()
            .map_err(|e| format!("Invalid attribute value: {}", e))?
            .to_string();
        attr_keys.push(ValueWord::from_string(Arc::new(key)));
        attr_values.push(ValueWord::from_string(Arc::new(value)));
    }
    let attributes = ValueWord::from_hashmap_pairs(attr_keys, attr_values);

    // Parse children and text
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
                // Self-closing element — treated as element with no children
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

    // Build the node HashMap: { name, attributes, children, text? }
    let mut node_keys = vec![
        ValueWord::from_string(Arc::new("name".to_string())),
        ValueWord::from_string(Arc::new("attributes".to_string())),
        ValueWord::from_string(Arc::new("children".to_string())),
    ];
    let mut node_values = vec![
        ValueWord::from_string(Arc::new(name)),
        attributes,
        ValueWord::from_array(shape_value::vmarray_from_vec(children)),
    ];

    if !text_parts.is_empty() {
        node_keys.push(ValueWord::from_string(Arc::new("text".to_string())));
        node_values.push(ValueWord::from_string(Arc::new(text_parts.join(""))));
    }

    Ok(ValueWord::from_hashmap_pairs(node_keys, node_values))
}

/// Parse a self-closing XML element (e.g. `<br/>`).
fn parse_empty_element(start: &BytesStart) -> Result<ValueWord, String> {
    let name = std::str::from_utf8(start.name().as_ref())
        .map_err(|e| format!("Invalid UTF-8 in element name: {}", e))?
        .to_string();

    let mut attr_keys = Vec::new();
    let mut attr_values = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("Invalid attribute: {}", e))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|e| format!("Invalid UTF-8 in attribute key: {}", e))?
            .to_string();
        let value = attr
            .unescape_value()
            .map_err(|e| format!("Invalid attribute value: {}", e))?
            .to_string();
        attr_keys.push(ValueWord::from_string(Arc::new(key)));
        attr_values.push(ValueWord::from_string(Arc::new(value)));
    }
    let attributes = ValueWord::from_hashmap_pairs(attr_keys, attr_values);

    let node_keys = vec![
        ValueWord::from_string(Arc::new("name".to_string())),
        ValueWord::from_string(Arc::new("attributes".to_string())),
        ValueWord::from_string(Arc::new("children".to_string())),
    ];
    let node_values = vec![
        ValueWord::from_string(Arc::new(name)),
        attributes,
        ValueWord::from_array(shape_value::vmarray_from_vec(Vec::new())),
    ];

    Ok(ValueWord::from_hashmap_pairs(node_keys, node_values))
}

/// Write a Shape node HashMap to XML using quick-xml Writer.
fn write_node(writer: &mut Writer<Cursor<Vec<u8>>>, node: &ValueWord) -> Result<(), String> {
    let (keys, values, _) = node
        .as_hashmap()
        .ok_or_else(|| "xml.stringify(): node must be a HashMap".to_string())?;

    // Extract fields by key
    let mut name_val = None;
    let mut attrs_val = None;
    let mut children_val = None;
    let mut text_val = None;

    for (k, v) in keys.iter().zip(values.iter()) {
        match k.as_str() {
            Some("name") => name_val = Some(v),
            Some("attributes") => attrs_val = Some(v),
            Some("children") => children_val = Some(v),
            Some("text") => text_val = Some(v),
            _ => {}
        }
    }

    let name = name_val
        .and_then(|v| v.as_str())
        .ok_or_else(|| "xml.stringify(): node missing 'name' field".to_string())?;

    let mut elem = BytesStart::new(name.to_string());

    // Add attributes
    if let Some(attrs) = attrs_val {
        if let Some((attr_keys, attr_values, _)) = attrs.as_hashmap() {
            for (ak, av) in attr_keys.iter().zip(attr_values.iter()) {
                if let (Some(key), Some(val)) = (ak.as_str(), av.as_str()) {
                    elem.push_attribute((key, val));
                }
            }
        }
    }

    // Check if there are children or text
    let has_children = children_val
        .and_then(|v| v.as_any_array())
        .map(|a| !a.to_generic().is_empty())
        .unwrap_or(false);
    let has_text = text_val.and_then(|v| v.as_str()).is_some();

    if !has_children && !has_text {
        // Self-closing
        writer
            .write_event(Event::Empty(elem))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;
    } else {
        writer
            .write_event(Event::Start(elem.clone()))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;

        // Write text
        if let Some(text) = text_val.and_then(|v| v.as_str()) {
            writer
                .write_event(Event::Text(BytesText::new(text)))
                .map_err(|e| format!("xml.stringify() write error: {}", e))?;
        }

        // Write children
        if let Some(children) = children_val {
            if let Some(arr) = children.as_any_array() {
                for child in arr.to_generic().iter() {
                    write_node(writer, child)?;
                }
            }
        }

        writer
            .write_event(Event::End(BytesEnd::new(name.to_string())))
            .map_err(|e| format!("xml.stringify() write error: {}", e))?;
    }

    Ok(())
}

/// Create the `xml` module with XML parsing and serialization functions.
pub fn create_xml_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::xml");
    module.description = "XML parsing and serialization".to_string();

    // xml.parse(text: string) -> Result<HashMap>
    module.add_function_with_schema(
        "parse",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "xml.parse() requires a string argument".to_string())?;

            let mut reader = Reader::from_str(text);
            reader.config_mut().trim_text(true);
            let mut buf = Vec::new();

            // Find the root element
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref e)) => {
                        let result = parse_element(&mut reader, e)?;
                        return Ok(ValueWord::from_ok(result));
                    }
                    Ok(Event::Empty(ref e)) => {
                        let result = parse_empty_element(e)?;
                        return Ok(ValueWord::from_ok(result));
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
        ModuleFunction {
            description: "Parse an XML string into a Shape HashMap node".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "XML string to parse".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<HashMap>".to_string()),
        },
    );

    // xml.stringify(value: HashMap) -> Result<string>
    module.add_function_with_schema(
        "stringify",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let value = args
                .first()
                .ok_or_else(|| "xml.stringify() requires a value argument".to_string())?;

            let mut writer = Writer::new(Cursor::new(Vec::new()));
            write_node(&mut writer, value)?;

            let output = String::from_utf8(writer.into_inner().into_inner())
                .map_err(|e| format!("xml.stringify(): invalid UTF-8 output: {}", e))?;

            Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(output))))
        },
        ModuleFunction {
            description: "Serialize a Shape HashMap node to an XML string".to_string(),
            params: vec![ModuleParam {
                name: "value".to_string(),
                type_name: "HashMap".to_string(),
                required: true,
                description:
                    "Node value to serialize (with name, attributes, children, text? fields)"
                        .to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<string>".to_string()),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    #[test]
    fn test_xml_module_creation() {
        let module = create_xml_module();
        assert_eq!(module.name, "std::core::xml");
        assert!(module.has_export("parse"));
        assert!(module.has_export("stringify"));
    }

    #[test]
    fn test_xml_parse_simple() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input =
            ValueWord::from_string(Arc::new("<root><child>hello</child></root>".to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");
        // Find the "name" field
        let mut found_name = false;
        for (k, v) in keys.iter().zip(values.iter()) {
            if k.as_str() == Some("name") {
                assert_eq!(v.as_str(), Some("root"));
                found_name = true;
            }
        }
        assert!(found_name, "should have a 'name' field");
    }

    #[test]
    fn test_xml_parse_with_attributes() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            r#"<person name="Alice" age="30">text</person>"#.to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");

        // Find attributes
        for (k, v) in keys.iter().zip(values.iter()) {
            if k.as_str() == Some("attributes") {
                let (attr_keys, _attr_values, _) = v.as_hashmap().expect("attrs should be hashmap");
                assert_eq!(attr_keys.len(), 2);
            }
            if k.as_str() == Some("text") {
                assert_eq!(v.as_str(), Some("text"));
            }
        }
    }

    #[test]
    fn test_xml_parse_nested() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            "<config><db><host>localhost</host><port>5432</port></db></config>".to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");

        // Find children
        for (k, v) in keys.iter().zip(values.iter()) {
            if k.as_str() == Some("children") {
                let arr = v.as_any_array().expect("should be array").to_generic();
                assert_eq!(arr.len(), 1); // <db>
            }
        }
    }

    #[test]
    fn test_xml_parse_self_closing() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(r#"<br class="spacer"/>"#.to_string()));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");

        let mut found_name = false;
        for (k, v) in keys.iter().zip(values.iter()) {
            if k.as_str() == Some("name") {
                assert_eq!(v.as_str(), Some("br"));
                found_name = true;
            }
        }
        assert!(found_name);
    }

    #[test]
    fn test_xml_parse_no_root() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("".to_string()));
        let result = parse_fn(&[input], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_xml_parse_requires_string() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let result = parse_fn(&[ValueWord::from_f64(42.0)], &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_xml_stringify_simple() {
        let module = create_xml_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        // Build a node: { name: "root", attributes: {}, children: [], text: "hello" }
        let node_keys = vec![
            ValueWord::from_string(Arc::new("name".to_string())),
            ValueWord::from_string(Arc::new("attributes".to_string())),
            ValueWord::from_string(Arc::new("children".to_string())),
            ValueWord::from_string(Arc::new("text".to_string())),
        ];
        let node_values = vec![
            ValueWord::from_string(Arc::new("root".to_string())),
            ValueWord::from_hashmap_pairs(vec![], vec![]),
            ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
            ValueWord::from_string(Arc::new("hello".to_string())),
        ];
        let node = ValueWord::from_hashmap_pairs(node_keys, node_values);

        let result = stringify_fn(&[node], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let s = inner.as_str().expect("should be string");
        assert!(s.contains("<root>"));
        assert!(s.contains("hello"));
        assert!(s.contains("</root>"));
    }

    #[test]
    fn test_xml_stringify_with_attributes() {
        let module = create_xml_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let attr_keys = vec![ValueWord::from_string(Arc::new("id".to_string()))];
        let attr_values = vec![ValueWord::from_string(Arc::new("42".to_string()))];
        let attrs = ValueWord::from_hashmap_pairs(attr_keys, attr_values);

        let node_keys = vec![
            ValueWord::from_string(Arc::new("name".to_string())),
            ValueWord::from_string(Arc::new("attributes".to_string())),
            ValueWord::from_string(Arc::new("children".to_string())),
        ];
        let node_values = vec![
            ValueWord::from_string(Arc::new("item".to_string())),
            attrs,
            ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
        ];
        let node = ValueWord::from_hashmap_pairs(node_keys, node_values);

        let result = stringify_fn(&[node], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let s = inner.as_str().expect("should be string");
        assert!(s.contains("id=\"42\""));
    }

    #[test]
    fn test_xml_stringify_self_closing() {
        let module = create_xml_module();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let node_keys = vec![
            ValueWord::from_string(Arc::new("name".to_string())),
            ValueWord::from_string(Arc::new("attributes".to_string())),
            ValueWord::from_string(Arc::new("children".to_string())),
        ];
        let node_values = vec![
            ValueWord::from_string(Arc::new("br".to_string())),
            ValueWord::from_hashmap_pairs(vec![], vec![]),
            ValueWord::from_array(shape_value::vmarray_from_vec(vec![])),
        ];
        let node = ValueWord::from_hashmap_pairs(node_keys, node_values);

        let result = stringify_fn(&[node], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let s = inner.as_str().expect("should be string");
        assert!(s.contains("<br/>") || s.contains("<br />"));
    }

    #[test]
    fn test_xml_roundtrip() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let stringify_fn = module.get_export("stringify").unwrap();
        let ctx = test_ctx();

        let xml_str = r#"<root><child attr="val">text</child></root>"#;
        let parsed = parse_fn(
            &[ValueWord::from_string(Arc::new(xml_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = parsed.as_ok_inner().expect("should be Ok");
        let re_stringified = stringify_fn(&[inner.clone()], &ctx).unwrap();
        let re_str = re_stringified.as_ok_inner().expect("should be Ok");
        let s = re_str.as_str().expect("should be string");
        assert!(s.contains("root"));
        assert!(s.contains("child"));
        assert!(s.contains("text"));
    }

    #[test]
    fn test_xml_schemas() {
        let module = create_xml_module();

        let parse_schema = module.get_schema("parse").unwrap();
        assert_eq!(parse_schema.params.len(), 1);
        assert_eq!(parse_schema.params[0].name, "text");
        assert!(parse_schema.params[0].required);
        assert_eq!(parse_schema.return_type.as_deref(), Some("Result<HashMap>"));

        let stringify_schema = module.get_schema("stringify").unwrap();
        assert_eq!(stringify_schema.params.len(), 1);
        assert!(stringify_schema.params[0].required);
    }

    #[test]
    fn test_xml_parse_with_declaration() {
        let module = create_xml_module();
        let parse_fn = module.get_export("parse").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new(
            r#"<?xml version="1.0" encoding="UTF-8"?><root>hello</root>"#.to_string(),
        ));
        let result = parse_fn(&[input], &ctx).unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");
        let mut found_name = false;
        for (k, v) in keys.iter().zip(values.iter()) {
            if k.as_str() == Some("name") {
                assert_eq!(v.as_str(), Some("root"));
                found_name = true;
            }
        }
        assert!(found_name);
    }
}
