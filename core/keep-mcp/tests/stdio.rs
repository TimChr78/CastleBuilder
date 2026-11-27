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
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": { "name": "undo", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "get_history", "arguments": {} }
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

    // The notification produces no response, so exactly seven requests respond.
    assert_eq!(responses.len(), 7, "responses: {responses:#?}");
    for (index, response) in responses.iter().enumerate() {
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], index as u64 + 1);
    }

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "keep-mcp");
    assert_eq!(
        responses[1]["result"]["tools"].as_array().unwrap().len(),
        23
    );

    let castle = &responses[2]["result"]["structuredContent"];
    assert_eq!(castle["element_count"], 3);
    assert_eq!(castle["opening_count"], 2);

    let plan = &responses[3]["result"]["structuredContent"];
    assert_eq!(plan["mime_type"], "image/svg+xml");
    assert!(plan["svg"].as_str().unwrap().starts_with("<svg "));

    let applied = &responses[4]["result"]["structuredContent"];
    assert_eq!(applied["applied_count"], 2);
    assert_eq!(applied["created_ids"]["east"], 12);

    let undo = &responses[5]["result"]["structuredContent"];
    assert_eq!(undo["seq"], 2);
    assert_eq!(undo["compensated_seq"], 1);
    assert_eq!(undo["document_hash"], castle["document_hash"]);
    assert_eq!(undo["layout_hash"], castle["layout_hash"]);

    let history = &responses[6]["result"]["structuredContent"];
    assert_eq!(history["count"], 2);
    assert_eq!(history["entries"][0]["seq"], 1);
    assert_eq!(history["entries"][0]["undone"], true);
    assert_eq!(history["entries"][1]["seq"], 2);
    assert_eq!(history["entries"][1]["summary"], "Undo command #1");
    assert_eq!(history["entries"][1]["undone"], false);
}

#[test]
fn offline_binary_serves_the_request_inbox_and_activity_feed() {
    let mut child = spawn(&["--offline"]);
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_activity_feed", "arguments": { "limit": 5 } }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "list_requests", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "resources/read",
            "params": { "uri": "keep://session/requests" }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "resources/read",
            "params": { "uri": "keep://session/activity" }
        }),
    );

    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let responses: Vec<Value> = BufReader::new(stdout)
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert!(child.wait().unwrap().success());
    assert_eq!(responses.len(), 5, "responses: {responses:#?}");

    let feed = &responses[1]["result"]["structuredContent"];
    assert!(feed["count"].as_u64().unwrap() >= 2);
    assert!(feed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["action"] == "get_castle" && entry["actor"] == "agent:mcp-offline"));

    let listed = &responses[2]["result"]["structuredContent"];
    assert_eq!(listed["count"], 0);
    assert_eq!(listed["requests"], json!([]));

    let requests = &responses[3]["result"]["contents"][0];
    let parsed: Value = serde_json::from_str(requests["text"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["count"], 0);

    let activity = &responses[4]["result"]["contents"][0];
    let parsed: Value = serde_json::from_str(activity["text"].as_str().unwrap()).unwrap();
    assert!(parsed["count"].as_u64().unwrap() >= 1);
    assert!(parsed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["action"] == "get_castle"));
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
fn m1_flag_starts_the_edwardian_showcase_on_the_coastal_headland() {
    let mut child = spawn(&["--offline", "--m1"]);
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_site", "arguments": {} }
        }),
    );
    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let responses: Vec<Value> = BufReader::new(stdout)
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert!(child.wait().unwrap().success());
    assert_eq!(responses.len(), 2, "responses: {responses:#?}");

    let castle = &responses[0]["result"]["structuredContent"];
    assert_eq!(castle["element_count"], 7);
    assert_eq!(castle["opening_count"], 3);
    let kinds: Vec<&str> = castle["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|element| element["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"gatehouse"));
    assert!(kinds.contains(&"ditch"));

    let site = &responses[1]["result"]["structuredContent"];
    assert_eq!(site["site"]["id"], "coastal_headland");
}

#[test]
fn m2_flag_starts_the_baltic_showcase_on_the_lake_island() {
    let mut child = spawn(&["--offline", "--m2"]);
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_site", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "get_assessment",
                "arguments": { "style_id": "baltic_brick" }
            }
        }),
    );
    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let responses: Vec<Value> = BufReader::new(stdout)
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert!(child.wait().unwrap().success());
    assert_eq!(responses.len(), 3, "responses: {responses:#?}");

    let castle = &responses[0]["result"]["structuredContent"];
    assert_eq!(castle["element_count"], 14);
    let kinds: Vec<&str> = castle["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|element| element["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"keep"));
    assert!(kinds.contains(&"range"));
    assert!(kinds.contains(&"gatehouse"));
    assert!(kinds.contains(&"ditch"));

    let site = &responses[1]["result"]["structuredContent"];
    assert_eq!(site["site"]["id"], "lake_island");

    let assessment = &responses[2]["result"]["structuredContent"];
    assert_eq!(assessment["style_id"], "baltic_brick");
    assert_eq!(assessment["passed"], true);
    assert!(
        assessment["historian"]["overall_score"].as_u64().unwrap() >= 80,
        "historian: {}",
        assessment["historian"]
    );
}

#[test]
fn site_flag_starts_empty_on_a_curated_site() {
    let mut child = spawn(&["--offline", "--site", "lake_island"]);
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        }),
    );
    write_line(
        &mut child,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_site", "arguments": {} }
        }),
    );
    drop(child.stdin.take());
    let stdout = child.stdout.take().expect("stdout is piped");
    let responses: Vec<Value> = BufReader::new(stdout)
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert!(child.wait().unwrap().success());
    assert_eq!(responses.len(), 2, "responses: {responses:#?}");

    let castle = &responses[0]["result"]["structuredContent"];
    assert_eq!(castle["element_count"], 0);
    assert_eq!(castle["opening_count"], 0);

    let site = &responses[1]["result"]["structuredContent"];
    assert_eq!(site["site"]["id"], "lake_island");
    assert_eq!(site["available_sites"].as_array().unwrap().len(), 3);
}

#[test]
fn unknown_site_exits_with_code_two() {
    for site in ["not_a_site", ""] {
        let status = Command::new(env!("CARGO_BIN_EXE_keep-mcp"))
            .args(["--offline", "--site", site])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("binary runs");
        assert_eq!(status.code(), Some(2), "site {site:?}");
    }
}

#[test]
fn missing_site_value_exits_with_code_two() {
    let status = Command::new(env!("CARGO_BIN_EXE_keep-mcp"))
        .arg("--site")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("binary runs");
    assert_eq!(status.code(), Some(2));
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
