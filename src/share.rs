//! Sketches in links: URL-safe base64 (no padding) of the sketch's text, so a
//! link like `monet.hunterharloff.com/#s=…` opens exactly what you made.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// URL-safe base64, no padding.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for k in 0..=chunk.len() {
            out.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
        }
    }
    out
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_any_text() {
        for s in ["", "a", "ab", "abc", "mesh(\"dawn\").flow(.3) // é ✓ 🎨", &"x".repeat(1000)] {
            let e = encode(s.as_bytes());
            assert!(e.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'), "{e}");
            assert_eq!(decode(&e).unwrap(), s.as_bytes());
        }
        assert_eq!(encode(b"Man"), "TWFu");
        assert_eq!(decode("TWE=").unwrap(), b"Ma", "padding and the standard alphabet are fine");
        assert!(decode("no spaces").is_none());
    }
}
