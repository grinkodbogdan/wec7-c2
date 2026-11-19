//! Architecture support. Only arm64 exists today; a second architecture
//! adds a sibling module exposing the same items.

mod aarch64;

pub use aarch64::*;
