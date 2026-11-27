/* M6M payload placement; MEMORY (regions.ld) is generated from `REGIONS` in crates/dp2m/src/m6m/mod.rs */

INCLUDE regions.ld

ENTRY(dp2m_ret0)

SECTIONS {
  .doom_render : {
    */doom_render.o(.text .text.*)
    */probe.o(.probe.a .probe.a.*)
    */probe.o(.rodata.str1.*)
  } > DOOM_A

  .doom_init : {
    *(.doom.init .doom.init.*)
    */probe.o(.probe.e .probe.e.*)
    */probe.o(.probe.ro .probe.ro.*)
  } > DOOM_E

  .doom_hud : {
    *(.doom.hud .doom.hud.*)
    *(.lv_text .lv_text.*)
    */probe.o(.probe.d .probe.d.*)
  } > DOOM_D

  .doom_text : {
    */doom.o(.text .text.*)
    */probe.o(.text .text.*)
  } > DOOM_B

  .doom_data : {
    */doom.o(.rodata .rodata.* .data .data.*)
    */doom_render.o(.rodata .rodata.* .data .data.*)
    */probe.o(.rodata .rodata.* .data .data.*)
    *(.doom.extra .doom.extra.*)
  } > DOOM_C

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
