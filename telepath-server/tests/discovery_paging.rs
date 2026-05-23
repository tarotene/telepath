extern crate std;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::vec::Vec;

use telepath_server::{command, commands, transport, TelepathServer};
use telepath_wire::framing::{cobs_decode, cobs_encode};
use telepath_wire::{
    DiscoveryEntry, DiscoveryPage, DiscoveryRequest, PacketType, Request, Response, ResponseStatus,
    CMD_ID_DISCOVERY,
};

// ---------------------------------------------------------------------------
// Register enough commands to exceed MAX_PAYLOAD_SIZE in a single response.
// Each entry has args_schema + ret_schema bytes from postcard-schema; the
// exact per-entry size is determined at test runtime. Registering 20 commands
// ensures at least 2 pages under any plausible schema encoding.
// ---------------------------------------------------------------------------

macro_rules! declare_commands {
    ($($name:ident),+) => {
        $(
            #[command]
            fn $name() -> u32 { 0 }
        )+
    };
}

declare_commands!(
    cmd_00, cmd_01, cmd_02, cmd_03, cmd_04, cmd_05, cmd_06, cmd_07, cmd_08, cmd_09, cmd_10, cmd_11,
    cmd_12, cmd_13, cmd_14, cmd_15, cmd_16, cmd_17, cmd_18, cmd_19
);

// ---------------------------------------------------------------------------
// Transport helpers
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

fn decode_page_owned(raw_tx: &[u8]) -> (u16, u16, Vec<(u16, std::string::String)>) {
    let delim = raw_tx
        .iter()
        .position(|&b| b == 0x00)
        .expect("no frame delimiter");
    let mut decoded = [0u8; 512];
    let dl = cobs_decode(&raw_tx[..delim], &mut decoded).unwrap();
    let resp: Response<'_> = postcard::from_bytes(&decoded[..dl]).unwrap();
    assert_eq!(
        resp.status,
        ResponseStatus::Ok,
        "discovery page must succeed"
    );
    let page: DiscoveryPage<'_> = postcard::from_bytes(resp.payload).unwrap();
    let total = page.total;
    let offset = page.offset;
    let (count, mut rest): (u32, &[u8]) = postcard::take_from_bytes(page.entries).unwrap();
    let mut entries = Vec::new();
    for _ in 0..count {
        let (entry, next): (DiscoveryEntry<'_>, &[u8]) = postcard::take_from_bytes(rest).unwrap();
        assert!(
            !entry.args_schema.is_empty(),
            "args_schema must be non-empty"
        );
        assert!(!entry.ret_schema.is_empty(), "ret_schema must be non-empty");
        entries.push((entry.id, entry.name.to_owned()));
        rest = next;
    }
    (total, offset, entries)
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test]
fn discovery_paging_covers_all_commands() {
    let total_registered = commands().len() as u16;
    assert!(
        total_registered >= 20,
        "expected at least 20 registered commands, got {}",
        total_registered
    );

    let mut all_names: HashSet<std::string::String> = HashSet::new();
    let mut all_ids: HashSet<u16> = HashSet::new();
    let mut offset = 0u16;
    let mut page_count = 0u32;
    let mut seen_total: Option<u16> = None;

    loop {
        // Build request payload with current offset.
        let dreq = DiscoveryRequest { offset };
        let mut req_payload_buf = [0u8; 16];
        let req_payload = postcard::to_slice(&dreq, &mut req_payload_buf)
            .unwrap()
            .to_vec();

        let req = Request {
            kind: PacketType::Request,
            seq_no: page_count as u16,
            cmd_id: CMD_ID_DISCOVERY,
            args: &req_payload,
        };
        let mut ser_buf = [0u8; 128];
        let n = postcard::to_slice(&req, &mut ser_buf).unwrap().len();
        let mut frame_buf = [0u8; 256];
        let m = cobs_encode(&ser_buf[..n], &mut frame_buf).unwrap();

        let tx_out = Rc::new(RefCell::new(Vec::new()));
        let transport = LoopbackTransport {
            rx: frame_buf[..m].to_vec(),
            tx: tx_out.clone(),
        };
        let mut server = TelepathServer::<_, 512>::new(transport, commands());
        server.poll();

        let raw_tx = tx_out.borrow().clone();
        let (total, page_offset, entries) = decode_page_owned(&raw_tx);

        assert_eq!(page_offset, offset, "server must echo the requested offset");

        // Verify total is consistent across pages.
        match seen_total {
            None => seen_total = Some(total),
            Some(prev) => assert_eq!(total, prev, "total must be identical on all pages"),
        }

        let page_entry_count = entries.len() as u16;

        for (id, name) in entries {
            assert_ne!(id, CMD_ID_DISCOVERY, "CDP ID must not be exposed");
            all_names.insert(name);
            all_ids.insert(id);
        }

        offset = offset.saturating_add(page_entry_count);
        page_count += 1;

        if offset >= total || page_entry_count == 0 {
            break;
        }

        // Guard against runaway loops.
        assert!(
            page_count <= total as u32 + 2,
            "too many pages; possible infinite loop"
        );
    }

    let total_seen = seen_total.unwrap();
    assert_eq!(
        total_seen, total_registered,
        "page.total must match total registered commands"
    );
    assert_eq!(
        all_ids.len(),
        total_registered as usize,
        "all command IDs must be unique and fully covered"
    );
    assert!(
        page_count >= 2,
        "with 20 commands and schemas, discovery should require at least 2 pages"
    );
}
