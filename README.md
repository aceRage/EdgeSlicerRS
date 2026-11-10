# EdgeSlicerRS

A from-scratch Rust slicing engine for EdgeSlicer, built for speed, with a
Tauri + web desktop UI to follow. The C++ EdgeSlicer stays the shipping
product and the reference this engine is checked against; nothing here
replaces it until output matches on the printers we care about.

## Layout

| Path | What it is |
| --- | --- |
| `crates/edge-geom` | Fixed-point 2D geometry (nanometre `i64`, same scale as libslic3r) |
| `crates/edge-mesh` | Indexed triangle mesh, transforms, STL read/write |
| `crates/edge-config` | Vendor presets resolved through `inherits`, `include` and `renamed_from` |
| `crates/edge-3mf` | 3MF projects: objects, parts and their roles, instances, plates, project settings |
| `crates/edge-slice` | Mesh to per-layer contours, parallel across layers |
| `crates/edge-cli` | The `edge` command line |
| `reference/EdgeSlicer` | Pinned submodule of the C++ slicer: profiles, test models, behaviour to match. Read-only here |
| `apps/desktop` | Tauri app (not started) |

## Build and run

```bash
git submodule update --init
cargo test --workspace
export EDGE_PROFILES=reference/EdgeSlicer/resources/profiles
cargo run --release -p edge-cli -- inspect reference/EdgeSlicer/tests/mcbrim_test.3mf
cargo run --release -p edge-cli -- slice reference/EdgeSlicer/tests/mcbrim_test.3mf --plate 2 --svg layer.svg
cargo run --release -p edge-cli -- preset Snapmaker process "0.20mm Standard @Snapmaker U1 (0.4 nozzle)"
```

## Status

Working:

- STL and 3MF in, closed oriented contours out (outer boundaries
  counter-clockwise, holes clockwise), deterministic across thread counts.
  Every 3MF in the reference repository loads and slices with no open contours.
- Every preset the reference ships resolves. `slice` takes layer heights from
  the command line, a process preset, or the 3MF's own settings, in that order.

Known gaps in what is read: painted triangles, per-object setting overrides,
plate origins (a plate's contours are in project coordinates, not relative to
its bed), and typed settings (values are text until a stage needs them).
Overlapping parts are not united yet.

Next, in pipeline order: polygon booleans and offsetting, perimeters, infill,
supports, G-code export, then a parity harness that slices the same project
with both engines and diffs the result.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
