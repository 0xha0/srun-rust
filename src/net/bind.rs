//! Connect with the local address pinned to one interface (`strict_bind`).

use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::io;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

pub fn connect_bound(addr: SocketAddr, local: IpAddr, timeout: Duration) -> io::Result<TcpStream> {
    if addr.is_ipv4() != local.is_ipv4() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bind address {local} and target {addr} are different ip families"),
        ));
    }
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    socket.bind(&SockAddr::from(SocketAddr::new(local, 0)))?;
    socket.connect_timeout(&SockAddr::from(addr), timeout)?;
    Ok(socket.into())
}
