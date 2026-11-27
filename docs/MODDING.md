# Modding Keepwright

Keepwright's historical content is data-driven. Style packs, site metadata,
agent eval briefs and the licence manifest are plain JSON or Python-checked
data, so they review and diff cleanly.

Release 1 ships two curated style packs and three curated sites. Style packs
and site metadata are embedded into the binaries at build time; adding a new
one means adding its data file **and** registering it in the small Rust list
that compiles it in. There is no runtime mod-folder loader in Release 1.

All lengths are integer millimetres and plan coordinates are `[x_mm, z_mm]`
unless a field name says metres or degrees.

## Style packs (`styles/*.json`)

A style pack encodes the rules a master mason would recognise: course height,
wall thickness, slenderness, batter, plinth and flanking distances. The
structural validator and historian read these values, and the in-game style
picker exposes every compiled-in pack.

The two shipped packs are:

- [`styles/edwardian_welsh.json`](../styles/edwardian_welsh.json) — Edwardian
  Welsh stone, 1277–1300, limestone.
- [`styles/baltic_brick.json`](../styles/baltic_brick.json) — Baltic Brick,
  1350–1450, red brick.

### Fields

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable lowercase identifier used by tools, evals and commissions. |
| `name` | string | Display name, for example `"Edwardian Welsh (1277–1300)"`. |
| `period` | string | Historical period shown in the UI, for example `"1277–1300"`. |
| `region` | string | Geographic region, for example `"North Wales"`. |
| `course_height_mm` | integer | Nominal ashlar or brick course height. |
| `min_wall_thickness_mm` | integer | Thinnest structurally credible curtain wall. |
| `default_wall_thickness_mm` | integer | Preferred curtain-wall thickness. |
| `max_slenderness_ratio` | number | Maximum height-to-thickness ratio before a wall reads as unstable. |
| `default_batter_mm` | integer | Default total batter inset over the full wall height. |
| `default_plinth_height_mm` | integer | Default plinth height. |
| `flanking_spacing_min_m` | number | Closest comfortable spacing between flanking towers, in metres. |
| `flanking_spacing_max_m` | number | Furthest flanking distance that still gives bowshot coverage, in metres. |
| `material_palette` | string | Dominant material, for example `"limestone"` or `"red_brick"`. Optional; defaults to `"limestone"`. |
| `source_notes` | array of strings | Short citations shown with validation and historian findings. |

### Skeleton

```json
{
  "id": "my_style",
  "name": "My Style (1400–1500)",
  "period": "1400–1500",
  "region": "Somewhere",
  "course_height_mm": 380,
  "min_wall_thickness_mm": 1600,
  "default_wall_thickness_mm": 2200,
  "max_slenderness_ratio": 4.8,
  "default_batter_mm": 600,
  "default_plinth_height_mm": 800,
  "flanking_spacing_min_m": 24.0,
  "flanking_spacing_max_m": 40.0,
  "material_palette": "limestone",
  "source_notes": [
    "One or two sentences naming real buildings and what they demonstrate."
  ]
}
```

Keep the rules internally consistent: `default_wall_thickness_mm` should be at
least `min_wall_thickness_mm`, and the flanking minimum should be below the
maximum. Give every pack at least one source note; the first note is used as
the citation on validation findings.

### Registering a pack

The runtime list is compiled in `core/keep-core/src/style.rs`:

1. Add `styles/my_style.json`.
2. Add an `include_str!("../../../styles/my_style.json")` constant.
3. Load it in `StylePack::list()` and add accessors if you want a named
   constructor such as `edwardian_welsh()`.
4. Add a parse test asserting the values you intend.

`StylePack::from_json` can also parse a pack outside the curated list for
tooling and tests. Curated packs are returned by `list_styles`, `get_style` and
the `keep://styles/<id>` resources.

## Sites (`sites/*.json`)

A site file is metadata for a curated terrain profile. It does not contain a
heightfield: the terrain shape is generated analytically in
`core/keep-core/src/terrain.rs`, while the JSON supplies the name, description,
latitude and landform tags shown to players and agents.

Shipped sites:

- [`sites/m0a_promontory.json`](../sites/m0a_promontory.json) — river
  promontory.
- [`sites/coastal_headland.json`](../sites/coastal_headland.json) — steep
  coastal headland.
- [`sites/lake_island.json`](../sites/lake_island.json) — low limestone island
  plateau.

### Fields

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable identifier passed to `keep-mcp --site <id>` and eval briefs. |
| `name` | string | Display name. |
| `description` | string | One or two sentences describing the terrain and its tactical character. |
| `latitude_deg` | number | Site latitude in degrees, exposed to tools and clients. |
| `landforms` | array of strings | Short tags describing the shelves, cliffs, shores and approaches. |

### Skeleton

```json
{
  "id": "my_site",
  "name": "My Site",
  "description": "A short description of the terrain and its approach.",
  "latitude_deg": 54.0,
  "landforms": ["upper shelf", "north cliff", "landward approach"]
}
```

### Registering a site

Metadata alone does not create a playable terrain. To add a curated site:

1. Add `sites/my_site.json`.
2. Add an `include_str!` constant and an entry in `SiteTerrain::list_sites()`
   in `core/keep-core/src/terrain.rs`.
3. Map the id to a terrain profile in `SiteTerrain::for_site`; add a
   `SiteKind` variant or reuse an existing profile if the shape is the same.
4. Add a test that every curated site has finite latitude and at least one
   landform tag.

Until the site is registered, `keep-mcp --site my_site` will reject the id.
In-game, players can still reshape a curated site with the terrain-sculpt tool;
those earthworks are stored in the Castle Document and saved with the castle.

## Agent eval briefs (`evals/briefs/*.json`)

The offline eval suite replays a recorded tool trace through the real
`keep-mcp --offline` sidecar and grades the resulting castle. It needs no API
key, network access or GPU, only Python 3 and a built `keep-mcp`.

Bundled briefs cover Edwardian and Baltic designs on all three sites. Run them
with:

```bash
python3 evals/run_evals.py
python3 evals/run_evals.py --brief baltic_island_castle
python3 evals/run_evals.py --json
python3 evals/run_evals.py --mcp-bin core/target/release/keep-mcp
```

The runner auto-detects `core/target/debug/keep-mcp` or
`core/target/release/keep-mcp`, and builds the debug sidecar if neither exists.
Exit status is `0` only when every selected brief passes its thresholds.
See [`evals/README.md`](../evals/README.md) for the full format.

### Brief fields

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Stable brief identifier, unique within `evals/briefs/`. |
| `title` | string | Human-readable title. |
| `description` | string | Summary of the design. |
| `style_id` | string | Style used for `get_assessment` and the historian (`edwardian_welsh` or `baltic_brick`). |
| `site_id` | string | Curated site passed to the sidecar (`m0a_promontory`, `coastal_headland` or `lake_island`). |
| `prompt` | string | Natural-language brief an external agent would receive. |
| `thresholds` | object | Pass/fail limits; see below. |
| `reference_steps` | array | Ordered `tools/call` steps replayed in a fresh sidecar. Each has `tool` and `arguments`. |

Threshold fields:

| Field | Meaning |
| --- | --- |
| `min_structure_score` | Minimum `get_assessment.score`, normally `100`. |
| `min_historian_score` | Minimum `get_assessment.historian.overall_score`. |
| `max_structure_errors` | Maximum findings with severity `error`, normally `0`. |
| `min_elements` | Minimum element count from `list_elements`. |
| `require_svg_plan` | When true, `render_plan` must return a valid top-plan SVG. |

### Skeleton

```json
{
  "id": "my_brief",
  "title": "My Brief",
  "description": "A short design summary.",
  "style_id": "edwardian_welsh",
  "site_id": "coastal_headland",
  "prompt": "You are a master mason. Build ...",
  "thresholds": {
    "min_structure_score": 100,
    "min_historian_score": 85,
    "max_structure_errors": 0,
    "min_elements": 12,
    "require_svg_plan": true
  },
  "reference_steps": [
    {
      "tool": "apply_operations",
      "arguments": {
        "operations": [
          {
            "op": "create_wall_run",
            "ref_id": "south",
            "name": "South curtain",
            "start_mm": [-16000, -16000],
            "end_mm": [16000, -16000],
            "height_mm": 8000,
            "thickness_mm": 1800,
            "batter_mm": 600
          }
        ]
      }
    }
  ]
}
```

`reference_steps` are ordinary MCP calls. Later operations can refer to earlier
ones by `ref_id` (or `wall_ref` for arrow loops). Replay always starts from an
empty document on the chosen site, so a brief must not depend on another
brief's state. Copying a successful agent run into `reference_steps` turns it
into a regression fixture.

## Zero-budget and licensing rules

Release 1 ships only procedural, self-made, MIT, CC0-1.0 or CC-BY-4.0 assets.
Paid assets and AI-generated assets are not allowed in shipped builds.

Every shipped file must be listed in
[`assets-src/LICENCE_MANIFEST.json`](../assets-src/LICENCE_MANIFEST.json) with
these fields:

| Field | Meaning |
| --- | --- |
| `path` | Repository-relative path to the shipped file. |
| `kind` | One of `procedural_shader`, `procedural_audio`, `data_pack`, `texture`, `model`, `audio`. |
| `source` | Short description of how the asset was made and where it comes from. |
| `author` | Author name. |
| `licence` | `MIT`, `CC0-1.0`, `CC-BY-4.0` or `self-made`. |
| `url` | Canonical source URL, or the project repository for self-made work. |
| `shipped` | `true` for files included in a build. |
| `ai_generated` | Must be `false` for shipped entries. |

A custom style pack or site is a `data_pack` entry. Validate the manifest and
keep [`docs/CREDITS.md`](CREDITS.md) in sync with:

```bash
python3 tools/check_licences.py --check
python3 tools/check_licences.py --write-credits
```

`--check` fails on a missing path, an unapproved or non-commercial licence on
a shipped asset, an AI-generated shipped asset, or stale generated credits.
`--write-credits` validates first, then regenerates `docs/CREDITS.md`; commit
both the manifest and the regenerated credits.
