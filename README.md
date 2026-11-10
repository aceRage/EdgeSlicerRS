# EdgeSlicerRS

A from-scratch Rust slicing engine for EdgeSlicer, built for speed, with a
Tauri + web desktop UI to follow. The C++ EdgeSlicer stays the shipping
product and the reference this engine is checked against; nothing here
replaces it until output matches on the printers we care about.

## Layout

| Path | What it is |
| --- | --- |
| `crates/edge-geom` | Fixed-point 2D geometry (nanometre `i64`, same scale as libslic3r) |
| `crates/edge-mesh` | Indexed triangle mesh, STL read/write |
| `crates/edge-slice` | Mesh to per-layer contours, parallel across layers |
| `crates/edge-cli` | The `edge` command line |
| `reference/EdgeSlicer` | Pinned submodule of the C++ slicer: profiles, test models, behaviour to match. Read-only here |
| `apps/desktop` | Tauri app (not started) |

## Build and run

```bash
git submodule update --init
cargo test --workspace
cargo run --release -p edge-cli -- slice reference/EdgeSlicer/resources/handy_models/OrcaToleranceTest.stl --svg layer.svg
```

## Status

Working: STL in, closed oriented contours out (outer boundaries
counter-clockwise, holes clockwise), deterministic across thread counts.

Next, in pipeline order: 3MF and profile loading from
`reference/EdgeSlicer/resources/profiles`, polygon booleans and offsetting,
perimeters, infill, supports, G-code export, then a parity harness that
slices the same project with both engines and diffs the result.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
