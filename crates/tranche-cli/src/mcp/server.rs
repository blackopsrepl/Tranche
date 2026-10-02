//! Newline-delimited JSON-RPC over stdio: the standard MCP stdio transport.
//!
//! One process serves one long-lived session. Every request re-reads the bound
//! report through the same gates the CLI uses, and the input fingerprint is taken
//! before and after the read so a file changing under it is refused rather than
//! half-served.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use super::error::ReportError;
use super::schema;
use super::tools::{DISCLAIMER, View};

/// Protocol versions this server speaks; the client's is echoed when known.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const PROTOCOL_VERSION: &str = PROTOCOL_VERSIONS[0];

/// Serve requests until stdin closes.
pub fn serve(root: &tranche_core::report::Root) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|error| format!("stdin: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => dispatch(root, &message),
            Err(_) => Some(error_response(-32700, "Parse error", None)),
        };
        if let Some(response) = response {
            let mut text = serde_json::to_string(&response).map_err(|error| error.to_string())?;
            text.push('\n');
            out.write_all(text.as_bytes())
                .map_err(|error| format!("stdout: {error}"))?;
            out.flush().map_err(|error| format!("stdout: {error}"))?;
        }
    }
    Ok(())
}

/// Handle one JSON-RPC message; `None` for a notification.
fn dispatch(root: &tranche_core::report::Root, message: &Value) -> Option<Value> {
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_response(-32600, "Invalid Request", id_of(message)));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Some(error_response(-32600, "Invalid Request", id_of(message)));
    };
    let request_id = id_of(message)?;
    match method {
        "initialize" => {
            let requested = message["params"]["protocolVersion"].as_str();
            let version = requested
                .filter(|requested| PROTOCOL_VERSIONS.contains(requested))
                .unwrap_or(PROTOCOL_VERSION);
            Some(json!({
                "jsonrpc": "2.0", "id": request_id, "result": {
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "tranche", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": DISCLAIMER,
                }
            }))
        }
        "ping" => Some(json!({"jsonrpc": "2.0", "id": request_id, "result": {}})),
        "tools/list" => {
            // The tool list names the question policy's categories, so it needs a
            // loaded report; a report that cannot be loaded still lists the tools,
            // because the list is static apart from that enumeration.
            let categories = loaded_view(root)
                .map(|view| view.judge_categories())
                .unwrap_or_default();
            Some(json!({
                "jsonrpc": "2.0", "id": request_id,
                "result": {"tools": schema::definitions(&categories)}
            }))
        }
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or("");
            let known = [
                "surface",
                "query",
                "pick",
                "next_prompt",
                "related",
                "digests",
            ]
            .contains(&name);
            if !known {
                return Some(error_response(
                    -32602,
                    &format!("Unknown tool: {name}"),
                    Some(request_id),
                ));
            }
            let arguments = message["params"]["arguments"].clone();
            match call_tool(root, name, &arguments) {
                Ok(result) => Some(json!({
                    "jsonrpc": "2.0", "id": request_id, "result": {
                        "content": [{"type": "text", "text": result}],
                        "isError": false,
                    }
                })),
                Err(error) => Some(json!({
                    "jsonrpc": "2.0", "id": request_id, "result": {
                        "content": [{"type": "text", "text": error.0}],
                        "isError": true,
                    }
                })),
            }
        }
        other => Some(error_response(
            -32601,
            &format!("Method not found: {other}"),
            Some(request_id),
        )),
    }
}

fn id_of(message: &Value) -> Option<Value> {
    match message.get("id") {
        Some(id) if !id.is_null() => Some(id.clone()),
        _ => None,
    }
}

fn error_response(code: i32, message: &str, request_id: Option<Value>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message}
    })
}

/// Run one tool call, with the schema enforced server-side.
fn call_tool(
    root: &tranche_core::report::Root,
    name: &str,
    arguments: &Value,
) -> Result<String, ReportError> {
    let view = loaded_view(root)?;
    let tools = schema::definitions(&view.judge_categories());
    schema::validate_arguments(name, arguments, &tools).map_err(ReportError)?;
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let answer = match name {
        "surface" => view.surface()?,
        "query" => view.query(super::QueryArguments {
            text: str_arg(&arguments, "text").unwrap_or(""),
            category: str_arg(&arguments, "category"),
            risk_band: str_arg(&arguments, "risk_band"),
            security: bool_arg(&arguments, "security"),
            finished_form: number_arg(&arguments, "finished_form"),
            batch: str_arg(&arguments, "batch"),
            queue: str_arg(&arguments, "queue").unwrap_or("all"),
            offset: u64_arg(&arguments, "offset").unwrap_or(0),
            limit: u64_arg(&arguments, "limit").unwrap_or(25),
        })?,
        "pick" => view.pick(str_arg(&arguments, "batch_id").unwrap_or(""))?,
        "next_prompt" => view.next_prompt(str_arg(&arguments, "after"))?,
        "related" => view.related(
            arguments
                .get("number")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ReportError("Expected captured PR number and bounded pagination".to_owned())
                })?,
            u64_arg(&arguments, "offset").unwrap_or(0),
            u64_arg(&arguments, "limit").unwrap_or(25),
        )?,
        "digests" => view.digests()?,
        other => return Err(ReportError(format!("Unknown tool: {other}"))),
    };
    // The exact JSON text is the response; it must not be reserialized.
    serde_json::to_string(&answer)
        .map_err(|error| ReportError(format!("a tool answer cannot be serialized: {error}")))
}

fn str_arg<'a>(arguments: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn bool_arg(arguments: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    arguments.get(key).and_then(Value::as_bool)
}

fn number_arg(arguments: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    arguments.get(key).and_then(Value::as_f64)
}

fn u64_arg(arguments: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    arguments.get(key).and_then(Value::as_u64)
}

/// Load and gate the bound report, refusing a report that changed under the read.
fn loaded_view(root: &tranche_core::report::Root) -> Result<View, ReportError> {
    let limits = tranche_core::report::Limits::default();
    let before =
        tranche_core::report::input_digests(root, &limits).map_err(|error| ReportError(error.0))?;
    let report = tranche_core::report::load(root, &limits).map_err(|error| ReportError(error.0))?;
    let after =
        tranche_core::report::input_digests(root, &limits).map_err(|error| ReportError(error.0))?;
    let mut identity = report.identity.clone();
    identity["input_bytes"] = json!(before);
    if after != before {
        return Err(ReportError(
            "Report files changed during read; retry".to_owned(),
        ));
    }
    Ok(View {
        dupes: report.dupes.clone(),
        batches: report.batches.clone(),
        parked: report.parked.clone(),
        prs: report.prs,
        judgments: report.judgments,
        pairs: report.pairs,
        latest_judgments: report.latest_judgments,
        identity,
    })
}
