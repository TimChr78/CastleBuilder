//! Procedural site terrain.
//!
//! `SiteTerrain` is the authority for ground height and bedrock. The M0a site
//! is an analytical promontory: a broad rock plateau with a cliff falling to a
//! river shelf. Heights are sampled onto a regular integer-millimetre grid so
//! every consumer (masonry, mesh AO, picking later) sees exactly the same
//! ground, and so sampling is cheap enough to bake vertex AO at 200k blocks.

use crate::document::CASTLE_SEED;
use libm::{cos, floor, round, sin, sqrt};

/// Default spacing of the cached height grid.
pub const DEFAULT_CELL_MM: i64 = 1_000;
/// Default extent of the cached height grid (240 m square).
pub const DEFAULT_RESOLUTION: usize = 241;
/// Default south-west origin of the height grid.
pub const DEFAULT_ORIGIN_MM: i64 = -120_000;

/// Mesh data for the terrain heightfield, in metres and Godot-friendly arrays.
#[derive(Clone, Debug, Default)]
pub struct TerrainMeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// `[normalised_height, bedrock, slope, 1.0]`
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<i32>,
}

/// Deterministic analytical/procedural terrain with a cached height grid.
#[derive(Clone, Debug)]
pub struct SiteTerrain {
    seed: u64,
    origin_mm: i64,
    cell_mm: i64,
    res: usize,
    heights_mm: Vec<i64>,
    bedrock: Vec<u8>,
}

impl SiteTerrain {
    /// Build the fixed M0a promontory site.
    pub fn m0a_promontory() -> Self {
        Self::new(CASTLE_SEED)
    }

    /// Build a terrain grid for `seed` at the default extent.
    pub fn new(seed: u64) -> Self {
        Self::with_grid(seed, DEFAULT_ORIGIN_MM, DEFAULT_CELL_MM, DEFAULT_RESOLUTION)
    }

    /// Build a terrain grid with explicit dimensions (useful for tests).
    pub fn with_grid(seed: u64, origin_mm: i64, cell_mm: i64, res: usize) -> Self {
        assert!(
            res >= 2,
            "terrain grid must have at least two samples per axis"
        );
        assert!(cell_mm > 0, "terrain cell size must be positive");
        let mut heights_mm = Vec::with_capacity(res * res);
        let mut bedrock = Vec::with_capacity(res * res);
        for iz in 0..res {
            let z = origin_mm + iz as i64 * cell_mm;
            for ix in 0..res {
                let x = origin_mm + ix as i64 * cell_mm;
                let (height, rock) = analytic_sample(seed, x, z);
                heights_mm.push(height);
                bedrock.push(rock);
            }
        }
        Self {
            seed,
            origin_mm,
            cell_mm,
            res,
            heights_mm,
            bedrock,
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn origin_mm(&self) -> i64 {
        self.origin_mm
    }

    pub fn cell_mm(&self) -> i64 {
        self.cell_mm
    }

    pub fn resolution(&self) -> usize {
        self.res
    }

    /// Maximum world X/Z extent of the cached grid, in millimetres.
    pub fn extent_mm(&self) -> i64 {
        self.origin_mm + (self.res as i64 - 1) * self.cell_mm
    }

    /// Bilinear ground height in millimetres. Values outside the grid are
    /// clamped to the border, matching a heightfield with a skirt.
    pub fn height_mm(&self, x_mm: i64, z_mm: i64) -> i64 {
        let fx = (x_mm - self.origin_mm) as f64 / self.cell_mm as f64;
        let fz = (z_mm - self.origin_mm) as f64 / self.cell_mm as f64;
        let fx = fx.clamp(0.0, (self.res - 1) as f64);
        let fz = fz.clamp(0.0, (self.res - 1) as f64);
        let ix = floor(fx) as usize;
        let iz = floor(fz) as usize;
        let ix1 = (ix + 1).min(self.res - 1);
        let iz1 = (iz + 1).min(self.res - 1);
        let tx = fx - ix as f64;
        let tz = fz - iz as f64;
        let h00 = self.heights_mm[iz * self.res + ix] as f64;
        let h10 = self.heights_mm[iz * self.res + ix1] as f64;
        let h01 = self.heights_mm[iz1 * self.res + ix] as f64;
        let h11 = self.heights_mm[iz1 * self.res + ix1] as f64;
        let top = h00 + (h10 - h00) * tx;
        let bottom = h01 + (h11 - h01) * tx;
        round(top + (bottom - top) * tz) as i64
    }

    pub fn height_m(&self, x_m: f32, z_m: f32) -> f32 {
        self.height_mm((x_m as f64 * 1000.0) as i64, (z_m as f64 * 1000.0) as i64) as f32 / 1000.0
    }

    /// Bedrock mask (1 = rock, 0 = soil). Bilinear, thresholded at 0.5.
    pub fn bedrock_mask(&self, x_mm: i64, z_mm: i64) -> bool {
        let fx = (x_mm - self.origin_mm) as f64 / self.cell_mm as f64;
        let fz = (z_mm - self.origin_mm) as f64 / self.cell_mm as f64;
        let ix = floor(fx.clamp(0.0, (self.res - 1) as f64)) as usize;
        let iz = floor(fz.clamp(0.0, (self.res - 1) as f64)) as usize;
        let idx = iz * self.res + ix;
        self.bedrock[idx] >= 128
    }

    /// Terrain normal from central differences, in metres.
    pub fn normal_at(&self, x_m: f32, z_m: f32) -> [f32; 3] {
        let e_mm = 500;
        let x_mm = (x_m as f64 * 1000.0) as i64;
        let z_mm = (z_m as f64 * 1000.0) as i64;
        let dhdx = (self.height_mm(x_mm + e_mm, z_mm) - self.height_mm(x_mm - e_mm, z_mm)) as f32
            / (2.0 * e_mm as f32 / 1000.0);
        let dhdz = (self.height_mm(x_mm, z_mm + e_mm) - self.height_mm(x_mm, z_mm - e_mm)) as f32
            / (2.0 * e_mm as f32 / 1000.0);
        let n = [-dhdx, 1.0, -dhdz];
        let len = sqrt((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]) as f64) as f32;
        if len <= f32::EPSILON {
            [0.0, 1.0, 0.0]
        } else {
            [n[0] / len, n[1] / len, n[2] / len]
        }
    }

    /// Build a regular-grid heightfield mesh. One shared vertex per grid point
    /// keeps the 240 m site at a modest triangle count.
    pub fn terrain_mesh(&self) -> TerrainMeshData {
        let res = self.res;
        let mut mesh = TerrainMeshData {
            positions: Vec::with_capacity(res * res),
            normals: Vec::with_capacity(res * res),
            uvs: Vec::with_capacity(res * res),
            colors: Vec::with_capacity(res * res),
            indices: Vec::with_capacity((res - 1) * (res - 1) * 6),
        };

        let inv_extent = if self.extent_mm() > self.origin_mm {
            1.0 / (self.extent_mm() - self.origin_mm) as f32
        } else {
            0.0
        };

        for iz in 0..res {
            let z_mm = self.origin_mm + iz as i64 * self.cell_mm;
            for ix in 0..res {
                let x_mm = self.origin_mm + ix as i64 * self.cell_mm;
                let h_mm = self.heights_mm[iz * res + ix];
                let x_m = x_mm as f32 / 1000.0;
                let z_m = z_mm as f32 / 1000.0;
                let h_m = h_mm as f32 / 1000.0;
                let n = self.normal_at(x_m, z_m);
                let u = (x_mm - self.origin_mm) as f32 * inv_extent;
                let v = (z_mm - self.origin_mm) as f32 * inv_extent;
                let height_norm = ((h_m + 20.0) / 32.0).clamp(0.0, 1.0);
                let bedrock = f32::from(self.bedrock[iz * res + ix]) / 255.0;
                let slope = (1.0 - n[1]).clamp(0.0, 1.0);
                mesh.positions.push([x_m, h_m, z_m]);
                mesh.normals.push(n);
                mesh.uvs.push([u, v]);
                mesh.colors.push([height_norm, bedrock, slope, 1.0]);
            }
        }

        for iz in 0..res - 1 {
            for ix in 0..res - 1 {
                let a = (iz * res + ix) as i32;
                let b = (iz * res + ix + 1) as i32;
                let c = ((iz + 1) * res + ix + 1) as i32;
                let d = ((iz + 1) * res + ix) as i32;
                // Godot treats clockwise winding as front-facing. Viewed from
                // above, this ordering presents an up-facing front face.
                mesh.indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }

        mesh
    }
}

impl Default for SiteTerrain {
    fn default() -> Self {
        Self::m0a_promontory()
    }
}

/// Analytical promontory height sample in millimetres, plus bedrock mask.
fn analytic_sample(seed: u64, x_mm: i64, z_mm: i64) -> (i64, u8) {
    let x = x_mm as f64 / 1000.0;
    let z = z_mm as f64 / 1000.0;
    let phase = (seed % 9973) as f64 * 0.000_731;
    let phase2 = (seed % 7919) as f64 * 0.000_913;

    // Elliptical plateau: slightly longer east-west than north-south.
    let r = sqrt((x * 0.82) * (x * 0.82) + (z * 1.18) * (z * 1.18));
    let t = ((r - 25.0) / 38.0).clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t);

    let plateau = 10_500.0 - 17_000.0 * s;
    let undulation = 210.0 * sin(x * 0.21 + phase) * cos(z * 0.17 - phase2) * (1.0 - s);
    let cliff_detail = 650.0 * sin(x * 0.55 + z * 0.33 + phase) * t * (1.0 - t);
    let river_shelf = -6_200.0 * s * s;
    let h = plateau + undulation + cliff_detail + river_shelf;

    let bedrock = if s < 0.62 { 255u8 } else { 0u8 };
    (round(h) as i64, bedrock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_deterministic() {
        let a = SiteTerrain::new(7);
        let b = SiteTerrain::new(7);
        assert_eq!(a.heights_mm, b.heights_mm);
        assert_eq!(a.bedrock, b.bedrock);
    }

    #[test]
    fn plateau_is_bedrock_and_valley_is_lower() {
        let terrain = SiteTerrain::m0a_promontory();
        let centre = terrain.height_mm(0, 0);
        let cliff = terrain.height_mm(70_000, 0);
        assert!(terrain.bedrock_mask(0, 0));
        assert!(centre > cliff);
    }

    #[test]
    fn normals_are_unit_length() {
        let terrain = SiteTerrain::m0a_promontory();
        for (x, z) in [(0.0, 0.0), (20.0, 10.0), (-40.0, 30.0)] {
            let n = terrain.normal_at(x, z);
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((length - 1.0).abs() < 1.0e-4, "normal length {length}");
            assert!(n[1] > 0.0);
        }
    }

    #[test]
    fn mesh_has_valid_indices() {
        let terrain = SiteTerrain::with_grid(3, -20_000, 1_000, 41);
        let mesh = terrain.terrain_mesh();
        assert_eq!(mesh.positions.len(), 41 * 41);
        assert_eq!(mesh.normals.len(), mesh.positions.len());
        assert_eq!(mesh.uvs.len(), mesh.positions.len());
        assert_eq!(mesh.colors.len(), mesh.positions.len());
        assert!(!mesh.indices.is_empty());
        for index in mesh.indices {
            assert!(index >= 0 && (index as usize) < mesh.positions.len());
        }
    }
}
