//! srun: a small client for the Srun campus-network authentication portal.

pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod http;
pub mod log;
pub mod net;
pub mod obf;
pub mod probe;
pub mod protocol;
pub mod term;
pub mod text;

pub use error::{Error, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const TARGET: &str = env!("SRUN_TARGET");
