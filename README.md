# EdgeSlicerRS

A from-scratch Rust slicing engine for EdgeSlicer, built for speed, with the
start of a Tauri desktop app. The C++ EdgeSlicer stays the shipping
product and the reference this engine is checked against; nothing here
replaces it until output matches on the printers we care about.

## Layout

| Path | What it is |
| --- | --- |
| `crates/edge-geom` | Fixed-point 2D geometry (nanometre `i64`, same scale as libslic3r): points, polygons, regions, convex hull |
| `crates/edge-mesh` | Indexed triangle mesh, transforms, STL read/write |
| `crates/edge-config` | Vendor presets resolved through `inherits`, `include` and `renamed_from`; the reference's defaults for every setting, and the text form it writes settings in |
| `crates/edge-3mf` | 3MF projects: objects, parts and their roles, instances, plates, project settings |
| `crates/edge-clip` | Polygon union, difference, intersection, offset and gap closing (wraps clipper2-rs) |
| `crates/edge-extrusion` | Extruded line cross-section (flow) and extrusion loops |
| `crates/edge-parity` | `edge-parity`: runs the reference slicer and this engine on one project and compares time and G-code |
| `crates/edge-perimeter` | Walls, the lines that fill the gaps between them, and the infill area inside (classic constant-width generator) |
| `crates/edge-medial` | The centre line of a narrow region and how wide it is along it (wraps boostvoronoi), and lines of changing width as extrusions |
| `crates/edge-adhesion` | Skirt and brim, with the automatic brim width |
| `crates/edge-support` | Supports under overhangs, normal (grid) and tree, with dense tops and feet |
| `crates/edge-gcode` | Print order, seams, travel, retraction, wipe and lift, cooling, overhangs, and extrusion, written as G-code |
| `crates/edge-infill` | Top, bottom, bridge, solid, bridged-over-infill and sparse surfaces, and the lines that fill them |
| `crates/edge-template` | The placeholder language of custom G-code: `{expression}`, `{if}`, `[variable]`, and scripts |
| `crates/edge-time` | How long a G-code file takes, with acceleration: the file is read as a printer reads it and planned as the reference's estimator plans, or as Klipper does |
| `crates/edge-tower` | The prime tower a nozzle is primed on after a tool change |
| `crates/edge-slice` | Mesh to per-layer contours, then to regions with holes; parallel across layers |
| `crates/edge-engine` | The whole pipeline behind one call, for whatever drives it: open a model, settle its settings, slice with progress and cancel, G-code, a layer's lines for drawing, and a vendor's presets by printer |
| `crates/edge-cli` | The `edge` command line |
| `tools/make_defaults.py` | Asks the reference for its default of every setting and writes the table `edge-config` carries; run again when the pin moves |
| `tools/make_fixture.py` | Writes small test 3MFs (parts that need supports or a brim, a plain cube, two blocks in two filaments) to compare with the reference on |
| `tools/make_project.py` | Writes one of those parts as a project whose settings are named presets, with as many filaments as are named: how the test prints for a printer are made |
| `reference/EdgeSlicer` | Pinned submodule of the C++ slicer: profiles, test models, behaviour to match. Read-only here |
| `apps/desktop` | The desktop app (Tauri): open a model, slice it, look through its layers, export G-code. A workspace of its own |

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

## The desktop app

```bash
cd apps/desktop
cargo run                      # the window
cargo run -- model.3mf         # the window, with a model open
cargo run -- --serve 4710      # no window: the same page at http://127.0.0.1:4710/
cargo test
```

A first slice of the app, not the app: it opens an STL or a 3MF project,
slices it with the project's own settings or with presets picked from the
vendor profiles (a printer, a process, a filament for each filament the
model prints in, starting from the ones the printer's own profile
names), and optionally a bed surface. It shows each layer coloured by
feature or by filament (wheel to zoom, drag to move, arrow keys or the
slider for layers) and saves the G-code, which is the file the command
line writes. What it has to say about a print is kept in three places:
what is wrong with this print, in red; what was settled without being
asked, such as the bed surface; and, folded away, the settings the
profile turns on that this engine does not follow yet. It has
no 3D view, no arranging or editing of the model, no settings editor and
no installer. The vendor profiles are found in `reference/EdgeSlicer`
beside the source, or wherever `EDGE_PROFILES` points; the folder can be
changed in the window.

`apps/desktop` is a Cargo workspace of its own, so `cargo test --workspace`
at the top builds no web view. The page (`ui/`) is plain HTML, CSS and
JavaScript with no build step, and asks the engine for everything through
one function (`src/api.rs`), which does not know about Tauri. `--serve`
puts the same page and the same function behind a local web server, for
working on the page in a browser and for checking it where a window
cannot be seen; it listens on the loopback address only and takes calls
only from the page it served. `EDGE_DESKTOP_TRACE=1` notes each call the
page makes on standard error.

Checked so far, on the Mac mini: in a browser through `--serve`, a
four-colour plate and a bare STL with presets were sliced, looked through
and exported, and the exported file is byte for byte the command line's
but for the time in its first line. The owner then ran the window by hand on the same day: its file dialogs,
slicing and the layer view worked. A model printed in two filaments
and sliced there with one filament preset had no G-code, which is since
put right (the preset is used for every filament). Not built on Windows
or Linux.

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
- Gap fill: a gap between walls too narrow for another wall gets a line
  down its middle, as wide as the gap is at each point, found as the
  reference finds it (the medial axis of the gap). On whenever
  `gap_infill_speed` is above zero.

Known gaps: painted triangles, per-object setting overrides, typed settings
(values are text until a stage needs them), and modifier parts.
- Infill: each layer's inside is split into top, bottom, bridge, solid
  shell and sparse areas (`top_shell_layers`, `bottom_shell_layers`,
  `sparse_infill_density`, `minimum_sparse_infill_area`) and filled with
  lines, or a grid for sparse areas. Solid lines run to the edge of their
  area and, where one is joined to the next, turn half a line spacing
  inside it, as the reference's do. `top_surface_pattern`,
  `bottom_surface_pattern` and `internal_solid_infill_pattern` say whether
  lines are joined (`monotonic`, `rectilinear`) or left separate
  (`monotonicline`); any other pattern is laid as joined lines, with a
  note. Top and bottom surfaces are fitted to their area: the lines are
  spread, by up to a fifth, so that a whole number of them fills the
  width, and each is made as wide as its share. They overlap their walls
  by `top_bottom_infill_wall_overlap` on the first layer and the last.
  Sparse fill is laid as the reference lays it. Its pattern passes
  through the middle of what is printed, so it sits the same way in a
  part wherever the part is on the bed. A grid's lines are cut half a
  spacing inside the area, none within a spacing of the area's own edge,
  joined along the wall where two ends are closer than
  `infill_anchor_max`, and an end joined to nothing runs `infill_anchor`
  along the wall to hold on to it, stopping half a line short of the
  next end; nothing is joined or anchored along a wall a line already
  runs beside (FillBase.cpp `connect_infill`). Lines in one direction
  (`rectilinear`, `zig-zag`, `line`) are one zigzag that turns half a
  spacing inside the wall. The first solid layer over sparse
  fill is laid as an internal bridge: round threads across the lines
  beneath, at `internal_bridge_speed`. Fill over air takes `bridge_flow`
  and `bridge_density`, and runs along the edges it has nothing beside,
  so its lines go from one support to the other. Bridges, bottoms and
  tops reach a wall's spacing into the fill beside them, to have
  something to hold on to.

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
  - Overhangs: every wall is measured against the outline of the layer
    beneath. A stretch that hangs clear of it is an overhang wall, printed
    as a bridge (`detect_overhang_wall`); one partly supported slows
    between the four overhang speeds by how much of its width has
    something under it (`enable_overhang_speed`); and bridges, overhang
    walls and stretches past `overhang_fan_threshold` get
    `overhang_fan_speed` where that is more than the layer's fan.
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
    was pulled when last put away. The tool being put away goes to the
    prime tower first, so it is over the tower and not over the part that
    it waits while the next comes up to temperature. The next tool starts
    warming `preheat_time` ahead by the file's own timing, which for the
    first change is during the start G-code as in the reference, but
    never before the last thing said of its temperature; and a tool not
    needed again is switched off.
  - Filament presets are named one for each filament (`--filament`
    given again for filament 2, and so on). One alone is used for every
    filament, and a filament past the last named takes the last; a note
    says so either way. A model printed in a
    filament its settings say nothing of gets no G-code and a sentence
    saying which. Printer scripts are given a bed temperature for every
    filament.
  - A prime tower, on every layer up to the last tool change: a wall, a
    block for each change where the new nozzle lays down `prime_volume`,
    lines across the depth no change uses, a solid first layer and a brim.
    The skirt goes round it and no part's brim is laid on its ground. A
    tower set to a ribbed wall is made as wide as it is deep, as the
    reference makes it.
- Printer profiles: the machine's start, end and layer-change G-code and the
  filament's start and end G-code are expanded from their templates, with
  filament-level retraction overrides applied. For the Snapmaker U1 projects
  in the reference, the expanded start G-code is identical to the
  reference's on every line. A bare model exports with a printer's own
  G-code when `--machine`, `--process` and `--filament` name its presets.
- File layout: the G-code file is laid out as the reference lays its own
  out, because printers and their hosts read it by that layout: a header
  block, the line widths, the G-code between `EXECUTABLE_BLOCK` markers
  with progress (`M73`) at each layer, what each filament used, every
  setting in a `CONFIG_BLOCK`, and totals with the time. The first line
  names OrcaSlicer and the reference's version, as the reference's does:
  the U1's host accepts files only from slicers on a fixed list. A setting
  the presets leave out takes the reference's default, both in what is
  printed and in the settings block, so a model sliced with named presets
  comes out as the same model in a project would. A print host's address and keys are left
  out of the settings. `SOURCE_DATE_EPOCH`
  fixes the date in the first line, for comparing two runs byte for byte.

**On hardware so far:** a deck across a 30 mm gap printed on an Elegoo
Centauri Carbon on 2026-11-10 and the bridge held; the wall across the
gap came out a little pulled in, having been printed five times faster
than the profile asks, which is since put right. A 20 mm cube
printed on a Snapmaker U1 the same day. One was started on the Centauri
Carbon too; how it came out is not recorded here. A two-colour
test (two blocks side by side, 20 tool changes, a prime tower) printed on
the U1 that day too: the first print in more than one filament. Supports
have not been on hardware at all. Without a printer
profile a generic preamble is written and the file is only good for viewing.

Multi-filament printing is a first working version. Where parts of
different filaments overlap, the higher-numbered filament has the overlap
(the reference gives it to the later part). The order of tools within a
layer after the first is simpler than the reference's. The tower's wall
is a plain box: no ribs or cone, no gap where the nozzle comes in, and no
ramming. A filament with no tool of its own (the fifth on a
four-tool printer) is written as the reference writes it, with a warning.
Painted colours and filaments mixed from others are not read.

Not done in G-code: supports only where painted (none are
generated, and a note says so), the older tree styles (`tree_slim`,
`tree_strong`, `tree_hybrid`: organic-style trees are built, with a note),
rafts, brims inside holes, draft shields, seams placed by visibility or kept
off overhangs, a fan speed of its own for bridges over infill, walls
slowed over curled edges, arc fitting, preview pictures in the file
(thumbnails), and object labels for cancelling (`EXCLUDE_OBJECT`). Bridge fill keeps its own speed along its whole
length, where the reference also slows it near its anchors. A setting that asks for one of these is listed as a
note when the file is written. A template that needs a variable not
provided stops the export with the variable's name.

- Print time: with a printer profile the finished file is read back as a
  printer reads it and every move is planned with acceleration
  (`edge-time`). That time is the one in the file, the one progress
  (`M73`) is worked out from, and the one a waiting tool is warmed by.
  For a Klipper printer the moves are planned as Klipper plans them
  (klippy/toolhead.py): corners by the square corner velocity, and a look-ahead
  that holds the peak of a short move to what `minimum_cruise_ratio`
  allows, read from the file's own `SET_VELOCITY_LIMIT`. The reference's
  estimator has the first and not the second, so this engine's figure
  for a Klipper printer is longer than the reference would give the
  same file, by 1 to 5% on the parts compared; a line at the end of the
  file says which way it was timed. For any other firmware the reference's own
  estimator is used, ported from GCodeProcessor.cpp. Heating, homing,
  bed levelling and whatever a printer's macros do are in neither. The
  time at commanded speeds is kept on a line of its own. Cooling still
  slows a layer by its time at commanded speeds, as the reference's does.
- Acceleration limits for Klipper are written with both `ACCEL_TO_DECEL`
  and `MINIMUM_CRUISE_RATIO`, as the reference writes them: current
  Klipper reads only the second.

Walls do not yet have thin-wall detection (the U1 profiles leave it off),
or extra perimeters over overhangs. A top surface below the last layer
keeps only its outer wall, as the reference's `only_one_wall_top` does with
`top_surface_expansion` at zero (PerimeterGenerator.cpp
`split_top_surfaces`); the other way of doing it, with an expansion set, is
not ported. Arachne (variable width) is not
started. Overhangs are measured as the reference measures them but probed
at fixed steps, so a wall's speed changes at slightly different places.
Infill is not all a port yet: solid lines are joined along
the walls nearest-first but not ordered (no monotonic), a bridge reaches 0.4 mm onto each support where the reference's
reaches 0.8 mm, internal bridges are not run out to the
sparse lines that carry them (they come 5 to 9% short of the
reference's), narrow parts of top and bottom surfaces get no gap fill
(`gap_fill_target`), shell thickness in mm and vertical shells are ignored, and sparse patterns other than line and
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

Next: thumbnails and painted colours.

## Comparing with the reference

```bash
cargo build --release
target/release/edge-parity reference/EdgeSlicer/tests/data/cylinder_drawcut.3mf \
  --reference /path/to/EdgeSlicer.app/Contents/MacOS/EdgeSlicer --svg overlay.svg
```

It slices the project with both, reads both G-code files, and reports time,
filament, path length by feature, and how far walls and layer extents differ
layer by layer; `--svg` draws one layer of both toolpaths over each other.
It also sets side by side how each feature is printed: speed, line width,
plastic per millimetre, acceleration and fan, on the first layer and on
the rest, with a mark where the two differ; and the print time at the
speeds written, and the temperatures set along the way. A wall printed
five times too fast shows there and nowhere in the lengths.

Last it gives the print time each file states, and times both files with
this engine's estimator, each planned both ways. The reference's own
file planned the reference's way should come to the reference's own
figure: that is the standing check on the port. On 2026-11-10 it did, to
the second, on every file sliced with the U1's presets (13m 17s, 14m 59s
against 15m 00s, 19m 42s, 24m 38s, 42m 32s), and within 1% on the
Centauri Carbon's, whose files have arcs that are cut up a little
differently here. This engine's own files, planned the same way, came to
15m 02s against 15m 00s on the cube with the U1's presets, 7m 15s
against 7m 15s on the bridge with the Centauri Carbon's and 93m 51s
against 94m 14s on `cylinder_drawcut.3mf`. Planned as Klipper plans,
with the cruise ratio of a half that both printers' profiles set, the
same files take 0.4 to 2% longer; at a ratio of three quarters, 2 to 8%.
No figure has been checked against a clock on a printer yet.

The reference binary has to be built from the commit `reference/EdgeSlicer`
is pinned to, or the comparison is with something older than the source
this engine is written against. Until 2026-11-10 the Mac mini's build
was from 28 October (`cb41c97a2e`), 948 commits behind the pin. It has
been rebuilt at the pin (`156e9591fe`, which calls itself 2.4.9.0) and
every figure below is against that build, as is the table of defaults in
`edge-config`. Three things changed with it. The reference lists 33 more
settings. It prints bridges at the bridge speed when
`slowdown_for_curled_perimeters` is on, as this engine does, where the old
build used the last overhang speed. And plate 1 of `mcbrim_test.3mf` came
out differently (8.8 m of bottom surface where the old build laid 22.9 m,
and sparse fill where it had none). Nothing else moved: a cube, a deck
across two posts, a 6 mm tower and two supported parts, each with the U1's
and the Centauri Carbon's presets, gave the same lengths of every feature
from both builds.
The reference takes about two seconds to start and exit whatever it is asked
to do, so its time is also taken from its own log.

Results, 2026-11-10, Mac mini, reference build 2.4.9.0, best of 3:

| Project | Reference work | This engine | Walls, per layer | Filament |
| --- | --- | --- | --- | --- |
| `cylinder_drawcut.3mf` (389 layers) | 0.41 s | 0.086 s | under 0.01% apart | -0.3% |
| `stabilizer_v1.3mf` (499 layers) | 0.87 s | 0.163 s | 0.85% apart | -43% (no supports) |
| `twopart_groups.3mf` (60 layers) | 0.18 s | 0.010 s | under 0.01% apart | -2.7% |

By feature, in metres of extruded path, reference against this engine:

| Project | Overhang wall | Internal bridge | Internal solid fill | Gap fill |
| --- | --- | --- | --- | --- |
| `cylinder_drawcut.3mf` | none | 11.22 / 11.30 | 61.04 / 60.62 | 0.02 / 0.00 |
| `twopart_groups.3mf` | 0.16 / 0.16 | 1.16 / 1.20 | 6.38 / 6.13 | none |
| `mushroom` fixture | 0.23 / 0.23 | 1.99 / 2.04 | | none |
| `mcbrim_test.3mf` plate 1 | 0.10 / 0.06 | | | 8.28 / 9.98 |
| `mcbrim_test.3mf` plate 2 | 0.03 / 0.03 | | | 4.95 / 5.03 |

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
the same layers (27 here, 28 in the reference), and on plate 1 the start
of the file is identical for 96 of its 99 lines; the tower here carries 24
to 30% less plastic, having no ribs. Plates 2 and 3 set the order of
filaments on the first layer (`first_layer_print_sequence`), which this
engine does not follow, so it starts with a different tool there. All three of its plates slice. It is a flat part
3 mm thick covered in lettering, and it is where this engine is furthest
from the reference: 6 to 10% less filament on plates 1 and 2. Top surface
is within 1.3% now that walls are thinned under it; what is left is the
prime tower (no ribs) and internal bridges.

Top and bottom surfaces now come to the reference's length within 0.1%,
at the same pitch and width, on the cube, the deck, the tower and the
mushroom with both printers' presets and on the cylinder (they were 2 to
26% over). A full layer of solid fill inside the part does too: 907.9 mm
against 907.0 mm on the cube. Over a whole part solid fill is 1 to 8%
under, because the reference lays one more, partly solid, layer under a
top shell (a band round the sparse fill beneath the first bridged layer)
and fills small gaps left inside solid fill; neither is done here. Solid
lines here are laid as one zigzag where the reference orders `monotonic`
lines so that each is laid beside the last.

Sparse fill comes to the reference's length within 1.5% on the cube, the
deck, the tower and the two-colour test with both printers' presets
(the U1's grid and the Centauri Carbon's rectilinear), on
`cylinder_drawcut.3mf` (663.1 m against 663.4 m) and on
`twopart_groups.3mf`; it was 8 to 60% under on small parts with the
grid, and up to 25% out either way with rectilinear. On the tower and
the deck the paths are the reference's, line for line. On the cube two
joins of equal length are taken in another order, which leaves one path
as two. Filament over the whole part is now within 0.6% on the cube,
the deck and the tower for both printers (the tower with the U1's
presets was 18% under). Where objects share a plate the pattern passes
through the middle of all of them, not of each as in the reference; and
on the lettered plate sparse fill is 6 to 8% over, where this engine
makes more of the inside sparse than the reference does.

Tree supports with the Centauri Carbon's presets (`tree(auto)`, arcs on)
come to about half the reference's length on the `shelf` fixture (2.26 m
against 4.37 m); with the settings of `twopart_groups.3mf` the same part
is within 7%.

The reference still does more in that time (arc fitting, time planning,
ironing out its fill order), so the speed gap will narrow as stages are
added; measuring overhangs took this engine from seven times faster to
five on the cylinder.
Closed since the first run: elephant-foot compensation, one wall on the
last layer, fill lines joined along the walls, seam gaps, wipe, ramped and
spiral lifts, and plate placement (outer walls land where the reference
puts them on all three plates of `mcbrim_test.3mf`), the prime tower, gap
fill between walls, overhang walls with their speeds and fan, and
internal bridges, and one wall under top surfaces below the last layer.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
