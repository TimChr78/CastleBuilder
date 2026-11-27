//! Localhost-only JSON-RPC 2.0 control server for the live game (M1.3).
//!
//! Architecture sections 7.2 and 7.6: `keep-mcp` (or a test/mod script) speaks
//! a small JSON-RPC protocol over a TCP socket bound **strictly to
//! `127.0.0.1`**. DNS rebinding and browser CSRF are handled by validating the
//! `Host` and `Origin` headers, and every request must carry the per-session
//! bearer token generated at startup.
//!
//! The listener thread never touches Godot. Each accepted connection runs on a
//! short-lived worker thread, parses and authorises the HTTP request under one
//! overall deadline, then hands a [`PendingRequest`] to
//! [`KeepBridge::poll_control`] on the main thread through an `mpsc` channel and
//! waits (bounded) for the reply.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Largest accepted request body (architecture section 7.6 size limit).
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// The single overall deadline for reading a request head and body, and for
/// the main thread to answer it. A slow-loris client is disconnected at this
/// point and can never stall another connection.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Cap on the HTTP request head, so a malformed client cannot exhaust memory.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Cap on simultaneous worker threads handling control connections.
const MAX_CONCURRENT_CONNS: usize = 8;

/// One JSON-RPC error as it will appear on the wire.
#[derive(Clone, Debug)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// The result of executing a control request on the main thread.
#[derive(Debug)]
pub enum RpcReply {
    Ok(Value),
    Err(RpcError),
}

/// A request forwarded from the listener thread to Godot's main thread.
#[derive(Debug)]
pub struct PendingRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<RpcReply>,
    /// Absolute instant after which the request must not be executed, even if
    /// it is still sitting in the queue when the game thread drains it (M5).
    pub deadline: Instant,
}

impl PendingRequest {
    fn is_expired(&self, now: Instant) -> bool {
        self.deadline <= now
    }
}

/// The running listener plus everything the main thread needs to drain it.
pub struct ControlServer {
    pub port: u16,
    pub token: String,
    receiver: Mutex<Receiver<PendingRequest>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ControlServer {
    /// Bind to `preferred_port` on `127.0.0.1`, or to an ephemeral port when it
    /// is busy/out of range. Never binds any other interface.
    pub fn start(preferred_port: i32) -> Result<Self, String> {
        let listener = bind_listener(preferred_port).map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let token = generate_token()?;
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel::<PendingRequest>();

        let thread_stop = Arc::clone(&stop);
        let thread_token = token.clone();
        let thread = std::thread::Builder::new()
            .name("keepwright-control".to_string())
            .spawn(move || run_listener(listener, port, thread_token, sender, thread_stop))
            .map_err(|error| error.to_string())?;

        Ok(Self {
            port,
            token,
            receiver: Mutex::new(receiver),
            stop,
            thread: Some(thread),
        })
    }

    /// `127.0.0.1:<port>` — the address the in-game panel shows.
    pub fn url(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Non-blocking drain of one request that is still within its deadline.
    /// Expired requests are answered with a timeout error and dropped without
    /// ever reaching the executor (M5).
    pub fn drain_live(&self) -> Option<PendingRequest> {
        let receiver = self.receiver.lock().ok()?;
        take_live(&receiver, Instant::now())
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the non-blocking accept loop promptly. The connection is never
        // read: the loop checks the stop flag before accepting more work.
        if let Ok(address) = format!("127.0.0.1:{}", self.port).parse::<SocketAddr>() {
            let _ = TcpStream::connect_timeout(&address, Duration::from_millis(200));
        }
        // Only the accept loop is joined. Worker threads observe `stop` and the
        // per-connection deadline, so dropping the server never blocks
        // indefinitely on a stalled client.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Pull the next request that is still within its deadline. Expired requests
/// are answered with a timeout error and dropped in place.
fn take_live(receiver: &Receiver<PendingRequest>, now: Instant) -> Option<PendingRequest> {
    loop {
        let request = receiver.try_recv().ok()?;
        if request.is_expired(now) {
            let _ = request.reply.send(RpcReply::Err(RpcError::new(
                -32_000,
                "request expired before it reached the game thread",
            )));
            continue;
        }
        return Some(request);
    }
}

/// Bind the loopback listener, falling back to an ephemeral port.
fn bind_listener(preferred_port: i32) -> std::io::Result<TcpListener> {
    if (1..=65_535).contains(&preferred_port) {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", preferred_port as u16)) {
            return Ok(listener);
        }
    }
    TcpListener::bind(("127.0.0.1", 0))
}

/// A 32-hex-character per-session bearer token from the OS CSPRNG. There is no
/// predictable fallback: if the CSPRNG fails, the control server fails closed.
fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("OS CSPRNG unavailable: {error}"))?;
    Ok(hex_16(&bytes))
}

fn hex_16(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(32);
    for byte in bytes.iter().take(16) {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn run_listener(
    listener: TcpListener,
    port: u16,
    token: String,
    sender: Sender<PendingRequest>,
    stop: Arc<AtomicBool>,
) {
    let _ = listener.set_nonblocking(true);
    let connections = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                let _ = stream.set_nonblocking(false);
                let active = connections.fetch_add(1, Ordering::SeqCst) + 1;
                if active > MAX_CONCURRENT_CONNS {
                    connections.fetch_sub(1, Ordering::SeqCst);
                    let mut stream = stream;
                    let _ = write_status(
                        &mut stream,
                        "503 Service Unavailable",
                        "too many concurrent control connections",
                        &[],
                    );
                    continue;
                }

                let worker_token = token.clone();
                let worker_sender = sender.clone();
                let worker_stop = Arc::clone(&stop);
                let worker_connections = Arc::clone(&connections);
                let spawned = std::thread::Builder::new()
                    .name("keepwright-control-conn".to_string())
                    .spawn(move || {
                        handle_connection(
                            stream,
                            port,
                            &worker_token,
                            &worker_sender,
                            &worker_stop,
                        );
                        worker_connections.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    connections.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    port: u16,
    token: &str,
    sender: &Sender<PendingRequest>,
    stop: &AtomicBool,
) {
    // One overall deadline covers the head, the body and the main-thread reply.
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));

    let (method, path, headers, leftover) = match read_request_head(&mut stream, deadline) {
        Ok(parts) => parts,
        Err(status) => {
            let _ = write_status(&mut stream, status, "", &[]);
            // Drain anything the client already sent so closing the socket
            // produces a clean FIN instead of a reset that could hide the
            // status line from a client still writing.
            let _ = stream.shutdown(std::net::Shutdown::Write);
            drain_input(&mut stream);
            return;
        }
    };

    if method != "POST" {
        let _ = write_status(
            &mut stream,
            "405 Method Not Allowed",
            "only POST /rpc is accepted",
            &[("Allow", "POST".to_string())],
        );
        return;
    }
    if path != "/rpc" {
        let _ = write_status(&mut stream, "404 Not Found", "unknown control path", &[]);
        return;
    }

    // DNS-rebinding protection: the Host header must name this exact loopback
    // authority. Anything else could be a rebinding attempt.
    let host = headers.get("host").map(String::as_str).unwrap_or("");
    if !is_allowed_host(host, port) {
        let _ = write_status(
            &mut stream,
            "403 Forbidden",
            "Host header must be 127.0.0.1 or localhost on this port",
            &[],
        );
        return;
    }

    // CSRF protection: a browser always attaches Origin to a cross-origin POST.
    // Every non-empty Origin is refused, so no web page can drive the control
    // server even if it somehow learned the bearer token.
    if headers
        .get("origin")
        .is_some_and(|origin| !origin.trim().is_empty())
    {
        let _ = write_status(
            &mut stream,
            "403 Forbidden",
            "browser Origin headers are not accepted",
            &[],
        );
        return;
    }

    if !authorised(headers.get("authorization").map(String::as_str), token) {
        let _ = write_status(
            &mut stream,
            "401 Unauthorized",
            "missing or invalid bearer token",
            &[("WWW-Authenticate", "Bearer".to_string())],
        );
        return;
    }

    if headers
        .get("transfer-encoding")
        .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        let _ = write_status(
            &mut stream,
            "411 Length Required",
            "chunked bodies are not supported",
            &[],
        );
        return;
    }

    let body_length = match headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
    {
        Some(length) => length,
        None => {
            let _ = write_status(
                &mut stream,
                "411 Length Required",
                "missing Content-Length",
                &[],
            );
            return;
        }
    };
    if body_length > MAX_BODY_BYTES {
        let _ = write_status(
            &mut stream,
            "413 Payload Too Large",
            "request body exceeds 256 KiB",
            &[],
        );
        return;
    }

    let body = match read_body(&mut stream, leftover, body_length, deadline) {
        Ok(body) => body,
        Err(status) => {
            let _ = write_status(&mut stream, status, "", &[]);
            return;
        }
    };

    let response = match serde_json::from_slice::<Value>(&body) {
        Ok(request) => route_request(request, sender, stop, deadline),
        Err(_) => error_response(
            &Value::Null,
            &RpcError::new(-32_700, "parse error: body is not valid JSON"),
        ),
    };

    let serialised = serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec());
    let _ = write_status(
        &mut stream,
        "200 OK",
        &String::from_utf8_lossy(&serialised),
        &[(
            "Content-Type",
            "application/json; charset=utf-8".to_string(),
        )],
    );
}

/// Read the request head under a strict byte cap and one overall deadline.
///
/// The `\r\n\r\n` terminator must be found within [`MAX_HEAD_BYTES`]; anything
/// longer is refused with `431` before more memory is allocated. Bytes that
/// arrive after the terminator are returned as the body prefix so the stream is
/// never over-read.
/// A parsed request head plus any body bytes that arrived with it.
type RequestHead = (String, String, HashMap<String, String>, Vec<u8>);

fn read_request_head(
    stream: &mut TcpStream,
    deadline: Instant,
) -> Result<RequestHead, &'static str> {
    let mut buffer = Vec::with_capacity(1024);
    loop {
        if let Some(position) = find_head_terminator(&buffer) {
            let head = buffer[..position].to_vec();
            let leftover = buffer[position + 4..].to_vec();
            let (method, path, headers) = parse_head(&head)?;
            return Ok((method, path, headers, leftover));
        }

        if buffer.len() >= MAX_HEAD_BYTES {
            return Err("431 Request Header Fields Too Large");
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("408 Request Timeout");
        }
        let _ = stream.set_read_timeout(Some(remaining));

        let mut chunk = [0u8; 1024];
        let allowance = (MAX_HEAD_BYTES - buffer.len()).min(chunk.len());
        match stream.read(&mut chunk[..allowance]) {
            Ok(0) => return Err("400 Bad Request"),
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err("408 Request Timeout");
            }
            Err(_) => return Err("400 Bad Request"),
        }
    }
}

fn find_head_terminator(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Parse a complete (terminator-free) request head.
fn parse_head(head: &[u8]) -> Result<(String, String, HashMap<String, String>), &'static str> {
    let text = std::str::from_utf8(head).map_err(|_| "400 Bad Request")?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("");
    if method.is_empty() || path.is_empty() || !version.starts_with("HTTP/1.") {
        return Err("400 Bad Request");
    }

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // Obsolete line folding is never accepted: it is a header-smuggling
            // vector and no legitimate client of this server sends it.
            return Err("400 Bad Request");
        }
        let (name, value) = line.split_once(':').ok_or("400 Bad Request")?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err("400 Bad Request");
        }
        // A duplicated security-relevant header is ambiguous, so refuse it
        // instead of letting one of the copies win.
        if matches!(
            name.as_str(),
            "host" | "content-length" | "authorization" | "origin"
        ) && headers.contains_key(&name)
        {
            return Err("400 Bad Request");
        }
        headers.insert(name, value.trim().to_string());
    }
    Ok((method, path, headers))
}

/// Read exactly `body_length` bytes, using the head's leftover prefix and the
/// same overall deadline as the head read.
fn read_body(
    stream: &mut TcpStream,
    mut prefix: Vec<u8>,
    body_length: usize,
    deadline: Instant,
) -> Result<Vec<u8>, &'static str> {
    prefix.truncate(body_length.min(prefix.len()));
    let mut body = prefix;
    while body.len() < body_length {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("408 Request Timeout");
        }
        let _ = stream.set_read_timeout(Some(remaining));

        let mut chunk = [0u8; 8192];
        let want = (body_length - body.len()).min(chunk.len());
        match stream.read(&mut chunk[..want]) {
            Ok(0) => return Err("400 Bad Request"),
            Ok(read) => body.extend_from_slice(&chunk[..read]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err("408 Request Timeout");
            }
            Err(_) => return Err("400 Bad Request"),
        }
    }
    Ok(body)
}

/// Discard already-buffered client bytes without blocking for long, so an
/// early HTTP error can close cleanly instead of resetting the connection.
fn drain_input(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut sink = [0u8; 4096];
    for _ in 0..16 {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

fn is_allowed_host(host: &str, port: u16) -> bool {
    let host = host.trim().to_ascii_lowercase();
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

fn authorised(header: Option<&str>, token: &str) -> bool {
    let Some(header) = header else {
        return false;
    };
    let (scheme, value) = match header.split_once(' ') {
        Some(parts) => parts,
        None => return false,
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return false;
    }
    constant_time_eq(value.trim().as_bytes(), token.as_bytes())
}

/// Branch-free length-and-bytes comparison for the bearer token.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Validate the JSON-RPC envelope, forward it to the main thread and wait for
/// the reply within the connection's overall deadline.
fn route_request(
    request: Value,
    sender: &Sender<PendingRequest>,
    stop: &AtomicBool,
    deadline: Instant,
) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return error_response(
            &id,
            &RpcError::new(-32_600, "invalid request: missing method"),
        );
    };
    if request
        .get("jsonrpc")
        .and_then(Value::as_str)
        .is_some_and(|version| version != "2.0")
    {
        return error_response(
            &id,
            &RpcError::new(-32_600, "invalid request: jsonrpc must be 2.0"),
        );
    }
    let params = match request.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(Value::Object(_)) => request.get("params").cloned().unwrap_or_else(|| json!({})),
        Some(_) => {
            return error_response(
                &id,
                &RpcError::new(-32_602, "invalid params: expected a JSON object"),
            )
        }
    };

    let (reply_sender, reply_receiver) = mpsc::channel();
    let pending = PendingRequest {
        method: method.to_string(),
        params,
        reply: reply_sender,
        deadline,
    };
    if sender.send(pending).is_err() {
        return error_response(
            &id,
            &RpcError::new(-32_000, "control server is shutting down"),
        );
    }

    loop {
        if stop.load(Ordering::Relaxed) {
            return error_response(
                &id,
                &RpcError::new(-32_000, "control server is shutting down"),
            );
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return error_response(
                &id,
                &RpcError::new(-32_000, "request timed out on the game thread"),
            );
        }
        match reply_receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(RpcReply::Ok(result)) => {
                return json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            Ok(RpcReply::Err(error)) => return error_response(&id, &error),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return error_response(&id, &RpcError::new(-32_000, "game thread dropped request"))
            }
        }
    }
}

fn error_response(id: &Value, error: &RpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": error.code, "message": error.message }
    })
}

/// Write a minimal HTTP/1.1 response and close the connection.
fn write_status(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
    extra_headers: &[(&str, String)],
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    fn http_request(port: u16, token: &str, host: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to test server");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let request = format!(
            "POST /rpc HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn raw_request(port: u16, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to test server");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(request).unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn wait_for_request(server: &ControlServer) -> PendingRequest {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(request) = server.drain_live() {
                return request;
            }
            assert!(
                Instant::now() < deadline,
                "no request reached the main thread"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn pending(method: &str, deadline: Instant) -> (PendingRequest, Receiver<RpcReply>) {
        let (reply, receiver) = mpsc::channel();
        (
            PendingRequest {
                method: method.to_string(),
                params: json!({}),
                reply,
                deadline,
            },
            receiver,
        )
    }

    #[test]
    fn host_validation_requires_the_exact_loopback_authority() {
        assert!(is_allowed_host("127.0.0.1:17731", 17_731));
        assert!(is_allowed_host("localhost:17731", 17_731));
        assert!(!is_allowed_host("127.0.0.1:9", 17_731));
        assert!(!is_allowed_host("localhost.evil.example:17731", 17_731));
        assert!(!is_allowed_host("0.0.0.0:17731", 17_731));
    }

    #[test]
    fn bearer_comparison_is_exact() {
        assert!(authorised(Some("Bearer abc123"), "abc123"));
        assert!(authorised(Some("bearer abc123"), "abc123"));
        assert!(!authorised(Some("Bearer abc124"), "abc123"));
        assert!(!authorised(Some("Basic abc123"), "abc123"));
        assert!(!authorised(None, "abc123"));
    }

    #[test]
    fn tokens_are_32_hex_characters() {
        let token = generate_token().expect("OS CSPRNG is available in the test environment");
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_ne!(token, generate_token().unwrap());
    }

    #[test]
    fn binds_only_loopback_addresses() {
        let server = ControlServer::start(0).expect("ephemeral loopback bind");
        assert!(server.port > 0);
        assert!(server.url().starts_with("127.0.0.1:"));
    }

    #[test]
    fn authorised_request_reaches_the_main_thread_and_replies() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let token = server.token.clone();
        let host = format!("127.0.0.1:{port}");
        let client = std::thread::spawn(move || {
            http_request(
                port,
                &token,
                &host,
                r#"{"jsonrpc":"2.0","id":7,"method":"get_castle","params":{}}"#,
            )
        });

        let request = wait_for_request(&server);
        assert_eq!(request.method, "get_castle");
        request
            .reply
            .send(RpcReply::Ok(json!({ "ok": true })))
            .unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#""id":7"#), "{response}");
        assert!(response.contains(r#""ok":true"#), "{response}");
    }

    #[test]
    fn foreign_hosts_origins_and_tokens_are_refused_before_dispatch() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let token = server.token.clone();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"get_castle"}"#;

        let foreign_host = http_request(port, &token, "evil.example:80", body);
        assert!(foreign_host.starts_with("HTTP/1.1 403"), "{foreign_host}");

        let bad_token = http_request(
            port,
            "00000000000000000000000000000000",
            &format!("127.0.0.1:{port}"),
            body,
        );
        assert!(bad_token.starts_with("HTTP/1.1 401"), "{bad_token}");

        assert!(
            server.drain_live().is_none(),
            "refused requests must never reach the game thread"
        );
    }

    #[test]
    fn any_non_empty_origin_is_refused() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let token = server.token.clone();
        let request = format!(
            "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:{port}\r\n\
             Authorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"
        );
        let response = raw_request(port, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(server.drain_live().is_none());
    }

    #[test]
    fn oversized_header_without_terminator_is_refused_with_431() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let request = format!(
            "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Padding: {}",
            "a".repeat(MAX_HEAD_BYTES + 512)
        );
        let response = raw_request(port, request.as_bytes());
        assert!(
            response.starts_with("HTTP/1.1 431 Request Header Fields Too Large"),
            "{response}"
        );
        assert!(server.drain_live().is_none());
    }

    #[test]
    fn duplicate_host_header_is_rejected() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let token = server.token.clone();
        let request = format!(
            "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: 127.0.0.1:{port}\r\n\
             Authorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"
        );
        let response = raw_request(port, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        assert!(server.drain_live().is_none());
    }

    #[test]
    fn duplicate_content_length_and_authorization_are_rejected() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let token = server.token.clone();
        for header in [
            "Content-Length: 0\r\nContent-Length: 0\r\n".to_string(),
            format!("Authorization: Bearer {token}\r\nAuthorization: Bearer {token}\r\n"),
        ] {
            let request = format!(
                "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n{header}\r\n"
            );
            let response = raw_request(port, request.as_bytes());
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        }
        assert!(server.drain_live().is_none());
    }

    #[test]
    fn slow_client_does_not_block_a_concurrent_valid_request() {
        let server = ControlServer::start(0).unwrap();
        let port = server.port;
        let token = server.token.clone();

        // A slow-loris client that never finishes its request head.
        let mut slow = TcpStream::connect(("127.0.0.1", port)).expect("slow client connects");
        slow.write_all(format!("POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n").as_bytes())
            .unwrap();
        slow.flush().unwrap();

        let host = format!("127.0.0.1:{port}");
        let client = std::thread::spawn(move || {
            http_request(
                port,
                &token,
                &host,
                r#"{"jsonrpc":"2.0","id":11,"method":"get_castle","params":{}}"#,
            )
        });

        let request = wait_for_request(&server);
        assert_eq!(request.method, "get_castle");
        request
            .reply
            .send(RpcReply::Ok(json!({ "ok": true })))
            .unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

        // The slow connection is still stalled but only occupies its own worker.
        drop(slow);
    }

    #[test]
    fn expired_pending_request_is_dropped_without_execution() {
        let (sender, receiver) = mpsc::channel();
        let (expired, expired_reply) = pending("apply_operations", Instant::now());
        let (live, live_reply) = pending("get_castle", Instant::now() + Duration::from_secs(5));
        sender.send(expired).unwrap();
        sender.send(live).unwrap();
        drop(sender);

        let mut executed = Vec::new();
        while let Some(request) = take_live(&receiver, Instant::now()) {
            executed.push(request.method.clone());
            let _ = request.reply.send(RpcReply::Ok(Value::Null));
        }

        assert_eq!(
            executed,
            vec!["get_castle".to_string()],
            "expired requests must never reach the executor"
        );
        match expired_reply.try_recv() {
            Ok(RpcReply::Err(error)) => assert_eq!(error.code, -32_000),
            other => panic!("expired request should receive a timeout error, got {other:?}"),
        }
        assert!(
            live_reply.try_recv().is_ok(),
            "live request should have been executed"
        );
    }
}
