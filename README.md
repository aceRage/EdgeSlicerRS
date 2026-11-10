# EdgeSlicerRS

A from-scratch Rust slicing engine for EdgeSlicer, built for speed, with a
Tauri + web desktop UI to follow. The C++ EdgeSlicer stays the shipping
product and the reference this engine is checked against; nothing here
replaces it until output matches on the printers we care about.

## Layout

| Path | What it is |
| --- | --- |
| `crates/edge-geom` | Fixed-point 2D geometry (nanometre `i64`, same scale as libslic3r): points, polygons, regions, convex hull |
| `crates/edge-mesh` | Indexed triangle mesh, transforms, STL read/write |
| `crates/edge-config` | Vendor presets resolved through `inherits`, `include` and `renamed_from` |
| `crates/edge-3mf` | 3MF projects: objects, parts and their roles, instances, plates, project settings |
| `crates/edge-clip` | Polygon union, difference, intersection, offset and gap closing (wraps clipper2-rs) |
| `crates/edge-extrusion` | Extruded line cross-section (flow) and extrusion loops |
| `crates/edge-parity` | `edge-parity`: runs the reference slicer and this engine on one project and compares time and G-code |
| `crates/edge-perimeter` | Walls and the infill area inside them (classic constant-width generator) |
| `crates/edge-adhesion` | Skirt and brim |
| `crates/edge-gcode` | Print order, seams, travel, retraction, wipe and lift, cooling, and extrusion, written as G-code |
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
cargo run --release -p edge-cli -- slice model.stl --vendor Snapmaker --machine "Snapmaker U1 (0.4 nozzle)" \
  --process "0.20mm Standard @Snapmaker U1 (0.4 nozzle)" --filament "Snapmaker PLA @U1" --bed-type "Textured PEI Plate" --gcode model.gcode
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

Known gaps: painted triangles, per-object setting overrides, typed settings
(values are text until a stage needs them), and modifier parts.
- Infill: each layer's inside is split into top, bottom, bridge, solid
  shell and sparse areas (`top_shell_layers`, `bottom_shell_layers`,
  `sparse_infill_density`, `minimum_sparse_infill_area`) and filled with
  lines, or a grid for sparse areas.

- G-code: `slice --gcode out.gcode` orders each layer (skirt and brim,
  walls island by island, then fill nearest-end-first), holds speeds to the
  hot end's melt rate, sets acceleration by feature, and reports filament,
  weight and time. Following the reference:
  - Seams: each outer wall starts at its best corner by the reference's
    angle score and stays in line with the layer below; inner walls start
    alongside. Walls stop `seam_gap` short of their start.
  - Travel: retraction with wipe, and straight, ramped or spiral lifts
    (`z_hop_types`), at feedrates the Z axis can follow.
  - Cooling: a layer shorter than `slow_down_layer_time` is laid out again
    with its fastest lines slowed, down to `slow_down_min_speed`; the part
    fan follows layer time and the auxiliary fan is driven.
- Skirt and brim: skirt loops around everything, and an outer brim of the
  width asked for.
- Placement: each plate of a 3MF is moved to bed coordinates, and a bare
  model is centred on the bed of the machine preset. Export refuses
  toolpaths that leave the bed, and writes the file whole or not at all.
- Printer profiles: the machine's start, end and layer-change G-code and the
  filament's start and end G-code are expanded from their templates, with
  filament-level retraction overrides applied. For the Snapmaker U1 projects
  in the reference, the expanded start G-code is identical to the
  reference's on every line. A bare model exports with a printer's own
  G-code when `--machine`, `--process` and `--filament` name its presets.

**The G-code has not been run on a printer.** It is written to print on the
U1 with one filament, and it matches the reference where the two have been
compared, but nothing here has been tested on hardware. Without a printer
profile a generic preamble is written and the file is only good for viewing.

Not done in G-code: tool changes and multiple extruders (the U1's
tool-change template uses the statement form of the placeholder language,
which is refused, not skipped), the automatic brim (`brim_type` `auto_brim`,
the default: no brim is added, and a note says so), brims inside holes,
draft shields, seams placed by visibility or kept off overhangs, overhang
and bridge fan speeds, arc fitting, progress reports (`M73`) and object
labels for cancelling. A setting that asks for one of these is listed as a
note when the file is written. A template that needs a variable not
provided stops the export with the variable's name. The time is at
commanded speeds with no acceleration.

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

Next: a first print on the U1 to check the output on hardware, then tool
changes, the automatic brim, and supports.

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
| `cylinder_drawcut.3mf` (389 layers) | 0.41 s | 0.060 s | under 0.01% apart | -0.5% |
| `stabilizer_v1.3mf` (499 layers) | 0.85 s | 0.109 s | 0.85% apart | -43% (no supports) |
| `twopart_groups.3mf` (60 layers) | 0.18 s | 0.008 s | under 0.01% apart | -1.4% |

The reference does more in that time (supports, seams, overhangs, gap fill,
fan and time planning), so the speed gap will narrow as stages are added.
Closed since the first run: elephant-foot compensation, one wall on the
last layer, fill lines joined along the walls, seam gaps, wipe, ramped and
spiral lifts, and plate placement (outer walls land where the reference
puts them on all three plates of `mcbrim_test.3mf`). Still different: no
supports, automatic brim, prime tower, gap fill or multi-material regions; solid fill
over sparse is not marked as an internal bridge; top surfaces below the last
layer keep all their walls.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
