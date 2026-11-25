/* M6M payload placement; MEMORY (regions.ld) is generated from `REGIONS` in crates/dp2m/src/m6m/mod.rs */

INCLUDE regions.ld

ENTRY(dp2m_ret0)

SECTIONS {
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

  .bss (NOLOAD) : { *(.bss .bss.* COMMON) } > RAM

  .dp2m.hooks (INFO) : { KEEP(*(.dp2m.hooks)) }

  /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) *(.comment) *(.note*) }
}
