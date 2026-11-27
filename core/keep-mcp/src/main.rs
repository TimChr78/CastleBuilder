//! `keep-mcp` binary: a synchronous line-delimited JSON-RPC loop over stdio.
//!
//! Offline mode links `keep-core` and is the default. Live mode
//! (`--live <loopback-url> --token <token>`) forwards tool calls to the running
//! game's local control server over HTTP/1.1; the URL/token can also come from
//! `KEEP_CONTROL_URL` / `KEEP_CONTROL_TOKEN`.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use keep_core::document::CastleDocument;
use keep_core::fixtures::{m1_edwardian_document, m2_baltic_document};
use keep_core::proposal::AgentPermission;
use keep_core::session::DEFAULT_SITE_ID;
use keep_mcp::McpServer;

fn main() -> ExitCode {
    let mut empty = false;
    let mut m1 = false;
    let mut m2 = false;
    let mut site: Option<String> = None;
    let mut live: Option<String> = None;
    let mut token: Option<String> = None;
    let mut permission: Option<AgentPermission> = None;
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--offline" => {}
            "--empty" => empty = true,
            "--m1" => m1 = true,
            "--m2" => m2 = true,
            "--site" => match arguments.next() {
                Some(id) => site = Some(id),
                None => {
                    eprintln!("keep-mcp: --site requires a site id");
                    return ExitCode::from(2);
                }
            },
            "--live" => match arguments.next() {
                Some(url) => live = Some(url),
                None => {
                    eprintln!("keep-mcp: --live requires a loopback URL");
                    return ExitCode::from(2);
                }
            },
            "--token" => match arguments.next() {
                Some(value) => token = Some(value),
                None => {
                    eprintln!("keep-mcp: --token requires a bearer token");
                    return ExitCode::from(2);
                }
            },
            "--permission" => {
                match arguments.next().as_deref() {
                    Some("read-only") | Some("read_only") => {
                        permission = Some(AgentPermission::ReadOnly)
                    }
                    Some("proposal-only") | Some("proposal_only") => {
                        permission = Some(AgentPermission::ProposalOnly)
                    }
                    Some("direct-edit") | Some("direct_edit") => {
                        permission = Some(AgentPermission::DirectEdit)
                    }
                    Some(other) => {
                        eprintln!(
                        "keep-mcp: unknown permission '{other}' (expected read-only, proposal-only or direct-edit)"
                    );
                        return ExitCode::from(2);
                    }
                    None => {
                        eprintln!("keep-mcp: --permission requires read-only, proposal-only or direct-edit");
                        return ExitCode::from(2);
                    }
                }
            }
            "--help" | "-h" => {
                println!(
                    "keep-mcp {version}\n\n\
                     MCP sidecar for Keepwright. Speaks line-delimited JSON-RPC 2.0\n\
                     over stdin/stdout.\n\n\
                     Usage: keep-mcp [--offline] [--empty] [--m1|--m2] [--site <id>]\n\
                            keep-mcp --live <url> --token <token> [--permission <level>]\n\n\
                     Options:\n  \
                       --offline              Use the offline keep-core backend (default)\n  \
                       --empty                Start with an empty document instead of the showcase\n  \
                       --m1                   Start with the M1 Edwardian document on the coastal headland\n  \
                       --m2                   Start with the M2 Baltic brick document on the lake island\n  \
                       --site <id>            Start empty on a curated site: m0a_promontory,\n                              \
                                              coastal_headland or lake_island\n  \
                       --live <url>           Forward tool calls to a running game at a loopback URL\n                              \
                                              (http://127.0.0.1:<port> or http://localhost:<port>)\n  \
                       --token <token>        Bearer token for the live control server\n                              \
                                              (or set KEEP_CONTROL_URL / KEEP_CONTROL_TOKEN)\n  \
                       --permission <level>   read-only, proposal-only or direct-edit.\n                              \
                                              Defaults to proposal-only with --live, direct-edit offline\n",
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

    let live = live.or_else(|| std::env::var("KEEP_CONTROL_URL").ok());
    let token = token.or_else(|| std::env::var("KEEP_CONTROL_TOKEN").ok());

    let mut server = if let Some(url) = live {
        // Proposal-only is the safe default for a newly connected live agent
        // (architecture section 7.6).
        let permission = permission.unwrap_or(AgentPermission::ProposalOnly);
        let Some(token) = token else {
            eprintln!("keep-mcp: live mode requires --token or KEEP_CONTROL_TOKEN");
            return ExitCode::from(2);
        };
        match McpServer::live(&url, &token, permission) {
            Ok(server) => server,
            Err(message) => {
                eprintln!("keep-mcp: {message}");
                return ExitCode::from(2);
            }
        }
    } else {
        let permission = permission.unwrap_or(AgentPermission::DirectEdit);
        let site_id = site.as_deref().unwrap_or(if m2 {
            "lake_island"
        } else if m1 {
            "coastal_headland"
        } else {
            DEFAULT_SITE_ID
        });
        let document = if m2 {
            m2_baltic_document()
        } else if m1 {
            m1_edwardian_document()
        } else if empty || site.is_some() {
            CastleDocument::new()
        } else {
            keep_core::fixtures::m0b_showcase_document()
        };

        let Some(server) = McpServer::with_start_permission(site_id, document, permission) else {
            eprintln!(
                "keep-mcp: unknown site '{site_id}' (expected m0a_promontory, coastal_headland or lake_island)"
            );
            return ExitCode::from(2);
        };
        server
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
