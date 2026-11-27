# Keepwright (`CastleBuilder`)

*Working title. A castle-building game about building beautiful, historically believable castles, where the act of building is the reward. AI agents are first-class users through MCP.*

## Vision

Choose a site — a rock above a river, a lake shore, a ridge — and shape a castle directly on the land like a living model. Drag a curtain wall along the contour, snap a round tower into it so the two fuse, run a wall across a ditch and an arch forms. When you commit the design, watch masons raise it course by course with climbing scaffolding, cranes, and roofs before winter.

Full architecture and implementation plan: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Design Pillars

1. **Building is the toy** — tactile drag, snap, and visual/audio feedback.
2. **Shape instantly, build theatrically** — instant design mode separated from the construction theater show.
3. **Historical grammar, not historical homework** — style packs encode real masonry and regional rules.
4. **Intent in, masonry out** — players and agents edit the semantic Castle Document; a deterministic Rust generator lays out every stone.
5. **Agents are players too** — full MCP sidecar (`keep-mcp`) with proposals, in-game agent panel, and co-play readiness from M1.
6. **Castle scale first** — polish what reads at 30–150 m and during construction; zero budget for paid art/sound (procedural + CC0 + self-made).

## Tech Stack (Pinned in `tools/versions.toml`)

- **Engine / Renderer / UI:** Godot 4.7 (Forward+), GDScript for glue/UI
- **Core Model & Procedural Masonry:** Rust (`core/keep-core`, engine-agnostic, deterministic)
- **Engine Binding:** `godot-rust` (`gdext` 0.5.6, `api-4-7`) in thin crate `core/keep-gd`
- **Agent Interface:** Rust MCP sidecar (`core/keep-mcp`) + shared protocol (`core/keep-proto`)
- **Hero Assets:** Blender 5.3 (headless glTF 2.0 export)

## Repository Layout

```text
CastleBuilder/
├── AGENTS.md / CLAUDE.md   # Agent build, test, and workflow instructions
├── tools/versions.toml     # Pinned Godot, gdext, Blender, and Rust versions
├── docs/ARCHITECTURE.md    # Full v4.1 architecture & milestone plan
├── core/                   # Rust workspace (keep-core, keep-gd, keep-mcp, keep-proto)
├── game/                   # Godot 4.7 project (scripts, shaders, UI, minimal scenes, tests)
├── assets-src/             # .blend hero sources (Git LFS), export scripts, manifest
├── styles/                 # Historical style packs with source notes
├── sites/                  # Curated terrain sites
├── templates/              # Historical castle layouts
└── evals/                  # Agent evaluation briefs and graders
```

## Current Milestone: M1 Vertical Slice

The vertical slice now runs end to end. M0a, M0b and M1.1–M1.4 are in place:

- **M0a — look & performance:** thin `keep-core` → `keep-gd` → merged chunk `ArrayMesh` pipeline in Godot 4.7 Forward+, baked hemisphere/sky vertex AO, the first limestone ashlar shader, shadowed golden-hour lighting and a 200,000-block stress mode with LODs inside the draw-call budget.
- **M0b — junctions & feel:** 3D massing with wall-to-round-tower junctions, splayed arrow loops through thick walls, live drag preview with snapping, rising construction, scaffolding, dust and procedural audio, plus offline `keep-mcp`.
- **M1.1 — session & save:** host-authoritative in-process session, unified command bus, author-tagged command log, compensating undo, and `castle.keep.json` save/load.
- **M1.2 — sites, style & massing:** three curated sites (river promontory, coastal headland, lake island) with terrain biomes and water; the Edwardian Welsh style pack; round, square and D-shaped towers, gatehouses and ditches; structural validation with one-click fixes.
- **M1.3 — live MCP & agent panel:** loopback-only control server with a bearer token, `keep-mcp --live` forwarding, proposal ghost overlays, `render_view`, and the in-game **Agents & MCP** panel for presence, permissions and Accept/Reject.
- **M1.4 — licence manifest:** every shipped shader, procedural audio generator, style pack and curated site is recorded in `assets-src/LICENCE_MANIFEST.json`; `docs/CREDITS.md` is generated from it.

### Live MCP

Start the game, then read the control-server URL and token from the **Agents & MCP** panel (also printed to the console). Point `keep-mcp` at it:

```bash
cargo run --manifest-path core/Cargo.toml -p keep-mcp -- \
  --live http://127.0.0.1:17731 --token <token> --permission proposal-only
```

Agents connect over stdio with any MCP client. Live agents default to proposal-only: edits appear as ghosts for the player to accept or reject. Use `--offline --m1` for headless design, validation and plan rendering without a game window.

### Keyboard Controls

- **RMB / MMB drag** orbit · **Shift+drag** pan · **wheel** zoom
- **WASD / arrow keys** pan · **Q/E** orbit · **R/F** tilt · **+/−** zoom · **Home** reset view
- **Ctrl+Z** undo
- **LMB drag** draws a wall or ditch · **LMB click** places a tower or gatehouse (choose the tool in the HUD first)
- HUD buttons switch M0a/M0b/M1 modes and sites, play or pause the build front, save/load, undo, and show the structural score with one-click fixes.

### Licence Check

```bash
python3 tools/check_licences.py --check
python3 tools/check_licences.py --write-credits
```

`--check` fails on a missing path, an unapproved or non-commercial licence on a shipped asset, an AI-generated shipped asset, or a stale `docs/CREDITS.md`.

## Quick Commands

```bash
# Run core Rust tests
cargo test --manifest-path core/Cargo.toml

# Build GDExtension library
cargo build --manifest-path core/Cargo.toml -p keep-gd

# Build and run the MCP sidecar (offline vertical-slice document)
cargo run --manifest-path core/Cargo.toml -p keep-mcp -- --offline --m1

# Run Godot headless checks
godot --headless --path game --quit
godot --headless --path game --script res://tests/run.gd

# Validate the licence manifest and generated credits
python3 tools/check_licences.py --check
```
