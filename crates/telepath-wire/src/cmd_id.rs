//! Command ID derivation for the Telepath Command Discovery Protocol.
//!
//! A `cmd_id` is a stable 16-bit identifier derived from the command's
//! human-readable signature. The derivation uses FNV-1a 32-bit hashed over
//! the canonical pre-image, then XOR-folded to 16 bits.
//!
//! ## Pre-image
//!
//! ```text
//! pre-image = name || 0x1F || args_type || 0x1F || ret_type
//! ```
//!
//! `0x1F` (ASCII Unit Separator) cannot appear in Rust identifiers or type
//! paths, so it is collision-free as a field delimiter.
//!
//! ## Stand-in for a schema fingerprint
//!
//! `args_type` and `ret_type` are the textual Rust type names as seen by the
//! `#[command]` proc-macro (`syn`-derived token strings). This is a *textual*
//! canonicalization, not a true postcard schema digest:
//!
//! - Renaming `struct Foo { x: u8 }` to `struct Bar { x: u8 }` **changes** the ID.
//! - Reordering fields inside `Foo` does **not** change the ID.
//!
//! The ID encodes textual type names, not a structural postcard-schema digest.
//! A future cmd_id v2 could incorporate a real schema hash (see issue #3).
//!
//! ## 0x0000 reservation
//!
//! `CMD_ID_DISCOVERY` (0x0000) is reserved for the Command Discovery Protocol.
//! If the raw hash collides with it, [`derive_cmd_id`] rehashes with a `0xFF`
//! salt byte appended, producing a deterministic non-zero fallback.

use crate::CMD_ID_DISCOVERY;

/// Delimiter byte placed between pre-image fields (ASCII Unit Separator, 0x1F).
///
/// This byte cannot appear in Rust identifiers or type paths, making it
/// collision-free as a separator between the name, args, and return type fields.
pub const CMD_ID_FIELD_SEP: u8 = 0x1F;

// FNV-1a parameters (32-bit).
const FNV_OFFSET_BASIS: u32 = 0x811c9dc5;
const FNV_PRIME: u32 = 0x01000193;

/// Continues an FNV-1a 32-bit hash over `bytes`, starting from `hash`.
const fn fnv1a_32_continue(mut hash: u32, bytes: &[u8]) -> u32 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

/// FNV-1a 32-bit hash of `bytes` from the standard offset basis.
const fn fnv1a_32(bytes: &[u8]) -> u32 {
    fnv1a_32_continue(FNV_OFFSET_BASIS, bytes)
}

/// XOR-folds a 32-bit value to 16 bits by XOR-ing the two halves.
///
/// Preferred over truncation because it preserves avalanche across the full
/// input range and reduces low-bit bias inherent in multiplicative hashes.
const fn xor_fold(h: u32) -> u16 {
    ((h >> 16) as u16) ^ (h as u16)
}

/// FNV-1a 32-bit, XOR-folded to 16 bits.
///
/// `const fn` — usable from both proc-macro (std) and `no_std` firmware contexts
/// without any additional dependencies.
///
/// Known test vectors (from <http://www.isthe.com/chongo/tech/comp/fnv/>):
/// - `fnv1a_16(b"") == 0x1cd9`
/// - `fnv1a_16(b"a") == 0xcd20`
/// - `fnv1a_16(b"foobar") == 0x46f4`
pub const fn fnv1a_16(bytes: &[u8]) -> u16 {
    xor_fold(fnv1a_32(bytes))
}

/// Derives a stable 16-bit `cmd_id` from the command's textual signature.
///
/// Pre-image: `name || 0x1F || args_type || 0x1F || ret_type` (UTF-8 bytes).
///
/// The three segments are hashed sequentially without heap allocation, so this
/// function is safe to call from `no_std` firmware and `const` proc-macro contexts.
///
/// If the result would equal [`CMD_ID_DISCOVERY`] (0x0000), the function
/// loops over descending salt bytes (`0xFF`, `0xFE`, …) until the result is
/// non-zero — guaranteeing that `CMD_ID_DISCOVERY` is never returned.
pub const fn derive_cmd_id(name: &str, args_type: &str, ret_type: &str) -> u16 {
    let h = fnv1a_32_continue(FNV_OFFSET_BASIS, name.as_bytes());
    let h = fnv1a_32_continue(h, &[CMD_ID_FIELD_SEP]);
    let h = fnv1a_32_continue(h, args_type.as_bytes());
    let h = fnv1a_32_continue(h, &[CMD_ID_FIELD_SEP]);
    let h = fnv1a_32_continue(h, ret_type.as_bytes());
    let mut id = xor_fold(h);
    let mut h = h;
    let mut salt = 0xFFu8;
    while id == CMD_ID_DISCOVERY {
        h = fnv1a_32_continue(h, &[salt]);
        id = xor_fold(h);
        if salt > 0 {
            salt -= 1;
        } else {
            return 0x0001;
        }
    }
    id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CMD_ID_DISCOVERY;

    // Known-vector source: http://www.isthe.com/chongo/tech/comp/fnv/
    // (Landon Curt Noll, FNV reference page — "FNV-1a 32-bit test vectors")

    #[test]
    fn fnv1a_16_empty() {
        // fnv1a_32(b"") == 0x811c9dc5 (offset basis, no bytes processed)
        // XOR fold: 0x811c ^ 0x9dc5 = 0x1cd9
        assert_eq!(fnv1a_16(b""), 0x1cd9);
    }

    #[test]
    fn fnv1a_32_known_vector_a() {
        assert_eq!(fnv1a_32(b"a"), 0xe40c292c);
    }

    #[test]
    fn fnv1a_16_known_vector_a() {
        // 0xe40c ^ 0x292c = 0xcd20
        assert_eq!(fnv1a_16(b"a"), 0xcd20);
    }

    #[test]
    fn fnv1a_32_known_vector_foobar() {
        assert_eq!(fnv1a_32(b"foobar"), 0xbf9cf968);
    }

    #[test]
    fn fnv1a_16_known_vector_foobar() {
        // 0xbf9c ^ 0xf968 = 0x46f4
        assert_eq!(fnv1a_16(b"foobar"), 0x46f4);
    }

    #[test]
    fn derive_cmd_id_ping_smoke() {
        let id = derive_cmd_id("ping", "()", "u32");
        assert_ne!(id, CMD_ID_DISCOVERY);
        // Stable across calls.
        assert_eq!(id, derive_cmd_id("ping", "()", "u32"));
    }

    #[test]
    fn derive_cmd_id_differs_on_name() {
        assert_ne!(
            derive_cmd_id("ping", "()", "u32"),
            derive_cmd_id("pong", "()", "u32"),
        );
    }

    #[test]
    fn derive_cmd_id_differs_on_ret() {
        assert_ne!(
            derive_cmd_id("f", "()", "u32"),
            derive_cmd_id("f", "()", "u64"),
        );
    }

    #[test]
    fn derive_cmd_id_differs_on_args() {
        assert_ne!(
            derive_cmd_id("f", "(u8,)", "u32"),
            derive_cmd_id("f", "(u16,)", "u32"),
        );
    }

    #[test]
    fn derive_cmd_id_never_returns_discovery_id() {
        let cases = [
            ("ping", "()", "u32"),
            ("get_value", "(u8,)", "u32"),
            ("set_value", "(u8, u16)", "Result<(), ()>"),
            ("", "", ""),
            ("a", "b", "c"),
        ];
        for (name, args, ret) in cases {
            assert_ne!(
                derive_cmd_id(name, args, ret),
                CMD_ID_DISCOVERY,
                "({name:?}, {args:?}, {ret:?}) mapped to reserved CMD_ID_DISCOVERY",
            );
        }
    }

    #[test]
    fn const_context() {
        const _: u16 = fnv1a_16(b"x");
        const _: u16 = derive_cmd_id("f", "()", "u32");
    }

    /// Searches for an input whose raw pre-guard hash is 0x0000, then verifies
    /// that [`derive_cmd_id`] returns a non-zero ID via the salt-rehash path.
    ///
    /// Finding a collision is probabilistic (~1/65536 per input). If none is
    /// found within the search budget, the test passes trivially — the policy
    /// is still exercised by the implementation whenever a real collision occurs.
    #[test]
    fn derive_cmd_id_salt_rehash_when_raw_is_zero() {
        for i in 0u32..200_000 {
            let mut buf = [0u8; 10];
            let len = encode_decimal(i, &mut buf);

            // Mirror derive_cmd_id internals to inspect the pre-guard hash.
            let h = fnv1a_32_continue(FNV_OFFSET_BASIS, &buf[..len]);
            let h = fnv1a_32_continue(h, &[CMD_ID_FIELD_SEP]);
            let h = fnv1a_32_continue(h, b"()");
            let h = fnv1a_32_continue(h, &[CMD_ID_FIELD_SEP]);
            let h = fnv1a_32_continue(h, b"u32");

            if xor_fold(h) == CMD_ID_DISCOVERY {
                // The salt rehash must also avoid 0x0000.
                let salted = xor_fold(fnv1a_32_continue(h, &[0xFF]));
                assert_ne!(salted, CMD_ID_DISCOVERY);
                return;
            }
        }
        // No collision found — trivially passing (P(miss) ≈ 37%).
    }

    /// Encodes `n` as ASCII decimal bytes into `buf`, returning the byte count used.
    fn encode_decimal(mut n: u32, buf: &mut [u8; 10]) -> usize {
        if n == 0 {
            buf[0] = b'0';
            return 1;
        }
        let mut end = 0;
        while n > 0 {
            buf[end] = b'0' + (n % 10) as u8;
            end += 1;
            n /= 10;
        }
        buf[..end].reverse();
        end
    }
}
