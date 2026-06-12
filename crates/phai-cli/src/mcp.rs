//! `phai mcp` — a Model Context Protocol server over stdio.
//!
//! Exposes phai's read-only report surface as MCP tools so any MCP-capable
//! client (Claude, IDEs, agent frameworks) can query the finance store
//! without shell access. Design decisions in ADR-0027:
//!
//! - **Hand-rolled JSON-RPC loop, no SDK dependency.** The server speaks the
//!   three MCP methods it needs (`initialize`, `tools/list`, `tools/call`)
//!   plus `ping`; `serde_json` is already a workspace dependency.
//! - **Tools re-exec the current binary** (`current_exe()` + the same args the
//!   CLI takes + `--raw`/`--json`) and return stdout. One code path serves
//!   humans, scripts and agents — the MCP layer cannot drift from the CLI,
//!   and the v1 surface is read-only by construction.
//!
//! Protocol framing: one JSON-RPC 2.0 message per line on stdin/stdout
//! (newline-delimited, the MCP stdio transport). Notifications get no reply.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::process::Command;

const PROTOCOL_VERSION: &str = "2024-11-05";

/// One callable tool: MCP metadata + the argv it maps onto.
struct ToolSpec {
    name: &'static str,
    description: &'static str,
    /// JSON Schema for the tool input (object; properties only — everything
    /// is optional unless listed in `required`).
    input_schema: fn() -> Value,
    /// Maps validated arguments to phai CLI argv (without the binary path).
    build_args: fn(&Value) -> Result<Vec<String>>,
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

/// `month` must look like YYYY-MM when present — fail early with a clear
/// message instead of letting the subprocess print usage noise.
fn push_month(argv: &mut Vec<String>, args: &Value) -> Result<()> {
    if let Some(month) = opt_str(args, "month") {
        let ok = month.len() == 7
            && month.as_bytes()[4] == b'-'
            && month[..4].chars().all(|c| c.is_ascii_digit())
            && month[5..].chars().all(|c| c.is_ascii_digit());
        if !ok {
            anyhow::bail!("month must be YYYY-MM, got {month:?}");
        }
        argv.push("--month".into());
        argv.push(month);
    }
    Ok(())
}

fn month_only_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "month": { "type": "string", "description": "Target month as YYYY-MM. Defaults to the current month." }
        }
    })
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "daily_pulse",
            description: "Recent transactions grouped by category (last N days). JSON.",
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "days": { "type": "integer", "minimum": 1, "maximum": 90, "description": "Lookback window in days (default 7)." }
                    }
                })
            },
            build_args: |args| {
                let mut argv = vec!["report".into(), "daily-pulse".into(), "--raw".into()];
                if let Some(days) = opt_u64(args, "days") {
                    argv.push("--days".into());
                    argv.push(days.to_string());
                }
                Ok(argv)
            },
        },
        ToolSpec {
            name: "monthly_spend",
            description: "One month's expenses by category and subcategory, with totals. JSON.",
            input_schema: month_only_schema,
            build_args: |args| {
                let mut argv = vec!["report".into(), "monthly-spend".into(), "--raw".into()];
                push_month(&mut argv, args)?;
                Ok(argv)
            },
        },
        ToolSpec {
            name: "cashflow",
            description: "Cash-basis monthly summary (income, expenses, net, balance), optionally with the forecast overlay. JSON.",
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "month": { "type": "string", "description": "Target month as YYYY-MM. Defaults to the current month." },
                        "details": { "type": "boolean", "description": "Include the per-category breakdown." },
                        "forecast": { "type": "boolean", "description": "Include forecast remainders." }
                    }
                })
            },
            build_args: |args| {
                let mut argv = vec!["report".into(), "cashflow".into(), "--raw".into()];
                push_month(&mut argv, args)?;
                if args.get("details").and_then(Value::as_bool).unwrap_or(false) {
                    argv.push("--details".into());
                }
                if args.get("forecast").and_then(Value::as_bool).unwrap_or(false) {
                    argv.push("--forecast".into());
                }
                Ok(argv)
            },
        },
        ToolSpec {
            name: "card_summary",
            description: "Credit-card cycles: open and closed bills per card. JSON.",
            input_schema: month_only_schema,
            build_args: |args| {
                let mut argv = vec!["report".into(), "card-summary".into(), "--raw".into()];
                push_month(&mut argv, args)?;
                Ok(argv)
            },
        },
        ToolSpec {
            name: "budget_status",
            description: "Budget vs actual per category, with alerts. JSON.",
            input_schema: month_only_schema,
            build_args: |args| {
                let mut argv = vec!["report".into(), "budget-status".into(), "--raw".into()];
                push_month(&mut argv, args)?;
                Ok(argv)
            },
        },
        ToolSpec {
            name: "installments",
            description: "Active installment chains (parcela X of Y) with projected end. JSON.",
            input_schema: empty_schema,
            build_args: |_| Ok(vec!["report".into(), "installments".into(), "--raw".into()]),
        },
        ToolSpec {
            name: "forecast_vs_actual",
            description: "Planned amounts vs what actually happened. JSON.",
            input_schema: month_only_schema,
            build_args: |args| {
                let mut argv = vec![
                    "report".into(),
                    "forecast-vs-actual".into(),
                    "--raw".into(),
                ];
                push_month(&mut argv, args)?;
                Ok(argv)
            },
        },
        ToolSpec {
            name: "uncategorized",
            description: "Transactions still needing a category. JSON.",
            input_schema: empty_schema,
            build_args: |_| Ok(vec!["report".into(), "uncategorized".into(), "--raw".into()]),
        },
        ToolSpec {
            name: "data_health",
            description: "Consistency checks across the dataset. JSON.",
            input_schema: empty_schema,
            build_args: |_| Ok(vec!["report".into(), "data-health".into(), "--raw".into()]),
        },
        ToolSpec {
            name: "find_transactions",
            description: "Search transactions by description substring. JSON.",
            input_schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Substring to search in descriptions." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Max rows (default 20)." }
                    },
                    "required": ["query"]
                })
            },
            build_args: |args| {
                let query = opt_str(args, "query")
                    .ok_or_else(|| anyhow::anyhow!("query is required"))?;
                let mut argv = vec![
                    "tx".into(),
                    "find".into(),
                    "--json".into(),
                    "--query".into(),
                    query,
                ];
                if let Some(limit) = opt_u64(args, "limit") {
                    argv.push("--limit".into());
                    argv.push(limit.to_string());
                }
                Ok(argv)
            },
        },
    ]
}

/// Run one tool by re-executing this binary with the mapped argv.
fn run_tool(spec: &ToolSpec, args: &Value) -> Result<(String, bool)> {
    let argv = (spec.build_args)(args)?;
    let exe = std::env::current_exe().context("cannot resolve current executable")?;
    let output = Command::new(exe)
        .arg("--no-auto-update")
        .args(&argv)
        .output()
        .with_context(|| format!("failed to spawn phai {}", argv.join(" ")))?;
    if output.status.success() {
        Ok((String::from_utf8_lossy(&output.stdout).into_owned(), false))
    } else {
        let mut msg = String::from_utf8_lossy(&output.stderr).into_owned();
        if msg.trim().is_empty() {
            msg = format!("phai {} exited with {}", argv.join(" "), output.status);
        }
        Ok((msg, true))
    }
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Handle one request; `None` for notifications (no reply expected).
fn handle(message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str)?;
    let id = message.get("id").cloned();
    // No id → notification. The only ones we expect are lifecycle no-ops.
    let id = match id {
        Some(id) => id,
        None => return None,
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    let reply = match method {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "phai",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            let list: Vec<Value> = tools()
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": (t.input_schema)(),
                    })
                })
                .collect();
            rpc_result(id, json!({ "tools": list }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let specs = tools();
            match specs.iter().find(|t| t.name == name) {
                None => rpc_error(id, -32602, &format!("unknown tool: {name}")),
                Some(spec) => match run_tool(spec, &args) {
                    Ok((text, is_error)) => rpc_result(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": text }],
                            "isError": is_error,
                        }),
                    ),
                    Err(e) => rpc_result(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": format!("{e:#}") }],
                            "isError": true,
                        }),
                    ),
                },
            }
        }
        _ => rpc_error(id, -32601, &format!("method not found: {method}")),
    };
    Some(reply)
}

/// Blocking stdio loop. Exits cleanly on EOF.
pub fn run() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.context("failed to read stdin")?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(_) => Some(rpc_error(Value::Null, -32700, "parse error")),
            Ok(message) => handle(&message),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut stdout, &reply)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_tools_capability() {
        let reply = handle(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": PROTOCOL_VERSION }
        }))
        .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        assert_eq!(reply["result"]["serverInfo"]["name"], "phai");
    }

    #[test]
    fn tools_list_exposes_the_read_only_surface() {
        let reply = handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in [
            "daily_pulse",
            "monthly_spend",
            "cashflow",
            "card_summary",
            "budget_status",
            "installments",
            "forecast_vs_actual",
            "uncategorized",
            "data_health",
            "find_transactions",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(handle(&json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }))
        .is_none());
    }

    #[test]
    fn unknown_method_is_minus_32601() {
        let reply =
            handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" })).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let reply = handle(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "rm_rf", "arguments": {} }
        }))
        .unwrap();
        assert_eq!(reply["error"]["code"], -32602);
    }

    #[test]
    fn month_argument_is_validated_before_spawning() {
        let specs = tools();
        let spec = specs.iter().find(|t| t.name == "monthly_spend").unwrap();
        let err = (spec.build_args)(&json!({ "month": "junho" })).unwrap_err();
        assert!(err.to_string().contains("YYYY-MM"));
        let ok = (spec.build_args)(&json!({ "month": "2026-06" })).unwrap();
        assert_eq!(
            ok,
            vec!["report", "monthly-spend", "--raw", "--month", "2026-06"]
        );
    }

    #[test]
    fn find_transactions_requires_query() {
        let specs = tools();
        let spec = specs
            .iter()
            .find(|t| t.name == "find_transactions")
            .unwrap();
        assert!((spec.build_args)(&json!({})).is_err());
        let ok = (spec.build_args)(&json!({ "query": "mercado", "limit": 5 })).unwrap();
        assert_eq!(
            ok,
            vec!["tx", "find", "--json", "--query", "mercado", "--limit", "5"]
        );
    }
}
