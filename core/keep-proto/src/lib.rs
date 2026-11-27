//! Versioned control-protocol types for Keepwright, shared by `keep-mcp`, the
//! live game control server and tests.
//!
//! The wire format is deliberately small and JSON-shaped: agents send semantic
//! operations (integer millimetres, relative anchors) and receive compact
//! summaries, hashes and rendered plan SVGs.
//!
//! The semantic [`Operation`] type and every validation rule are re-exported
//! from `keep-core`'s command engine, so the protocol schema, the offline
//! sidecar and the game host share exactly one implementation. Security limits
//! ([`MAX_BATCH_OPS`], [`MAX_NAME_CHARS`], …) are likewise single-sourced and
//! used directly by [`tool_definitions`] (architecture section 7.6).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// One command engine for every transport: the wire protocol re-exports the
// `keep-core` operation type, validation helpers and bounds instead of keeping a
// second copy that could drift (architecture sections 7.6 and 15).
pub use keep_core::command::{
    sanitize_name, sanitize_ref_id, validate_batch, validate_element_budget,
    CommandError as ProtoError, CommandOp as Operation, MAX_BATCH_OPS, MAX_BATTER_MM, MAX_COORD_MM,
    MAX_DOCUMENT_ELEMENTS, MAX_INNER_SPLAY_WIDTH_MM, MAX_LOOP_HEIGHT_MM, MAX_LOOP_SILL_HEIGHT_MM,
    MAX_LOOP_T_PERMILLE, MAX_NAME_CHARS, MAX_OUTER_SLIT_MM, MAX_REF_ID_CHARS, MAX_TOWER_HEIGHT_MM,
    MAX_TOWER_RADIUS_MM, MAX_WALL_HEIGHT_MM, MAX_WALL_LENGTH_MM, MAX_WALL_THICKNESS_MM,
    MIN_COORD_MM, MIN_INNER_SPLAY_WIDTH_MM, MIN_LOOP_HEIGHT_MM, MIN_LOOP_T_PERMILLE,
    MIN_OUTER_SLIT_MM, MIN_TOWER_HEIGHT_MM, MIN_TOWER_RADIUS_MM, MIN_WALL_HEIGHT_MM,
    MIN_WALL_LENGTH_MM, MIN_WALL_THICKNESS_MM,
};

/// Default view used by `render_plan` when no view is supplied.
pub const DEFAULT_PLAN_VIEW: &str = "top_plan";

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

/// Result of the `undo` tool: the compensating command and the restored state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoResponse {
    /// Sequence number of the newly appended compensating command.
    pub seq: u64,
    /// Sequence number of the batch that was compensated.
    pub compensated_seq: u64,
    pub document_hash: String,
    pub layout_hash: String,
    pub block_count: usize,
    pub chunk_count: usize,
    pub summary: String,
}

/// One entry in the append-only command history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub seq: u64,
    pub command_id: String,
    pub author: String,
    pub summary: String,
    pub touched_elements: Vec<u64>,
    /// True when a later undo compensated this batch.
    pub undone: bool,
}

/// Result of the `get_history` tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetHistoryResponse {
    pub count: usize,
    pub entries: Vec<HistoryEntry>,
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

fn undo_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "seq": { "type": "integer", "minimum": 0 },
            "compensated_seq": { "type": "integer", "minimum": 0 },
            "document_hash": { "type": "string" },
            "layout_hash": { "type": "string" },
            "block_count": { "type": "integer", "minimum": 0 },
            "chunk_count": { "type": "integer", "minimum": 0 },
            "summary": { "type": "string" }
        },
        "required": [
            "seq", "compensated_seq", "document_hash", "layout_hash",
            "block_count", "chunk_count", "summary"
        ]
    })
}

fn history_entry_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "seq": { "type": "integer", "minimum": 0 },
            "command_id": { "type": "string" },
            "author": { "type": "string" },
            "summary": { "type": "string" },
            "touched_elements": {
                "type": "array",
                "items": { "type": "integer", "minimum": 0 }
            },
            "undone": { "type": "boolean" }
        },
        "required": [
            "seq", "command_id", "author", "summary", "touched_elements", "undone"
        ]
    })
}

fn get_history_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "count": { "type": "integer", "minimum": 0 },
            "entries": {
                "type": "array",
                "items": history_entry_schema()
            }
        },
        "required": ["count", "entries"]
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
        },
        {
            "name": "undo",
            "description": "Undo this agent's latest still-live batch by appending a compensating command. Refused when a later command touched the same elements.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "outputSchema": undo_schema(),
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": false
            }
        },
        {
            "name": "get_history",
            "description": "Read the append-only command history with sequence numbers, authors and undone markers. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "outputSchema": get_history_schema(),
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
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
                "apply_operations",
                "undo",
                "get_history"
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

        // Undo is a destructive (compensating) write, so hosts must gate it.
        let undo = &tools[4];
        assert_eq!(undo["name"], "undo");
        assert_eq!(undo["annotations"]["destructiveHint"], true);

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

    #[test]
    fn undo_and_history_responses_serialise() {
        let undo = UndoResponse {
            seq: 2,
            compensated_seq: 1,
            document_hash: "0000000000000003".into(),
            layout_hash: "0000000000000004".into(),
            block_count: 10,
            chunk_count: 2,
            summary: "Undo command #1".into(),
        };
        let value = serde_json::to_value(&undo).unwrap();
        assert_eq!(value["compensated_seq"], 1);
        assert_eq!(serde_json::from_value::<UndoResponse>(value).unwrap(), undo);

        let history = GetHistoryResponse {
            count: 1,
            entries: vec![HistoryEntry {
                seq: 1,
                command_id: "018bcfe5-6800-7000-8000-000000000001".into(),
                author: "agent:mcp-offline".into(),
                summary: "+1 wall".into(),
                touched_elements: vec![12],
                undone: true,
            }],
        };
        let value = serde_json::to_value(&history).unwrap();
        assert_eq!(value["entries"][0]["undone"], true);
        assert_eq!(
            serde_json::from_value::<GetHistoryResponse>(value).unwrap(),
            history
        );
    }
}
