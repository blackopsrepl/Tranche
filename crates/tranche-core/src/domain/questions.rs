//! The question policy.
//!
//! This is a frozen contract, not configuration. `judgment_binding()` and
//! `pair_binding()` digest it, so editing a single character re-asks every
//! question in the corpus and re-bills the full backlog. It is written here as
//! data for the same reason: the shape is the API.

use serde_json::{Value, json};

/// A metric's ceiling, which decides whether a model answer is usable.
///
/// Absent, non-finite or out-of-range values are unknown, never zero.
pub fn metric_ceiling(name: &str, field: &str) -> Option<f64> {
    match field {
        "noul" => Some(1.0),
        "score" if name == "risk" => Some(4.0),
        "score" => Some(3.0),
        _ => None,
    }
}

/// The seven typed questions asked of one PR in a single batched call.
pub fn judge_questions() -> Value {
    json!({
        "category": {
            "type": "choice",
            "instructions": {
                "question": "Which area of Omarchy does this pull request mainly touch? Read `pr.title` and `pr.body`; `pr.diffstat` shows the size. Pick exactly one; use `unclear` only when the text is too thin to tell."
            },
            "criteria": {
                "install-setup": "omarchy-setup menu, installer, first boot, ISO, dotfiles bootstrap",
                "desktop-config": "Hyprland, Walker, waybar, wlogout, mako, keybinds, wallpapers, theming",
                "user-experience": "a default/taste or look-and-feel proposal (themes, wallpapers, icons, fonts, bar or menu aesthetics); judge by the change's effect, not the subsystem it edits",
                "shell-cli": "zsh config, aliases, starship, CLI tool defaults, terminal usage",
                "apps-integrations": "default apps, mime handling, new application integrations (e.g. dropbox, spotify, 1password)",
                "hardware-drivers": "NVIDIA, wifi, bluetooth, audio, power, HiDPI, laptops, ARM/Snapdragon, firmware",
                "update-release": "omarchy-update, version bumps, release machinery, migration between versions, boot entries",
                "agents-ai": "AI coding agents, agent hooks, MCP, integrations for Claude/Codex/Gemini-like tools",
                "docs": "README, documentation, wiki, help text only",
                "fix-misc": "a real change that fits none of the above",
                "unclear": "cannot be placed from title and body alone"
            }
        },
        "risk": {
            "type": "score",
            "instructions": {
                "question": "How risky is merging this pull request for existing Omarchy installations? Judge from `pr.title`, `pr.body` and `pr.diffstat`."
            },
            "criteria": [
                "Text-only: docs, themes, wallpapers, menu definitions; nothing executes",
                "Config or script change that is scoped and revertible; no root-side effects at install/update time",
                "Runs as root during install or update, edits boot entries or mounts, or flips system-wide defaults",
                "Touches disk layout, networking, sudo/permissions, security posture, or kernel drivers/firmware",
                "Could break existing installs outright: data loss, boot failure, or user lockout"
            ]
        },
        "is_fix": {
            "type": "noul",
            "instructions": "Is this pull request primarily a fix for a bug or regression, rather than a new feature, a default/taste change, or a refactor? Judge from `pr.title` and `pr.body`."
        },
        "dupe_signal": {
            "type": "noul",
            "instructions": "Do `pr.title` or `pr.body` indicate this pull request duplicates another change, or is superseded by / supersedes one (another open PR, or already-merged upstream work)?"
        },
        "finished_form": {
            "type": "score",
            "instructions": {
                "question": "Is this pull request in finished, reviewable form as described by `pr.body` (with `pr.title`)? DHH asked the triage team to ensure everything is ready for consideration in a finished form."
            },
            "criteria": [
                "Empty or near-empty body; no description of what or why",
                "Says what it does but not why, or shows no evidence it was tried",
                "Clear what and why; states that it was tested on a real system",
                "Clear what and why plus concrete QA evidence (before/after, screenshots, test steps); small and focused"
            ]
        },
        "review_effort": {
            "type": "score",
            "instructions": {
                "question": "How much reviewer effort does this pull request need, judging by `pr.diffstat` and the change described?"
            },
            "criteria": [
                "Trivial and mechanical: a typo, version number, or one-line constant",
                "Small: one focused change a reviewer can hold in their head",
                "Moderate: several related edits that must be checked together",
                "Substantial: architectural or many-part change needing deep review"
            ]
        },
        "security_flag": {
            "type": "noul",
            "instructions": "Does this change touch credentials or secrets, download-and-execute remote code, sudo/permission changes, network exposure, or crypto material? Judge from `pr.title` and `pr.body`."
        },
        "required_skills": {
            "type": "multi_label",
            "instructions": {
                "question": "Which skills does reviewing this pull request require? Read `pr.title`, `pr.body` and `pr.diffstat`. Select all that apply; use `none` when no special skill is needed."
            },
            "criteria": {
                "packaging": "PKGBUILD, makepkg, package build, install scripts, Arch packaging",
                "nvidia": "NVIDIA driver, GPU, graphics stack, CUDA, Optimus",
                "waybar": "waybar config, modules, styling, CSS",
                "release-engineering": "version bumps, changelog, release machinery, CI/CD",
                "security-review": "credentials, permissions, network exposure, crypto, sudo",
                "hardware-drivers": "wifi, bluetooth, audio, power, HiDPI, laptops, ARM/Snapdragon, firmware",
                "desktop-config": "Hyprland, Walker, wlogout, mako, keybinds, wallpapers, theming",
                "shell-cli": "zsh config, aliases, starship, CLI tool defaults, terminal usage",
                "apps-integrations": "default apps, mime handling, new application integrations",
                "update-release": "omarchy-update, version bumps, release machinery, migration between versions, boot entries",
                "agents-ai": "AI coding agents, agent hooks, MCP, integrations for Claude/Codex/Gemini-like tools",
                "docs": "README, documentation, wiki, help text",
                "install-setup": "omarchy-setup menu, installer, first boot, ISO, dotfiles bootstrap",
                "user-experience": "default/taste or look-and-feel proposals, themes, icons, fonts, bar or menu aesthetics",
                "none": "no special skill needed for review"
            }
        }
    })
}

/// The one question asked of a candidate pair.
pub fn pair_questions() -> Value {
    json!({
        "sameness": {
            "type": "choice",
            "instructions": {
                "question": "Do `pr_a` and `pr_b` propose the same underlying change to Omarchy? Judge by what they modify and the outcome, not by wording."
            },
            "criteria": {
                "same_change": "two attempts at the same change; merging one makes the other redundant",
                "related_but_different": "same area or theme but distinct outcomes; both could merge",
                "unrelated": "different changes that merely share words"
            }
        }
    })
}

/// The criterion labels for `sameness`, in policy order.
pub fn sameness_criteria() -> Vec<&'static str> {
    vec!["same_change", "related_but_different", "unrelated"]
}

/// The field a question's answer is read from.
pub fn answer_field(question_type: &str) -> Option<&'static str> {
    match question_type {
        "choice" => Some("choice"),
        "score" => Some("score"),
        "noul" => Some("noul"),
        "multi_label" => Some("labels"),
        _ => None,
    }
}
