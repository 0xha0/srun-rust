//! Interface enumeration via `if-addrs`.

use std::net::IpAddr;

/// `(interface name, ip)` for every non-loopback address.
pub fn list() -> Vec<(String, IpAddr)> {
    match if_addrs::get_if_addrs() {
        Ok(ifs) => ifs
            .into_iter()
            .filter(|i| !i.is_loopback())
            .map(|i| (i.name.clone(), i.ip()))
            .collect(),
        Err(e) => {
            crate::log_warn!("cannot list interfaces: {e}");
            Vec::new()
        }
    }
}

/// First IPv4 of the interface whose name contains `needle`.
pub fn ipv4_by_name(needle: &str) -> Option<IpAddr> {
    list()
        .into_iter()
        .find(|(name, ip)| name.contains(needle) && ip.is_ipv4())
        .map(|(_, ip)| ip)
}
