# Keepwright: architecture and implementation plan

*Working title. A castle-building game about building beautiful, historically believable castles, where the act of building is the reward. Agents are first-class users through MCP.*

Status: **v4 (final), 2026-11-26. Ready to start M0a.** v1 was reviewed by Gemini 4 Neon ([REVIEW-gemini.md](REVIEW-gemini.md)); v2 adopted most of that review (Appendix A). v3 recorded the owner's decisions on scope, co-play, interiors, platform and budget (Appendix B). v4 applies Gemini's final review ([REVIEW-gemini-2.md](REVIEW-gemini-2.md), Appendix C). Earlier versions: [v1](ARCHITECTURE-v1.md), [v2](ARCHITECTURE-v2.md), [v3](ARCHITECTURE-v3.md).

To be built on the developer's own PC by one developer working with coding agents. **Target platform: PC only** (Windows first; a Linux build is kept working because Godot makes it cheap). **No budget for paid art or sound**: everything is procedural, self-made, or free-licensed (section 9.1).

---

## 1. What we are making

### 1.1 The pitch
You choose a site: a rock above a river, a lake shore, a ridge. You shape the castle on the land like a living model: drag a curtain wall along the crest and it hugs the contour, pull a tower against it and the two fuse, run a wall across a ditch and an arch forms. When you're happy, you commit it and watch it built: masons lay it course by course, scaffolding climbs with it, a treadwheel crane swings ashlar into place, and the roofs go on before winter. At the end you have a castle that looks like it could have stood since 1290, and a time-lapse of it going up.

### 1.2 Design pillars
1. **Building is the toy.** Every drag, snap, course and roof should feel good to watch and hear. Speed of feedback beats depth of economy.
2. **Shape instantly, build theatrically.** Design is immediate and tactile; construction is a show you can speed up, slow down or skip. The two are separate on purpose (section 8).
3. **Historical grammar, not historical homework.** The game knows how a curtain wall, a mural tower, a machicolation or a stepped brick gable goes together, and does the detail for you. You make the architectural decisions; the generator gets the masonry right.
4. **Intent in, masonry out.** The player (or an agent) edits *what* the castle is. A deterministic generator decides *how* each stone sits.
5. **Agents are players too.** Anything a human can do through the UI, an agent can do through MCP, under the same rules and undo, with proposals the human approves. Building together, with other players and with agents in the same session, is on the roadmap for an early release (section 15), and the architecture is ready for it from the start.
6. **Castle scale first.** Polish what reads at 30 to 150 m and in the construction show. Sub-metre detail gets attention only where the camera lingers (gates, wall-walks, the dollhouse cutaway). We don't spend weeks on the smallest details.

### 1.3 What it is not
- Not a siege or combat game at launch. Defensive logic is expressed through the historian's assessment (section 6).
- Not a colony sim. Workers exist to make construction visible and paced, not to be micromanaged.
- Not a freeform voxel builder. You shape elements; you don't place single stones.
- Not a walk-through interior game. Interiors are a cutaway "dollhouse" view (section 8.5).

### 1.4 Modes
- **Sandbox:** unlimited resources; build speed from instant to real seasons.
- **Commissions** (campaign): a patron, a site, a budget, a deadline in seasons, a period and style. Inspired by real building programmes, not reconstructions of them.
- **Study mode:** a curated historical layout as a reference ghost to build within.

---

## 2. Technology choices

| Concern | Choice | Why |
|---|---|---|
| Engine / renderer / UI | **Godot 4.x**, one pinned minor version, Forward+ | Text formats, headless CLI, GDExtension, small and open. |
| Core model, generators, terrain authority | **Rust** crate `keep-core`, no engine dependencies | Castle model, terrain data, geometry, validation and planning are fast, deterministic and testable with `cargo test` anywhere. |
| Engine binding | **godot-rust (gdext)**, pinned to the Godot version; a deliberately thin crate `keep-gd` | Only commands in, mesh buffers and events out. 90% of iteration happens without Godot running. |
| Gameplay glue, tools, UI | **GDScript**, building UI and scene trees in code where possible | Agents edit scripts reliably; hand-edited `.tscn` files with `uid://` and sub-resource ids are fragile under agents, so scenes stay small. |
| Terrain rendering | Custom chunked/CDLOD heightfield mesh + splat shader fed from `keep-core` (no Terrain3D) | `keep-core` owns height, bedrock and earthwork masks so live play, validation and offline MCP see the same ground. |
| Assets | **Blender, one pinned version**, Geometry Nodes for hero-piece variants, headless glTF 2.0 export with a version check | Reproducible, agent-runnable asset builds. |
| Materials | Texture-array PBR (stone, brick, timber, slate, lead), detail normals, weathering masks. CC0 scans (Poly Haven, ambientCG, ShareTextures) plus procedural materials baked from Blender or Material Maker | Realism comes from materials and light more than polygons; all free (9.1). |
| Audio | Godot audio buses; procedural layering in code; CC0 and royalty-free libraries plus self-recorded foley | Construction sound is half the satisfaction; all free (9.1). |
| Agent interface | **MCP sidecar** `keep-mcp` (Rust; stdio and Streamable HTTP) over a local JSON-RPC control protocol | Section 7. |
| Save format | Versioned JSON castle document + its command log + regenerable binary cache | Diffable and readable by agents; the log is what co-play syncs (section 15). |
| Networking (co-play release) | Godot high-level multiplayer (ENet) on LAN/direct IP, Steam networking relays through the free GodotSteam integration for release | Only commands travel, so bandwidth is tiny. |
| Source control | git + git LFS | |

### 2.1 Why not Unreal or Unity
Unreal 5 has the higher visual ceiling (Nanite, Lumen) but loses on agent-friendliness (binary `.uasset`, long C++ builds) and solo iteration speed. Bevy lacks a mature editor, UI and renderer. **Godot plus a Rust core** keeps the important files as text and the important code testable.

The visual ceiling is the real risk of this choice. It is tested first (M0a) and managed by an art direction of *grounded, painterly realism* and a lighting approach that doesn't depend on Godot's dynamic GI (section 9). Because the core is engine-agnostic, the renderer could be replaced later without rewriting the game's heart.

### 2.2 Version pinning
Godot, gdext, Blender and the Rust toolchain are pinned in the repo (`rust-toolchain.toml`, a `tools/versions.toml` checked by build scripts). Upgrades are deliberate, one at a time, on a branch. On Windows, GDExtension DLLs lock during editor sessions; the dev loop is "cargo test, then restart the game", not editor hot reload.

---

## 3. System overview

```
             ┌───────────────────────── Godot game process ─────────────────────────┐
 Player ───► │  Tools & UI (GDScript)                                                 │
             │      │ commands                                                        │
 Agent ──MCP─►keep-mcp ─JSON-RPC (localhost)─► Control server ─► Command bus          │
             │                                                     │                  │
             │                    ┌────────── keep-core (Rust, via thin keep-gd) ────┐│
             │                    │ Castle Document (semantic model, command log)     ││
             │                    │ Site: heightfield, bedrock, earthwork masks       ││
             │                    │ Style packs & reactive architectural rules        ││
             │                    │ Massing: 3D solids, voids, junction graph         ││
             │                    │ Masonry layout (per element) → chunk meshing      ││
             │                    │ Validation: structure + historian                 ││
             │                    │ Construction planner & simulator                  ││
             │                    │ Plan/elevation renderer (SVG/PNG)                 ││
             │                    └───────────────────────────────────────────────────┘│
             │      │ mesh buffers, terrain tiles, build events                        │
             │  Renderer: terrain, chunks, LOD, VFX, audio, camera, photo mode          │
             └───────────────────────────────────────────────────────────────────────┘
```

Rules that hold the design together:
1. **The Castle Document is the single source of truth.** Meshes, schedules and reports are derived and cacheable.
2. **Every change is a Command on the bus.** Player tools, MCP agents, tests and campaign events all produce the same commands: validated, undoable, attributed, logged.
3. **Generation is deterministic.** Same document, seed and generator version give the same layout. Layout decisions (course heights, block splits, ordering) are made on quantized values with `libm` maths, so they agree across Windows and Linux; final vertex positions are compared with tolerance, not byte for byte.
4. **Generation is incremental.** An edit re-lays out the touched element and its junction neighbours, then re-meshes only the dirty chunks.
5. **Picking and spatial queries go through `keep-core`.** Raycasts against the castle and terrain don't depend on the GPU, so headless tests and offline agents behave like the live game.
6. **Single player is a session of one.** From M1 the game runs through a session layer: an in-process host owns the document and orders commands, and the local client sends commands to it. Co-play later swaps the in-process link for a network one instead of retrofitting the core (section 15).

---

## 4. The Castle Document (semantic model)

The document describes architecture, not geometry. It is what the player edits, what agents read and write, and what is saved.

### 4.1 Entities
- **Site:** terrain reference, seed, latitude (sun), period and region defaults, named landforms (ridge crest, cliff edge, river, approach), computed on load.
- **Element** (base type): id, human name, style ref, phase, overrides.
  - `WallRun`: path on terrain, height, thickness, batter, plinth, wall-walk, parapet, crenellation rhythm, facing (ashlar, coursed rubble, brick bond), core, openings.
  - `Tower`: plan (round, D, square, polygonal), diameter, height, floors, roof, projection, attachment.
  - `Gatehouse`: passage, flanking towers, portcullis, murder holes, drawbridge.
  - `Keep`: shell, square, round, tower house.
  - `Range` (hall, chapel, kitchen): footprint, storeys, roof, chimneys, windows.
  - `Earthwork`: ditch, moat, motte, glacis, causeway.
  - `Feature` (attached): window, arrow loop with embrasure, garderobe, machicolation, hoarding, bartizan, chimney, stair turret, banner.
- **Anchors:** positions can be absolute or relative ("on `north_curtain` at 0.35 along", "at the junction of A and B", "on the ridge crest, snapped to contour"). Relative anchors survive edits, and they are how agents place things without doing 3D trigonometry.
- **Junction graph:** where elements meet, solved by priority (section 5.3), with per-junction overrides (toothed bond, straight joint, butt joint).
- **Phase:** ordered building campaigns so a castle can grow over centuries.

### 4.2 Representation
- In memory: Rust structs with stable ids, an arena and an operation log.
- On disk: versioned JSON (`castle.keep.json`) with a published JSON Schema and migrations between versions.
- **History is a linear, host-ordered command log.** Every command carries an author (player or agent id) and globally unique ids (UUIDv7).
- **Undo appends a compensating command** ("undo my last command"). It is allowed only if no later command touched the same element, an anchor that refers to it, or a shared junction; otherwise it is refused with a reason. In single player this feels like ordinary undo; in co-play it never undoes other people's work.
- **Proposals and what-ifs are overlay diffs** evaluated on a forked in-memory copy of the document, not branches of the history. Accepting one applies it as a single batch of commands.
- **Canonical ordering:** sorted collections (`BTreeMap`, sorted ids) everywhere in `keep-core`, and massing/junction inputs quantized to integer millimetres, so the same document gives the same layout on every machine.
- Semantic diff ("added 2 towers, raised north curtain by 2 m, historian +6") produced in `keep-core` and reused by UI, MCP, proposals and replays.
- Free-text fields (names, notes) are length- and character-limited, because they are returned to agents (section 7.6).

---

## 5. Procedural masonry: intent to stones

The heart of the game and its biggest technical risk. v2 replaces v1's 2D face layout with a 3D massing stage, generates stones in Rust instead of instancing rigid meshes, and merges chunks into single meshes.

### 5.1 Pipeline
1. **Resolve style:** element params + style pack → concrete rules (course height range, block length distribution, bond, joint width, quoin treatment, parapet module, splay angles).
2. **Massing (3D solids and voids):** each element becomes solids (an outer and an inner masonry shell; tower shells; plinths and batters stepping down bedrock) and voids (gate passages, embrasures with splayed reveals, wall-walk). The rubble core is not modelled as a solid: it appears as a procedural top cap on the active build lift and as the cut fill in the cutaway. Thick-wall openings are real voids through both shells. Mural stairs are door portals plus a simple volume, not carved stairwells.
3. **Junction solve:** the junction graph (5.3) trims solids, routes the continuous wall-walk (inserting tower doors and steps), and yields seam curves.
4. **Course planes:** courses are horizontal world-space planes, stepping with the bedrock and plinth, not rectangles mapped onto a face. Courses on sloping sites step down in levelling increments, as built walls do.
5. **Block layout, per element:** each course is cut where it meets each trimmed surface. The resulting curves are divided into blocks following the bond, with quoins at corners, voussoirs around arches, jambs, sills and lintels around voids, and dressing strips along seams. Runs across chunk boundaries, so a 4 m gate arch or a running bond is never cut by a chunk seam. Target: under 2 ms per typical element.
6. **Build order:** every block gets a build index (lift, course, then working-face order), which drives the construction animation.
7. **Block meshing:** blocks are generated in Rust as jittered prisms conforming to curvature and batter, so round towers don't gap on the outside or interpenetrate on the inside. Only exposed faces are emitted (about 6 to 10 triangles per ordinary block); edge bevels come from the shader and normal maps. Geometric bevels are reserved for trim (quoins, voussoirs, coping) and for LOD0 within about 15 m. Surface detail comes from shared detail-normal and texture-array layers, not unique geometry.
8. **Chunk meshing:** blocks are bucketed into spatial chunks and merged on worker threads (drag preview in 8.1, mid-build edits in 8.4).

### 5.2 Rendering the masonry
- **One merged `ArrayMesh` per chunk**, one shared texture-array material: one draw call per chunk (plus shadows).
- Per-vertex custom attributes carry `build_index`, `age`, tint and a jitter seed. The vertex shader animates stones rising and settling and the fragment shader does weathering, both on the GPU with no per-frame uploads. The construction "front" is a uniform the shader compares against `build_index`.
- **LOD:** LOD0 full blocks near the camera; LOD1 a simplified chunk shell with triplanar masonry shading and baked per-vertex AO; LOD2/HLOD a per-element massing mesh for far views. No per-chunk baked textures (VRAM).
- **MultiMesh** only for repeated props at castle scale: scaffolding poles, foliage, scattered stone piles.
- Budget check, mid-range PC at 1440p and 60 fps: under 2,000 draw calls including shadow passes, about 2 M triangles in view after LOD. M0a has to show this on 200,000 blocks.

### 5.3 Junctions
Real junctions are three- and four-way (two curtains, a battered round tower, a hall range against the wall, a stair turret), so pairwise "who owns it" is not enough. Junctions are solved as a **priority-ordered spatial graph**: keep/tower > gatehouse > curtain > range > feature, with overrides. The solver handles:
- volumetric trimming, including plane-against-battered-cone seams;
- roof clipping planes, so roofs never poke through parapets;
- wall-walk topology (doors through towers, steps, landings);
- seam dressing: quoins, straight joints or toothing along the solved seam curve, and merlon rhythm fitted to end cleanly at towers.

### 5.4 Realism details that pay off
Only details that read at castle scale or in the construction show:
- Putlog holes where scaffolding was; building breaks between seasons' lifts.
- Weathering by age and orientation (base staining, streaks under openings, lichen on north faces, mortar loss), driven by vertex data so castles visibly age.
- Distinct facings by phase, so additions read as additions.
- Correct merlon proportions and rhythm per style.

### 5.5 Authoring split with Blender
- Rust generates all ordinary blocks.
- Rust also generates the simple repeated parts: roof tiles and slates, timber floors and plain trusses, scaffolding poles and boards, merlons and plain crenellations.
- Blender makes a small set of **hero pieces**: arrow-loop and window frames, a portcullis and doors, corbels, one or two tracery windows, the treadwheel crane, carts. Aim for about 10 per style, built parametrically with Geometry Nodes so one model gives several variants. **This is human Blender time**, budgeted as such: agents automate export, LOD generation and manifest checks, but don't sculpt tasteful stonework blind.
- **Workers** are stylized low-poly cloaked silhouettes with simple procedural bobbing and hammering motion. No full rigging or character animation: they read at 30 to 150 m and cost a couple of days.
- Each asset is exported headless with a manifest entry (semantic tags, dimensions, sockets, LODs, licence), so the generator and agents can find assets by meaning.

---

## 6. Validation: structure and the historian

Advisory in sandbox, binding in commissions.

### 6.1 Structural plausibility (simple, legible)
Checks a medieval master mason would recognise, not finite elements:
- wall slenderness (height to thickness) within the style's range;
- footing on bedrock versus soil, slope under footprint, required plinth or batter;
- corbels for machicolations and bartizans; jetty limits;
- openings too many or too wide low in a wall;
- roof spans against timber; vaults need abutment.

Failures show as overlays with plain-language reasons and a one-click "make it sound" fix ("This 14 m wall is only 1.2 m thick; the style expects 2.0 to 3.5 m. Thicken to 2.2 m?").

### 6.2 The historian
A rules-based critic for the chosen period and region:
- **Authenticity:** features and materials in or out of period and region.
- **Defensive logic:** flanking coverage (towers within bowshot, dead ground), gate depth, approach visibility.
- **Domestic logic:** hall, kitchen, water, chapel, access.

Output is a short report with scores and concrete suggestions, phrased as tendencies, each rule citing its source note (section 10). Available to players and through `get_assessment`.

---

## 7. Agents and MCP

### 7.1 Who the agents are
1. **Co-builder agents:** any MCP client (Claude, Gemini, others) designing or extending castles at the player's request.
2. **Developer agents:** coding agents building the game through the repo, headless tests and dev-time tooling.
3. *(Post-1.0, optional)* in-game characters backed by an LLM. Cut from 1.0: API keys, cost and privacy are painful in a shipped game, and MCP plus the deterministic historian covers the need.

### 7.2 Architecture
```
MCP client ──stdio or Streamable HTTP──► keep-mcp (Rust sidecar)
                                           ├─ live mode: JSON-RPC over localhost WebSocket ─► running game
                                           └─ offline mode: links keep-core directly (no GPU)
```
- `keep-mcp` is a separate binary so clients can spawn it over stdio and it survives game restarts.
- **Live mode** drives a running game; renders are real.
- **Offline mode** links `keep-core`; agents design, validate, diff and render plans and elevations (7.4) without a game window. CI and agent evals use it.
- The JSON-RPC control protocol is also used by tests and modding scripts; MCP is a thin translation over it, not a second API.

### 7.3 Tools (semantic, batchable, relative)
Each tool has an `outputSchema` and annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`), so hosts can auto-approve reads and renders and gate edits.

Read:
- `get_site`: named landforms, approach vectors, a compact elevation/slope/bedrock grid, heights along a path.
- `get_castle` (summary at a chosen detail level), `list_elements`, `get_element`.
- `get_assessment` (structure + historian), `get_build_status`.
- `list_styles`, `get_style`, `search_assets`.
- `render_view` (live): camera preset or pose, time of day, overlays; returns compact JPEG/WebP (default about 768×512) with optional annotations: element ids, compass, scale bar.
- `render_plan` (live and offline): plan, elevation or section as SVG/PNG from `keep-core`.

Write (all via the command bus; all accept relative anchors):
- `apply_operations`: an atomic batch of create/modify/delete operations, with ids assignable inside the batch so later operations can reference earlier ones. This is the main agent tool; a castle is a handful of calls, not a hundred.
- Single-operation conveniences: `create_wall_run`, `create_tower`, `create_gatehouse`, `create_range`, `create_earthwork`, `add_feature`, `modify_element`, `delete_element`, `set_junction`.
- `apply_template`: a curated historical layout fitted to the site.

Workflow:
- `begin_proposal` → edits → `submit_proposal`. The game shows a ghost overlay, the semantic diff and the historian delta, and **the human accepts or rejects in-game**. Where the host supports MCP elicitation, the approval can also be asked in the client. Agents with proposal-only permission cannot commit; `commit_proposal` exists only at direct-edit permission.
- `undo`, `redo`, `get_history`.
- `set_build_speed`, `start_timelapse`, `export_timelapse`.

### 7.4 Feedback loops for agents
- Live: `render_view` for real visuals.
- Offline: `render_plan` (pure Rust, SVG/PNG) for plans, elevations and sections, plus the assessment. Good enough for an agent to see that a tower is off the ridge or a wall skips a junction; it's not for judging beauty, which needs live renders.

### 7.5 Resources, subscriptions and long tasks
- Resource templates: `keep://castle/document`, `keep://castle/elements/{id}`, `keep://castle/assessment`, `keep://styles/{id}`, `keep://reference/sites/{slug}`, `keep://schema/castle`.
- `resources/subscribe` with update notifications, so an agent reacts to human edits and build milestones without polling.
- Long operations (fast-forwarding a season, exporting a time-lapse, regenerating a large castle) use MCP's task and progress mechanisms where the client supports them, falling back to job ids. Check this against the MCP spec version current at implementation time, since tasks were still new in late 2025.

### 7.6 Security
- **Bound to 127.0.0.1 only.** Streamable HTTP requires a bearer token, rejects unexpected `Origin` headers and validates `Host` (against DNS rebinding and browser CSRF). The control socket uses a per-session token.
- Permission per connected agent: read-only, proposal-only (default), direct-edit.
- Size and rate limits (no 10,000-element batch); long operations become tasks.
- Every agent command is attributed in the history.
- **Untrusted text:** names and notes from shared saves and mod style packs are length-limited and returned in clearly delimited data fields, never mixed into instruction-like text, to blunt prompt injection.
- No file-system or arbitrary-code tools in the game's MCP surface.

### 7.7 In-game agent panel
The player works with agents from inside the game; the MCP client is how the agent connects, not where the player has to work.
- **Presence:** connected agents with name, colour, permission level and state (idle, thinking, building a proposal, waiting for you). A clear "not listening" state when the agent's client isn't running a loop that reads requests.
- **Request box:** the player types a request ("add a barbican in front of the gate, Edwardian style"), optionally pinned to a selection or a point on the map. Requests go to a session inbox that agents read through an MCP resource with subscription (`keep://session/requests`) and acknowledge with a tool call. Replies and status lines from the agent show in the panel's activity feed.
- **Proposals:** each proposal card shows the agent's summary, the semantic diff, the historian and structure deltas, and buttons to fly the camera to it, toggle its ghost, **Accept**, **Reject**, or **Ask for changes** (sends a follow-up request tied to that proposal).
- **Activity feed:** what the agent has read, rendered and proposed, so its work isn't a black box; each entry links to the history.
- **Controls:** change an agent's permission, pause it (its commands are refused until resumed) or disconnect it.
- **Co-play:** in Release 2 every participant sees the panel; requests and proposals show who asked and who can approve.
- Scope: the panel is a GDScript UI over the session and the proposal system, which exist anyway. It doesn't contain an LLM; the player brings their own agent (Claude, Gemini or any MCP client). Basic version (presence, proposals, Accept/Reject) in M1; request box, activity feed and Ask for changes in M2.

### 7.8 Dev-time agent tooling
- `AGENTS.md` / `CLAUDE.md` with build, test and asset commands.
- Headless entry points: `cargo test` (core), `godot --headless --script res://tests/run.gd` (command flows only; no GPU picking), `blender -b -P tools/export_assets.py`, `keep-mcp --offline` (agent evals).
- GPU tests (screenshots, shader animation, LOD transitions) run on the dev PC, not headless.
- Community Godot and Blender MCP servers are worth evaluating for dev loops. Verify their maturity first; don't depend on them.

---

## 8. Making building satisfying

v1 put a slow construction simulation between the player and the result. v2 separates the two.

### 8.1 Design mode: instant and tactile
- Dragging shows the **real masonry preview live**, at reduced LOD, with springy motion: the wall settles to the contour, a tower snaps against a curtain with a resonant thud.
- **Reactive architectural rules** (from the style pack) make the castle respond:
  - a wall dragged across a ditch forms a bridge arch;
  - a tower near a curtain snaps and fuses, adding the wall-walk door and, where the style has it, a stair turret;
  - a wall raised past a threshold sprouts a wall-walk and crenellations;
  - two ranges meeting form a clean roof valley;
  - towers on a run space themselves to the style's flanking distance when you hold a modifier.
- **Terrain hints while dragging:** ridge contours, bedrock highlights, bowshot flanking arcs from existing towers, dead ground shading. The land should invite architecture.
- Live readouts: length, height, cost, historian and structure deltas.
- Undo is instant and visual.

### 8.2 Construction theater: the show
Commit a design (or a phase) and it is built:
- **Speed:** pause, 1x to 100x, or "raise instantly" (sandbox).
- **Set-out:** strings and pegs, then trenches.
- **Rising walls:** stones arrive and settle with a small drop and dust in build order, on the active lift. Mortar lines darken as they cure.
- **Scaffolding** climbs with the wall and is struck, leaving putlog holes.
- **Cranes** swing loads; the camera can follow one.
- **Life for scale:** small masons on scaffolds, lime-kiln smoke, carts, swallows in putlog holes, banners, warm light in loops at dusk.
- **Completion beats:** last merlon, final slate, banner raised, each with a short sound and light sting.
- **Camera:** orbit, a cinematic "watch" mode that picks interesting work, and photo mode.
- **Time-lapse:** build events are recorded and can be replayed from any camera path (in-game replay in M1; video export later).

### 8.3 Sound
- Zoom-reactive mix: close shows chisels, trowels and crane creak; wide shows wind, birds, distant rhythmic hammering.
- Procedural feedback: snaps and placements with pitch variation tied to size and material, and a rising pitch sequence as a wall extends. An experiment to tune by ear, since it can become a gimmick.
- Weather and season ambience.

### 8.4 Editing a castle under construction
Changing an element mid-build diffs its block list: unchanged lower courses stay built, changed blocks are taken down (animated, salvaged as material) and re-laid. This works because layout is deterministic per element.

### 8.5 Dollhouse interiors
Interiors are a cutaway view, not walk-throughs:
- Toggle a section plane (horizontal by storey, or vertical along a wall or tower). A shader clip plane cuts the masonry, and a flat cross-section fill with a rubble texture shows wall thickness. No cut geometry.
- Interiors stay architectural: floors, vaults, hearths, door portals, from the massing stage. One shared set of 5 to 8 low-poly props (table, bench, bed, altar, cauldron, barrel, chest) is placed by room type.
- Room types come from elements and the historian's domestic logic, so the cutaway also explains how the castle worked.
- The same section planes feed `render_plan` sections for agents, so one feature serves both.
- Small people living in the rooms (cooks, guards) are a polish item, not a milestone.

### 8.6 Pacing and economy (commissions)
Design freely before committing a phase. Budget in money and labour per season, the patron's patience, events (hard winter, quarry flood, war tax). Seasonal effects (no masonry in frost, wall tops thatched over winter) come in with commissions, not M1. Kept shallow: the economy frames building decisions; it isn't a spreadsheet game.

---

## 9. Rendering and art direction

- **Look:** grounded painterly realism, checked against the curated reference photos in this research folder (Bodiam, Eltz, Castell y Bere, Belvoir, Ballycarbery, Glimmingehus, Borgholm, Bohus).
- **Lighting, without depending on dynamic GI:**
  - Rust-baked sky visibility and hemisphere AO per chunk vertex, computed on worker threads against the heightfield and the castle;
  - SSAO for contact detail;
  - sky-fill and a directional bounce term in the shader;
  - reflection probes and local lights for gate passages, courtyards and interiors;
  - cascaded shadow splits tuned to camera altitude, because panoramas and close-ups want very different splits.
  - If Godot's GI improves in the pinned version, use it as a bonus, not a foundation.
- **Atmosphere:** volumetric fog, low cloud on hill sites, rain wetting stone.
- **Materials:** one stone family done extremely well first (limestone ashlar + rubble core), then sandstone, granite, flint, red and glazed brick, lime render, slate, lead, oak.
- **Vegetation:** grass and shrubs at M1; ivy growth on aged chunks later.
- **Art and audio are human-paced.** Agents speed up code, not taste. With no budget, scope is kept small and procedural (9.1).

### 9.1 Zero-budget art and audio
- **Procedural first:** stones, bricks, roofs, timber, scaffolding and terrain are generated, so the hand-made set is small (5.5).
- **Textures:** CC0 only for core materials (Poly Haven, ambientCG, ShareTextures), plus procedural materials baked from Blender or Material Maker.
- **Models:** the about-10-per-style hero kit, made in Blender; CC0 models (for example Poly Haven) for generic props.
- **Sound:** self-recorded foley (tapping and scraping real stone, wood and rope with a phone or cheap recorder gives authentic, free construction sound), CC0 sounds from Freesound (filtered by licence), and the royalty-free game-audio bundles that are released free. Ambience-led mix (wind, birds, rain, stone foley). Music optional: medieval *compositions* are public domain but modern *recordings* usually aren't, so only verified CC0 or CC-BY tracks, or self-made arrangements rendered through free soundfonts.
- **Licence manifest:** every third-party file has an entry (source, author, licence, URL), and the game's credits and attribution page are generated from it, like the existing castle reference set. Anything CC-BY gets credited; non-commercial (NC) licences are excluded because the game will be sold.
- **AI-generated assets:** not in shipped builds by default (dev placeholders only, tagged in the manifest). Procedural and CC0 cover the need, and Steam's AI disclosure is a reputational risk for a craft-focused game. Owner may revisit.

---

## 10. Using the research corpus

The research folder holds work on medieval castle typology, construction techniques and regional traditions, site reports, and a licensed photo reference set. It feeds the game as:
- **Style packs:** for example "Edwardian Welsh, 1277–1300", "Baltic brick, 1350–1450", "Swedish Vasa royal castle, 1550–1600", "Irish tower house, 1400–1600". Each encodes course sizes, materials, feature sets, proportions, reactive rules and historian rules, every rule with a source note. Packs are data and moddable.
- **Templates and study ghosts:** simplified plans of reference sites.
- **In-game codex:** short entries with licensed photos and attributions, reachable from the historian's comments.
- **MCP resources:** the same material for agents.

---

## 11. Repository layout

```
keepwright/
  AGENTS.md / CLAUDE.md
  tools/versions.toml     # pinned Godot, gdext, Blender versions
  core/                   # Rust workspace
    keep-core/            # document, site, styles, massing, masonry, validation, planner, plan renderer
    keep-gd/              # thin gdext bindings
    keep-mcp/             # MCP sidecar, live and offline
    keep-proto/           # control protocol types + JSON Schema
  game/                   # Godot project: scripts/, ui/, shaders/, audio/, small scenes/, tests/
  assets-src/             # .blend sources (LFS), export scripts, asset manifest
  styles/                 # style packs with source notes
  sites/                  # curated terrain sites
  templates/              # historical layouts
  docs/                   # design docs, ADRs
  evals/                  # agent eval briefs and graders
```

---

## 12. Testing and quality

- **Core:** unit and property tests: no overlapping blocks, closed courses, voids framed, junction seams watertight, incremental regeneration equals full regeneration, layout determinism across Windows and Linux (quantized), vertex positions within tolerance.
- **Generator snapshots:** layout hashes per fixture element, plus rendered plan SVGs for human review of diffs.
- **Performance:** benchmark fixtures (100 m battered wall on a slope, a large concentric castle) tracked per commit; layout ms per element, meshing ms per chunk.
- **Gameplay:** headless Godot tests for command flows (picking through `keep-core`).
- **Visual:** golden screenshots on the dev PC with tolerance.
- **MCP contract:** schema tests per tool, security tests (Origin/Host rejection, permission enforcement), recorded-session replays.
- **Agent evals:** design briefs run against `keep-mcp --offline`, graded by validator and historian scores and plan renders; a smaller live set graded on real renders by a vision model or human.

---

## 13. Milestones

v1's timeline was too optimistic for one person; agents speed up Rust, schemas, tests and MCP, not look-dev, art or audio. Estimates below are roughly double v1's and still need checking after M0.

### M0a: look and performance (3 weeks)
Thin `keep-core` → `keep-gd` → merged chunk meshes. One battered wall and one round tower on a fixed terrain at golden hour, baked vertex AO, the first stone material.
- **Kill criteria:** a screenshot holds up next to the reference photos at a glance, and 200,000 blocks render at 60 fps with LODs within the draw-call budget.
- If the look fails: decide between painterly re-direction and moving the renderer before anything else is built.

### M0b: junctions and feel (3–4 weeks)
- 3D massing and voids: wall to round tower, sloped wall to wall at an angle, a splayed arrow loop through a 2.5 m wall.
- Live drag preview with snapping; construction rising via the shader with dust, a scaffold pass and sound.
- `keep-mcp --offline` with `apply_operations`, `get_castle`, `render_plan`.
- **Kill criteria:** junctions read as built, not modelled; dragging a wall is fun; watching 30 seconds of it rise is pleasant.

### M1: vertical slice (14–18 weeks)
Three curated sites (no terrain sculpting yet), ditches; curtain walls, towers (round, square, D), a simple gatehouse; one style pack (Edwardian Welsh stone) with reactive rules; construction theater with speed control and in-game time-lapse replay; session layer with an in-process host, author-tagged command log and per-author undo, save/load; a replay test (command log on a fresh instance gives identical layout hashes); structural checks; basic in-game agent panel (7.7); live MCP with proposals, `render_view`, subscriptions and the security model.

### M2: depth and a second style (12–16 weeks)
Architectural completeness: ranges and pitched roofs, roof clipping, keeps, more features, the historian, the Baltic brick style pack, weathering and aging, full agent panel (request box, activity feed, Ask for changes), agent eval suite.

### M3: commissions (10–12 weeks)
Patrons, budgets, seasons and frost, events, three to five commissions, workforce and materials, mid-build edit salvage polished, plus the items moved from M2: dollhouse cutaway, terrain sculpting, photo mode, codex. Video time-lapse export if time allows.

### M4: Release 1, Early Access (single player + MCP agents)
Performance, accessibility, localisation, Steam integration, modding docs, licence and attribution pass. One player, with any number of MCP agents working through proposals.

### M5: Release 2, co-play (about 10–14 weeks after Release 1)
Shared sessions for 2 players plus agents, extended to 4 only if it costs nothing in experience (section 15.4): network transport (LAN/direct IP, then Steam relays), lobby and join, presence, soft claims, shared proposals, host-controlled construction, divergence checks and resync. Because single player has run through the session layer since M1, this is mainly transport, presence and UI work.

### Later
Multi-century phasing in depth, ivy growth, optional LLM characters, siege mode.

---

## 14. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Design vs construction feel conflict | High | High | Separate instant design mode from construction theater (section 8); speed control and instant raise. |
| 3D thick-wall, junction and roof geometry | High | High | Massing-first pipeline, priority junction graph, M0b spike, roofs deferred to M2. |
| Godot visual ceiling | Medium | High | M0a look spike before anything else; lighting that doesn't rely on dynamic GI; engine-agnostic core. |
| Procedural masonry looks mechanical | High | High | Hand-tuned per-style distributions, hero pieces, weathering, deliberate imperfection. |
| Art and audio bottleneck with no budget | High | High | Procedural-first, one stone family first, about 10 hero pieces per style, CC0 and self-recorded foley, licence manifest (9.1). |
| Co-play needs a retrofit | Medium | High | Session layer, author-tagged command log and per-author undo from M1; replay and determinism tests (section 15). |
| Polishing tiny details eats the schedule | Medium | Medium | Pillar 6; detail work only where the camera lingers; milestones gated on castle-scale kill criteria. |
| Block counts and draw calls | Medium | High | Merged chunk meshes, GPU-side animation, LOD/HLOD; proven in M0a. |
| Terrain split-brain | Medium | Medium | `keep-core` owns terrain data; renderer is display-only. |
| Toolchain and FFI tax (gdext churn, DLL locks, Blender drift) | Medium | Medium | Pinned versions, thin binding, most tests without Godot. |
| Structural rules frustrate | Medium | Medium | Advisory in sandbox, plain-language reasons, one-click fixes. |
| Scope creep (sieges, colony sim, LLM NPCs) | High | High | Pillars; explicit post-1.0 list. |
| MCP drifts into a second API | Medium | Medium | MCP is a thin layer over the control protocol. |
| Agent edits harm a save or prompt injection via shared content | Low | Medium | Proposal-only default, human approval, attribution, per-author undo, delimited untrusted text. |
| Historical claims wrong | Medium | Low | Rules cite sources; historian speaks in tendencies. |

---

## 15. Co-play and multiplayer (Release 2, designed for now)

Building together, with other players and with agents in the same session, comes in an early release after the first (M5). It isn't needed at launch, but the architecture is built for it from M1 so it isn't a rewrite.

### 15.1 Model
- **Host-authoritative session.** One player's game hosts. The host owns the Castle Document and the construction simulation, validates every command, gives it a sequence number and broadcasts it. Clients send commands and apply the ordered stream.
- **Only commands travel.** Layout and meshes are regenerated locally from the document, which is why deterministic, quantized layout matters (section 3). Two hashes are exchanged periodically: a **document hash** per element (a mismatch means a missed or corrupted command: the host resends that element's state) and a **layout hash** (a mismatch with matching documents means generator divergence, a bug: log a report with both inputs; resending state wouldn't help). On join, clients must match `generator_version`, `style_pack_hash` and `site_hash`.
- **Dragging stays local.** The live drag preview (8.1) runs only on the dragging player's machine; others see a throttled (~10 Hz) lightweight ghost footprint for presence. Releasing the mouse sends one command, shown as a pending ghost until the host confirms (or rolled back with a reason if rejected).
- **No CRDTs.** Commands are coarse (create tower, modify wall) and the host serializes them. That is enough for a few people and some agents, and much simpler.
- **Join in progress:** a joining client gets a snapshot of the document plus the command-log tail, then regenerates.

### 15.2 Agents as participants
- An agent connects through the host's `keep-mcp` and joins the session as a participant with a name and colour, like a player. In Release 2 only the host attaches MCP agents (keep-mcp stays bound to 127.0.0.1); guests can approve agent proposals if their role allows. Letting guests bring their own agents, proxied through their authenticated session, is a later option.
- **Presence:** players and agents show a camera marker and a highlight on what they're working on. Agents announce what they're doing ("working on the barbican") through a session status line.
- **Soft claims:** a participant can claim an element or area. Others, agents included, are warned (not blocked) when editing inside it. Agents respect claims by default.
- **Shared proposals:** an agent's proposal is visible as a ghost to everyone; the host, or the player who asked for it, accepts it.
- MCP subscriptions (7.5) are how an agent hears about other people's edits in real time.

### 15.3 Undo, construction and permissions
- Undo is per author. If a later command by someone else depends on the change being undone, the undo is refused with an explanation, or offered as a new command that removes the element.
- Construction speed and pause are host-controlled. Only the simulation clock, speed, pause and phase commits are synced; each client advances the build front locally from the deterministic schedule.
- Host disconnect ends the session (save and re-host); host migration is out of scope.
- Roles: host, builder, viewer; agents get read-only, proposal-only or direct-edit within the host's settings.

### 15.4 Player count: two first
- **Two players plus agents is the design target.** Every co-play feature is tuned for two people building one castle together, and Release 2 ships when that feels great.
- **Four players is a stretch goal**, only if it needs no compromises: same latency, same readability of presence and proposals, no extra locking rules. Test with four once two works; ship four only if the experience is as good.
- The architecture doesn't cap the number; the limit is a host setting, defaulting to 2 until four is proven.

### 15.5 What M1 must already do
- Run single player through the session layer with an in-process host.
- Author-tag every command, use UUIDv7 ids, keep UI and camera state out of the document.
- Keep a replay test: the command log applied to a fresh instance gives identical layout hashes, on Windows and Linux.
- Expose MCP through the same session so a single-player agent is already a "participant".

---

## 16. Decisions and open questions

Decided (v3):
1. **Co-play:** planned for Release 2, designed for now (section 15).
2. **Detail level:** balanced; castle scale first; no time on the smallest details (pillar 6).
3. **Interiors:** dollhouse cutaway (8.5).
4. **Platform:** PC only, Windows first, Linux kept working.
5. **Budget:** none for paid art or sound (9.1).

6. **Co-play size:** 2 players plus agents is the target and must feel great. 4 players is a stretch goal, taken only if it costs nothing in experience (15.4).
7. **In-game agent panel:** yes, from Release 1 (section 7.7).

Nothing remains open that blocks M0a. Revisit timelines after M0b with measured velocity.

---

## Appendix A: what changed after review

Gemini 4 Neon reviewed v1 ([REVIEW-gemini.md](REVIEW-gemini.md)). Adopted:
- **Rendering:** MultiMesh-per-chunk with 20–40 variants broke the draw-call budget, and baked per-chunk LOD textures would cost too much VRAM. Now one merged mesh per chunk, GPU-side build animation through vertex attributes, and a geometric LOD with triplanar shading.
- **Masonry:** v1's 2D (along, up) face layout fails on battered walls, slopes, curved towers and thick openings. Now 3D massing with solids and voids, horizontal world-space course planes, and Rust-generated conforming blocks. Blender is reserved for hero pieces.
- **Junctions:** pairwise ownership replaced by a priority-ordered junction graph that also routes the wall-walk and clips roofs.
- **Chunk seams:** layout is per element; chunks are only for meshing.
- **Feel:** design mode and construction theater separated; reactive rules, terrain hints, zoom-reactive sound, life for scale, mid-build edit salvage.
- **Engine:** no reliance on SDFGI; `keep-core` owns terrain; pinned versions; thin binding; scenes built in code.
- **MCP:** `apply_operations` batches and relative anchors; `submit_proposal` with human approval (v1 let proposal-only agents commit their own proposals); `render_plan` for offline mode (v1 graded offline evals on renders that offline mode couldn't produce); resource templates, subscriptions, tool annotations, output schemas, tasks and progress; HTTP bearer auth with Origin/Host checks; delimited untrusted text.
- **Errors fixed:** byte-for-byte cross-platform determinism with float trig was not achievable (now quantized layout plus tolerance); headless Godot can't do GPU picking (picking moved to `keep-core`); Blender and Godot versions are pinned.
- **Scope:** M0 split into M0a and M0b with kill criteria; terrain sculpting, roofs and ranges, seasons and video export moved out of M1; LLM characters cut from 1.0; estimates roughly doubled.

Adopted with changes or reservations:
- *"LOD1 baking can't run on Rust worker threads."* Rust can build mesh and texture data off-thread without touching Godot's RenderingServer, so the threading claim is overstated. The VRAM cost alone justifies dropping per-chunk baked textures, so the conclusion stands.
- *Harmonic pitch sequences for placement audio* are an experiment to tune by ear, not a pillar; they can easily become a gimmick.
- *Tasks in MCP:* adopted, but flagged to check against the spec version current at implementation time.
- *2–3× timeline:* estimates roughly doubled, to be checked against measured velocity after M0 rather than fixed now.

---

## Appendix B: owner decisions in v3

- **Co-play** added to the roadmap as Release 2 (M5), with the architecture prepared in M1: session layer, host-authoritative ordering, author-tagged command log, per-author undo, replay tests, agents as session participants (section 15).
- **Balanced detail:** new pillar "castle scale first"; hero kit cut from about 15 to about 10 pieces per style, made parametrically; more simple parts generated in Rust; the "mason's eye" stretch goal dropped.
- **Interiors:** dollhouse cutaway with section planes shared with agents' `render_plan` (8.5), in M2.
- **Platform:** PC only; Steam Deck and consoles dropped from consideration.
- **Budget:** zero; paid assets and contracted audio removed; procedural-first art, CC0 sources, self-recorded foley, licence manifest (9.1).

---

## Appendix C: final review changes (v4)

From Gemini's second review ([REVIEW-gemini-2.md](REVIEW-gemini-2.md)), verdict "go for M0a". Adopted:
- **Triangle budget:** ordinary blocks emit exposed faces only, bevels in the shader; geometric bevels for trim and close LOD0 only.
- **Terrain:** committed to a custom heightfield mesh; Terrain3D dropped.
- **Co-play correctness:** separate document and layout hashes (resending state can't fix generator divergence); version and pack hashes checked on join; canonical ordering and integer-millimetre inputs; linear command log with compensating-command undo and overlay proposals instead of an undo tree; drag preview stays local with one command on release; clock-only construction sync instead of streaming build events; host-only MCP agents in Release 2; no host migration.
- **Pillar 6 cuts:** two masonry shells plus a rubble-core cap instead of three solid leaves; mural stairs as portals; shader clip-plane cutaway with 5 to 8 shared props instead of six prop sets; low-poly silhouette workers.
- **M2 trimmed:** cutaway, terrain sculpting, photo mode and codex moved to M3.
- **Assets:** human Blender time budgeted for hero pieces; music licensing corrected (recordings vs compositions); AI-generated assets kept out of shipped builds by default.
- **Fixes:** wrong section references corrected; open questions closed.

Reservation: the review's figure of 9 to 12 M triangles assumes every one of the 200,000 blocks is drawn at full bevelled LOD0, which the LOD scheme never does. The recommendation is cheap and right anyway, so it is adopted.

Owner follow-up (v4.1): co-play targets 2 players plus agents, with 4 only if no compromise (15.4); the in-game agent panel is a full interface, not a minimal bar (7.7).
