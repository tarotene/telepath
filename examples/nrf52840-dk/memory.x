MEMORY
{
  /* nRF52840: 1 MiB Flash, 256 KiB RAM */
  FLASH    : ORIGIN = 0x00000000, LENGTH = 1024K
  /* Reserve 256 B at the head of RAM for the SEGGER RTT control block.
     The host-side CLI attaches to _SEGGER_RTT at this exact address via
     ScanRegion::Exact — see tools/telepath-cli/src/rtt_transport.rs. */
  RTT_CTRL : ORIGIN = 0x20000000, LENGTH = 0x100
  RAM      : ORIGIN = 0x20000100, LENGTH = 256K - 0x100
}

SECTIONS
{
  /* Place the RTT control block (section_cb: ".segger_rtt" in rtt_init!) at
     the start of RTT_CTRL so _SEGGER_RTT is always at 0x20000000. */
  .segger_rtt (NOLOAD) : ALIGN(4)
  {
    KEEP(*(.segger_rtt .segger_rtt.*))
  } > RTT_CTRL
} INSERT BEFORE .bss;

/* linkme distributed-slice placement: coerces linkme_* / linkm2_* orphan
   sections into FLASH so cortex-m-rt's .data init does not corrupt them.
   linkme.x is provided by telepath-firmware via its build.rs OUT_DIR. */
INCLUDE linkme.x;
