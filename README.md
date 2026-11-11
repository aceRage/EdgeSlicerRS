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
| `crates/edge-arachne` | Walls of changing width: a region's skeleton, how many lines fit along it and where that number changes, and the lines themselves (a port of the reference's Arachne) |
| `crates/edge-paint` | Which filament prints what where colours are painted on: each layer shared out from its painted outline, and paint on top and bottom faces carried through the shell |
| `crates/edge-clip` | Polygon union, difference, intersection, offset and gap closing (wraps clipper2-rs) |
| `crates/edge-extrusion` | Extruded line cross-section (flow) and extrusion loops |
| `crates/edge-parity` | `edge-parity`: runs the reference slicer and this engine on one project and compares time and G-code |
| `crates/edge-perimeter` | Walls, the lines that fill the gaps between them, and the infill area inside (classic constant-width generator) |
| `crates/edge-medial` | The centre line of a narrow region and how wide it is along it (wraps boostvoronoi), what part of the plane lies nearest each of a set of edges, the whole Voronoi diagram of a set of edges as plain data, and lines of changing width as extrusions |
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
- Gap fill: a gap between walls too narrow for another wall gets a line
  down its middle, as wide as the gap is at each point, found as the
  reference finds it (the medial axis of the gap). On whenever
  `gap_infill_speed` is above zero.

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
  value of that setting is noted and not followed. Fill over air takes `bridge_flow`
  and `bridge_density`, and runs along the edges it has nothing beside,
  so its lines go from one support to the other. Bridges, bottoms and
  tops reach a wall's spacing into the fill beside them, to have
  something to hold on to.

- G-code: `slice --gcode out.gcode` orders each layer (skirt and brim,
  each part's brim whole before the next, then island by island: an
  island's walls from the inside out, each beside the last, then the
  gap fill and the fill that lie in it), holds speeds to the
  hot end's melt rate, sets acceleration by feature, and reports filament,
  weight and time. Following the reference:
  - Seams: each outer wall starts at its best corner by the reference's
    angle score and stays in line with the layer below; inner walls start
    alongside. Walls stop `seam_gap` short of their start.
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

Walls do not yet have thin-wall detection (the U1 profiles leave it off).
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
the lettered plate), narrow solid areas are filled with the same lines
as wide ones where the reference lays loops along them
(`detect_narrow_internal_solid_infill`, noted), narrow parts of top and
bottom surfaces get no gap fill
(`gap_fill_target`), shell thickness in mm is ignored, and eight of the
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
- With one wall over top surfaces the inner walls keep the numbers of
  the pass that made them; the reference numbers them again by which
  wall each touches (`renumber_inner_walls_by_depth`), which decides
  their order with `inner-outer-inner`. With `top_surface_expansion`
  set, the reference cuts the inner walls where they run over the top
  surface instead; that is not ported, for these walls or the classic.
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
not read: the settings fuzz every part alike. Thin walls (classic) are
not made, so there are none to fuzz. `overhang_reverse` is not
followed, with or without fuzzy skin, and a note says so.

Classic walls are untouched: every file that sets `classic` is byte for
byte what it was.

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

The reference still does more in that time (arc fitting, time planning,
ironing out its fill order), so the speed gap will narrow as stages are
added; measuring overhangs took this engine from seven times faster to
five on the cylinder.
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

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
