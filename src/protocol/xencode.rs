//! The `xEncode` scramble from srun's portal JS (a TEA-like mixer on
//! little-endian 32-bit words). Written from the algorithm, not copied.

fn to_words(bytes: &[u8], append_len: bool) -> Vec<u32> {
    let mut v: Vec<u32> = bytes
        .chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect();
    if append_len {
        v.push(bytes.len() as u32);
    }
    v
}

fn from_words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

pub fn xencode(msg: &[u8], key: &[u8]) -> Vec<u8> {
    if msg.is_empty() {
        return Vec::new();
    }
    let mut v = to_words(msg, true);
    let mut k = to_words(key, false);
    while k.len() < 4 {
        k.push(0);
    }
    let n = v.len() - 1;
    let mut z = v[n];
    let delta: u32 = 0x9E37_79B9;
    let mut d: u32 = 0;
    let mut q = 6 + 52 / (n + 1);
    while q > 0 {
        q -= 1;
        d = d.wrapping_add(delta);
        let e = ((d >> 2) & 3) as usize;
        for p in 0..n {
            let y = v[p + 1];
            let mut m = (z >> 5) ^ (y << 2);
            m = m.wrapping_add((y >> 3) ^ (z << 4) ^ (d ^ y));
            m = m.wrapping_add(k[(p & 3) ^ e] ^ z);
            v[p] = v[p].wrapping_add(m);
            z = v[p];
        }
        let y = v[0];
        let mut m = (z >> 5) ^ (y << 2);
        m = m.wrapping_add((y >> 3) ^ (z << 4) ^ (d ^ y));
        m = m.wrapping_add(k[(n & 3) ^ e] ^ z);
        v[n] = v[n].wrapping_add(m);
        z = v[n];
    }
    from_words(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_message() {
        assert!(xencode(b"", b"key").is_empty());
    }

    #[test]
    fn short_key_is_padded_not_panicking() {
        let out = xencode(b"hello", b"k");
        assert_eq!(out.len(), 12);
    }

    #[test]
    fn output_length_is_words_plus_len_word() {
        assert_eq!(xencode(b"abcd", b"0123456789abcdef").len(), 8);
        assert_eq!(xencode(b"abcde", b"0123456789abcdef").len(), 12);
    }
}
