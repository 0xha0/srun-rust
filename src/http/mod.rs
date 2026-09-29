//! A deliberately small HTTP/1.1 client: one GET per connection,
//! `Connection: close`, Content-Length / chunked / read-to-EOF bodies,
//! no redirect following (callers decide). TLS is layered in behind the
//! `Stream` trait when the `tls` feature is on.

#[cfg(feature = "tls")]
pub mod tls;
pub mod url;

use crate::error::{Error, Result};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
pub use url::{build_query, Url};

pub const MAX_BODY: usize = 64 * 1024;

/// Socket timeouts surface as `TimedOut` on Linux/Windows but `WouldBlock`
/// (EAGAIN) on macOS and the BSDs; give both the same readable text.
fn io_msg(what: &str, e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => format!("{what}: timed out"),
        _ => format!("{what}: {e}"),
    }
}

pub trait Stream: Read + Write {}
impl Stream for TcpStream {}

#[derive(Clone, Debug)]
pub struct ConnOpts {
    pub bind_ip: Option<IpAddr>,
    pub connect_timeout: Duration,
    pub io_timeout: Duration,
    pub tls_insecure: bool,
}

impl Default for ConnOpts {
    fn default() -> Self {
        ConnOpts {
            bind_ip: None,
            connect_timeout: Duration::from_secs(5),
            io_timeout: Duration::from_secs(10),
            tls_insecure: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn resolve(url: &Url) -> Result<Vec<SocketAddr>> {
    if let Ok(ip) = url.host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, url.port)]);
    }
    let addrs: Vec<SocketAddr> = (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| Error::network(format!("resolve {}: {e}", url.host)))?
        .collect();
    if addrs.is_empty() {
        return Err(Error::network(format!("resolve {}: no address", url.host)));
    }
    Ok(addrs)
}

fn tcp_connect(url: &Url, opts: &ConnOpts) -> Result<TcpStream> {
    let mut last: Option<io::Error> = None;
    for addr in resolve(url)? {
        let attempt = match opts.bind_ip {
            Some(local) => crate::net::bind::connect_bound(addr, local, opts.connect_timeout),
            None => TcpStream::connect_timeout(&addr, opts.connect_timeout),
        };
        match attempt {
            Ok(s) => {
                s.set_read_timeout(Some(opts.io_timeout))?;
                s.set_write_timeout(Some(opts.io_timeout))?;
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => {
                crate::log_debug!("connect {addr}: {e}");
                last = Some(e);
            }
        }
    }
    Err(Error::network(format!(
        "connect {}: {}",
        url.host_header(),
        last.map(|e| e.to_string()).unwrap_or_default()
    )))
}

fn open(url: &Url, opts: &ConnOpts) -> Result<Box<dyn Stream>> {
    let tcp = tcp_connect(url, opts)?;
    if url.scheme == "https" {
        #[cfg(feature = "tls")]
        {
            return tls::wrap(tcp, &url.host, opts.tls_insecure);
        }
        #[cfg(not(feature = "tls"))]
        {
            let _ = tcp;
            return Err(Error::usage(
                "https requested but this build has no tls support (rebuild with --features tls)",
            ));
        }
    }
    Ok(Box::new(tcp))
}

/// Perform one GET. `url.path` must already contain the query string.
pub fn get(url: &Url, opts: &ConnOpts) -> Result<Response> {
    let mut stream = open(url, opts)?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: srun/{}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        url.path,
        url.host_header(),
        crate::VERSION
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| Error::network(io_msg("send request", &e)))?;
    stream.flush().ok();
    let resp = read_response(&mut *stream)?;
    crate::log_trace!("status {} body {} bytes", resp.status, resp.body.len());
    Ok(resp)
}

fn read_line(r: &mut dyn BufRead) -> Result<String> {
    let mut line = String::new();
    let n = r
        .read_line(&mut line)
        .map_err(|e| Error::network(io_msg("read response", &e)))?;
    if n == 0 {
        return Err(Error::network("connection closed before response"));
    }
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

pub(crate) fn read_response(stream: &mut dyn Stream) -> Result<Response> {
    let mut r = BufReader::new(stream);
    let status_line = read_line(&mut r)?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(Error::network(format!("not http: {status_line}")));
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::network(format!("bad status line: {status_line}")))?;
    let mut headers = Vec::new();
    loop {
        let line = read_line(&mut r)?;
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut resp = Response {
        status,
        headers,
        body: Vec::new(),
    };
    let chunked = resp
        .header("transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    let length = resp
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok());
    if chunked {
        resp.body = read_chunked(&mut r)?;
    } else if let Some(n) = length {
        if n > MAX_BODY {
            return Err(Error::network(format!("response too large: {n} bytes")));
        }
        let mut buf = vec![0u8; n];
        r.read_exact(&mut buf)
            .map_err(|e| Error::network(io_msg("read body", &e)))?;
        resp.body = buf;
    } else {
        let mut buf = Vec::new();
        r.take(MAX_BODY as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| Error::network(io_msg("read body", &e)))?;
        if buf.len() > MAX_BODY {
            return Err(Error::network("response too large"));
        }
        resp.body = buf;
    }
    Ok(resp)
}

fn read_chunked(r: &mut dyn BufRead) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let size_line = read_line(r)?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| Error::network(format!("bad chunk size: {size_line}")))?;
        if size == 0 {
            // drain trailers
            loop {
                let t = read_line(r)?;
                if t.is_empty() {
                    break;
                }
            }
            return Ok(body);
        }
        if body.len() + size > MAX_BODY {
            return Err(Error::network("response too large"));
        }
        let mut chunk = vec![0u8; size];
        r.read_exact(&mut chunk)
            .map_err(|e| Error::network(io_msg("read chunk", &e)))?;
        body.extend_from_slice(&chunk);
        let _ = read_line(r)?; // CRLF after chunk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Mem(Cursor<Vec<u8>>);
    impl Read for Mem {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Mem {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Stream for Mem {}

    fn parse(raw: &str) -> Response {
        let mut m = Mem(Cursor::new(raw.as_bytes().to_vec()));
        read_response(&mut m).unwrap()
    }

    #[test]
    fn content_length() {
        let r =
            parse("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello");
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-type"), Some("text/html"));
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn chunked() {
        let r = parse("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;ext\r\nde\r\n0\r\nX-Trailer: 1\r\n\r\n");
        assert_eq!(r.body, b"abcde");
    }

    #[test]
    fn eof_body_and_http10() {
        let r = parse("HTTP/1.0 302 Found\r\nLocation: /x\r\n\r\nrest of body");
        assert_eq!(r.status, 302);
        assert_eq!(r.header("LOCATION"), Some("/x"));
        assert_eq!(r.body, b"rest of body");
    }

    #[test]
    fn garbage_is_error() {
        let mut m = Mem(Cursor::new(b"<html>".to_vec()));
        assert!(read_response(&mut m).is_err());
    }
}
