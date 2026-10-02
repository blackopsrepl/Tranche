//! Writing the page and its payload.
//!
//! Two escapes live here and they are not the same escape. The template's is
//! HTML, for the category picker. The payload's is JSON-with-non-ASCII-flattened,
//! because the page is served without a charset guarantee: the payload is fetched
//! and injected, so a title containing markup must not survive as markup.

use serde_json::Value;
use tranche_core::report::Root;

use super::CATEGORY_LABELS;

/// Write the page and its payload, and report the sizes.
pub(super) fn write(root: &Root, payload: &Value) -> Result<(usize, usize), String> {
    let template = std::fs::read_to_string(root.template_path())
        .map_err(|error| format!("cannot read {}: {error}", root.template_path().display()))?;
    let options: String = CATEGORY_LABELS
        .iter()
        .map(|(key, label)| format!("<option value=\"{}\">{}</option>", html(key), html(label)))
        .collect();
    let count = payload["prs"].as_array().map(Vec::len).unwrap_or(0);
    let page = template
        .replace("{{options}}", &options)
        .replace("{{count_commas}}", &commas(count))
        .replace("{{count}}", &count.to_string());

    let compact = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
    let encoded = escape_json(&compact);

    let docs = root.docs_dir();
    let data = docs.join("data");
    std::fs::create_dir_all(&data)
        .map_err(|error| format!("cannot create {}: {error}", data.display()))?;
    std::fs::write(data.join("workbench.json"), &encoded)
        .map_err(|error| format!("cannot write the payload: {error}"))?;
    std::fs::write(docs.join("index.html"), &page)
        .map_err(|error| format!("cannot write the page: {error}"))?;
    Ok((page.len(), encoded.len()))
}

/// Escape a string for an HTML attribute or text node.
fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Flatten the payload to ASCII, escaping markup and every non-ASCII character.
fn escape_json(compact: &str) -> String {
    let mut out = String::with_capacity(compact.len());
    for character in compact.chars() {
        match character {
            '&' => out.push_str("\\u0026"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            character if (character as u32) < 0x20 || (character as u32) > 0x7e => {
                let mut buffer = [0u16; 2];
                for unit in character.encode_utf16(&mut buffer) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            character => out.push(character),
        }
    }
    out
}

/// Thousands separators, as the page shows them.
fn commas(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
