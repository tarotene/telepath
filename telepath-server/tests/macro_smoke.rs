use telepath_server::{command, CommandMetadata, DispatchOutcome};
use telepath_wire::AppErrorPayload;

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

/// A fallible command: divides `a` by `b`, returning an `AppErrorPayload` on
/// divide-by-zero.
#[command]
fn checked_div(a: u32, b: u32) -> Result<u32, AppErrorPayload<'static>> {
    if b == 0 {
        Err(AppErrorPayload {
            code: 1,
            message: "divide by zero",
        })
    } else {
        Ok(a / b)
    }
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
    let reg = telepath_server::ResourceRegistry::new();
    let outcome = (__TELEPATH_CMD_NULLARY_PING.invoke)(&[], &mut out, &reg).unwrap();
    let DispatchOutcome::Ok(n) = outcome else {
        panic!("expected DispatchOutcome::Ok, got {outcome:?}");
    };
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 0xDEAD_BEEF);
}

#[test]
fn shim_multiarg_roundtrip() {
    let mut input_buf = [0u8; 16];
    let serialized = postcard::to_slice(&(3u32, 4u32), &mut input_buf).unwrap();
    let mut out = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let outcome = (__TELEPATH_CMD_ADD.invoke)(serialized, &mut out, &reg).unwrap();
    let DispatchOutcome::Ok(n) = outcome else {
        panic!("expected DispatchOutcome::Ok, got {outcome:?}");
    };
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 7);
}

#[test]
fn shim_unary_roundtrip() {
    let mut input_buf = [0u8; 4];
    let serialized = postcard::to_slice(&(42u8,), &mut input_buf).unwrap();
    let mut out = [0u8; 4];
    let reg = telepath_server::ResourceRegistry::new();
    let outcome = (__TELEPATH_CMD_UNARY_ECHO.invoke)(serialized, &mut out, &reg).unwrap();
    let DispatchOutcome::Ok(n) = outcome else {
        panic!("expected DispatchOutcome::Ok, got {outcome:?}");
    };
    let val: u8 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 42);
}

#[test]
fn shim_nullary_rejects_nonempty_input() {
    let mut out = [0u8; 16];
    let reg = telepath_server::ResourceRegistry::new();
    let result = (__TELEPATH_CMD_NULLARY_PING.invoke)(&[0xAB], &mut out, &reg);
    assert!(matches!(
        result,
        Err(telepath_server::DispatchError::DeserializeError)
    ));
}

// ---------------------------------------------------------------------------
// Result<T, AppErrorPayload> shim tests
// ---------------------------------------------------------------------------

#[test]
fn shim_result_ok_arm() {
    let mut input_buf = [0u8; 16];
    let serialized = postcard::to_slice(&(10u32, 2u32), &mut input_buf).unwrap();
    let mut out = [0u8; 32];
    let reg = telepath_server::ResourceRegistry::new();
    let outcome = (__TELEPATH_CMD_CHECKED_DIV.invoke)(serialized, &mut out, &reg).unwrap();
    let DispatchOutcome::Ok(n) = outcome else {
        panic!("expected DispatchOutcome::Ok, got {outcome:?}");
    };
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 5);
}

#[test]
fn shim_result_err_arm_emits_app_error() {
    let mut input_buf = [0u8; 16];
    // Divide by zero triggers the Err arm.
    let serialized = postcard::to_slice(&(10u32, 0u32), &mut input_buf).unwrap();
    let mut out = [0u8; 32];
    let reg = telepath_server::ResourceRegistry::new();
    let outcome = (__TELEPATH_CMD_CHECKED_DIV.invoke)(serialized, &mut out, &reg).unwrap();
    let DispatchOutcome::AppError(n) = outcome else {
        panic!("expected DispatchOutcome::AppError, got {outcome:?}");
    };
    let payload = telepath_wire::decode_app_error(&out[..n]).unwrap();
    assert_eq!(payload.code, 1);
    assert_eq!(payload.message, "divide by zero");
}

#[test]
fn shim_result_cmd_id_uses_result_type() {
    // cmd_id must include the full "Result<u32, AppErrorPayload<'static>>" text
    // so that a previously infallible command with the same name/args would get
    // a different ID (wire ABI break accepted, as per the issue spec).
    let id_result_variant =
        telepath_wire::cmd_id::derive_cmd_id("checked_div", "(u32, u32)", "u32");
    // The actual cmd_id was derived from the full Result<...> ret_type_str, but
    // the schema uses just `u32` — verify the schema writer compiles and runs.
    let mut schema_buf = [0u8; 64];
    let n = (__TELEPATH_CMD_CHECKED_DIV.ret_schema)(&mut schema_buf)
        .expect("ret_schema writer must not fail");
    assert!(n > 0, "ret_schema must write non-zero bytes for u32");
    // The cmd_id must differ from the hypothetical infallible `checked_div(u32,u32)->u32`.
    assert_ne!(
        __TELEPATH_CMD_CHECKED_DIV.id, id_result_variant,
        "Result<T, AppErrorPayload> and T must produce different cmd_ids"
    );
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

#[test]
fn commands_have_correct_arg_names() {
    assert_eq!(
        COMMANDS[0].arg_names, "",
        "0-arg command must have empty arg_names"
    );
    assert_eq!(COMMANDS[1].arg_names, "a,b", "add(a, b) must emit \"a,b\"");
    assert_eq!(COMMANDS[2].arg_names, "x", "unary_echo(x) must emit \"x\"");
}
