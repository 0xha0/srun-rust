//! "Am I online?" checks shared by `login --test` and the daemon.

use crate::error::{Error, Result};
use crate::protocol::Client;
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

pub fn tcp_reachable(target: &str, timeout: Duration) -> bool {
    let Ok(addrs) = target.to_socket_addrs() else {
        return false;
    };
    for a in addrs {
        if TcpStream::connect_timeout(&a, timeout).is_ok() {
            return true;
        }
    }
    false
}

/// `Ok(true)` when online. Network failures are reported as errors for the
/// server probe (the caller decides) and as offline for the tcp probe.
pub fn is_online(client: &Client, probe: &Probe) -> Result<bool> {
    match probe {
        Probe::Server => Ok(client.status()?.is_online()),
        Probe::Tcp(t) => Ok(tcp_reachable(t, client.opts.connect_timeout)),
        Probe::None => Ok(false),
    }
}
