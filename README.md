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
| `crates/edge-adhesion` | Skirt and brim, with the automatic brim width |
| `crates/edge-support` | Supports under overhangs, normal (grid) and tree, with dense tops and feet |
| `crates/edge-gcode` | Print order, seams, travel, retraction, wipe and lift, cooling, and extrusion, written as G-code |
| `crates/edge-infill` | Top, bottom, bridge, solid and sparse surfaces, and the lines that fill them |
| `crates/edge-template` | The placeholder language of custom G-code: `{expression}`, `{if}`, `[variable]`, and scripts |
| `crates/edge-tower` | The prime tower a nozzle is primed on after a tool change |
| `crates/edge-slice` | Mesh to per-layer contours, then to regions with holes; parallel across layers |
| `crates/edge-cli` | The `edge` command line |
| `tools/make_fixture.py` | Writes small test 3MFs (parts that need supports or a brim) to compare with the reference on |
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
- Skirt and brim: skirt loops around everything, and a brim round the
  outside of each part, either of the width asked for or, with
  `brim_type` `auto_brim` (the default), as wide as the reference's rule of
  thumb says each object needs from its height, footprint, material and
  print speed.
- Supports (`enable_support`), in the reference's two styles. Both hold
  up what overhangs past `support_threshold_angle`, with the reference's
  air gap above, its gaps beside and below the part, and dense layers at
  the top and at a foot that stands on the part.
  - Normal (`normal(auto)`): a column down to the bed or to the part,
    squared off to the reference's grid, with a wider, denser first layer.
  - Tree (`tree(auto)`, which the U1 profiles use): under a dense roof,
    thin tips that thicken into branches, lean together into trunks and
    flare out at the bed, or stand on the part where the bed is out of
    reach. Branches are hollow loops, as in the reference's default
    "organic" style, and keep clear of anywhere they could not get down
    from.
- Placement: each plate of a 3MF is moved to bed coordinates, and a bare
  model is centred on the bed of the machine preset. Export refuses
  toolpaths that leave the bed, and writes the file whole or not at all.
- More than one filament, for a printer with a nozzle for each:
  - Each part's filament is read from the 3MF, and every layer is shared
    out among the filaments it is printed in. Each share gets its own
    walls and fill, so parts of different filaments that touch each have
    a wall along the join.
  - Tools are taken in the reference's order on the first layer, and after
    that each layer starts with the tool already in hand.
  - A tool change follows the reference: pull the filament well back, cool
    the tool being put away, run the printer's own tool-change G-code,
    heat and select the new tool, and push its filament back as far as it
    was pulled when last put away. The next tool starts warming half a
    minute ahead, and a tool not needed again is switched off.
  - A prime tower, on every layer up to the last tool change: a wall, a
    block for each change where the new nozzle lays down `prime_volume`,
    lines across the depth no change uses, a solid first layer and a brim.
- Printer profiles: the machine's start, end and layer-change G-code and the
  filament's start and end G-code are expanded from their templates, with
  filament-level retraction overrides applied. For the Snapmaker U1 projects
  in the reference, the expanded start G-code is identical to the
  reference's on every line. A bare model exports with a printer's own
  G-code when `--machine`, `--process` and `--filament` name its presets.

**The G-code has not been run on a printer.** It is written to print on the
U1, and it matches the reference where the two have been compared, but nothing here has been tested on hardware. Without a printer
profile a generic preamble is written and the file is only good for viewing.

Multi-filament printing is a first working version. Where parts of
different filaments overlap, the higher-numbered filament has the overlap
(the reference gives it to the later part). The order of tools within a
layer after the first is simpler than the reference's. The tower is a
plain box as wide as `prime_tower_width`: no ribbed or cone wall, no gap
where the nozzle comes in, no ramming, and it is not kept out of the way
of the skirt and brim. A filament with no tool of its own (the fifth on a
four-tool printer) is written as the reference writes it, with a warning.
Painted colours and filaments mixed from others are not read.

Not done in G-code: supports only where painted (none are
generated, and a note says so), the older tree styles (`tree_slim`,
`tree_strong`, `tree_hybrid`: organic-style trees are built, with a note),
rafts, brims inside holes, draft shields, seams placed by visibility or kept off overhangs, overhang
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

Supports are a first working version, not a port: the grid of normal
supports is always square to the bed, a support's lines are not sheltered
from retraction, bridges are supported like any overhang, and where a brim
and a support meet on the first layer neither gives way. Tree supports use
the reference's numbers (how a branch thickens, leans and flares, how far
apart tips stand) but a much simpler method: each branch is one point
moved layer by layer, where the reference follows every place a branch
could be and picks afterwards; a leaning branch is a circle on each layer,
not an oval; and branches are not smoothed.

Next: a first print on the U1 to check the output on hardware, then wall
quality (thin walls, gap fill, overhangs).

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

Supports and the automatic brim are compared on parts written by
`tools/make_fixture.py`, with normal supports turned on:

| Part | Support body | Dense layers | Brim |
| --- | --- | --- | --- |
| `mushroom` (a cap on a stem), normal | -1.6%, same extents | -3.6% | none in either |
| `shelf` (a shelf over a plate), normal | -1.5%, same extents | -5.0% | none in either |
| `mushroom`, tree | -3.5% | -5.3% | none in either |
| `shelf`, tree | -6.3%, five trunks on the plate in both | -6.3% | none in either |
| `tower` (6 x 6 x 80 mm) | | | same length, same reach (5.63 mm) |
| `bar` (200 x 10 x 30 mm, PLA) | | | same length, same reach (7.42 mm) |

Normal support lines come as close to the part as the reference's
(0.54 mm). Both styles put support on the same layers as the reference,
with the same layers of air above and below. Trees start from about as
many tips (96 against 101 under the mushroom's cap) and join up a little
more slowly on the way down.

On `mcbrim_test.3mf`, printed in four filaments, both make tool changes on
the same layers (27 here, 28 in the reference), and the start of the file
is identical for 96 of its 99 lines; the tower here carries 30% less
plastic, having no ribs.

The reference does more in that time (overhangs, gap fill,
fan and time planning), so the speed gap will narrow as stages are added.
Closed since the first run: elephant-foot compensation, one wall on the
last layer, fill lines joined along the walls, seam gaps, wipe, ramped and
spiral lifts, and plate placement (outer walls land where the reference
puts them on all three plates of `mcbrim_test.3mf`). Still different: no
prime tower, gap fill or multi-material regions; solid fill
over sparse is not marked as an internal bridge; top surfaces below the last
layer keep all their walls.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
