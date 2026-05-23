use postcard_schema::schema::owned::{
    OwnedDataModelType, OwnedDataModelVariant, OwnedNamedType, OwnedNamedVariant,
};
use serde_json::{json, Map as JsonMap, Value};

const MAX_DEPTH: usize = 8;

pub fn named_type_to_json_schema(nt: &OwnedNamedType) -> Value {
    dmt_to_schema(&nt.ty, 0)
}

fn dmt_to_schema(dmt: &OwnedDataModelType, depth: usize) -> Value {
    if depth >= MAX_DEPTH {
        return json!({"type": "object", "description": "schema depth exceeded"});
    }
    use OwnedDataModelType::*;
    match dmt {
        Bool => json!({"type": "boolean"}),
        U8 => json!({"type": "integer", "minimum": 0, "maximum": 255u64}),
        U16 => json!({"type": "integer", "minimum": 0, "maximum": 65535u64}),
        U32 => json!({"type": "integer", "minimum": 0, "maximum": 4294967295u64}),
        U64 => json!({"type": "integer", "minimum": 0, "maximum": 18446744073709551615u64}),
        U128 => json!({"type": "string", "pattern": "^[0-9]+$"}),
        Usize => json!({"type": "integer", "minimum": 0, "maximum": 18446744073709551615u64}),
        I8 => json!({"type": "integer", "minimum": -128i64, "maximum": 127i64}),
        I16 => json!({"type": "integer", "minimum": -32768i64, "maximum": 32767i64}),
        I32 => json!({"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64}),
        I64 => json!({"type": "integer", "minimum": i64::MIN, "maximum": i64::MAX}),
        I128 => json!({"type": "string", "pattern": "^-?[0-9]+$"}),
        Isize => json!({"type": "integer", "minimum": i64::MIN, "maximum": i64::MAX}),
        F32 | F64 => json!({"type": "number"}),
        Char => json!({"type": "string", "minLength": 1u64, "maxLength": 1u64}),
        String => json!({"type": "string"}),
        ByteArray => json!({
            "type": "array",
            "items": {"type": "integer", "minimum": 0, "maximum": 255u64}
        }),
        Option(inner) => {
            let inner_schema = dmt_to_schema(&inner.ty, depth + 1);
            json!({"oneOf": [inner_schema, {"type": "null"}]})
        }
        Unit | UnitStruct => json!({"type": "null"}),
        NewtypeStruct(inner) => dmt_to_schema(&inner.ty, depth + 1),
        Seq(inner) => {
            let items = dmt_to_schema(&inner.ty, depth + 1);
            json!({"type": "array", "items": items})
        }
        Tuple(elems) | TupleStruct(elems) => {
            // elems: Vec<OwnedNamedType>; each elem.ty is OwnedDataModelType
            let prefix: Vec<Value> = elems
                .iter()
                .map(|e| dmt_to_schema(&e.ty, depth + 1))
                .collect();
            let n = prefix.len();
            json!({
                "type": "array",
                "prefixItems": prefix,
                "minItems": n,
                "maxItems": n
            })
        }
        Map { key, val } => {
            // key, val: Box<OwnedNamedType>; .ty is OwnedDataModelType
            let val_schema = dmt_to_schema(&val.ty, depth + 1);
            if matches!(&key.ty, OwnedDataModelType::String) {
                json!({"type": "object", "additionalProperties": val_schema})
            } else {
                let key_schema = dmt_to_schema(&key.ty, depth + 1);
                json!({
                    "type": "array",
                    "items": {
                        "type": "array",
                        "prefixItems": [key_schema, val_schema],
                        "minItems": 2u64,
                        "maxItems": 2u64
                    }
                })
            }
        }
        Struct(fields) => {
            // fields: Vec<OwnedNamedValue>; f.ty is OwnedNamedType; f.ty.ty is OwnedDataModelType
            let mut props = JsonMap::new();
            let mut required: Vec<Value> = Vec::new();
            for f in fields {
                props.insert(f.name.to_string(), dmt_to_schema(&f.ty.ty, depth + 1));
                required.push(Value::String(f.name.to_string()));
            }
            json!({
                "type": "object",
                "properties": props,
                "required": required,
                "additionalProperties": false
            })
        }
        Enum(variants) => {
            let all_unit = variants
                .iter()
                .all(|v| matches!(&v.ty, OwnedDataModelVariant::UnitVariant));
            if all_unit {
                let names: Vec<Value> = variants
                    .iter()
                    .map(|v| Value::String(v.name.to_string()))
                    .collect();
                json!({"type": "string", "enum": names})
            } else {
                let branches: Vec<Value> = variants
                    .iter()
                    .map(|v| variant_to_schema(v, depth + 1))
                    .collect();
                json!({"oneOf": branches})
            }
        }
        Schema => json!({"type": "object", "description": "opaque postcard schema"}),
    }
}

fn variant_to_schema(v: &OwnedNamedVariant, depth: usize) -> Value {
    match &v.ty {
        OwnedDataModelVariant::UnitVariant => {
            // In mixed enums, unit variants are encoded as a JSON string so the
            // schema and json_to_postcard agree on the representation.
            json!({"type": "string", "const": v.name.as_str()})
        }
        OwnedDataModelVariant::NewtypeVariant(inner) => {
            let inner_schema = dmt_to_schema(&inner.ty, depth);
            json!({
                "type": "object",
                "properties": {v.name.as_str(): inner_schema},
                "required": [v.name.as_str()],
                "additionalProperties": false
            })
        }
        OwnedDataModelVariant::TupleVariant(elems) => {
            let prefix: Vec<Value> = elems.iter().map(|e| dmt_to_schema(&e.ty, depth)).collect();
            let n = prefix.len();
            let inner = json!({
                "type": "array",
                "prefixItems": prefix,
                "minItems": n,
                "maxItems": n
            });
            json!({
                "type": "object",
                "properties": {v.name.as_str(): inner},
                "required": [v.name.as_str()],
                "additionalProperties": false
            })
        }
        OwnedDataModelVariant::StructVariant(fields) => {
            // fields: Vec<OwnedNamedValue>; f.ty is OwnedNamedType
            let mut props = JsonMap::new();
            let mut required: Vec<Value> = Vec::new();
            for f in fields {
                props.insert(f.name.to_string(), dmt_to_schema(&f.ty.ty, depth));
                required.push(Value::String(f.name.to_string()));
            }
            let inner = json!({
                "type": "object",
                "properties": props,
                "required": required,
                "additionalProperties": false
            });
            json!({
                "type": "object",
                "properties": {v.name.as_str(): inner},
                "required": [v.name.as_str()],
                "additionalProperties": false
            })
        }
    }
}
