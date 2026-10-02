//! The categories the picker shows, and the labels they carry.
//!
//! The order is the order the picker lists them, which is why they are one table
//! rather than a lookup and a separate ordering.

/// Category keys to the labels the picker shows.
pub const CATEGORY_LABELS: [(&str, &str); 13] = [
    ("security-review", "Security (meta)"),
    ("install-setup", "Install & Setup"),
    ("desktop-config", "Desktop Config"),
    ("user-experience", "User Experience"),
    ("shell-cli", "Shell & CLI"),
    ("apps-integrations", "Apps & Integrations"),
    ("hardware-drivers", "Hardware & Drivers"),
    ("update-release", "Update & Release"),
    ("agents-ai", "Agents & AI"),
    ("docs", "Docs"),
    ("fix-misc", "Fixes & Misc"),
    ("unclear", "Unclear"),
    ("unknown", "Unknown"),
];
