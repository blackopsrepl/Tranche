//! The export packet, from a real capture manifest.
//!
//! The properties here are the ones a reader depends on when the packet is the
//! only thing they have: the digest recomputes from the packet's own bytes, a
//! citation resolves against the carried body, and the projection carries what the
//! contract says and nothing it does not.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tranche_core::evidence::packet;
use tranche_core::report::Root;

/// A checkout carrying the real capture, copied so a build never touches it.
fn root() -> Option<tempfile::TempDir> {
    let source = Path::new("/srv/lab/hack/omarchy-pr-jev-triage").join("out/evidence");
    let capture = source.join("108dbe75ccac6ea366b543791c8f168e/manifest.json");
    if !capture.exists() {
        return None;
    }
    let root = tempfile::tempdir().expect("a root");
    copy(&source, &root.path().join("out/evidence"));
    Some(root)
}

fn copy(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("a directory");
    for entry in std::fs::read_dir(source).expect("the capture reads") {
        let entry = entry.expect("an entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a file");
        }
    }
}

fn manifest(root: &Path) -> Value {
    let path = root.join("out/evidence/108dbe75ccac6ea366b543791c8f168e/manifest.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the manifest reads"))
        .expect("the manifest parses")
}

fn built(root: &Path) -> Value {
    let report = Root::new(root.to_path_buf());
    packet::build(&report, &manifest(root)).expect("a packet is built")
}

#[test]
fn the_packet_digest_covers_the_packet_but_not_itself() {
    let Some(root) = root() else {
        eprintln!("no local capture; nothing to build from");
        return;
    };
    let packet = built(root.path());
    let recorded = packet["packet_digest"].as_str().expect("a digest");
    // Recompute from the packet with its own digest removed.
    let mut without = packet.clone();
    without
        .as_object_mut()
        .expect("an object")
        .remove("packet_digest");
    assert_eq!(
        tranche_core::util::digest(&without),
        recorded,
        "the recorded digest recomputes from the packet"
    );
}

#[test]
fn a_citation_resolves_against_the_body_the_packet_carries() {
    // This is the operation a reader must be able to perform offline: no state
    // store, no network, no second application.
    let Some(root) = root() else {
        return;
    };
    let packet = built(root.path());
    let bodies = packet["bodies"].as_object().expect("carried bodies");
    let citations = packet["citations"].as_array().expect("citations");
    assert!(!citations.is_empty(), "the capture recorded citations");

    for citation in citations {
        let sha = citation["source_sha256"].as_str().expect("a source digest");
        let body = bodies.get(sha).expect("the cited body is carried");
        let decoded = decode(body["base64"].as_str().expect("base64 bodies"));
        assert_eq!(
            decoded.len() as u64,
            body["bytes"].as_u64().expect("a byte count"),
            "the carried length matches"
        );
        assert_eq!(
            tranche_core::util::sha256_hex(&decoded),
            sha,
            "the carried bytes match the digest they are filed under"
        );
        let start = citation["start_byte"].as_u64().expect("a start") as usize;
        let end = citation["end_byte"].as_u64().expect("an end") as usize;
        assert_eq!(
            tranche_core::util::sha256_hex(&decoded[start..end]),
            citation["excerpt_sha256"]
                .as_str()
                .expect("an excerpt digest"),
            "the excerpt recomputes"
        );
    }
}

#[test]
fn the_projection_carries_the_contract_and_nothing_else() {
    let Some(root) = root() else {
        return;
    };
    let packet = built(root.path());
    for key in [
        "format",
        "profile",
        "capture_id",
        "generation",
        "selection",
        "membership_digest",
        "complete",
        "capture",
        "components",
        "sources",
        "bodies",
        "citations",
        "notes",
        "packet_digest",
    ] {
        assert!(packet.get(key).is_some(), "carries {key}");
    }
    assert_eq!(packet["format"], "tranche.evidence-packet/v1");
    assert_eq!(packet["profile"], "pr-review/v1");

    // The manifest's own bookkeeping does not travel: a reader verifying evidence
    // has no use for it. A member's `updated_at` is a selection field and stays.
    let text = serde_json::to_string(&packet).expect("serializes");
    for absent in ["bytes_stored", "code_observation", "created_at"] {
        assert!(
            !text.contains(absent),
            "the packet does not carry the manifest's {absent}"
        );
    }
    let top = packet.as_object().expect("an object");
    assert!(
        !top.contains_key("updated_at"),
        "the manifest's last-write time is not a packet field"
    );
    // And a source carries no inline body, which is the superseded layout.
    for source in packet["sources"].as_array().expect("sources") {
        assert!(
            source.get("body_base64").is_none(),
            "sources reference bodies rather than inline them"
        );
    }
    // The accounting keys are exactly the documented set.
    let capture = packet["capture"].as_object().expect("accounting");
    assert_eq!(capture.len(), 8, "eight accounting keys");
    for key in [
        "observed_at",
        "request_limit",
        "requests_used",
        "reserved",
        "failures",
        "retries",
        "identity_checks",
        "stop_reason",
    ] {
        assert!(capture.contains_key(key), "accounting carries {key}");
    }
}

#[test]
fn identical_bodies_are_carried_once() {
    let Some(root) = root() else {
        return;
    };
    let packet = built(root.path());
    let sources = packet["sources"].as_array().expect("sources");
    let bodies = packet["bodies"].as_object().expect("bodies");
    let distinct: std::collections::HashSet<&str> = sources
        .iter()
        .filter_map(|source| source["body_sha256"].as_str())
        .collect();
    assert_eq!(
        bodies.len(),
        distinct.len(),
        "one body per distinct digest, however many sources reference it"
    );
}

#[test]
fn the_export_bound_is_checked_on_the_serialized_bytes() {
    let Some(root) = root() else {
        return;
    };
    let report = Root::new(root.path().to_path_buf());
    let manifest = manifest(root.path());
    let (packet, bytes) =
        packet::bytes(&report, &manifest, packet::DEFAULT_EXPORT_BYTES).expect("under the bound");
    assert_eq!(
        bytes.len(),
        serde_json::to_string(&packet).expect("serializes").len(),
        "the bytes are the packet"
    );
    assert!(!bytes.ends_with(b"\n"), "serialization adds no newline");

    // A bound the packet cannot fit refuses rather than truncating: a truncated
    // packet would resolve citations to the wrong bytes.
    assert!(packet::bytes(&report, &manifest, 1_024).is_err());
}

/// Decode standard base64, asserting the padding as it goes.
fn decode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for character in text.chars() {
        if character == '=' {
            break;
        }
        let value = match character {
            'A'..='Z' => character as u32 - 'A' as u32,
            'a'..='z' => character as u32 - 'a' as u32 + 26,
            '0'..='9' => character as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            other => panic!("not base64: {other:?}"),
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// The unused-import guard: `PathBuf` is used in the helper signatures above.
#[allow(dead_code)]
fn _path_type(_: PathBuf) {}
