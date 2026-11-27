//! Unified command and validation engine.
//!
//! The same semantic operations are accepted from the Godot UI, the MCP sidecar
//! and the in-process session host. Keeping the strict bounds, Unicode
//! sanitisation and error messages here means every transport enforces exactly
//! the same rules (architecture sections 4.2, 7.6 and 15).
//!
//! Commands are log entries: a [`LoggedCommand`] carries its author, a
//! deterministic UUIDv7 id, the semantic payload, the element ids it touched
//! and an exact [`InverseRecord`] so a later compensating undo can restore the
//! document without ever truncating history.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::document::{
    validate_element, BatteredWall, CastleDocument, Ditch, Element, ElementId, Gatehouse, Keep,
    Opening, OpeningKind, Range, RangeKind, RoundTower, TerrainBrushKind, TerrainSculpt, TowerPlan,
};

// ---------------------------------------------------------------------------
// Security and validation bounds. Mirrored from `keep-proto` so the two crates
// can converge on a single implementation without changing the wire format.
// ---------------------------------------------------------------------------

/// Maximum number of operations accepted in one atomic batch.
pub const MAX_BATCH_OPS: usize = 256;

/// Maximum number of characters in a free-text display name.
pub const MAX_NAME_CHARS: usize = 64;

/// Maximum number of characters in a reference id.
pub const MAX_REF_ID_CHARS: usize = 64;

/// Maximum number of characters in an author id.
pub const MAX_AUTHOR_ID_CHARS: usize = 64;

/// Maximum number of elements and openings a document may hold. Batches are
/// capped well below this, but many batches could otherwise grow the document
/// without bound.
pub const MAX_DOCUMENT_ELEMENTS: usize = 512;

/// Plan coordinates are limited to ±500 m.
pub const MIN_COORD_MM: i64 = -500_000;
/// See [`MIN_COORD_MM`].
pub const MAX_COORD_MM: i64 = 500_000;

/// Shortest accepted curtain-wall run, in millimetres.
pub const MIN_WALL_LENGTH_MM: i128 = 500;
/// Longest accepted curtain-wall run, in millimetres.
pub const MAX_WALL_LENGTH_MM: i128 = 500_000;

/// Wall height bounds, in millimetres (1–60 m).
pub const MIN_WALL_HEIGHT_MM: i64 = 1_000;
/// See [`MIN_WALL_HEIGHT_MM`].
pub const MAX_WALL_HEIGHT_MM: i64 = 60_000;

/// Wall thickness bounds, in millimetres (0.3–8 m).
pub const MIN_WALL_THICKNESS_MM: i64 = 300;
/// See [`MIN_WALL_THICKNESS_MM`].
pub const MAX_WALL_THICKNESS_MM: i64 = 8_000;

/// Batter bounds, in millimetres.
pub const MAX_BATTER_MM: i64 = 3_000;

/// Tower radius bounds, in millimetres (0.8–20 m).
pub const MIN_TOWER_RADIUS_MM: i64 = 800;
/// See [`MIN_TOWER_RADIUS_MM`].
pub const MAX_TOWER_RADIUS_MM: i64 = 20_000;

/// Tower height bounds, in millimetres (1–60 m).
pub const MIN_TOWER_HEIGHT_MM: i64 = 1_000;
/// See [`MIN_TOWER_HEIGHT_MM`].
pub const MAX_TOWER_HEIGHT_MM: i64 = 60_000;

/// Arrow-loop position bounds, in permille along the wall.
pub const MIN_LOOP_T_PERMILLE: i64 = 50;
/// See [`MIN_LOOP_T_PERMILLE`].
pub const MAX_LOOP_T_PERMILLE: i64 = 950;

/// Arrow-loop sill-height bounds, in millimetres.
pub const MAX_LOOP_SILL_HEIGHT_MM: i64 = 20_000;

/// Arrow-loop opening height bounds, in millimetres.
pub const MIN_LOOP_HEIGHT_MM: i64 = 300;
/// See [`MIN_LOOP_HEIGHT_MM`].
pub const MAX_LOOP_HEIGHT_MM: i64 = 6_000;

/// Arrow-loop outer slit width bounds, in millimetres.
pub const MIN_OUTER_SLIT_MM: i64 = 60;
/// See [`MIN_OUTER_SLIT_MM`].
pub const MAX_OUTER_SLIT_MM: i64 = 600;

/// Arrow-loop inner splay width bounds, in millimetres.
pub const MIN_INNER_SPLAY_WIDTH_MM: i64 = 300;
/// See [`MIN_INNER_SPLAY_WIDTH_MM`].
pub const MAX_INNER_SPLAY_WIDTH_MM: i64 = 3_000;

/// Gatehouse passage width bounds, in millimetres.
pub const MIN_PASSAGE_WIDTH_MM: i64 = 1_200;
/// See [`MIN_PASSAGE_WIDTH_MM`].
pub const MAX_PASSAGE_WIDTH_MM: i64 = 12_000;
/// Gatehouse passage height bounds, in millimetres.
pub const MIN_PASSAGE_HEIGHT_MM: i64 = 2_000;
/// See [`MIN_PASSAGE_HEIGHT_MM`].
pub const MAX_PASSAGE_HEIGHT_MM: i64 = 12_000;
/// Gatehouse depth bounds, in millimetres.
pub const MIN_GATEHOUSE_DEPTH_MM: i64 = 2_000;
/// See [`MIN_GATEHOUSE_DEPTH_MM`].
pub const MAX_GATEHOUSE_DEPTH_MM: i64 = 30_000;
/// Gatehouse height bounds, in millimetres.
pub const MIN_GATEHOUSE_HEIGHT_MM: i64 = 4_000;
/// See [`MIN_GATEHOUSE_HEIGHT_MM`].
pub const MAX_GATEHOUSE_HEIGHT_MM: i64 = 40_000;
/// Gatehouse flanking-tower radius bounds, in millimetres.
pub const MIN_GATEHOUSE_TOWER_RADIUS_MM: i64 = 1_500;
/// See [`MIN_GATEHOUSE_TOWER_RADIUS_MM`].
pub const MAX_GATEHOUSE_TOWER_RADIUS_MM: i64 = 10_000;
/// Plan-orientation bounds, in degrees.
pub const MIN_ORIENTATION_DEG: i64 = -360;
/// See [`MIN_ORIENTATION_DEG`].
pub const MAX_ORIENTATION_DEG: i64 = 360;

/// Ditch width bounds, in millimetres.
pub const MIN_DITCH_WIDTH_MM: i64 = 500;
/// See [`MIN_DITCH_WIDTH_MM`].
pub const MAX_DITCH_WIDTH_MM: i64 = 30_000;
/// Ditch depth bounds, in millimetres.
pub const MIN_DITCH_DEPTH_MM: i64 = 100;
/// See [`MIN_DITCH_DEPTH_MM`].
pub const MAX_DITCH_DEPTH_MM: i64 = 10_000;
/// Shortest accepted ditch centreline, in millimetres.
pub const MIN_DITCH_LENGTH_MM: i64 = 500;
/// Longest accepted ditch centreline, in millimetres.
pub const MAX_DITCH_LENGTH_MM: i64 = 500_000;

/// Narrowest accepted keep plan dimension, in millimetres.
pub const MIN_KEEP_DIMENSION_MM: i64 = 4_000;
/// Widest accepted keep plan dimension, in millimetres.
pub const MAX_KEEP_DIMENSION_MM: i64 = 40_000;
/// Lowest accepted keep, in millimetres.
pub const MIN_KEEP_HEIGHT_MM: i64 = 4_000;
/// Highest accepted keep, in millimetres.
pub const MAX_KEEP_HEIGHT_MM: i64 = 40_000;
/// Thinnest accepted keep wall, in millimetres.
pub const MIN_KEEP_WALL_THICKNESS_MM: i64 = 600;
/// Thickest accepted keep wall, in millimetres.
pub const MAX_KEEP_WALL_THICKNESS_MM: i64 = 4_000;

/// Shortest accepted domestic range, in millimetres.
pub const MIN_RANGE_LENGTH_MM: i64 = 4_000;
/// Longest accepted domestic range, in millimetres.
pub const MAX_RANGE_LENGTH_MM: i64 = 60_000;
/// Narrowest accepted domestic range, in millimetres.
pub const MIN_RANGE_WIDTH_MM: i64 = 3_000;
/// Widest accepted domestic range, in millimetres.
pub const MAX_RANGE_WIDTH_MM: i64 = 30_000;
/// Lowest accepted range wall, in millimetres.
pub const MIN_RANGE_WALL_HEIGHT_MM: i64 = 2_000;
/// Highest accepted range wall, in millimetres.
pub const MAX_RANGE_WALL_HEIGHT_MM: i64 = 30_000;
/// Shallowest accepted pitched roof, in degrees.
pub const MIN_ROOF_PITCH_DEG: i64 = 5;
/// Steepest accepted pitched roof, in degrees.
pub const MAX_ROOF_PITCH_DEG: i64 = 70;

/// Terrain sculpt radius bounds, in millimetres.
pub const MIN_SCULPT_RADIUS_MM: i64 = 2_000;
/// See [`MIN_SCULPT_RADIUS_MM`].
pub const MAX_SCULPT_RADIUS_MM: i64 = 30_000;
/// Sculpt centres must stay inside the curated site, in millimetres.
pub const MIN_SCULPT_COORD_MM: i64 = -110_000;
/// See [`MIN_SCULPT_COORD_MM`].
pub const MAX_SCULPT_COORD_MM: i64 = 110_000;
/// Raise/lower delta bounds, in millimetres.
pub const MIN_SCULPT_DELTA_MM: i64 = 250;
/// See [`MIN_SCULPT_DELTA_MM`].
pub const MAX_SCULPT_DELTA_MM: i64 = 10_000;
/// Flatten target-elevation bounds, in millimetres.
pub const MIN_FLATTEN_TARGET_MM: i64 = -15_000;
/// See [`MIN_FLATTEN_TARGET_MM`].
pub const MAX_FLATTEN_TARGET_MM: i64 = 25_000;
/// Smooth strength bounds, in permille.
pub const MIN_SMOOTH_STRENGTH_PERMILLE: i64 = 100;
/// See [`MIN_SMOOTH_STRENGTH_PERMILLE`].
pub const MAX_SMOOTH_STRENGTH_PERMILLE: i64 = 1_000;

// ---------------------------------------------------------------------------
// Authors
// ---------------------------------------------------------------------------

/// Who authored a command. Kept separate from the identifier so the UI can
/// colour players and agents differently without parsing strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Player,
    Agent,
    System,
}

/// A stable, sanitised author identity (for example `player:alex`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AuthorId {
    pub kind: AuthorKind,
    pub id: String,
}

impl AuthorId {
    /// Create a sanitised author id. The identifier is trimmed, length-limited
    /// and rejected if it carries control, bidi, formatting or private-use
    /// characters (the same policy as display names).
    pub fn new(kind: AuthorKind, id: impl Into<String>) -> Result<Self, CommandError> {
        let id = sanitize_author_id(&id.into())?;
        Ok(Self { kind, id })
    }

    pub fn player(id: impl Into<String>) -> Result<Self, CommandError> {
        Self::new(AuthorKind::Player, id)
    }

    pub fn agent(id: impl Into<String>) -> Result<Self, CommandError> {
        Self::new(AuthorKind::Agent, id)
    }

    pub fn system() -> Self {
        Self {
            kind: AuthorKind::System,
            id: "system".to_string(),
        }
    }
}

impl fmt::Display for AuthorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.kind {
            AuthorKind::Player => "player",
            AuthorKind::Agent => "agent",
            AuthorKind::System => "system",
        };
        write!(f, "{kind}:{}", self.id)
    }
}

fn sanitize_author_id(id: &str) -> Result<String, CommandError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(CommandError::AuthorIdEmpty);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CommandError::AuthorIdHasControlChars);
    }
    if trimmed.chars().any(is_forbidden_display_char) {
        return Err(CommandError::AuthorIdHasForbiddenChars);
    }
    let chars = trimmed.chars().count();
    if chars > MAX_AUTHOR_ID_CHARS {
        return Err(CommandError::AuthorIdTooLong {
            chars,
            max: MAX_AUTHOR_ID_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// Semantic operations
// ---------------------------------------------------------------------------

/// One semantic edit. Identical in shape to `keep_proto::Operation` so the
/// Godot UI and MCP sidecar can feed the same engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CommandOp {
    /// Create a straight curtain-wall run between two plan points.
    CreateWallRun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        start_mm: [i64; 2],
        end_mm: [i64; 2],
        height_mm: i64,
        thickness_mm: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batter_mm: Option<i64>,
    },
    /// Create a round drum tower.
    CreateTower {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        centre_mm: [i64; 2],
        radius_mm: i64,
        height_mm: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batter_mm: Option<i64>,
    },
    /// Create a simple gatehouse with two flanking towers and a clear passage.
    CreateGatehouse {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        centre_mm: [i64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orientation_deg: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passage_width_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tower_radius_mm: Option<i64>,
    },
    /// Carve a linear ditch earthwork across the terrain.
    CreateDitch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        start_mm: [i64; 2],
        end_mm: [i64; 2],
        width_mm: i64,
        depth_mm: i64,
    },
    /// Create a great rectangular keep with a crenellated parapet.
    CreateKeep {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        centre_mm: [i64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wall_thickness_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orientation_deg: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batter_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plinth_height_mm: Option<i64>,
    },
    /// Create a roofed domestic range (hall, chapel or kitchen).
    CreateRange {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        kind: RangeKind,
        centre_mm: [i64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        length_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wall_height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        roof_pitch_deg: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orientation_deg: Option<i32>,
    },
    /// Sculpt the site terrain with an authoritative brush.
    SculptTerrain {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        kind: TerrainBrushKind,
        centre_mm: [i64; 2],
        radius_mm: i64,
        delta_mm: i64,
    },
    /// Modify one existing element. Only the fields that are `Some` change.
    ModifyElement {
        id: ElementId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thickness_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batter_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plinth_height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<TowerPlan>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orientation_deg: Option<i32>,
    },
    /// Cut a splayed arrow loop through an existing wall.
    AddArrowLoop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wall_id: Option<ElementId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wall_ref: Option<String>,
        t_permille: i64,
        sill_height_mm: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outer_slit_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inner_splay_width_mm: Option<i64>,
    },
    /// Delete an element (and any openings attached to it) or a lone opening.
    DeleteElement { id: ElementId },
}

impl CommandOp {
    /// Validate and normalise the operation. Free-text fields are sanitised and
    /// numeric inputs are checked against the hard bounds. The returned
    /// operation has a trimmed, control-free name and a safe reference id.
    pub fn validated(&self) -> Result<Self, CommandError> {
        let mut operation = self.clone();
        match &mut operation {
            Self::CreateWallRun {
                ref_id,
                name,
                start_mm,
                end_mm,
                height_mm,
                thickness_mm,
                batter_mm,
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in start_mm.iter() {
                    check_coordinate("start_mm", *value)?;
                }
                for value in end_mm.iter() {
                    check_coordinate("end_mm", *value)?;
                }
                check_dimension(
                    "height_mm",
                    *height_mm,
                    MIN_WALL_HEIGHT_MM,
                    MAX_WALL_HEIGHT_MM,
                )?;
                check_dimension(
                    "thickness_mm",
                    *thickness_mm,
                    MIN_WALL_THICKNESS_MM,
                    MAX_WALL_THICKNESS_MM,
                )?;
                if let Some(batter_mm) = batter_mm {
                    check_dimension("batter_mm", *batter_mm, 0, MAX_BATTER_MM)?;
                }
                check_wall_length(*start_mm, *end_mm)?;
            }
            Self::CreateTower {
                ref_id,
                name,
                centre_mm,
                radius_mm,
                height_mm,
                batter_mm,
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in centre_mm.iter() {
                    check_coordinate("centre_mm", *value)?;
                }
                check_dimension(
                    "radius_mm",
                    *radius_mm,
                    MIN_TOWER_RADIUS_MM,
                    MAX_TOWER_RADIUS_MM,
                )?;
                check_dimension(
                    "height_mm",
                    *height_mm,
                    MIN_TOWER_HEIGHT_MM,
                    MAX_TOWER_HEIGHT_MM,
                )?;
                if let Some(batter_mm) = batter_mm {
                    check_dimension("batter_mm", *batter_mm, 0, MAX_BATTER_MM)?;
                }
            }
            Self::CreateGatehouse {
                ref_id,
                name,
                centre_mm,
                orientation_deg,
                passage_width_mm,
                height_mm,
                tower_radius_mm,
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in centre_mm.iter() {
                    check_coordinate("centre_mm", *value)?;
                }
                let orientation = orientation_deg.unwrap_or(0);
                check_dimension(
                    "orientation_deg",
                    i64::from(orientation),
                    MIN_ORIENTATION_DEG,
                    MAX_ORIENTATION_DEG,
                )?;
                check_dimension(
                    "passage_width_mm",
                    passage_width_mm.unwrap_or(3_200),
                    MIN_PASSAGE_WIDTH_MM,
                    MAX_PASSAGE_WIDTH_MM,
                )?;
                check_dimension(
                    "height_mm",
                    height_mm.unwrap_or(14_000),
                    MIN_GATEHOUSE_HEIGHT_MM,
                    MAX_GATEHOUSE_HEIGHT_MM,
                )?;
                check_dimension(
                    "tower_radius_mm",
                    tower_radius_mm.unwrap_or(3_400),
                    MIN_GATEHOUSE_TOWER_RADIUS_MM,
                    MAX_GATEHOUSE_TOWER_RADIUS_MM,
                )?;
            }
            Self::CreateDitch {
                ref_id,
                name,
                start_mm,
                end_mm,
                width_mm,
                depth_mm,
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in start_mm.iter() {
                    check_coordinate("start_mm", *value)?;
                }
                for value in end_mm.iter() {
                    check_coordinate("end_mm", *value)?;
                }
                check_dimension(
                    "width_mm",
                    *width_mm,
                    MIN_DITCH_WIDTH_MM,
                    MAX_DITCH_WIDTH_MM,
                )?;
                check_dimension(
                    "depth_mm",
                    *depth_mm,
                    MIN_DITCH_DEPTH_MM,
                    MAX_DITCH_DEPTH_MM,
                )?;
                let dx = i128::from(end_mm[0]) - i128::from(start_mm[0]);
                let dz = i128::from(end_mm[1]) - i128::from(start_mm[1]);
                let length_sq = dx * dx + dz * dz;
                let min_sq = i128::from(MIN_DITCH_LENGTH_MM) * i128::from(MIN_DITCH_LENGTH_MM);
                let max_sq = i128::from(MAX_DITCH_LENGTH_MM) * i128::from(MAX_DITCH_LENGTH_MM);
                if length_sq < min_sq || length_sq > max_sq {
                    return Err(CommandError::WallLengthOutOfRange {
                        min_mm: i128::from(MIN_DITCH_LENGTH_MM),
                        max_mm: i128::from(MAX_DITCH_LENGTH_MM),
                    });
                }
            }
            Self::CreateKeep {
                ref_id,
                name,
                centre_mm,
                width_mm,
                depth_mm,
                height_mm,
                wall_thickness_mm,
                orientation_deg,
                batter_mm,
                plinth_height_mm,
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in centre_mm.iter() {
                    check_coordinate("centre_mm", *value)?;
                }
                let width = width_mm.unwrap_or(14_000);
                check_dimension(
                    "width_mm",
                    width,
                    MIN_KEEP_DIMENSION_MM,
                    MAX_KEEP_DIMENSION_MM,
                )?;
                let depth = depth_mm.unwrap_or(12_000);
                check_dimension(
                    "depth_mm",
                    depth,
                    MIN_KEEP_DIMENSION_MM,
                    MAX_KEEP_DIMENSION_MM,
                )?;
                let height = height_mm.unwrap_or(20_000);
                check_dimension("height_mm", height, MIN_KEEP_HEIGHT_MM, MAX_KEEP_HEIGHT_MM)?;
                let max_thickness = width.min(depth) / 2 - 500;
                let thickness = wall_thickness_mm.unwrap_or(2_600);
                check_dimension(
                    "wall_thickness_mm",
                    thickness,
                    MIN_KEEP_WALL_THICKNESS_MM,
                    MAX_KEEP_WALL_THICKNESS_MM.min(max_thickness),
                )?;
                let orientation = orientation_deg.unwrap_or(0);
                check_dimension(
                    "orientation_deg",
                    i64::from(orientation),
                    MIN_ORIENTATION_DEG,
                    MAX_ORIENTATION_DEG,
                )?;
                if let Some(batter_mm) = batter_mm {
                    check_dimension("batter_mm", *batter_mm, 0, MAX_BATTER_MM)?;
                }
                let plinth = plinth_height_mm.unwrap_or(700);
                check_dimension("plinth_height_mm", plinth, 0, height.saturating_sub(1))?;
            }
            Self::CreateRange {
                ref_id,
                name,
                centre_mm,
                length_mm,
                width_mm,
                wall_height_mm,
                roof_pitch_deg,
                orientation_deg,
                ..
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in centre_mm.iter() {
                    check_coordinate("centre_mm", *value)?;
                }
                check_dimension(
                    "length_mm",
                    length_mm.unwrap_or(18_000),
                    MIN_RANGE_LENGTH_MM,
                    MAX_RANGE_LENGTH_MM,
                )?;
                check_dimension(
                    "width_mm",
                    width_mm.unwrap_or(8_500),
                    MIN_RANGE_WIDTH_MM,
                    MAX_RANGE_WIDTH_MM,
                )?;
                check_dimension(
                    "wall_height_mm",
                    wall_height_mm.unwrap_or(7_500),
                    MIN_RANGE_WALL_HEIGHT_MM,
                    MAX_RANGE_WALL_HEIGHT_MM,
                )?;
                check_dimension(
                    "roof_pitch_deg",
                    i64::from(roof_pitch_deg.unwrap_or(42)),
                    MIN_ROOF_PITCH_DEG,
                    MAX_ROOF_PITCH_DEG,
                )?;
                check_dimension(
                    "orientation_deg",
                    i64::from(orientation_deg.unwrap_or(0)),
                    MIN_ORIENTATION_DEG,
                    MAX_ORIENTATION_DEG,
                )?;
            }
            Self::SculptTerrain {
                ref_id,
                name,
                kind,
                centre_mm,
                radius_mm,
                delta_mm,
            } => {
                *name = sanitize_name(name)?;
                validate_optional_ref_id(ref_id)?;
                for value in centre_mm.iter() {
                    check_dimension(
                        "centre_mm",
                        *value,
                        MIN_SCULPT_COORD_MM,
                        MAX_SCULPT_COORD_MM,
                    )?;
                }
                check_dimension(
                    "radius_mm",
                    *radius_mm,
                    MIN_SCULPT_RADIUS_MM,
                    MAX_SCULPT_RADIUS_MM,
                )?;
                match kind {
                    TerrainBrushKind::Raise | TerrainBrushKind::Lower => {
                        check_dimension(
                            "delta_mm",
                            *delta_mm,
                            MIN_SCULPT_DELTA_MM,
                            MAX_SCULPT_DELTA_MM,
                        )?;
                    }
                    TerrainBrushKind::Flatten => {
                        check_dimension(
                            "delta_mm",
                            *delta_mm,
                            MIN_FLATTEN_TARGET_MM,
                            MAX_FLATTEN_TARGET_MM,
                        )?;
                    }
                    TerrainBrushKind::Smooth => {
                        check_dimension(
                            "delta_mm",
                            *delta_mm,
                            MIN_SMOOTH_STRENGTH_PERMILLE,
                            MAX_SMOOTH_STRENGTH_PERMILLE,
                        )?;
                    }
                }
            }
            Self::ModifyElement {
                height_mm,
                thickness_mm,
                batter_mm,
                plinth_height_mm,
                orientation_deg,
                ..
            } => {
                if let Some(height_mm) = height_mm {
                    check_dimension(
                        "height_mm",
                        *height_mm,
                        MIN_WALL_HEIGHT_MM,
                        MAX_WALL_HEIGHT_MM,
                    )?;
                }
                if let Some(thickness_mm) = thickness_mm {
                    check_dimension(
                        "thickness_mm",
                        *thickness_mm,
                        MIN_WALL_THICKNESS_MM,
                        MAX_WALL_THICKNESS_MM,
                    )?;
                }
                if let Some(batter_mm) = batter_mm {
                    check_dimension("batter_mm", *batter_mm, 0, MAX_BATTER_MM)?;
                }
                if let Some(plinth_height_mm) = plinth_height_mm {
                    check_dimension("plinth_height_mm", *plinth_height_mm, 0, MAX_WALL_HEIGHT_MM)?;
                }
                if let Some(orientation_deg) = orientation_deg {
                    check_dimension(
                        "orientation_deg",
                        i64::from(*orientation_deg),
                        MIN_ORIENTATION_DEG,
                        MAX_ORIENTATION_DEG,
                    )?;
                }
            }
            Self::AddArrowLoop {
                wall_ref,
                t_permille,
                sill_height_mm,
                height_mm,
                outer_slit_mm,
                inner_splay_width_mm,
                ..
            } => {
                if let Some(wall_ref) = wall_ref {
                    sanitize_ref_id(wall_ref)?;
                }
                check_dimension(
                    "t_permille",
                    *t_permille,
                    MIN_LOOP_T_PERMILLE,
                    MAX_LOOP_T_PERMILLE,
                )?;
                check_dimension(
                    "sill_height_mm",
                    *sill_height_mm,
                    0,
                    MAX_LOOP_SILL_HEIGHT_MM,
                )?;
                let height_mm = height_mm.unwrap_or(1_500);
                check_dimension(
                    "height_mm",
                    height_mm,
                    MIN_LOOP_HEIGHT_MM,
                    MAX_LOOP_HEIGHT_MM,
                )?;
                let outer_slit_mm = outer_slit_mm.unwrap_or(140);
                check_dimension(
                    "outer_slit_mm",
                    outer_slit_mm,
                    MIN_OUTER_SLIT_MM,
                    MAX_OUTER_SLIT_MM,
                )?;
                let inner_splay_width_mm = inner_splay_width_mm.unwrap_or(1_100);
                check_dimension(
                    "inner_splay_width_mm",
                    inner_splay_width_mm,
                    MIN_INNER_SPLAY_WIDTH_MM,
                    MAX_INNER_SPLAY_WIDTH_MM,
                )?;
                sill_height_mm
                    .checked_add(height_mm)
                    .ok_or(CommandError::OpeningHeightOverflow)?;
            }
            Self::DeleteElement { .. } => {}
        }
        Ok(operation)
    }
}

// ---------------------------------------------------------------------------
// Deterministic UUIDv7
// ---------------------------------------------------------------------------

/// Format a syntactically valid RFC 9562 UUIDv7 string from a deterministic
/// `(timestamp_ms, counter)` pair. No randomness or external crate: the high 48
/// bits are the Unix millisecond timestamp, the version nibble is `7`, the
/// variant bits are `10`, and the remaining bits come from `counter` so
/// consecutive commands are strictly increasing.
pub fn uuid_v7(timestamp_ms: u64, counter: u64) -> String {
    let timestamp = timestamp_ms & 0x0000_FFFF_FFFF_FFFF;
    let rand_a = (counter >> 50) & 0x0FFF;
    let rand_b = counter & 0x003F_FFFF_FFFF_FFFF;

    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().take(6).enumerate() {
        *byte = ((timestamp >> (8 * (5 - index))) & 0xFF) as u8;
    }
    bytes[6] = 0x70 | ((rand_a >> 8) as u8 & 0x0F);
    bytes[7] = (rand_a & 0xFF) as u8;
    bytes[8] = 0x80 | ((rand_b >> 56) as u8 & 0x3F);
    for (index, byte) in bytes.iter_mut().enumerate().skip(9) {
        let shift = 8 * (15 - index);
        *byte = ((rand_b >> shift) & 0xFF) as u8;
    }

    let mut hex = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            hex.push('-');
        }
        hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap());
        hex.push(char::from_digit(u32::from(byte & 0x0F), 16).unwrap());
    }
    hex
}

/// Namespace for the deterministic UUIDv7 helper.
pub struct UuidV7;

impl UuidV7 {
    /// See [`uuid_v7`].
    pub fn generate(timestamp_ms: u64, counter: u64) -> String {
        uuid_v7(timestamp_ms, counter)
    }
}

// ---------------------------------------------------------------------------
// Logged commands
// ---------------------------------------------------------------------------

/// The payload of a log entry. Undo is itself a command so history is strictly
/// append-only (architecture section 4.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandPayload {
    Batch { ops: Vec<CommandOp> },
    CompensateUndo { target_seq: u64 },
}

/// Exact snapshots needed to reverse a command. Restoring these lets undo
/// remove created ids and put deleted elements/openings back unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InverseRecord {
    #[serde(default)]
    pub created_elements: Vec<Element>,
    #[serde(default)]
    pub created_openings: Vec<Opening>,
    #[serde(default)]
    pub deleted_elements: Vec<Element>,
    #[serde(default)]
    pub deleted_openings: Vec<Opening>,
}

impl InverseRecord {
    pub fn is_empty(&self) -> bool {
        self.created_elements.is_empty()
            && self.created_openings.is_empty()
            && self.deleted_elements.is_empty()
            && self.deleted_openings.is_empty()
    }

    /// Apply this inverse to a document. Created items are removed and deleted
    /// items are restored, so the result is byte-identical to the document
    /// before the original command.
    pub fn apply(&self, document: &mut CastleDocument) {
        for opening in &self.created_openings {
            document.remove_opening(opening.id);
        }
        for element in &self.created_elements {
            document.remove_element(element.id());
        }
        for element in &self.deleted_elements {
            document.insert(element.clone());
        }
        for opening in &self.deleted_openings {
            document.insert_opening(opening.clone());
        }
    }
}

/// One entry in the host-ordered command log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggedCommand {
    pub seq: u64,
    pub command_id: String,
    pub author: AuthorId,
    pub payload: CommandPayload,
    #[serde(default)]
    pub touched_elements: Vec<ElementId>,
    /// Elements that shared a junction with a touched element immediately
    /// before or after this command. Undo consults this so removing one half of
    /// a junction cannot silently orphan the other (architecture section 15.3).
    #[serde(default)]
    pub junction_elements: Vec<ElementId>,
    pub inverse: InverseRecord,
    pub summary: String,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Validation and application failures shared by every transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    BatchTooLarge {
        count: usize,
        max: usize,
    },
    TooManyElements {
        count: usize,
        max: usize,
    },
    NameEmpty,
    NameTooLong {
        chars: usize,
        max: usize,
    },
    NameHasControlChars,
    NameHasForbiddenChars,
    RefIdEmpty,
    RefIdTooLong {
        chars: usize,
        max: usize,
    },
    RefIdHasInvalidChars,
    AuthorIdEmpty,
    AuthorIdTooLong {
        chars: usize,
        max: usize,
    },
    AuthorIdHasControlChars,
    AuthorIdHasForbiddenChars,
    CoordinateOutOfRange {
        field: &'static str,
        value: i64,
        min: i64,
        max: i64,
    },
    DimensionOutOfRange {
        field: &'static str,
        value: i64,
        min: i64,
        max: i64,
    },
    WallLengthOutOfRange {
        min_mm: i128,
        max_mm: i128,
    },
    WallStartEqualsEnd,
    OpeningHeightOverflow,
    MissingWallReference,
    UnknownWallRef(String),
    UnknownWallId(ElementId),
    NotAWallRun(ElementId),
    UnknownElementId(ElementId),
    /// A field was supplied that does not apply to the target element's kind.
    InvalidField {
        field: &'static str,
        kind: &'static str,
    },
    DuplicateRefId(String),
    UnknownTargetSeq(u64),
    IdOverflow,
    /// A document map key did not match the id stored inside the value.
    ElementIdMismatch {
        key: ElementId,
        actual: ElementId,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BatchTooLarge { count, max } => {
                write!(f, "batch has {count} operations; maximum is {max}")
            }
            Self::TooManyElements { count, max } => write!(
                f,
                "document would hold {count} elements/openings; maximum is {max}"
            ),
            Self::NameEmpty => write!(f, "name must not be empty"),
            Self::NameTooLong { chars, max } => {
                write!(f, "name has {chars} characters; maximum is {max}")
            }
            Self::NameHasControlChars => write!(f, "name must not contain control characters"),
            Self::NameHasForbiddenChars => write!(
                f,
                "name must not contain bidi, formatting, separator or private-use characters"
            ),
            Self::RefIdEmpty => write!(f, "ref_id must not be empty"),
            Self::RefIdTooLong { chars, max } => {
                write!(f, "ref_id has {chars} characters; maximum is {max}")
            }
            Self::RefIdHasInvalidChars => write!(
                f,
                "ref_id may only contain ASCII letters, digits, '_' and '-'"
            ),
            Self::AuthorIdEmpty => write!(f, "author id must not be empty"),
            Self::AuthorIdTooLong { chars, max } => {
                write!(f, "author id has {chars} characters; maximum is {max}")
            }
            Self::AuthorIdHasControlChars => {
                write!(f, "author id must not contain control characters")
            }
            Self::AuthorIdHasForbiddenChars => write!(
                f,
                "author id must not contain bidi, formatting, separator or private-use characters"
            ),
            Self::CoordinateOutOfRange {
                field,
                value,
                min,
                max,
            } => write!(
                f,
                "{field} coordinate {value} mm is outside {min}..={max} mm"
            ),
            Self::DimensionOutOfRange {
                field,
                value,
                min,
                max,
            } => write!(f, "{field} is {value}; expected {min}..={max}"),
            Self::WallLengthOutOfRange { min_mm, max_mm } => {
                write!(f, "wall length must be between {min_mm} mm and {max_mm} mm")
            }
            Self::WallStartEqualsEnd => write!(f, "wall start_mm and end_mm must differ"),
            Self::OpeningHeightOverflow => {
                write!(f, "sill_height_mm plus height_mm overflows")
            }
            Self::MissingWallReference => write!(f, "add_arrow_loop needs wall_id or wall_ref"),
            Self::UnknownWallRef(reference) => write!(
                f,
                "unknown wall_ref '{reference}' (declare it earlier in the batch)"
            ),
            Self::UnknownWallId(id) => write!(f, "unknown wall_id {id}"),
            Self::NotAWallRun(id) => write!(f, "element {id} is not a wall run"),
            Self::UnknownElementId(id) => write!(f, "unknown element id {id}"),
            Self::InvalidField { field, kind } => {
                write!(f, "{field} does not apply to a {kind}")
            }
            Self::DuplicateRefId(reference) => {
                write!(f, "duplicate ref_id '{reference}' in batch")
            }
            Self::UnknownTargetSeq(seq) => {
                write!(f, "command log has no batch entry with seq {seq}")
            }
            Self::IdOverflow => write!(f, "element id space exhausted"),
            Self::ElementIdMismatch { key, actual } => {
                write!(
                    f,
                    "document map key {key} does not match element id {actual}"
                )
            }
        }
    }
}

impl std::error::Error for CommandError {}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn is_forbidden_display_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}')
        || matches!(
            c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF
        )
        || matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Trim and validate a free-text display name (architecture section 7.6).
pub fn sanitize_name(name: &str) -> Result<String, CommandError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CommandError::NameEmpty);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CommandError::NameHasControlChars);
    }
    if trimmed.chars().any(is_forbidden_display_char) {
        return Err(CommandError::NameHasForbiddenChars);
    }
    let chars = trimmed.chars().count();
    if chars > MAX_NAME_CHARS {
        return Err(CommandError::NameTooLong {
            chars,
            max: MAX_NAME_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

/// Validate a stable reference id (ASCII identifier alphabet only).
pub fn sanitize_ref_id(ref_id: &str) -> Result<(), CommandError> {
    if ref_id.is_empty() {
        return Err(CommandError::RefIdEmpty);
    }
    let chars = ref_id.chars().count();
    if chars > MAX_REF_ID_CHARS {
        return Err(CommandError::RefIdTooLong {
            chars,
            max: MAX_REF_ID_CHARS,
        });
    }
    let valid = ref_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err(CommandError::RefIdHasInvalidChars);
    }
    Ok(())
}

fn validate_optional_ref_id(ref_id: &Option<String>) -> Result<(), CommandError> {
    match ref_id {
        Some(ref_id) => sanitize_ref_id(ref_id),
        None => Ok(()),
    }
}

pub(crate) fn check_coordinate(field: &'static str, value: i64) -> Result<(), CommandError> {
    if !(MIN_COORD_MM..=MAX_COORD_MM).contains(&value) {
        return Err(CommandError::CoordinateOutOfRange {
            field,
            value,
            min: MIN_COORD_MM,
            max: MAX_COORD_MM,
        });
    }
    Ok(())
}

pub(crate) fn check_dimension(
    field: &'static str,
    value: i64,
    min: i64,
    max: i64,
) -> Result<(), CommandError> {
    if !(min..=max).contains(&value) {
        return Err(CommandError::DimensionOutOfRange {
            field,
            value,
            min,
            max,
        });
    }
    Ok(())
}

pub(crate) fn check_wall_length(start_mm: [i64; 2], end_mm: [i64; 2]) -> Result<(), CommandError> {
    if start_mm == end_mm {
        return Err(CommandError::WallStartEqualsEnd);
    }
    let dx = i128::from(end_mm[0]) - i128::from(start_mm[0]);
    let dz = i128::from(end_mm[1]) - i128::from(start_mm[1]);
    let length_sq = dx * dx + dz * dz;
    let min_sq = MIN_WALL_LENGTH_MM * MIN_WALL_LENGTH_MM;
    let max_sq = MAX_WALL_LENGTH_MM * MAX_WALL_LENGTH_MM;
    if length_sq < min_sq || length_sq > max_sq {
        return Err(CommandError::WallLengthOutOfRange {
            min_mm: MIN_WALL_LENGTH_MM,
            max_mm: MAX_WALL_LENGTH_MM,
        });
    }
    Ok(())
}

/// Reject oversized batches. Empty batches are allowed (a successful no-op).
pub fn validate_batch(ops: &[CommandOp]) -> Result<(), CommandError> {
    if ops.len() > MAX_BATCH_OPS {
        return Err(CommandError::BatchTooLarge {
            count: ops.len(),
            max: MAX_BATCH_OPS,
        });
    }
    Ok(())
}

/// Reject documents that would exceed [`MAX_DOCUMENT_ELEMENTS`]. Elements and
/// openings are counted together because both consume memory during scene
/// building.
pub fn validate_element_budget(
    element_count: usize,
    opening_count: usize,
) -> Result<(), CommandError> {
    let count = element_count.saturating_add(opening_count);
    if count > MAX_DOCUMENT_ELEMENTS {
        return Err(CommandError::TooManyElements {
            count,
            max: MAX_DOCUMENT_ELEMENTS,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Batch execution
// ---------------------------------------------------------------------------

/// Result of executing a batch against a document without committing it.
pub(crate) struct BatchApplication {
    pub document: CastleDocument,
    pub next_element_id: ElementId,
    pub created_ids: BTreeMap<String, ElementId>,
    pub inverse: InverseRecord,
    pub touched_elements: Vec<ElementId>,
    pub summary: String,
}

/// Validate and apply `ops` against `base`, allocating ids from `next_id`.
/// Nothing is committed when any operation fails, which is what makes batches
/// atomic at the session layer.
pub(crate) fn execute_batch(
    base: &CastleDocument,
    next_id: ElementId,
    ops: &[CommandOp],
) -> Result<BatchApplication, CommandError> {
    validate_batch(ops)?;

    let mut document = base.clone();
    let mut next_element_id = next_id;
    let mut created_ids: BTreeMap<String, ElementId> = BTreeMap::new();
    let mut inverse = InverseRecord::default();
    let mut touched: std::collections::BTreeSet<ElementId> = std::collections::BTreeSet::new();
    let mut created_walls = 0usize;
    let mut created_towers = 0usize;
    let mut created_gatehouses = 0usize;
    let mut created_ditches = 0usize;
    let mut created_keeps = 0usize;
    let mut created_ranges = 0usize;
    let mut created_sculpts = 0usize;
    let mut created_openings = 0usize;
    let mut modified = 0usize;
    let mut deleted = 0usize;

    for op in ops {
        let op = op.validated()?;
        match op {
            CommandOp::CreateWallRun {
                ref_id,
                name,
                start_mm,
                end_mm,
                height_mm,
                thickness_mm,
                batter_mm,
            } => {
                let mut wall = BatteredWall::new(
                    next_element_id,
                    name,
                    start_mm,
                    end_mm,
                    height_mm,
                    thickness_mm,
                );
                if let Some(batter_mm) = batter_mm {
                    wall.batter_mm = batter_mm;
                }
                let id = wall.id;
                inverse
                    .created_elements
                    .push(Element::BatteredWall(wall.clone()));
                document.add_wall(wall);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_walls += 1;
            }
            CommandOp::CreateTower {
                ref_id,
                name,
                centre_mm,
                radius_mm,
                height_mm,
                batter_mm,
            } => {
                let mut tower =
                    RoundTower::new(next_element_id, name, centre_mm, radius_mm, height_mm);
                if let Some(batter_mm) = batter_mm {
                    tower.batter_mm = batter_mm;
                }
                let id = tower.id;
                inverse
                    .created_elements
                    .push(Element::RoundTower(tower.clone()));
                document.add_tower(tower);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_towers += 1;
            }
            CommandOp::CreateGatehouse {
                ref_id,
                name,
                centre_mm,
                orientation_deg,
                passage_width_mm,
                height_mm,
                tower_radius_mm,
            } => {
                let mut gatehouse = Gatehouse::new(
                    next_element_id,
                    name,
                    centre_mm,
                    orientation_deg.unwrap_or(0),
                );
                if let Some(passage_width_mm) = passage_width_mm {
                    gatehouse.passage_width_mm = passage_width_mm;
                }
                if let Some(height_mm) = height_mm {
                    gatehouse.height_mm = height_mm;
                }
                if let Some(tower_radius_mm) = tower_radius_mm {
                    gatehouse.tower_radius_mm = tower_radius_mm;
                }
                let id = gatehouse.id;
                inverse
                    .created_elements
                    .push(Element::Gatehouse(gatehouse.clone()));
                document.add_gatehouse(gatehouse);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_gatehouses += 1;
            }
            CommandOp::CreateDitch {
                ref_id,
                name,
                start_mm,
                end_mm,
                width_mm,
                depth_mm,
            } => {
                let ditch = Ditch::new(next_element_id, name, start_mm, end_mm, width_mm, depth_mm);
                let id = ditch.id;
                inverse.created_elements.push(Element::Ditch(ditch.clone()));
                document.add_ditch(ditch);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_ditches += 1;
            }
            CommandOp::CreateKeep {
                ref_id,
                name,
                centre_mm,
                width_mm,
                depth_mm,
                height_mm,
                wall_thickness_mm,
                orientation_deg,
                batter_mm,
                plinth_height_mm,
            } => {
                let mut keep = Keep::new(
                    next_element_id,
                    name,
                    centre_mm,
                    orientation_deg.unwrap_or(0),
                );
                if let Some(width_mm) = width_mm {
                    keep.width_mm = width_mm;
                }
                if let Some(depth_mm) = depth_mm {
                    keep.depth_mm = depth_mm;
                }
                if let Some(height_mm) = height_mm {
                    keep.height_mm = height_mm;
                }
                if let Some(wall_thickness_mm) = wall_thickness_mm {
                    keep.wall_thickness_mm = wall_thickness_mm;
                }
                if let Some(batter_mm) = batter_mm {
                    keep.batter_mm = batter_mm;
                }
                if let Some(plinth_height_mm) = plinth_height_mm {
                    keep.plinth_height_mm = plinth_height_mm;
                }
                let id = keep.id;
                inverse.created_elements.push(Element::Keep(keep.clone()));
                document.add_keep(keep);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_keeps += 1;
            }
            CommandOp::CreateRange {
                ref_id,
                name,
                kind,
                centre_mm,
                length_mm,
                width_mm,
                wall_height_mm,
                roof_pitch_deg,
                orientation_deg,
            } => {
                let mut range = Range::new(
                    next_element_id,
                    name,
                    kind,
                    centre_mm,
                    orientation_deg.unwrap_or(0),
                );
                if let Some(length_mm) = length_mm {
                    range.length_mm = length_mm;
                }
                if let Some(width_mm) = width_mm {
                    range.width_mm = width_mm;
                }
                if let Some(wall_height_mm) = wall_height_mm {
                    range.wall_height_mm = wall_height_mm;
                }
                if let Some(roof_pitch_deg) = roof_pitch_deg {
                    range.roof_pitch_deg = roof_pitch_deg;
                }
                let id = range.id;
                inverse.created_elements.push(Element::Range(range.clone()));
                document.add_range(range);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_ranges += 1;
            }
            CommandOp::SculptTerrain {
                ref_id,
                name,
                kind,
                centre_mm,
                radius_mm,
                delta_mm,
            } => {
                let sculpt =
                    TerrainSculpt::new(next_element_id, name, kind, centre_mm, radius_mm, delta_mm);
                let id = sculpt.id;
                inverse
                    .created_elements
                    .push(Element::TerrainSculpt(sculpt.clone()));
                document.add_terrain_sculpt(sculpt);
                register_ref(&mut created_ids, ref_id, id)?;
                touched.insert(id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_sculpts += 1;
            }
            CommandOp::ModifyElement {
                id,
                height_mm,
                thickness_mm,
                batter_mm,
                plinth_height_mm,
                plan,
                orientation_deg,
            } => {
                let previous = document
                    .get(id)
                    .cloned()
                    .ok_or(CommandError::UnknownElementId(id))?;
                let invalid = |field: &'static str| CommandError::InvalidField {
                    field,
                    kind: match &previous {
                        Element::BatteredWall(_) => "wall",
                        Element::RoundTower(_) => "tower",
                        Element::Gatehouse(_) => "gatehouse",
                        Element::Ditch(_) => "ditch",
                        Element::Keep(_) => "keep",
                        Element::Range(_) => "range",
                        Element::TerrainSculpt(_) => "terrain_sculpt",
                    },
                };
                match &previous {
                    Element::BatteredWall(_) => {
                        if plan.is_some() {
                            return Err(invalid("plan"));
                        }
                        if orientation_deg.is_some() {
                            return Err(invalid("orientation_deg"));
                        }
                    }
                    Element::RoundTower(_) => {}
                    Element::Gatehouse(_) => {
                        if thickness_mm.is_some() {
                            return Err(invalid("thickness_mm"));
                        }
                        if batter_mm.is_some() {
                            return Err(invalid("batter_mm"));
                        }
                        if plinth_height_mm.is_some() {
                            return Err(invalid("plinth_height_mm"));
                        }
                        if plan.is_some() {
                            return Err(invalid("plan"));
                        }
                    }
                    Element::Ditch(_) => {
                        if height_mm.is_some() {
                            return Err(invalid("height_mm"));
                        }
                        if thickness_mm.is_some() {
                            return Err(invalid("thickness_mm"));
                        }
                        if batter_mm.is_some() {
                            return Err(invalid("batter_mm"));
                        }
                        if plinth_height_mm.is_some() {
                            return Err(invalid("plinth_height_mm"));
                        }
                        if plan.is_some() {
                            return Err(invalid("plan"));
                        }
                        if orientation_deg.is_some() {
                            return Err(invalid("orientation_deg"));
                        }
                    }
                    Element::Keep(_) => {
                        if plan.is_some() {
                            return Err(invalid("plan"));
                        }
                    }
                    Element::Range(_) => {
                        if thickness_mm.is_some() {
                            return Err(invalid("thickness_mm"));
                        }
                        if batter_mm.is_some() {
                            return Err(invalid("batter_mm"));
                        }
                        if plinth_height_mm.is_some() {
                            return Err(invalid("plinth_height_mm"));
                        }
                        if plan.is_some() {
                            return Err(invalid("plan"));
                        }
                    }
                    Element::TerrainSculpt(_) => {
                        if height_mm.is_some() {
                            return Err(invalid("height_mm"));
                        }
                        if thickness_mm.is_some() {
                            return Err(invalid("thickness_mm"));
                        }
                        if batter_mm.is_some() {
                            return Err(invalid("batter_mm"));
                        }
                        if plinth_height_mm.is_some() {
                            return Err(invalid("plinth_height_mm"));
                        }
                        if plan.is_some() {
                            return Err(invalid("plan"));
                        }
                        if orientation_deg.is_some() {
                            return Err(invalid("orientation_deg"));
                        }
                    }
                }
                let mut updated = previous.clone();
                match &mut updated {
                    Element::BatteredWall(wall) => {
                        if let Some(height_mm) = height_mm {
                            wall.height_mm = height_mm;
                        }
                        if let Some(thickness_mm) = thickness_mm {
                            wall.thickness_mm = thickness_mm;
                        }
                        if let Some(batter_mm) = batter_mm {
                            wall.batter_mm = batter_mm;
                        }
                        if let Some(plinth_height_mm) = plinth_height_mm {
                            wall.plinth_height_mm = plinth_height_mm;
                        }
                    }
                    Element::RoundTower(tower) => {
                        if let Some(height_mm) = height_mm {
                            tower.height_mm = height_mm;
                        }
                        if let Some(thickness_mm) = thickness_mm {
                            tower.thickness_mm = thickness_mm;
                        }
                        if let Some(batter_mm) = batter_mm {
                            tower.batter_mm = batter_mm;
                        }
                        if let Some(plinth_height_mm) = plinth_height_mm {
                            tower.plinth_height_mm = plinth_height_mm;
                        }
                        if let Some(plan) = plan {
                            tower.plan = plan;
                        }
                        if let Some(orientation_deg) = orientation_deg {
                            tower.orientation_deg = orientation_deg;
                        }
                    }
                    Element::Gatehouse(gatehouse) => {
                        if let Some(height_mm) = height_mm {
                            gatehouse.height_mm = height_mm;
                        }
                        if let Some(orientation_deg) = orientation_deg {
                            gatehouse.orientation_deg = orientation_deg;
                        }
                    }
                    Element::Ditch(_) => {}
                    Element::Keep(keep) => {
                        if let Some(height_mm) = height_mm {
                            keep.height_mm = height_mm;
                        }
                        if let Some(thickness_mm) = thickness_mm {
                            keep.wall_thickness_mm = thickness_mm;
                        }
                        if let Some(batter_mm) = batter_mm {
                            keep.batter_mm = batter_mm;
                        }
                        if let Some(plinth_height_mm) = plinth_height_mm {
                            keep.plinth_height_mm = plinth_height_mm;
                        }
                        if let Some(orientation_deg) = orientation_deg {
                            keep.orientation_deg = orientation_deg;
                        }
                    }
                    Element::Range(range) => {
                        if let Some(height_mm) = height_mm {
                            range.wall_height_mm = height_mm;
                        }
                        if let Some(orientation_deg) = orientation_deg {
                            range.orientation_deg = orientation_deg;
                        }
                    }
                    Element::TerrainSculpt(_) => {}
                }
                // Re-validate the resulting element against the same bounds a
                // create command would enforce, including cross-field invariants
                // such as tower thickness versus radius and plinth versus height,
                // so a modify cannot smuggle in an invalid state.
                validate_element(&updated)?;
                document.insert(updated);
                if let Some(index) = inverse
                    .created_elements
                    .iter()
                    .position(|created| created.id() == id)
                {
                    inverse.created_elements[index] = previous;
                } else if !inverse
                    .deleted_elements
                    .iter()
                    .any(|deleted| deleted.id() == id)
                {
                    inverse.deleted_elements.push(previous);
                }
                touched.insert(id);
                modified += 1;
            }
            CommandOp::AddArrowLoop {
                wall_id,
                wall_ref,
                t_permille,
                sill_height_mm,
                height_mm,
                outer_slit_mm,
                inner_splay_width_mm,
            } => {
                let wall_id = match (wall_id, wall_ref.as_deref()) {
                    (Some(id), _) => id,
                    (None, Some(reference)) => *created_ids
                        .get(reference)
                        .ok_or_else(|| CommandError::UnknownWallRef(reference.to_string()))?,
                    (None, None) => return Err(CommandError::MissingWallReference),
                };
                let wall = match document.get(wall_id) {
                    Some(Element::BatteredWall(wall)) => wall.clone(),
                    Some(_) => return Err(CommandError::NotAWallRun(wall_id)),
                    None => return Err(CommandError::UnknownWallId(wall_id)),
                };
                let opening = Opening::new(
                    next_element_id,
                    wall_id,
                    format!("Arrow loop {next_element_id}"),
                    OpeningKind::ArrowLoop,
                    t_permille,
                    sill_height_mm,
                    height_mm.unwrap_or(1_500),
                    outer_slit_mm.unwrap_or(140),
                    inner_splay_width_mm.unwrap_or(1_100),
                    wall.thickness_mm,
                );
                inverse.created_openings.push(opening.clone());
                document.add_opening(opening);
                // The parent wall is the element the opening modifies.
                touched.insert(wall_id);
                next_element_id = next_element_id
                    .checked_add(1)
                    .ok_or(CommandError::IdOverflow)?;
                created_openings += 1;
            }
            CommandOp::DeleteElement { id } => {
                delete_id(
                    &mut document,
                    &mut inverse,
                    &mut created_ids,
                    &mut touched,
                    id,
                )?;
                deleted += 1;
            }
        }
    }

    validate_element_budget(document.len(), document.opening_count())?;
    let touched_elements: Vec<ElementId> = touched.into_iter().collect();
    let summary = batch_summary(
        created_walls,
        created_towers,
        created_gatehouses,
        created_ditches,
        created_keeps,
        created_ranges,
        created_sculpts,
        created_openings,
        modified,
        deleted,
    );

    Ok(BatchApplication {
        document,
        next_element_id,
        created_ids,
        inverse,
        touched_elements,
        summary,
    })
}

fn register_ref(
    created_ids: &mut BTreeMap<String, ElementId>,
    ref_id: Option<String>,
    id: ElementId,
) -> Result<(), CommandError> {
    if let Some(reference) = ref_id {
        sanitize_ref_id(&reference)?;
        if created_ids.insert(reference.clone(), id).is_some() {
            return Err(CommandError::DuplicateRefId(reference));
        }
    }
    Ok(())
}

/// Delete an element or a lone opening, recording exactly what was removed so
/// the inverse can restore it. Items created earlier in the same batch are
/// removed from the inverse instead, because the batch's net effect is a no-op
/// for them.
fn delete_id(
    document: &mut CastleDocument,
    inverse: &mut InverseRecord,
    created_ids: &mut BTreeMap<String, ElementId>,
    touched: &mut std::collections::BTreeSet<ElementId>,
    id: ElementId,
) -> Result<(), CommandError> {
    let has_element = document.get(id).is_some();
    let has_opening = document.opening(id).is_some();
    if !has_element && !has_opening {
        return Err(CommandError::UnknownElementId(id));
    }

    let mut removed_openings = Vec::new();
    if has_element {
        if let Some((element, openings)) = document.remove_element(id) {
            if let Some(index) = inverse
                .created_elements
                .iter()
                .position(|created| created.id() == id)
            {
                inverse.created_elements.remove(index);
            } else if !inverse
                .deleted_elements
                .iter()
                .any(|deleted| deleted.id() == id)
            {
                // A modify earlier in the batch already recorded the original
                // element; keep that snapshot rather than the modified state.
                inverse.deleted_elements.push(element);
            }
            removed_openings.extend(openings);
        }
    }
    if let Some(opening) = document.remove_opening(id) {
        removed_openings.push(opening);
    }
    for opening in &removed_openings {
        touched.insert(opening.wall_id);
        if let Some(index) = inverse
            .created_openings
            .iter()
            .position(|created| created.id == opening.id)
        {
            inverse.created_openings.remove(index);
        } else {
            inverse.deleted_openings.push(opening.clone());
        }
    }
    touched.insert(id);
    created_ids.retain(|_, value| *value != id);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn batch_summary(
    walls: usize,
    towers: usize,
    gatehouses: usize,
    ditches: usize,
    keeps: usize,
    ranges: usize,
    sculpts: usize,
    openings: usize,
    modified: usize,
    deleted: usize,
) -> String {
    let mut parts = Vec::new();
    if walls > 0 {
        parts.push(format!("+{walls} wall{}", plural(walls)));
    }
    if towers > 0 {
        parts.push(format!("+{towers} tower{}", plural(towers)));
    }
    if gatehouses > 0 {
        parts.push(format!("+{gatehouses} gatehouse{}", plural(gatehouses)));
    }
    if ditches > 0 {
        parts.push(format!("+{ditches} ditch{}", plural(ditches)));
    }
    if keeps > 0 {
        parts.push(format!("+{keeps} keep{}", plural(keeps)));
    }
    if ranges > 0 {
        parts.push(format!("+{ranges} range{}", plural(ranges)));
    }
    if sculpts > 0 {
        parts.push(format!("+{sculpts} terrain sculpt{}", plural(sculpts)));
    }
    if openings > 0 {
        parts.push(format!("+{openings} arrow loop{}", plural(openings)));
    }
    if modified > 0 {
        parts.push(format!("~{modified} element{}", plural(modified)));
    }
    if deleted > 0 {
        parts.push(format!("-{deleted} element{}", plural(deleted)));
    }
    if parts.is_empty() {
        "no changes".to_string()
    } else {
        parts.join(", ")
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v7_has_the_required_format_and_is_monotonic() {
        let mut previous = String::new();
        for counter in 0..1_000 {
            let id = uuid_v7(1_700_000_000_000, counter);
            assert_eq!(id.len(), 36);
            let groups: Vec<&str> = id.split('-').collect();
            assert_eq!(
                groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
                vec![8, 4, 4, 4, 12]
            );
            assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
            assert_eq!(&id[14..15], "7", "version nibble must be 7");
            let variant = id.as_bytes()[19] as char;
            assert!(
                matches!(variant, '8' | '9' | 'a' | 'b'),
                "variant nibble was {variant}"
            );
            assert!(id > previous, "UUIDv7 must be strictly increasing");
            previous = id;
        }
    }

    #[test]
    fn uuid_v7_encodes_the_timestamp() {
        let id = uuid_v7(0x1234_5678_9ABC, 0);
        assert_eq!(&id[..13], "12345678-9abc");
    }

    #[test]
    fn operations_round_trip_through_json() {
        let op = CommandOp::CreateWallRun {
            ref_id: Some("north".into()),
            name: "North curtain".into(),
            start_mm: [-8_000, 0],
            end_mm: [18_000, 0],
            height_mm: 9_000,
            thickness_mm: 2_500,
            batter_mm: Some(800),
        };
        let value = serde_json::to_value(&op).unwrap();
        assert_eq!(value["op"], "create_wall_run");
        let parsed: CommandOp = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, op);
    }

    #[test]
    fn names_and_ref_ids_are_sanitised() {
        assert_eq!(sanitize_name("  North  ").unwrap(), "North");
        assert_eq!(sanitize_name("   "), Err(CommandError::NameEmpty));
        assert_eq!(
            sanitize_name("bad\nname"),
            Err(CommandError::NameHasControlChars)
        );
        assert_eq!(
            sanitize_name("curtain \u{202E}evil"),
            Err(CommandError::NameHasForbiddenChars)
        );
        assert!(sanitize_ref_id("north-1_A").is_ok());
        assert_eq!(sanitize_ref_id(""), Err(CommandError::RefIdEmpty));
        assert_eq!(
            sanitize_ref_id("north wall"),
            Err(CommandError::RefIdHasInvalidChars)
        );
    }

    #[test]
    fn geometry_bounds_are_enforced() {
        let base = CommandOp::CreateWallRun {
            ref_id: None,
            name: "Wall".into(),
            start_mm: [0, 0],
            end_mm: [10_000, 0],
            height_mm: 8_000,
            thickness_mm: 2_000,
            batter_mm: None,
        };
        assert!(base.validated().is_ok());

        let mut huge = base.clone();
        if let CommandOp::CreateWallRun { end_mm, .. } = &mut huge {
            *end_mm = [9_000_000_000, 0];
        }
        assert!(matches!(
            huge.validated(),
            Err(CommandError::CoordinateOutOfRange {
                field: "end_mm",
                ..
            })
        ));

        let mut tall = base;
        if let CommandOp::CreateWallRun { height_mm, .. } = &mut tall {
            *height_mm = 900_000_000;
        }
        assert!(matches!(
            tall.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "height_mm",
                ..
            })
        ));
    }

    #[test]
    fn sculpt_terrain_ops_validate_apply_hash_and_undo_cleanly() {
        use crate::terrain::SiteTerrain;

        let base = CastleDocument::new();
        let op = CommandOp::SculptTerrain {
            ref_id: Some("mound".into()),
            name: "Motte mound".into(),
            kind: TerrainBrushKind::Raise,
            centre_mm: [0, 0],
            radius_mm: 10_000,
            delta_mm: 2_000,
        };
        assert!(op.validated().is_ok());
        let value = serde_json::to_value(&op).unwrap();
        assert_eq!(value["op"], "sculpt_terrain");
        assert_eq!(serde_json::from_value::<CommandOp>(value).unwrap(), op);

        let applied = execute_batch(&base, 1, &[op]).expect("sculpt applies");
        assert_eq!(applied.document.terrain_sculpts().count(), 1);
        assert_ne!(base.document_hash(), applied.document.document_hash());
        assert_eq!(applied.touched_elements, vec![1]);

        let terrain = SiteTerrain::m0a_promontory();
        let sculpts: Vec<&TerrainSculpt> = applied.document.terrain_sculpts().collect();
        let raised = terrain.with_earthworks(&[], &sculpts);
        assert!(raised.height_mm(0, 0) > terrain.height_mm(0, 0));

        // Undo restores the exact document and therefore the exact hash.
        let mut restored = applied.document.clone();
        applied.inverse.apply(&mut restored);
        assert_eq!(restored, base);
        assert_eq!(restored.document_hash(), base.document_hash());

        // Delete + inverse also round-trips a sculpt.
        let deleted = execute_batch(
            &applied.document,
            applied.next_element_id,
            &[CommandOp::DeleteElement { id: 1 }],
        )
        .expect("sculpt deletes");
        assert_eq!(deleted.document.terrain_sculpts().count(), 0);
        let mut undeleted = deleted.document.clone();
        deleted.inverse.apply(&mut undeleted);
        assert_eq!(undeleted, applied.document);
    }

    #[test]
    fn sculpt_terrain_bounds_depend_on_the_brush_kind() {
        let base = CommandOp::SculptTerrain {
            ref_id: None,
            name: "Brush".into(),
            kind: TerrainBrushKind::Raise,
            centre_mm: [0, 0],
            radius_mm: 8_000,
            delta_mm: 1_000,
        };
        assert!(base.validated().is_ok());

        let mut invalid = base.clone();
        if let CommandOp::SculptTerrain {
            radius_mm,
            centre_mm,
            ..
        } = &mut invalid
        {
            *radius_mm = MIN_SCULPT_RADIUS_MM - 1;
            centre_mm[0] = MAX_SCULPT_COORD_MM + 1;
        }
        assert!(matches!(
            invalid.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "centre_mm",
                ..
            })
        ));
        if let CommandOp::SculptTerrain {
            centre_mm,
            radius_mm,
            ..
        } = &mut invalid
        {
            centre_mm[0] = 0;
            *radius_mm = MIN_SCULPT_RADIUS_MM - 1;
        }
        assert!(matches!(
            invalid.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "radius_mm",
                ..
            })
        ));

        let mut raise = base.clone();
        if let CommandOp::SculptTerrain { delta_mm, .. } = &mut raise {
            *delta_mm = MIN_SCULPT_DELTA_MM - 1;
        }
        assert!(matches!(
            raise.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "delta_mm",
                ..
            })
        ));

        let mut flatten = base.clone();
        if let CommandOp::SculptTerrain { kind, delta_mm, .. } = &mut flatten {
            *kind = TerrainBrushKind::Flatten;
            *delta_mm = MIN_FLATTEN_TARGET_MM - 1;
        }
        assert!(matches!(
            flatten.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "delta_mm",
                ..
            })
        ));

        let mut smooth = base;
        if let CommandOp::SculptTerrain { kind, delta_mm, .. } = &mut smooth {
            *kind = TerrainBrushKind::Smooth;
            *delta_mm = MAX_SMOOTH_STRENGTH_PERMILLE + 1;
        }
        assert!(matches!(
            smooth.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "delta_mm",
                ..
            })
        ));
    }

    #[test]
    fn gatehouse_ditch_and_modify_ops_validate_and_round_trip() {
        let gatehouse = CommandOp::CreateGatehouse {
            ref_id: Some("gate".into()),
            name: "Great gate".into(),
            centre_mm: [0, 0],
            orientation_deg: Some(90),
            passage_width_mm: None,
            height_mm: None,
            tower_radius_mm: None,
        };
        let value = serde_json::to_value(&gatehouse).unwrap();
        assert_eq!(value["op"], "create_gatehouse");
        let parsed: CommandOp = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.validated().unwrap(), gatehouse);

        let mut tiny_gate = gatehouse.clone();
        if let CommandOp::CreateGatehouse {
            tower_radius_mm, ..
        } = &mut tiny_gate
        {
            *tower_radius_mm = Some(10);
        }
        assert!(matches!(
            tiny_gate.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "tower_radius_mm",
                ..
            })
        ));

        let ditch = CommandOp::CreateDitch {
            ref_id: None,
            name: "Ditch".into(),
            start_mm: [0, 0],
            end_mm: [20_000, 0],
            width_mm: 4_000,
            depth_mm: 2_000,
        };
        assert!(ditch.validated().is_ok());

        let modify = CommandOp::ModifyElement {
            id: 1,
            height_mm: Some(10_000),
            thickness_mm: Some(2_400),
            batter_mm: Some(900),
            plinth_height_mm: Some(900),
            plan: Some(TowerPlan::DShaped),
            orientation_deg: Some(180),
        };
        assert!(modify.validated().is_ok());
        let mut bad_orientation = modify.clone();
        if let CommandOp::ModifyElement {
            orientation_deg, ..
        } = &mut bad_orientation
        {
            *orientation_deg = Some(9_999);
        }
        assert!(matches!(
            bad_orientation.validated(),
            Err(CommandError::DimensionOutOfRange {
                field: "orientation_deg",
                ..
            })
        ));
    }

    #[test]
    fn modify_element_rejects_foreign_fields_and_invalid_cross_field_state() {
        use crate::document::{BatteredWall, Ditch, Gatehouse, RoundTower, TowerPlan};

        let mut document = CastleDocument::new();
        document.add_wall(BatteredWall::new(
            1,
            "Wall",
            [0, 0],
            [10_000, 0],
            8_000,
            2_400,
        ));
        let mut tower = RoundTower::new(2, "Slim tower", [20_000, 0], 1_000, 12_000);
        tower.thickness_mm = 900;
        document.add_tower(tower);
        document.add_gatehouse(Gatehouse::new(3, "Gate", [40_000, 0], 0));
        document.add_ditch(Ditch::new(
            4,
            "Ditch",
            [60_000, -10_000],
            [60_000, 10_000],
            4_000,
            2_000,
        ));

        let apply = |op: CommandOp| execute_batch(&document, 10, &[op]);

        assert!(matches!(
            apply(CommandOp::ModifyElement {
                id: 1,
                height_mm: None,
                thickness_mm: None,
                batter_mm: None,
                plinth_height_mm: None,
                plan: Some(TowerPlan::Square),
                orientation_deg: None,
            }),
            Err(CommandError::InvalidField {
                field: "plan",
                kind: "wall"
            })
        ));

        assert!(matches!(
            apply(CommandOp::ModifyElement {
                id: 4,
                height_mm: Some(9_000),
                thickness_mm: None,
                batter_mm: None,
                plinth_height_mm: None,
                plan: None,
                orientation_deg: None,
            }),
            Err(CommandError::InvalidField {
                field: "height_mm",
                kind: "ditch"
            })
        ));

        assert!(matches!(
            apply(CommandOp::ModifyElement {
                id: 3,
                height_mm: Some(1_000),
                thickness_mm: None,
                batter_mm: None,
                plinth_height_mm: None,
                plan: None,
                orientation_deg: None,
            }),
            Err(CommandError::DimensionOutOfRange {
                field: "height_mm",
                ..
            })
        ));

        assert!(matches!(
            apply(CommandOp::ModifyElement {
                id: 2,
                height_mm: None,
                thickness_mm: Some(8_000),
                batter_mm: None,
                plinth_height_mm: None,
                plan: None,
                orientation_deg: None,
            }),
            Err(CommandError::DimensionOutOfRange {
                field: "thickness_mm",
                ..
            })
        ));

        // A 4 m gatehouse cannot keep a 4.2 m passage plus 1 m of masonry.
        assert!(matches!(
            apply(CommandOp::ModifyElement {
                id: 3,
                height_mm: Some(4_000),
                thickness_mm: None,
                batter_mm: None,
                plinth_height_mm: None,
                plan: None,
                orientation_deg: None,
            }),
            Err(CommandError::DimensionOutOfRange {
                field: "passage_height_mm",
                ..
            })
        ));
    }
}
