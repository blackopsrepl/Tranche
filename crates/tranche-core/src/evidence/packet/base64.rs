//! Standard base64, as the packet carries bodies.
//!
//! Written out rather than pulled in because it is thirty lines and the encoding is
//! part of the packet contract: a reader checks the carried bytes against the digest
//! they are filed under, so the encoder must be exactly standard.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes with standard base64 and padding.
pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let packed = (chunk[0] as u32) << 16
            | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for shift in [18, 12, 6, 0] {
            out.push(ALPHABET[((packed >> shift) & 0x3f) as usize] as char);
        }
        if chunk.len() < 3 {
            let padding = 3 - chunk.len();
            for _ in 0..padding {
                out.pop();
            }
            for _ in 0..padding {
                out.push('=');
            }
        }
    }
    out
}
