//! Error type shared by every layer, carrying the process exit code.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Bad command line. Exit code 2.
    Usage(String),
    /// Config file missing, malformed, or user not found. Exit code 5.
    Config(String),
    /// Could not reach or parse the server. Exit code 4.
    Network(String),
    /// The server answered but refused the action. Exit code 3.
    Rejected { code: String, message: String },
    /// Anything else. Exit code 1.
    Internal(String),
}

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Internal(_) => 1,
            Error::Usage(_) => 2,
            Error::Rejected { .. } => 3,
            Error::Network(_) => 4,
            Error::Config(_) => 5,
        }
    }

    pub fn usage(msg: impl Into<String>) -> Self {
        Error::Usage(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Error::Config(msg.into())
    }

    pub fn network(msg: impl Into<String>) -> Self {
        Error::Network(msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(msg.into())
    }

    pub fn rejected(code: impl Into<String>, message: impl Into<String>) -> Self {
        Error::Rejected {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) => write!(f, "{m}"),
            Error::Config(m) => write!(f, "config: {m}"),
            Error::Network(m) => write!(f, "network: {m}"),
            Error::Rejected { message, .. } => write!(f, "rejected: {message}"),
            Error::Internal(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Network(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Network(format!("bad json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
