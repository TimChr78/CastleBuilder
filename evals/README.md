# Keepwright Offline Agent Evals

Deterministic, zero-dependency evals for castle-design agents. Each brief is a
JSON scenario with a natural-language prompt, style/site selection, grading
thresholds and a **reference tool trace**. The runner replays that trace through
the real `keep-mcp --offline` stdio sidecar, then grades the resulting castle
with the same tools an external agent would use.

No LLM API key, network access or GPU is required. Python 3 (standard library
only) plus a built `keep-mcp` binary is enough.

## Run it

```bash
# Auto-detect (or cargo build) core/target/debug/keep-mcp, then run every brief
python3 evals/run_evals.py

# Machine-readable report on stdout (build logs stay on stderr)
python3 evals/run_evals.py --json

# One brief by id or filename stem
python3 evals/run_evals.py --brief baltic_island_castle

# Use an explicit binary
python3 evals/run_evals.py --mcp-bin core/target/release/keep-mcp
```

Exit status is `0` only when all selected briefs pass. The runner auto-detects
`core/target/debug/keep-mcp` or `core/target/release/keep-mcp`; if neither
exists it runs:

```bash
cargo build --manifest-path core/Cargo.toml -p keep-mcp
```

The summary table shows brief id, style, element count, structure score,
historian score, structural error count, SVG bytes and PASS/FAIL.

## Brief format

Briefs live in [`briefs/`](briefs) and are loaded in sorted filename order.

```json
{
  "id": "edwardian_coastal_fort",
  "title": "Edwardian Coastal Headland Fort",
  "description": "Human-readable design summary.",
  "style_id": "edwardian_welsh",
  "site_id": "coastal_headland",
  "prompt": "The natural-language brief handed to an AI agent.",
  "thresholds": {
    "min_structure_score": 100,
    "min_historian_score": 85,
    "max_structure_errors": 0,
    "min_elements": 14,
    "require_svg_plan": true
  },
  "reference_steps": [
    {
      "tool": "apply_operations",
      "arguments": { "operations": [ ... ] }
    }
  ]
}
```

- `style_id` is passed to `get_assessment`; valid values are `edwardian_welsh`
  and `baltic_brick`.
- `site_id` is passed to the sidecar as `keep-mcp --offline --site <id>`, so the
  brief always starts from an empty document on the chosen curated site. Valid
  ids are `m0a_promontory`, `coastal_headland` and `lake_island`.
- `reference_steps` are ordinary MCP `tools/call` steps. They may create walls,
  towers, gatehouses, keeps, ranges, ditches and arrow loops, then modify
  elements by their deterministic ids. A trace is replayed in a fresh sidecar
  per brief, so it never depends on another brief's state.

## Grading

After replaying the reference steps, the runner calls:

1. `get_assessment` with the brief's `style_id`
2. `list_elements`
3. `render_plan` with `{"view": "top_plan"}`

It then checks:

| Check | Source |
| --- | --- |
| `structure_score >= min_structure_score` | `get_assessment.score` |
| `structure_errors <= max_structure_errors` | `get_assessment.findings` with severity `error` |
| `historian_score >= min_historian_score` | `get_assessment.historian.overall_score` |
| `element_count >= min_elements` | `list_elements.element_count` |
| valid SVG | `render_plan.svg` starts with `<svg` and ends with `</svg>` |

All four bundled briefs are expected to produce a structure score of `100`,
zero structural errors and a non-empty top-plan SVG.

## Plugging in an external LLM agent

The reference trace exists so CI can grade the *pipeline* deterministically.
To compare a real agent, give it the brief's `prompt` and access to the same MCP
server, for example:

```bash
keep-mcp --offline --site coastal_headland
```

The agent should discover tools with `tools/list` and edit through
`apply_operations`, exactly like the reference trace. When it has finished,
score its castle with the same five checks above. You can reuse a brief's
`thresholds` unchanged, or copy a brief and point its `reference_steps` at the
agent's recorded tool calls to turn a successful run into a regression fixture.

Protocol notes for custom clients:

- Newline-delimited JSON-RPC 2.0 over stdin/stdout.
- `initialize`, then the `notifications/initialized` notification, then
  `tools/call`.
- Tool payloads arrive in `result.structuredContent`; failures set
  `result.isError` and put the message in `content[0].text`.
- `render_view` requires live mode; offline briefs use `render_plan`.
