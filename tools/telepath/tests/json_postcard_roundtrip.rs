use postcard_schema::schema::owned::{OwnedDataModelType as DMT, OwnedNamedType, OwnedNamedValue};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use telepath::codec::json_to_postcard::{json_to_postcard, ConvertError};
use telepath::codec::postcard_to_json::postcard_to_json;

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

fn roundtrip_schema(schema: &OwnedNamedType, json_val: serde_json::Value) {
    let encoded = json_to_postcard(schema, &json_val).expect("encode");
    let decoded = postcard_to_json(schema, &encoded).expect("decode");
    assert_eq!(decoded, json_val, "roundtrip failed for {:?}", json_val);
}

// ── primitives ──────────────────────────────────────────────────────────────

#[test]
fn bool_roundtrip() {
    let s = wrap("bool", DMT::Bool);
    roundtrip_schema(&s, json!(true));
    roundtrip_schema(&s, json!(false));
}

#[test]
fn u8_roundtrip() {
    roundtrip_schema(&wrap("u8", DMT::U8), json!(0u8));
    roundtrip_schema(&wrap("u8", DMT::U8), json!(255u8));
}

#[test]
fn u16_roundtrip() {
    roundtrip_schema(&wrap("u16", DMT::U16), json!(1000u16));
}

#[test]
fn u32_roundtrip() {
    let s = wrap("u32", DMT::U32);
    roundtrip_schema(&s, json!(0u32));
    roundtrip_schema(&s, json!(0xDEADBEEFu32));
    roundtrip_schema(&s, json!(4294967295u64));
}

#[test]
fn u64_roundtrip() {
    roundtrip_schema(&wrap("u64", DMT::U64), json!(u64::MAX));
}

#[test]
fn u128_roundtrip() {
    let s = wrap("u128", DMT::U128);
    roundtrip_schema(&s, json!("0"));
    roundtrip_schema(&s, json!("340282366920938463463374607431768211455"));
}

#[test]
fn i32_roundtrip() {
    let s = wrap("i32", DMT::I32);
    roundtrip_schema(&s, json!(-1i32));
    roundtrip_schema(&s, json!(42i32));
}

#[test]
fn i128_roundtrip() {
    let s = wrap("i128", DMT::I128);
    roundtrip_schema(&s, json!("-1"));
    roundtrip_schema(&s, json!("170141183460469231731687303715884105727"));
}

#[test]
fn f32_roundtrip() {
    let s = wrap("f32", DMT::F32);
    let v = json!(1.5f64); // JSON stores as f64
    let encoded = json_to_postcard(&s, &v).unwrap();
    let decoded = postcard_to_json(&s, &encoded).unwrap();
    // f32 precision: decode back to f64 approximation
    let orig = v.as_f64().unwrap() as f32 as f64;
    assert!((decoded.as_f64().unwrap() - orig).abs() < 1e-6);
}

#[test]
fn f64_roundtrip() {
    roundtrip_schema(&wrap("f64", DMT::F64), json!(std::f64::consts::PI));
}

#[test]
fn char_roundtrip() {
    roundtrip_schema(&wrap("char", DMT::Char), json!("A"));
}

#[test]
fn string_roundtrip() {
    roundtrip_schema(&wrap("str", DMT::String), json!("hello world"));
    roundtrip_schema(&wrap("str", DMT::String), json!(""));
}

// ── byte array ──────────────────────────────────────────────────────────────

#[test]
fn byte_array_roundtrip() {
    let s = wrap("bytes", DMT::ByteArray);
    roundtrip_schema(&s, json!([]));
    roundtrip_schema(&s, json!([1u8, 2u8, 255u8]));
}

// ── option ───────────────────────────────────────────────────────────────────

#[test]
fn option_none_roundtrip() {
    let s = wrap("opt", DMT::Option(Box::new(wrap("u32", DMT::U32))));
    let encoded = json_to_postcard(&s, &json!(null)).unwrap();
    assert_eq!(encoded, &[0x00]);
    let decoded = postcard_to_json(&s, &encoded).unwrap();
    assert_eq!(decoded, json!(null));
}

#[test]
fn option_some_roundtrip() {
    let s = wrap("opt", DMT::Option(Box::new(wrap("u32", DMT::U32))));
    let v = json!(42u32);
    let encoded = json_to_postcard(&s, &v).unwrap();
    assert_eq!(encoded[0], 0x01);
    let decoded = postcard_to_json(&s, &encoded).unwrap();
    assert_eq!(decoded, v);
}

// ── unit ─────────────────────────────────────────────────────────────────────

#[test]
fn unit_emits_no_bytes() {
    let s = wrap("()", DMT::Unit);
    let encoded = json_to_postcard(&s, &json!(null)).unwrap();
    assert_eq!(encoded, Vec::<u8>::new());
}

// ── seq ──────────────────────────────────────────────────────────────────────

#[test]
fn seq_roundtrip() {
    let s = wrap("seq", DMT::Seq(Box::new(wrap("u32", DMT::U32))));
    roundtrip_schema(&s, json!([]));
    roundtrip_schema(&s, json!([1u32, 2u32, 3u32]));
}

// ── tuple ────────────────────────────────────────────────────────────────────

#[test]
fn tuple_roundtrip() {
    let s = wrap(
        "tup",
        DMT::Tuple(vec![wrap("0", DMT::U32), wrap("1", DMT::Bool)]),
    );
    roundtrip_schema(&s, json!([7u32, true]));
}

#[test]
fn tuple_single_element_bare_scalar() {
    let s = wrap("tup", DMT::Tuple(vec![wrap("0", DMT::U32)]));
    let from_array = json_to_postcard(&s, &json!([42u32])).expect("array encode");
    let from_scalar = json_to_postcard(&s, &json!(42u32)).expect("bare scalar encode");
    assert_eq!(from_array, from_scalar);
}

#[test]
fn tuple_single_element_array_still_works() {
    let s = wrap("tup", DMT::Tuple(vec![wrap("0", DMT::U32)]));
    roundtrip_schema(&s, json!([42u32]));
}

#[test]
fn tuple_arity_mismatch_is_error() {
    let s = wrap(
        "tup",
        DMT::Tuple(vec![wrap("0", DMT::U32), wrap("1", DMT::Bool)]),
    );
    let err = json_to_postcard(&s, &json!([1u32])).unwrap_err();
    assert!(matches!(err, ConvertError::ArityMismatch { .. }));
}

// ── struct ───────────────────────────────────────────────────────────────────

#[test]
fn struct_roundtrip() {
    let fields = vec![field("x", DMT::I32), field("y", DMT::I32)];
    let s = wrap("Point", DMT::Struct(fields));
    roundtrip_schema(&s, json!({"x": 5, "y": -3}));
}

#[test]
fn struct_missing_field_is_error() {
    let fields = vec![field("x", DMT::I32), field("y", DMT::I32)];
    let s = wrap("Point", DMT::Struct(fields));
    let err = json_to_postcard(&s, &json!({"x": 5})).unwrap_err();
    assert!(matches!(err, ConvertError::MissingField { .. }));
}

// ── native oracle: postcard roundtrip matches schema-driven encode ─────────────

#[derive(Serialize, Deserialize, Schema, Debug, PartialEq)]
struct Point {
    x: i32,
    y: i32,
}

#[test]
fn point_encode_matches_native_postcard() {
    let native_bytes = postcard::to_allocvec(&Point { x: 5, y: -3 }).unwrap();

    let fields = vec![field("x", DMT::I32), field("y", DMT::I32)];
    let schema = wrap("Point", DMT::Struct(fields));
    let json_val = json!({"x": 5, "y": -3});

    let our_bytes = json_to_postcard(&schema, &json_val).unwrap();
    assert_eq!(our_bytes, native_bytes);
}

#[test]
fn point_decode_matches_native_postcard() {
    let native_bytes = postcard::to_allocvec(&Point { x: 5, y: -3 }).unwrap();

    let fields = vec![field("x", DMT::I32), field("y", DMT::I32)];
    let schema = wrap("Point", DMT::Struct(fields));

    let decoded = postcard_to_json(&schema, &native_bytes).unwrap();
    assert_eq!(decoded, json!({"x": 5, "y": -3}));
}

// ── enum ─────────────────────────────────────────────────────────────────────

use postcard_schema::schema::owned::{OwnedDataModelVariant, OwnedNamedVariant};

fn evariant(name: &str, ty: OwnedDataModelVariant) -> OwnedNamedVariant {
    OwnedNamedVariant {
        name: name.to_string(),
        ty,
    }
}

#[test]
fn unit_enum_roundtrip() {
    use OwnedDataModelVariant::UnitVariant;
    let variants = vec![
        evariant("Red", UnitVariant),
        evariant("Green", UnitVariant),
        evariant("Blue", UnitVariant),
    ];
    let s = wrap("Color", DMT::Enum(variants));
    roundtrip_schema(&s, json!("Red"));
    roundtrip_schema(&s, json!("Blue"));
}

#[test]
fn newtype_enum_roundtrip() {
    use OwnedDataModelVariant::{NewtypeVariant, UnitVariant};
    let variants = vec![
        evariant("None", UnitVariant),
        evariant("Some", NewtypeVariant(Box::new(wrap("u32", DMT::U32)))),
    ];
    let s = wrap("OptionEnum", DMT::Enum(variants));
    roundtrip_schema(&s, json!("None"));
    roundtrip_schema(&s, json!({"Some": 42u32}));
}

#[test]
fn unknown_enum_variant_is_error() {
    use OwnedDataModelVariant::UnitVariant;
    let s = wrap("E", DMT::Enum(vec![evariant("A", UnitVariant)]));
    let err = json_to_postcard(&s, &json!("B")).unwrap_err();
    assert!(matches!(err, ConvertError::UnknownEnumVariant { .. }));
}

// ── count-guard error cases (DoS / 32-bit truncation protection) ─────────────

#[test]
fn seq_decoding_rejects_oversize_count() {
    let s = wrap("seq", DMT::Seq(Box::new(wrap("u8", DMT::U8))));
    // Encode u64::MAX as a postcard varint, then pass it as the full payload.
    // The count guard must reject this because count >> remaining.len().
    let bytes = postcard::to_allocvec::<u64>(&u64::MAX).unwrap();
    let err = postcard_to_json(&s, &bytes).unwrap_err();
    assert!(
        matches!(err, ConvertError::Postcard(_)),
        "expected Postcard error, got {err:?}"
    );
}

#[test]
fn map_decoding_rejects_oversize_count() {
    let s = wrap(
        "map",
        DMT::Map {
            key: Box::new(wrap("k", DMT::String)),
            val: Box::new(wrap("v", DMT::U8)),
        },
    );
    let bytes = postcard::to_allocvec::<u64>(&u64::MAX).unwrap();
    let err = postcard_to_json(&s, &bytes).unwrap_err();
    assert!(
        matches!(err, ConvertError::Postcard(_)),
        "expected Postcard error, got {err:?}"
    );
}

// ── Schema variant must hard-error (not silently corrupt the wire) ────────────

#[test]
fn schema_variant_encode_errors() {
    let s = wrap("meta", DMT::Schema);
    let err = json_to_postcard(&s, &json!(null)).unwrap_err();
    assert!(
        matches!(err, ConvertError::TypeUnsupported { ty: "Schema", .. }),
        "expected TypeUnsupported, got {err:?}"
    );
}

#[test]
fn schema_variant_decode_errors() {
    let s = wrap("meta", DMT::Schema);
    // Any byte slice; Schema must error before consuming anything.
    let err = postcard_to_json(&s, &[0x01, 0x02]).unwrap_err();
    assert!(
        matches!(err, ConvertError::TypeUnsupported { ty: "Schema", .. }),
        "expected TypeUnsupported, got {err:?}"
    );
}

// ── error cases ──────────────────────────────────────────────────────────────

#[test]
fn u8_out_of_range_negative() {
    let err = json_to_postcard(&wrap("u8", DMT::U8), &json!(-1)).unwrap_err();
    assert!(matches!(err, ConvertError::TypeMismatch { .. }));
}

#[test]
fn u8_out_of_range_too_large() {
    let err = json_to_postcard(&wrap("u8", DMT::U8), &json!(256u32)).unwrap_err();
    assert!(matches!(err, ConvertError::OutOfRange { ty: "u8", .. }));
}

#[test]
fn u32_out_of_range_too_large() {
    let err = json_to_postcard(&wrap("u32", DMT::U32), &json!(5_000_000_000u64)).unwrap_err();
    assert!(matches!(err, ConvertError::OutOfRange { ty: "u32", .. }));
}

#[test]
fn type_mismatch_string_for_bool() {
    let err = json_to_postcard(&wrap("bool", DMT::Bool), &json!("true")).unwrap_err();
    assert!(matches!(err, ConvertError::TypeMismatch { .. }));
}
