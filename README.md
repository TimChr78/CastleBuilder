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

## Current Milestone: M0a (Look & Performance Spike)

- Thin `keep-core` → `keep-gd` → merged chunk `ArrayMesh` pipeline in Godot 4.7 Forward+
- One battered curtain wall + one round tower on a fixed terrain at golden hour
- Rust-baked hemisphere/sky vertex AO + first limestone ashlar shader with procedural bevels
- **Kill criteria:** holds up next to reference photography at a glance, and **200,000 blocks** render at **60 fps** with LODs within the `< 2,000` draw-call budget.

## Quick Commands

```bash
# Run core Rust tests
cargo test --manifest-path core/Cargo.toml

# Build GDExtension library
cargo build --manifest-path core/Cargo.toml -p keep-gd

# Run Godot headless checks
godot --headless --path game --quit
```
