use postcard_schema::schema::owned::{
    OwnedDataModelType as DMT, OwnedDataModelVariant, OwnedNamedType, OwnedNamedValue,
    OwnedNamedVariant,
};
use serde_json::{json, Value};
use telepath::codec::schema_to_json::named_type_to_json_schema;

fn wrap(name: &str, ty: DMT) -> OwnedNamedType {
    OwnedNamedType {
        name: name.to_string(),
        ty,
    }
}

fn field(name: &str, ty: DMT) -> OwnedNamedValue {
    OwnedNamedValue {
        name: name.to_string(),
        ty: wrap(name, ty),
    }
}

fn variant(name: &str, ty: OwnedDataModelVariant) -> OwnedNamedVariant {
    OwnedNamedVariant {
        name: name.to_string(),
        ty,
    }
}

fn schema(ty: DMT) -> Value {
    named_type_to_json_schema(&wrap("_", ty))
}

#[test]
fn bool_maps_to_boolean() {
    assert_eq!(schema(DMT::Bool), json!({"type": "boolean"}));
}

#[test]
fn u8_is_bounded_integer() {
    assert_eq!(
        schema(DMT::U8),
        json!({"type": "integer", "minimum": 0, "maximum": 255u64})
    );
}

#[test]
fn u16_is_bounded_integer() {
    assert_eq!(
        schema(DMT::U16),
        json!({"type": "integer", "minimum": 0, "maximum": 65535u64})
    );
}

#[test]
fn u32_is_bounded_integer() {
    assert_eq!(
        schema(DMT::U32),
        json!({"type": "integer", "minimum": 0, "maximum": 4294967295u64})
    );
}

#[test]
fn u64_is_bounded_integer() {
    assert_eq!(
        schema(DMT::U64),
        json!({"type": "integer", "minimum": 0, "maximum": 18446744073709551615u64})
    );
}

#[test]
fn u128_is_decimal_string() {
    assert_eq!(
        schema(DMT::U128),
        json!({"type": "string", "pattern": "^[0-9]+$"})
    );
}

#[test]
fn usize_same_as_u64() {
    assert_eq!(
        schema(DMT::Usize),
        json!({"type": "integer", "minimum": 0, "maximum": 18446744073709551615u64})
    );
}

#[test]
fn i8_is_bounded_integer() {
    assert_eq!(
        schema(DMT::I8),
        json!({"type": "integer", "minimum": -128i64, "maximum": 127i64})
    );
}

#[test]
fn i16_is_bounded_integer() {
    assert_eq!(
        schema(DMT::I16),
        json!({"type": "integer", "minimum": -32768i64, "maximum": 32767i64})
    );
}

#[test]
fn i32_is_bounded_integer() {
    assert_eq!(
        schema(DMT::I32),
        json!({"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64})
    );
}

#[test]
fn i64_is_bounded_integer() {
    assert_eq!(
        schema(DMT::I64),
        json!({"type": "integer", "minimum": i64::MIN, "maximum": i64::MAX})
    );
}

#[test]
fn i128_is_decimal_string() {
    assert_eq!(
        schema(DMT::I128),
        json!({"type": "string", "pattern": "^-?[0-9]+$"})
    );
}

#[test]
fn isize_same_as_i64() {
    assert_eq!(
        schema(DMT::Isize),
        json!({"type": "integer", "minimum": i64::MIN, "maximum": i64::MAX})
    );
}

#[test]
fn f32_is_number() {
    assert_eq!(schema(DMT::F32), json!({"type": "number"}));
}

#[test]
fn f64_is_number() {
    assert_eq!(schema(DMT::F64), json!({"type": "number"}));
}

#[test]
fn char_is_single_char_string() {
    assert_eq!(
        schema(DMT::Char),
        json!({"type": "string", "minLength": 1u64, "maxLength": 1u64})
    );
}

#[test]
fn string_is_string() {
    assert_eq!(schema(DMT::String), json!({"type": "string"}));
}

#[test]
fn byte_array_is_u8_array() {
    assert_eq!(
        schema(DMT::ByteArray),
        json!({"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255u64}})
    );
}

#[test]
fn option_wraps_in_one_of_with_null() {
    let inner = wrap("u32", DMT::U32);
    let s = schema(DMT::Option(Box::new(inner)));
    assert_eq!(
        s,
        json!({"oneOf": [
            {"type": "integer", "minimum": 0, "maximum": 4294967295u64},
            {"type": "null"}
        ]})
    );
}

#[test]
fn unit_is_null() {
    assert_eq!(schema(DMT::Unit), json!({"type": "null"}));
}

#[test]
fn unit_struct_is_null() {
    assert_eq!(schema(DMT::UnitStruct), json!({"type": "null"}));
}

#[test]
fn newtype_struct_is_transparent() {
    let inner = wrap("u32", DMT::U32);
    assert_eq!(
        schema(DMT::NewtypeStruct(Box::new(inner))),
        json!({"type": "integer", "minimum": 0, "maximum": 4294967295u64})
    );
}

#[test]
fn seq_is_typed_array() {
    let inner = wrap("bool", DMT::Bool);
    assert_eq!(
        schema(DMT::Seq(Box::new(inner))),
        json!({"type": "array", "items": {"type": "boolean"}})
    );
}

#[test]
fn tuple_is_prefix_items_array() {
    let elems = vec![wrap("0", DMT::U32), wrap("1", DMT::Bool)];
    assert_eq!(
        schema(DMT::Tuple(elems)),
        json!({
            "type": "array",
            "prefixItems": [
                {"type": "integer", "minimum": 0, "maximum": 4294967295u64},
                {"type": "boolean"}
            ],
            "minItems": 2u64,
            "maxItems": 2u64
        })
    );
}

#[test]
fn tuple_struct_is_prefix_items_array() {
    let elems = vec![wrap("0", DMT::I32)];
    assert_eq!(
        schema(DMT::TupleStruct(elems)),
        json!({
            "type": "array",
            "prefixItems": [{"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64}],
            "minItems": 1u64,
            "maxItems": 1u64
        })
    );
}

#[test]
fn map_with_string_key_is_object() {
    let key = wrap("String", DMT::String);
    let val = wrap("u32", DMT::U32);
    assert_eq!(
        schema(DMT::Map {
            key: Box::new(key),
            val: Box::new(val)
        }),
        json!({"type": "object", "additionalProperties": {"type": "integer", "minimum": 0, "maximum": 4294967295u64}})
    );
}

#[test]
fn map_with_non_string_key_is_pair_array() {
    let key = wrap("u32", DMT::U32);
    let val = wrap("bool", DMT::Bool);
    assert_eq!(
        schema(DMT::Map {
            key: Box::new(key),
            val: Box::new(val)
        }),
        json!({
            "type": "array",
            "items": {
                "type": "array",
                "prefixItems": [
                    {"type": "integer", "minimum": 0, "maximum": 4294967295u64},
                    {"type": "boolean"}
                ],
                "minItems": 2u64,
                "maxItems": 2u64
            }
        })
    );
}

#[test]
fn struct_is_object_with_properties_and_required() {
    let fields = vec![field("x", DMT::I32), field("y", DMT::I32)];
    let s = schema(DMT::Struct(fields));
    assert_eq!(
        s,
        json!({
            "type": "object",
            "properties": {
                "x": {"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64},
                "y": {"type": "integer", "minimum": -2147483648i64, "maximum": 2147483647i64}
            },
            "required": ["x", "y"],
            "additionalProperties": false
        })
    );
}

#[test]
fn enum_all_unit_variants_is_string_enum() {
    let variants = vec![
        variant("Red", OwnedDataModelVariant::UnitVariant),
        variant("Green", OwnedDataModelVariant::UnitVariant),
        variant("Blue", OwnedDataModelVariant::UnitVariant),
    ];
    assert_eq!(
        schema(DMT::Enum(variants)),
        json!({"type": "string", "enum": ["Red", "Green", "Blue"]})
    );
}

#[test]
fn enum_mixed_variants_is_one_of() {
    let variants = vec![
        variant("None", OwnedDataModelVariant::UnitVariant),
        variant(
            "Some",
            OwnedDataModelVariant::NewtypeVariant(Box::new(wrap("u32", DMT::U32))),
        ),
    ];
    let s = schema(DMT::Enum(variants));
    assert_eq!(
        s,
        json!({
            "oneOf": [
                {"type": "string", "const": "None"},
                {
                    "type": "object",
                    "properties": {
                        "Some": {"type": "integer", "minimum": 0u64, "maximum": 4294967295u64}
                    },
                    "required": ["Some"],
                    "additionalProperties": false
                }
            ]
        })
    );
}

#[test]
fn schema_variant_is_opaque_object() {
    let s = schema(DMT::Schema);
    assert_eq!(s["type"], json!("object"));
}

#[test]
fn depth_guard_does_not_stack_overflow() {
    // Build a deeply nested Option<Option<Option<...>>> beyond MAX_DEPTH
    let mut ty = DMT::U32;
    for _ in 0..20 {
        ty = DMT::Option(Box::new(wrap("_", ty)));
    }
    // Should not panic; truncates at depth 8
    let _ = schema(ty);
}
