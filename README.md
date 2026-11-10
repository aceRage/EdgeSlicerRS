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
| `crates/edge-extrusion` | Extruded line cross-section (flow) and extrusion loops |
| `crates/edge-parity` | `edge-parity`: runs the reference slicer and this engine on one project and compares time and G-code |
| `crates/edge-perimeter` | Walls and the infill area inside them (classic constant-width generator) |
| `crates/edge-gcode` | Print order, travel, retraction and extrusion, written as G-code |
| `crates/edge-infill` | Top, bottom, bridge, solid and sparse surfaces, and the lines that fill them |
| `crates/edge-template` | The placeholder language of custom G-code: `{expression}`, `{if}`, `[variable]` |
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
cargo run --release -p edge-cli -- slice reference/EdgeSlicer/resources/handy_models/3DBenchy.3mf --gcode benchy.gcode
cargo run --release -p edge-cli -- slice model.stl --vendor Snapmaker --machine "Snapmaker U1 (0.4 nozzle)" --process "0.20mm Standard @Snapmaker U1 (0.4 nozzle)"
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
- Walls: the classic constant-width generator ported from the reference,
  with widths and counts from the same settings (`wall_loops`,
  `outer_wall_line_width`, `inner_wall_line_width`, `line_width`,
  `initial_layer_line_width`, `infill_wall_overlap`, `resolution`), and the
  infill area left inside them. `slice --svg` draws each wall at its real width.

Known gaps: painted triangles, per-object setting overrides, plate origins (a
plate's regions are in project coordinates, not relative to its bed), typed
settings (values are text until a stage needs them), and modifier parts.
- Infill: each layer's inside is split into top, bottom, bridge, solid
  shell and sparse areas (`top_shell_layers`, `bottom_shell_layers`,
  `sparse_infill_density`, `minimum_sparse_infill_area`) and filled with
  lines, or a grid for sparse areas.

- G-code: `slice --gcode out.gcode` orders each layer (walls island by
  island, then fill nearest-end-first), travels with retraction and lift,
  holds speeds to the hot end's melt rate, sets fan speed by layer time and
  acceleration by feature, and reports filament, weight and time.
- Printer profiles: the machine's start, end and layer-change G-code and the
  filament's start and end G-code are expanded from their templates, with
  filament-level retraction overrides applied. For the Snapmaker U1 projects
  in the reference, the expanded start G-code is identical to the
  reference's, line for line.

**The G-code has not been run on a printer.** It is written to print on the
U1 from a 3MF's first plate with one filament, and it matches the reference
where the two have been compared, but nothing here has been tested on
hardware. Without a printer profile (a bare STL) a generic preamble is
written and the file is only good for viewing.

Not done in G-code: tool changes and multiple extruders (the U1's
tool-change template uses the statement form of the placeholder language,
which is refused, not skipped), slowing down short layers, the auxiliary
fan, overhang and bridge fan speeds, seam placement, wipe, arc fitting,
skirt and brim, progress reports and object labels. Templates of some
other printers need variables not provided yet (Bambu start G-code asks
for `outer_wall_volumetric_speed`, for one); export stops with the
variable's name. Coordinates are the project's own, which for a first plate
are the bed's; other plates and bare STLs are not moved onto the bed. The
time is at commanded speeds with no acceleration.

Walls do not yet have thin-wall detection, gap fill, overhang handling,
extra or one-wall-on-top perimeters, print ordering or seams, and Arachne
(variable width) is not started. Infill is a first working version, not a
port: fill lines are joined along the walls nearest-first but not ordered
(no monotonic) and get no anchors where they stay unjoined, solid lines
are not fitted to the region's width, bridges use the ordinary flow and
direction, external surfaces are not grown to anchor, shell thickness in mm
and vertical shells are ignored, and sparse patterns other than line and
grid fall back to grid with a note. Gap closing runs across objects, so two separate objects closer than twice
the radius merge; the reference closes within one object only.

Clipping is clipper2-rs, a port of Clipper2. The reference uses Clipper 6,
so regions agree in shape and area but not vertex for vertex.

Next: a first print on the U1 to check the output on hardware, then plate
origins, seams and wipe, and supports.

## Comparing with the reference

```bash
cargo build --release
target/release/edge-parity reference/EdgeSlicer/tests/data/cylinder_drawcut.3mf \
  --reference /path/to/EdgeSlicer.app/Contents/MacOS/EdgeSlicer --svg overlay.svg
```

It slices the project with both, reads both G-code files, and reports time,
filament, path length by feature, and how far walls and layer extents differ
layer by layer; `--svg` draws one layer of both toolpaths over each other.
The reference takes about two seconds to start and exit whatever it is asked
to do, so its time is also taken from its own log.

Results, 2026-11-09, Mac mini, reference build 2.4.7.6, best of 3:

| Project | Reference work | This engine | Walls, per layer | Filament |
| --- | --- | --- | --- | --- |
| `cylinder_drawcut.3mf` (389 layers) | 0.41 s | 0.055 s | 0.02% apart | -0.5% |
| `stabilizer_v1.3mf` (499 layers) | 0.84 s | 0.064 s | 0.53% apart | -43% (no supports) |
| `twopart_groups.3mf` (60 layers) | 0.18 s | 0.007 s | 0.12% apart | -1.4% |

The reference does more in that time (supports, seams, overhangs, gap fill,
fan and time planning), so the speed gap will narrow as stages are added.
Closed since the first run: elephant-foot compensation, one wall on the
last layer, and fill lines joined along the walls. Still different: no
supports, brim, prime tower, gap fill or multi-material regions; solid fill
over sparse is not marked as an internal bridge; top surfaces below the last
layer keep all their walls.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
