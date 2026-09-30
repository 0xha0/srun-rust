//! "Am I online?" checks shared by `login --test` and the daemon.

use crate::error::{Error, Result};
use crate::protocol::{Client, StatusResp};
use std::fmt;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// Ask the portal (`rad_user_info`). Works without DNS or internet.
    Server,
    /// TCP connect to `host:port`.
    Tcp(String),
    /// Never probe; always assume offline.
    None,
}

impl Probe {
    pub fn parse(s: &str) -> Result<Probe> {
        match s {
            "" | "server" => Ok(Probe::Server),
            "none" => Ok(Probe::None),
            other if other.contains(':') => Ok(Probe::Tcp(other.to_string())),
            other => Err(Error::usage(format!(
                "probe must be server, none or HOST:PORT, got {other}"
            ))),
        }
    }
}

impl fmt::Display for Probe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Probe::Server => f.write_str("server"),
            Probe::None => f.write_str("none"),
            Probe::Tcp(t) => f.write_str(t),
        }
    }
}

pub enum Online {
    /// The portal reports a session on this address.
    Portal(Box<StatusResp>),
    /// The tcp target answered.
    Tcp,
    Offline,
}

impl Online {
    pub fn is_online(&self) -> bool {
        !matches!(self, Online::Offline)
    }
}

pub fn tcp_reachable(target: &str, timeout: Duration) -> bool {
    let Ok(addrs) = target.to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
}

/// Network failures are errors for the server probe (the caller decides
/// whether to keep going) and simply "offline" for the tcp probe.
pub fn check(client: &Client, probe: &Probe) -> Result<Online> {
    match probe {
        Probe::Server => {
            let st = client.status()?;
            Ok(if st.is_online() {
                Online::Portal(Box::new(st))
            } else {
                Online::Offline
            })
        }
        Probe::Tcp(t) => Ok(if tcp_reachable(t, client.opts.connect_timeout) {
            Online::Tcp
        } else {
            Online::Offline
        }),
        Probe::None => Ok(Online::Offline),
    }
}
