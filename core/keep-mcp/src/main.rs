//! `keep-mcp` binary: a synchronous line-delimited JSON-RPC loop over stdio.
//!
//! Offline mode is the default and the only mode implemented for M0b. Live mode
//! (driving a running game over localhost) arrives with the session layer.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use keep_mcp::McpServer;

fn main() -> ExitCode {
    let mut empty = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--offline" => {}
            "--empty" => empty = true,
            "--help" | "-h" => {
                println!(
                    "keep-mcp {version}\n\n\
                     Offline MCP sidecar for Keepwright. Speaks line-delimited\n\
                     JSON-RPC 2.0 over stdin/stdout.\n\n\
                     Usage: keep-mcp [--offline] [--empty]\n\n\
                     Options:\n  \
                       --offline  Use the offline keep-core backend (default, required for M0b)\n  \
                       --empty    Start with an empty document instead of the M0b showcase\n",
                    version = env!("CARGO_PKG_VERSION")
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("keep-mcp: unknown argument '{other}' (try --help)");
                return ExitCode::from(2);
            }
        }
    }

    let mut server = McpServer::new(empty);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("keep-mcp: failed to read stdin: {error}");
                return ExitCode::FAILURE;
            }
        };
        if let Some(response) = server.handle_line(&line) {
            if writeln!(out, "{response}")
                .and_then(|()| out.flush())
                .is_err()
            {
                // The client closed the pipe; nothing left to do.
                break;
            }
        }
    }

    ExitCode::SUCCESS
}
