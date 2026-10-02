//! A terminal-safe rendering of stored bytes.
//!
//! Control characters, escape sequences and non-ASCII are shown as escapes rather
//! than emitted: a captured body is untrusted text and must not be able to drive the
//! terminal that reads it.

pub fn printable(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        // A newline is the one control character a reader needs literally.
        if character == '\n' || ('\u{20}'..='\u{7e}').contains(&character) {
            out.push(character);
        } else {
            for unit in character.encode_utf16(&mut [0u16; 2]) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}
