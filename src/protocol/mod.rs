//! Srun portal protocol: challenge, login, logout, status.

pub mod b64;
pub mod client;
pub mod errors;
pub mod hash;
pub mod xencode;

pub use client::{Client, LoginOutcome, LoginRequest, PasswordMode, StatusResp};

/// `{SRBX1}` + custom base64 of xEncode(json, token).
pub fn encode_info(json: &str, token: &str) -> String {
    let scrambled = xencode::xencode(json.as_bytes(), token.as_bytes());
    format!("{{SRBX1}}{}", b64::encode(&scrambled))
}

/// The `info` login field, built like the official portal:
/// `JSON.stringify({username, password, ip, acid, enc_ver})` with `acid` as a string.
pub fn info(username: &str, password: &str, ip: &str, acid: &str, token: &str) -> String {
    let json = serde_json::json!({
        "username": username,
        "password": password,
        "ip": ip,
        "acid": acid,
        "enc_ver": "srun_bx1",
    });
    // serde_json's Map keeps insertion order only with `preserve_order`; build
    // the string by hand so the key order matches the portal exactly.
    let s = format!(
        "{{\"username\":{},\"password\":{},\"ip\":{},\"acid\":{},\"enc_ver\":\"srun_bx1\"}}",
        json["username"], json["password"], json["ip"], json["acid"]
    );
    encode_info(&s, token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vector {
        msg: String,
        token: String,
        xencode_hex: String,
        username: String,
        password: String,
        ip: String,
        acid: String,
        go_info_json: String,
        info: String,
        hmd5_real: String,
        hmd5_empty: String,
        chksum: String,
    }

    fn vectors() -> Vec<Vector> {
        let raw = include_str!("../../tests/vectors.json");
        serde_json::from_str(raw).unwrap()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn xencode_matches_go_oracle() {
        for v in vectors() {
            let got = hex(&xencode::xencode(v.msg.as_bytes(), v.token.as_bytes()));
            assert_eq!(got, v.xencode_hex, "msg={:?}", v.msg);
        }
    }

    #[test]
    fn info_encoding_matches_go_oracle() {
        for v in vectors() {
            assert_eq!(encode_info(&v.go_info_json, &v.token), v.info);
        }
    }

    #[test]
    fn hmd5_and_chksum_match_go_oracle() {
        for v in vectors() {
            let real = hash::hmd5(&v.password, &v.token);
            assert_eq!(format!("{{MD5}}{real}"), v.hmd5_real);
            assert_eq!(format!("{{MD5}}{}", hash::hmd5("", &v.token)), v.hmd5_empty);
            let sum = hash::chksum(
                &v.token,
                &v.username,
                &real,
                &v.acid,
                &v.ip,
                "200",
                "1",
                &v.info,
            );
            assert_eq!(sum, v.chksum);
        }
    }

    #[test]
    fn info_uses_portal_key_order_and_string_acid() {
        let tok = "0123456789abcdef";
        let ours = info("u", "p", "1.2.3.4", "8", tok);
        let manual = encode_info(
            r#"{"username":"u","password":"p","ip":"1.2.3.4","acid":"8","enc_ver":"srun_bx1"}"#,
            tok,
        );
        assert_eq!(ours, manual);
    }
}
