//! Free-form attributes: key and value pairs that a script attaches to
//! entities and block definitions.
//!
//! The core stores attributes, keeps them through sync, undo, copy, JSON and
//! DWG, and matches them in selections. It never interprets them: a key
//! means whatever the script that wrote it says it means. See
//! `docs/api-surface.md`, "Attributes".

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The attributes of one entity or block definition, sorted by key.
pub type Attrs = BTreeMap<String, AttrValue>;

/// An attribute value: a JSON scalar. Arrays, objects and null are not
/// values (null removes a key in an update).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttrValue {
    Str(String),
    Num(f64),
    Bool(bool),
}

/// At most this many keys per entity or block definition.
pub const MAX_KEYS: usize = 64;
/// A key is 1 to 64 bytes.
pub const MAX_KEY_BYTES: usize = 64;
/// A string value is at most 1024 bytes of UTF-8.
pub const MAX_STR_BYTES: usize = 1024;
/// All keys and values of one entity together: at most 8192 bytes (a number
/// counts 8 bytes, a boolean 1). Keeps sync blobs small and DWG extended
/// data far below its 16 KB limit per entity.
pub const MAX_TOTAL_BYTES: usize = 8192;

impl AttrValue {
    pub fn to_json(&self) -> Value {
        match self {
            AttrValue::Str(s) => Value::String(s.clone()),
            AttrValue::Num(n) => {
                serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number)
            }
            AttrValue::Bool(b) => Value::Bool(*b),
        }
    }

    /// Parse a JSON value. `Ok(None)` for null.
    pub fn from_json(v: &Value) -> Result<Option<AttrValue>, String> {
        match v {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(AttrValue::Str(s.clone()))),
            Value::Bool(b) => Ok(Some(AttrValue::Bool(*b))),
            Value::Number(n) => n
                .as_f64()
                .filter(|x| x.is_finite())
                .map(|x| Some(AttrValue::Num(x)))
                .ok_or_else(|| format!("attribute value {n} is not a finite number")),
            Value::Array(_) | Value::Object(_) => Err(
                "an attribute value is a string, number or boolean (JSON.stringify structured data)"
                    .to_string(),
            ),
        }
    }

    fn byte_len(&self) -> usize {
        match self {
            AttrValue::Str(s) => s.len(),
            AttrValue::Num(_) => 8,
            AttrValue::Bool(_) => 1,
        }
    }
}

/// Keys are 1 to 64 ASCII letters, digits and `_ - . : /`.
pub fn check_key(k: &str) -> Result<(), String> {
    if k.is_empty() || k.len() > MAX_KEY_BYTES {
        return Err(format!(
            "attribute key {k:?} must be 1 to {MAX_KEY_BYTES} characters"
        ));
    }
    if !k
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_-.:/".contains(&b))
    {
        return Err(format!(
            "attribute key {k:?} may only contain ASCII letters, digits and _ - . : /"
        ));
    }
    Ok(())
}

/// Check every limit on a complete attribute set.
pub fn check(attrs: &Attrs) -> Result<(), String> {
    if attrs.len() > MAX_KEYS {
        return Err(format!(
            "at most {MAX_KEYS} attributes per entity (got {})",
            attrs.len()
        ));
    }
    let mut total = 0;
    for (k, v) in attrs {
        check_key(k)?;
        if let AttrValue::Str(s) = v {
            if s.len() > MAX_STR_BYTES {
                return Err(format!(
                    "attribute {k:?}: a string value is at most {MAX_STR_BYTES} bytes (got {})",
                    s.len()
                ));
            }
        }
        if let AttrValue::Num(n) = v {
            if !n.is_finite() {
                return Err(format!("attribute {k:?}: number must be finite"));
            }
        }
        total += k.len() + v.byte_len();
    }
    if total > MAX_TOTAL_BYTES {
        return Err(format!(
            "attributes of one entity are at most {MAX_TOTAL_BYTES} bytes in total (got {total})"
        ));
    }
    Ok(())
}

/// Attributes as a JSON object.
pub fn to_json(attrs: &Attrs) -> Value {
    Value::Object(
        attrs
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect(),
    )
}

/// An update: each key with its new value, or None to remove it.
pub type Updates = Vec<(String, Option<AttrValue>)>;

/// Parse an update object `{key: value | null}`.
pub fn parse_updates(v: &Value) -> Result<Updates, String> {
    let obj = v
        .as_object()
        .ok_or("attributes are an object {key: value}")?;
    obj.iter()
        .map(|(k, v)| {
            check_key(k)?;
            Ok((k.clone(), AttrValue::from_json(v)?))
        })
        .collect()
}

/// Parse a complete attribute set (null values are left out) and check it.
pub fn parse(v: &Value) -> Result<Attrs, String> {
    let mut out = Attrs::new();
    for (k, v) in parse_updates(v)? {
        if let Some(v) = v {
            out.insert(k, v);
        }
    }
    check(&out)?;
    Ok(out)
}

/// Optional `attrs` argument of a creation call.
pub fn parse_opt(v: &Option<Value>) -> Result<Attrs, String> {
    match v {
        None | Some(Value::Null) => Ok(Attrs::new()),
        Some(v) => parse(v),
    }
}

/// For `#[serde(default, deserialize_with = "crate::attrs::deserialize")]`
/// on an `attrs` argument: parses and checks it, null is none.
pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Attrs, D::Error> {
    let v = Value::deserialize(d)?;
    if v.is_null() {
        return Ok(Attrs::new());
    }
    parse(&v).map_err(serde::de::Error::custom)
}

/// Apply an update. With `replace`, start from no attributes. The target
/// is unchanged when the result would break a limit.
pub fn apply(target: &mut Attrs, updates: &Updates, replace: bool) -> Result<(), String> {
    let mut next = if replace {
        Attrs::new()
    } else {
        target.clone()
    };
    for (k, v) in updates {
        match v {
            Some(v) => {
                next.insert(k.clone(), v.clone());
            }
            None => {
                next.remove(k);
            }
        }
    }
    check(&next)?;
    *target = next;
    Ok(())
}

/// The pairs both sets hold with equal values.
pub fn common(a: &Attrs, b: &Attrs) -> Attrs {
    a.iter()
        .filter(|(k, v)| b.get(*k) == Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Layers of attributes merged in order: a later layer wins on equal keys.
pub fn layered(layers: &[&Attrs]) -> Attrs {
    let mut out = Attrs::new();
    for l in layers {
        for (k, v) in l.iter() {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_are_json_scalars() {
        assert_eq!(
            AttrValue::from_json(&json!("GND")).unwrap(),
            Some(AttrValue::Str("GND".into()))
        );
        assert_eq!(
            AttrValue::from_json(&json!(2.5)).unwrap(),
            Some(AttrValue::Num(2.5))
        );
        assert_eq!(
            AttrValue::from_json(&json!(true)).unwrap(),
            Some(AttrValue::Bool(true))
        );
        assert_eq!(AttrValue::from_json(&json!(null)).unwrap(), None);
        assert!(AttrValue::from_json(&json!([1])).is_err());
        assert!(AttrValue::from_json(&json!({"a": 1})).is_err());
    }

    #[test]
    fn keys_and_limits_are_checked() {
        assert!(check_key("net").is_ok());
        assert!(check_key("pcb.net/2:a-b_c").is_ok());
        assert!(check_key("").is_err());
        assert!(check_key("has space").is_err());
        assert!(check_key("ü").is_err());
        assert!(check_key(&"k".repeat(65)).is_err());
        let big = json!({ "a": "x".repeat(MAX_STR_BYTES + 1) });
        assert!(parse(&big).is_err());
        let many: serde_json::Map<String, Value> = (0..=MAX_KEYS)
            .map(|i| (format!("k{i}"), json!(1)))
            .collect();
        assert!(parse(&Value::Object(many)).is_err());
        let heavy: serde_json::Map<String, Value> = (0..9)
            .map(|i| (format!("k{i}"), json!("y".repeat(1000))))
            .collect();
        assert!(parse(&Value::Object(heavy))
            .unwrap_err()
            .contains("in total"));
    }

    #[test]
    fn apply_merges_removes_and_replaces() {
        let mut a = parse(&json!({"net": "GND", "w": 0.2})).unwrap();
        apply(
            &mut a,
            &parse_updates(&json!({"w": null, "layer2": true})).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(to_json(&a), json!({"layer2": true, "net": "GND"}));
        apply(&mut a, &parse_updates(&json!({"x": 1})).unwrap(), true).unwrap();
        assert_eq!(to_json(&a), json!({"x": 1.0}));
        // A failing update leaves the set unchanged.
        let before = a.clone();
        let bad = parse_updates(&json!({"s": "z".repeat(MAX_STR_BYTES + 1)})).unwrap();
        assert!(apply(&mut a, &bad, false).is_err());
        assert_eq!(a, before);
    }

    #[test]
    fn common_and_layered() {
        let a = parse(&json!({"net": "GND", "w": 1})).unwrap();
        let b = parse(&json!({"net": "GND", "w": 2})).unwrap();
        assert_eq!(to_json(&common(&a, &b)), json!({"net": "GND"}));
        assert_eq!(
            to_json(&layered(&[&a, &b])),
            json!({"net": "GND", "w": 2.0})
        );
    }
}
