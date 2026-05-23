use telepath_server::{command, CommandMetadata};

// ---------------------------------------------------------------------------
// Functions under test
// ---------------------------------------------------------------------------

#[command]
fn nullary_ping() -> u32 {
    0xDEAD_BEEF
}

#[command]
fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[command]
fn unary_echo(x: u8) -> u8 {
    x
}

// ---------------------------------------------------------------------------
// Metadata presence
// ---------------------------------------------------------------------------

#[test]
fn nullary_metadata_name() {
    assert_eq!(__TELEPATH_CMD_NULLARY_PING.name, "nullary_ping");
}

#[test]
fn nullary_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_NULLARY_PING.id, 0x0000);
}

#[test]
fn multiarg_metadata_name() {
    assert_eq!(__TELEPATH_CMD_ADD.name, "add");
}

#[test]
fn multiarg_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_ADD.id, 0x0000);
}

#[test]
fn unary_metadata_name() {
    assert_eq!(__TELEPATH_CMD_UNARY_ECHO.name, "unary_echo");
}

#[test]
fn unary_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_UNARY_ECHO.id, 0x0000);
}

// ---------------------------------------------------------------------------
// Determinism: macro-derived ID matches direct derive_cmd_id call
// ---------------------------------------------------------------------------

#[test]
fn cmd_id_deterministic() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("nullary_ping", "()", "u32");
    assert_eq!(__TELEPATH_CMD_NULLARY_PING.id, expected);
}

#[test]
fn unary_cmd_id_matches_canonical_tuple() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("unary_echo", "(u8,)", "u8");
    assert_eq!(__TELEPATH_CMD_UNARY_ECHO.id, expected);
}

// ---------------------------------------------------------------------------
// Shim roundtrips
// ---------------------------------------------------------------------------

#[test]
fn shim_nullary_roundtrip() {
    let mut out = [0u8; 16];
    let n = (__TELEPATH_CMD_NULLARY_PING.invoke)(&[], &mut out).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 0xDEAD_BEEF);
}

#[test]
fn shim_multiarg_roundtrip() {
    let mut input_buf = [0u8; 16];
    let serialized = postcard::to_slice(&(3u32, 4u32), &mut input_buf).unwrap();
    let mut out = [0u8; 16];
    let n = (__TELEPATH_CMD_ADD.invoke)(serialized, &mut out).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 7);
}

#[test]
fn shim_unary_roundtrip() {
    let mut input_buf = [0u8; 4];
    let serialized = postcard::to_slice(&(42u8,), &mut input_buf).unwrap();
    let mut out = [0u8; 4];
    let n = (__TELEPATH_CMD_UNARY_ECHO.invoke)(serialized, &mut out).unwrap();
    let val: u8 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 42);
}

#[test]
fn shim_nullary_rejects_nonempty_input() {
    let mut out = [0u8; 16];
    let result = (__TELEPATH_CMD_NULLARY_PING.invoke)(&[0xAB], &mut out);
    assert!(matches!(
        result,
        Err(telepath_server::DispatchError::DeserializeError)
    ));
}

// ---------------------------------------------------------------------------
// Original functions remain callable directly
// ---------------------------------------------------------------------------

#[test]
fn original_fns_callable() {
    assert_eq!(nullary_ping(), 0xDEAD_BEEF);
    assert_eq!(add(2, 3), 5);
    assert_eq!(unary_echo(7), 7);
}

// ---------------------------------------------------------------------------
// CommandMetadata usable in a static array (Copy / const check)
// ---------------------------------------------------------------------------

static COMMANDS: [CommandMetadata; 3] = [
    __TELEPATH_CMD_NULLARY_PING,
    __TELEPATH_CMD_ADD,
    __TELEPATH_CMD_UNARY_ECHO,
];

#[test]
fn commands_array_has_correct_names() {
    assert_eq!(COMMANDS[0].name, "nullary_ping");
    assert_eq!(COMMANDS[1].name, "add");
    assert_eq!(COMMANDS[2].name, "unary_echo");
}
