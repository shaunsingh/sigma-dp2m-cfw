//! fujitsu fr60/fr80/fr81 BE assembler, disassembler, & emulator for dp2m m5slave

pub mod asm;
pub mod dis;
pub mod emu;

pub use asm::{assemble_file, assemble_sections, assemble_str, AsmError};
pub use dis::{dis_one, dis_range};
pub use emu::{Emulator, Handler};
