use crate::json_to_postcard::ConvertError;
use postcard_schema::schema::owned::{OwnedDataModelType, OwnedDataModelVariant, OwnedNamedType};
use serde_json::{json, Map as JsonMap, Value};

const MAX_DEPTH: usize = 8;

pub fn postcard_to_json(schema: &OwnedNamedType, bytes: &[u8]) -> Result<Value, ConvertError> {
    let (val, remaining) = decode_value(&schema.ty, bytes, "$", 0)?;
    if !remaining.is_empty() {
        return Err(ConvertError::Postcard(format!(
            "{} trailing bytes after decoding root value",
            remaining.len()
        )));
    }
    Ok(val)
}

fn decode_value<'a>(
    ty: &OwnedDataModelType,
    bytes: &'a [u8],
    path: &str,
    depth: usize,
) -> Result<(Value, &'a [u8]), ConvertError> {
    if depth >= MAX_DEPTH {
        return Err(ConvertError::DepthExceeded {
            path: path.to_string(),
        });
    }

    fn de<'b, T: serde::de::DeserializeOwned>(
        bytes: &'b [u8],
        path: &str,
    ) -> Result<(T, &'b [u8]), ConvertError> {
        postcard::take_from_bytes::<T>(bytes)
            .map_err(|e| ConvertError::Postcard(format!("{e} at {path}")))
    }

    use OwnedDataModelType::*;

    match ty {
        Bool => {
            let (v, rest) = de::<bool>(bytes, path)?;
            Ok((Value::Bool(v), rest))
        }
        U8 => {
            let (v, rest) = de::<u8>(bytes, path)?;
            Ok((json!(v), rest))
        }
        U16 => {
            let (v, rest) = de::<u16>(bytes, path)?;
            Ok((json!(v), rest))
        }
        U32 => {
            let (v, rest) = de::<u32>(bytes, path)?;
            Ok((json!(v), rest))
        }
        U64 => {
            let (v, rest) = de::<u64>(bytes, path)?;
            Ok((json!(v), rest))
        }
        U128 => {
            let (v, rest) = de::<u128>(bytes, path)?;
            Ok((Value::String(v.to_string()), rest))
        }
        Usize => {
            let (v, rest) = de::<u64>(bytes, path)?;
            Ok((json!(v), rest))
        }
        I8 => {
            let (v, rest) = de::<i8>(bytes, path)?;
            Ok((json!(v), rest))
        }
        I16 => {
            let (v, rest) = de::<i16>(bytes, path)?;
            Ok((json!(v), rest))
        }
        I32 => {
            let (v, rest) = de::<i32>(bytes, path)?;
            Ok((json!(v), rest))
        }
        I64 => {
            let (v, rest) = de::<i64>(bytes, path)?;
            Ok((json!(v), rest))
        }
        I128 => {
            let (v, rest) = de::<i128>(bytes, path)?;
            Ok((Value::String(v.to_string()), rest))
        }
        Isize => {
            let (v, rest) = de::<i64>(bytes, path)?;
            Ok((json!(v), rest))
        }
        F32 => {
            let (v, rest) = de::<f32>(bytes, path)?;
            Ok((json!(v), rest))
        }
        F64 => {
            let (v, rest) = de::<f64>(bytes, path)?;
            Ok((json!(v), rest))
        }
        Char => {
            let (v, rest) = de::<char>(bytes, path)?;
            Ok((Value::String(v.to_string()), rest))
        }
        String => {
            let (v, rest) = de::<std::string::String>(bytes, path)?;
            Ok((Value::String(v), rest))
        }
        ByteArray => {
            let (v, rest) = de::<Vec<u8>>(bytes, path)?;
            let arr: Vec<Value> = v.into_iter().map(|b| json!(b)).collect();
            Ok((Value::Array(arr), rest))
        }
        Option(inner) => {
            if bytes.is_empty() {
                return Err(ConvertError::Postcard(format!("unexpected EOF at {path}")));
            }
            match bytes[0] {
                0x00 => Ok((Value::Null, &bytes[1..])),
                0x01 => {
                    let (val, rest) = decode_value(&inner.ty, &bytes[1..], path, depth + 1)?;
                    Ok((val, rest))
                }
                b => Err(ConvertError::Postcard(format!(
                    "invalid option tag {b:#02x} at {path}"
                ))),
            }
        }
        Unit | UnitStruct => Ok((Value::Null, bytes)),
        NewtypeStruct(inner) => decode_value(&inner.ty, bytes, path, depth + 1),
        Seq(inner) => {
            let (count_u64, rest_after_count) = de::<u64>(bytes, path)?;
            let count = usize::try_from(count_u64).map_err(|_| {
                ConvertError::Postcard(format!("seq count {count_u64} exceeds usize at {path}"))
            })?;
            if count > rest_after_count.len() {
                return Err(ConvertError::Postcard(format!(
                    "seq count {count} exceeds remaining bytes {} at {path}",
                    rest_after_count.len()
                )));
            }
            let mut rest = rest_after_count;
            let mut arr = Vec::with_capacity(count);
            for i in 0..count {
                let (item, next) =
                    decode_value(&inner.ty, rest, &format!("{path}[{i}]"), depth + 1)?;
                arr.push(item);
                rest = next;
            }
            Ok((Value::Array(arr), rest))
        }
        Tuple(elems) | TupleStruct(elems) => {
            // elems: Vec<OwnedNamedType>; e.ty is OwnedDataModelType
            let mut arr = Vec::with_capacity(elems.len());
            let mut rest = bytes;
            for (i, sch) in elems.iter().enumerate() {
                let (item, next) = decode_value(&sch.ty, rest, &format!("{path}.{i}"), depth + 1)?;
                arr.push(item);
                rest = next;
            }
            Ok((Value::Array(arr), rest))
        }
        Map { key, val } => {
            let (count_u64, rest_after_count) = de::<u64>(bytes, path)?;
            let count = usize::try_from(count_u64).map_err(|_| {
                ConvertError::Postcard(format!("map count {count_u64} exceeds usize at {path}"))
            })?;
            if count > rest_after_count.len() {
                return Err(ConvertError::Postcard(format!(
                    "map count {count} exceeds remaining bytes {} at {path}",
                    rest_after_count.len()
                )));
            }
            let mut rest = rest_after_count;
            if matches!(&key.ty, OwnedDataModelType::String) {
                let mut obj = JsonMap::new();
                for i in 0..count {
                    let (k, next) = de::<std::string::String>(rest, &format!("{path}[{i}].key"))?;
                    let (v, next2) =
                        decode_value(&val.ty, next, &format!("{path}[{i}].val"), depth + 1)?;
                    obj.insert(k, v);
                    rest = next2;
                }
                Ok((Value::Object(obj), rest))
            } else {
                let mut arr = Vec::new();
                for i in 0..count {
                    let (k, next) =
                        decode_value(&key.ty, rest, &format!("{path}[{i}].key"), depth + 1)?;
                    let (v, next2) =
                        decode_value(&val.ty, next, &format!("{path}[{i}].val"), depth + 1)?;
                    arr.push(json!([k, v]));
                    rest = next2;
                }
                Ok((Value::Array(arr), rest))
            }
        }
        Struct(fields) => {
            // fields: Vec<OwnedNamedValue>; f.ty is OwnedNamedType; f.ty.ty is OwnedDataModelType
            let mut obj = JsonMap::new();
            let mut rest = bytes;
            for f in fields {
                let (val, next) =
                    decode_value(&f.ty.ty, rest, &format!("{path}.{}", f.name), depth + 1)?;
                obj.insert(f.name.to_string(), val);
                rest = next;
            }
            Ok((Value::Object(obj), rest))
        }
        Enum(variants) => {
            let (idx, rest) = de::<u32>(bytes, path)?;
            let variant = variants.get(idx as usize).ok_or_else(|| {
                ConvertError::Postcard(format!("enum discriminant {idx} out of range at {path}"))
            })?;
            let (payload, rest2) = decode_variant_payload(&variant.ty, rest, path, depth + 1)?;
            match payload {
                None => Ok((Value::String(variant.name.to_string()), rest2)),
                Some(p) => {
                    let mut obj = JsonMap::new();
                    obj.insert(variant.name.to_string(), p);
                    Ok((Value::Object(obj), rest2))
                }
            }
        }
        Schema => Err(ConvertError::TypeUnsupported {
            ty: "Schema",
            path: path.to_string(),
        }),
    }
}

fn decode_variant_payload<'a>(
    ty: &OwnedDataModelVariant,
    bytes: &'a [u8],
    path: &str,
    depth: usize,
) -> Result<(Option<Value>, &'a [u8]), ConvertError> {
    match ty {
        OwnedDataModelVariant::UnitVariant => Ok((None, bytes)),
        OwnedDataModelVariant::NewtypeVariant(inner) => {
            let (val, rest) = decode_value(&inner.ty, bytes, path, depth)?;
            Ok((Some(val), rest))
        }
        OwnedDataModelVariant::TupleVariant(elems) => {
            let mut arr = Vec::new();
            let mut rest = bytes;
            for (i, sch) in elems.iter().enumerate() {
                let (item, next) = decode_value(&sch.ty, rest, &format!("{path}.{i}"), depth + 1)?;
                arr.push(item);
                rest = next;
            }
            Ok((Some(Value::Array(arr)), rest))
        }
        OwnedDataModelVariant::StructVariant(fields) => {
            let mut obj = JsonMap::new();
            let mut rest = bytes;
            for f in fields {
                let (val, next) =
                    decode_value(&f.ty.ty, rest, &format!("{path}.{}", f.name), depth + 1)?;
                obj.insert(f.name.to_string(), val);
                rest = next;
            }
            Ok((Some(Value::Object(obj)), rest))
        }
    }
}
