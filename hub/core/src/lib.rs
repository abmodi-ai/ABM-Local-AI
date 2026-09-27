//! ABM Local AI hub core.
//!
//! M0 scope: supervise one llama-server child (loopback only, random port and key), tie its
//! lifetime to the hub on every OS, and expose just enough to prove packaging and performance.
//! The M1 API layer (pairing, per-app keys, policy) builds on top of this crate.

pub mod http;
pub mod llama;
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
