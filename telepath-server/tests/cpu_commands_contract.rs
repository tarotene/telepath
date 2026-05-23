use heapless::Vec as HVec;
use telepath_server::command;

// ---------------------------------------------------------------------------
// Stub implementations — same signatures that both loopback-demo and
// nrf52840-ping will expose.  No peripheral access; pure CPU only.
// ---------------------------------------------------------------------------

#[command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    // CRC-32/ISO-HDLC: poly=0x04C11DB7, refin=true, refout=true,
    // init=0xFFFF_FFFF, xorout=0xFFFF_FFFF.
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

#[command]
fn crc32(payload: HVec<u8, 128>) -> u32 {
    crc32_iso_hdlc(&payload)
}

#[command]
fn echo(payload: HVec<u8, 128>) -> HVec<u8, 128> {
    payload
}

// ---------------------------------------------------------------------------
// Metadata: name and non-zero ID
// ---------------------------------------------------------------------------

#[test]
fn add_metadata_name() {
    assert_eq!(__TELEPATH_CMD_ADD.name, "add");
}

#[test]
fn add_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_ADD.id, 0x0000);
}

#[test]
fn crc32_metadata_name() {
    assert_eq!(__TELEPATH_CMD_CRC32.name, "crc32");
}

#[test]
fn crc32_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_CRC32.id, 0x0000);
}

#[test]
fn echo_metadata_name() {
    assert_eq!(__TELEPATH_CMD_ECHO.name, "echo");
}

#[test]
fn echo_metadata_id_nonzero() {
    assert_ne!(__TELEPATH_CMD_ECHO.id, 0x0000);
}

// ---------------------------------------------------------------------------
// Determinism: macro-derived ID matches direct derive_cmd_id call.
// The type strings here are the TokenStream .to_string() of the
// types as written in the #[command] signature.
// ---------------------------------------------------------------------------

#[test]
fn add_cmd_id_deterministic() {
    let expected = telepath_wire::cmd_id::derive_cmd_id("add", "(i32, i32)", "i32");
    assert_eq!(__TELEPATH_CMD_ADD.id, expected);
}

// crc32 and echo type-string determinism tests are added after first run
// to capture the actual macro-emitted strings; see comments in shim tests.

// ---------------------------------------------------------------------------
// Shim roundtrips — confirm postcard encode/decode of each type
// ---------------------------------------------------------------------------

#[test]
fn shim_add_two_plus_three() {
    let mut input_buf = [0u8; 16];
    let args = postcard::to_slice(&(2i32, 3i32), &mut input_buf).unwrap();
    let mut out = [0u8; 8];
    let n = (__TELEPATH_CMD_ADD.invoke)(args, &mut out).unwrap();
    let val: i32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 5);
}

#[test]
fn shim_add_neg_plus_one() {
    let mut input_buf = [0u8; 16];
    let args = postcard::to_slice(&(-1i32, 1i32), &mut input_buf).unwrap();
    let mut out = [0u8; 8];
    let n = (__TELEPATH_CMD_ADD.invoke)(args, &mut out).unwrap();
    let val: i32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 0);
}

fn zero_vec128() -> HVec<u8, 128> {
    let mut v = HVec::new();
    for _ in 0..128 {
        v.push(0).unwrap();
    }
    v
}

fn seq_vec128() -> HVec<u8, 128> {
    let mut v = HVec::new();
    for i in 0u8..128 {
        v.push(i).unwrap();
    }
    v
}

#[test]
fn shim_crc32_zero_payload() {
    // All-zero 128-byte Vec → CRC-32/ISO-HDLC = 0xC2A8FA9D
    // (verified with Python: zlib.crc32(bytes(128)) & 0xFFFFFFFF)
    let payload = zero_vec128();
    let mut input_buf = [0u8; 200];
    let args = postcard::to_slice(&(payload,), &mut input_buf).unwrap();
    let mut out = [0u8; 8];
    let n = (__TELEPATH_CMD_CRC32.invoke)(args, &mut out).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 0xC2A8_FA9D, "CRC-32/ISO-HDLC over 128 zero bytes");
}

#[test]
fn shim_crc32_sequential_payload() {
    // Bytes 0..128 → CRC-32/ISO-HDLC = 0x24650D57
    // (verified with Python: zlib.crc32(bytes(range(128))) & 0xFFFFFFFF)
    let payload = seq_vec128();
    let mut input_buf = [0u8; 200];
    let args = postcard::to_slice(&(payload,), &mut input_buf).unwrap();
    let mut out = [0u8; 8];
    let n = (__TELEPATH_CMD_CRC32.invoke)(args, &mut out).unwrap();
    let val: u32 = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(val, 0x2465_0D57, "CRC-32/ISO-HDLC over bytes 0..128");
}

#[test]
fn shim_echo_round_trip() {
    let payload = seq_vec128();
    let mut input_buf = [0u8; 200];
    let args = postcard::to_slice(&(payload.clone(),), &mut input_buf).unwrap();
    let mut out = [0u8; 200];
    let n = (__TELEPATH_CMD_ECHO.invoke)(args, &mut out).unwrap();
    let returned: HVec<u8, 128> = postcard::from_bytes(&out[..n]).unwrap();
    assert_eq!(returned, payload);
}

// ---------------------------------------------------------------------------
// Original functions remain directly callable
// ---------------------------------------------------------------------------

#[test]
fn original_fns_callable() {
    assert_eq!(add(2, 3), 5);
    assert_eq!(add(-1, 1), 0);
    assert_eq!(crc32(zero_vec128()), 0xC2A8_FA9D);
    let seq = seq_vec128();
    assert_eq!(echo(seq.clone()), seq);
}
