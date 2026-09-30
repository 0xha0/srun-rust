//! HMAC-MD5 password digest and SHA1 checksum, exactly as the portal JS does.

use hmac::{Hmac, Mac};
use md5::Md5;
use sha1::{Digest, Sha1};

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// `md5(password, token)` in the portal JS: HMAC-MD5 keyed by the token, lowercase hex.
pub fn hmd5(password: &str, token: &str) -> String {
    let mut mac = Hmac::<Md5>::new_from_slice(token.as_bytes()).expect("hmac accepts any key size");
    mac.update(password.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// SHA1 over `["", username, hmd5, acid, ip, n, type, info]` joined by the token.
#[allow(clippy::too_many_arguments)]
pub fn chksum(
    token: &str,
    username: &str,
    hmd5: &str,
    acid: &str,
    ip: &str,
    n: &str,
    utype: &str,
    info: &str,
) -> String {
    let parts = ["", username, hmd5, acid, ip, n, utype, info];
    let joined = parts.join(token);
    let mut h = Sha1::new();
    h.update(joined.as_bytes());
    hex(&h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_md5_known_answer() {
        assert_eq!(
            hmd5("The quick brown fox jumps over the lazy dog", "key"),
            "80070713463e7749b90c2dc24911e275"
        );
    }
}
