//! Password obfuscation for the config file. This is not encryption: the
//! binary can always undo it, and so can anyone who reads this file. It only
//! keeps the password out of plain sight (backups, screenshots, shoulder
//! surfing) and ties the stored form to the username.
//!
//! Format: `obf1:` + shuffled-alphabet base64( plaintext XOR keystream ),
//! keystream = HMAC-MD5(key = username, msg = "srun-obf-1" || counter).

use crate::error::{Error, Result};
use crate::protocol::b64;
use hmac::{Hmac, Mac};
use md5::Md5;

pub const PREFIX: &str = "obf1:";
const LABEL: &[u8] = b"srun-obf-1";

pub fn is_obfuscated(s: &str) -> bool {
    s.starts_with(PREFIX)
}

fn keystream(username: &str, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 16);
    let mut counter: u32 = 0;
    while out.len() < len {
        let mut mac = Hmac::<Md5>::new_from_slice(username.as_bytes()).expect("any key size");
        mac.update(LABEL);
        mac.update(&counter.to_le_bytes());
        out.extend_from_slice(&mac.finalize().into_bytes());
        counter += 1;
    }
    out.truncate(len);
    out
}

pub fn encode(username: &str, plaintext: &str) -> String {
    let ks = keystream(username, plaintext.len());
    let mixed: Vec<u8> = plaintext.bytes().zip(ks).map(|(b, k)| b ^ k).collect();
    format!("{PREFIX}{}", b64::encode_obf(&mixed))
}

pub fn decode(username: &str, stored: &str) -> Result<String> {
    let body = stored
        .strip_prefix(PREFIX)
        .ok_or_else(|| Error::config("not an obfuscated password"))?;
    let mixed = b64::decode_obf(body)
        .ok_or_else(|| Error::config(format!("corrupt obfuscated password for {username}")))?;
    let ks = keystream(username, mixed.len());
    let plain: Vec<u8> = mixed.iter().zip(ks).map(|(b, k)| b ^ k).collect();
    String::from_utf8(plain).map_err(|_| {
        Error::config(format!(
            "obfuscated password for {username} does not decode (username changed?)"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let long = "x".repeat(100);
        for pw in ["", "p", "p@ss word!", "\u{5bc6}\u{7801}123", long.as_str()] {
            let stored = encode("1120240001", pw);
            assert!(is_obfuscated(&stored));
            assert!(pw.is_empty() || !stored.contains(pw));
            assert_eq!(decode("1120240001", &stored).unwrap(), pw);
        }
    }

    #[test]
    fn bound_to_username_and_rejects_garbage() {
        let a = encode("user-a", "same password");
        let b = encode("user-b", "same password");
        assert_ne!(a, b);
        assert_ne!(decode("user-b", &a).ok(), Some("same password".to_string()));
        assert!(decode("x", "plain").is_err());
        assert!(decode("x", "obf1:!!").is_err());
    }
}
