//! XML Parser for SWTOR Game Objects
//!
//! Extracted GOM data is XML with structure like:
//!   <Quest GUID="..." fqn="qst.class.warrior.act1...." Version="1" Revision="42">
//!     <NameList>...</NameList>
//!     <ObjectiveList>...</ObjectiveList>
//!     ...
//!   </Quest>
//!
//! We extract: GUID, fqn, Kind (root element name), and the full JSON representation

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Map, Value};

use crate::schema::GameObject;

/// Parse XML data into a GameObject
pub fn parse(data: &[u8]) -> Result<GameObject> {
    let mut reader = Reader::from_reader(data);
    reader.trim_text(true);

    let mut buf = Vec::new();
    let mut obj = GameObject::default();

    // Find root element and extract attributes
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) => {
                obj.kind = String::from_utf8_lossy(e.name().as_ref()).to_string();

                // Extract standard attributes
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let value = String::from_utf8_lossy(&attr.value).to_string();

                    match key.as_str() {
                        "GUID" => obj.guid = value,
                        "fqn" | "Id" => {
                            if obj.fqn.is_empty() {
                                obj.fqn = value;
                            }
                        }
                        "Version" => obj.version = value.parse().unwrap_or(0),
                        "Revision" => obj.revision = value.parse().unwrap_or(0),
                        _ => {}
                    }
                }

                // Parse full document to JSON
                obj.json = xml_to_json(data)?;
                break;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(obj)
}

/// Convert XML to JSON representation
fn xml_to_json(data: &[u8]) -> Result<Value> {
    let mut reader = Reader::from_reader(data);
    reader.trim_text(true);

    let mut stack: Vec<(String, Map<String, Value>)> = Vec::new();
    let mut buf = Vec::new();
    let mut current_text = String::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let mut attrs = Map::new();

                // Store attributes under "^" key (matching bespin convention)
                let mut attr_map = Map::new();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let value = String::from_utf8_lossy(&attr.value).to_string();
                    attr_map.insert(key, Value::String(value));
                }
                if !attr_map.is_empty() {
                    attrs.insert("^".to_string(), Value::Object(attr_map));
                }

                stack.push((name, attrs));
                current_text.clear();
            }

            Event::End(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                if let Some((elem_name, mut elem_data)) = stack.pop() {
                    // Add text content under "%" key if present
                    if !current_text.trim().is_empty() {
                        elem_data.insert(
                            "%".to_string(),
                            Value::String(current_text.trim().to_string()),
                        );
                    }

                    let elem_value = Value::Object(elem_data);

                    if let Some((_, parent_data)) = stack.last_mut() {
                        // Check if this is a list element (ends with "List")
                        if elem_name.ends_with("List") || parent_data.contains_key(&elem_name) {
                            // Handle as array
                            if let Some(existing) = parent_data.get_mut(&elem_name) {
                                if let Value::Array(arr) = existing {
                                    arr.push(elem_value);
                                }
                            } else {
                                parent_data.insert(elem_name, Value::Array(vec![elem_value]));
                            }
                        } else {
                            parent_data.insert(elem_name, elem_value);
                        }
                    } else {
                        // This is the root element
                        return Ok(json!({ name: elem_value }));
                    }
                }
                current_text.clear();
            }

            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let mut attrs = Map::new();

                let mut attr_map = Map::new();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let value = String::from_utf8_lossy(&attr.value).to_string();
                    attr_map.insert(key, Value::String(value));
                }
                if !attr_map.is_empty() {
                    attrs.insert("^".to_string(), Value::Object(attr_map));
                }

                let elem_value = Value::Object(attrs);

                if let Some((_, parent_data)) = stack.last_mut() {
                    if name.ends_with("List") || parent_data.contains_key(&name) {
                        if let Some(existing) = parent_data.get_mut(&name) {
                            if let Value::Array(arr) = existing {
                                arr.push(elem_value);
                            }
                        } else {
                            parent_data.insert(name, Value::Array(vec![elem_value]));
                        }
                    } else {
                        parent_data.insert(name, elem_value);
                    }
                }
            }

            Event::Text(e) => {
                current_text.push_str(&e.unescape().context("Failed to unescape text")?);
            }

            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(Value::Null)
}
