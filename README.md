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
| `crates/edge-3mf` | 3MF projects: objects, parts and their roles, instances, plates, project settings, and colours painted onto triangles |
| `crates/edge-fuzzy` | Fuzzy skin: walls roughened by moving their points or changing their width, with the noise it is drawn from (a port of the reference's FuzzySkin and of the libnoise modules it uses) |
| `crates/edge-avoid` | Travel routed round the inside of a part's walls where a straight move would cross them: what a layer's moves are routed by, and the way round for each (a port of the reference's AvoidCrossingPerimeters) |
| `crates/edge-arachne` | Walls of changing width: a region's skeleton, how many lines fit along it and where that number changes, and the lines themselves (a port of the reference's Arachne) |
| `crates/edge-paint` | Which filament prints what where colours are painted on: each layer shared out from its painted outline, and paint on top and bottom faces carried through the shell |
| `crates/edge-clip` | Polygon union, difference, intersection, offset, an inner offset with a distance of its own for every vertex, and gap closing (wraps clipper2-rs) |
| `crates/edge-extrusion` | Extruded line cross-section (flow) and extrusion loops |
| `crates/edge-parity` | `edge-parity`: runs the reference slicer and this engine on one project and compares time and G-code |
| `crates/edge-perimeter` | Walls, the lines that fill the gaps between them, and the infill area inside (classic constant-width generator) |
| `crates/edge-medial` | The centre line of a narrow region and how wide it is along it (wraps boostvoronoi), what part of the plane lies nearest each of a set of edges, the whole Voronoi diagram of a set of edges as plain data, and lines of changing width as extrusions |
| `crates/edge-adhesion` | Skirt and brim, with the automatic brim width |
| `crates/edge-support` | Supports under overhangs, normal (grid) and tree, with dense tops and feet |
| `crates/edge-gcode` | Print order, seams, travel, retraction, wipe and lift, cooling, overhangs, and extrusion, written as G-code |
| `crates/edge-infill` | Top, bottom, bridge, solid, bridged-over-infill and sparse surfaces, and the lines that fill them; and ironing: which areas are gone over again, and the lines that do it |
| `crates/edge-template` | The placeholder language of custom G-code: `{expression}`, `{if}`, `[variable]`, and scripts |
| `crates/edge-time` | How long a G-code file takes, with acceleration: the file is read as a printer reads it and planned as the reference's estimator plans, or as Klipper does |
| `crates/edge-tower` | The prime tower a nozzle is primed on after a tool change |
| `crates/edge-slice` | Mesh to per-layer contours, then to regions with holes; parallel across layers. Elephant foot compensation of the first layers; round holes made polygons of few sides, and round outlines and holes resized by what the filament prints them off by; the last layers' heights tuned so a part comes out its exact height, and layers grown downward under overhangs so they can be printed |
| `crates/edge-engine` | The whole pipeline behind one call, for whatever drives it: open a model and list its plates, settle its settings, slice with progress and cancel, G-code, a layer's lines for drawing, a vendor's presets by printer, and its printer models with the presets, picture and sizes of each |
| `crates/edge-cli` | The `edge` command line |
| `tools/make_defaults.py` | Asks the reference for its default of every setting and writes the table `edge-config` carries; run again when the pin moves |
| `tools/make_fixture.py` | Writes small test 3MFs (parts that need supports or a brim, a plain cube, two blocks in two filaments, a comb with thin teeth on the bed, three tiers with a top surface on each, bars over a base for small bridges over infill, a slab with round holes for the hole options, a slab with pockets underneath, three round ones for arc fitting: a post, a post with a hole, a slab with a wide curve and tight ones, and a post that widens as it rises for scarf seams) to compare with the reference on |
| `tools/make_project.py` | Writes one of those parts as a project whose settings are named presets, with as many filaments as are named: how the test prints for a printer are made |
| `reference/EdgeSlicer` | Pinned submodule of the C++ slicer: profiles, test models, behaviour to match. Read-only here |
| `apps/desktop` | The desktop app (Tauri): several projects open at once, each prepared, sliced, looked through layer by layer and exported, in the C++ application's tabs. A workspace of its own |

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

A first pass at the C++ application's main window, not yet the
application: `apps/desktop/UI_PARITY.md` lists, for each of its tabs and
for the project strip, what the C++ offers, what is here, and what is
not yet.
How the page looks is the app's own (dark first, spare); the tokens and
the controls it is built from are described in `apps/desktop/ui/STYLE.md`.

- **The project strip**, at the far left: several projects open at once
  (eight at most), each with its own file, plate, presets, slices and
  Preview. New, open, switch (a click, Ctrl+PgDn / Ctrl+PgUp), drag to
  reorder, close; a dot on a project whose choices would be lost, a tick
  on one that is sliced. A slice goes on while another project is looked
  at or sliced.
- **Prepare**: the printer, picked as in the C++ application from the
  printers added (one row for each model, its nozzles under "Diameter"),
  or the project's own; a filament preset for each filament the file
  prints in and the process, which follow a change of printer as the
  C++ application's do; the bed type the printer takes, remembered for
  each printer; layer height,
  initial layer height, wall loops and the gap closing radius set over
  the preset; the scene in 3D: the printer's bed (the vendor's model
  and picture of it where its profile has them), every plate of the
  project where the C++ application stands it, each object in its
  filaments' colours, painted colours included; a click selects an
  object or a plate. An STL or a 3MF opens as a project.
- **Preview**: going there slices the plate. The print in 3D, read back
  from its G-code as the C++ application's is: every line as wide and
  as high as it is laid, by line type, filament, speed, layer height,
  line width, flow, fan speed or temperature, with the legend to hide
  and show features, travels, wipes, retractions, seams and filament
  changes; a slider with two thumbs for the layers shown and one along
  the bottom for how far the print has got, with the nozzle and where
  it is; what the print comes to; and, kept apart,
  what is wrong with this print, what was settled without being asked,
  and the settings the profile turns on that this engine does not follow
  yet. "Slice plate" or "Slice all", and "Export G-code file", which
  writes the file the command line writes.
- **Device**: the printer of the project shown, as its preset describes
  it, with its address when the preset has one. Nothing is sent to a
  printer or read from one yet; the inventory says what that takes.

- **Printer Selection** ("Select/Remove printers (system presets)" in
  the printer list, or the gear beside "Printer"): every printer model
  of every vendor in one table, with its picture, bed and toolheads, to
  search, filter, sort and tick. It opens by itself while no printer is
  added. Removing a printer moves the open projects that were on it to
  one that is still added, and says so.

The two views share a camera, the C++ application's: the left button
turns the view, the right or the middle one moves it, the wheel zooms at
the pointer; Ctrl+0 to Ctrl+6 and the cube at the bottom left are its
views, Z fits, and the "View" menu has perspective and orthographic.
They are drawn on WebGL2 by the page's own code (`ui/gl`, no library);
a browser without WebGL2 gets the plate and one layer at a time from
above instead. Meshes and moves come from the session as bytes, each
object once and a print a stretch at a time (`src/api/scene.rs` has the
layouts).

It has no arranging or editing of the model, no settings
editor beyond those four, no saving of projects and no installer. The
vendor profiles are found in `reference/EdgeSlicer` beside the source, or
wherever `EDGE_PROFILES` points; the folder can be changed in the window,
and is then kept.

What the app keeps between runs is in a file of its own, `settings.json`:
the profiles folder, the printers added, the printer last selected, and
the process, filaments and bed type last used with each printer. It is
in `~/Library/Application Support/EdgeSlicerRS` on macOS,
`%APPDATA%\EdgeSlicerRS` on Windows and `$XDG_CONFIG_HOME/EdgeSlicerRS`
(or `~/.config/EdgeSlicerRS`) elsewhere; `EDGE_DESKTOP_DATA` names another
folder for it. The C++ application's own settings are never read or
written.

`apps/desktop` is a Cargo workspace of its own, so `cargo test --workspace`
at the top builds no web view. The page (`ui/`) is plain HTML, CSS and
JavaScript with no build step, and asks the engine for everything through
one function (`src/api.rs`; its twin there answers the 3D views' meshes
and moves in bytes), which does not know about Tauri and holds
every open project. `--serve`
puts the same page and the same function behind a local web server, for
working on the page in a browser and for checking it where a window
cannot be seen; it listens on the loopback address only and takes calls
only from the page it served. `EDGE_DESKTOP_TRACE=1` notes each call the
page makes on standard error.

Checked so far, on the Mac mini: in a browser through `--serve`, in light
and dark, three projects were open at once (a three-plate project of
seven filaments, a bare STL with presets, a small project): opened,
switched, reordered, closed and refused past the eighth; plates sliced
one at a time and all at once, cancelled, looked through in each view
and exported. With an empty settings folder (`EDGE_DESKTOP_DATA`), again
in a browser, light and dark: Printer Selection opening by itself,
searched, filtered and sorted; printers of three vendors added and one
removed while a project was on it; a three-plate project and a bare STL
moved between printers and nozzles, and sliced on one; the bed type and
the last printer found again after the page was loaded anew and after
the server was started again. The 3D views, in the app's own browser
pane (WebGL2 on the Mac's graphics card), light and dark, wide and
narrow: the three-plate project, a Benchy and a 20 mm cube turned, moved
and zoomed, the views and their keys, objects and plates selected by a
click, each plate sliced and looked through in every view type with the
two-thumbed slider, the moves slider and the legend's switches; the
inventory has the sizes and times measured. The tabbed window itself (Tauri) has not
been run since it got its tabs and strip; before that the owner ran it by hand, and its
file dialogs, slicing and layer view worked, with G-code byte for byte
the command line's but for the time in its first line. Not built on
Windows or Linux.

## Status

Working:

- STL and 3MF in, per-layer regions out: outer boundaries counter-clockwise
  with their holes clockwise, overlapping parts united, negative parts
  subtracted from their own object, and cracks between flush parts closed by
  `slice_closing_radius` as the reference does. Deterministic across thread
  counts. Every 3MF in the reference repository slices with no open contours.
- Elephant foot compensation (`elefant_foot_compensation`, over
  `elefant_foot_compensation_layers`, the amount falling off evenly
  layer by layer): the first layers are printed smaller than modelled
  by an amount that follows how wide the part is at each point of its
  outline. Where the part is wider than an external wall and its
  spacing plus twice the compensation, the outline comes in by the
  whole of it; where it is no wider than the wall and its spacing, not
  at all; between the two by half of what the part has to spare, so a
  thin stroke is left exactly a wall wide, and the amount is evened out
  along the outline so there is no step where one case meets the next.
  A region too small to spare anything is left alone, and so is one
  the shrinking would part in two. A port of the reference's
  `elephant_foot_compensation` (ElephantFootCompensation.cpp: the
  outline resampled every 0.5 mm, the width measured across the part at
  each point, the smoothing, and an offset with a distance for every
  vertex), called as the reference calls it (PrintObjectSlice.cpp): a
  layer in one material is shrunk as it is; a layer in several is first
  closed by one and a half closing radii, so the seams between
  materials are not taken for outline, and each material keeps what it
  has inside the result. What a layer was before shrinking is kept as
  its modelled outline, which supports, brim, skirt and the layers
  above go by. Not as the reference, each marked in the code: a layer
  counts as being in several materials when more than one filament has
  a share of it, where the reference counts each object's regions; the
  two lengths arrive in whole nanometres, where the reference carries
  fractions of one; an outline edge of no length does not spoil the
  lengths measured along a ring (the reference divides by zero there),
  and a region that simplifying parts in two is shrunk piece by piece
  (the reference keeps the first piece and drops the rest). On a comb
  with teeth 0.5 to 2 mm wide (`tools/make_fixture.py`, `comb`) the
  first layer's outer wall lies within 0.011 mm of the C++ slicer's
  everywhere; it was up to 0.17 mm off while wide parts were shrunk by
  the whole amount and narrow ones not at all.
- A precise height (`precise_z_height`): layers are laid from the bed
  up for as long as the middle of one is still inside the part, so the
  last ends up to half a layer above or below the part's top; with this
  on, the last five layers share that difference out between them, each
  made a little thinner or thicker within what the nozzles in use allow
  (`min_layer_height`, `max_layer_height`), and are cut through their
  middles as they then lie. A port of the reference's
  `adjust_layer_series_to_align_object_height` (Slicing.cpp). Walls,
  fill, skirt, prime tower and supports on those layers are laid at the
  layer's own height. Not as the reference, marked in the code: a layer
  that has reached the least or greatest height takes no further share
  (the reference gives it one, and overshoots; it shows only where the
  five layers differ in height, which they do not here). On a 20 mm
  cube at 0.2 mm on a 0.25 mm first layer both slicers end at 20.05 mm
  without it and with it lay the last five layers at 0.19 mm, to
  19.24, 19.43, 19.62, 19.81 and 20.00 mm; on one 20.14 mm tall, at
  0.218 mm, to 20.14 mm. All 100 layers are at the C++ slicer's heights
  in both, walls and layer extents the same to 0.000 mm and plastic per
  millimetre the same on the changed layers, supports included. A shell
  is measured in the layers as tuned: five of 0.19 mm come to 0.95 mm,
  under the 1 mm of the U1's `top_shell_thickness`, so a sixth layer is
  made solid under the cube's top, as in the reference (5.56 m of
  internal solid fill there and 5.57 m here). Noted
  and not done: a part of fewer than six layers, or a difference the
  five cannot take up, is left as it was; and everything on a plate is
  cut at the same heights, so only the tallest object comes out exact
  (the reference tunes each object).
- Overhangs made printable (`make_overhang_printable`, with
  `make_overhang_printable_angle` and
  `make_overhang_printable_hole_size`): from the top layer down, each
  layer is made to hold the layer above less what a layer may reach out
  past the one beneath at the angle set (measured from upright: 90
  changes nothing, 0 lets nothing overhang), so a slope grows under
  every overhang down to where it meets the part or the bed. The roof
  of a hole in the underside is not sloped but filled, hole and all,
  unless the hole is smaller than the size set. A port of the
  reference's `apply_conical_overhang` (PrintObjectSlice.cpp), done
  where the reference does it: on the layers as cut, before paint, size
  compensation and the elephant's foot; a brim goes round what the
  slope puts on the bed. On a layer in several filaments what is added
  goes to the filament above it. Not as the reference, marked in the
  code: the reach is `tan(angle)` times the height of the layer that
  overhangs, where the reference takes the `layer_height` setting for
  every layer (with a precise height on as well, a slope under the
  changed layers lies up to 0.043 mm from the C++ slicer's); and
  everything on the plate is grown as one stack, which differs only
  where a part floats over nothing. Against the C++ slicer, same layers
  throughout: on the mushroom at the default 55° the wall length of a
  layer differs by 0.01% on average and 0.1% at most, layer extents by
  0.001 mm and 0.004 mm; at 30° by 0.02% and 0.005 mm at most; on the
  shelf by 0.00% and 0.002 mm; on the 3DBenchy, with a precise height
  as well, by 0.07% on average (as without either) and 0.013 mm at
  most. On a slab with two pockets underneath (`tools/make_fixture.py`,
  `pocket`) both are filled by default and the bridge over them
  (0.43 m in the C++ slicer's file, 0.46 m here) goes in both; with a
  hole size of 50 mm² the 16 mm² pocket is kept and the 100 mm² one
  filled, walls and extents the same to 0.000 mm. A part whose slope reaches the bed gets the same
  1.12 m of brim in both. Growing the layers is done one after another
  and takes about 0.2 s on the 3DBenchy's 240 layers. Noted and not
  done: on a part with colour painted on it, what is added is printed
  in the filament the part is made of. Not read: the option set for one
  part or modifier, as with every setting.
- Every preset the reference ships resolves. `slice` takes layer heights and
  the closing radius from the command line, a process preset, or the 3MF's
  own settings, in that order.
- Walls: the classic constant-width generator ported from the reference,
  with widths and counts from the same settings (`wall_loops`,
  `outer_wall_line_width`, `inner_wall_line_width`, `line_width`,
  `initial_layer_line_width`, `infill_wall_overlap`, `resolution`), and the
  infill area left inside them. `slice --svg` draws each wall at its real width.
- Walls of changing width (`wall_generator` at `arachne`, the reference's
  default; the U1 and Elegoo presets set `classic`): as many lines as fit
  across a part wherever it is measured, each as wide as its share, so a
  thin part is one line and nothing is left to gap fill. A port of the
  reference's Arachne, tuned by the same settings (`min_bead_width`,
  `initial_layer_min_bead_width`, `min_feature_size`,
  `wall_transition_length`, `wall_transition_angle`,
  `wall_transition_filter_deviation`, `wall_distribution_count`,
  `min_length_factor`, `wall_maximum_resolution`, `wall_maximum_deviation`).
- A precise outer wall (`precise_outer_wall`, with inner walls laid
  first, as the reference has it): the outer wall keeps clear of the
  wall behind it by their widths, not their spacings, so the wall behind
  does not push it outward. Both wall generators follow it.
- Walls over air (`extra_perimeters_on_overhangs`): where the fill of a
  layer hangs over nothing and cannot be bridged, it is covered with
  lines that start from what holds it up and work inward, each against
  the one before, laid as overhang walls ahead of the island's loops. A
  port of the reference's `generate_extra_perimeters_over_overhangs`.
- Fuzzy skin (`fuzzy_skin` at `external`, `hole`, `all` or `allwalls`):
  a point every `fuzzy_skin_point_distance` or so along a wall, moved up
  to `fuzzy_skin_thickness` to one side (`displacement`), or the wall
  made wider and narrower there (`extrusion`, with walls of changing
  width), or both (`combined`). The noise is the reference's: `classic`
  (random), `perlin`, `billow`, `ridgedmulti`, `voronoi` and `ripple`,
  with `fuzzy_skin_scale`, `fuzzy_skin_octaves`,
  `fuzzy_skin_persistence` and the ripple settings; not on the first
  layer unless `fuzzy_skin_first_layer`; kept off walls with nothing
  near beneath them, and off every wall over air with
  `fuzzy_skin_skip_overhangs`.
- XY size compensation (`xy_contour_compensation`, `xy_hole_compensation`):
  outlines are moved out and holes made larger by what is set, or in
  and smaller, growing first and shrinking after, as the reference does
  it (`_shrink_contour_holes`). On a layer in several filaments the layer
  is resized as a whole and each filament keeps its share of it. Not
  used on a plate with colour painted on it, and a note says so: the
  reference leaves out the painted object only.
- Polyholes (`hole_to_polyhole`): a round hole that runs up through
  the part is walled as a polygon of few sides with its flats on the
  circle, so the hole prints the size it is drawn (PrintObject.cpp
  `_transform_hole_to_polyholes` and `create_polyholes`). A hole is
  round when it has more than eight points, is convex, and its points
  and the middles of its sides keep within `hole_to_polyhole_threshold`
  (a length, or a share of the radius) of one distance from its middle;
  it is converted where it runs through two layers or more, or is on
  the first layer. The number of sides comes from the radius and the
  nozzle of the filament the hole is in (twice the diameter in mm for a
  0.4 mm nozzle, three at least, `hole_to_polyhole_max_edges` at most),
  and with `hole_to_polyhole_twisted` each layer's polygon is turned
  two fifths of a side from the one below. It is done last, as in the
  reference: round the hole as XY compensation, elephant foot
  compensation and the circle compensation left it, so the first
  layer's hole, which elephant foot compensation opens, gets a polygon
  of its own, and with that compensation spread over several layers
  the layers between the first and the last of them keep their round
  hole.
- Auto circle compensation (`enable_circle_compensation`): outlines and
  holes that are circles (eight points or more, all near one distance
  from the middle) are resized by `c1 * speed + c2 * diameter + c3`,
  held between two limits, with the filament's own
  `circle_compensation_speed`, `counter_coef_1` to `3`,
  `counter_limit_min` and `max` for outlines and `hole_coef_1` to `3`,
  `hole_limit_min` and `max` for holes (LayerRegion.cpp
  `auto_circle_compensation`, which the reference has from Bambu
  Studio); a filament that carries none has the reference's defaults.
  `circle_compensation_manual_offset` takes half of itself off either:
  positive makes holes larger and outlines smaller. It is done after
  XY compensation and elephant foot compensation, layer by layer. The
  walls round a circle under the filament's `diameter_limit` are told
  apart (every outline wall of a round island, and every wall round a
  round hole) and laid at the compensation speed in place of the
  walls' own, the first layer's speed, the hot end's flow and a small
  loop's lower speed still holding them, and they are not slowed to
  give a layer time to cool; a loop any of which is laid as an overhang
  wall is a wall like any other. They are joined with a scarf seam
  (`apply_scarf_seam_on_circles`, on unless turned off; scarf seams,
  below), whatever `seam_slope_type` says. Not written: the
  reference's "; Slow Down Start" and "; Slow Down End" comments round
  them, which only its own cooling stage reads.
- With either of the two on, the layers are cut without the points
  where the cutting plane crosses an edge inside a flat face, as the
  reference cuts them (TriangleMeshSlicer.cpp `coplanar_edges`): they
  are corners of the mesh's triangles, not of the model. Cut the plain
  way, a round hole is not round enough to be told for one: where such
  a point lies a unit or two off its flat, closing the outline by
  `slice_closing_radius` leaves a kink there, up to some hundredths of
  a millimetre from where the corner beside it was. With both options
  off the layers are cut as they always were, kinks and all; cutting
  them this way always is the better outline and is still to do.
- Thin walls (`detect_thin_wall`, classic walls): a part too thin for a
  wall down each side gets one line down its middle, as wide as the
  part, laid as an outer wall; a fuzzy skin on the outer wall is on it
  too.
- Which way round and in what order walls go: every wall
  counter-clockwise, round a hole as round the part, or all clockwise
  with `wall_direction` at `cw`; inner wall first, outer wall first, or
  the outer wall between the inner ones (`wall_sequence` at
  `inner-outer-inner wall`, above the first layer); one wall more on
  every other layer (`alternate_extra_wall`); and walls over a steep
  overhang turned about on every other layer (`overhang_reverse`, with
  its threshold and `overhang_reverse_internal_only`). A wall loop no
  longer than a circle of `small_perimeter_threshold` goes at
  `small_perimeter_speed`.
- Gap fill: a gap between walls too narrow for another wall gets a line
  down its middle, as wide as the gap is at each point, found as the
  reference finds it (the medial axis of the gap). On whenever
  `gap_infill_speed` is above zero.
- The fill's own gap fill (`gap_fill_target` at `topbottom` or
  `everywhere`; `nowhere` is the reference's default and the U1's
  presets set `topbottom`): what the lines of a top or bottom surface
  leave bare inside the walls, at the points of corners and against
  slanting walls, gets lines of whatever width fits, found as the gaps
  between walls are; with `everywhere`, solid fill inside the part
  does too. A port of the reference's `_create_gap_fill` (FillBase.cpp).
  On the lettered plate with walls of changing width the C++ slicer
  lays 0.09 m of it and this engine 0.10 m, where it laid none. With
  classic walls, layer by layer: a layer with a top surface has 294 mm
  of gap fill in both (287 mm here before), another 357 mm against
  368 mm (319 mm before); the first layer has 52 mm in the C++
  slicer's file and 55 mm here (151 mm before the first layer was
  shrunk as the reference shrinks it: most of that was gaps between
  walls that should not have been there). Not as the reference: the gap lines of a surface swept from
  side to side follow that surface's lines as a group, where the
  reference slips each in among them wherever that saves travel
  (`insert_gap_fill`).

Known gaps: painted supports and seams, per-object setting overrides, typed settings
(values are text until a stage needs them), and modifier parts.

- Infill: each layer's inside is split into top, bottom, bridge, solid
  shell and sparse areas (`top_shell_layers`, `bottom_shell_layers`,
  `sparse_infill_density`, `minimum_sparse_infill_area`) and filled with
  lines, or for sparse areas one of nineteen patterns (below). Solid lines run to the edge of their
  area and, where one is joined to the next, turn half a line spacing
  inside it, as the reference's do. `top_surface_pattern`,
  `bottom_surface_pattern` and `internal_solid_infill_pattern` say whether
  lines are joined (`monotonic`, `rectilinear`) or left separate
  (`monotonicline`); any other pattern is laid as joined lines, with a
  note. Both monotonic patterns are swept from one side of the area to
  the other, so that no line goes down between two already laid, which
  shows on a surface: the area falls into regions that can each be
  zigzagged in one go, and a region waits for everything beside its
  first row on the side already swept (FillRectilinear.cpp
  `generate_montonous_regions`). The writer lays such lines in the
  order given and never turns one round. Top and bottom surfaces are fitted to their area: the lines are
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
  beneath, at `internal_bridge_speed`. What it lies over is taken from
  the whole layer beneath, whatever the filament, so a colour that
  starts part of the way up a part is bridged over the fill of the
  colour under it. Under the edge of such a bridge, where the edge runs
  along the walls of the layer below, a band of that layer's sparse fill
  is made solid to carry the ends of its lines
  (PrintObject.cpp `bridge_over_infill`).
  With `ensure_vertical_shell_thickness` at `ensure_all`, whatever of a
  layer's fill is not over and under the fill of the layers within a
  shell of it is made solid too, so a leaning wall stays as thick
  measured straight up as it is sideways; walls between two filaments
  inside a part do not count (`discover_vertical_shells`). Any other
  value of that setting is noted and not followed.
  A shell is as many layers as `top_shell_layers` or
  `bottom_shell_layers` says, and as many more as it takes to measure
  `top_shell_thickness` or `bottom_shell_thickness`: a layer is shell
  under a top face while the layers from its top to the face's come to
  less than the thickness set, and over a bottom face or a bridge while
  those from its bottom to the face's do, each layer at the height it
  has. That is the reference's rule in both the places it makes shells
  (PrintObject.cpp `discover_vertical_shells` and
  `discover_horizontal_shells`). Zero leaves it to the count, and with
  no layers asked for there is no shell whatever the thickness. While
  only the count was followed, a 20 mm cube with the Snapmaker U1's
  0.16 mm process (six layers, 1 mm) got 6.34 m of internal solid fill
  where the C++ slicer lays 7.22 m; it gets 7.22 m now, on the same
  layers (more under Comparing with the reference). Where the count
  already makes the thickness, as five layers of 0.2 mm do with the
  U1's 0.2 mm process, the file is byte for byte what it was. Noted
  and not done: paint on a top or bottom face is carried as deep as
  the thickness takes in layers of the layer height, which is not the
  shell's depth where the first layer or the last five are of another
  height. Fill over air takes `bridge_flow`
  and `bridge_density`, and runs along the edges it has nothing beside,
  so its lines go from one support to the other. Bridges, bottoms and
  tops reach a wall's spacing into the fill beside them, to have
  something to hold on to.
  The bridging options of the reference's Quality tab are followed, all
  but one. `bridge_line_width` gives bridges, bridges over infill and
  walls over air a width of their own: a round thread is that thick in
  place of the nozzle's bore, a flattened line that wide before
  `bridge_flow` thins it; wider than the nozzle, or no wider than the
  layer is tall while either kind of bridge is flat, is an error, as in
  the reference (`LayerRegion::bridging_flow`, `Print::validate`).
  `bridge_expansion_extra` carries a bridge that many millimetres
  further onto solid fill and onto a top or bottom surface beside it,
  grown in the reference's waves so that it follows the area and does
  not jump a gap, and bridges that grow into one another become one
  (`expand_bridges_detect_orientations`, Algorithm/RegionExpansion.cpp);
  sparse fill beside a bridge is reached no further, in the reference
  either. `dont_filter_internal_bridges` at `limited` and `nofilter`
  lets smaller patches of solid fill over sparse fill be bridged (the
  first sorting-out of `bridge_over_infill` with one spacing where the
  default has three, and with no filter every patch found, four
  spacings round it), kept as the reference keeps them to where a
  thick bridge's thread has sparse fill to sink into and no bridge just
  beneath. `enable_extra_bridge_layer` lays a second layer of bridge
  over bridges over air (the same way as the first, and the bottom
  shell then starts from the second), over bridges over infill (square
  to the first), or both. Each is off by default and changes nothing
  then. `counterbore_hole_bridging` is noted and not followed.

- Ironing (`ironing_type`): a finished surface gone over again with
  lines close together and next to no plastic, the nozzle smoothing
  what is there. A port of the reference's `Layer::make_ironing`
  (Fill.cpp). `top` irons wherever a layer faces up with nothing over
  it, at whatever height; `topmost` the last layer of each object;
  `solid` every layer that is solid right through, walls and all, and
  on any other layer what faces up, what lies on the bed and whatever
  of its fill is solid (never a bridge). The ironed area is kept
  `ironing_inset` from the edge of the layer, half a nozzle when that
  is left at zero, and from nothing else: a top surface is ironed right
  up to what stands on it, and over its own walls. Lines are
  `ironing_spacing` apart (never under 0.05 mm) and carry
  `ironing_flow` percent of a layer's plastic shared out over that
  spacing: with the U1's presets a line 0.0075 mm tall and 0.402 mm
  wide, 0.003 mm³ a millimetre. `ironing_pattern` is `rectilinear`,
  straight lines swept from one side of the area to the other and
  joined along its edge where that is no more than three spacings, or
  `concentric`, loops that follow the edge from the outside in. Straight
  lines run along the top surface's own (`solid_infill_direction`, a
  quarter turn on every other layer, or `top_layer_direction` on every
  layer) turned by `ironing_angle`, or with `ironing_angle_fixed` at
  that angle itself on every layer, and lie on a grid through the
  middle of their object. A filament's own `filament_ironing_flow`,
  `_spacing`, `_inset` and `_speed` are used in place of the process's
  where it sets them. Each filament irons its own share of a layer.
  The writer lays an island's ironing last, when its walls and fill
  are down, as `;TYPE:Ironing`, at `ironing_speed` and the default
  acceleration, each area swept from the side it was made to start
  from, and with `ironing_fan_speed` set the fan runs at that for as
  long as the ironing lasts. With `ironing_type` at `no ironing` a
  file is byte for byte what it was.
- G-code: `slice --gcode out.gcode` orders each layer (skirt and brim,
  each part's brim whole before the next, then island by island: an
  island's walls from the inside out, each beside the last, then the
  gap fill and the fill that lie in it), holds speeds to the
  hot end's melt rate, sets acceleration by feature, and reports filament,
  weight and time. Following the reference:
  - Seams: every point of every outermost wall is weighed as a place
    for its seam before the file is written, as the reference's
    SeamPlacer weighs it, object by object and in each object's own
    coordinates. First, how far the wall's line hangs out over the
    layer beneath there (the middle of the line outside that layer's
    outline, with 0.65 of the line's width, less the layer's height):
    of two points either of which hangs out, the one that hangs out
    less is the better. Then whether the point lies hidden inside the
    layer's own outline, more than half a mm, which it can where two
    materials meet. Then a score: the angle the wall turns through
    there, taken between the points an arm of the nozzle's width either
    side, least where it folds into the part and most on a straight
    run. With `seam_position` at `aligned` the best score takes the
    seam; at `nearest` the distance from where the nozzle is counts
    beside it, and the seam is picked as the wall is laid; at `back`
    the point furthest back takes it whatever its score; `aligned_back`,
    `aligned_front`, `left` and `right` lean to that side of the bed,
    what faces the other way counting as easier seen; `random` picks a
    place along the stretches that set off from points about as good as
    the best, by a figure made from the wall's first point, the same on
    every run. Of points as good as each other the first is taken,
    counting from where the reference's walls start, which is the
    point after a wall's lowest (its clipping library lists a polygon
    so; this engine's starts one elsewhere). An inner wall starts
    beside the seam of the outer wall nearest it: at the point nearest
    the seam, or where the outer wall folds inward there, in its own
    corner; with `staggered_inner_seams` on round from that by its
    depth and no less than its own width, so the seams do not lie one
    behind another. Walls stop `seam_gap` short of their start. Seams
    are kept in line from layer to layer by following the one beneath
    where a point near it is no worse by much; the reference's curve
    through them is not fitted yet. What differs by intent: a wall is
    measured against everything on the layer beneath, not its own
    object's layer alone; and a loop opened at a seam that is not one
    of its points is opened at the nearest point of the loop, where the
    reference takes the nearest of its stretches that rest.
  - Scarf seams (`seam_slope_type`): a loop joined this way is laid
    whole, with no gap at its seam. Its first 20 mm
    (`seam_slope_min_length`, or all of it with
    `seam_slope_entire_loop`) climb from the top of the layer beneath,
    or from `seam_slope_start_height` above it, to the layer's own
    height, in moves no longer than a tenth of that length
    (`seam_slope_steps`), each laying as much of the line as the height
    it ends at; and when the loop comes round to its start it goes on
    over that stretch, level, laying what is left of the line, to two
    seam gaps short of where the slope ends. The joint is laid at
    `scarf_joint_speed` where that is slower than the wall (a
    percentage is of the wall's own speed, or of the compensation
    speed on a compensated circle, on a small loop as on any), with
    `scarf_joint_flow_ratio` of the plastic. `external` joins the loops
    round the part so and `all` those round holes too: outer walls,
    and with `seam_slope_inner_walls` inner ones, whose way up starts
    two fifths of the slope on, so the joints do not lie side by side;
    never on the first layer, nor a loop that starts over air. With
    `seam_slope_conditional` only loops with no corner sharper than
    `scarf_angle_threshold` whose seam is short of the layer beneath by
    less than `scarf_overhang_threshold` of the wall's width. The walls
    of a circle the auto circle compensation resized are joined so
    whatever those say (`apply_scarf_seam_on_circles`). A slope that
    crosses a stretch over air climbs on through it as an overhang
    wall. With arc fitting the slope is straight moves through the
    points its arcs were drawn by and the rest of the loop keeps its
    arcs, as in the reference; a wall of changing width has its slope
    run on from one width into the next. The nozzle comes down to the
    foot of the slope where it would come down to the layer, never
    lower than the layer's bottom, and is wiped on round the loop from
    where the slope came up. Five things are not as in the reference,
    each a mistake there: a slope of no length is a plain seam with
    its gap, where the reference closes the loop with neither; an
    inner wall's way up starts two fifths on whatever the seam gap,
    where the reference clips nothing with the gap at 0;
    `scarf_joint_speed` holds the joint on a small loop too, where the
    reference lets it go at `small_perimeter_speed` (half of a
    100 mm/s wall is 50 mm/s on a loop small enough to go at 60, as
    the fix to the reference has it, aceRage/EdgeSlicer#958); a joint speed of 0
    is the wall's own speed, and said, where the reference then goes
    as fast as the hot end melts; and how far a seam may hang out is a
    share of the wall's own width, where the reference takes the width
    as set, which is 0 when it is left to follow the nozzle. One thing
    is different without being wrong there: the nozzle goes down to
    the foot of the slope as a move of its own after the travel, where
    the reference makes an unretracted travel's last leg the way down.
  - Moves at a wall's seam that hide it, none of which lays anything
    or is made where it would end off the bed (GCode.cpp
    `extrude_loop`). With `wipe_on_loops` an outer wall of a part set
    to have more than one wall ends with a move a fifth of the nozzle's
    width in from where it started, a third of the corner there round
    toward the part, at whatever speed the wall ended at; the nozzle
    leaves, or is wiped, from there. With `wipe_before_external_loop`
    an outer wall with another wall beside its seam is come at from
    inside the part: the travel ends a nozzle's width from the wall's
    start (or the line's width, if that is less), back the way the
    wall ends and a third of the corner in; the filament comes forward
    there, and the nozzle moves out to the start at the outer wall's
    speed. With `wipe_inward` the wipe after an outer wall goes over
    the inner walls already down behind it, `wipe_inward_distance` in
    from the wall (half the wall's width unless set, never more than
    its width) and `wipe_distance` long, pulling nothing back; a
    retraction the next move needs is then made whole where the wipe
    ends. The wipe's path is the reference's
    (GCode/WipePathHelpers.cpp, function for function, and it passes
    that file's own 36 tests): the wall moved in toward the nearest
    inner wall with its corners mitred, or moved bodily, or one short
    move in, whichever comes first that lies over walls already
    printed all along and on the inner wall's side of the outer one;
    with none of them, as when the outer wall goes down first, the
    nozzle is wiped along the wall as ever. All three go with either
    wall generator, walls laid either way round, every wall order,
    arcs and scarf seams (after a scarf they are made about where the
    loop's flat part begins and its way down ends, as in the
    reference). Two things are not as in the reference, each a mistake
    there: an outer wall is taken to lie beside the point it is itself
    come at from, where the reference measures, and on the part's side
    of a corner of 270 degrees or more (any corner of a square hole)
    finds that point exactly a nozzle's width away, in reach or out of
    it by a rounding; and a filament whose own `wipe` is off gets no
    inward wipe, where the reference makes one if any filament's is
    on. One thing is narrower: the walls looked for beside a seam are
    the island's loops, where the reference also has the walls of thin
    parts that are not loops among them.
  - Order of fill and supports: the fill of an island comes in parts,
    one for each area filled, of whatever kind. The parts are chained by
    their closest ends from where the nozzle is, the shortest hop
    between two ends first, and then the lines of each part the same
    way, so an area is finished before the next is begun
    (ShortestPath.cpp `chain_segments_greedy_constrained_reversals_`,
    which `chain_and_reorder_extrusion_entities` orders with). Where
    that chain would turn round a line that may not be turned, a swept
    surface or a tree branch's wall, the lines are taken nearest first
    instead, as there. Gap fill and supports are chained the same way,
    each kind by itself. The reference finds near points with a k-d
    tree; here every point is looked at and of two equally near the
    earlier wins, so equal hops can be taken in a different order.
  - Travel: retraction with wipe, and straight, ramped or spiral lifts
    (`z_hop_types`), at feedrates the Z axis can follow. With
    `reduce_infill_retraction` a move to a fill line goes unretracted
    when it stays inside one piece of the layer that is neither a top
    face nor a bottom one, walls included
    (RetractWhenCrossingPerimeters.cpp), unless it leaves an outer or
    an overhanging wall.
  - Travel round the walls (`reduce_crossing_wall`): every travel whose
    start is known is routed as the reference routes it
    (AvoidCrossingPerimeters.cpp, GCode.cpp `travel_to`). A move in the
    part that does not stay within one piece of the layer, clear of its
    outer wall, follows the layer's outline drawn in by one and a half
    wall spacings (by less where the part is too narrow for that, so
    that no piece is lost or parted), from where it first meets it to
    where it last leaves it, the shorter way about, and is then
    straightened as far as that outline lets it be. Top surfaces are
    gone round. Moves between supports are routed by the part's
    outline as well, and moves to, round and from a skirt, a brim or
    the prime tower go round holes only, in the hole's own wall. The
    first move of the print, the one from the first layer's skirt and
    the one from a brim go straight. A way round longer than
    `max_travel_detour_distance` allows, a length or a share of the
    straight move, is not taken. A move is as long as its way round
    when it is asked whether it retracts; with
    `reduce_infill_retraction` a way round that stays inside the part
    goes unretracted; and the lift is made over the first stretch of
    the way. Everything on a plate is sliced as one, so a move from one
    object to another is routed as one between two pieces of an object,
    where the reference takes it round holes only; a note says so.
    Layers are made ready to route on in parallel, a stretch of them at
    a time, ahead of being written, and a routed travel that would
    leave the bed is not made.
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
  - Arcs (`enable_arc_fitting`): the stretches of a path that lie on a
    circle are written as one `G2` or `G3` each, with the reference's
    fitter (ArcFitter.cpp, Circle.cpp): walls, fill of every kind and
    gap fill within `resolution` of the path, sparse infill within
    0.04 mm and supports within 0.0375 mm. A wall loop is fitted whole
    and then opened at its seam, which cuts the arc there in two; a
    line is fitted as it was made and turned about with its arcs if it
    is laid from its far end. A stretch of wall whose speed or fan
    changes on the way over an overhang is straight moves, as there.
    Skirt and brim are never arcs, nor wipes and travel, but for the
    spiral lift, which is then one `G17` and `G3` in place of its many
    short moves. The prime tower's rounded ribs stay straight moves
    (the reference fits them within 0.1 mm). An arc counts by its
    bulge when the print is checked against the bed. Three things are
    not as in the reference, each a mistake there: it leaves a path's
    middle point out when it looks for the circle a path strays from
    least, it keeps the wrong angle with an arc it had to turn about,
    and it takes a counter-clockwise arc whose middle point is exactly
    due east of its centre for no arc at all, which cuts the arc of a
    round wall drawn square to the axes in two on every layer.
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
    from. They pair off, nearest first, and each pair leans together
    until it is one. A leaning branch is drawn as the oval a level cut
    through its tube makes. Each loop starts from a hook laid inside it
    (SupportCommon.cpp `tree_supports_generate_paths`), and is always
    laid hook first. On the bed the trunks spread out by
    `raft_first_layer_expansion`, as other supports do, which joins
    their feet into a sheet.
- Placement: each plate of a 3MF is moved to bed coordinates, and a bare
  model is centred on the bed of the machine preset. Export refuses
  toolpaths that leave the bed, and writes the file whole or not at all.
- More than one filament, for a printer with a nozzle for each:
  - Each part's filament is read from the 3MF, and every layer is shared
    out among the filaments it is printed in. Each share gets its own
    walls and fill, so parts of different filaments that touch each have
    a wall along the join.
  - Tools are taken in the reference's order (ToolOrdering.cpp). On the
    first layer the tool whose smallest piece is largest goes first, or
    the plate's own order where it sets one
    (`first_layer_print_sequence`). After that each layer starts with
    the tool in hand and takes the rest as the reference's solver does.
    With a nozzle for each filament every order costs the tower the
    same, so which one comes out is a matter of how that solver breaks
    ties and of the order it is handed the tools in; both are ported as
    they stand, because the tower is a layer taller or shorter by it.
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
    a frame and lines across the depth no change uses, a solid first
    layer and a brim. The skirt goes round it and no part's brim is laid
    on its ground. With `wipe_tower_wall_type` at `rib`, as the U1's
    profiles have it, the tower is made as wide as it is deep and has a
    rib along each diagonal that runs out past the corners, longest on
    the bed and gone at the top, with rounded corners
    (`wipe_tower_rib_width`, `wipe_tower_extra_rib_length`,
    `wipe_tower_fillet_wall`); a tower slight for its height gets longer
    ribs. It stands where the ends of its ribs put it, as the reference's
    does. The brim follows the wall, and above the first layer steps in
    a loop a layer from at most `prime_tower_brim_chamfer_max_width`
    (WipeTower2.cpp `finish_layer`, `generate_rib_polygon`).
    A filament set to be rammed (`filament_multitool_ramming`, as most
    of the U1's are) is, by the tool being put away and before the
    change: a line `ramming_line_width_ratio` tower lines wide holding
    `filament_multitool_ramming_volume`, laid at
    `filament_multitool_ramming_flow`, after which that tool waits just
    clear of it and the new tool's first line carries on from where it
    stopped (`toolchange_Unload`, `set_toolchange`). The wall is left
    open for 2.5 mm where each new tool comes in, and the tool comes in
    that way (`wipe_tower_wall_gap`).
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
the U1 that day too: the first print in more than one filament. It was
printed again with the tower as it now is (ribs, the brim stepping in,
each filament rammed before its tool is put away, the wall open where
the new tool comes in): the tower stood, its ribs and stepped brim came
out as drawn, and the owner saw no blobs to speak of. A photograph of
the tower shows a dark smear where the rams are laid and a wall end
by the gap a little out of line; the part itself was not looked at
against the first print. The
`mushroom` fixture with tree supports printed on the U1 as well, beside
the C++ slicer's file for the same project: both caps were carried and
both sets of supports came away. This engine's, as it was then, stood
on separate feet where the reference's first layer is one sheet, with
more and thinner trunks, and kept further from the stem (2.8 mm against 1.6 mm, taking
the middle layer's least distance). Its bridge under the cap runs across
the lines of the support's roof where the reference's runs along them,
and came out flatter with a few blobs; the two were printed in different
filaments, so the look of the surfaces is not a fair comparison. The
same fixture was printed again on the U1 with trees as they are now
(paired branches, hooked loops, a sheet on the bed), and the owner
judged it better than the first: the cap was carried, the first layer
went down as one sheet with a scalloped edge, the branches can be seen
joining in pairs on the way down, and the support came away in one
piece but for a small fragment. The underside of the cap was not
photographed, and the C++ slicer's file was not printed beside it this
time. Normal (grid) supports have not been on hardware. Without a printer
profile a generic preamble is written and the file is only good for viewing.

Multi-filament printing is a first working version. Where parts of
different filaments overlap, the higher-numbered filament has the overlap
(the reference gives it to the later part). `toolchange_ordering` other
than the default and `other_layers_print_sequence` are noted and not
followed. On the first layer two tools whose smallest pieces are the
same to a millionth count as the same, and the lower number goes first;
the reference compares exactly, so where it has identical parts in
several filaments its order among them can differ (plate 3 of
`mcbrim_test.3mf` starts 1, 5, 4, 3, 2 there and 1, 3, 4, 5, 2 here). The tower has no
cone wall; a nozzle coming in is not wiped flat on it first, tool changes
all start at the same place in their blocks
(`wipe_tower_stagger_toolchange_start`), and pressure advance is not
changed for a ram (`enable_change_pressure_when_wiping`); each is noted
when the profile asks for it. The tower is not turned end for end on
alternate layers. A filament with no tool of its own (the fifth on a
four-tool printer) is written as the reference writes it, with a warning.
Filaments mixed from others are not read.

Painted colours are a first working version. A project's `paint_color`
is read, triangles cut up as the paint cuts them (a port of
`TriangleSelector::deserialize`), and each painted object is shared out
between filaments as the reference's `segmentation_by_painting` shares
it with `paint_depth_mode` at `unlimited`, its default: every point of a
layer takes the colour of the nearest stretch of the layer's outline
(the Voronoi diagram of the outline's edges, each coloured by the
painted triangles that cut the layer there), and what is painted on a
top or bottom face is carried through the shell layers behind it, in by
a wall and a space, where it wins over what the sides say. Not a port
line for line: a triangle's cut colours an edge of the outline when it
lies within 0.15 mm of it, where the reference walks a grid and tests
angles; runs of colour under 0.2 mm go to their neighbour; islands of
paint too small for a classic wall are not dropped. Two parts of an
object rarely meet exactly: their meshes follow the same curve with
different corners, and cracks a few micrometres wide are left between
them. This engine closes those cracks when it slices. The reference
does not, so where paint makes both sides of a crack one filament it
lays walls round the crack, a slit of two loops in the middle of a
solid area. That is not copied. The bounded depths
(`paint_depth_mode` at `walls` or `millimeters`), interlocking and
`top_color_penetration_layers` are noted and not followed. Painted
supports, seams and fuzzy skin are not read; a project that has any of
them painted on says so when it is sliced.

Not done in G-code: supports only where painted (none are
generated, and a note says so), the older tree styles (`tree_slim`,
`tree_strong`, `tree_hybrid`: organic-style trees are built, with a note),
rafts, brims inside holes, draft shields, seams placed by visibility, lined
up along a curve fitted through the layers, moved to where parts meet
(`seam_prefer_part_joints`) or painted on or off, a fan speed of its own for bridges over infill, walls
slowed over curled edges, preview pictures in the file
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

A top surface below the last layer
keeps only its outer wall, as the reference's `only_one_wall_top` does with
`top_surface_expansion` at zero (PerimeterGenerator.cpp
`split_top_surfaces`); the other way of doing it, with an expansion set, is
not ported. Overhangs are measured as the reference measures them but probed
at fixed steps, so a wall's speed changes at slightly different places.
Infill is not all a port yet: solid lines are joined along
the walls in sweep order, each to its neighbour on the next row
whatever the distance, as the reference's rectilinear fill joins them
(it means to stop at three spacings, but works the limit out from a
spacing that is still zero). Monotonic lines follow the reference's
rule but not always its order: of the orders the rule allows the
reference looks for a short one with an ant colony run on random
numbers, and here the shortest is worked out exactly for up to twelve
regions and the best of the nearest-next orders is kept for more.
Where the reference tells two rows are beside each other by following
the outline between them, here they are when they overlap. A bridge reaches 0.4 mm onto each support where the reference's
reaches 0.8 mm, internal bridges are not run out to the
sparse lines that carry them nor turned as the reference turns them
(they come within 6% of the reference's on plain parts and 9% over on
the lettered plate; small ones, which gain most from being run out,
come out a quarter to a third short, and that is what
`dont_filter_internal_bridges` at `nofilter` makes more of: below),
bridges that reach into one another are made one only with
`bridge_expansion_extra` set, a hole that narrows on the way up gets
no bridge of its own under the narrow part (`counterbore_hole_bridging`,
noted), narrow solid areas are filled with the same lines
as wide ones where the reference lays loops along them
(`detect_narrow_internal_solid_infill`, noted), narrow parts of top and
bottom surfaces get no gap fill
(`gap_fill_target`), and eight of the
reference's sparse patterns are not laid (below). Gap closing runs across objects, so two separate objects closer than twice
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
could be and picks afterwards; and branches are not smoothed, so they
stand in more regular rows than the reference's.

Next: thumbnails.

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
Centauri Carbon's, whose files have arcs that were then cut up a little
differently here. An arc is now cut into the straight moves the
reference's estimator cuts it into (GCodeProcessor.cpp `process_G2_G3`:
chords within 0.0125 mm of the arc, or for Marlin 2 as its `plan_arc`
does), and on 2026-11-11 the Centauri Carbon's files came to the
reference's figure to the second as well, eight of them, box-shaped and
round (5m 15s to 23m 39s; the bridge was 7m 15s against 7m 14s before,
the shelf 10m 48s against 10m 42s). Planned as Klipper plans, an arc is
cut as Klipper cuts it, into moves a millimetre long (the default
`resolution` of its `[gcode_arcs]`). This engine's own files, planned the same way, came to
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
| `cylinder_drawcut.3mf` | none | 11.22 / 11.30 | 61.04 / 61.08 | 0.02 / 0.00 |
| `twopart_groups.3mf` | 0.16 / 0.16 | 1.16 / 1.20 | 6.38 / 6.38 | none |
| `mushroom` fixture | 0.23 / 0.23 | 1.99 / 2.04 | 10.93 / 10.93 | none |
| `mcbrim_test.3mf` plate 2 | | 2.60 / 2.83 | 24.29 / 25.05 | |
| `mcbrim_test.3mf` plate 1 | 0.10 / 0.06 | | | 8.28 / 9.98 |
| `mcbrim_test.3mf` plate 2 | 0.03 / 0.03 | | | 4.95 / 5.03 |

Supports and the automatic brim are compared on parts written by
`tools/make_fixture.py`, with normal supports turned on:

| Part | Support body | Dense layers | Brim |
| --- | --- | --- | --- |
| `mushroom` (a cap on a stem), normal | -1.6%, same extents | -3.6% | none in either |
| `shelf` (a shelf over a plate), normal | -1.5%, same extents | -5.0% | none in either |
| `mushroom`, tree | -5.2% | -5.3% | none in either |
| `shelf`, tree | +4.0% | -8.9% | none in either |
| `tower` (6 x 6 x 80 mm) | | | same length, same reach (5.63 mm) |
| `bar` (200 x 10 x 30 mm, PLA) | | | same length, same reach (7.42 mm) |

Normal support lines come as close to the part as the reference's
(0.54 mm). Both styles put support on the same layers as the reference,
with the same layers of air above and below. Trees start from about as
many tips (96 against 101 under the mushroom's cap) and join up a little
more slowly on the way down.

On `mcbrim_test.3mf`, printed in up to six filaments, tools are taken in
the reference's order on every layer of plates 1 and 2 (plate 2 sets its
own order on the first layer), and on every layer but the first of
plate 3, with the same number of tool changes (28 on plate 1, 50 on
plates 2 and 3). All three of its plates slice. It is a flat part
3 mm thick covered in lettering. The prime tower is the reference's
length on every one of its twelve layers on all three plates, to within
half a millimetre a layer (39.87 m against 39.87 m on plate 2), with the
same rows, frame and gaps in its wall; its top layer, which has one
change where those below have five, is as shallow as that needs and sits
in the middle, as the reference's does. Plates 2 and 3 carry 0.3% more
filament than the reference (they carried 7.9% less before the tower had
its ribs and stepped brim). On the two-colour test, whose filament is rammed,
the first layer of the tower is the reference's length to a tenth of a
millimetre (2251.4 mm) and stands on the same ground; the tower is 1.4%
under in all, the reference filling out each block where it starts
tool changes at different places. Top surface is within 1.3%, bridges
over infill 9% over (they were 78% under, when a colour that starts
part of the way up was not bridged over the colour beneath), solid fill
3% over and sparse fill 4% under.

The parts on plate 1 are painted as well as made of parts in different
filaments, and the paint is now read. The plate has the reference's 28
tool changes and its four filaments (it had both before, from the
parts alone), filament over the whole plate is 0.5% under (it was 0.8%
over) and the prime tower is the same to the centimetre. Layer by
layer and tool by tool, what each tool feeds is 9.2% away from the
reference's in all, where it was 11.3% before the paint was read: on
the middle layers the border between two colours has moved in from the
rim to where the reference has it, and the tool that prints the rim
feeds 1186 mm there against the reference's 1398 mm (it fed 1027 mm).
What is left: between the bottom shell and the inlays the reference
lays small loops of wall that are not here, 49 of them on the fifth
layer. They are walls round the cracks between parts described above:
each is as wide as two half walls and as long as its crack, they stand
on the border between two parts, they are there only with the paint
(with it stripped from the project the reference lays none), and they
start above the bottom shell, where the bottom face's paint keeps the
two sides different colours. And the reference fills narrow parts of
the top surface with gap fill (405 mm against 310 mm for the inlays'
tool on their top layer). The plate takes 0.9 s to slice, most of it the painting:
eight objects of 85,000 triangles each.

Top and bottom surfaces now come to the reference's length within 0.1%,
at the same pitch and width, on the cube, the deck, the tower and the
mushroom with both printers' presets and on the cylinder (they were 2 to
26% over). A full layer of solid fill inside the part does too: 907.9 mm
against 907.0 mm on the cube. Over a whole part solid fill is within
0.2% on the cube, the deck, the shelf, the mushroom, the two-part
project and the cylinder (it was 1 to 5% under, for want of the band
under the first bridged layer) and 2% over on the tower. Small gaps
left inside solid fill are not filled.

Monotonic surfaces are now swept in order. On the three plates of
`mcbrim_test.3mf`, counting the lines of a surface laid between two
already down on the layers looked at (the first, the eleventh and the
last): 13 on plate 2 and 10 on the eleventh layer of plate 3 before,
none now, as in the reference's files; 19 on the eleventh layer of
plate 1 before, none now. A blade's top surface comes out in the same
pieces as the reference's, with the same number of lines (190 against
189) and of long moves between them (6 each).

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
presets was 18% under). Each object's pattern is laid through that
object's own middle, as in the reference; it used to be through the
middle of everything on the plate, which put the lines of every object
but a lone one somewhere else than the reference's.

Walls of changing width. `edge-arachne` is a port of the reference's
Arachne (src/libslic3r/Arachne): the outline is prepared as
`WallToolPaths` prepares it, cut into the trapezoids of its Voronoi
skeleton (`SkeletalTrapezoidation`), the number of lines that fit is
decided along the skeleton by the reference's beading strategies, each
change in that number is given a stretch of skeleton to happen over,
and the lines are laid across each trapezoid, joined end to end and
simplified. A diagram that comes out broken is built again for the
outline turned by a few odd angles, as the reference does. With the
U1's presets and `wall_generator` set to `arachne`, C++ slicer against
this engine, walls in metres:

| Part | Outer wall | Inner wall |
|---|---|---|
| 20 mm cube | 7.82 / 7.82 | 7.44 / 7.44 |
| tower | 8.90 / 8.90 | 7.63 / 7.63 |
| mushroom | 4.16 / 4.16 | 3.83 / 3.83 |
| shelf | 4.22 / 4.22 | 3.83 / 3.91 |
| `mcbrim_test.3mf` plate 2, lettered | 21.08 / 21.12 | 18.13 / 18.19 |
| `mcbrim_test.3mf` plate 1, painted | 44.18 / 42.34 | 37.18 / 36.28 |

On the lettered plate a layer's walls are within half a percent of the
reference's in length on average, and laid over one another the two
engines' lines coincide through the thin strokes, where the widths
change. A loop is a loop whatever its widths: one whose width changes
carries its width at each point, is ordered and given its seam by the
writer as any wall is, and is laid from the seam as pieces end to end,
each of one width, cut where the width has drifted by 0.05 mm
(`thick_polyline_to_multi_path`). `only_one_wall_top` is followed as
the reference follows it with `top_surface_expansion` at zero: the
outer wall is made alone, the top surface inside it is found from the
layers above and below, and the inner walls go round what is left; on
the lettered plate the top surface is then 4.47 m against 4.54 m. The
painted plate's walls are 2 to 4% short of the reference's with classic
walls as well (44.38 m against 42.41 m): that is the painted regions
being split differently, not these walls. Not a port of the whole of
`process_arachne`, and what differs is said here:

- The walls that are not loops (lines down the middle of thin parts)
  are laid after the island's loops, whichever of walls and fill goes
  first, and begun where they happen to begin. The reference orders
  them among the loops (`getRegionOrder`).
- With `top_surface_expansion` set, the reference cuts the inner walls
  where they run over the top surface; that is not ported, for these
  walls or the classic, and a note says so.
- With `inner-outer-inner` the reference first orders these walls by
  which lies against which (`reorderPerimetersByProximity`); here they
  are ordered by nesting, as the classic walls are, and then put in
  the same inner-outer-inner order.
- The check of a diagram that needs exact arithmetic
  (`is_voronoi_diagram_planar_angle`) is not here.

Wall options, with the U1's presets, C++ slicer against this engine:

| Option, part | Feature | C++ slicer | This engine |
|---|---|---|---|
| `precise_outer_wall`, cube, classic | Inner wall | 7.40 m | 7.40 m |
| `precise_outer_wall`, cube, `arachne` | Inner wall | 7.42 m | 7.42 m |
| `extra_perimeters_on_overhangs`, shelf | Overhang wall | 0.62 m (0.10 m off) | 0.62 m |
| `extra_perimeters_on_overhangs`, mushroom | Overhang wall | 2.50 m (0.23 m off) | 2.49 m |
| `fuzzy_skin` `external`, cube, classic | Outer wall | 8.82 m (7.82 m off) | 8.83 m |
| `fuzzy_skin` `external`, cube, `arachne` | Outer wall | 8.89 m | 8.87 m |
| `fuzzy_skin` `all`, `perlin`, cube | Outer wall | 8.06 m | 8.05 m |
| `fuzzy_skin` `allwalls`, `extrusion`, cube, `arachne` | Outer wall | 7.88 m | 7.87 m |
| `fuzzy_skin` `external`, `ripple`, cube, `arachne` | Outer wall | 7.94 m | 7.96 m |
| `detect_thin_wall`, fins 0.5 to 1.1 mm thick | Outer wall | 1.85 m (2.08 m off) | 1.85 m |
| XY compensation, frame: hole +0.1, outline +0.08 | Outer / inner wall | 8.44 m / 8.16 m | 8.44 m / 8.16 m |
| XY compensation, frame: hole -0.1, outline -0.08 | Outer / inner wall | 8.35 m / 8.07 m | 8.35 m / 8.07 m |
| XY compensation, frame: hole +0.15, outline -0.1 | Outer / inner wall | 8.41 m / 8.13 m | 8.41 m / 8.13 m |
| `alternate_extra_wall`, cube, either generator | Inner wall | 10.96 m | 10.96 m |

And by what is laid which way and when: on a frame (a part with a
hole) all 60 outer and 58 inner loops are counter-clockwise in both
slicers' files, and all clockwise with `wall_direction` at `cw`, where
this engine used to lay the walls round a hole clockwise; on a cube
with four walls and `inner-outer-inner wall` the first layer and the
sixth lay their walls in the same order in both, with either
generator; on the mushroom with `overhang_reverse`, a threshold of 0
and the outer wall left alone, the inner wall of every odd layer is
clockwise in both, and at the default threshold no wall is, in both.

The hole options, on a part made for them (`tools/make_fixture.py`,
`holes`: a slab with round holes 2, 3, 5, 8 and 12 mm across, each in
64 sides, a hole that is nearly round, one of eight sides, a round boss,
a round tube, and a hole from front to back) with the U1's presets, 80
layers. Each of the 300 outer walls round a hole or a boss was found in
both files and measured from the middle it goes round: how many sides
it has, how far out its corners are, how far out its flats are, and
which way its first corner lies. The most any of them differs, and the
walls' length over the whole part (C++ slicer / this engine):

| Options | Sides | Corners | Flats | Turned | Outer wall, m | Inner wall, m |
|---|---|---|---|---|---|---|
| `hole_to_polyhole` | the same | under 0.0001 mm | 0.0009 mm | 0.03 deg | 16.53 / 16.53 | 16.42 / 16.42 |
| and not twisted | the same | under 0.0001 mm | 0.0004 mm | 0.03 deg | 16.53 / 16.53 | 16.42 / 16.42 |
| and 8 sides at most | the same | 0.0002 mm | 0.0013 mm | 0.03 deg | 16.57 / 16.63 | 16.54 / 16.54 |
| and a margin of 5% | the same | 0.0002 mm | 0.0009 mm | 0.02 deg | 16.62 / 16.62 | 16.51 / 16.51 |
| and XY compensation (hole +0.1, outline -0.05) | the same | 0.0002 mm | 0.0011 mm | 0.01 deg | 16.65 / 16.65 | 16.53 / 16.53 |
| and elephant foot over 3 layers | the same | under 0.0001 mm | 0.0009 mm | 0.03 deg | 16.52 / 16.52 | 16.42 / 16.42 |
| `enable_circle_compensation` | the same | 0.0002 mm | 0.0003 mm | | 16.59 / 16.59 | 16.44 / 16.44 |
| and a manual offset of 0.1 | the same | under 0.0001 mm | 0.0003 mm | | 16.65 / 16.65 | 16.50 / 16.50 |
| and a manual offset of -0.2 | the same | 0.0004 mm | 0.0003 mm | | 16.47 / 16.47 | 16.33 / 16.33 |
| and XY compensation | the same | under 0.0001 mm | 0.0003 mm | | 16.70 / 16.70 | 16.54 / 16.54 |
| both options | the same | 0.0001 mm | 0.0009 mm | 0.02 deg | 16.73 / 16.73 | 16.60 / 16.60 |

The polygons have 4, 6, 10, 16 and 24 sides for the five holes and 8
for the tube's (5, 7, 11, 17, 25 and 9 on the first layer, whose holes
elephant foot compensation has opened by 0.15 mm), turned 0, 2, 4, 1
and 3 fifths of a side on five layers running; the nearly round hole,
the eight-sided one and the hole from front to back are as drawn in
both. (The circle compensation rows are with
`apply_scarf_seam_on_circles` off. With it on, as it is unless turned
off, the outer walls come to 20.97 m in both and the inner walls to
19.78 m in the reference's file and 19.79 m here: the scarf seams lay
the start of each such loop twice. The loops themselves are the same.)
With eight sides at most the 0.06 m is
overhang: the corners of a 12 mm hole in eight sides stand out over the
layer beneath as it turns, and the two slicers cut a wall into overhang
and wall differently.

With the walls at 60 and 80 mm/s and the circle compensation on, the
walls round the round holes and bosses are written at 162 mm/s in the
reference's file and 159 here (the compensation speed of 200 mm/s, held
by the hot end's flow; the 3 mm/s between them is on every wall of this
part that goes at full speed, with no option on as well), and the wall
round the nearly round hole at 60 in both.
With the coefficients a filament ships with (a project of Bambu's
presets for an H2S and its PLA Basic, start G-code replaced, arcs off)
all 300 loops agree to 0.0004 mm in size and to the mm/s in speed. With
walls of changing width the corners agree to 0.0005 mm under the circle
compensation and to 0.013 mm as polyholes, the loops being cut into
points differently, as they are with no option on.

Two things do not match. At a margin of nothing
(`hole_to_polyhole_threshold = 0`, which both slicers take as a
ten-thousandth of a millimetre) this engine converts the tube's hole
below the tube and the reference does not: whether its points keep to
one distance within 0.0002 mm is decided by rounding that the two do not
share. And with both options off the same walls are up to 0.17 mm from
the reference's at their corners, on 173 of the 300 loops: that is the
kinks closing leaves in an outline cut with the crossings in it (Status,
above), which the hole options are rid of and the plain cut is not.

Both options off, the G-code is byte for byte what it was before they
were followed: on this part, the cube, frame, steps, mushroom and fins,
two blocks in two filaments, 3DBenchy and both plates of
`mcbrim_test.3mf`. With both on, the parts made of boxes are line for
line what they are with both off.

Scarf seams, 2026-11-11, on parts from `tools/make_fixture.py` with the
U1's presets (classic walls, a seam gap of 0.06 mm) but where the
Centauri Carbon's are named, which have arc fitting on and a seam gap
of 0.04 mm. `funnel` is new for this: a round post that widens as it
rises, each 0.2 mm layer lying 0.06, 0.12, 0.18 and at the top 0.24 mm
outside the one beneath. Lengths are the C++ slicer's and this
engine's; a way up or down is a run of moves that say a height as they
lay (two to a joint), counted over the whole file and paired by kind
and length, and the last column is the most any pair differs by.

| Options, part | Outer wall, m | Inner wall, m | Ways up and down | Furthest apart: length, height |
|---|---|---|---|---|
| `external`, cylinder | 4.04 / 4.04 | 2.89 / 2.89 | 98 / 98 | 0.001 mm, 0.004 mm |
| `external`, tube | 5.29 / 5.29 | 4.41 / 4.41 | 78 / 78 | 0.002 mm, 0.004 mm |
| `all`, tube | 6.06 / 6.06 | 4.41 / 4.41 | 156 / 156 | 0.001 mm, 0.004 mm |
| `all` and inner walls, tube | 6.06 / 6.06 | 5.31 / 5.31 | 308 / 308 | 0.001 mm, 0.004 mm |
| and outer wall first | 6.06 / 6.06 | 5.31 / 5.31 | 308 / 308 | 0.001 mm, 0.004 mm |
| and walls clockwise | 6.06 / 6.06 | 5.31 / 5.31 | 308 / 308 | 0.001 mm, 0.005 mm |
| and seams at the nearest point | 6.06 / 6.06 | 5.31 / 5.31 | 308 / 308 | 0.002 mm, 0.002 mm |
| and `arachne` | 6.06 / 6.06 | 5.31 / 5.31 | 308 / 308 | 0.001 mm, 0.006 mm |
| and the whole loop | 8.91 / 8.91 | 8.69 / 8.69 | 308 / 308 | |
| `external`, cube | 9.79 / 9.79 | 7.44 / 7.44 | 198 / 198 | none, none |
| and `arachne` | 9.79 / 9.79 | 7.44 / 7.44 | 198 / 198 | none, none |
| and 40 steps | 9.79 / 9.79 | 7.44 / 7.44 | 198 / 198 | none, none |
| and conditional | 7.82 / 7.82 | 7.44 / 7.44 | 0 / 0 | |
| `external` and conditional, cylinder | 4.04 / 4.04 | 2.89 / 2.89 | 98 / 98 | 0.001 mm, 0.004 mm |
| `external` and the whole loop, cylinder | 6.07 / 6.08 | 2.89 / 2.89 | 98 / 98 | |
| started at 50%, or at 0.1 mm, cylinder | 4.04 / 4.04 | 2.89 / 2.89 | 98 / 98 | 0.001 mm, 0.002 mm |
| 10 mm in 3 steps, cylinder | 3.55 / 3.55 | 2.89 / 2.89 | 98 / 98 | 0.001 mm, 0.010 mm |
| at 20 mm/s with 0.9 of the flow, cylinder | 4.04 / 4.04 | 2.89 / 2.89 | 98 / 98 | 0.001 mm, 0.004 mm |
| at 50% and inner walls, cylinder | 4.04 / 4.04 | 3.46 / 3.46 | 194 / 194 | 0.001 mm, 0.004 mm |
| `seam_position` `random`, cylinder | 4.04 / 4.04 | 2.89 / 2.89 | 98 / 98 | 0.001 mm, 0.007 mm |
| `all`, steps | 6.53 / 6.53 | 5.00 / 5.00 | 126 / 126 | none, 0.007 mm |
| `external`, mushroom | 5.51 / 5.51 | 3.83 / 3.83 | 136 / 136 | none, none |
| and conditional | 4.16 / 4.16 | 3.83 / 3.83 | 0 / 0 | |
| `external`, funnel | 3.36 / 3.36 | 2.19 / 2.19 | 98 / 98 | 0.001 mm, 0.005 mm |
| and conditional | 3.16 / 3.16 | 2.19 / 2.19 | 78 / 78 | 0.001 mm, 0.005 mm |
| with a threshold of 20% | 2.96 / 2.96 | 2.19 / 2.19 | 58 / 58 | 0.001 mm, 0.005 mm |
| with a threshold of 0 | 3.36 / 3.36 | 2.19 / 2.19 | 98 / 98 | 0.001 mm, 0.005 mm |
| and inner walls | 3.16 / 3.16 | 2.66 / 2.66 | 156 / 156 | 0.002 mm, 0.012 mm |
| `all`, seams at the back, holes | 23.06 / 23.07 | 16.22 / 16.23 | 792 / 792 | |
| `enable_circle_compensation`, holes | 20.97 / 20.97 | 19.78 / 19.79 | 1034 / 1034 | |
| and no scarf on circles | 16.59 / 16.59 | 16.44 / 16.44 | 0 / 0 | |
| arcs: `all`, curve | 3.66 / 3.66 | 2.90 / 2.91 | 58 / 58 | 0.001 mm, 0.002 mm |
| arcs: `all` and inner walls, tube | 6.07 / 6.07 | 5.32 / 5.33 | 308 / 308 | 0.002 mm, 0.012 mm |
| arcs: `external` and the whole loop, cylinder | 6.08 / 6.08 | 2.89 / 2.89 | 98 / 98 | |
| arcs: `all`, inner walls, `arachne`, curve | 3.65 / 3.65 | 3.24 / 3.24 | 114 / 114 | 0.001 mm, 0.005 mm |
| arcs: `enable_circle_compensation`, holes | 20.99 / 20.99 | 19.80 / 19.81 | 1034 / 1034 | |

Where the last column is empty the pairs cannot be told apart by
length: a slope the length of its loop, or a layer of many loops whose
seams the two slicers put in different places. The same loops are
joined in every row, and on the cube, whose loops are the same points
in both files, the joint is the reference's move for move: on the layer
at 5.05 mm the way up is 17 moves, 16 of 1.224 mm and a last of
0.42 mm, from Z 4.861 to 5.05, feeding 0.00188, 0.00440, 0.00672 mm
and so on to 0.03681, and the way down 17 moves feeding from 0.03529 mm
to 0.00003, the same figures in both to the last place. On the round parts the reference's walls have
half as many corners (its sides are 0.96 mm, this engine's 0.48), so a
way up has 21 moves there and 42 here, and since each move is laid at
the flow its far end is to have, the way up comes to 0.3200 mm of
filament there and 0.3127 here and the way down to 0.2918 and 0.2990:
0.6118 for the joint in both. Height along the slope agrees to the
0.01 mm of a move's difference in length. The joint's feed rate is the
wall's in both, with what a layer is slowed by to cool (which the two
reckon a few percent apart, as with no scarf); at a `scarf_joint_speed`
of 20 mm/s every way up and down is at F1200 in both files, and at 50%
the wipe after an outer wall is at F6000 in both. On the funnel a
conditional scarf stops where the wall leans out 0.24 mm a layer at the
default threshold and where it leans 0.18 mm at 20%, on the same layers
in both, and on the mushroom the cap's first layer, which starts over
air, has none in either. Where a slope crosses a stretch over air
(`holes`, seams at the back, over the hole that runs from front to
back) both files climb through it as an overhang wall at the bridge's
speed, 20 such moves in each.

The reference's own file, timed here as the reference plans it, comes
to within a second of the reference's own time on all 50 files (a move that
climbs as it lays was already read as the move it is: its length is
the slope's, and the Z axis's limits hold it). With `seam_slope_type` at
`none` and no compensated circles the G-code is byte for byte what it
was before any of this, on these parts and on 16 projects besides (the
two these take their settings from and 14 of the reference's test
data), and a file is the same on one thread as on all of them. Every extruding move in the 50 files is at its
layer's height or, on a scarf, between the layer's bottom and its top.

Three rows are left out of the table because they differ on purpose
(Status, above). With `seam_slope_min_length` at 0, which the Centauri
Carbon's presets have, the reference closes each loop of the cylinder
with no gap at all, and here it stops a seam gap short as with no
scarf. With `seam_gap` at 0 the reference starts an inner wall's way up
at the seam, 5.89 m of inner wall on the tube, where here it starts two
fifths of the slope on as it does at any other gap, 5.28 m (5.31 m in
both at 0.06 mm). And with `small_perimeter_threshold` at 3 mm,
`small_perimeter_speed` at 60 mm/s and `scarf_joint_speed` at 20, the
reference lays the joints of the `holes` part's small loops at 60 mm/s
and this engine at 20: its file comes to 37m 33s and this engine's to
43m 9s (23m 32s and 24m 59s with no scarf). An inner wall's joint is a
fifth short of plastic in both: its way up, cut to three fifths, still
climbs from nothing, and the way down over it thins out over the whole
slope (on the tube with arcs, 0.203 and 0.311 mm of filament against
0.643 for 20 mm of line); that is how the reference has it, and it is
not changed here.

Where seams are put, 2026-11-11, on twelve parts from
`tools/make_fixture.py` with the U1's presets (cube, cylinder, tube,
curve, holes, funnel, frame, fins, comb, steps, mushroom, twotone), each
sliced with `seam_position` at `aligned`, `aligned_back`, `nearest`,
`back` and `random`, and on the Benchy and the three plates of
`mcbrim_test.3mf` as they are (`aligned`). A script pairs every outer
wall loop of the reference's file with this engine's on the same layer
by the middle and size of its bounds and takes how far apart the two
start. The share of loops whose seams are within 0.05 mm and within
1 mm of the reference's, by stage of the work:

| | loops | before | overhangs | comparators |
| --- | --- | --- | --- | --- |
| `aligned` | 1021 | 2.6%, 12.3% | 3.2%, 12.9% | 14.8%, 25.7% |
| `aligned_back` | 1011 | 13.2%, 31.1% | 13.5%, 31.4% | 18.0%, 41.5% |
| `back` | 1011 | 69.8%, 91.6% | 70.1%, 91.9% | 71.2%, 87.4% |
| `nearest` | 1019 | 14.6%, 22.7% | 15.0%, 23.1% | 16.6%, 24.7% |
| `random` | 1016 | 0.0%, 7.4% | 0.1%, 7.4% | 23.2%, 30.0% |
| Benchy | 745 | 35.1%, 41.7% | 43.1%, 52.8% | 24.6%, 31.7% |
| every loop, with the three plates | 6235 | 20.6%, 31.9% | 21.9%, 33.5% | 26.6%, 38.3% |

Overhangs (a seam kept off what hangs out over the layer beneath):
the Benchy gains most, its worst seam 34 mm from the reference's where
it was 54; on the `holes` part 4 to 6 loops more are within 0.05 mm at
each setting, those beside the hole that runs from front to back as it
closes over; nothing else moves but two or three `random` seams round
holes, which are now picked going round the way the reference goes.
With scarf seams on (`seam_slope_type=all`, `seam_slope_inner_walls=1`)
the `holes` part's layer at 10.45 mm was 8.4% short of the reference's
wall: each side of the closing hole is an island 6.7 by 15.6 mm whose
side by the hole is an overhang wall, the reference's seams are at the
corners away from it and this engine's were at the corners by it, and a
loop that starts over air has no scarf in either slicer. Both seams are
now where the reference's are, to the micron, and wall length per layer
is off by 0.03% on average (0.14% before), the most 0.6% on the first
layer as with no scarf. Travel and retractions on the Benchy: 26.96 m
and 1520 in the reference's file, 27.78 m and 1685 here before, 26.24 m
and 1746 now.

Comparators (the reference's own way of telling two points apart, its
angles, points hidden where two materials meet, each object measured
from its own middle, walls counted from where the reference's start,
and inner walls started as it starts them): the cube at `aligned` goes
from none of its 100 seams to all of them, every corner of a box being
as good as another and the first then taken; `random` from none within
0.05 mm to 23%, 48% on the cube and 50% on the frame. A random seam is
put by the sine of a figure made from its wall's first point to the
nanometre, times 43758, so two slicers whose walls differ in the last
digit scatter their seams differently, and that is all the rest is:
on round walls the two never agree. The Benchy loses what it gained,
24.6% from 43.1%: with nothing known yet of what can be seen from
where, the best angle takes every seam, where the reference weighs
visibility beside it and the simpler rule before happened to follow
it more often. At the back the round parts (cylinder, tube, funnel)
are within 1.7 mm where they were within 1 mm: their seams follow the
one beneath from corner to corner, with no curve yet to draw them
straight. What is left is the reference's visibility measure, which
is what puts most `aligned` and `nearest` seams where they are, and
its lining up of seams along a curve.

Moves at the seam (`wipe_on_loops`, `wipe_before_external_loop`,
`wipe_inward`), 2026-11-11, on parts from `tools/make_fixture.py` with
the U1's presets (classic walls, inner wall first, two walls, a seam gap
of 0.06 mm, a 2 mm wipe with none of the 1.5 mm retraction made before
it) but where the Centauri Carbon's are named, which have arc fitting
on, a seam gap of 0.04 mm and a 1.2 mm wipe. Neither printer's presets
turn any of the three on. Figures are the C++ slicer's and this
engine's. A script goes through both files and takes, for every outer
wall loop, the moves between the travel to it and its first line and
between its last line and the next: the move it is come at with, the
move in it ends with, and its wipe. "All" is the three options
together.

| Options, part | Outer loops | Come at from inside | End with a move in: loops, mean mm | Wiped: loops, mean mm | Wipe ends this far off the wall, mm | Retractions |
|---|---|---|---|---|---|---|
| none, cube | 100 / 100 | 0 / 0 | 0 / 0 | 99 / 100, 2.000 / 2.000 | 0.054 / 0.054 | 200 / 201 |
| `wipe_on_loops` | 100 / 100 | 0 / 0 | 100 / 100, 0.072 / 0.072 | 99 / 100, 2.000 / 2.000 | 0.036 / 0.036 | 200 / 201 |
| `wipe_before_external_loop` | 100 / 100 | 99 / 99 | 0 / 0 | 99 / 100, 2.000 / 2.000 | 0.054 / 0.054 | 200 / 201 |
| `wipe_inward` | 100 / 100 | 0 / 0 | 0 / 0 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 198 / 201 |
| at 100% | 100 / 100 | 0 / 0 | 0 / 0 | 99 / 100, 2.000 / 2.000 | 0.421 / 0.417 | 198 / 200 |
| at 0.1 mm | 100 / 100 | 0 / 0 | 0 / 0 | 99 / 100, 2.000 / 2.000 | 0.100 / 0.100 | 198 / 201 |
| all | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 198 / 201 |
| all, outer wall first | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 0 / 1 | | 102 / 102 |
| all, inner-outer-inner, three walls | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 200 / 201 |
| all, walls clockwise | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 199 / 195 |
| all, `arachne` | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 198 / 201 |
| all, scarf seams on all | 100 / 100 | 99 / 99 | 100 / 100, 0.177 / 0.177 | 100 / 99, 2.000 / 2.000 | 0.209 / 0.210 | 201 / 200 |
| all, `wipe` off, or `wipe_distance` 0 | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 0 / 0 | | 200 / 201 |
| all, `wipe_distance` 8 | 100 / 100 | 99 / 99 | 100 / 100, 0.072 / 0.072 | 99 / 100, 8.000 / 8.000 | 0.210 / 0.209 | 106 / 101 |
| all, `seam_gap` 0 | 100 / 100 | 99 / 99 | 100 / 100, 0.080 / 0.080 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 198 / 201 |
| all, `seam_gap` 0.5 mm | 100 / 100 | 99 / 99 | 100 / 100, 0.465 / 0.465 | 99 / 100, 2.000 / 2.000 | 0.210 / 0.209 | 198 / 201 |
| all, Centauri Carbon | 100 / 100 | 99 / 99 | 100 / 100, 0.069 / 0.069 | 100 / 100, 1.200 / 1.200 | 0.209 / 0.209 | 202 / 201 |
| all, frame | 60 / 60 | 58 / 58 | 60 / 60, 0.130 / 0.106 | 60 / 60, 2.000 / 2.000 | 0.207 / 0.206 | 100 / 94 |
| all, steps | 65 / 65 | 61 / 62 | 65 / 65, 0.099 / 0.088 | 65 / 65, 2.000 / 2.000 | 0.201 / 0.204 | 118 / 124 |
| all, cylinder | 50 / 50 | 49 / 49 | 50 / 50, 0.119 / 0.121 | 50 / 50, 2.000 / 2.001 | 0.207 / 0.206 | 109 / 118 |
| all, scarf seams, cylinder | 50 / 50 | 49 / 49 | 50 / 50, 0.170 / 0.173 | 50 / 50, 2.000 / 2.000 | 0.206 / 0.207 | 109 / 123 |
| all, tube | 80 / 80 | 78 / 77 | 80 / 80, 0.122 / 0.122 | 80 / 80, 2.000 / 1.976 | 0.206 / 0.205 | 156 / 165 |
| all, outer wall first, tube | 80 / 80 | 78 / 77 | 80 / 80, 0.122 / 0.122 | 2 / 1 | | 120 / 122 |
| all, walls clockwise, tube | 80 / 80 | 78 / 78 | 80 / 80, 0.122 / 0.122 | 80 / 80, 2.000 / 2.000 | 0.206 / 0.206 | 141 / 164 |
| all, `arachne`, tube | 80 / 80 | 78 / 78 | 80 / 80, 0.122 / 0.122 | 80 / 80, 2.000 / 2.000 | 0.206 / 0.206 | 154 / 158 |
| all, scarf seams on all, tube | 80 / 80 | 78 / 77 | 80 / 80, 0.174 / 0.174 | 80 / 80, 2.000 / 1.976 | 0.206 / 0.205 | 158 / 159 |
| all, `wipe_distance` 8, tube | 80 / 80 | 78 / 77 | 80 / 80, 0.122 / 0.122 | 80 / 80, 8.000 / 7.901 | 0.206 / 0.205 | 121 / 113 |
| all, holes | 402 / 407 | 390 / 391 | 402 / 406, 0.116 / 0.111 | 402 / 402, 2.000 / 1.953 | 0.205 / 0.226 | 736 / 760 |
| all, `arachne`, holes | 402 / 402 | 390 / 388 | 402 / 401, 0.116 / 0.112 | 402 / 402, 2.000 / 2.000 | 0.205 / 0.205 | 722 / 723 |
| all, Centauri Carbon, cylinder | 50 / 50 | 49 / 49 | 50 / 50, 0.104 / 0.105 | 50 / 50, 1.200 / 1.200 | 0.207 / 0.212 | 108 / 108 |
| all, Centauri Carbon, tube | 80 / 80 | 78 / 68 | 80 / 80, 0.106 / 0.102 | 80 / 80, 1.200 / 1.091 | 0.205 / 0.202 | 172 / 149 |
| all, arcs, tube | 80 / 80 | 78 / 71 | 80 / 80, 0.122 / 0.118 | 80 / 80, 2.000 / 1.881 | 0.206 / 0.201 | 154 / 163 |

The two slicers put the cube's seam on different corners, so its loops
are paired by what a corner is like, not where it is: a move is
measured in its own loop's frame, along the way the loop sets off and
across it. In every cube row all 100 loops pair, and the move a loop
is come at with (0.400 mm, 30 degrees in behind its last edge), the
move in (0.072 mm, a third of the corner round), and the wipe (its
length, and how far from the wall it ends) are the same in both files
to the last place written. With `wipe_on_loops` alone the wipe sets
off from the move in and runs back onto the wall, ending 0.036 mm off
it in both; inward it runs 0.21 mm inside the wall, half its width,
0.42 at 100% and 0.10 at 0.1 mm. What is pulled back agrees as it did
before any of this: with `retract_before_wipe` at 0% and at 70%, 1.130
and 0.370 mm before and during the wipe in the reference and 1.123 and
0.377 here (the wipe's speed, the wall's, is F9742 there and F9540
here, as it was), at 100% the whole 1.5 mm before it in both, and
after an inward wipe the whole 1.5 mm in one move in both whatever
that setting. Retraction counts move as they do without the options,
with the seams' places: on the cube the reference's inward wipe ends
within a millimetre of where its first two layers' fill starts, and
those two moves go unretracted.

On round parts the reference's walls have half as many corners, so
no loop pairs point for point; of the tube's 80 loops 51 have seams
that turn alike in the two files, and on those the two moves differ
by at most 0.002 mm and 2 degrees, and the wipe's length and its
distance from the wall by 0.002 mm (68 loops with `arachne`, 61 laid
clockwise, 36 with scarf seams, to the same figures or within 0.3
degrees of them). Where a row's
means differ the seams do: on the frame the reference starts its
outer loop on a straight edge and this engine at a corner, and the 29
loops round the hole, where both start at a corner, are move for move
the same; on `holes` this engine has five outer loops more than the
reference with classic walls, as it had.

Fewer loops are come at from inside, or wiped their whole length, in
three kinds of row, each for the shape of the wall there and not for
these moves. The tube's first layer: this engine's hole has a notch
0.16 mm deep in its inner wall there, the seam is put at it, and the
point to come from has no wall beside it (one loop, wiped 0.12 mm
straight in where the reference wipes 2 mm). And with arc fitting on,
the wall round the tube's hole has a hook 0.05 mm long in it on some
layers, the seam goes to that as the sharpest corner, and a seam that
doubles back is not come at from inside, and most such are wiped by
the one short move in: 10 of 78 loops with the Centauri Carbon's
presets and 7 of 78 with the U1's. The notch and the hook are in the
files from before this work.

The reference's own file, timed here as the reference plans it, comes
to within a second of the reference's own time on all 45 files (a
move that lays nothing was already read as travel, and a wipe that
pulls nothing back as a wipe). With the three options off, as both
printers' presets have them, the G-code is byte for byte what it was
before, on one thread and on seven, on 56 files: seven parts with each
printer's presets as they are, with `arachne`, with scarf seams on all
loops and with the outer wall first.

Two things differ from the reference on purpose (Status, above). On
the frame, the reference comes at the wall round the square hole from
past its corner on all 29 layers that have one, and so does this
engine; but there the point is exactly 0.4 mm from the wall it belongs
to, and the reference only counts a wall nearer than 0.4 mm, so it is
the rounding of that point's coordinates that lets the move be made.
Here the wall counts as beside its own point. And the reference makes
the inward wipe for a filament whose own `wipe` is off, if any other
filament's is on (found by reading; the two-filament fixture's
settings name one filament, and the reference prints it in one).

Fuzzy skin differs from the reference in three ways, on purpose:

- The reference's fuzz is random and comes out differently every time
  it slices. Here each wall's random numbers are seeded from the wall
  itself (its layer's height, its first point, how many points it has),
  so a file is the same on every run and at any thread count, which is
  why the lengths above only agree to a fraction of a percent.
- `voronoi` noise is libnoise's as written. The reference's result
  depends on what compiled it: libnoise's value noise overflows a signed
  integer on purpose, and a compiler that takes that for impossible
  (clang, so the macOS build) drops a mask and returns noise from -3 to
  1 where libnoise documents -1 to 1, pushing walls out by up to three
  times the thickness set. A compiler that wraps gives what this engine
  gives; MSVC is expected to, which has not been checked on Windows. On
  the mushroom the macOS build lays 1.24 m of overhang
  wall with `voronoi` fuzz where this engine lays 0.23 m, as both do
  with any other noise.
- A wall is cut where fuzz starts and stops by plain geometry, not
  through the clipping library the reference uses for it.

Fuzzy skin painted on a model, or set for one part by a modifier, is
not read: the settings fuzz every part alike.

Classic walls are untouched: every file that sets `classic` is byte for
byte what it was.

Ironing. With the U1's presets (lines 0.15 mm apart, 10% flow,
30 mm/s) on parts from `tools/make_fixture.py` (`cube`; `steps`, three
tiers with a top surface on each, the lowest with a hole through it;
`twotone`, in two filaments) and on plate 2 of `mcbrim_test.3mf`, the
lettered plate. C++ slicer against this engine: the layers ironed, the
ironing laid, and the time ironing adds, which is each slicer's file
timed here as the reference plans, less the same slicer's file with
ironing off.

| Part, `ironing_type` | Layers ironed | Ironing, m | Time added |
|---|---|---|---|
| cube, `top` | 1 / 1, the same | 2.561 / 2.561 | 85 s / 85 s |
| cube, `topmost` | 1 / 1, the same | 2.561 / 2.561 | 85 s / 85 s |
| cube, `solid` | 8 / 8, the same | 18.284 / 18.284 | 574 s / 614 s |
| steps, `top` | 3 / 3, the same | 7.251 / 7.251 | 243 s / 243 s |
| steps, `topmost` | 1 / 1, the same | 1.254 / 1.254 | 40 s / 40 s |
| steps, `solid` | 18 / 18, the same | 51.346 / 51.303 | 1601 s / 1724 s |
| twotone, `top` | 2 / 2, the same | 5.148 / 5.148 | 173 s / 173 s |
| twotone, `topmost` | 1 / 1, the same | 2.561 / 2.561 | 86 s / 86 s |
| twotone, `solid` | 13 / 13, the same | 36.827 / 36.855 | 1167 s / 1248 s |
| lettered plate, `top` | 3 / 3, the same | 13.457 / 14.557 | 486 s / 520 s |
| lettered plate, `topmost` | 1 / 1, the same | 2.257 / 2.256 | 88 s / 88 s |
| lettered plate, `solid` | 16 / 16, the same | 136.78 / 143.37 | 4754 s / 5220 s |

On the three made parts the ironing of a layer is the reference's to a
tenth of a millimetre of path on every layer of `top` and `topmost`,
with the lines in the same places and swept from the same corner (on
the cube and the two-colour part the box round a layer's ironing and
the point it starts at agree to 0.001 mm on every layer; on the top
tier of `steps` the first line is laid from its other end, which moves
the turns to the other side and the box by 0.07 mm), at the same width
(0.40 mm), plastic (0.003 mm³ a millimetre), speed (30 mm/s) and
acceleration, and last in its island in both. Every
other feature is what this engine lays without ironing, to the
millimetre and the filament fed. The same holds with the options
changed, on `steps` with `top`: an inset of 0.6 mm with lines 0.1 mm
apart at 15% flow (9.929 m in both), `ironing_angle` at 30 (7.244 m),
that angle fixed (7.252 m), `concentric` (7.138 m, the same loops,
begun at the same corner or, where two corners are as near the middle
of the object, at the other of them), `ironing_speed` at 15 (the time added is 483 s in both),
`top_layer_direction` at 20, and with `only_one_wall_top` off and the
top surface `monotonic`.

Where the two differ, and why:

- A first layer that is ironed (`solid`) is ironed at
  `initial_layer_infill_speed` by the reference, 105 mm/s with the U1's
  presets where ironing is set to 30: its writer gives every line of
  the first layer but a bottom surface's the first layer's speed
  outright, ironing with the rest (GCode.cpp `_extrude`). Here ironing
  never goes faster than its own speed, and on the first layer no
  faster than that layer's fill either. That is the whole of the 7% by
  which `solid` takes longer here on the made parts: with
  `initial_layer_infill_speed` set to 30 the time added is 612 s
  against 614 s on the cube and 1724 s in both on `steps`.
- On two layers of `steps` with `solid` the ironing is 4.7% short of
  the reference's (519 mm against 545 mm, 381 mm against 398 mm): those
  are the layers whose only solid fill is the band under a bridge's
  edge, which this engine draws a little narrower (206 mm of solid
  fill there against 217 mm), and ironing follows the fill.
- Two filaments' shares of a layer meet 0.05 mm from where the
  reference's regions do, so on `twotone` with `solid` one filament
  irons 0.25% more and the other as much less.
- On the lettered plate this engine irons 8% more with `top` and 5%
  more with `solid`, all of it on the layers where parts in different
  filaments lie side by side, and all of it within 0.2 mm of where they
  meet. The reference keeps ironing the inset back from the edge of the
  layer's outline, and its outline has cracks a few micrometres wide
  where two parts of an object do not meet exactly, so it keeps the
  inset back from those as well; this engine closes such cracks when it
  slices (see the painted plate above), and irons up to where the parts
  meet, as the reference itself does where they meet exactly (on
  `twotone`, flush, both iron to the join; with the second block moved
  3 micrometres away the reference stops 0.2 mm short of it, 2561 mm on
  that layer where it laid 2587 mm, and this engine lays 2587 mm). The
  last layer, all one filament, is the reference's to 0.04%.

Not as the reference besides: the fan is set for ironing before the
move to it and given back straight after, where the reference changes
it as the first ironing line begins and as the next line of another
kind does; the file has no `;HEIGHT:` comment under `;TYPE:Ironing`
(this engine writes a layer's height once, at its start); where the
loops of a `concentric` area fall into several nests each is taken
nearest first; `solid_infill_rotate_template` and
`align_infill_direction_to_model` are noted and not followed;
ironing the dense top of a support (`support_ironing`) is noted by the
supports and not done; and the reference's ironing flow calibration
and its dithering of ironing through an image are not here.

Sparse patterns. Nineteen of the reference's twenty-seven are laid:
`grid`, `rectilinear`, `alignedrectilinear`,
`triangles`, `tri-hexagon`, `cubic`, `quartercubic`, `crosshatch` (the
reference's default, and what 130 of the 184 Elegoo process profiles
set), `gyroid`, `honeycomb`, `3dhoneycomb`, `lateral-honeycomb`,
`lateral-lattice`, `zigzag`, `crosszag`, `hilbertcurve`,
`archimedeanchords`, `octagramspiral` and `tpmsd`. The first nine are
every pattern the Snapmaker
and Elegoo profiles in the reference ask for. The six made of straight
lines are the sets of lines FillRectilinear.cpp hands to
`fill_surface_by_multilines`, each set's direction and sideways move
taken from there (cubic and quarter cubic move with the layer's
height); the lateral lattice and lateral honeycomb are two more such
sets, leaning by `lateral_lattice_angle_1` and `_2` and by
`infill_overhang_angle`. `zigzag` and `crosszag` are rectilinear lines
joined only to the row after, from alternate ends row by row, so the
joins fall on the same side layer after layer (`has_consistent_pattern`);
a cross zag's lines move `infill_shift_step` every second layer. The
rest are laid over the area, cut to it, and their ends joined and
anchored along the outline as the crossing lines' are: cross hatch
(FillCrossHatch.cpp), gyroid (`make_gyroid_waves` in FillGyroid.cpp),
honeycomb (FillHoneycomb.cpp), 3D honeycomb (Fill3DHoneycomb.cpp),
TPMS-D (FillTpmsD.cpp) and the three curves that fill a plane with one
line (FillPlanePath.cpp), which are laid over the whole object and are
joined only up to half density, as there. On a 20 mm cube with the U1's
presets, C++ slicer against this engine, sparse fill in metres and how
much of each file's sparse plastic lies within 0.3 mm of the other's:

| Pattern | At 15% | Lies together | Also at |
|---|---|---|---|
| `grid` | 14.51 / 14.36 | 95% | |
| `rectilinear` | 14.06 / 14.06 | | |
| `alignedrectilinear` | 14.06 / 14.06 | | |
| `triangles` | 12.97 / 12.97 | 97% | |
| `tri-hexagon` | 12.97 / 12.97 | | 30%: 24.49 / 24.49, 100% |
| `cubic` | 13.10 / 13.10 | 99.8% | 35%: 28.30 / 28.29, 99.4% |
| `quartercubic` | 13.79 / 13.81 | | 10%: 9.76 / 9.73, 93% |
| `crosshatch` | 14.18 / 14.20 | 98.6% | 40%: 31.90 / 31.72, 99.4% |
| `gyroid` | 13.29 / 13.29 | 99.9% | 5%: 5.37 / 5.37, 99.6% |
| `honeycomb` | 16.81 / 16.89 | 98.5% | |
| `3dhoneycomb` | 12.62 / 12.69 | 94% | |
| `lateral-honeycomb` | 14.52 / 14.56 | 94% | |
| `lateral-lattice` | 13.84 / 13.93 | 97% | |
| `zigzag` | 14.06 / 14.06 | 99.9% | |
| `crosszag` | 14.11 / 14.11 | 99.9% | |
| `hilbertcurve` | 13.59 / 13.59 | 98% | |
| `archimedeanchords` | 14.16 / 14.16 | 99.8% | |
| `octagramspiral` | 14.21 / 14.21 | 98% | |
| `tpmsd` | 14.41 / 14.42 | 92% | |

Where less than everything lies together the pattern's own lines are
in the same places, checked line by line; what differs is which wall a
join or an anchor runs along (for `tpmsd`, `3dhoneycomb` and
`lateral-honeycomb` that is most of the difference there is). On plate
2 of `mcbrim_test.3mf`, four
objects of several parts each, cross hatch, gyroid, cubic and grid lie
98% together and come to within 1% of the reference's length (they lay
45 to 50% together before each object had its own middle), and
honeycomb, 3D honeycomb, the Hilbert curve and the lateral lattice
come to within 0.5% of its length. `rectilinear`, `zigzag` and
`crosszag` are 1%, 3.7% and 0.6% from it there, and lie only 77 to 79%
together: their lines are at the reference's offsets, but in areas as
small as a letter's the runs along the walls that join them are much of
the plastic, and they are not always taken along the same wall.

Not laid, and printed as a grid with a note: `lockedzag`, `tpmsfk`,
`concentric`, `spiralinset`, `adaptivecubic`, `supportcubic` and
`lightning`. `fill_multiline` above one,
`gyroid_optimized`, `sparse_infill_rotate_template`,
`infill_complete_top` (the closed tops of 3D honeycomb cells) and
`symmetric_infill_y_axis` (a zigzag mirrored about the object's
middle) are noted and
not followed, and `line` is noted as laid like `rectilinear`; the
adaptive forms of the gyroid and corner smoothing are not looked at.

Tree supports now join into trunks as soon as the reference's do. On
the `mushroom` with the U1's presets there are as many separate outlines
as in the reference all the way down (38 against 37 thirty layers up,
23 against 24 at twenty), where there were a third more; the file has
1687 retractions against the reference's 1739 (it had 2228) and is
timed at 19m 26s against 19m 42s (it was 21m 15s). Layer by layer its
support is within 10% of the reference's from the bed to two thirds of
the way up, and 15 to 35% under in the last ten layers below the roof,
where the reference's tips are drawn fatter and lean more than a branch
may here. With the Centauri Carbon's presets (`tree(auto)`, arcs on)
the `shelf` fixture is still 30% under (3.08 m against 4.37 m; it was
2.26 m).

Arcs, 2026-11-11, on three round parts from `tools/make_fixture.py`
(`cylinder`, `tube`, `curve`) with the Centauri Carbon's presets, which
have arc fitting on. The length of every feature is what it was as
straight moves, to the rounding of the table, and the spiral lifts are
as many (49, 39 and 29). Fill agrees arc for arc where the two engines
fill alike: sparse infill 252 arcs against the reference's 252 on the
cylinder and 380 against 382 on the tube, the bottom surface 22 against
22 and 64 against 63, and each of the cylinder's layers of solid fill 27
against 27. That took one correction besides the
arcs: with arc fitting on, what is left for infill was outlined at a
fifth of `resolution` like the walls, where the reference outlines it at
`resolution` itself, so the fill's turns along a round wall had twice
the corners and every one of them became an arc. Walls are arcs for the
same share of their length (98 to 100% on the round parts, 56% on the
`curve`) but in more pieces on the round parts: 277 against 198 on the
cylinder, 404 against 278 on the tube, 303 against 325 on the `curve`.
On the `curve` the arcs are the reference's own, centre for centre (the
80 mm bow, the half-round end, the 1.5 mm corners), bar one: on every
layer the reference has the outer wall's half-round end in two arcs,
the first stopping two corners short of the end, which is where its
mistake with a middle point due east stops an arc. On the cylinder and the tube two things
upstream of the fitter make the difference. A sliced ring here keeps one
point that is no corner, where its outline starts, between two corners
of the mesh, and loses the corner next to it; an arc cannot be carried
round past it, so a ring is two arcs, or three, before its seam cuts
one of them. (With that point taken out, in a trial, the cylinder's
walls came to 117 arcs.) And the seam of a round wall is put at the
ring's start here and a third of the way round it in the reference, so
it cuts a different arc. Tree supports are arcs for 94% of their length
against 90% (1825 against 2719 on the `shelf`, whose supports are 30%
shorter here, as above). On the box-shaped parts nothing changed but
the spiral lifts, and four arcs on the `mushroom` where a support's
outline turns a corner; the reference has five to eight arcs there that
this engine has not, each a dot of narrow solid fill it lays as a loop
(`detect_narrow_internal_solid_infill`, not done).

The reference still does more in that time (time planning, ironing out
its fill order), so the speed gap will narrow as stages are added;
measuring overhangs took this engine from seven times faster to five on
the cylinder. Arcs are fitted a stretch of layers at a time on every
thread, ahead of the writer. On the `tube` fixture with the Centauri
Carbon's presets, whose round walls are the fitter's worst case (128
sides, each just too far from the circle for its first guess to hold),
the G-code is written in 14 ms where it took 19 ms as straight moves,
and in 77 ms on one thread; the `curve` and `shelf` fixtures take 5 and
9 ms on any number (9 and 12 ms before).
Closed since the first run: elephant-foot compensation, one wall on the
last layer, fill lines joined along the walls, seam gaps, wipe, ramped and
spiral lifts, and plate placement (outer walls land where the reference
puts them on all three plates of `mcbrim_test.3mf`), the prime tower, gap
fill between walls, overhang walls with their speeds and fan, and
internal bridges, one wall under top surfaces below the last layer,
vertical shells, the solid band under a bridge's edge, and the prime
tower's ribs, stepped brim, ramming and the gap in its wall, the
order tools are taken in, the order within a layer with where a
move may go unretracted, and the monotonic order of surface lines.

The first layer of the lettered plates, in millimetres of path (C++
slicer, this engine while it shrank wide parts by the whole
compensation and narrow ones not at all, this engine now), plate 2
with plate 3 in brackets where it differs:

| First layer | C++ slicer | before | now |
|---|---|---|---|
| Outer wall | 1371.0 | 1391.4 (1391.7) | 1372.6 (1372.4) |
| Inner wall | 1313.5 | 1323.2 (1324.5) | 1313.6 |
| Gap infill | 52.3 | 151.1 (131.8) | 55.4 (55.1) |
| Bottom surface | 4388.7 | 4458.4 (4456.9) | 4416.5 (4416.8) |
| Brim | 7489.0 | 7718.2 (7730.7) | 7718.2 (7730.7) |
| Support | 219.2 | 416.1 (418.2) | 415.1 (418.6) |

The outer wall now lies as far from the reference's on the first layer
as on the second, which is not shrunk (0.024 mm on average on both,
0.057 mm on the first before). The brim does not go by the shrunk
outline in either slicer (`brim_use_efc_outline` is off; turned on, it
is not followed here yet) and has not moved: all of its 229 mm over is
in the four notches at the ends of the two blades, where a support
stands on the bed and the reference lays no brim under it, and away
from those it is within 0.2%. That and the support's own length are
the first layer's supports, not its outline.

Retractions on the lettered plates were 41% over the reference's and
are now within 2% (plate 2: 854 against 848, from 1199; plate 3: 856
against 846), with 15% and 12% less travel than the reference. Four
things did it: an island is finished, walls and fill, before the next
is begun, where every island's walls used to be laid first and each
kind of fill then took another trip round the layer; moves between
fill lines are sheltered over the whole inside of the part, not just
inside its walls; solid lines round a curved outline stay one zigzag
(the body of a blade's first layer was in ten pieces and is in one, as
in the reference); and each part's brim is laid whole. On the plain
parts the count moved by four at most and is within 1% of the
reference's on the cube, the tower and the bridge with either
printer's presets and on the shelf with the U1's. What is left on plate 2 is
supports (a support's lines are not sheltered, and there is twice the
support the reference lays under these parts), and seams: where an
outer wall's seam sits on a sharp tip the inner wall ends 1.7 mm from
it and the move retracts, and the reference puts those seams elsewhere.

Chaining fill and supports by their closest ends, as the reference
does, took 2 m of travel and 38 retractions off the painted plate 1
(57.30 m and 1703, from 59.25 m and 1741; the reference has 64.44 m
and 2188) and 0.3 m of travel off plates 2 and 3, whose retractions
moved by two at most. The mushroom with tree supports has 14 more
retractions (1722 against the reference's 1741) and 0.1 m less travel.
The plain parts are what they were.

The bridging options, each turned on against the C++ slicer with the
same option on, in metres of path (C++ slicer / this engine). The parts
are `tools/make_fixture.py`'s with the Snapmaker U1's presets
(`bridge_flow` 0.8, flat bridges, thick bridges over infill, 15% grid,
two walls); `gaps` is a base with five bars across it 1, 2, 3 and 5 mm
apart, so that the solid layers under the strips of top between them
are small bridges over infill of different sizes. With every option
off bridges are 2 to 6% over the reference's and bridges over infill
within 5% of them, as above, but 26% under on `gaps`, whose bridges
are small; that is the difference each row starts from. Besides the
features shown only the solid and sparse fill a bridge takes the place
of moves, in either slicer, and with the options off the file is byte
for byte what it was.

| Option | Part | Feature | Off | On |
| --- | --- | --- | --- | --- |
| `bridge_line_width` 0.3 | `bridge` | Bridge | 0.67 / 0.70 | 1.05 / 1.05 |
| | | Bridge over infill | 1.01 / 1.06 | 1.30 / 1.35 |
| | `shelf` | Bridge | 0.42 / 0.45 | 0.66 / 0.67 |
| `bridge_expansion_extra` 2, 6, 50 | `bridge`, all solid (`top_shell_layers=16`) | Bridge | 0.69 / 0.73 | 0.77 / 0.82, 0.94 / 0.99, 1.02 / 1.07 |
| `bridge_expansion_extra` 3 | `mushroom`, all solid | Bridge | 2.02 / 2.06 | 2.14 / 2.19 |
| `dont_filter_internal_bridges` `limited` | `gaps` | Bridge over infill | 2.58 / 1.91 | 2.58 / 2.04 |
| `dont_filter_internal_bridges` `nofilter` | `gaps` | Bridge over infill | 2.58 / 1.91 | 2.88 / 2.62 |
| | a wedge, top at 14° | Bridge over infill | 0.00 / 0.09 | 5.61 / 3.88 |
| `enable_extra_bridge_layer` `external_bridge_only` | `bridge` | Bridge | 0.67 / 0.70 | 1.33 / 1.41 |
| | `mushroom` | Bridge | 1.97 / 2.02 | 3.94 / 4.03 |
| `enable_extra_bridge_layer` `internal_bridge_only` | `cube` | Bridge over infill | 0.84 / 0.87 | 1.68 / 1.74 |
| | `gaps` | Bridge over infill | 2.58 / 1.91 | 5.14 / 3.82 |

With `bridge_line_width` the width and the plastic per millimetre are
the reference's for all three features (a 0.3 mm width with these
presets: bridges and walls over air 0.25 mm wide and 0.040 mm³/mm,
bridges over infill a 0.27 mm thread at 0.055). With
`bridge_expansion_extra` the solid fill gives up what the bridge gains
in both (on the deck's first layer, with 0, 6 and 50 mm set, 351, 68
and 0 mm of solid lines in the reference and 353, 83 and 0 here), and
on the plain `bridge`, where
sparse fill is what lies beside the bridge, neither slicer's toolpaths
change at all. A second layer over a bridge over air is the first
layer's length again in both, on the next layer, laid the same way,
with the bottom shell one layer higher; over infill it is laid square
to the first in both. There the reference drops from the layer
whatever of a surface it cuts is narrower than two lines and prints
nothing in its place (an error in it, not copied: going by the lengths
of line laid, 7 to 21 mm² less is filled on each such layer of a
sloping part than with the option off); here everything that is not
second bridge stays what it was.

What is still off is `dont_filter_internal_bridges`. With no filter
the bridges appear on the same layers as the reference's (every second
layer under a sloping top: a bridge is not laid over one just beneath
it), but each is about 70% of the reference's length, 170 mm a layer
against 240 mm on the wedge, because the reference then runs each
patch out to the sparse lines that carry it and this engine does not,
for any bridge over infill. For the same reason `limited` adds a
little here where it adds nothing in the reference: on `gaps` the
reference's bridges on the first solid layer, run out, already cover
what `limited` would find on the next, and here they do not (138 mm
more on one layer). On the plain parts (cube, deck, mushroom) neither
value changes anything in either slicer.

`counterbore_hole_bridging` is not followed: the note stays. In the
reference it is done where walls are made (PerimeterGenerator.cpp
`process_no_bridge`): on the first layer of the narrow part of a hole
the area with nothing under it is taken out of what gets walls and
handed to the fill as it is, so it is bridged, either the part of it a
bridge detector says can be spanned from the wide hole's edge
(`partiallybridge`) or all of it, the narrow hole covered over as well
(`sacrificiallayer`). It needs the reference's `BridgeDetector` (the
direction with most anchored lines and the strips it can span), which
is not here: this engine turns a bridge by the edges it has nothing
beside.

Travel round the walls (`reduce_crossing_wall`), against the C++
slicer with the same settings (C++ slicer / this engine). The parts are
`tools/make_fixture.py`'s with the Snapmaker U1's presets (0.4 mm z-hop,
wipe, `reduce_infill_retraction` on): `frame` and the same with two
skirt loops and a brim, `holes`, `tube`, `steps` and `comb`; then
3DBenchy with those presets and the three plates of `mcbrim_test.3mf`
(plate 1 painted, plates 2 and 3 four objects each in three filaments,
with supports and a prime tower). Each is sliced with the option off,
on, on with `max_travel_detour_distance` 10 mm, and on with it at 50%.
A travel is counted as crossing an outer wall when it goes right
through one on its own layer: within 1.5 mm of travel either side of
the crossing it gets a quarter of a millimetre or more from the wall's
centre line, on opposite sides. Setting off from a wall just laid,
arriving on the one about to be laid, running along a wall and the
spiral of a lift are not crossings.

| Part | | Travel, m | Retractions | Travels across an outer wall |
| --- | --- | --- | --- | --- |
| `frame` | off | 4.27 / 4.07 | 100 / 94 | 62 / 59 |
| | on | 5.62 / 5.21 | 121 / 122 | 3 / 4 |
| | 10 mm | 4.43 / 4.21 | 127 / 123 | 60 / 59 |
| | 50% | 5.47 / 5.07 | 95 / 93 | 3 / 4 |
| `frame`, skirt and brim | off | 4.32 / 4.08 | 102 / 96 | 61 / 58 |
| | on | 5.69 / 5.22 | 123 / 124 | 3 / 3 |
| | 10 mm | 4.47 / 4.22 | 129 / 125 | 60 / 58 |
| | 50% | 5.53 / 5.08 | 97 / 95 | 3 / 3 |
| `holes` | off | 18.72 / 17.81 | 747 / 737 | 394 / 393 |
| | on | 19.46 / 18.46 | 643 / 643 | 102 / 85 |
| | 10 mm | 19.24 / 18.35 | 648 / 647 | 105 / 89 |
| | 50% | 19.24 / 18.19 | 648 / 671 | 104 / 116 |
| `tube` | off | 2.97 / 2.33 | 156 / 166 | 84 / 26 |
| | on | 3.25 / 2.39 | 152 / 160 | 4 / 3 |
| | 10 mm | 3.25 / 2.39 | 152 / 160 | 4 / 3 |
| | 50% | 3.14 / 2.39 | 152 / 160 | 20 / 3 |
| `steps` | off | 3.01 / 3.31 | 126 / 123 | 33 / 28 |
| | on | 3.17 / 3.55 | 138 / 138 | 7 / 6 |
| | 10 mm | 3.14 / 3.54 | 138 / 138 | 10 / 6 |
| | 50% | 3.10 / 3.48 | 125 / 124 | 10 / 6 |
| `comb` | off | 1.37 / 1.51 | 69 / 54 | 47 / 36 |
| | on | 1.79 / 1.89 | 37 / 23 | 12 / 5 |
| | 10 mm | 1.43 / 1.54 | 63 / 49 | 39 / 31 |
| | 50% | 1.43 / 1.53 | 62 / 49 | 38 / 31 |
| 3DBenchy | off | 29.55 / 30.11 | 1520 / 1734 | 992 / 985 |
| | on | 33.41 / 34.66 | 1555 / 1717 | 475 / 391 |
| | 10 mm | 32.20 / 33.51 | 1572 / 1746 | 537 / 437 |
| | 50% | 31.27 / 32.38 | 1464 / 1529 | 622 / 526 |
| `mcbrim_test.3mf` plate 1 | off | 64.44 / 58.02 | 2185 / 1921 | 1493 / 1105 |
| | on | 67.00 / 60.46 | 2126 / 1877 | 1372 / 948 |
| | 10 mm | 65.77 / 60.07 | 2145 / 1905 | 1390 / 968 |
| | 50% | 66.36 / 60.27 | 2126 / 1848 | 1372 / 977 |
| plate 2 | off | 36.33 / 31.54 | 843 / 915 | 434 / 431 |
| | on | 37.61 / 32.32 | 803 / 922 | 365 / 404 |
| | 10 mm | 36.84 / 32.16 | 812 / 922 | 373 / 408 |
| | 50% | 37.33 / 32.24 | 795 / 903 | 365 / 404 |
| plate 3 | off | 38.91 / 34.85 | 841 / 917 | 434 / 441 |
| | on | 40.21 / 35.66 | 801 / 922 | 365 / 410 |
| | 10 mm | 39.43 / 35.49 | 810 / 922 | 373 / 414 |
| | 50% | 39.99 / 35.56 | 793 / 901 | 365 / 410 |

On the single parts the crossings fall as the reference's do, travel
grows by about as much (1.35 m and 1.14 m on `frame`, 0.74 and 0.65 on
`holes`, 3.9 and 4.6 on the Benchy) and retractions move the same way:
up where short hops to an outer wall become longer than
`retraction_minimum_travel` by going along it, down where a way round
stays inside the part (`holes`, `comb`). The ways themselves are the
reference's: on `frame` both run at the same 0.61 mm inside the
outline, to the micrometre. What is left crossing with the option on
is the same in both: the print's first move, the top layer, which is
all top surface and so has nothing to go along, and thin parts whose
drawn-in outline is the outline itself. A cube, two blocks in two
filaments (`twotone`), a part on supports (`mushroom`) and thin fins
are hardly changed by the option in either slicer: travel within
0.06 m, retractions within one, and the same travels crossing.

On the plates of several objects it does less here than there
(plate 2: 69 fewer travels cross in the reference, 27 fewer here, and
its retractions fall by 40 where here they rise by 7). Two things make
that, neither of them the routing. The reference's aligned seams on
those parts lie on the walls between two filaments, inside the part,
where the hop from the wall behind stays clear of the outline and goes
straight; here the seams are on the outside, at an inside corner of
the part, and the same hop is taken along the drawn-in outline and
round its corner: 0.40 mm straight and 1.27 mm that way, which is over
`retraction_minimum_travel` (19 of 72 such hops on plate 2). And fill
that belongs to no island is laid last, with long moves from object to
object that the reference, printing object by object, does not make.

Not as the reference, on purpose, where it is in error. It picks the
shorter way round a ring from lengths it has corrected twice over, so
some moves go the longer way about, by up to the length of the two
edges they meet the ring on; here the shorter way is taken. It makes
the outline moves are routed by on a layer's first such move and keeps
it, and a move between supports leaves the top surfaces in: supports
come first, so on a layer that has them moves in the part cross top
surfaces that they go round on other layers; here each kind of move has
its own. A move that was to go straight (from a brim, say) is routed
after all once a wipe has moved the nozzle; here it stays straight. And
two that change nothing a print would show: a point met at the start
of an edge is left on the ring instead of stepped a tenth of a
micrometre off it, and the width of a part is misjudged across from the
edge that closes a ring. With `max_travel_detour_distance` at 50% the
two differ more than without it (`tube` 20 travels crossing against 3,
`holes` 104 against 116): whether a way round is within the limit goes
by where the move starts, and the seams are not in the same places.

The reference's own file, timed here as the reference plans, comes to
the reference's own time with the option on as with it off (Benchy 40m
00s, plate 1 65m 44s, `holes` 23m 46s, each to the second). With the
option off the file is byte for byte what it was, and with it on it is
the same at any number of threads. What it costs: making the layers
ready is most of it and is done in parallel, the routing itself is in
the writer. Plate 1 of `mcbrim_test.3mf`, the largest of these, slices
in 2.44 s with it on against 2.13 s off on 18 threads (4.04 against
3.65 on four), and the Benchy in 0.67 s against 0.43 s (2.80 against
2.04 on one thread), each the best of five taken turn about.

Shells as thick as asked (`top_shell_thickness`,
`bottom_shell_thickness`), against the C++ slicer on
`tools/make_fixture.py`'s parts: metres of internal solid fill (C++
slicer / this engine), what this engine laid while it followed the
count alone, and the layers with solid fill inside them, counted from
0, which are the same in both slicers in every row. The first eight
rows are whole stock processes; the rest are the U1's 0.2 mm process
(five top layers and 1 mm, three bottom layers and no thickness, a
first layer of 0.25 mm) with what is named changed. Top and bottom
surfaces are the same length in both to 0.01 m in every row, and each
file is the same made on one thread and on many.

| Part and settings | Solid fill, m | By count | Layers solid inside |
| --- | --- | --- | --- |
| `cube`, U1 0.08 mm (9 layers, 0.8 mm) | 11.93 / 11.95 | 11.10 | 1-6, 238-247 of 249 |
| `cube`, U1 0.10 mm Color Mixing (5, 0.6 mm) | 6.96 / 6.97 | 6.12 | 1-4, 193-198 of 200 |
| `cube`, U1 0.12 mm (5, 0.6 mm) | 6.18 / 6.19 | same file | 1-4, 160-164 of 166 |
| `cube`, U1 0.16 mm (6, 1 mm) | 7.22 / 7.22 | 6.34 | 1-3, 117-123 of 125 |
| `cube`, U1 0.20 mm (5, 1 mm) | 4.68 / 4.68 | same file | 1-2, 94-98 of 100 |
| `cube`, U1 0.24 mm (4, 1 mm) | 4.80 / 4.80 | 3.87 | 1-2, 78-82 of 84 |
| `cube`, U1 0.28 mm (4, 1 mm) | 3.96 / 3.96 | same file | 1-2, 67-70 of 72 |
| `cube`, Centauri Carbon 0.16 mm (5, 1 mm; bottom 3, 0.6 mm) | 7.22 / 7.22 | 4.57 | 1-3, 117-123 of 125 |
| `cube`, `layer_height` 0.16, 0.12, 0.08 | 6.33 / 6.34, 7.91 / 7.92, 11.09 / 11.10 | 4.57, 4.46, 4.36 | 1-2 and 116-122, 156-164, 234-246 |
| `cube`, 0.16, `top_shell_thickness=0` | 4.56 / 4.57 | same file | 1-2, 118-122 |
| `cube`, 0.16, `top_shell_layers=0` | 1.77 / 1.77 | same file | 1-2; no top surface in either |
| `cube`, `bottom_shell_thickness=1` | 6.49 / 6.50 | 4.68 | 1-4, 94-98 |
| `cube`, 0.12, `bottom_shell_thickness=0.8` | 10.50 / 10.51 | 4.46 | 1-5, 156-164 |
| the same with `bottom_shell_layers=0` | 6.18 / 6.19 | 2.73 | 156-164; no bottom surface in either |
| `steps`, 0.16 | 18.74 / 18.76 | 13.53 | 1-2, 10-16, 29-35, 54-60 |
| `steps`, 0.12, `bottom_shell_thickness=0.8` | 31.04 / 31.08 | 13.22 | 1-5, 14-22, 39-47, 72-80 |
| `mushroom`, 0.16 | 14.88 / 14.88 | 10.68 | 1-2, 63-64, 79-85 |
| `mushroom`, 0.12, `bottom_shell_thickness=0.8` | 26.68 / 26.70 | 10.43 | 1-5, 83-88, 106-114 |
| `cube`, `precise_z_height=1` | 5.56 / 5.57 | 4.66 | 1-2, 93-98 |
| `cube`, 0.16, `ensure_vertical_shell_thickness=none` | 6.33 / 6.34 | 4.57 | 1-2, 116-122 |

The lowest layer of each run under a top is the band of sparse fill
made solid under the edge of a bridge over infill, not shell. The
shell over the underside of the mushroom's cap, a bridge, is made up
to the thickness as the one over the bed is. One
part differs, and did before: `steps` with the U1's own 0.16 mm
process has the top of its lowest tier, at 3 mm, exactly where layer
18 is cut; the reference takes that layer for part of the tier and
this engine does not, so the tier's top and the shell under it lie a
layer lower here (10-16 against 11-17), each as deep as the other's.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
