use telepath_server::command;

// ---------------------------------------------------------------------------
// Stub implementations — sentinel values stand in for real hardware reads.
// These mirror the exact signatures that nrf52840-ping/src/main.rs will use.
// ---------------------------------------------------------------------------

#[command]
fn temp_read() -> i16 {
    100 // 100 * 0.25 °C = 25 °C
}

#[command]
fn rng_u32() -> u32 {
    0xCAFE_BABE
}

#[command]
fn ficr_uid() -> (u32, u32) {
    (0xAAAA_AAAA, 0x5555_5555)
}

#[command]
fn saadc_vdd_mv() -> u16 {
    3300
}

// ---------------------------------------------------------------------------
// Metadata: name and non-zero ID
// ---------------------------------------------------------------------------

#[test]
fn temp_read_metadata_name() {
    assert_eq!(__TELEPATH_CMD_TEMP_READ.name, "temp_read");
}

#[test]
fn temp_read_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_TEMP_READ.id, 0x0000);
}

#[test]
fn rng_u32_metadata_name() {
    assert_eq!(__TELEPATH_CMD_RNG_U32.name, "rng_u32");
}

#[test]
fn rng_u32_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_RNG_U32.id, 0x0000);
}

#[test]
fn ficr_uid_metadata_name() {
    assert_eq!(__TELEPATH_CMD_FICR_UID.name, "ficr_uid");
}

#[test]
fn ficr_uid_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_FICR_UID.id, 0x0000);
}

#[test]
fn saadc_vdd_mv_metadata_name() {
    assert_eq!(__TELEPATH_CMD_SAADC_VDD_MV.name, "saadc_vdd_mv");
}

#[test]
fn saadc_vdd_mv_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_SAADC_VDD_MV.id, 0x0000);
}

// ---------------------------------------------------------------------------
// Determinism: macro-derived ID matches direct derive_cmd_id call
// ---------------------------------------------------------------------------

#[test]
fn temp_read_cmd_id_deterministic() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("temp_read", "()", "i16");
    assert_eq!(__TELEPATH_CMD_TEMP_READ.id, expected);
}

#[test]
fn rng_u32_cmd_id_deterministic() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("rng_u32", "()", "u32");
    assert_eq!(__TELEPATH_CMD_RNG_U32.id, expected);
}

#[test]
fn ficr_uid_cmd_id_deterministic() {
    // quote! { (u32, u32) }.to_string() → "(u32, u32)"
    let expected = telepath_wire::cmd_id::derive_cmd_id("ficr_uid", "()", "(u32, u32)");
    assert_eq!(__TELEPATH_CMD_FICR_UID.id, expected);
}

#[test]
fn saadc_vdd_mv_cmd_id_deterministic() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("saadc_vdd_mv", "()", "u16");
    assert_eq!(__TELEPATH_CMD_SAADC_VDD_MV.id, expected);
}

// ---------------------------------------------------------------------------
// Shim roundtrips — confirm postcard encode/decode of each return type
// ---------------------------------------------------------------------------

#[test]
fn shim_temp_read_roundtrip() {
    let mut out = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let n = (__TELEPATH_CMD_TEMP_READ.invoke)(&[], &mut out, &reg).unwrap();
    let val: i16 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 100);
}

#[test]
fn shim_rng_u32_roundtrip() {
    let mut out = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let n = (__TELEPATH_CMD_RNG_U32.invoke)(&[], &mut out, &reg).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 0xCAFE_BABE);
}

#[test]
fn shim_ficr_uid_roundtrip() {
    let mut out = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let n = (__TELEPATH_CMD_FICR_UID.invoke)(&[], &mut out, &reg).unwrap();
    let val: (u32, u32) = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, (0xAAAA_AAAA, 0x5555_5555));
}

#[test]
fn shim_saadc_vdd_mv_roundtrip() {
    let mut out = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let n = (__TELEPATH_CMD_SAADC_VDD_MV.invoke)(&[], &mut out, &reg).unwrap();
    let val: u16 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 3300);
}

// ---------------------------------------------------------------------------
// Tuple return type: postcard encodes (u32, u32) as two consecutive varints.
// Use postcard as the encoding oracle rather than hand-derived varint math.
// ---------------------------------------------------------------------------

#[test]
fn ficr_uid_postcard_byte_width() {
    let mut buf = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let n = (__TELEPATH_CMD_FICR_UID.invoke)(&[], &mut buf, &reg).unwrap();
    let mut oracle_buf = [0u8; 16];
    let expected = postcard::to_slice(&(0xAAAA_AAAAu32, 0x5555_5555u32), &mut oracle_buf)
        .unwrap()
        .len();
    assert_eq!(
        n, expected,
        "unexpected postcard encoding length for (u32, u32) sentinel"
    );
}

// ---------------------------------------------------------------------------
// Original functions remain directly callable
// ---------------------------------------------------------------------------

#[test]
fn original_fns_callable() {
    assert_eq!(temp_read(), 100);
    assert_eq!(rng_u32(), 0xCAFE_BABE);
    assert_eq!(ficr_uid(), (0xAAAA_AAAA, 0x5555_5555));
    assert_eq!(saadc_vdd_mv(), 3300);
}
