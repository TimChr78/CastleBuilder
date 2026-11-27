//! Spawn the compiled `keep-mcp --offline` binary and drive it over piped stdio,
//! exactly as an MCP host would.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn spawn(args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_keep-mcp"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("keep-mcp binary starts")
}

fn write_line(child: &mut std::process::Child, value: &Value) {
    let stdin = child.stdin.as_mut().expect("stdin is piped");
    writeln!(stdin, "{value}").expect("write request");
    stdin.flush().expect("flush request");
}

#[test]
fn offline_binary_completes_a_full_mcp_session() {
    let mut child = spawn(&["--offline"]);
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        }),
    );
    write_line(
        &mut child,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    write_line(
        &mut child,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "render_plan", "arguments": { "view": "top_plan" } }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "apply_operations",
                "arguments": {
                    "operations": [
                        {
                            "op": "create_wall_run",
                            "ref_id": "east",
                            "name": "East curtain",
                            "start_mm": [20_000, -8_000],
                            "end_mm": [20_000, 8_000],
                            "height_mm": 7_500,
                            "thickness_mm": 2_100
                        },
                        {
                            "op": "add_arrow_loop",
                            "wall_ref": "east",
                            "t_permille": 500,
                            "sill_height_mm": 1_600
                        }
                    ]
                }
            }
        }),
    );

    // Closing stdin ends the server loop; then collect every response line.
    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let responses: Vec<Value> = BufReader::new(stdout)
        .lines()
        .map(|line| serde_json::from_str(&line.expect("stdout line")).expect("valid JSON"))
        .collect();
    let status = child.wait().expect("server exits");
    assert!(status.success());

    // The notification produces no response, so exactly five requests respond.
    assert_eq!(responses.len(), 5, "responses: {responses:#?}");
    for (index, response) in responses.iter().enumerate() {
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], index as u64 + 1);
    }

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "keep-mcp");
    assert_eq!(responses[1]["result"]["tools"].as_array().unwrap().len(), 4);

    let castle = &responses[2]["result"]["structuredContent"];
    assert_eq!(castle["element_count"], 3);
    assert_eq!(castle["opening_count"], 2);

    let plan = &responses[3]["result"]["structuredContent"];
    assert_eq!(plan["mime_type"], "image/svg+xml");
    assert!(plan["svg"].as_str().unwrap().starts_with("<svg "));

    let applied = &responses[4]["result"]["structuredContent"];
    assert_eq!(applied["applied_count"], 2);
    assert_eq!(applied["created_ids"]["east"], 12);
}

#[test]
fn empty_flag_starts_from_an_empty_document() {
    let mut child = spawn(&["--offline", "--empty"]);
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        }),
    );
    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut lines = BufReader::new(stdout).lines();
    let response: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(response["result"]["structuredContent"]["element_count"], 0);
    assert!(lines.next().is_none());
    assert!(child.wait().unwrap().success());
}

#[test]
fn offline_is_the_default_and_unknown_flags_are_refused() {
    let mut child = spawn(&[]);
    write_line(
        &mut child,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }),
    );
    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let response: Value =
        serde_json::from_str(&BufReader::new(stdout).lines().next().unwrap().unwrap()).unwrap();
    assert_eq!(response["result"], json!({}));
    assert!(child.wait().unwrap().success());

    let status = Command::new(env!("CARGO_BIN_EXE_keep-mcp"))
        .arg("--live")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("binary runs");
    assert_eq!(status.code(), Some(2));
}
