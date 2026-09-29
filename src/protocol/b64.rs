//! Base64 with custom alphabets: srun's `{SRBX1}` alphabet for the login
//! info field, and our own shuffled alphabet for obfuscated passwords.

const SRUN_ALPHABET: &[u8; 64] =
    b"LVoJPiCN2R8G90yg+hmFHuacZ1OWMnrsSTXkYpUq/3dlbfKwv6xztjI7DeBE45QA";
const OBF_ALPHABET: &[u8; 64] = b"qZ7mK2xWpB9cVfL4nTjR0yGsHdE6aUiN3oXkAvC8bMwQ1eYtF5gPzJrIhSuD+/lO";

pub fn encode(data: &[u8]) -> String {
    encode_with(SRUN_ALPHABET, data)
}

pub fn encode_obf(data: &[u8]) -> String {
    encode_with(OBF_ALPHABET, data)
}

pub fn decode_obf(s: &str) -> Option<Vec<u8>> {
    decode_with(OBF_ALPHABET, s)
}

fn decode_with(alphabet: &[u8; 64], s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = alphabet.iter().position(|&a| a == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

fn encode_with(alphabet: &[u8; 64], data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(alphabet[((n >> 18) & 63) as usize] as char);
        out.push(alphabet[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(alphabet[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(alphabet[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obf_alphabet_is_a_permutation_and_round_trips() {
        let mut sorted = OBF_ALPHABET.to_vec();
        sorted.sort_unstable();
        let mut std = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".to_vec();
        std.sort_unstable();
        assert_eq!(sorted, std);
        for len in 0..40 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode_obf(&encode_obf(&data)).unwrap(), data);
        }
        assert!(decode_obf("q!").is_none());
    }

    #[test]
    fn known_values() {
        let std = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let expect: String = "TWFu"
            .bytes()
            .map(|c| SRUN_ALPHABET[std.iter().position(|&s| s == c).unwrap()] as char)
            .collect();
        assert_eq!(encode(b"Man"), expect);
        assert_eq!(encode(b""), "");
        assert!(encode(b"M").ends_with("=="));
        assert!(encode(b"Ma").ends_with('='));
        assert_eq!(encode(b"Ma").len(), 4);
    }
}
