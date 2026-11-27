//! `keep-mcp` binary: a synchronous line-delimited JSON-RPC loop over stdio.
//!
//! Offline mode is the default and the only mode implemented so far. Live mode
//! (driving a running game over localhost) arrives with the session layer.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use keep_core::document::CastleDocument;
use keep_core::fixtures::m1_edwardian_document;
use keep_core::session::DEFAULT_SITE_ID;
use keep_mcp::McpServer;

fn main() -> ExitCode {
    let mut empty = false;
    let mut m1 = false;
    let mut site: Option<String> = None;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--offline" => {}
            "--empty" => empty = true,
            "--m1" => m1 = true,
            "--site" => match arguments.next() {
                Some(id) => site = Some(id),
                None => {
                    eprintln!("keep-mcp: --site requires a site id");
                    return ExitCode::from(2);
                }
            },
            "--help" | "-h" => {
                println!(
                    "keep-mcp {version}\n\n\
                     Offline MCP sidecar for Keepwright. Speaks line-delimited\n\
                     JSON-RPC 2.0 over stdin/stdout.\n\n\
                     Usage: keep-mcp [--offline] [--empty] [--m1] [--site <id>]\n\n\
                     Options:\n  \
                       --offline    Use the offline keep-core backend (default)\n  \
                       --empty      Start with an empty document instead of the showcase\n  \
                       --m1         Start with the M1 Edwardian document on the coastal headland\n  \
                       --site <id>  Start empty on a curated site: m0a_promontory,\n               \
                                    coastal_headland or lake_island\n",
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

    let site_id = site.as_deref().unwrap_or(if m1 {
        "coastal_headland"
    } else {
        DEFAULT_SITE_ID
    });
    let document = if m1 {
        m1_edwardian_document()
    } else if empty || site.is_some() {
        CastleDocument::new()
    } else {
        keep_core::fixtures::m0b_showcase_document()
    };

    let Some(mut server) = McpServer::with_start(site_id, document) else {
        eprintln!(
            "keep-mcp: unknown site '{site_id}' (expected m0a_promontory, coastal_headland or lake_island)"
        );
        return ExitCode::from(2);
    };

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
