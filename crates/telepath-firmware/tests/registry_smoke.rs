// Integration test: verify that #[command]-annotated functions appear in the
// linkme-collected commands() slice.
//
// Each tests/*.rs compiles to a separate binary, so only the #[command] items
// defined here (alpha, beta) land in TELEPATH_COMMANDS for this binary.
// That isolation makes exact-count assertions safe.

use telepath_firmware::{command, commands};

// ---------------------------------------------------------------------------
// Commands under test
// ---------------------------------------------------------------------------

#[command]
fn alpha() -> u8 {
    1
}

#[command]
fn beta(x: u8) -> u8 {
    x.wrapping_add(1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn registry_includes_both_commands() {
    let names: Vec<&'static str> = commands().iter().map(|c| c.name).collect();
    assert!(names.contains(&"alpha"), "alpha must appear in commands()");
    assert!(names.contains(&"beta"), "beta must appear in commands()");
}

#[test]
fn registry_has_exactly_two_commands() {
    assert_eq!(
        commands().len(),
        2,
        "this binary defines exactly alpha + beta; \
         a linker-section regression would change the count"
    );
}

#[test]
fn registry_ids_unique_and_nonzero() {
    let ids: Vec<u16> = commands().iter().map(|c| c.id).collect();
    for &id in &ids {
        assert_ne!(id, 0x0000, "reserved CDP ID must not appear in registry");
    }
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "command IDs must be unique");
}

#[test]
fn registry_id_matches_macro_const() {
    // Verify that the linkme-registered entry and the macro-generated const agree.
    // A mismatch would indicate that the distributed_slice registration was
    // compiled against a different CommandMetadata than the one in the slice.
    let alpha_entry = commands()
        .iter()
        .find(|c| c.name == "alpha")
        .expect("alpha must be in commands()");
    assert_eq!(
        alpha_entry.id, __TELEPATH_CMD_ALPHA.id,
        "commands() id must match __TELEPATH_CMD_ALPHA.id"
    );

    let beta_entry = commands()
        .iter()
        .find(|c| c.name == "beta")
        .expect("beta must be in commands()");
    assert_eq!(
        beta_entry.id, __TELEPATH_CMD_BETA.id,
        "commands() id must match __TELEPATH_CMD_BETA.id"
    );
}
