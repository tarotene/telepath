extern crate std;

use std::cell::RefCell;
use std::rc::Rc;
use std::vec::Vec;

use telepath_server::{command, commands, transport, TelepathServer};
use telepath_wire::framing::{cobs_decode, cobs_encode};
use telepath_wire::{
    DiscoveryEntry, DiscoveryPage, PacketType, Request, Response, ResponseStatus, CMD_ID_DISCOVERY,
};

// ---------------------------------------------------------------------------
// Commands under test  (this binary's TELEPATH_COMMANDS slice = foo + bar only)
// ---------------------------------------------------------------------------

#[command]
fn foo() -> u8 {
    0
}

#[command]
fn bar() -> u8 {
    1
}

// ---------------------------------------------------------------------------
// Test transport: tx is shared via Rc<RefCell> so the test can read it after
// TelepathServer takes ownership of the transport.
// ---------------------------------------------------------------------------

struct LoopbackTransport {
    rx: Vec<u8>,
    tx: Rc<RefCell<Vec<u8>>>,
}

impl transport::Transport for LoopbackTransport {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        if self.rx.is_empty() {
            return 0;
        }
        let n = buf.len().min(self.rx.len());
        buf[..n].copy_from_slice(&self.rx[..n]);
        self.rx.drain(..n);
        n
    }

    fn write(&mut self, buf: &[u8]) -> usize {
        self.tx.borrow_mut().extend_from_slice(buf);
        buf.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn discovery_returns_registered_commands() {
    // ── Build a COBS-framed CDP request ───────────────────────────────────
    let req = Request {
        kind: PacketType::Request,
        seq_no: 7,
        cmd_id: CMD_ID_DISCOVERY,
        args: &[],
    };
    let mut ser_buf = [0u8; 64];
    let n = postcard::to_slice(&req, &mut ser_buf).unwrap().len();
    let mut frame_buf = [0u8; 64];
    let m = cobs_encode(&ser_buf[..n], &mut frame_buf).unwrap();

    // ── Drive TelepathServer::poll() ──────────────────────────────────────
    let tx_shared = Rc::new(RefCell::new(Vec::new()));
    let transport = LoopbackTransport {
        rx: frame_buf[..m].to_vec(),
        tx: tx_shared.clone(),
    };
    let mut server = TelepathServer::<_, 512>::new(transport, commands());
    server.poll();

    // ── COBS-decode the response frame ────────────────────────────────────
    let tx = tx_shared.borrow();
    let delim = tx
        .iter()
        .position(|&b| b == 0x00)
        .expect("no frame delimiter");
    let mut decoded = [0u8; 512];
    let dl = cobs_decode(&tx[..delim], &mut decoded).unwrap();
    let resp: Response<'_> = postcard::from_bytes(&decoded[..dl]).unwrap();

    assert_eq!(resp.seq_no, 7);
    assert_eq!(resp.status, ResponseStatus::Ok, "CDP must succeed");

    // ── Decode DiscoveryPage and its embedded entry sequence ──────────────
    let page: DiscoveryPage<'_> = postcard::from_bytes(resp.payload).unwrap();
    assert_eq!(
        page.total as usize,
        commands().len(),
        "page.total must equal the total registered command count"
    );
    assert_eq!(page.offset, 0, "first page must start at offset 0");

    let (count, mut rest): (u32, &[u8]) = postcard::take_from_bytes(page.entries).unwrap();
    assert_eq!(
        count as usize,
        commands().len(),
        "entry count must match total for single-page registration"
    );
    assert!(count >= 2, "expected at least foo + bar");

    let mut names: Vec<&str> = Vec::new();
    let mut ids: Vec<u16> = Vec::new();
    for _ in 0..count {
        let (entry, next): (DiscoveryEntry<'_>, &[u8]) = postcard::take_from_bytes(rest).unwrap();
        // Schema fingerprints must be populated by the #[command] macro.
        assert!(
            !entry.args_schema.is_empty(),
            "{} args_schema must be non-empty",
            entry.name
        );
        assert!(
            !entry.ret_schema.is_empty(),
            "{} ret_schema must be non-empty",
            entry.name
        );
        names.push(entry.name);
        ids.push(entry.id);
        rest = next;
    }

    assert!(names.contains(&"foo"), "foo must appear in discovery");
    assert!(names.contains(&"bar"), "bar must appear in discovery");

    // Reserved CDP ID 0x0000 must not appear in any entry.
    for &id in &ids {
        assert_ne!(id, 0x0000, "discovery must not expose the reserved CDP ID");
    }

    // All IDs must be unique.
    let mut sorted_ids = ids.clone();
    sorted_ids.sort_unstable();
    sorted_ids.dedup();
    assert_eq!(sorted_ids.len(), ids.len(), "command IDs must be unique");
}
