use telepath_server::{command, ResourceRegistry};

// ---------------------------------------------------------------------------
// Resource types
// ---------------------------------------------------------------------------

struct Counter(u32);
struct Label(&'static str);

// ---------------------------------------------------------------------------
// Commands under test
// ---------------------------------------------------------------------------

#[command]
fn increment(#[resource] ctr: &mut Counter, amount: u32) -> u32 {
    ctr.0 += amount;
    ctr.0
}

#[command]
fn read_label(#[resource] label: &Label) -> u8 {
    label.0.len() as u8
}

#[command]
fn mixed(#[resource] ctr: &mut Counter, a: u32, #[resource] label: &Label, b: u32) -> u32 {
    ctr.0 += a + b;
    ctr.0 + label.0.len() as u32
}

#[command]
fn no_resources(x: u32) -> u32 {
    x + 1
}

// ---------------------------------------------------------------------------
// Resource injection roundtrip
// ---------------------------------------------------------------------------

#[test]
fn resource_mut_injection() {
    let mut reg = ResourceRegistry::new();
    reg.insert(Counter(0));

    let mut input_buf = [0u8; 16];
    let args = postcard::to_slice(&(5u32,), &mut input_buf).unwrap();
    let mut out = [0u8; 16];

    let n = (__TELEPATH_CMD_INCREMENT.invoke)(args, &mut out, &reg).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 5);

    // Second call — resource state persists
    let n = (__TELEPATH_CMD_INCREMENT.invoke)(args, &mut out, &reg).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 10);
}

#[test]
fn resource_ref_injection() {
    let mut reg = ResourceRegistry::new();
    reg.insert(Label("hello"));

    let mut out = [0u8; 16];
    let n = (__TELEPATH_CMD_READ_LABEL.invoke)(&[], &mut out, &reg).unwrap();
    let val: u8 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 5);
}

#[test]
fn mixed_resource_and_wire_args() {
    let mut reg = ResourceRegistry::new();
    reg.insert(Counter(10));
    reg.insert(Label("hi"));

    let mut input_buf = [0u8; 16];
    let args = postcard::to_slice(&(3u32, 7u32), &mut input_buf).unwrap();
    let mut out = [0u8; 16];

    let n = (__TELEPATH_CMD_MIXED.invoke)(args, &mut out, &reg).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    // ctr.0 = 10 + 3 + 7 = 20, + label.len() = 2 → 22
    assert_eq!(val, 22);
}

#[test]
fn missing_resource_returns_error() {
    let reg = ResourceRegistry::new(); // no Counter registered

    let mut input_buf = [0u8; 16];
    let args = postcard::to_slice(&(1u32,), &mut input_buf).unwrap();
    let mut out = [0u8; 16];

    let result = (__TELEPATH_CMD_INCREMENT.invoke)(args, &mut out, &reg);
    assert!(matches!(
        result,
        Err(telepath_server::DispatchError::ResourceUnavailable)
    ));
}

// ---------------------------------------------------------------------------
// CmdID and arg_names exclude resource arguments
// ---------------------------------------------------------------------------

#[test]
fn resource_cmd_id_excludes_resource_args() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("increment", "(u32,)", "u32");
    assert_eq!(__TELEPATH_CMD_INCREMENT.id, expected);
}

#[test]
fn resource_arg_names_excludes_resource_args() {
    assert_eq!(__TELEPATH_CMD_INCREMENT.arg_names, "amount");
}

#[test]
fn mixed_cmd_id_uses_wire_args_only() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("mixed", "(u32, u32)", "u32");
    assert_eq!(__TELEPATH_CMD_MIXED.id, expected);
}

#[test]
fn mixed_arg_names_uses_wire_args_only() {
    assert_eq!(__TELEPATH_CMD_MIXED.arg_names, "a,b");
}

#[test]
fn no_resource_cmd_works_with_empty_registry() {
    let reg = ResourceRegistry::new();
    let mut input_buf = [0u8; 16];
    let args = postcard::to_slice(&(41u32,), &mut input_buf).unwrap();
    let mut out = [0u8; 16];

    let n = (__TELEPATH_CMD_NO_RESOURCES.invoke)(args, &mut out, &reg).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 42);
}

// ---------------------------------------------------------------------------
// read_label: zero wire args → nullary on the wire
// ---------------------------------------------------------------------------

#[test]
fn read_label_cmd_id_is_nullary() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("read_label", "()", "u8");
    assert_eq!(__TELEPATH_CMD_READ_LABEL.id, expected);
}

#[test]
fn read_label_arg_names_empty() {
    assert_eq!(__TELEPATH_CMD_READ_LABEL.arg_names, "");
}
