//! MCP (Model Context Protocol) server over stdio: newline-delimited JSON-RPC 2.0.

use std::io::{BufRead, Write};
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::admin::{JsonWrite, WriteErrors, admin_json_write};
use crate::auth::Auth;
use crate::i18n::Lang;
use crate::schedule::State;
use crate::views::{admin_json, home_view};

const DEFAULT_PROTOCOL: &str = "2025-06-18";
const UNAUTHORIZED: &str = "unauthorized: TRASHDIFF_PASSWORD required";

pub fn mcp_run(db: std::path::PathBuf) -> Result<(), String> {
    let auth = Auth::from_env();
    auth.warn_if_unset();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(resp) = handle(&db, &line, auth.configured()) else {
            continue;
        };
        writeln!(out, "{}", serde_json::to_string(&resp).unwrap())
            .map_err(|e| format!("stdout: {e}"))?;
        out.flush().map_err(|e| format!("stdout: {e}"))?;
    }
    Ok(())
}

/// True when the message calls the only write tool. Lets the HTTP transport
/// answer `401` before dispatching.
pub fn is_write_request(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    v.get("method").and_then(Value::as_str) == Some("tools/call")
        && v.pointer("/params/name").and_then(Value::as_str) == Some("schedule_write")
}

/// Handle one JSON-RPC message. Returns `None` for notifications (no `id`).
/// `authed` is the caller's write authorization (HTTP header check, or the
/// presence of `TRASHDIFF_PASSWORD` in the stdio process environment).
pub fn handle(db: &Path, line: &str, authed: bool) -> Option<Value> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(&Value::Null, -32700, &format!("parse error: {e}"))),
    };
    if req.is_array() {
        return Some(error(&Value::Null, -32600, "batch requests not supported"));
    }
    let id = req.get("id")?.clone();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let res = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(db, &params, authed),
        _ => Err((-32601, format!("unknown method: {method}"))),
    };
    Some(match res {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, msg)) => error(&id, code, &msg),
    })
}

fn initialize(params: &Value) -> Value {
    json!({
        "protocolVersion": params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL),
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "trashdiff", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "what_to_throw",
            "description": "Waste type that can be put out right now, the window deadline, and this week's pickups.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "schedule_read",
            "description": "Read the full configuration: timezone, pickup_time, default_lang, custom_js, custom_css and the per-day schedule rows.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "schedule_write",
            "description": "Replace the whole configuration. Arguments have the same shape as schedule_read returns. Invalid input returns the validation errors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "timezone": { "type": "string", "description": "IANA timezone, e.g. Europe/Rome" },
                    "pickup_time": { "type": "string", "description": "HH:MM local pickup time" },
                    "default_lang": { "type": ["string", "null"], "enum": ["it", "en", null] },
                    "custom_js": { "type": ["string", "null"] },
                    "custom_css": { "type": ["string", "null"] },
                    "schedule": {
                        "type": "array",
                        "description": "Seven arrays, monday..sunday. Each row is {weeks:[1..5], type:\"...\"}.",
                        "items": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "weeks": { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 5 } },
                                    "type": { "type": "string" }
                                },
                                "required": ["weeks", "type"]
                            }
                        }
                    }
                },
                "required": ["timezone", "pickup_time", "schedule"]
            }
        }
    ])
}

fn call(db: &Path, params: &Value, authed: bool) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    match name {
        "what_to_throw" => Ok(ok_json(&home_view(&load(db)?, chrono::Utc::now()))),
        "schedule_read" => Ok(ok_json(&admin_json(&load(db)?))),
        "schedule_write" => {
            if !authed {
                return Err((-32001, UNAUTHORIZED.to_string()));
            }
            Ok(write_schedule(db, &load(db)?, &args))
        }
        _ => Err((-32602, format!("unknown tool: {name}"))),
    }
}

fn load(db: &Path) -> Result<State, (i64, String)> {
    State::load(db.to_path_buf()).map_err(|e| (-32000, e))
}

fn write_schedule(db: &Path, st: &State, args: &Value) -> Value {
    let lng = args
        .get("default_lang")
        .and_then(|v| serde_json::from_value::<Lang>(v.clone()).ok())
        .or(st.default_lang)
        .unwrap_or_else(Lang::from_env);
    let raw = serde_json::to_vec(args).unwrap_or_default();
    match admin_json_write(&db.to_path_buf(), &raw, lng) {
        JsonWrite::Saved => match State::load(db.to_path_buf()) {
            Ok(st) => ok_json(&json!({ "saved": true, "schedule": admin_json(&st) })),
            Err(e) => content(e, true),
        },
        JsonWrite::ParseErr(e) => content(e, true),
        JsonWrite::Invalid(errs) => err_json(&WriteErrors::from(errs)),
    }
}

fn content(text: String, is_error: bool) -> Value {
    let mut v = json!({ "content": [{ "type": "text", "text": text }] });
    if is_error {
        v["isError"] = json!(true);
    }
    v
}

fn ok_json(v: &impl Serialize) -> Value {
    content(serde_json::to_string_pretty(v).unwrap_or_default(), false)
}

fn err_json(v: &impl Serialize) -> Value {
    content(serde_json::to_string_pretty(v).unwrap_or_default(), true)
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
