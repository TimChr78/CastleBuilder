# Keepwright — Agent Instructions

Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) before making architectural decisions.

## Core Architecture Rules

1. **Castle Document is the single source of truth.** Never store authoritative castle state in Godot scene nodes.
2. **Engine-agnostic core (`core/keep-core`).** No Godot dependencies in `keep-core`. Layout, terrain authority, massing, masonry, validation, picking, and plan rendering live in Rust so 90% of iteration runs via `cargo test`.
3. **Thin GDExtension binding (`core/keep-gd`).** Only commands in, mesh buffers / terrain tiles / events out.
4. **Determinism.** Use sorted collections (`BTreeMap`, sorted IDs), quantized integer-millimetre layout inputs, and `libm` math where cross-platform determinism matters.
5. **Scenes in code.** Keep `.tscn` files minimal; build UI and dynamic scene trees in GDScript so diffs stay clean and agent-friendly.
6. **Zero-budget & licensing.** Procedural-first, CC0, or self-made assets only. Track third-party assets in a licence manifest. No AI-generated assets in shipped builds.
7. **Pinned toolchain.** Check [`tools/versions.toml`](tools/versions.toml) before upgrading Godot, `gdext`, Blender, or Rust.

## Build & Test Commands

```bash
# Rust workspace unit & integration tests
cargo test --manifest-path core/Cargo.toml

# Build GDExtension shared library (debug / release)
cargo build --manifest-path core/Cargo.toml -p keep-gd
cargo build --manifest-path core/Cargo.toml -p keep-gd --release

# Check formatting and lints
cargo fmt --manifest-path core/Cargo.toml --check
cargo clippy --manifest-path core/Cargo.toml --all-targets

# Godot headless verification
godot --headless --path game --quit
godot --headless --path game --script res://tests/run.gd
```

## Development Workflow

- Use well-scoped, testable tasks.
- Always run `cargo test` and `cargo clippy` after Rust changes, and verify `keep-gd` loads cleanly in `godot --headless --path game --quit`.
- Do not modify `docs/ARCHITECTURE.md` without explicit approval; record new architectural decisions in `docs/adr/`.
