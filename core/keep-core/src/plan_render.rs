//! Pure-Rust plan and elevation renderer.
//!
//! This is the offline feedback loop for agents (architecture section 7.4): a
//! deterministic SVG built from the document and terrain authority with no
//! external crates, no GPU and no browser. Plans show the architectural intent
//! (walls, towers, openings) over terrain contours; elevations show how the
//! design sits on the sloping site.

use crate::document::{CastleDocument, Element};
use crate::terrain::SiteTerrain;
use libm::{round, sqrt};

/// Which view to render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanView {
    /// Architectural floor plan looking down: contours, walls, towers, loops.
    TopPlan,
    /// Side elevation looking from the north, with the terrain profile.
    ElevationNorth,
}

/// Render `document` over `terrain` as a standalone SVG document.
pub fn render_plan_svg(document: &CastleDocument, terrain: &SiteTerrain, view: PlanView) -> String {
    match view {
        PlanView::TopPlan => render_top_plan(document, terrain),
        PlanView::ElevationNorth => render_elevation(document, terrain),
    }
}

/// Format a metre value deterministically (two decimals, no negative zero).
fn n(value: f64) -> String {
    let value = if value.abs() < 0.005 { 0.0 } else { value };
    format!("{value:.2}")
}

fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn svg_header(width: f64, height: f64) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\" role=\"img\">\n",
        w = n(width),
        h = n(height)
    )
}

/// Inline presentation styles. Coordinates are in metres, so label sizes are
/// also in metres and scale with the viewBox.
const STYLE: &str = "<style>\n\
.contour{fill:none;stroke:#c3bcae;stroke-width:0.12;stroke-linecap:round;}\n\
.wall{fill:#d9c8a6;stroke:#3a3226;stroke-width:0.35;stroke-linejoin:round;}\n\
.tower{fill:#d4c09c;stroke:#3a3226;stroke-width:0.35;}\n\
.opening{fill:#f3e3ad;stroke:#8a4f1c;stroke-width:0.28;}\n\
.terrain-profile{fill:#e8ddc7;stroke:#8c7b5a;stroke-width:0.22;}\n\
.scale-bar{stroke:#1f1a12;stroke-width:0.6;stroke-linecap:round;}\n\
.north-arrow{fill:#1f1a12;stroke:none;}\n\
.element-label,.scale-label,.north-label{font-family:\"DejaVu Sans\",sans-serif;fill:#2a241a;}\n\
.element-label{font-size:1.2;}\n\
.scale-label{font-size:1.2;}\n\
.north-label{font-size:1.4;font-weight:bold;}\n\
</style>\n";

/// Terrain or element bounds in metres.
struct Bounds {
    min_x: f64,
    max_x: f64,
    min_z: f64,
    max_z: f64,
}

impl Bounds {
    fn terrain(terrain: &SiteTerrain) -> Self {
        Self {
            min_x: terrain.origin_mm() as f64 / 1000.0,
            max_x: terrain.extent_mm() as f64 / 1000.0,
            min_z: terrain.origin_mm() as f64 / 1000.0,
            max_z: terrain.extent_mm() as f64 / 1000.0,
        }
    }

    /// Tight plan bounds around the castle elements, expanded by each element's
    /// footprint so strokes and plinths are not clipped.
    fn elements(document: &CastleDocument) -> Option<Self> {
        let mut bounds: Option<Self> = None;
        let mut include = |x0: f64, z0: f64, x1: f64, z1: f64| {
            let entry = bounds.get_or_insert(Self {
                min_x: x0,
                max_x: x1,
                min_z: z0,
                max_z: z1,
            });
            entry.min_x = entry.min_x.min(x0);
            entry.max_x = entry.max_x.max(x1);
            entry.min_z = entry.min_z.min(z0);
            entry.max_z = entry.max_z.max(z1);
        };
        for element in document.elements() {
            match element {
                Element::BatteredWall(wall) => {
                    let half = (wall.thickness_mm as f64 * 0.5 + wall.plinth_projection_mm as f64)
                        / 1000.0;
                    let ax = wall.start_mm[0] as f64 / 1000.0;
                    let az = wall.start_mm[1] as f64 / 1000.0;
                    let bx = wall.end_mm[0] as f64 / 1000.0;
                    let bz = wall.end_mm[1] as f64 / 1000.0;
                    include(
                        ax.min(bx) - half,
                        az.min(bz) - half,
                        ax.max(bx) + half,
                        az.max(bz) + half,
                    );
                }
                Element::RoundTower(tower) => {
                    let cx = tower.centre_mm[0] as f64 / 1000.0;
                    let cz = tower.centre_mm[1] as f64 / 1000.0;
                    let r = (tower.radius_mm + tower.plinth_projection_mm) as f64 / 1000.0;
                    include(cx - r, cz - r, cx + r, cz + r);
                }
            }
        }
        bounds
    }

    fn padded(self, margin: f64) -> Self {
        Self {
            min_x: self.min_x - margin,
            max_x: self.max_x + margin,
            min_z: self.min_z - margin,
            max_z: self.max_z + margin,
        }
    }
}

fn height_at(terrain: &SiteTerrain, x_m: f64, z_m: f64) -> f64 {
    terrain.height_mm(round(x_m * 1000.0) as i64, round(z_m * 1000.0) as i64) as f64 / 1000.0
}

/// Marching-squares contour paths at `level` metres. Emits one `<path>` per
/// segment with a stable ordering, so output is byte-for-byte deterministic.
fn contour_paths(
    terrain: &SiteTerrain,
    bounds: &Bounds,
    level: f64,
    to_svg: &dyn Fn(f64, f64) -> (f64, f64),
) -> String {
    let stride_cells = 3;
    let cell_m = terrain.cell_mm() as f64 / 1000.0 * stride_cells as f64;
    let mut out = String::new();
    let mut z = bounds.min_z;
    while z + cell_m <= bounds.max_z + 1.0e-6 {
        let mut x = bounds.min_x;
        while x + cell_m <= bounds.max_x + 1.0e-6 {
            let corners = [
                height_at(terrain, x, z),
                height_at(terrain, x + cell_m, z),
                height_at(terrain, x + cell_m, z + cell_m),
                height_at(terrain, x, z + cell_m),
            ];
            let points = [
                (x, z),
                (x + cell_m, z),
                (x + cell_m, z + cell_m),
                (x, z + cell_m),
            ];
            let mut hits: Vec<(f64, f64)> = Vec::with_capacity(4);
            for edge in 0..4 {
                let a = corners[edge];
                let b = corners[(edge + 1) % 4];
                if (a - level) * (b - level) < 0.0 {
                    let t = (level - a) / (b - a);
                    let pa = points[edge];
                    let pb = points[(edge + 1) % 4];
                    hits.push((pa.0 + (pb.0 - pa.0) * t, pa.1 + (pb.1 - pa.1) * t));
                }
            }
            if hits.len() == 2 {
                push_contour_segment(&mut out, hits[0], hits[1], to_svg);
            } else if hits.len() == 4 {
                push_contour_segment(&mut out, hits[0], hits[1], to_svg);
                push_contour_segment(&mut out, hits[2], hits[3], to_svg);
            }
            x += cell_m;
        }
        z += cell_m;
    }
    out
}

fn push_contour_segment(
    out: &mut String,
    a: (f64, f64),
    b: (f64, f64),
    to_svg: &dyn Fn(f64, f64) -> (f64, f64),
) {
    let a = to_svg(a.0, a.1);
    let b = to_svg(b.0, b.1);
    out.push_str(&format!(
        "<path class=\"contour\" d=\"M {} {} L {} {}\"/>\n",
        n(a.0),
        n(a.1),
        n(b.0),
        n(b.1)
    ));
}

fn legend_and_scale(width: f64, height: f64, top: bool, north: bool) -> String {
    let y = if top { height - 9.0 } else { 9.0 };
    let mut out = format!(
        "<line class=\"scale-bar\" x1=\"8\" y1=\"{y1}\" x2=\"18\" y2=\"{y1}\"/>\n\
         <text class=\"scale-label\" x=\"8\" y=\"{ty}\">10 m</text>\n",
        y1 = n(y),
        ty = n(y + 3.0),
    );
    if north {
        // A real three-vertex triangle; the base spans 3 m so it has area.
        out.push_str(&format!(
            "<polygon class=\"north-arrow\" points=\"{nbsp1},{ny0} {nx1},{ny1} {nx3},{ny2}\"/>\n\
             <text class=\"north-label\" x=\"{nx2}\" y=\"{ny3}\">N</text>\n",
            nbsp1 = n(width - 10.0),
            nx1 = n(width - 7.0),
            nx3 = n(width - 13.0),
            ny0 = n(if top { height - 14.0 } else { 4.0 }),
            ny1 = n(if top { height - 8.0 } else { 10.0 }),
            ny2 = n(if top { height - 14.0 } else { 4.0 }),
            nx2 = n(width - 12.0),
            ny3 = n(if top { height - 6.0 } else { 12.0 }),
        ));
    }
    out
}

fn element_label(x: f64, y: f64, id: u64, name: &str) -> String {
    format!(
        "<text class=\"element-label\" x=\"{}\" y=\"{}\">#{} {}</text>\n",
        n(x),
        n(y),
        id,
        escape_xml(name)
    )
}

fn render_top_plan(document: &CastleDocument, terrain: &SiteTerrain) -> String {
    // Crop to the castle plus a 20 m margin (10 m on each side). Only an empty
    // document falls back to the full terrain extent.
    let bounds = Bounds::elements(document)
        .unwrap_or_else(|| Bounds::terrain(terrain))
        .padded(10.0);
    let world_w = (bounds.max_x - bounds.min_x).max(1.0);
    let world_h = (bounds.max_z - bounds.min_z).max(1.0);
    let width = world_w;
    let height = world_h;
    let to_svg = move |x: f64, z: f64| -> (f64, f64) { (x - bounds.min_x, (bounds.max_z - z)) };

    let mut svg = svg_header(width, height);
    svg.push_str(STYLE);
    svg.push_str(
        "<rect class=\"paper\" x=\"0\" y=\"0\" width=\"100%\" height=\"100%\" fill=\"#f6f1e6\"/>\n",
    );

    // Contours every 2.5 m over the site.
    let mut level = -15_000i64;
    while level <= 15_000 {
        svg.push_str(&contour_paths(
            terrain,
            &bounds,
            level as f64 / 1000.0,
            &to_svg,
        ));
        level += 2_500;
    }

    for element in document.elements() {
        match element {
            Element::BatteredWall(wall) => {
                let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
                let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
                let len = sqrt(dx * dx + dz * dz);
                if len < 1.0 {
                    continue;
                }
                let dir = [dx / len, dz / len];
                let right = [-dir[1], dir[0]];
                let half = wall.thickness_mm as f64 * 0.5;
                let a = [
                    wall.start_mm[0] as f64 / 1000.0,
                    wall.start_mm[1] as f64 / 1000.0,
                ];
                let b = [
                    wall.end_mm[0] as f64 / 1000.0,
                    wall.end_mm[1] as f64 / 1000.0,
                ];
                let mut points = Vec::with_capacity(4);
                for (base, sign) in [(a, -1.0), (b, -1.0), (b, 1.0), (a, 1.0)] {
                    let x = base[0] + right[0] * half * sign / 1000.0;
                    let z = base[1] + right[1] * half * sign / 1000.0;
                    points.push(to_svg(x, z));
                }
                let points: Vec<String> = points
                    .iter()
                    .map(|p| format!("{},{}", n(p.0), n(p.1)))
                    .collect();
                svg.push_str(&format!(
                    "<polygon class=\"wall\" points=\"{}\"/>\n",
                    points.join(" ")
                ));
                let mid = to_svg((a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5);
                svg.push_str(&element_label(mid.0, mid.1, wall.id, &wall.name));
            }
            Element::RoundTower(tower) => {
                let c = to_svg(
                    tower.centre_mm[0] as f64 / 1000.0,
                    tower.centre_mm[1] as f64 / 1000.0,
                );
                svg.push_str(&format!(
                    "<circle class=\"tower\" cx=\"{}\" cy=\"{}\" r=\"{}\"/>\n",
                    n(c.0),
                    n(c.1),
                    n(tower.radius_mm as f64 / 1000.0)
                ));
                svg.push_str(&element_label(
                    c.0 - tower.radius_mm as f64 / 1000.0,
                    c.1 - tower.radius_mm as f64 / 1000.0 - 1.0,
                    tower.id,
                    &tower.name,
                ));
            }
        }
    }

    // Openings: draw the splayed embrasure footprint through the wall.
    for opening in document.openings() {
        let Some(Element::BatteredWall(wall)) = document.get(opening.wall_id) else {
            continue;
        };
        let dx = (wall.end_mm[0] - wall.start_mm[0]) as f64;
        let dz = (wall.end_mm[1] - wall.start_mm[1]) as f64;
        let len = sqrt(dx * dx + dz * dz);
        if len < 1.0 {
            continue;
        }
        let dir = [dx / len, dz / len];
        let right = [-dir[1], dir[0]];
        let centre = opening.plan_point_mm(wall);
        let c = [centre[0] as f64 / 1000.0, centre[1] as f64 / 1000.0];
        let half = wall.thickness_mm as f64 * 0.5 / 1000.0;
        let outer = opening.outer_slit_mm as f64 * 0.5 / 1000.0;
        let inner = opening.inner_splay_width_mm as f64 * 0.5 / 1000.0;
        // Exterior is `-d` (left of the run). The narrow slit sits on the
        // exterior face and splays toward the interior, matching masonry.rs.
        let plan = [
            (
                c[0] - dir[0] * outer - right[0] * half,
                c[1] - dir[1] * outer - right[1] * half,
            ),
            (
                c[0] + dir[0] * outer - right[0] * half,
                c[1] + dir[1] * outer - right[1] * half,
            ),
            (
                c[0] + dir[0] * inner + right[0] * half,
                c[1] + dir[1] * inner + right[1] * half,
            ),
            (
                c[0] - dir[0] * inner + right[0] * half,
                c[1] - dir[1] * inner + right[1] * half,
            ),
        ];
        let points: Vec<String> = plan
            .iter()
            .map(|p| {
                let s = to_svg(p.0, p.1);
                format!("{},{}", n(s.0), n(s.1))
            })
            .collect();
        svg.push_str(&format!(
            "<polygon class=\"opening\" points=\"{}\"/>\n",
            points.join(" ")
        ));
        let label = to_svg(c[0], c[1]);
        svg.push_str(&element_label(
            label.0,
            label.1 - 0.6,
            opening.id,
            &opening.name,
        ));
    }

    svg.push_str(&legend_and_scale(width, height, true, true));
    svg.push_str("</svg>\n");
    svg
}

fn render_elevation(document: &CastleDocument, terrain: &SiteTerrain) -> String {
    let bounds = Bounds::elements(document)
        .unwrap_or_else(|| Bounds::terrain(terrain))
        .padded(10.0);
    let world_w = (bounds.max_x - bounds.min_x).max(1.0);
    let width = world_w;

    // Vertical range: from the lowest ground along the section to the highest
    // element top, with a little headroom.
    let section_z = 0.0f64;
    let mut min_y = 0.0f64;
    let mut max_y = 1.0f64;
    let samples = 120;
    for i in 0..=samples {
        let x = bounds.min_x + world_w * i as f64 / samples as f64;
        let h = height_at(terrain, x, section_z);
        min_y = min_y.min(h);
    }
    for element in document.elements() {
        match element {
            Element::BatteredWall(wall) => {
                let ground = height_at(
                    terrain,
                    wall.start_mm[0] as f64 / 1000.0,
                    wall.start_mm[1] as f64 / 1000.0,
                );
                max_y = max_y.max(ground + wall.height_mm as f64 / 1000.0);
            }
            Element::RoundTower(tower) => {
                let ground = height_at(
                    terrain,
                    tower.centre_mm[0] as f64 / 1000.0,
                    tower.centre_mm[1] as f64 / 1000.0,
                );
                max_y = max_y.max(ground + tower.height_mm as f64 / 1000.0);
            }
        }
    }
    let world_h = (max_y - min_y).max(5.0);
    let height = world_h + 20.0;
    let to_svg = move |x: f64, y: f64| -> (f64, f64) { (x - bounds.min_x, (max_y - y) + 10.0) };

    let mut svg = svg_header(width, height);
    svg.push_str(STYLE);
    svg.push_str(
        "<rect class=\"paper\" x=\"0\" y=\"0\" width=\"100%\" height=\"100%\" fill=\"#f6f1e6\"/>\n",
    );

    // Horizontal elevation references double as contour lines in this view.
    let mut level = (min_y / 2.5).floor() * 2.5;
    while level <= max_y {
        let a = to_svg(bounds.min_x, level);
        let b = to_svg(bounds.max_x, level);
        svg.push_str(&format!(
            "<path class=\"contour\" d=\"M {} {} L {} {}\"/>\n",
            n(a.0),
            n(a.1),
            n(b.0),
            n(b.1)
        ));
        level += 2.5;
    }

    // Terrain section profile.
    let mut profile = String::new();
    for i in 0..=samples {
        let x = bounds.min_x + world_w * i as f64 / samples as f64;
        let h = height_at(terrain, x, section_z);
        let p = to_svg(x, h);
        if i == 0 {
            profile.push_str(&format!("M {} {}", n(p.0), n(p.1)));
        } else {
            profile.push_str(&format!(" L {} {}", n(p.0), n(p.1)));
        }
    }
    let floor = to_svg(bounds.min_x, min_y - 2.0);
    svg.push_str(&format!(
        "<path class=\"terrain-profile\" d=\"{profile} L {} {} Z\"/>\n",
        n(to_svg(bounds.max_x, min_y - 2.0).0),
        n(floor.1)
    ));

    for element in document.elements() {
        match element {
            Element::BatteredWall(wall) => {
                let x0 = wall.start_mm[0] as f64 / 1000.0;
                let x1 = wall.end_mm[0] as f64 / 1000.0;
                let h0 = height_at(terrain, x0, wall.start_mm[1] as f64 / 1000.0);
                let h1 = height_at(terrain, x1, wall.end_mm[1] as f64 / 1000.0);
                let top = h0.max(h1) + wall.height_mm as f64 / 1000.0;
                // Keep each ground height paired with its own x, even when the
                // wall runs right-to-left.
                let (left_x, left_h, right_x, right_h) = if x0 <= x1 {
                    (x0, h0, x1, h1)
                } else {
                    (x1, h1, x0, h0)
                };
                let p0 = to_svg(left_x, left_h);
                let p1 = to_svg(right_x, right_h.min(top));
                let p2 = to_svg(right_x, top);
                let p3 = to_svg(left_x, top);
                svg.push_str(&format!(
                    "<polygon class=\"wall\" points=\"{},{} {},{} {},{} {},{}\"/>\n",
                    n(p0.0),
                    n(p0.1),
                    n(p1.0),
                    n(p1.1),
                    n(p2.0),
                    n(p2.1),
                    n(p3.0),
                    n(p3.1)
                ));
                let label = to_svg((x0 + x1) * 0.5, top + 0.8);
                svg.push_str(&element_label(label.0, label.1, wall.id, &wall.name));
            }
            Element::RoundTower(tower) => {
                let cx = tower.centre_mm[0] as f64 / 1000.0;
                let cz = tower.centre_mm[1] as f64 / 1000.0;
                let ground = height_at(terrain, cx, cz);
                let top = ground + tower.height_mm as f64 / 1000.0;
                let r = tower.radius_mm as f64 / 1000.0;
                let p0 = to_svg(cx - r, ground);
                let p1 = to_svg(cx + r, ground);
                let p2 = to_svg(cx + r, top);
                let p3 = to_svg(cx - r, top);
                svg.push_str(&format!(
                    "<polygon class=\"tower\" points=\"{},{} {},{} {},{} {},{}\"/>\n",
                    n(p0.0),
                    n(p0.1),
                    n(p1.0),
                    n(p1.1),
                    n(p2.0),
                    n(p2.1),
                    n(p3.0),
                    n(p3.1)
                ));
                let label = to_svg(cx, top + 0.8);
                svg.push_str(&element_label(label.0, label.1, tower.id, &tower.name));
            }
        }
    }

    // Openings as small framed voids on the elevation.
    for opening in document.openings() {
        let Some(Element::BatteredWall(wall)) = document.get(opening.wall_id) else {
            continue;
        };
        let point = opening.plan_point_mm(wall);
        let ground = height_at(terrain, point[0] as f64 / 1000.0, point[1] as f64 / 1000.0);
        let sill = ground + opening.sill_height_mm as f64 / 1000.0;
        let lintel = sill + opening.height_mm as f64 / 1000.0;
        let x = point[0] as f64 / 1000.0;
        let half = opening.outer_slit_mm as f64 * 0.5 / 1000.0;
        let p0 = to_svg(x - half, sill);
        let p1 = to_svg(x + half, lintel);
        svg.push_str(&format!(
            "<rect class=\"opening\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>\n",
            n(p0.0),
            n(p0.1.min(p1.1)),
            n((p1.0 - p0.0).max(0.05)),
            n((p1.1 - p0.1).abs().max(0.05))
        ));
        let label = to_svg(x, lintel + 0.6);
        svg.push_str(&element_label(label.0, label.1, opening.id, &opening.name));
    }

    svg.push_str(&legend_and_scale(width, height, false, false));
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{m0a_showcase_document, m0b_showcase_document};

    #[test]
    fn top_plan_is_valid_svg_with_labels_and_contours() {
        let terrain = SiteTerrain::m0a_promontory();
        let document = m0b_showcase_document();
        let svg = render_plan_svg(&document, &terrain, PlanView::TopPlan);
        assert!(svg.starts_with("<svg "));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("class=\"contour\""));
        assert!(svg.contains("class=\"element-label\""));
        assert!(svg.contains("#1"));
        assert!(svg.contains("North curtain"));
        assert!(svg.contains("class=\"scale-bar\""));
        assert!(svg.contains("class=\"north-arrow\""));
        assert!(svg.contains("class=\"opening\""));
        assert!(svg.contains("<style>"));
        assert!(svg.contains(".contour{"));
        // The view is cropped to the castle + 20 m, not the full 240 m terrain.
        assert!(svg.contains("viewBox=\"0 0 "));
        let header = svg.lines().next().unwrap();
        let width: f64 = header
            .split("width=\"")
            .nth(1)
            .unwrap()
            .split("mm\"")
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(width < 100.0, "plan viewBox was not cropped: {width} m");
    }

    #[test]
    fn elevation_is_valid_svg_with_labels_and_contours() {
        let terrain = SiteTerrain::m0a_promontory();
        let document = m0a_showcase_document();
        let svg = render_plan_svg(&document, &terrain, PlanView::ElevationNorth);
        assert!(svg.starts_with("<svg "));
        assert!(svg.trim_end().ends_with("</svg>"));
        assert!(svg.contains("class=\"contour\""));
        assert!(svg.contains("class=\"terrain-profile\""));
        assert!(svg.contains("class=\"element-label\""));
        assert!(svg.contains("#2"));
        assert!(svg.contains("Drum tower"));
        assert!(
            !svg.contains("class=\"north-arrow\""),
            "elevation should omit the north arrow"
        );
        assert!(svg.contains("<style>"));
    }

    #[test]
    fn rendering_is_deterministic() {
        let terrain = SiteTerrain::m0a_promontory();
        let document = m0b_showcase_document();
        let a = render_plan_svg(&document, &terrain, PlanView::TopPlan);
        let b = render_plan_svg(&document, &terrain, PlanView::TopPlan);
        assert_eq!(a, b);
    }
}
