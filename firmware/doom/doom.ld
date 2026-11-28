/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Linker script for standalone SD-loaded DOOM.BIN applet at 0xA1CC0000..0xA1E00000 */
ENTRY(dp2m_doom_entry)

MEMORY {
  APP_RAM (rwx) : ORIGIN = 0xA1CC0000, LENGTH = 0x00140000
}

SECTIONS {
  .header 0xA1CC0000 : {
    KEEP(*(.dp2m.hdr))
  } > APP_RAM

  .text : ALIGN(4) {
    *(.text .text.*)
  } > APP_RAM

  .rodata : ALIGN(4) {
    *(.rodata .rodata.*)
  } > APP_RAM

  .data : ALIGN(8) {
    *(.data .data.*)
  } > APP_RAM

  . = ALIGN(16);
  PROVIDE(__image_end = .);

  .bss (NOLOAD) : ALIGN(16) {
    PROVIDE(__bss_start = .);
    *(.bss .bss.* COMMON)
    . = ALIGN(16);
    PROVIDE(__bss_end = .);
  } > APP_RAM

  ASSERT(__image_end - 0xA1CC0000 >= 32, "missing D2AP header")
  ASSERT(__bss_end <= 0xA1E00000, "DOOM.BIN + .bss exceeds 0xA1E00000")

  /DISCARD/ : {
    *(.ARM.exidx*)
    *(.ARM.extab*)
    *(.comment)
    *(.note*)
  }
}
