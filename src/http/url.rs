//! Minimal URL handling: `scheme://host[:port][/path[?query]]`, IPv6 literals
//! in brackets, relative `Location` resolution and query-string helpers.

use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// Path plus optional `?query`, always starting with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url> {
        let (scheme, rest) = s
            .split_once("://")
            .ok_or_else(|| Error::usage(format!("url without scheme: {s}")))?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(Error::usage(format!("unsupported scheme: {scheme}")));
        }
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = if let Some(after) = authority.strip_prefix('[') {
            let end = after
                .find(']')
                .ok_or_else(|| Error::usage(format!("bad ipv6 host in {s}")))?;
            let host = &after[..end];
            let port = after[end + 1..].strip_prefix(':');
            (host.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), Some(p)),
                None => (authority.to_string(), None),
            }
        };
        if host.is_empty() {
            return Err(Error::usage(format!("url without host: {s}")));
        }
        let port = match port {
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| Error::usage(format!("bad port in {s}")))?,
            None if scheme == "https" => 443,
            None => 80,
        };
        Ok(Url {
            scheme,
            host,
            port,
            path: path.to_string(),
        })
    }

    pub fn is_default_port(&self) -> bool {
        (self.scheme == "http" && self.port == 80) || (self.scheme == "https" && self.port == 443)
    }

    pub fn host_header(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.is_default_port() {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// `scheme://host[:port]` without a trailing slash.
    pub fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.host_header())
    }

    pub fn with_path(&self, path: &str) -> Url {
        let mut u = self.clone();
        u.path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        u
    }

    /// Resolve a `Location` header (absolute, absolute-path, or relative).
    pub fn join(&self, location: &str) -> Result<Url> {
        if location.contains("://") {
            return Url::parse(location);
        }
        if let Some(rest) = location.strip_prefix("//") {
            return Url::parse(&format!("{}://{rest}", self.scheme));
        }
        if location.starts_with('/') {
            return Ok(self.with_path(location));
        }
        let base = match self.path.split_once('?') {
            Some((p, _)) => p,
            None => self.path.as_str(),
        };
        let dir = match base.rfind('/') {
            Some(i) => &base[..=i],
            None => "/",
        };
        Ok(self.with_path(&format!("{dir}{location}")))
    }

    pub fn query_param(&self, key: &str) -> Option<String> {
        let (_, q) = self.path.split_once('?')?;
        for pair in q.split('&') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if k == key {
                return Some(percent_decode(v));
            }
        }
        None
    }
}

impl std::fmt::Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.origin(), self.path)
    }
}

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~')
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `k=v&k2=v2` with both sides percent-encoded.
pub fn build_query(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variants() {
        let u = Url::parse("http://10.0.0.55").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("10.0.0.55", 80, "/")
        );
        let u = Url::parse("https://auth.example.edu:8443/x?a=1").unwrap();
        assert_eq!(u.port, 8443);
        assert_eq!(u.host_header(), "auth.example.edu:8443");
        let u = Url::parse("http://[fe80::1]:8080/p").unwrap();
        assert_eq!(u.host, "fe80::1");
        assert_eq!(u.host_header(), "[fe80::1]:8080");
        assert!(Url::parse("ftp://x").is_err());
        assert!(Url::parse("10.0.0.55").is_err());
    }

    #[test]
    fn join_and_query() {
        let base = Url::parse("http://10.0.0.55/index_1.html").unwrap();
        let abs = base
            .join("http://10.0.0.55/srun_portal_pc?ac_id=8&theme=bit")
            .unwrap();
        assert_eq!(abs.query_param("ac_id").as_deref(), Some("8"));
        assert_eq!(base.join("/a/b").unwrap().path, "/a/b");
        let pr = base.join("//other.host:8080/p?q=1").unwrap();
        assert_eq!(
            (pr.host.as_str(), pr.port, pr.path.as_str()),
            ("other.host", 8080, "/p?q=1")
        );
        assert_eq!(base.join("c").unwrap().path, "/c");
        let deep = Url::parse("http://h/a/b/c?x=1").unwrap();
        assert_eq!(deep.join("d").unwrap().path, "/a/b/d");
    }

    #[test]
    fn encoding() {
        assert_eq!(
            percent_encode("{MD5}a+b/c=d e"),
            "%7BMD5%7Da%2Bb%2Fc%3Dd%20e"
        );
        assert_eq!(
            build_query(&[("a", "1 2"), ("b", "x&y")]),
            "a=1%202&b=x%26y"
        );
        assert_eq!(percent_decode("a%20b+c%ZZ"), "a b c%ZZ");
        assert_eq!(percent_decode("x%\u{4e2d}y%"), "x%\u{4e2d}y%");
    }
}
