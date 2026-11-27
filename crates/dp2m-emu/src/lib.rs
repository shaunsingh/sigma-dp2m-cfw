//! Unified DP2 Merrill emulation, simulation, RE, and verification core.
//! Zero external deps so `dp2m-web` builds straight to `wasm32-unknown-unknown`.
#![forbid(unsafe_code)]

pub mod arm9;
pub mod fr80;
pub mod image;
pub mod machine;
pub mod re;
pub mod sim;
pub mod stock;
pub mod verify;
pub mod x3f;
