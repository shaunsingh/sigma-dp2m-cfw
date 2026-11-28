/* M6M payload placement; MEMORY (regions.ld) is generated from `REGIONS` in crates/dp2m/src/m6m/mod.rs */

INCLUDE regions.ld

ENTRY(dp2m_ret0)

SECTIONS {
  .applet : {
    */doom.o(.text .text.*)
    */doom.o(.rodata .rodata.* .data .data.*)
  } > APPLET

  .lv_text : {
    *(.lv_text .lv_text.*)
  } > LVCODE

  .text : {
    KEEP(*(.text.dp2m_ret0))
    *(.text .text.*)
  } > CODE

  .menu : { *(.menu .menu.*) } > MENU

  .lv : { *(.lv .lv.*) } > LOOKS

  .data : {
    *(.rodata .rodata.*)
    *(.data .data.*)
  } > DATA

  .bss (NOLOAD) : {
    PROVIDE(__bss_start = .);
    *(.bss .bss.* COMMON)
    PROVIDE(__bss_end = .);
  } > RAM

  PROVIDE(__ram_start = ORIGIN(RAM));
  PROVIDE(__ram_end = ORIGIN(RAM) + LENGTH(RAM));

  .dp2m.hooks (INFO) : { KEEP(*(.dp2m.hooks)) }

  /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) *(.comment) *(.note*) }
}
