use postcard_schema::schema::owned::{OwnedDataModelType, OwnedDataModelVariant, OwnedNamedType};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("type mismatch at {path}: expected {expected}, got {got}")]
    TypeMismatch {
        expected: String,
        got: String,
        path: String,
    },
    #[error("value out of range at {path}: {ty} cannot hold {value}")]
    OutOfRange {
        ty: &'static str,
        value: String,
        path: String,
    },
    #[error("missing required field '{name}' at {path}")]
    MissingField { name: String, path: String },
    #[error("unknown enum variant '{variant}' at {path}")]
    UnknownEnumVariant { variant: String, path: String },
    #[error("arity mismatch at {path}: expected {expected}, got {got}")]
    ArityMismatch {
        expected: usize,
        got: usize,
        path: String,
    },
    #[error("postcard error: {0}")]
    Postcard(String),
    #[error("schema depth exceeded at {path}")]
    DepthExceeded { path: String },
}

const MAX_DEPTH: usize = 8;

fn ext<T: serde::Serialize>(val: &T, out: &mut Vec<u8>) -> Result<(), ConvertError> {
    let tmp =
        postcard::to_allocvec(val).map_err(|e| ConvertError::Postcard(e.to_string()))?;
    out.extend_from_slice(&tmp);
    Ok(())
}

pub fn json_to_postcard(schema: &OwnedNamedType, json: &Value) -> Result<Vec<u8>, ConvertError> {
    let mut out = Vec::new();
    encode_value(&schema.ty, json, &mut out, "$", 0)?;
    Ok(out)
}

fn encode_value(
    ty: &OwnedDataModelType,
    v: &Value,
    out: &mut Vec<u8>,
    path: &str,
    depth: usize,
) -> Result<(), ConvertError> {
    if depth >= MAX_DEPTH {
        return Err(ConvertError::DepthExceeded {
            path: path.to_string(),
        });
    }

    use OwnedDataModelType::*;

    match ty {
        Bool => {
            let b = v.as_bool().ok_or_else(|| ConvertError::TypeMismatch {
                expected: "boolean".into(),
                got: json_kind(v),
                path: path.to_string(),
            })?;
            ext(&b, out)?;
        }
        U8 => {
            let n = require_u64(v, path)?;
            let val = u8::try_from(n).map_err(|_| ConvertError::OutOfRange {
                ty: "u8",
                value: n.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        U16 => {
            let n = require_u64(v, path)?;
            let val = u16::try_from(n).map_err(|_| ConvertError::OutOfRange {
                ty: "u16",
                value: n.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        U32 => {
            let n = require_u64(v, path)?;
            let val = u32::try_from(n).map_err(|_| ConvertError::OutOfRange {
                ty: "u32",
                value: n.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        U64 => {
            let n = require_u64(v, path)?;
            ext(&n, out)?;
        }
        U128 => {
            let s = require_str(v, path)?;
            let val: u128 = s.parse().map_err(|_| ConvertError::TypeMismatch {
                expected: "decimal string for u128".into(),
                got: s.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        Usize => {
            let n = require_u64(v, path)?;
            ext(&(n as usize), out)?;
        }
        I8 => {
            let n = require_i64(v, path)?;
            let val = i8::try_from(n).map_err(|_| ConvertError::OutOfRange {
                ty: "i8",
                value: n.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        I16 => {
            let n = require_i64(v, path)?;
            let val = i16::try_from(n).map_err(|_| ConvertError::OutOfRange {
                ty: "i16",
                value: n.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        I32 => {
            let n = require_i64(v, path)?;
            let val = i32::try_from(n).map_err(|_| ConvertError::OutOfRange {
                ty: "i32",
                value: n.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        I64 => {
            let n = require_i64(v, path)?;
            ext(&n, out)?;
        }
        I128 => {
            let s = require_str(v, path)?;
            let val: i128 = s.parse().map_err(|_| ConvertError::TypeMismatch {
                expected: "decimal string for i128".into(),
                got: s.to_string(),
                path: path.to_string(),
            })?;
            ext(&val, out)?;
        }
        Isize => {
            let n = require_i64(v, path)?;
            ext(&(n as isize), out)?;
        }
        F32 => {
            let f = require_f64(v, path)? as f32;
            ext(&f, out)?;
        }
        F64 => {
            let f = require_f64(v, path)?;
            ext(&f, out)?;
        }
        Char => {
            let s = require_str(v, path)?;
            let mut chars = s.chars();
            let c = chars.next().ok_or_else(|| ConvertError::TypeMismatch {
                expected: "single-char string".into(),
                got: "empty string".into(),
                path: path.to_string(),
            })?;
            if chars.next().is_some() {
                return Err(ConvertError::TypeMismatch {
                    expected: "single-char string".into(),
                    got: format!("string of length {}", s.len()),
                    path: path.to_string(),
                });
            }
            ext(&c, out)?;
        }
        String => {
            let s = require_str(v, path)?;
            ext(&s, out)?;
        }
        ByteArray => {
            let arr = require_array(v, path)?;
            let mut bytes: Vec<u8> = Vec::with_capacity(arr.len());
            for (i, item) in arr.iter().enumerate() {
                let n = require_u64(item, &format!("{path}[{i}]"))?;
                let b = u8::try_from(n).map_err(|_| ConvertError::OutOfRange {
                    ty: "u8",
                    value: n.to_string(),
                    path: format!("{path}[{i}]"),
                })?;
                bytes.push(b);
            }
            ext(&bytes, out)?;
        }
        Option(inner) => {
            if v.is_null() {
                out.push(0x00);
            } else {
                out.push(0x01);
                encode_value(&inner.ty, v, out, path, depth + 1)?;
            }
        }
        Unit | UnitStruct => {
            // postcard emits nothing for ()
        }
        NewtypeStruct(inner) => {
            encode_value(&inner.ty, v, out, path, depth + 1)?;
        }
        Seq(inner) => {
            let arr = require_array(v, path)?;
            ext(&(arr.len() as u64), out)?;
            for (i, item) in arr.iter().enumerate() {
                encode_value(&inner.ty, item, out, &format!("{path}[{i}]"), depth + 1)?;
            }
        }
        Tuple(elems) | TupleStruct(elems) => {
            // elems: Vec<OwnedNamedType>; sch.ty is OwnedDataModelType
            let arr = require_array(v, path)?;
            if arr.len() != elems.len() {
                return Err(ConvertError::ArityMismatch {
                    expected: elems.len(),
                    got: arr.len(),
                    path: path.to_string(),
                });
            }
            for (i, (sch, item)) in elems.iter().zip(arr.iter()).enumerate() {
                encode_value(&sch.ty, item, out, &format!("{path}.{i}"), depth + 1)?;
            }
        }
        Map { key, val } => {
            // key, val: Box<OwnedNamedType>; .ty is OwnedDataModelType
            if matches!(&key.ty, OwnedDataModelType::String) {
                let obj = require_object(v, path)?;
                ext(&(obj.len() as u64), out)?;
                for (k, item) in obj {
                    ext(&k.as_str(), out)?;
                    encode_value(&val.ty, item, out, &format!("{path}.{k}"), depth + 1)?;
                }
            } else {
                let arr = require_array(v, path)?;
                ext(&(arr.len() as u64), out)?;
                for (i, pair) in arr.iter().enumerate() {
                    let pair_arr = require_array(pair, &format!("{path}[{i}]"))?;
                    if pair_arr.len() != 2 {
                        return Err(ConvertError::ArityMismatch {
                            expected: 2,
                            got: pair_arr.len(),
                            path: format!("{path}[{i}]"),
                        });
                    }
                    encode_value(
                        &key.ty,
                        &pair_arr[0],
                        out,
                        &format!("{path}[{i}].key"),
                        depth + 1,
                    )?;
                    encode_value(
                        &val.ty,
                        &pair_arr[1],
                        out,
                        &format!("{path}[{i}].val"),
                        depth + 1,
                    )?;
                }
            }
        }
        Struct(fields) => {
            // fields: Vec<OwnedNamedValue>; f.ty is OwnedNamedType; f.ty.ty is OwnedDataModelType
            let obj = v.as_object();
            for f in fields {
                let item = if let Some(obj) = obj {
                    obj.get(&*f.name).ok_or_else(|| ConvertError::MissingField {
                        name: f.name.to_string(),
                        path: path.to_string(),
                    })?
                } else {
                    return Err(ConvertError::TypeMismatch {
                        expected: "object".into(),
                        got: json_kind(v),
                        path: path.to_string(),
                    });
                };
                encode_value(&f.ty.ty, item, out, &format!("{path}.{}", f.name), depth + 1)?;
            }
        }
        Enum(variants) => {
            if let Some(s) = v.as_str() {
                let (idx, variant) = variants
                    .iter()
                    .enumerate()
                    .find(|(_, var)| var.name.as_str() == s)
                    .ok_or_else(|| ConvertError::UnknownEnumVariant {
                        variant: s.to_string(),
                        path: path.to_string(),
                    })?;
                if !matches!(variant.ty, OwnedDataModelVariant::UnitVariant) {
                    return Err(ConvertError::TypeMismatch {
                        expected: format!(
                            "object with key '{s}' and payload (string is only valid for unit variants)"
                        ),
                        got: "string".into(),
                        path: path.to_string(),
                    });
                }
                ext(&(idx as u32), out)?;
            } else if let Some(obj) = v.as_object() {
                if obj.len() != 1 {
                    return Err(ConvertError::TypeMismatch {
                        expected: "object with exactly one key (enum variant)".into(),
                        got: format!("object with {} keys", obj.len()),
                        path: path.to_string(),
                    });
                }
                let (variant_name, payload) = obj.iter().next().unwrap();
                let (idx, variant) = variants
                    .iter()
                    .enumerate()
                    .find(|(_, var)| var.name.as_str() == variant_name.as_str())
                    .ok_or_else(|| ConvertError::UnknownEnumVariant {
                        variant: variant_name.to_string(),
                        path: path.to_string(),
                    })?;
                ext(&(idx as u32), out)?;
                encode_variant_payload(&variant.ty, payload, out, path, depth + 1)?;
            } else {
                return Err(ConvertError::TypeMismatch {
                    expected: "string or object for enum".into(),
                    got: json_kind(v),
                    path: path.to_string(),
                });
            }
        }
        Schema => {
            // Should not appear in args/ret in practice
        }
    }
    Ok(())
}

fn encode_variant_payload(
    ty: &OwnedDataModelVariant,
    v: &Value,
    out: &mut Vec<u8>,
    path: &str,
    depth: usize,
) -> Result<(), ConvertError> {
    match ty {
        OwnedDataModelVariant::UnitVariant => {}
        OwnedDataModelVariant::NewtypeVariant(inner) => {
            encode_value(&inner.ty, v, out, path, depth)?;
        }
        OwnedDataModelVariant::TupleVariant(elems) => {
            let arr = require_array(v, path)?;
            if arr.len() != elems.len() {
                return Err(ConvertError::ArityMismatch {
                    expected: elems.len(),
                    got: arr.len(),
                    path: path.to_string(),
                });
            }
            for (i, (sch, item)) in elems.iter().zip(arr.iter()).enumerate() {
                encode_value(&sch.ty, item, out, &format!("{path}.{i}"), depth + 1)?;
            }
        }
        OwnedDataModelVariant::StructVariant(fields) => {
            let obj = require_object(v, path)?;
            for f in fields {
                let item = obj.get(&*f.name).ok_or_else(|| {
                    ConvertError::MissingField {
                        name: f.name.to_string(),
                        path: path.to_string(),
                    }
                })?;
                encode_value(&f.ty.ty, item, out, &format!("{path}.{}", f.name), depth + 1)?;
            }
        }
    }
    Ok(())
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn require_u64(v: &Value, path: &str) -> Result<u64, ConvertError> {
    v.as_u64().ok_or_else(|| ConvertError::TypeMismatch {
        expected: "non-negative integer".into(),
        got: json_kind(v),
        path: path.to_string(),
    })
}

fn require_i64(v: &Value, path: &str) -> Result<i64, ConvertError> {
    v.as_i64().ok_or_else(|| ConvertError::TypeMismatch {
        expected: "integer".into(),
        got: json_kind(v),
        path: path.to_string(),
    })
}

fn require_f64(v: &Value, path: &str) -> Result<f64, ConvertError> {
    v.as_f64().ok_or_else(|| ConvertError::TypeMismatch {
        expected: "number".into(),
        got: json_kind(v),
        path: path.to_string(),
    })
}

fn require_str<'a>(v: &'a Value, path: &str) -> Result<&'a str, ConvertError> {
    v.as_str().ok_or_else(|| ConvertError::TypeMismatch {
        expected: "string".into(),
        got: json_kind(v),
        path: path.to_string(),
    })
}

fn require_array<'a>(v: &'a Value, path: &str) -> Result<&'a Vec<Value>, ConvertError> {
    v.as_array().ok_or_else(|| ConvertError::TypeMismatch {
        expected: "array".into(),
        got: json_kind(v),
        path: path.to_string(),
    })
}

fn require_object<'a>(
    v: &'a Value,
    path: &str,
) -> Result<&'a serde_json::Map<String, Value>, ConvertError> {
    v.as_object().ok_or_else(|| ConvertError::TypeMismatch {
        expected: "object".into(),
        got: json_kind(v),
        path: path.to_string(),
    })
}

fn json_kind(v: &Value) -> String {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
    .to_string()
}
