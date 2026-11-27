//! Minimal JSON-RPC 2.0 client for the running game's loopback control server.
//!
//! Live mode must not pull in an HTTP stack or an async runtime, so this module
//! speaks HTTP/1.1 over a plain [`std::net::TcpStream`]: one request per
//! connection with `Connection: close`, a bearer token and a JSON-RPC body.
//!
//! Security is deliberately narrow (architecture section 7.6): only
//! `http://127.0.0.1:<port>` and `http://localhost:<port>` are accepted, so a
//! `--live` URL can never point the sidecar at a remote host.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{json, Value};

/// Read/write timeout for one control request.
const IO_TIMEOUT: Duration = Duration::from_secs(15);
/// Refuse absurdly large responses rather than buffering them forever.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// A synchronous JSON-RPC client bound to a loopback control server.
#[derive(Debug)]
pub struct LiveClient {
    /// Host exactly as written in the URL; used verbatim in the `Host` header.
    host: String,
    port: u16,
    token: String,
    next_id: u64,
}

impl LiveClient {
    /// Validate `url` and prepare a client. Rejects every non-loopback URL.
    pub fn connect(url: &str, token: &str) -> Result<Self, String> {
        let (host, port) = parse_loopback_url(url)?;
        if token.is_empty() {
            return Err("live control token is empty".to_string());
        }
        if token.chars().any(char::is_control) {
            return Err("live control token contains control characters".to_string());
        }
        Ok(Self {
            host,
            port,
            token: token.to_string(),
            next_id: 1,
        })
    }

    /// The loopback host used in the `Host` header (`127.0.0.1` or `localhost`).
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The validated loopback port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Call one control method and return its JSON-RPC `result`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();

        let request = format!(
            "POST /rpc HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Authorization: Bearer {token}\r\n\
             Content-Type: application/json\r\n\
             Connection: close\r\n\
             Content-Length: {length}\r\n\
             \r\n\
             {body}",
            host = self.host,
            port = self.port,
            token = self.token,
            length = body.len(),
        );

        // Connect to the IPv4 loopback explicitly so `localhost` cannot resolve
        // to an unrelated address, while the Host header keeps the URL spelling.
        let address = SocketAddr::from(([127, 0, 0, 1], self.port));
        let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)
            .map_err(|error| format!("failed to connect to the live game: {error}"))?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|error| format!("failed to configure the live socket: {error}"))?;
        stream
            .write_all(request.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|error| format!("failed to send the live request: {error}"))?;

        let mut bytes = Vec::new();
        // Bound the read itself: a hostile or buggy server must not make the
        // sidecar buffer an unbounded response before the size check (M1.3 Low).
        (&mut stream)
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read the live response: {error}"))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("live response exceeded the size limit".to_string());
        }

        let (status, body) = parse_http_response(&bytes)?;
        // Surface transport failures as HTTP status errors before trying to
        // parse the body, so 401/403/431 are not misreported as bad JSON-RPC
        // (M1.3 Low).
        if !(200..300).contains(&status) {
            return Err(http_status_error(status));
        }
        let value: Value = serde_json::from_slice(body)
            .map_err(|error| format!("live response was not valid JSON-RPC: {error}"))?;

        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown control error");
            return Err(format!("control server error: {message}"));
        }
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            return Err("live response id did not match the request".to_string());
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "live response had no result".to_string())
    }
}

/// Human-readable transport error for a non-2xx response, with a hint for the
/// statuses a misconfigured live connection is most likely to hit.
fn http_status_error(status: u16) -> String {
    let hint = match status {
        400 => " (bad request: the live control server rejected the request)",
        401 => " (unauthorized: the live control token was rejected)",
        403 => " (forbidden: this agent is not allowed to use the live control server)",
        404 => " (not found: the game's /rpc control endpoint is not available)",
        431 => " (request header fields too large: the live control token is too long)",
        _ => "",
    };
    format!("live control server returned HTTP {status}{hint}")
}

/// Strictly accept `http://127.0.0.1:<port>` or `http://localhost:<port>`.
pub fn parse_loopback_url(url: &str) -> Result<(String, u16), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!("live URL '{url}' must start with http:// and point at 127.0.0.1 or localhost")
    })?;
    let (host, port) = rest
        .rsplit_once(':')
        .ok_or_else(|| format!("live URL '{url}' must include an explicit loopback port"))?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err(format!(
            "live URL '{url}' must point at 127.0.0.1 or localhost (got '{host}')"
        ));
    }
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("live URL '{url}' has an invalid port"));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| format!("live URL '{url}' has an out-of-range port"))?;
    if port == 0 {
        return Err(format!("live URL '{url}' has an invalid port"));
    }
    Ok((host.to_string(), port))
}

/// Parse the status line, headers and body of one HTTP/1.1 response.
fn parse_http_response(bytes: &[u8]) -> Result<(u16, &[u8]), String> {
    let separator = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "live response had no HTTP header terminator".to_string())?;
    let header_text = std::str::from_utf8(&bytes[..separator])
        .map_err(|_| "live response headers were not UTF-8".to_string())?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "live response had no status line".to_string())?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(format!(
            "unexpected live response status line: {status_line}"
        ));
    }
    let status: u16 = status_parts
        .next()
        .ok_or_else(|| "live response had no status code".to_string())?
        .parse()
        .map_err(|_| "live response had an invalid status code".to_string())?;

    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    value
                        .trim()
                        .parse()
                        .map_err(|_| "live response had an invalid Content-Length".to_string())?,
                );
            }
        }
    }

    let body = &bytes[separator + 4..];
    let body = match content_length {
        Some(length) if body.len() >= length => &body[..length],
        Some(_) => return Err("live response body was shorter than Content-Length".to_string()),
        // No Content-Length: `Connection: close` means read to EOF.
        None => body,
    };
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_urls_are_accepted_with_host_and_port() {
        assert_eq!(
            parse_loopback_url("http://127.0.0.1:8765").unwrap(),
            ("127.0.0.1".to_string(), 8765)
        );
        assert_eq!(
            parse_loopback_url("http://localhost:1").unwrap(),
            ("localhost".to_string(), 1)
        );
    }

    #[test]
    fn non_loopback_and_malformed_urls_are_rejected() {
        for url in [
            "https://127.0.0.1:8765",
            "http://example.com:8765",
            "http://192.168.1.5:8765",
            "http://0.0.0.0:8765",
            "http://[::1]:8765",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "http://127.0.0.1:99999",
            "http://127.0.0.1:8765/rpc",
            "http://user@127.0.0.1:8765",
            "ftp://127.0.0.1:8765",
        ] {
            assert!(parse_loopback_url(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn tokens_are_validated() {
        assert!(LiveClient::connect("http://127.0.0.1:8765", "").is_err());
        assert!(LiveClient::connect("http://127.0.0.1:8765", "bad\ntoken").is_err());
        let client = LiveClient::connect("http://localhost:8765", "secret").unwrap();
        assert_eq!(client.host(), "localhost");
        assert_eq!(client.port(), 8765);
    }

    #[test]
    fn http_responses_parse_with_and_without_content_length() {
        let with_length =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let (status, body) = parse_http_response(with_length).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{}");

        let without_length = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}";
        let (status, body) = parse_http_response(without_length).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{\"a\":1}");

        assert!(parse_http_response(b"not http\r\n\r\n").is_err());
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n{}").is_err());
    }

    #[test]
    fn non_2xx_statuses_produce_informative_transport_errors() {
        assert!(http_status_error(401).contains("HTTP 401"));
        assert!(http_status_error(401).contains("token was rejected"));
        assert!(http_status_error(403).contains("forbidden"));
        assert!(http_status_error(431).contains("header fields too large"));
        assert!(http_status_error(500).contains("HTTP 500"));
    }
}
