//! `evidence show` and `evidence export`: read a stored capture offline.
//!
//! `show` answers from the recorded state and the stored bodies alone. `export`
//! writes the self-contained packet, and a historical export is explicit rather than
//! implied.

use tranche_core::evidence::report;
use tranche_core::report::Root;

use crate::commands::Outcome;

use super::reading::{load_manifest, print_coverage, resume_hint};
use crate::cli::{Export, Show};
use serde_json::json;

/// `evidence show`: inspect coverage, a source window or a citation, offline.
pub fn show(root: &Root, args: &Show, json_output: bool) -> Outcome {
    let manifest = match load_manifest(root, args.capture.as_deref(), args.batch.as_deref()) {
        Ok((manifest, note)) => {
            if let Some(note) = note {
                println!("{note}");
            }
            manifest
        }
        Err(error) => return Outcome::refusal(error.0, 3),
    };

    if let Some(source_id) = args.source.as_deref() {
        let window = match report::window(
            root,
            &manifest,
            source_id,
            args.start_byte as usize,
            args.length as usize,
        ) {
            Ok(window) => window,
            Err(error) => return Outcome::refusal(error, 3),
        };
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&window).unwrap_or_else(|_| "{}".to_owned())
            );
        } else {
            println!(
                "source {source_id}  #{} {}/{} page {}",
                window["number"], window["component"], window["group"], window["page"]
            );
            println!(
                "  {}  {}  {} bytes  sha256 {}",
                window["url"].as_str().unwrap_or(""),
                window["media_type"].as_str().unwrap_or(""),
                window["body_bytes"],
                window["body_sha256"].as_str().unwrap_or("")
            );
            println!(
                "  bytes {}..{}{}",
                window["start_byte"],
                window["end_byte"],
                match window["next_start"].as_u64() {
                    Some(next) => format!("  next --start-byte {next}"),
                    None => "  (end of source)".to_owned(),
                }
            );
            print!("{}", window["content"].as_str().unwrap_or(""));
            if !window["content"].as_str().unwrap_or("").ends_with('\n') {
                println!();
            }
        }
        return Outcome::success(String::new());
    }

    if let Some(citation_id) = args.citation.as_deref() {
        let resolved = match report::resolve_citation(root, &manifest, citation_id) {
            Ok(resolved) => resolved,
            Err(error) => return Outcome::refusal(error, 3),
        };
        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&resolved).unwrap_or_else(|_| "{}".to_owned())
            );
        } else {
            let citation = &resolved["citation"];
            println!(
                "citation {citation_id}  #{} {}",
                citation["number"],
                citation["component"].as_str().unwrap_or("")
            );
            println!(
                "  source {}  bytes {}..{}  excerpt sha256 {}",
                citation["source_id"].as_str().unwrap_or(""),
                citation["start_byte"],
                citation["end_byte"],
                resolved["excerpt_sha256"].as_str().unwrap_or("")
            );
            print!("{}", resolved["excerpt"].as_str().unwrap_or(""));
            if !resolved["excerpt"].as_str().unwrap_or("").ends_with('\n') {
                println!();
            }
        }
        return Outcome::success(String::new());
    }

    let coverage = report::coverage(&manifest);
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&coverage).unwrap_or_else(|_| "{}".to_owned())
        );
    } else {
        if args.capture.is_some() {
            println!("historical capture - nothing here is checked against the current report");
        }
        print_coverage(&coverage);
        if coverage["complete"].as_bool() != Some(true) {
            let batch = manifest["selection"]["batch"]["id"].as_str().unwrap_or("");
            let id = manifest["capture_id"].as_str().unwrap_or("");
            println!("  {}", resume_hint(batch, id));
        }
    }
    Outcome::success(String::new())
}
/// `evidence export`: write the self-contained packet.
pub fn export(root: &Root, args: &Export, json_output: bool) -> Outcome {
    let manifest = match load_manifest(root, args.capture.as_deref(), args.batch.as_deref()) {
        Ok((manifest, _)) => manifest,
        Err(error) => return Outcome::refusal(error.0, 3),
    };
    // A historical export is explicit and never claims current-batch compatibility.
    let historical = args.capture.is_some();
    if historical && !args.allow_historical {
        return Outcome::refusal(
            "exporting a stored capture that is not the current batch needs --allow-historical",
            3,
        );
    }
    let (packet, bytes) =
        match tranche_core::evidence::packet::bytes(root, &manifest, args.max_bytes as usize) {
            Ok(built) => built,
            Err(error) => return Outcome::refusal(error, 3),
        };
    if args.output == "-" {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&bytes);
        return Outcome::success(String::new());
    }
    if let Err(error) = std::fs::write(&args.output, &bytes) {
        return Outcome::refusal(format!("cannot write {}: {error}", args.output), 3);
    }
    let complete = packet["complete"].as_bool() == Some(true);
    if json_output {
        println!(
            "{}",
            json!({
                "capture_id": packet["capture_id"],
                "packet_digest": packet["packet_digest"],
                "bytes": bytes.len(),
                "complete": complete,
                "output": args.output,
            })
        );
    } else {
        println!("wrote {} ({} bytes)", args.output, bytes.len());
        println!(
            "  packet digest {}",
            packet["packet_digest"].as_str().unwrap_or("")
        );
        println!(
            "  capture {}  {}",
            packet["capture_id"].as_str().unwrap_or(""),
            if complete { "COMPLETE" } else { "INCOMPLETE" }
        );
    }
    Outcome::success(String::new())
}
