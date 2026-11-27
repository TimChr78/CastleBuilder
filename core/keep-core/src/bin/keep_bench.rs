//! `keep-bench`: deterministic Release 1 performance benchmark.
//!
//! Builds the four canonical fixtures through `keep_core::mesh::build_scene`
//! and reports block count, chunk count, LOD0 triangle count, wall-clock build
//! time and the layout hash. Run with `--json` for machine-readable output.

use std::time::Instant;

use keep_core::document::CastleDocument;
use keep_core::fixtures::{
    m0a_showcase_document, m1_edwardian_document, m2_baltic_document, stress_200k_document,
};
use keep_core::mesh::build_scene;
use keep_core::terrain::SiteTerrain;

struct Fixture {
    name: &'static str,
    site: &'static str,
    document: fn() -> CastleDocument,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "m0a_showcase",
        site: "m0a_promontory",
        document: m0a_showcase_document,
    },
    Fixture {
        name: "m1_edwardian",
        site: "coastal_headland",
        document: m1_edwardian_document,
    },
    Fixture {
        name: "m2_baltic",
        site: "lake_island",
        document: m2_baltic_document,
    },
    Fixture {
        name: "stress_200k",
        site: "m0a_promontory",
        document: stress_200k_document,
    },
];

struct Row {
    fixture: &'static str,
    site: &'static str,
    blocks: usize,
    chunks: usize,
    lod0_tris: usize,
    build_ms: f64,
    layout_hash: u64,
}

fn measure(fixture: &Fixture) -> Row {
    let terrain = SiteTerrain::for_site(fixture.site)
        .unwrap_or_else(|| panic!("unknown curated site: {}", fixture.site));
    let document = (fixture.document)();

    let started = Instant::now();
    let scene = build_scene(&document, &terrain);
    let build_ms = started.elapsed().as_secs_f64() * 1_000.0;

    Row {
        fixture: fixture.name,
        site: fixture.site,
        blocks: scene.block_count,
        chunks: scene.chunk_count(),
        lod0_tris: scene.triangle_count,
        build_ms,
        layout_hash: scene.layout_hash,
    }
}

fn print_table(rows: &[Row]) {
    let headers = [
        "fixture",
        "site",
        "blocks",
        "chunks",
        "lod0_tris",
        "build_ms",
        "layout_hash",
    ];

    let mut values: Vec<[String; 7]> = Vec::with_capacity(rows.len());
    for row in rows {
        values.push([
            row.fixture.to_string(),
            row.site.to_string(),
            group_digits(row.blocks),
            group_digits(row.chunks),
            group_digits(row.lod0_tris),
            format!("{:.1}", row.build_ms),
            format!("{:016x}", row.layout_hash),
        ]);
    }

    let mut widths = headers.map(str::len);
    for row in &values {
        for (width, value) in widths.iter_mut().zip(row.iter()) {
            *width = (*width).max(value.len());
        }
    }

    let print_row = |cells: &[String; 7]| {
        let mut line = String::new();
        for (index, (width, cell)) in widths.iter().zip(cells.iter()).enumerate() {
            if index > 0 {
                line.push_str("  ");
            }
            if index <= 1 {
                line.push_str(&format!("{cell:<width$}"));
            } else {
                line.push_str(&format!("{cell:>width$}"));
            }
        }
        println!("{}", line.trim_end());
    };

    let header_cells: [String; 7] = headers.map(str::to_string);
    print_row(&header_cells);
    for row in &values {
        print_row(row);
    }
}

fn print_json(rows: &[Row]) {
    let fixtures: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "fixture": row.fixture,
                "site": row.site,
                "blocks": row.blocks,
                "chunks": row.chunks,
                "lod0_tris": row.lod0_tris,
                "build_ms": (row.build_ms * 1_000.0).round() / 1_000.0,
                "layout_hash": format!("{:016x}", row.layout_hash),
            })
        })
        .collect();

    let report = serde_json::json!({
        "generator_version": keep_core::GENERATOR_VERSION,
        "fixtures": fixtures,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("benchmark report serialises")
    );
}

fn group_digits(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    grouped
}

fn main() {
    let json = std::env::args()
        .skip(1)
        .any(|argument| argument == "--json");
    let rows: Vec<Row> = FIXTURES.iter().map(measure).collect();

    if json {
        print_json(&rows);
    } else {
        print_table(&rows);
    }
}
