//! ABM Local AI hub core.
//!
//! Supervises one llama-server child (loopback only, random port and key) and ties its lifetime to
//! the hub on every OS; holds the model library, the hardware check that decides which models a
//! computer can run, and the verified pack downloader. The M1 API layer (pairing, per-app keys,
//! policy) builds on top of this crate.

pub mod catalog;
pub mod hardware;
pub mod http;
pub mod llama;
pub mod packs;
pub mod platform;
pub mod supervisor;

pub use llama::{LlamaConfig, LlamaServer};
pub use supervisor::{State, Supervisor};

#[derive(Debug, Clone)]
pub struct Error(String);

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}
