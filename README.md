# EdgeSlicerRS

A from-scratch Rust slicing engine for EdgeSlicer, built for speed, with a
Tauri + web desktop UI to follow. The C++ EdgeSlicer stays the shipping
product and the reference this engine is checked against; nothing here
replaces it until output matches on the printers we care about.

## Layout

| Path | What it is |
| --- | --- |
| `crates/edge-geom` | Fixed-point 2D geometry (nanometre `i64`, same scale as libslic3r): points, polygons, regions |
| `crates/edge-mesh` | Indexed triangle mesh, transforms, STL read/write |
| `crates/edge-config` | Vendor presets resolved through `inherits`, `include` and `renamed_from` |
| `crates/edge-3mf` | 3MF projects: objects, parts and their roles, instances, plates, project settings |
| `crates/edge-clip` | Polygon union, difference, intersection, offset and gap closing (wraps clipper2-rs) |
| `crates/edge-slice` | Mesh to per-layer contours, then to regions with holes; parallel across layers |
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

- STL and 3MF in, per-layer regions out: outer boundaries counter-clockwise
  with their holes clockwise, overlapping parts united, negative parts
  subtracted from their own object, and cracks between flush parts closed by
  `slice_closing_radius` as the reference does. Deterministic across thread
  counts. Every 3MF in the reference repository slices with no open contours.
- Every preset the reference ships resolves. `slice` takes layer heights and
  the closing radius from the command line, a process preset, or the 3MF's
  own settings, in that order.
- Offsetting, the basis for perimeters: `slice --offset -0.4` insets every
  layer and reports the time.

Known gaps: painted triangles, per-object setting overrides, plate origins (a
plate's regions are in project coordinates, not relative to its bed), typed
settings (values are text until a stage needs them), and modifier parts.
Gap closing runs across objects, so two separate objects closer than twice
the radius merge; the reference closes within one object only.

Clipping is clipper2-rs, a port of Clipper2. The reference uses Clipper 6,
so regions agree in shape and area but not vertex for vertex.

Next, in pipeline order: perimeters, infill, supports, G-code export, then a
parity harness that slices the same project with both engines and diffs the
result.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
