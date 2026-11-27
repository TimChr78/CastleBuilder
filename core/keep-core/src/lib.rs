//! Engine-agnostic core for Keepwright.
//!
//! Everything in this crate is deterministic and free of Godot dependencies so
//! that the document, terrain, masonry layout and mesh generation can be
//! iterated with `cargo test` alone. Coordinates crossing the public boundary
//! are integer millimetres (`i64`) unless a type explicitly says otherwise.
//!
//! Canonical ordering is established with [`std::collections::BTreeMap`] and
//! sorted ids throughout.

pub mod command;
pub mod document;
pub mod fixtures;
pub mod historian;
pub mod junction;
pub mod masonry;
pub mod mesh;
pub mod picking;
pub mod plan_render;
pub mod proposal;
pub mod save;
pub mod session;
pub mod style;
pub mod terrain;
pub mod validate;

/// Version stamped into layout hashes. Bump whenever the generator's output
/// changes, even if the document schema is unchanged.
pub const GENERATOR_VERSION: u32 = 2;

/// FNV-1a 64-bit offset basis.
pub const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Continue an FNV-1a 64-bit hash over another byte slice. Using this instead of
/// concatenating everything into one buffer keeps large layout hashes
/// allocation-free.
pub fn fnv1a_64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// FNV-1a 64-bit hash. Deliberately zero-dependency so that layout hashes can
/// be recomputed anywhere without relying on a hasher implementation whose
/// output could change between Rust releases.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    fnv1a_64_update(FNV_OFFSET_BASIS, bytes)
}

/// Stable scalar hash used for deterministic stone-level variation (tint,
/// jitter, block lengths). Mixes a few integers and returns `[0, 1)` without
/// allocating.
pub fn hash01(seed: u64, a: i64, b: i64, c: i64) -> f32 {
    let mut bytes = [0u8; 32];
    bytes[0..8].copy_from_slice(&seed.to_le_bytes());
    bytes[8..16].copy_from_slice(&a.to_le_bytes());
    bytes[16..24].copy_from_slice(&b.to_le_bytes());
    bytes[24..32].copy_from_slice(&c.to_le_bytes());
    let hash = fnv1a_64(&bytes);
    // Top 24 bits give a stable value in [0, 1) with ample mantissa precision.
    ((hash >> 40) as f32) / ((1u32 << 24) as f32)
}
