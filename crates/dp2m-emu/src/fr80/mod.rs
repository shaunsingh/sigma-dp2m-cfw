//! Fujitsu FR80 tooling for M5 firmware.

pub mod asm;
pub mod dis;
pub mod emu;

pub use asm::{assemble_file, assemble_file_labels, assemble_sections};
pub use dis::dis_range;
pub use emu::{Emulator, Handler, RETURN};
