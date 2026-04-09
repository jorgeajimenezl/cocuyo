#![cfg(target_os = "windows")]

pub mod capture_target;
pub mod dx12_import;
pub mod held_frame;
pub mod recording;

pub use held_frame::HeldFrame;
