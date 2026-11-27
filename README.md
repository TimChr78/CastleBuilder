# Keepwright (`CastleBuilder`)

*Working title. A castle-building game about building beautiful, historically believable castles, where the act of building is the reward. AI agents are first-class users through MCP.*

## Vision

Choose a site — a rock above a river, a lake shore, a ridge — and shape a castle directly on the land like a living model. Drag a curtain wall along the contour, snap a round tower into it so the two fuse, run a wall across a ditch and an arch forms. When you commit the design, watch masons raise it course by course with climbing scaffolding, cranes, and roofs before winter.

Full architecture and milestone plan: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Release 1 Features (M0–M4)

Release 1 is a single-player castle builder with any number of MCP agents working alongside the player through proposals.

- **M0a — look and performance.** Thin `keep-core` → `keep-gd` → merged chunk `ArrayMesh` pipeline in Godot 4.7 Forward+, baked hemisphere/sky vertex AO, procedural limestone ashlar shader, shadowed golden-hour lighting, and a 200,000-block stress scene with geometric LODs inside the draw-call budget.
- **M0b — junctions and feel.** 3D massing with wall-to-round-tower junctions, angled wall-to-wall seams, splayed arrow loops through thick walls, live drag preview with snapping, rising construction, scaffolding, dust, procedural audio, and offline `keep-mcp`.
- **M1 — vertical slice.** Host-authoritative session, unified command bus, author-tagged command log, compensating undo and `castle.keep.json` save/load; three curated sites with terrain biomes and water; Edwardian Welsh style pack; round, square and D-shaped towers, gatehouses, ditches and curtain walls; structural validation with one-click fixes; loopback-only live MCP with bearer token, proposal ghosts, `render_view`, and the in-game **Agents & MCP** panel; licence manifest and generated credits.
- **M2 — depth and a second style.** Baltic brick style pack, keeps, roofed hall/chapel/kitchen ranges with roof clipping, the historian's authenticity/defensive/domestic critique, the full agent panel (request inbox, activity feed, Ask for changes), and the offline agent eval suite.
- **M3 — commissions.** Patron commissions with budgets, seasons, winter frost and thatching, deterministic events, mid-build edit salvage, authoritative terrain sculpting, dollhouse cutaway with interior props, photo mode, and the historical codex.
- **M4 — Release 1 hardening.** Deterministic `keep-bench` performance benchmark, modding documentation, AI co-builder/MCP guide, licence and attribution pass, and this README.

## Design Pillars

1. **Building is the toy** — tactile drag, snap, and visual/audio feedback.
2. **Shape instantly, build theatrically** — instant design mode separated from the construction theater show.
3. **Historical grammar, not historical homework** — style packs encode real masonry and regional rules.
4. **Intent in, masonry out** — players and agents edit the semantic Castle Document; a deterministic Rust generator lays out every stone.
5. **Agents are players too** — a full MCP sidecar with proposals, an in-game agent panel, request inbox and activity feed.
6. **Castle scale first** — polish what reads at 30–150 m and during construction; zero budget for paid art/sound (procedural + CC0 + self-made).

## Tech Stack (Pinned in `tools/versions.toml`)

- **Engine / Renderer / UI:** Godot 4.7 (Forward+), GDScript for glue and UI
- **Core Model & Procedural Masonry:** Rust (`core/keep-core`, engine-agnostic, deterministic)
- **Engine Binding:** `godot-rust` (`gdext` 0.5.6, `api-4-7`) in the thin crate `core/keep-gd`
- **Agent Interface:** Rust MCP sidecar (`core/keep-mcp`) plus shared protocol (`core/keep-proto`)
- **Hero Assets:** Blender 5.3 (headless glTF 2.0 export)

## Repository Layout

```text
CastleBuilder/
├── AGENTS.md / CLAUDE.md   # Agent build, test, and workflow instructions
├── tools/versions.toml     # Pinned Godot, gdext, Blender, and Rust versions
├── docs/
│   ├── ARCHITECTURE.md     # Full v4.1 architecture & milestone plan
│   ├── MODDING.md          # Style packs, sites, eval briefs and licensing
│   ├── MCP_GUIDE.md        # AI co-builder and MCP client guide
│   └── CREDITS.md          # Generated asset attribution
├── core/                   # Rust workspace (keep-core, keep-gd, keep-mcp, keep-proto)
│   └── keep-core/src/bin/keep_bench.rs  # keep-bench Release 1 benchmark
├── game/                   # Godot 4.7 project (scripts, shaders, UI, minimal scenes, tests)
├── assets-src/             # .blend hero sources (Git LFS), export scripts, manifest
├── styles/                 # Historical style packs with source notes
├── sites/                  # Curated terrain site metadata
├── templates/              # Historical castle layouts
└── evals/                  # Agent evaluation briefs and offline grader
```

## Build and Run

```bash
# Rust workspace tests
cargo test --manifest-path core/Cargo.toml

# Build the GDExtension library (debug / release)
cargo build --manifest-path core/Cargo.toml -p keep-gd
cargo build --manifest-path core/Cargo.toml -p keep-gd --release

# Build and launch the game
godot --path game

# Godot headless verification
godot --headless --path game --quit
godot --headless --path game --script res://tests/run.gd
```

Use the HUD to switch M0a/M0b/M1/M2 showcase modes and curated sites, play or pause the build front, change construction speed, save/load, undo, sculpt terrain, open photo mode or the codex, and start a commission.

## Benchmarks

`keep-bench` builds the four canonical fixtures and reports blocks, chunks,
LOD0 triangles, build time and layout hash:

```bash
cargo run --release --manifest-path core/Cargo.toml -p keep-core --bin keep-bench
cargo run --release --manifest-path core/Cargo.toml -p keep-core --bin keep-bench -- --json
```

Example release run on the development machine (timings vary by hardware; layout hashes are deterministic):

```text
fixture       site               blocks  chunks  lod0_tris  build_ms       layout_hash
m0a_showcase  m0a_promontory      1,257      10      7,626      21.6  bce911fd87bcbc12
m1_edwardian  coastal_headland    4,683      10     29,334      52.1  2df25a9fcd7b3e59
m2_baltic     lake_island        12,087      21     74,904     118.2  2b672544cc874763
stress_200k   m0a_promontory    185,062     274  1,116,260    2026.4  496fe4ddb99cc7fe
```

## Tests and Evaluations

```bash
# Rust formatting and lints
cargo fmt --manifest-path core/Cargo.toml --check
cargo clippy --manifest-path core/Cargo.toml --all-targets

# Offline agent evals (replays briefs through keep-mcp --offline)
python3 evals/run_evals.py
python3 evals/run_evals.py --json
python3 evals/run_evals.py --brief baltic_island_castle

# Licence manifest and generated credits
python3 tools/check_licences.py --check
python3 tools/check_licences.py --write-credits
```

The eval runner auto-detects a debug or release `keep-mcp`, builds one if needed, and exits non-zero unless every selected brief passes its structure, historian, element-count and SVG checks. See [`evals/README.md`](evals/README.md).

## Keyboard and Mouse Controls

- **RMB / MMB drag** orbit · **Shift+drag** pan · **wheel** zoom
- **WASD / arrow keys** pan · **Shift** boost pan · **Q/E** orbit · **R/F** tilt · **+/−** zoom · **Home** reset view
- **LMB drag** draws a wall or ditch · **LMB click** places a tower or gatehouse (choose the tool in the HUD first)
- **Ctrl+Z** undo · **P** photo mode · **C** historical codex · **Esc** close photo mode or codex

The HUD buttons switch showcase/site modes, start and pause construction, adjust speed and time-lapse, save/load, undo, sculpt terrain, show the structural score with one-click fixes, and open the commission and agent panels.

## AI Co-Builder and MCP

Start the game and open the **Agents & MCP** panel. It shows the loopback control URL and a per-session bearer token; press **Copy Token**, then point `keep-mcp` at the running game:

```bash
cargo run --release --manifest-path core/Cargo.toml -p keep-mcp -- \
  --live http://127.0.0.1:17731 --token <token> --permission proposal-only
```

Live agents default to proposal-only: their edits appear as ghosts and only the human can accept or reject them. For headless design, validation, plan rendering and the eval suite:

```bash
cargo run --release --manifest-path core/Cargo.toml -p keep-mcp -- --offline --empty --site coastal_headland
```

Connect Claude Desktop, Cursor, OpenCode or any custom MCP client over stdio. The guide covers all 23 tools, the resource surface, proposal and request workflows, and the loopback/token/Origin security model: [`docs/MCP_GUIDE.md`](docs/MCP_GUIDE.md).

## Modding and Licensing

Style packs (`styles/*.json`), site metadata (`sites/*.json`) and agent eval briefs (`evals/briefs/*.json`) are plain data with documented fields. The modding guide covers authoring and registering each, running the eval suite, and the zero-budget asset policy: [`docs/MODDING.md`](docs/MODDING.md).

Every shipped asset is recorded in [`assets-src/LICENCE_MANIFEST.json`](assets-src/LICENCE_MANIFEST.json) and attributed in [`docs/CREDITS.md`](docs/CREDITS.md). Only procedural, self-made, MIT, CC0-1.0 and CC-BY-4.0 assets ship; AI-generated assets are excluded from shipped builds.
