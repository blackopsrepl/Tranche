//! The category labels the Omarchy deployment shipped with, kept as data.
//!
//! The engine no longer reads this table: `tranche.json`'s
//! `display.category_labels` is the source the picker renders from. The file
//! stays so the original labeling survives next to the emitted policy and a
//! regenerated contract can be diffed against the deployment's first edition.

/// Category keys to the labels the picker showed in the Omarchy deployment.
#[allow(dead_code)]
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
