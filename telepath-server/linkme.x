/* Telepath linkme distributed-slice placement for Cortex-M / bare-metal ELF targets.
 *
 * MUST be included from the consuming firmware's memory.x via:
 *   INCLUDE linkme.x;
 *
 * Without this, rust-lld places `linkme_*` orphan sections in RAM (no flash
 * backing). cortex-m-rt's .data init then copies unrelated flash bytes into
 * those addresses, so the resulting CommandMetadata entries contain garbage —
 * every RPC dispatch fails with SystemError.
 *
 * Both the slice section (linkme_*) and the duplicate-check helper (linkm2_*)
 * must be placed; missing either can cause undefined-symbol errors or wrong
 * section overlap.
 *
 * Reference: https://github.com/dtolnay/linkme/blob/master/tests/cortex/memory.x
 *
 * Maintenance: add one line per new #[distributed_slice] static in this crate:
 *   linkme_<NAME>  : { *( linkme_<NAME>) } > FLASH
 *   linkm2_<NAME>  : { *(linkm2_<NAME>) } > FLASH
 */
SECTIONS
{
  linkme_TELEPATH_COMMANDS  : { *(linkme_TELEPATH_COMMANDS)  } > FLASH
  linkm2_TELEPATH_COMMANDS  : { *(linkm2_TELEPATH_COMMANDS)  } > FLASH
} INSERT AFTER .rodata;
