//! End-to-end live-mode test: the real `keep-mcp` binary against a loopback
//! mock of the game's control server. Verifies the CLI flags, the proposal-only
//! default and the exact bearer/Host headers.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

fn header_value(request: &str, name: &str) -> Option<String> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

#[test]
fn live_binary_forwards_after_local_permission_check() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock control server");
    let port = listener.local_addr().unwrap().port();
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&captured);

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept live request");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut head = String::new();
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("read request line");
            if read == 0 || line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).expect("read request body");
        recorded
            .lock()
            .unwrap()
            .push(format!("{head}\r\n{}", String::from_utf8_lossy(&body)));

        let payload = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "document_hash": "0123456789abcdef",
                "layout_hash": "fedcba9876543210",
                "element_count": 0,
                "opening_count": 0,
                "block_count": 0,
                "chunk_count": 0,
                "elements": [],
                "openings": []
            }
        }))
        .unwrap();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        stream.write_all(reply.as_bytes()).unwrap();
        stream.flush().unwrap();
    });

    let url = format!("http://127.0.0.1:{port}");
    let mut child = Command::new(env!("CARGO_BIN_EXE_keep-mcp"))
        .args(["--live", &url, "--token", "test-token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn keep-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();

    // Direct edits are refused locally: live mode defaults to proposal-only.
    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "apply_operations", "arguments": { "operations": [] } }
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let first: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(first["id"], 1);
    assert_eq!(first["result"]["isError"], true);
    assert!(first["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("proposal-only"));

    // Reads are forwarded over the loopback control server.
    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "get_castle", "arguments": {} }
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let second: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(second["id"], 2);
    assert_eq!(
        second["result"]["isError"], false,
        "live get_castle failed: {second}"
    );
    assert_eq!(
        second["result"]["structuredContent"]["document_hash"],
        "0123456789abcdef"
    );

    drop(stdin);
    let status = child.wait().expect("wait for keep-mcp");
    assert!(status.success(), "keep-mcp exited with {status}");
    server.join().expect("mock server thread");

    let requests = captured.lock().unwrap();
    assert_eq!(requests.len(), 1, "only the forwarded call hit the network");
    let request = &requests[0];
    assert!(request.starts_with("POST /rpc HTTP/1.1\r\n"));
    let expected_host = format!("127.0.0.1:{port}");
    assert_eq!(
        header_value(request, "Host").as_deref(),
        Some(expected_host.as_str())
    );
    assert_eq!(
        header_value(request, "Authorization").as_deref(),
        Some("Bearer test-token")
    );
    assert!(request.contains("\"method\":\"get_castle\""));
}
