# AI Co-Builder and MCP Guide

Keepwright treats AI agents as first-class players. The deterministic Rust core
owns the Castle Document and the masonry; an external agent connects over the
Model Context Protocol (MCP) and edits either its own headless castle or the
live game through a human-approved proposal flow.

- **`keep-mcp`** is a stdio MCP sidecar (line-delimited JSON-RPC 2.0).
- **`--offline`** runs a full headless castle, ideal for design, validation and
  the eval suite.
- **`--live`** forwards tool calls to the running game's loopback control
  server, so the player can watch an agent work and accept or reject changes.

Build it once:

```bash
cargo build --release --manifest-path core/Cargo.toml -p keep-mcp
```

The binary is `core/target/release/keep-mcp`.

## Connect to the live game

1. Start Keepwright and open the **Agents & MCP** panel.
2. The panel shows the loopback control URL and a per-session bearer token.
   Press **Copy Token**.
3. Start the sidecar with that URL and token:

```bash
core/target/release/keep-mcp \
  --live http://127.0.0.1:17731 \
  --token <token> \
  --permission proposal-only
```

The URL and token can instead be supplied through `KEEP_CONTROL_URL` and
`KEEP_CONTROL_TOKEN`:

```bash
export KEEP_CONTROL_URL=http://127.0.0.1:17731
export KEEP_CONTROL_TOKEN=<token>
core/target/release/keep-mcp --live "$KEEP_CONTROL_URL" --token "$KEEP_CONTROL_TOKEN"
```

`--live` defaults to `proposal-only`. Supported permission levels are
`read-only`, `proposal-only` and `direct-edit`; the human can also change a
connected agent's permission in the panel. A newly connected live agent should
stay on `proposal-only` unless the player deliberately grants direct editing.

Point any MCP client at the sidecar's stdin/stdout. For example, Claude Desktop
or Cursor:

```json
{
  "mcpServers": {
    "keepwright": {
      "command": "/absolute/path/to/CastleBuilder/core/target/release/keep-mcp",
      "args": [
        "--live",
        "http://127.0.0.1:17731",
        "--token",
        "<token>",
        "--permission",
        "proposal-only"
      ]
    }
  }
}
```

OpenCode (`opencode.json`):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "keepwright": {
      "type": "local",
      "command": [
        "/absolute/path/to/CastleBuilder/core/target/release/keep-mcp",
        "--live",
        "http://127.0.0.1:17731",
        "--token",
        "<token>",
        "--permission",
        "proposal-only"
      ],
      "enabled": true
    }
  }
}
```

The token is a secret for the current game session. Do not commit it, paste it
into a shared transcript, or expose the control port beyond loopback.

## Run headless (`--offline`)

Offline mode needs no game window and defaults to direct editing:

```bash
# Curated M0a showcase document on the river promontory
core/target/release/keep-mcp --offline

# Empty castle on a chosen curated site
core/target/release/keep-mcp --offline --empty --site coastal_headland
core/target/release/keep-mcp --offline --empty --site lake_island

# Named showcase documents
core/target/release/keep-mcp --offline --m1   # Edwardian coastal headland
core/target/release/keep-mcp --offline --m2   # Baltic brick lake island
```

Curated site ids are `m0a_promontory`, `coastal_headland` and `lake_island`.
Offline mode exposes the same read, edit, assessment, codex, commission and
proposal tools as live mode, plus the `keep://castle/save` resource. It has no
`render_view` (which needs the running game's camera); use `render_plan` for a
deterministic SVG top plan or north elevation instead.

The offline sidecar is exactly what `python3 evals/run_evals.py` drives, so an
agent and the eval suite see the same protocol surface.

## The 23 tools

### Read and assess

| Tool | What it does |
| --- | --- |
| `get_castle` | Castle summary: document and layout hashes, block and element counts, element and opening lists. |
| `list_elements` | Elements with ids, kinds and quantised dimensions. |
| `render_plan` | Deterministic SVG `top_plan` or `elevation_north`, with optional horizontal or vertical section. |
| `get_history` | Append-only command history with sequence numbers, authors and undone markers. |
| `get_assessment` | Structural findings and one-click fixes plus the historian's authenticity, defensive and domestic scores. Optional `style_id`; defaults to `edwardian_welsh`. |
| `list_styles` | Every curated style pack compiled into this build. |
| `get_style` | One style pack by id, including rules and source notes. |
| `get_site` | The active site and every curated site with name, description, latitude and landforms. |
| `list_codex` | Historical codex entries in reading order. |
| `get_codex_entry` | One codex entry by id, with body, citations and related style packs. |
| `list_commissions` | Curated patron briefs: patron, site, style, starting silver, season limit, workforce and goals. |
| `get_commission_status` | The active commission's budget, season, frost, current event and remaining goals, or the free-build state. |

### Edit

| Tool | What it does |
| --- | --- |
| `apply_operations` | Apply one atomic batch of create/add/delete operations. Later operations can reference earlier ones by `ref_id` or `wall_ref`. |
| `undo` | Append a compensating command that undoes this author's latest still-live batch. Refused if a later command touched the same elements. |

Operations cover wall runs, round/square/D-shaped towers, gatehouses, keeps,
roofed ranges (hall, chapel, kitchen), ditches, arrow loops and terrain
sculpting. Coordinates are integer millimetres.

### Proposal workflow

| Tool | What it does |
| --- | --- |
| `begin_proposal` | Open a draft overlay. Nothing touches the live castle yet. |
| `stage_proposal_ops` | Stage another atomic batch on the draft. Each batch is replayed on a fork to compute the semantic diff and structural delta. |
| `submit_proposal` | Submit a non-empty draft for human review. |

Only the human can accept or reject a submitted proposal in the **Agents &
MCP** panel. Acceptance is the only path from a proposal to the live castle.

### Request inbox and activity

| Tool | What it does |
| --- | --- |
| `list_requests` | Read the player's request inbox: status, author, optional proposal/element link and reply. |
| `ack_request` | Claim an open request so the player can see an agent is working on it. |
| `reply_request` | Resolve an open or acknowledged request, optionally leaving a reply. |
| `get_activity_feed` | Recent session activity (requests, proposals, agents and tool use), newest first. Default limit 50, maximum 200. |
| `list_proposals` | Every proposal overlay with status, semantic diff and structural delta. |

### Render and review

| Tool | What it does |
| --- | --- |
| `render_view` | Live only: render the running game's camera as a PNG. |

A typical live session is: `get_castle` → `render_plan`/`render_view` →
`begin_proposal` → `stage_proposal_ops` → `submit_proposal` → wait for the
human. Use `list_proposals` to poll status and `list_requests` /
`get_activity_feed` to see what the player is asking for.

## Resources

Resources are read with `resources/read`. Release 1 exposes twelve concrete
URIs grouped into seven families:

| Family | URI | Contents |
| --- | --- | --- |
| Castle | `keep://castle/document` | The full Castle Document JSON. |
| Castle | `keep://castle/save` | Portable `castle.keep.json` save. Offline only. |
| Castle plans | `keep://castle/plan/top` | SVG top plan. |
| Castle plans | `keep://castle/plan/elevation` | SVG north elevation. |
| Assessment | `keep://castle/assessment` | Structural and historian assessment. |
| Styles | `keep://styles/edwardian_welsh` | Edwardian Welsh style pack. |
| Styles | `keep://styles/baltic_brick` | Baltic brick style pack. |
| Codex | `keep://codex` | All historical codex entries. |
| Codex | `keep://codex/{id}` | One codex entry by id. |
| Session | `keep://session/requests` | The player request inbox. |
| Session | `keep://session/activity` | The activity feed. |
| Session | `keep://session/commission` | Active commission status. |

Use the concrete URIs above rather than bare collection names: `keep://castle`,
`keep://styles` and `keep://sites` are not separate resources. Castle state is
read from `keep://castle/document`, style packs from their
`keep://styles/<id>` URIs, and sites with the `get_site` tool.
`keep://castle/save` is available only in `--offline` mode.

## Security model

- **Loopback only.** The live control server binds `127.0.0.1` (or `localhost`)
  and never a public interface. `keep-mcp` rejects non-loopback URLs.
- **Per-session bearer token.** Every request carries the token shown by the
  **Agents & MCP** panel. Restarting the game rotates the session.
- **Host and Origin checks.** The control server requires an exact loopback
  `Host` authority and refuses any non-empty browser `Origin`, which blocks
  DNS-rebinding and cross-site request forgery.
- **Proposal-only by default.** A live agent starts with proposal rights, not
  direct editing. `read-only` and `direct-edit` are explicit choices.
- **Human-only Accept/Reject.** Agents can create, stage and submit proposals,
  but only the player can accept or reject them. Submitted proposals never
  mutate the live document on their own.
- **Per-author compensating undo.** Undo appends a compensating command for the
  caller's latest batch only, and is refused when a later command has touched
  the same elements. It cannot rewrite another author's work.
- **Untrusted text stays text.** Agent names, request replies and other free
  text are delimited and length-limited in the UI and protocol; they are data,
  not instructions.

For the full architecture and milestone context, see
[`ARCHITECTURE.md`](ARCHITECTURE.md). For style, site, eval and licence data
formats, see [`MODDING.md`](MODDING.md).
