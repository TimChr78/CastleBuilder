//! Versioned control-protocol types for Keepwright, shared by `keep-mcp`, the
//! live game control server and tests.
//!
//! The protocol is deliberately small and JSON-shaped: agents send semantic
//! operations (integer millimetres, relative anchors) and receive compact
//! summaries, hashes and rendered plan SVGs. Nothing here depends on the
//! engine or on `keep-core`, so the schema can be reused by clients written in
//! any language.
//!
//! Security limits live here too ([`MAX_BATCH_OPS`], [`MAX_NAME_CHARS`]) so
//! every transport applies the same rules (architecture section 7.6).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum number of operations accepted in one atomic batch.
pub const MAX_BATCH_OPS: usize = 256;

/// Maximum number of characters in a free-text display name.
pub const MAX_NAME_CHARS: usize = 64;

/// Maximum number of characters in a reference id.
pub const MAX_REF_ID_CHARS: usize = 64;

/// Maximum number of elements and openings a document may hold. Batches are
/// capped well below this, but many batches could otherwise grow the document
/// without bound and exhaust sidecar memory during scene building.
pub const MAX_DOCUMENT_ELEMENTS: usize = 512;

/// Plan coordinates are limited to ±500 m. This keeps squared distances inside
/// `i64` and stops huge values from overflowing geometry maths.
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

/// Arrow-loop position bounds, in permille along the wall. Loops are kept away
/// from the very ends so they always stay framed in masonry.
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

/// Default view used by `render_plan` when no view is supplied.
pub const DEFAULT_PLAN_VIEW: &str = "top_plan";

/// A single semantic edit. Operations are applied atomically and in order by
/// [`crate::ApplyOperationsRequest`] consumers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Create a straight curtain-wall run between two plan points.
    CreateWallRun {
        /// Optional stable reference usable by later operations in the batch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        /// Start point `[x, z]`, integer millimetres.
        start_mm: [i64; 2],
        /// End point `[x, z]`, integer millimetres.
        end_mm: [i64; 2],
        height_mm: i64,
        thickness_mm: i64,
        /// Total horizontal inset per face over the full height.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batter_mm: Option<i64>,
    },
    /// Create a round drum tower.
    CreateTower {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_id: Option<String>,
        name: String,
        /// Plan centre `[x, z]`, integer millimetres.
        centre_mm: [i64; 2],
        radius_mm: i64,
        height_mm: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batter_mm: Option<i64>,
    },
    /// Cut a splayed arrow loop through an existing wall. The wall may be given
    /// by numeric id or by a `ref_id` created earlier in the same batch.
    AddArrowLoop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wall_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wall_ref: Option<String>,
        /// Position along the wall run, `0` at the start, `1000` at the end.
        t_permille: i64,
        sill_height_mm: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outer_slit_mm: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inner_splay_width_mm: Option<i64>,
    },
    /// Delete an element (and any openings attached to it) by id.
    DeleteElement { id: u64 },
}

impl Operation {
    /// Validate and normalise the operation: free-text fields are sanitised and
    /// numeric inputs are checked against the protocol's hard bounds. The
    /// returned operation always has a trimmed, control-character-free name and
    /// a safe reference id where the variant has one.
    pub fn validated(&self) -> Result<Self, ProtoError> {
        let mut operation = self.clone();
        match &mut operation {
            Operation::CreateWallRun {
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
            Operation::CreateTower {
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
            Operation::AddArrowLoop {
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
                    .ok_or(ProtoError::OpeningHeightOverflow)?;
            }
            Operation::DeleteElement { .. } => {}
        }
        Ok(operation)
    }
}

/// A whole batch of edits. Batches are atomic: either every operation applies
/// or the document is left untouched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApplyOperationsRequest {
    pub operations: Vec<Operation>,
}

/// Result of a successful [`ApplyOperationsRequest`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApplyOperationsResponse {
    pub applied_count: usize,
    /// Named references declared during the batch mapped to committed ids.
    pub created_ids: BTreeMap<String, u64>,
    pub document_hash: String,
    pub layout_hash: String,
    pub block_count: usize,
    pub chunk_count: usize,
    pub summary: String,
}

/// Compact description of one element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementSummary {
    pub id: u64,
    /// `"wall"` or `"tower"`.
    pub kind: String,
    pub name: String,
    pub height_mm: i64,
    pub thickness_mm: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_mm: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_mm: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centre_mm: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_mm: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_mm: Option<i64>,
}

/// Compact description of one opening.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpeningSummary {
    pub id: u64,
    pub wall_id: u64,
    /// `"arrow_loop"`.
    pub kind: String,
    pub name: String,
    pub t_permille: i64,
    pub sill_height_mm: i64,
    pub height_mm: i64,
    pub outer_slit_mm: i64,
    pub inner_splay_width_mm: i64,
}

/// Full castle summary returned by the `get_castle` tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GetCastleResponse {
    pub document_hash: String,
    pub layout_hash: String,
    pub element_count: usize,
    pub opening_count: usize,
    pub block_count: usize,
    pub chunk_count: usize,
    pub elements: Vec<ElementSummary>,
    pub openings: Vec<OpeningSummary>,
}

/// Result of the `list_elements` tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementListResponse {
    pub element_count: usize,
    pub opening_count: usize,
    pub elements: Vec<ElementSummary>,
    pub openings: Vec<OpeningSummary>,
}

/// Request for a plan or elevation render.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPlanRequest {
    /// `"top_plan"` or `"elevation_north"`. Defaults to `"top_plan"` so an
    /// empty arguments object renders the top plan as documented.
    #[serde(default = "default_plan_view")]
    pub view: String,
}

fn default_plan_view() -> String {
    DEFAULT_PLAN_VIEW.to_string()
}

impl Default for RenderPlanRequest {
    fn default() -> Self {
        Self {
            view: default_plan_view(),
        }
    }
}

/// A rendered SVG plan or elevation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPlanResponse {
    pub view: String,
    pub document_hash: String,
    pub mime_type: String,
    pub svg: String,
}

/// Validation failures raised by the protocol layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
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
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BatchTooLarge { count, max } => {
                write!(f, "batch has {count} operations; maximum is {max}")
            }
            Self::TooManyElements { count, max } => {
                write!(
                    f,
                    "document would hold {count} elements/openings; maximum is {max}"
                )
            }
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
        }
    }
}

impl std::error::Error for ProtoError {}

/// Reject oversized batches. Empty batches are allowed (a successful no-op).
pub fn validate_batch(operations: &[Operation]) -> Result<(), ProtoError> {
    if operations.len() > MAX_BATCH_OPS {
        return Err(ProtoError::BatchTooLarge {
            count: operations.len(),
            max: MAX_BATCH_OPS,
        });
    }
    Ok(())
}

/// Reject documents that would exceed [`MAX_DOCUMENT_ELEMENTS`]. Counts
/// elements and openings together because both consume memory during scene
/// building.
pub fn validate_element_budget(
    element_count: usize,
    opening_count: usize,
) -> Result<(), ProtoError> {
    let count = element_count.saturating_add(opening_count);
    if count > MAX_DOCUMENT_ELEMENTS {
        return Err(ProtoError::TooManyElements {
            count,
            max: MAX_DOCUMENT_ELEMENTS,
        });
    }
    Ok(())
}

fn is_forbidden_display_char(c: char) -> bool {
    // C0/C1 controls, line/paragraph separators, bidi overrides and other
    // Unicode format characters, plus private-use code points. These can be
    // used to spoof or hide instruction-like text in names returned to agents.
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}')
        || matches!(
            c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFEFF
        )
        || matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Trim and validate a free-text display name.
///
/// Names are returned to agents, so they are length-limited and must not carry
/// control characters, bidi overrides or invisible formatting characters that
/// could be used to fabricate instruction-like text (architecture section 7.6).
pub fn sanitize_name(name: &str) -> Result<String, ProtoError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProtoError::NameEmpty);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ProtoError::NameHasControlChars);
    }
    if trimmed.chars().any(is_forbidden_display_char) {
        return Err(ProtoError::NameHasForbiddenChars);
    }
    let chars = trimmed.chars().count();
    if chars > MAX_NAME_CHARS {
        return Err(ProtoError::NameTooLong {
            chars,
            max: MAX_NAME_CHARS,
        });
    }
    Ok(trimmed.to_string())
}

/// Validate a stable reference id.
///
/// Reference ids are echoed back in summaries and used as map keys, so they are
/// limited to a conservative ASCII identifier alphabet. Control characters,
/// bidi overrides and other invisible characters are rejected as invalid.
pub fn sanitize_ref_id(ref_id: &str) -> Result<(), ProtoError> {
    if ref_id.is_empty() {
        return Err(ProtoError::RefIdEmpty);
    }
    let chars = ref_id.chars().count();
    if chars > MAX_REF_ID_CHARS {
        return Err(ProtoError::RefIdTooLong {
            chars,
            max: MAX_REF_ID_CHARS,
        });
    }
    let valid = ref_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err(ProtoError::RefIdHasInvalidChars);
    }
    Ok(())
}

fn validate_optional_ref_id(ref_id: &Option<String>) -> Result<(), ProtoError> {
    match ref_id {
        Some(ref_id) => sanitize_ref_id(ref_id),
        None => Ok(()),
    }
}

fn check_coordinate(field: &'static str, value: i64) -> Result<(), ProtoError> {
    if !(MIN_COORD_MM..=MAX_COORD_MM).contains(&value) {
        return Err(ProtoError::CoordinateOutOfRange {
            field,
            value,
            min: MIN_COORD_MM,
            max: MAX_COORD_MM,
        });
    }
    Ok(())
}

fn check_dimension(field: &'static str, value: i64, min: i64, max: i64) -> Result<(), ProtoError> {
    if !(min..=max).contains(&value) {
        return Err(ProtoError::DimensionOutOfRange {
            field,
            value,
            min,
            max,
        });
    }
    Ok(())
}

/// Bound a wall run by comparing squared distances, which avoids both floating
/// point and `i64` overflow (coordinates are already range-checked).
fn check_wall_length(start_mm: [i64; 2], end_mm: [i64; 2]) -> Result<(), ProtoError> {
    if start_mm == end_mm {
        return Err(ProtoError::WallStartEqualsEnd);
    }
    let dx = i128::from(end_mm[0]) - i128::from(start_mm[0]);
    let dz = i128::from(end_mm[1]) - i128::from(start_mm[1]);
    let length_sq = dx * dx + dz * dz;
    let min_sq = MIN_WALL_LENGTH_MM * MIN_WALL_LENGTH_MM;
    let max_sq = MAX_WALL_LENGTH_MM * MAX_WALL_LENGTH_MM;
    if length_sq < min_sq || length_sq > max_sq {
        return Err(ProtoError::WallLengthOutOfRange {
            min_mm: MIN_WALL_LENGTH_MM,
            max_mm: MAX_WALL_LENGTH_MM,
        });
    }
    Ok(())
}

fn int_pair_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": { "type": "integer", "minimum": MIN_COORD_MM, "maximum": MAX_COORD_MM },
        "minItems": 2,
        "maxItems": 2
    })
}

fn element_summary_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "minimum": 0 },
            "kind": { "type": "string", "enum": ["wall", "tower"] },
            "name": { "type": "string" },
            "height_mm": { "type": "integer" },
            "thickness_mm": { "type": "integer" },
            "start_mm": int_pair_schema(),
            "end_mm": int_pair_schema(),
            "centre_mm": int_pair_schema(),
            "radius_mm": { "type": "integer", "minimum": MIN_TOWER_RADIUS_MM, "maximum": MAX_TOWER_RADIUS_MM },
            "length_mm": { "type": "integer" }
        },
        "required": ["id", "kind", "name", "height_mm", "thickness_mm"]
    })
}

fn opening_summary_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "minimum": 0 },
            "wall_id": { "type": "integer", "minimum": 0 },
            "kind": { "type": "string", "enum": ["arrow_loop"] },
            "name": { "type": "string" },
            "t_permille": { "type": "integer", "minimum": 0, "maximum": 1000 },
            "sill_height_mm": { "type": "integer" },
            "height_mm": { "type": "integer" },
            "outer_slit_mm": { "type": "integer" },
            "inner_splay_width_mm": { "type": "integer" }
        },
        "required": [
            "id", "wall_id", "kind", "name", "t_permille",
            "sill_height_mm", "height_mm", "outer_slit_mm", "inner_splay_width_mm"
        ]
    })
}

fn castle_summary_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "document_hash": { "type": "string" },
            "layout_hash": { "type": "string" },
            "element_count": { "type": "integer", "minimum": 0 },
            "opening_count": { "type": "integer", "minimum": 0 },
            "block_count": { "type": "integer", "minimum": 0 },
            "chunk_count": { "type": "integer", "minimum": 0 },
            "elements": {
                "type": "array",
                "items": element_summary_schema()
            },
            "openings": {
                "type": "array",
                "items": opening_summary_schema()
            }
        },
        "required": [
            "document_hash", "layout_hash", "element_count", "opening_count",
            "block_count", "chunk_count", "elements", "openings"
        ]
    })
}

fn element_list_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "element_count": { "type": "integer", "minimum": 0 },
            "opening_count": { "type": "integer", "minimum": 0 },
            "elements": {
                "type": "array",
                "items": element_summary_schema()
            },
            "openings": {
                "type": "array",
                "items": opening_summary_schema()
            }
        },
        "required": ["element_count", "opening_count", "elements", "openings"]
    })
}

fn render_plan_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "view": { "type": "string", "enum": ["top_plan", "elevation_north"] },
            "document_hash": { "type": "string" },
            "mime_type": { "type": "string", "enum": ["image/svg+xml"] },
            "svg": { "type": "string" }
        },
        "required": ["view", "document_hash", "mime_type", "svg"]
    })
}

fn apply_operations_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "applied_count": { "type": "integer", "minimum": 0 },
            "created_ids": {
                "type": "object",
                "additionalProperties": { "type": "integer", "minimum": 0 }
            },
            "document_hash": { "type": "string" },
            "layout_hash": { "type": "string" },
            "block_count": { "type": "integer", "minimum": 0 },
            "chunk_count": { "type": "integer", "minimum": 0 },
            "summary": { "type": "string" }
        },
        "required": [
            "applied_count", "created_ids", "document_hash", "layout_hash",
            "block_count", "chunk_count", "summary"
        ]
    })
}

fn operation_schema() -> serde_json::Value {
    let ref_id = serde_json::json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_REF_ID_CHARS,
        "pattern": "^[A-Za-z0-9_-]+$"
    });
    serde_json::json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "op": { "const": "create_wall_run" },
                    "ref_id": ref_id,
                    "name": { "type": "string", "minLength": 1, "maxLength": MAX_NAME_CHARS },
                    "start_mm": int_pair_schema(),
                    "end_mm": int_pair_schema(),
                    "height_mm": { "type": "integer", "minimum": MIN_WALL_HEIGHT_MM, "maximum": MAX_WALL_HEIGHT_MM },
                    "thickness_mm": { "type": "integer", "minimum": MIN_WALL_THICKNESS_MM, "maximum": MAX_WALL_THICKNESS_MM },
                    "batter_mm": { "type": "integer", "minimum": 0, "maximum": MAX_BATTER_MM }
                },
                "required": ["op", "name", "start_mm", "end_mm", "height_mm", "thickness_mm"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "create_tower" },
                    "ref_id": ref_id,
                    "name": { "type": "string", "minLength": 1, "maxLength": MAX_NAME_CHARS },
                    "centre_mm": int_pair_schema(),
                    "radius_mm": { "type": "integer", "minimum": MIN_TOWER_RADIUS_MM, "maximum": MAX_TOWER_RADIUS_MM },
                    "height_mm": { "type": "integer", "minimum": MIN_TOWER_HEIGHT_MM, "maximum": MAX_TOWER_HEIGHT_MM },
                    "batter_mm": { "type": "integer", "minimum": 0, "maximum": MAX_BATTER_MM }
                },
                "required": ["op", "name", "centre_mm", "radius_mm", "height_mm"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "add_arrow_loop" },
                    "wall_id": { "type": "integer", "minimum": 0 },
                    "wall_ref": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_REF_ID_CHARS,
                        "pattern": "^[A-Za-z0-9_-]+$"
                    },
                    "t_permille": { "type": "integer", "minimum": MIN_LOOP_T_PERMILLE, "maximum": MAX_LOOP_T_PERMILLE },
                    "sill_height_mm": { "type": "integer", "minimum": 0, "maximum": MAX_LOOP_SILL_HEIGHT_MM },
                    "height_mm": { "type": "integer", "minimum": MIN_LOOP_HEIGHT_MM, "maximum": MAX_LOOP_HEIGHT_MM },
                    "outer_slit_mm": { "type": "integer", "minimum": MIN_OUTER_SLIT_MM, "maximum": MAX_OUTER_SLIT_MM },
                    "inner_splay_width_mm": { "type": "integer", "minimum": MIN_INNER_SPLAY_WIDTH_MM, "maximum": MAX_INNER_SPLAY_WIDTH_MM }
                },
                "required": ["op", "t_permille", "sill_height_mm"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "op": { "const": "delete_element" },
                    "id": { "type": "integer", "minimum": 0 }
                },
                "required": ["op", "id"],
                "additionalProperties": false
            }
        ]
    })
}

/// MCP tool definitions: name, description, input/output schemas and
/// annotations. Hosts use the annotations to auto-approve reads and renders and
/// gate edits; the schemas make the JSON-RPC surface self-describing.
pub fn tool_definitions() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "get_castle",
            "description": "Summarise the castle: hashes, counts and element/openings lists. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "outputSchema": castle_summary_schema(),
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        },
        {
            "name": "list_elements",
            "description": "List castle elements with ids, kinds and quantised dimensions. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "outputSchema": element_list_schema(),
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        },
        {
            "name": "render_plan",
            "description": "Render a deterministic SVG top plan or north elevation of the castle. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "view": {
                        "type": "string",
                        "enum": ["top_plan", "elevation_north"],
                        "default": "top_plan"
                    }
                },
                "additionalProperties": false
            },
            "outputSchema": render_plan_schema(),
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        },
        {
            "name": "apply_operations",
            "description": "Apply an atomic batch of create/add/delete operations. Later operations may reference earlier ones via ref_id or wall_ref.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operations": {
                        "type": "array",
                        "items": operation_schema(),
                        "minItems": 0,
                        "maxItems": MAX_BATCH_OPS
                    }
                },
                "required": ["operations"],
                "additionalProperties": false
            },
            "outputSchema": apply_operations_schema(),
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": false
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn operations_round_trip_with_tagged_names() {
        let operation = Operation::CreateWallRun {
            ref_id: Some("north".into()),
            name: "North curtain".into(),
            start_mm: [-8_000, 0],
            end_mm: [18_000, 0],
            height_mm: 9_000,
            thickness_mm: 2_500,
            batter_mm: Some(800),
        };
        let value = serde_json::to_value(&operation).unwrap();
        assert_eq!(value["op"], "create_wall_run");
        assert_eq!(value["start_mm"], json!([-8_000, 0]));
        let parsed: Operation = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, operation);

        let loop_op: Operation = serde_json::from_value(json!({
            "op": "add_arrow_loop",
            "wall_ref": "north",
            "t_permille": 280,
            "sill_height_mm": 1_800
        }))
        .unwrap();
        assert_eq!(
            loop_op,
            Operation::AddArrowLoop {
                wall_id: None,
                wall_ref: Some("north".into()),
                t_permille: 280,
                sill_height_mm: 1_800,
                height_mm: None,
                outer_slit_mm: None,
                inner_splay_width_mm: None,
            }
        );

        let tower: Operation = serde_json::from_value(json!({
            "op": "create_tower",
            "name": "Drum",
            "centre_mm": [19_000, 0],
            "radius_mm": 4_200,
            "height_mm": 15_000
        }))
        .unwrap();
        assert!(matches!(tower, Operation::CreateTower { .. }));

        let delete: Operation =
            serde_json::from_value(json!({ "op": "delete_element", "id": 7 })).unwrap();
        assert_eq!(delete, Operation::DeleteElement { id: 7 });
    }

    #[test]
    fn batch_limit_is_enforced() {
        assert!(validate_batch(&[]).is_ok());
        assert!(validate_batch(&vec![Operation::DeleteElement { id: 1 }; MAX_BATCH_OPS]).is_ok());
        let too_many = vec![Operation::DeleteElement { id: 1 }; MAX_BATCH_OPS + 1];
        assert_eq!(
            validate_batch(&too_many),
            Err(ProtoError::BatchTooLarge {
                count: MAX_BATCH_OPS + 1,
                max: MAX_BATCH_OPS
            })
        );
    }

    #[test]
    fn names_are_trimmed_limited_and_control_free() {
        assert_eq!(sanitize_name("  North curtain  ").unwrap(), "North curtain");
        assert_eq!(
            sanitize_name(&"a".repeat(MAX_NAME_CHARS)).unwrap(),
            "a".repeat(MAX_NAME_CHARS)
        );
        assert_eq!(sanitize_name("   "), Err(ProtoError::NameEmpty));
        assert_eq!(
            sanitize_name(&"a".repeat(MAX_NAME_CHARS + 1)),
            Err(ProtoError::NameTooLong {
                chars: MAX_NAME_CHARS + 1,
                max: MAX_NAME_CHARS
            })
        );
        assert_eq!(
            sanitize_name("bad\nname"),
            Err(ProtoError::NameHasControlChars)
        );

        let operation = Operation::CreateTower {
            ref_id: None,
            name: "  Drum tower  ".into(),
            centre_mm: [0, 0],
            radius_mm: 3_000,
            height_mm: 12_000,
            batter_mm: None,
        };
        assert_eq!(
            operation.validated().unwrap(),
            Operation::CreateTower {
                ref_id: None,
                name: "Drum tower".into(),
                centre_mm: [0, 0],
                radius_mm: 3_000,
                height_mm: 12_000,
                batter_mm: None,
            }
        );
    }

    #[test]
    fn names_reject_bidi_formatting_and_private_use_chars() {
        for bad in [
            "curtain \u{202E}evil\u{202C}",
            "zero\u{200B}width",
            "word\u{2060}joiner",
            "bom\u{FEFF}name",
            "line\u{2028}sep",
            "para\u{2029}sep",
            "private\u{E000}use",
        ] {
            assert_eq!(
                sanitize_name(bad),
                Err(ProtoError::NameHasForbiddenChars),
                "expected rejection of {bad:?}"
            );
        }
        // Zero-width and bidi characters in names are rejected even when only
        // used as invisible padding.
        assert_eq!(
            sanitize_name("\u{200F}North curtain"),
            Err(ProtoError::NameHasForbiddenChars)
        );
    }

    #[test]
    fn ref_ids_are_length_limited_to_safe_identifiers() {
        assert_eq!(sanitize_ref_id("north-1_A"), Ok(()));
        assert_eq!(sanitize_ref_id(""), Err(ProtoError::RefIdEmpty));
        assert_eq!(
            sanitize_ref_id(&"a".repeat(MAX_REF_ID_CHARS + 1)),
            Err(ProtoError::RefIdTooLong {
                chars: MAX_REF_ID_CHARS + 1,
                max: MAX_REF_ID_CHARS
            })
        );
        for bad in [
            "north wall",
            "north\nwall",
            "north\u{202E}wall",
            "north/wall",
            "nörd",
        ] {
            assert_eq!(
                sanitize_ref_id(bad),
                Err(ProtoError::RefIdHasInvalidChars),
                "expected rejection of {bad:?}"
            );
        }

        let operation = Operation::CreateWallRun {
            ref_id: Some("bad ref".into()),
            name: "Wall".into(),
            start_mm: [0, 0],
            end_mm: [10_000, 0],
            height_mm: 8_000,
            thickness_mm: 2_000,
            batter_mm: None,
        };
        assert_eq!(operation.validated(), Err(ProtoError::RefIdHasInvalidChars));
    }

    #[test]
    fn wall_geometry_bounds_are_enforced() {
        let base = Operation::CreateWallRun {
            ref_id: None,
            name: "Wall".into(),
            start_mm: [0, 0],
            end_mm: [10_000, 0],
            height_mm: 8_000,
            thickness_mm: 2_000,
            batter_mm: None,
        };
        assert!(base.validated().is_ok());

        let mut too_short = base.clone();
        if let Operation::CreateWallRun { end_mm, .. } = &mut too_short {
            *end_mm = [400, 0];
        }
        assert_eq!(
            too_short.validated(),
            Err(ProtoError::WallLengthOutOfRange {
                min_mm: MIN_WALL_LENGTH_MM,
                max_mm: MAX_WALL_LENGTH_MM
            })
        );

        let mut too_long = base.clone();
        if let Operation::CreateWallRun {
            start_mm, end_mm, ..
        } = &mut too_long
        {
            *start_mm = [-250_000, 0];
            *end_mm = [250_001, 0];
        }
        assert_eq!(
            too_long.validated(),
            Err(ProtoError::WallLengthOutOfRange {
                min_mm: MIN_WALL_LENGTH_MM,
                max_mm: MAX_WALL_LENGTH_MM
            })
        );

        let mut huge = base.clone();
        if let Operation::CreateWallRun {
            start_mm, end_mm, ..
        } = &mut huge
        {
            *start_mm = [0, 0];
            *end_mm = [9_000_000_000, 0];
        }
        assert_eq!(
            huge.validated(),
            Err(ProtoError::CoordinateOutOfRange {
                field: "end_mm",
                value: 9_000_000_000,
                min: MIN_COORD_MM,
                max: MAX_COORD_MM
            })
        );

        let mut tall = base.clone();
        if let Operation::CreateWallRun { height_mm, .. } = &mut tall {
            *height_mm = 900_000_000;
        }
        assert_eq!(
            tall.validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "height_mm",
                value: 900_000_000,
                min: MIN_WALL_HEIGHT_MM,
                max: MAX_WALL_HEIGHT_MM
            })
        );

        let mut thin = base;
        if let Operation::CreateWallRun { thickness_mm, .. } = &mut thin {
            *thickness_mm = 0;
        }
        assert_eq!(
            thin.validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "thickness_mm",
                value: 0,
                min: MIN_WALL_THICKNESS_MM,
                max: MAX_WALL_THICKNESS_MM
            })
        );
    }

    #[test]
    fn tower_and_loop_geometry_bounds_are_enforced() {
        let tower = Operation::CreateTower {
            ref_id: None,
            name: "Tower".into(),
            centre_mm: [0, 0],
            radius_mm: 400_000_000,
            height_mm: 12_000,
            batter_mm: None,
        };
        assert_eq!(
            tower.validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "radius_mm",
                value: 400_000_000,
                min: MIN_TOWER_RADIUS_MM,
                max: MAX_TOWER_RADIUS_MM
            })
        );

        let loop_op = |t_permille, sill_height_mm| Operation::AddArrowLoop {
            wall_id: Some(1),
            wall_ref: None,
            t_permille,
            sill_height_mm,
            height_mm: None,
            outer_slit_mm: None,
            inner_splay_width_mm: None,
        };
        assert!(loop_op(50, 0).validated().is_ok());
        assert!(loop_op(950, 20_000).validated().is_ok());
        assert_eq!(
            loop_op(49, 1_000).validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "t_permille",
                value: 49,
                min: MIN_LOOP_T_PERMILLE,
                max: MAX_LOOP_T_PERMILLE
            })
        );
        assert_eq!(
            loop_op(951, 1_000).validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "t_permille",
                value: 951,
                min: MIN_LOOP_T_PERMILLE,
                max: MAX_LOOP_T_PERMILLE
            })
        );
        assert_eq!(
            loop_op(500, -1).validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "sill_height_mm",
                value: -1,
                min: 0,
                max: MAX_LOOP_SILL_HEIGHT_MM
            })
        );

        let mut tall_loop = loop_op(500, 1_000);
        if let Operation::AddArrowLoop { height_mm, .. } = &mut tall_loop {
            *height_mm = Some(9_000);
        }
        assert_eq!(
            tall_loop.validated(),
            Err(ProtoError::DimensionOutOfRange {
                field: "height_mm",
                value: 9_000,
                min: MIN_LOOP_HEIGHT_MM,
                max: MAX_LOOP_HEIGHT_MM
            })
        );
    }

    #[test]
    fn render_plan_request_defaults_to_top_plan() {
        assert_eq!(
            serde_json::from_value::<RenderPlanRequest>(json!({})).unwrap(),
            RenderPlanRequest {
                view: "top_plan".into()
            }
        );
        assert_eq!(RenderPlanRequest::default().view, "top_plan");
        assert_eq!(
            serde_json::from_value::<RenderPlanRequest>(json!({ "view": "elevation_north" }))
                .unwrap()
                .view,
            "elevation_north"
        );
    }

    #[test]
    fn tool_definitions_cover_the_offline_surface() {
        let tools = tool_definitions();
        let tools = tools.as_array().expect("tools array");
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "get_castle",
                "list_elements",
                "render_plan",
                "apply_operations"
            ]
        );
        for tool in tools {
            assert!(tool["inputSchema"].is_object());
            assert!(tool["outputSchema"].is_object());
            assert!(tool["annotations"]["readOnlyHint"].is_boolean());
            assert!(tool["annotations"]["destructiveHint"].is_boolean());
            assert!(tool["annotations"]["idempotentHint"].is_boolean());
        }
        let apply = &tools[3];
        assert_eq!(apply["annotations"]["readOnlyHint"], false);
        assert_eq!(apply["annotations"]["destructiveHint"], true);
        assert_eq!(apply["annotations"]["idempotentHint"], false);
        assert_eq!(
            apply["inputSchema"]["properties"]["operations"]["maxItems"],
            MAX_BATCH_OPS
        );

        // Numeric bounds are mirrored from the Rust-side checks.
        let wall = &apply["inputSchema"]["properties"]["operations"]["items"]["oneOf"][0];
        assert_eq!(
            wall["properties"]["height_mm"]["minimum"],
            MIN_WALL_HEIGHT_MM
        );
        assert_eq!(
            wall["properties"]["height_mm"]["maximum"],
            MAX_WALL_HEIGHT_MM
        );
        assert_eq!(
            wall["properties"]["thickness_mm"]["minimum"],
            MIN_WALL_THICKNESS_MM
        );
        assert_eq!(
            wall["properties"]["thickness_mm"]["maximum"],
            MAX_WALL_THICKNESS_MM
        );
        assert_eq!(wall["properties"]["batter_mm"]["maximum"], MAX_BATTER_MM);
        assert_eq!(
            wall["properties"]["start_mm"]["items"]["minimum"],
            MIN_COORD_MM
        );
        assert_eq!(
            wall["properties"]["start_mm"]["items"]["maximum"],
            MAX_COORD_MM
        );

        let tower = &apply["inputSchema"]["properties"]["operations"]["items"]["oneOf"][1];
        assert_eq!(
            tower["properties"]["radius_mm"]["minimum"],
            MIN_TOWER_RADIUS_MM
        );
        assert_eq!(
            tower["properties"]["radius_mm"]["maximum"],
            MAX_TOWER_RADIUS_MM
        );

        let arrow_loop = &apply["inputSchema"]["properties"]["operations"]["items"]["oneOf"][2];
        assert_eq!(
            arrow_loop["properties"]["t_permille"]["minimum"],
            MIN_LOOP_T_PERMILLE
        );
        assert_eq!(
            arrow_loop["properties"]["t_permille"]["maximum"],
            MAX_LOOP_T_PERMILLE
        );
        assert_eq!(
            arrow_loop["properties"]["sill_height_mm"]["maximum"],
            MAX_LOOP_SILL_HEIGHT_MM
        );
        assert_eq!(
            arrow_loop["properties"]["outer_slit_mm"]["minimum"],
            MIN_OUTER_SLIT_MM
        );
        assert_eq!(
            arrow_loop["properties"]["inner_splay_width_mm"]["maximum"],
            MAX_INNER_SPLAY_WIDTH_MM
        );

        // Round-trip every tool definition through JSON to make sure it is
        // actually serialisable and stable.
        let text = serde_json::to_string(&tool_definitions()).unwrap();
        let reparsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reparsed, tool_definitions());
    }

    #[test]
    fn document_element_budget_is_enforced() {
        assert!(validate_element_budget(0, 0).is_ok());
        assert!(validate_element_budget(MAX_DOCUMENT_ELEMENTS, 0).is_ok());
        assert!(validate_element_budget(0, MAX_DOCUMENT_ELEMENTS).is_ok());
        assert_eq!(
            validate_element_budget(MAX_DOCUMENT_ELEMENTS, 1),
            Err(ProtoError::TooManyElements {
                count: MAX_DOCUMENT_ELEMENTS + 1,
                max: MAX_DOCUMENT_ELEMENTS
            })
        );
    }

    #[test]
    fn responses_serialise() {
        let response = ApplyOperationsResponse {
            applied_count: 2,
            created_ids: BTreeMap::from([("north".to_string(), 1)]),
            document_hash: "0000000000000001".into(),
            layout_hash: "0000000000000002".into(),
            block_count: 10,
            chunk_count: 2,
            summary: "Applied 2 operations".into(),
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["created_ids"]["north"], 1);
        let parsed: ApplyOperationsResponse = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, response);
    }
}
