//! Device-side TTS runtime for OpenVML Studio.

pub mod audio;
pub mod cache;
pub mod config;
pub mod discovery;
pub mod edge;
pub mod engine;
pub mod piper;
pub mod sentence;
pub mod server;
pub mod voice;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SCHEMA_VERSION: u8 = 1;
