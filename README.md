# EdgeSlicerRS

A from-scratch Rust slicing engine for EdgeSlicer, built for speed, with the
start of a Tauri desktop app. The C++ EdgeSlicer stays the shipping
product and the reference this engine is checked against; nothing here
replaces it until output matches on the printers we care about.

## Layout

| Path | What it is |
| --- | --- |
| `crates/edge-geom` | Fixed-point 2D geometry (nanometre `i64`, same scale as libslic3r): points, polygons, regions, convex hull |
| `crates/edge-mesh` | Indexed triangle mesh, transforms, STL read/write; the triangle selector (`selector`: paint on a mesh's triangles, split as finely as a stroke needs, and the codes a project stores them as, a port of the reference's `TriangleSelector`) and a tree of boxes for what a line of sight meets (`raycast`), which the desktop app's painting tools work with; a mesh cut in two by a plane (`cut`: both halves closed, each capped over the edges the cut opened by ear cutting that keeps every corner, the reference's `cut_mesh` made watertight), and the shapes of a cut's connectors (`connector`: prisms, frustums, a dowel's two cones, a snap, solids of revolution and prisms over an outline, which the engine's push-fit pin, `edge_engine::pushfit`, is made of), which the desktop app's "Cut" works with; corners a hair apart welded into one with the mesh kept closed (`tidy`); solid booleans (`boolean`: union, difference, intersection of two closed meshes, and `csg` for a model's parts in order, through the Manifold library's pure-Rust port `manifold-rust`, which only this module names). An operand must bound a volume: not empty, finite, closed and consistently wound (welded along open edges first, turned outward if inside out); one that is not is refused ("does not bound a volume"). Shells that cross one another are united first; a shell that crosses itself is taken (or refused, on request) and the result then checked against its complement by volume. A result is always closed, consistently wound and free of zero-area triangles in single precision, or an error; the same input gives the same bytes. Where two parts of a result only touch, or a sliver is thinner than single precision, the library's exact test can still find the result touching itself, by no more than the rounding |
| `crates/edge-config` | Vendor presets resolved through `inherits`, `include` and `renamed_from`; a person's own presets (`user`), read, written, renamed and deleted in the reference's layout and format (`<data>/user/default/{machine,filament,process}/<name>.json` with `.info`, only what differs from the parent) and resolved through their chain into the vendor's; preset bundles imported and exported as the reference's "Import Configs" and "Export Preset Bundle" do (`bundle`: `.json`, `.zip`, `.orca_printer`, `.orca_filament`); the reference's defaults for every setting, and the text form it writes settings in |
| `crates/edge-3mf` | 3MF projects: objects, parts and their roles, instances, plates, project settings, and colours painted onto triangles; and writing a project back in the reference's own layout (`write`), keeping what was read and not changed (every kind of paint, per-object and per-part settings, layer ranges and heights, cut information and the recipes of cuts kept editable, a plate's settings and pictures, every other entry) and leaving out print-host addresses and keys; the G-code set at each plate's layers (pauses, custom G-code, filament changes), read and written as the reference does |
| `crates/edge-fuzzy` | Fuzzy skin: walls roughened by moving their points or changing their width, with the noise it is drawn from (a port of the reference's FuzzySkin and of the libnoise modules it uses) |
| `crates/edge-avoid` | Travel routed round the inside of a part's walls where a straight move would cross them: what a layer's moves are routed by, and the way round for each (a port of the reference's AvoidCrossingPerimeters) |
| `crates/edge-arachne` | Walls of changing width: a region's skeleton, how many lines fit along it and where that number changes, and the lines themselves (a port of the reference's Arachne) |
| `crates/edge-paint` | Which filament prints what where colours are painted on: each layer shared out from its painted outline, and paint on top and bottom faces carried through the shell |
| `crates/edge-clip` | Polygon union, difference, intersection, offset, an inner offset with a distance of its own for every vertex, and gap closing (wraps clipper2-rs); Minkowski sums and no-fit polygons of convex outlines, for arranging |
| `crates/edge-arrange` | Arranging objects on the bed as the reference does: each object's outline and the room kept round it, libnest2d's no-fit-polygon placer and first-fit selection over several beds with the objective of the reference's `Arrange.cpp`, and NLopt's subplex on one variable, ported step for step |
| `crates/edge-extrusion` | Extruded line cross-section (flow) and extrusion loops |
| `crates/edge-parity` | `edge-parity`: runs the reference slicer and this engine on one project and compares time and G-code |
| `crates/edge-perimeter` | Walls, the lines that fill the gaps between them, and the infill area inside (classic constant-width generator) |
| `crates/edge-medial` | The centre line of a narrow region and how wide it is along it (wraps boostvoronoi), what part of the plane lies nearest each of a set of edges, the whole Voronoi diagram of a set of edges as plain data, and lines of changing width as extrusions. The same edges give the same diagram on every run and thread: the library's skip list has its levels from a generator each list owns, not from the thread's random numbers (`vendor/cpp_map`). And wherever they lie: all three are swept in coordinates of the edges' own, the whole diagram about the middle of the box round them, which is handed back with it |
| `crates/edge-adhesion` | Skirt and brim, with the automatic brim width |
| `crates/edge-contour` | Z contouring: a top surface, its ironing and the walls round it laid at the height of the model's surface under each point, within the layer (a port of the reference's ContourZ.cpp) |
| `crates/edge-support` | Supports under overhangs, normal (grid) and tree, with dense tops and feet |
| `crates/edge-gcode` | Print order, seams, travel, retraction, wipe and lift, cooling, overhangs, and extrusion, written as G-code |
| `crates/edge-visibility` | How much of a part's outside can be seen from each place on its surface, which is where a seam shows least: points scattered over the mesh, rays cast from each, and the measure at any place taken from the points round it (a port of the visibility measure of the reference's SeamPlacer); and where the parts of an object meet, points on each part's surface that touch another part, which seams are moved to (its `gather_part_joints`) |
| `crates/edge-infill` | Top, bottom, bridge, solid, bridged-over-infill and sparse surfaces, and the lines that fill them, with loops for the narrow parts of solid fill; and ironing: which areas are gone over again, and the lines that do it |
| `crates/edge-template` | The placeholder language of custom G-code: `{expression}`, `{if}`, `[variable]`, and scripts |
| `crates/edge-time` | How long a G-code file takes, with acceleration: the file is read as a printer reads it and planned as the reference's estimator plans, or as Klipper does |
| `crates/edge-tower` | The prime tower a nozzle is primed on after a tool change |
| `crates/edge-slice` | Mesh to per-layer contours, with the model's corners and not its triangles', then to regions with holes; parallel across layers. Elephant foot compensation of the first layers; round holes made polygons of few sides, and round outlines and holes resized by what the filament prints them off by; the last layers' heights tuned so a part comes out its exact height, and layers grown downward under overhangs so they can be printed |
| `crates/edge-engine` | The whole pipeline behind one call, for whatever drives it: open a model and list its plates, settle its settings, slice with progress and cancel, G-code, a layer's lines for drawing, a vendor's presets by printer, arranging the plates (`arrange.rs`), auto orient (`orient.rs`, the reference's Orient.cpp with its cost and weights, and `orient_job.rs`, what its OrientJob orients and how the turn is put back), and its printer models with the presets, picture and sizes of each; and, for a set of settings, which of them a slice reads and what it notes (`followed`); where a setting may be set for one object, part or height range and whether it is followed there (`settable`), and the regions a plate is shared out among (`regions.rs`); the model's cuts (`cut.rs`: by a plane, a dovetail, with connectors and push-fit pins (`pushfit`), by a curved surface (`curved`) and by a surface swept along a line drawn on the model (`drawn`, a port of the reference's DrawCut.cpp), with the fork's print-in-place Flexi joints (`flexi`, FlexiJoint.cpp) and its TAC Joint (`tac`, TacJoint.cpp, its reference meshes in `tac_data.rs`), each kept editable in the project's recipe) |
| `crates/edge-cli` | The `edge` command line |
| `tools/make_defaults.py` | Asks the reference for its default of every setting and writes the table `edge-config` carries; run again when the pin moves |
| `tools/make_settings_pages.py` | Reads the reference's process, filament and printer pages and option definitions from its source and writes the data the desktop app draws its settings pages from; run again when the pin moves, after `make_defaults.py` |
| `tools/make_translations.py` | Reads the reference's gettext catalogs (`localization/i18n/<language>/*.po`) and writes the desktop app's table of each language, English included, only the entries gettext would use; run again when the pin moves |
| `tools/make_fixture.py` | Writes small test 3MFs (parts that need supports or a brim, a plain cube, two blocks in two filaments, a comb with thin teeth on the bed, three tiers with a top surface on each, bars over a base for small bridges over infill, a slab with round holes for the hole options, a slab with holes that narrow on the way up for bridging them, a slab with pockets underneath, three round ones for arc fitting: a post, a post with a hole, a slab with a wide curve and tight ones, a post that widens as it rises for scarf seams, two of two parts for seams where parts meet: a round post cut in two, and a block with a post pushed half into it, and two with seams painted on: a post with a strip painted to have them, and a cube with two sides painted to keep them off, a plate on a frame with walls wholly over air, and for Z contouring a wedge of 4 degrees, a shallow dome, a ramp meeting a flat, and the wedge with a hole down through it; and for regions, settings set on the object, a part, a height range (`object:`, `partN:`, `range:LOW:HIGH:` before a setting), a cube with a modifier box through its right half and with a second through its front half, a mushroom with a support blocker under half its cap, and a block leaning out 50 degrees with a support enforcer over its leaning side or that side's near half painted to have supports) to compare with the reference on |
| `tools/make_app.sh` | Builds the desktop app and wraps it in a macOS `.app` that opens with a double click (macOS only) |
| `tools/make_project.py` | Writes one of those parts as a project whose settings are named presets, with as many filaments as are named, each list holding the value each filament reads at standard flow: how the test prints for a printer are made |
| `reference/EdgeSlicer` | Pinned submodule of the C++ slicer: profiles, test models, behaviour to match. Read-only here |
| `vendor/cpp_map` | A copy of the `cpp_map` crate (MIT or Apache-2.0), the skip list under the Voronoi library, changed so that a list is the same shape on every run; patched in for the published crate by both workspaces. Its README says what was changed. Not a member of the workspace, and kept in its own formatting |
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
# A project arranged as the reference's Arrange does (every plate, or --plate N), saved, and where
# everything stands written as JSON, to compare with the reference's --arrange 1 --export-3mf
cargo run --release -p edge-cli -- arrange reference/EdgeSlicer/tests/mcbrim_test.3mf --out arranged.3mf --layout arranged.json
# Every copy turned the way up the reference's "Auto orient" turns it (or one plate's, --plate N);
# with --command-line, what the reference's --orient 1 finds for each object, in its own lines
cargo run --release -p edge-cli -- orient reference/EdgeSlicer/resources/handy_models/3DBenchy.3mf --command-line
# A model or project saved as a project file the reference opens as its own: a 3MF keeps its own
# settings, an STL is saved without any; --vendor/--machine/--process/--filament save it with presets.
# What the file holds that would not be kept is listed and nothing is written, unless --accept-losses.
# --timings says how long each stage of a slice took (on stdout, never in the G-code).
cargo run --release -p edge-cli -- save model.stl --out model.3mf --vendor Snapmaker --machine "Snapmaker U1 (0.4 nozzle)" \
  --process "0.20mm Standard @Snapmaker U1 (0.4 nozzle)" --filament "Snapmaker PLA @U1"
```

## The desktop app

```bash
cd apps/desktop
cargo run                      # the window
cargo run -- model.3mf         # the window, with a model open
cargo run -- --serve 4710      # no window: the same page at http://127.0.0.1:4710/
cargo test
```

On a Mac, `tools/make_app.sh` builds the release app and wraps it in
`target/EdgeSlicerRS.app` (or the path given), which opens as a window with
a double click or `open target/EdgeSlicerRS.app`. The bundle finds the
profiles in `reference/EdgeSlicer` of the source tree it was built from, so
it works while that tree stays where it is; build it again after pulling.

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
  each printer; the C++ application's six pages of process settings
  ("Quality" to "Others", 433 options, drawn from data made from its
  source), any of which is set over the preset for the project, with a
  mark on each option the engine does not read and the engine's note on
  each it follows in part; the scene in 3D: the printer's bed (the vendor's model
  and picture of it where its profile has them), every plate of the
  project where the C++ application stands it, each object in its
  filaments' colours, painted colours included; a click selects an
  object, one of its parts, or a plate. Objects and their parts are
  edited there as in the C++ application (moved, turned, scaled,
  mirrored, split, deleted, renamed, given filaments; parts changed in
  kind and added; section 13 of `UI_PARITY.md`). An STL or a 3MF opens
  as a project; "Save Project" and "Save Project as..." write it as a
  3MF the C++ application opens as its own, asking first about anything
  its file held that would not be kept (section 15).
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
  yet. "Slice plate" or "Slice all", and the print button: "Export
  G-code file", which writes the file the command line writes, or
  "Print", which sends those same bytes to the printer ("Upload" or
  "Upload and Print", and the "Print host upload queue").
- **Sending to a printer**: "Connect to printer" on the Printer card sets
  where the printer is ("Host Type", "Hostname, IP or URL", "Device UI",
  "API Key / Password", "HTTPS CA File", "Test"), kept for that printer
  preset in the app's data folder, the key in a file of its own. Built:
  OctoPrint's API ("Octo/Klipper"), a Snapmaker U1 over its Moonraker,
  and the Elegoo Centauri Carbon ("Elegoo Link"); `src/hosts/`, with a
  mock of each printer in the tests. Nothing is asked of a printer
  unless the person asks.
- **Device**: with a connection, the printer's own page in an isolated
  frame, and for a U1 its files ("Storage": "Print", "Delete"); without
  one, the printer of the project shown as its preset describes it.

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

It has no editing or saving of presets (a setting changed is the open
project's) and no installer. The
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
inventory has the sizes and times measured. The tabbed window itself (Tauri, WebKit on a Mac) was
opened again with the restyle, with the three-plate project, at its first
size (1280 × 820) and its least (900 × 560), dark and light, and looked at
(not driven: nothing was clicked in it); before that the owner ran it by
hand, and its file dialogs, slicing and layer view worked, with G-code
byte for byte the command line's but for the time in its first line. Not built on
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
- Thinner solid fill over the first layer (`elefant_foot_layers_density`,
  from 50% to 100%; 100% is the default and leaves everything as it
  was): what the first layer is squeezed out by has somewhere to go when
  the layers resting on it hold less plastic. The second layer's solid
  fill inside the part is laid at the density set, and each layer after
  it nearer to full by an even step, over as many layers as
  `elefant_foot_compensation_layers` says (one, where it says less): at
  70% over three layers, 70%, 80% and 90%, and full from the fifth
  layer. The lines keep their width and lie that much further apart, on
  the grid through the middle of the object that sparse lines are laid
  on, not spread from the area's own edge; they are joined and ordered
  as solid lines are, and get none of the fill's own gap fill. Top and
  bottom surfaces, bridges and bridges over infill are left alone. A
  port of the reference's few lines in `Layer::make_fills` (Fill.cpp)
  and of what its line fill does with a density under one
  (FillRectilinear.cpp `fill_surface_by_lines`). A value outside 50% to
  100% is an error, as the reference refuses to slice with one. On a
  20 mm cube on the U1 and on the Centauri Carbon the lines are the
  reference's distance apart on every layer at 70% and at 50%, over one
  layer and over three (at 70% over three: 0.539, 0.471 and 0.419 mm
  where full fill is 0.377 mm), and on the U1 in the same places to
  the micron; the length laid is the reference's to 0.1 mm on every
  thinned layer (at 70% over three layers 632.7, 723.5 and 818.5 mm
  against 632.6, 723.5 and 818.5, and 907.1 against 907.0 at full; at
  50% over one layer 472.5 against 472.5), and on the three plates of
  `mcbrim_test.3mf` at 70% the solid fill is 2.92 m, 1.45 m and 1.45 m
  shorter than at 100% in the reference's files and 2.90 m, 1.46 m and
  1.45 m here. Not as the reference: it does none of this for a part
  on a raft, and no raft is printed here, so every part is on the bed.
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
- Every layer is cut without the points where the cutting plane
  crosses an edge inside a flat face, as the reference cuts them
  (TriangleMeshSlicer.cpp `coplanar_edges`): they are corners of the
  mesh's triangles, not of the model. Left in, such a point lies a unit
  or two off its flat, and closing the outline by `slice_closing_radius`
  leaves a kink there, up to some hundredths of a millimetre from where
  the corner beside it was; a round hole cut that way is not round
  enough to be told for one. (Until 2026-11-11 this was done only with
  one of the two hole options on.) Where the plane passes through a
  corner of the mesh the corner is kept, whichever triangle the contour
  starts on.
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
  `small_perimeter_speed`. With `unsupported_wall_last` a loop wholly
  over air goes after the island's other walls, innermost first, and
  one that comes within a wall spacing and a half of none of the
  anchored walls waits until the island's fill is down, as the
  reference has it (PerimeterGenerator.cpp `defer_unsupported_loops`);
  here the loop is told by the same cut against the layer beneath that
  makes its overhang walls, as it is laid. On the parts compared
  (`ledge`, `mushroom`, `shelf`, `bridge`, each wall order, classic and
  Arachne walls) the layers that change are the same in both files and
  their walls go down in the same order; with it off the files are what
  they were, to the byte.
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
- Narrow solid areas (`detect_narrow_internal_solid_infill`, on by
  default and in the stock presets): the parts of the solid fill inside
  a part that straight lines would cross in strokes too short to be
  worth laying are told from the rest and filled with loops along their
  outline instead, of whatever width fits. A layer's solid fill is
  taken together, areas that touch as one. Where its lines would go is
  worked out much as the fill lays them (the area less what is under
  four spacings wide, half a spacing in from its edge, crossed by a
  line every spacing), and of those strokes the ones under 4 mm go
  where they follow one another: from the sixth of a run of them, and
  a run that neither began beside a long stroke nor leads into one.
  What the strokes left cover is the wide part, which is filled with
  lines as before; the rest is narrow, save a piece too thin for a
  line of its own that touches the wide part and goes with it, and a
  patch under three spacings across both ways and four spacings
  squared in area, which is left out. A narrow area is filled inside
  the walls, without the fill's overlap into them, by walls of
  changing width: as many loops as fit and a line down the middle
  where an odd number does, each loop stopped `seam_gap` short of
  where it began and cut where its width changes; they are solid fill
  in name, speed and cooling, a part of their own, laid in the order
  made. Turned off, narrow areas are filled with the same lines as
  wide ones, as before. A port of the reference's `split_solid_surface`
  and `filter_vibrating_extrusions`, of where `group_fills` uses them
  (Fill/Fill.cpp) and of Fill/FillConcentricInternal.cpp. So a 20 mm
  cube gets a triangle of loops in one corner of every solid layer,
  the corner its slanting lines end in, and loops for the whole of the
  layer of strips that carries the edges of the bridge over its sparse
  fill: where the reference's file has them. The triangle is one
  stroke larger here, two loops, the outer 1.9 mm on a side, where
  the reference has one loop 1.3 mm on a side and a dot 0.16 mm long
  in the middle of it: where a run of short strokes began is asked of
  the stroke itself here (below), and of the short strokes running
  out into a corner the first stays a line, where the reference keeps
  two. Of the 64 lines of a solid layer of the cube 62 are the
  reference's, at the same places and of the same lengths to three
  microns, the last before the triangle is loops here and the one
  before it 0.24 mm longer; so with 24 of the 26 of a `mushroom`'s
  solid layer and 255 of the 257 of a `frame`'s. (Asked the
  reference's way the triangle's box was the reference's to the
  hundredth of a millimetre and the cube's lines its own, all 64.)
  On the U1's presets, reference against this engine, loops in metres
  by areas on layers: cube 0.164 against 0.182 (6 on 6 in both),
  `frame` 0.582 against 0.601 (6 on 6), `steps` 0.844 against 0.911
  (22 on 14), `comb` 0.191 against 0.210 (6 on 6), `fins` 0.017
  against 0.029 (3 on 3), `mushroom` 0.261 against 0.289 (8 on 8),
  `holes` 2.869 against 2.930 (126 areas against 124, on 47 layers);
  with the Centauri Carbon's the same to 0.001 m but for `steps`
  (0.848 against 0.904) and `holes` (2.895 against 2.904, 116 areas
  against 114). Pieces of solid fill under a millimetre long, dots:
  none here on the first six of those parts, where the reference lays
  3 to 14, one in each triangle, and 44 against 60 on `holes`. All
  the solid fill inside the part on those eight parts is within 0.2%
  of the reference's length on the U1 but for the `tube`, 0.5% under
  (before narrow areas were told apart it was 0.0% to 0.2% over, and
  1.0% under on the `tube`); with arcs fitted its solid fill had 140
  arcs against the reference's 137 on the cylinder (179 before), 337
  against 339 on the `tube` (476) and 7 against 7 on the `mushroom`
  (none), counted while a run's start was asked the reference's way;
  the `mushroom`'s seven were its dots, and are gone with them. On
  the three plates of `mcbrim_test.3mf` the reference lays 13.6 m, 5.3 m and 5.3 m of its
  solid fill as loops and this engine 12.3 m, 5.3 m and 5.3 m, where
  it laid none; solid fill in all is 49.53 m against 50.51 m on plate
  1 (1.9% under, from 1.0% over) and 24.58 m against 24.29 m on the
  other two (1.2% over, from 3.5%). Plate 1 is under because on four
  of its layers this engine has less solid area to fill, whatever it
  is filled with (with the option off in both, those layers are 2%,
  4%, 6% and 54% short: bridges over infill and sparse fill are
  sorted out differently there, as above), and the joins of short
  lines no longer make up for it; layer by layer the two files differ
  by 1.28 m in all where they differed by 2.00 m. Not as the reference, each marked in the
  code: there every loop starts at its point nearest the object's own
  origin, the place the last one ended at never being noted, and here
  where the one before it ended; a line that is no loop is not
  shortened by the seam gap at its end; the loops of an area are laid
  each after the one that starts nearest, not in the reference's chain
  of nearest pairs; and three mistakes in `filter_vibrating_extrusions`
  and in the tracing of the wide part are not carried over (a stroke
  reached by two runs of short strokes counts the first comer's run
  there, not the deeper; a run of strokes that finds the stroke beside
  it taken goes on into that one all the same; and whether a stroke's
  run began beside a long stroke is read there off the strokes before
  it, whatever they were last told, where the function that reads it
  is described as answering for the stroke itself and what it
  replaced in PrusaSlicer asked every stroke of a run where that run
  began: so the stroke a run begins with is never asked, a lone short
  stroke after a long one is taken out, the one the note is made to
  keep, and of a run into a corner the first two stay). Round a
  `tube` that is seen as one line: the last stroke against the round
  wall is a short one after a long one, the reference takes it out and
  lays a line of 4.3 mm there by itself as a narrow area on each solid
  layer, and here it stays with the lines, which have none to lay
  there (strokes are counted half a spacing from where the fill puts
  its lines), as they have none in the same place on the other side
  of the tube in either.
  Solid fill inside the part is fitted to its area as top and bottom
  surfaces are, as in the reference (FillRectilinear.cpp
  `fill_surface_by_lines` fits all fill at full density but bridges,
  FillBase.cpp `_adjust_solid_spacing`). With the option off that is
  every area: a full layer of the cube is 67 lines 0.3795 mm apart and
  0.422 mm wide, 902.55 mm, each where the reference's is to the
  micrometre (68 lines at 0.377 and 907.87 mm before); 669.39 mm in
  both on the `tube`, 2136.36 on the `mushroom`, and the round boss of
  `holes` has the reference's 22 lines 0.429 mm wide. Over the whole
  part solid fill is then within 0.05% on the cube, the deck, the
  `frame`, the `mushroom`, the `tube` and `counterbore`, 0.1% under on
  `steps` and 0.2% under on `holes` (it was up to 0.9% over, and 0.4%
  under on the `tube`). With
  the option on what is fitted is the wide part, as there. That part
  is cut along its own lines a whole number of spacings from the
  area's edge, where there is nothing to spread, unless its last line
  reaches the far edge and it is the area's whole width: so a cube's
  solid layer keeps its pitch in both slicers, and a round one with
  nothing narrow in it is spread (`cylinder_drawcut.3mf`: 202 lines
  and 12112.63 mm a layer in both, from 201 and 12126.80). Still
  different: the boss of `holes`. Its lines end in two short strokes
  after a long one; the reference keeps both (as above), so the wide
  part is the whole boss and its lines are spread, 22 of them 0.429 mm
  wide, and here the second is loops, the wide part stops there and
  its 21 lines keep their 0.42 mm.
- Small area flow compensation (`small_area_infill_flow_compensation`,
  off by default, with its curve
  `small_area_infill_flow_compensation_model`): a small area fills with
  lines too short for the nozzle to get up to speed on and comes out
  overfilled, so a short move of solid fill is laid with a share of its
  plastic that falls with its length. The curve is given as points, a
  length in mm and a share (the reference's own: nothing at 0 mm, 44%
  at 0.2 mm, 76% at 0.8 mm, 92% at 3 mm, all of it from 10 mm), and
  drawn through them so that it never overshoots between two. Each
  move of a top surface, a bottom surface or solid fill inside the
  part is thinned by its own length, where that kind of fill is set to
  a pattern of straight lines (`rectilinear`, `alignedrectilinear`,
  `monotonic`, `monotonicline`); nothing else is touched. A port of
  the reference's GCode/SmallAreaInfillFlowCompensator.cpp and
  GCode/PchipInterpolatorHelper.cpp and of where its writer applies
  them (GCode.cpp `_needSAFC`, `_extrude`); a model it refuses (lengths
  that do not start at 0 and rise, shares that do not rise to 1, fewer
  than two points) is an error here too. On a 20 mm cube, a frame and
  the three tiers of `steps`, on the U1 and the Centauri Carbon, the
  filament fed per mm of move is the reference's for every length of
  move on top surfaces, to the last figure its file gives (on the U1's
  cube: 0.02517 mm a mm on moves of 0.8 to 1.5 mm, 0.02924 on 3 to 5 mm,
  0.03104 on those over 10 mm), on bottom surfaces wherever the two lay
  the same moves, and for the lines and joins of solid fill inside the
  part (a join between two lines, 0.53 mm long: 0.02089 in both where
  it was 0.03073; lines of 3 to 5 mm 0.02887 against 0.02887; over
  10 mm 0.03064 against 0.03064). Turning it on takes 3.3 mm of
  filament off the U1's cube in the reference and 3.1 mm here, 22.2 mm
  and 21.9 mm off the `frame`, 11.2 mm and 9.8 mm off `steps`. Not as
  the reference, each where it goes against what it is for: it thins
  the loops a narrow solid area is filled with as if every stretch of
  a loop were a line (round a 20 mm tube, cut into chords of half a
  millimetre, such loops are laid at 68% to 74% of their plastic from
  end to end; on plate 1 of `mcbrim_test.3mf`, a quarter of whose
  solid fill is loops, it takes 160.6 mm of filament off where this
  engine takes 68.1 mm), and here a loop keeps its flow; and it takes
  a bottom surface for whatever is on the first layer, so one resting
  on supports is never compensated there, where here each kind of fill
  goes by its own pattern on every layer (which changes nothing yet:
  this engine lays a bottom surface only on the bed). A join that
  follows a curved wall is cut into more moves here than in the
  reference, each thinned by its own length: on a round tube the joins
  of a solid layer keep 57% of their plastic where the reference's
  keep 66% (0.84 mm of filament of 1.47 mm, against 1.06 mm of 1.59).

- Regions: settings that belong to one object, one part or one band
  of an object's height, and the parts that are not printed but change
  what is (`crates/edge-engine/src/regions.rs`, ported from
  PrintApply.cpp `generate_print_object_regions`, PrintObject.cpp
  `region_config_from_model_volume`, PrintObjectSlice.cpp
  `slices_to_regions`, Layer.cpp `make_perimeters`, Fill.cpp
  `group_fills`). Everything on a plate is sliced together, and each
  layer is shared out among keys, one a region: a filament's number for
  the plate's settings in that filament, as it always was, and a number
  from 2^20 up for a region with settings of its own: the plate's with
  the object's laid over them (object-level and region-level keys),
  then the part's (region-level), then a height range's, then each
  modifier's in turn. A part that sets back what its object changed is
  the plate's region again, as the reference finds regions by comparing
  configs, so a plate that sets nothing of its own slices exactly as
  before. Walls, surfaces and shells, fill, ironing and the bridges
  laid over an object's holes read their settings from the region; the
  G-code writer lays a region's walls and fill at its own speeds,
  accelerations, flow and wall order (`edge_gcode::Layer::region`).
  Regions whose walls are alike (`is_perimeter_compatible`'s keys, and
  fuzzy skin, below) have one set laid round all of them, by the
  settings of the densest, each keeping its own part of the area inside;
  regions in one filament whose fill is laid alike and at the same
  speeds on a layer are filled as one. The single set of walls that
  tells walls between regions from the shell is as deep as the deepest
  walls on each layer (`discover_vertical_shells`). Tool order is
  worked out for tools, each printing its regions in turn.
  - Modifiers take what of their own object's parts lies inside them on
    each layer, every region they meet becoming one with the modifier's
    settings over its own, a later modifier laid over an earlier where
    they overlap; one that sets nothing new leaves its parent's region,
    and one with a filament of its own prints what it takes in it.
    Height ranges (`Metadata/layer_config_ranges.xml`) give the object's
    parts their settings on the ranges' layers, before the modifiers.
  - Support blockers and enforcers, and supports painted on or off
    (`paint_supports`, the downward faces projected onto the layers they
    hang over): normal supports take blockers out of the overhangs and
    hold up what of a layer an enforcer covers that the layer below does
    not; organic trees do the same, an enforced area grown by a tip's
    diameter, and keep their branches out of blocker volumes
    (TreeModelVolumes.cpp `m_anti_overhang`). `normal(manual)` and
    `tree(manual)` hold up only what is enforced. With `enable_support`
    off nothing is supported, enforcers or not, as in the reference.
  - An object's own brim (`brim_type`, `brim_width`, `brim_object_gap`).
  - `edge_engine::settable(key)` says whether the reference reads a
    setting on an object, a part or modifier, a height range, and
    whether this engine follows it there; the desktop app's pages of an
    object's, a part's and a range's settings show it for each option.

  What is followed where (anything else set on an object, part,
  modifier or range that differs from the plate is a note naming it and
  what set it; a setting the reference itself ignores there is not):

  | Settings | Object | Part | Modifier | Height range |
  | --- | --- | --- | --- | --- |
  | wall count, wall order, line widths, wall generator (object), Arachne options (object) | yes | yes | yes | yes |
  | top and bottom shell layers and thicknesses, solid infill | yes | yes | yes | yes |
  | sparse infill density and pattern, infill direction, anchors | yes | yes | yes | yes |
  | speeds, accelerations, flow ratios of walls and fill | yes | yes | yes | yes |
  | fuzzy skin | yes | yes | yes | yes |
  | ironing | yes | yes | yes | yes |
  | the part's filament (`extruder`) | yes | yes | yes | no (as the reference) |
  | wall, sparse and solid infill filaments | noted | noted | noted | noted |
  | brim type, width and gap | yes | ignored (as the reference) | ignored | ignored |
  | `layer_height`, `precise_z_height` (object) | yes | ignored | ignored | yes (`layer_height`) |
  | supports on/off and their options, `seam_position`, elephant foot, XY compensation, closing radius | noted | ignored | ignored | ignored |
  | holes to polyholes, `make_overhang_printable`, Z contouring, scarf seams, bridges over sparse fill, `enable_extra_bridge_layer` | noted | noted | noted | noted |
  | EdgeSlicer support groups (`support_group` on a part) | | noted | | |

  Not as the reference, each marked in the code: where parts of one
  object with settings of their own overlap, the region with the
  higher key keeps the overlap, where the reference gives it to the
  part later in the object's list; a region keeps one set of fuzzy skin
  settings, so regions that differ only in fuzzy skin get walls of their
  own where the reference lays one set and fuzzes each piece by its
  region; painted triangles that stand upright, which the reference
  hands tree supports as points to grow a tip from, are not followed;
  colour painted on an object with settings of its own prints with the
  plate's settings (a note says so). Not done, and noted: supports on
  or off for one object, and an object's own seam position.

- An object's own layers (`crates/edge-engine/src/layering.rs`,
  `crates/edge-slice/src/layering.rs`, ported from Slicing.cpp and
  SlicingAdaptive.cpp): its own `layer_height`, its height ranges'
  `layer_height`, or its variable layer height profile
  (`Metadata/layer_heights_profile.txt`, read and written as bbs_3mf.cpp
  does: used where it ends at the object's top, else the profile its
  ranges give, `PrintObject::update_layer_height_profile`), each layer as
  thick as the profile asks at its middle (`generate_object_layers`, with
  `precise_z_height` tuning the object's own last five), held within the
  least and greatest height its nozzles allow
  (`SlicingParameters::create_from_config`). The adaptive profile from the
  slopes of the object's faces, its smoothing and a stroke of the layer
  height tool are ported for the desktop app. The objects that set nothing
  of their heights are sliced together at the plate's, as before; each set
  of objects whose layers agree is sliced by itself, everything up to the
  order of tools, the prime tower, skirt and brim; their layers are then
  laid together by their tops, tops within `EPSILON` as one, at their
  middle (GCode.cpp `collect_layers_to_print`). A layer's height in the
  G-code is the step from the one before, and each line's height is said
  where it changes; what lies under an object's layer is its own layer
  below; sparse fill is spaced by the object's layer height
  (Fill.cpp `group_fills`). Refused with the reference's words
  (`Print::validate`): a prime tower with objects of different layer
  heights, of different raft layers or of different variable layer
  heights, and organic tree supports with a variable layer height.
  Not as the reference: an object's first layer for its brim is cut at
  the first group's heights (the same first layer for all; with
  `make_overhang_printable` the growth down to it is the first group's);
  `seam_slope_start_height` given in mm and the height a seam is placed
  at go by the step from the layer before where objects of their own
  heights share a layer. Parity (`edge-parity`, each object's layers and
  every Z the same as the reference's, feature lengths within 0.1% but
  sparse fill, which is 1% short here as on a plain cube): on the
  Snapmaker U1 and the Elegoo Centauri Carbon, two 10 mm cubes at 0.2 and
  0.12 mm (50 and 82 layers on the U1, 50 and 83 on the Carbon, 115 and
  116 layers of G-code); a 20 mm cube with a 5-10 mm range at 0.1 mm
  beside a plain one (125 and 50); a cone with a variable profile beside a
  cube (117 and 50).

Known gaps: typed settings (values are text until a stage needs them),
an object's own supports and seam position, and the
feature filaments (`wall_filament`, `sparse_infill_filament`,
`solid_infill_filament`).

- Infill: each layer's inside is split into top, bottom, bridge, solid
  shell and sparse areas (`top_shell_layers`, `bottom_shell_layers`,
  `sparse_infill_density`, `minimum_sparse_infill_area`) and filled with
  lines, or for sparse areas one of nineteen patterns (below). Solid lines run to the edge of their
  area and, where one is joined to the next, turn half a line spacing
  inside it, as the reference's do. A row that a hole parts in two is
  laid as one line, round the side of the hole, where the outline
  leads from one part to the other without crossing another row: on
  the first row to meet the hole and on the last, always when the
  lines are swept in order, and otherwise when there is no row beside
  it left to go on to (FillRectilinear.cpp, the joins along the outline
  on one line). For solid fill such a join is no longer than three
  line spacings, and for a bridge as long as the side is. The reference
  means that limit and does not keep it (Fill.cpp works it out from a
  spacing not yet set, which leaves it at nothing, and nothing is no
  limit; fixed there in aceRage/EdgeSlicer#968), so until its fix a
  bottom surface with round holes in it is a little shorter here:
  4.30 m against 4.32 on `holes` and 0.51 against 0.52 on the `tube`,
  which were the reference's length to the tenth of a millimetre with
  no limit kept (4315.5 mm and 519.8 mm in both, from 18.6 and 7.2 mm
  short before rows were joined round holes at all). Sparse lines round the
  holes of `holes` and `counterbore` are within 0.1% (`rectilinear`;
  1.4 and 1.7% short before). `top_surface_pattern`,
  `bottom_surface_pattern` and `internal_solid_infill_pattern` say whether
  lines are joined (`monotonic`, `rectilinear`) or left separate
  (`monotonicline`); any other pattern is laid as joined lines, with a
  note. Both monotonic patterns are swept from one side of the area to
  the other, so that no line goes down between two already laid, which
  shows on a surface: the area falls into regions that can each be
  zigzagged in one go, and a region waits for everything beside its
  first row on the side already swept (FillRectilinear.cpp
  `generate_montonous_regions`). The writer lays such lines in the
  order given and never turns one round. Top and bottom surfaces and solid fill inside the part are fitted to their area: the lines are
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
  colour under it. Each patch is run out, the way its lines go, to the
  sparse lines that carry it, a thread's width past the first line at
  either end, into the sparse and solid fill beside it: the sparse
  lines of the layer beneath are laid for this as they will be laid,
  from that layer's sparse surfaces (the reference lays them again,
  `generate_sparse_infill_polylines_for_anchoring`; here they are laid
  only under layers with something to bridge, and are not printed).
  Its direction is chosen by those lines: every 2 mm or so along its
  outline the nearest line votes for running square to itself, and a
  patch run out near one already made takes that one's direction; for
  a Hilbert curve or octagram spiral it is square to the pattern's
  direction, for 3D honeycomb and cross hatch 45° on from it, and
  `internal_bridge_angle` sets it or, with `relative_bridge_angle`,
  turns it. A patch is kept to where the sparse fill beneath is deep
  enough for a thick bridge's thread to sink into and not bridged
  within that depth already. Under the edge of such a bridge, where the
  edge runs along the walls of the layer below, a band of that layer's
  sparse fill is made solid to carry the ends of its lines
  (PrintObject.cpp `bridge_over_infill`).
  With `ensure_vertical_shell_thickness` at `ensure_all`, whatever of a
  layer's fill is not over and under the fill of the layers within a
  shell of it is made solid too, so a leaning wall stays as thick
  measured straight up as it is sideways; walls between two filaments
  inside a part do not count (`discover_vertical_shells`). The faces
  a shell is made from are first grown by a twentieth of a solid
  line's spacing, as the reference grows them, so that the faces of
  neighbouring layers on a slope, which meet a hair apart, make one
  shell and not strips each too narrow to keep; and a face that is its
  whole shell (`top_shell_layers` 1 with `top_shell_thickness` 0, or
  the same over a bottom) is held at its edges, the fill an outer
  wall's spacing round it on the layer over a top or under a bottom
  made solid. Bambu Studio's projects say `enabled` for `ensure_all`
  (older ones `1`), and are read so, as the reference reads them
  (Format/BambuKeyAliases.cpp). Any other value of that setting is
  noted and not followed.
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
  so its lines go from one support to the other. A bridge's lines,
  over air or over infill, are laid as the reference lays them
  (Fill.cpp `group_fills` and `Layer::make_fills`, FillRectilinear.cpp
  `fill_surface_by_lines`): as solid lines are, running to the edge
  and turning half a spacing inside it, with no line that does not
  come half a spacing inside the area; at their own spacing over the
  density, never spread to fit the area; on the lines every pitch from
  the middle of the object, half a pitch off those at full density;
  joined, and swept in order where the top surface is
  (`top_surface_pattern` `monotonic` or `monotonicline`). On the
  `bridge` fixture with the U1's presets the deck's 21 bridge lines lie
  where the reference's do and end where they do, to the micrometre,
  666.2 mm in both; laid from edge to edge, as they were, there were
  22, each 0.14 mm longer at either end and 0.13 mm to one side, and
  703.6 mm. Bridges, bottoms and
  tops reach into the fill beside them, to have something to hold on
  to: a wall's spacing into sparse fill and the width of the walls
  times the square root of two into solid fill, grown in the
  reference's waves within each kind of fill, its touching pieces
  taken as one, so that a surface never jumps a wall into fill it does
  not touch, and bridges that grow into one another become one
  (LayerRegion.cpp `process_external_surfaces`,
  `expand_merge_surfaces`, `expand_bridges_detect_orientations`).
  The bridging options of the reference's Quality tab are followed.
  `bridge_line_width` gives bridges, bridges over infill and
  walls over air a width of their own: a round thread is that thick in
  place of the nozzle's bore, a flattened line that wide before
  `bridge_flow` thins it; wider than the nozzle, or no wider than the
  layer is tall while either kind of bridge is flat, is an error, as in
  the reference (`LayerRegion::bridging_flow`, `Print::validate`).
  `bridge_expansion_extra` carries a bridge that many millimetres
  further onto solid fill and onto a top or bottom surface beside it,
  in the same waves (`expand_bridges_detect_orientations`,
  Algorithm/RegionExpansion.cpp); sparse fill beside a bridge is
  reached no further, in the reference either. `dont_filter_internal_bridges` at `limited` and `nofilter`
  lets smaller patches of solid fill over sparse fill be bridged (the
  first sorting-out of `bridge_over_infill` with one spacing where the
  default has three, and with no filter every patch found, four
  spacings round it), and run out as any other. `enable_extra_bridge_layer` lays a second layer of bridge
  over bridges over air (the same way as the first, and the bottom
  shell then starts from the second), over bridges over infill (square
  to the first), or both. `counterbore_hole_bridging` is for a hole
  that narrows on the way up, a counterbore printed wide end down,
  whose narrow part would start with walls laid in the air. On the
  first layer of the narrow part, what has nothing under it and has
  the narrow hole on its edge is taken out of what gets walls and laid
  as a bridge, resting a wall's width on the wide hole's edge, and the
  layer's walls go round the wide hole (PerimeterGenerator.cpp
  `process_no_bridge`, with a port of `BridgeDetector`, classic walls
  or Arachne). `partiallybridge` bridges only what lines can span from
  one side of the wide hole to the other, the way that spans most: a
  slot as wide as the narrow hole stays open right across, the rest of
  that layer over the wide hole is not printed, and the layer above
  closes the slot down to the hole. `sacrificiallayer` bridges all of
  it, the narrow hole covered over as well, to be drilled out, where
  the wide hole's outline is convex. With `bridge_angle` set every
  such bridge is laid that way, or turned by it from the direction
  found (`relative_bridge_angle`). Each is off by default and changes
  nothing then.

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
- Z contouring (`zaa_enabled`, "Z anti-aliasing"): top surfaces,
  their ironing and the walls round them are laid not flat at their
  layer's top but at the height of the model's own surface under each
  point, within the layer, so a shallow slope or a curved top prints
  without its stair steps. A port of the reference's ContourZ.cpp
  (`edge-contour`). Every layer but the first is sliced `zaa_min_z`
  above its bottom rather than through its middle (a `zaa_min_z` over
  the layer height is refused, as the reference refuses it). Once the
  fill and ironing are made, each top surface line, ironing line and
  wall of every layer but the first is cut into points 0.1 mm apart
  and a ray is cast straight up from each, from the slicing plane, to
  the parts' surface, the triangles in a tree and the layers on as
  many threads as there are. How far that is above or below the
  layer's top is the point's height: a top surface down to the
  slicing plane and up by `zaa_min_z`, fading to nothing over the
  0.03 mm above that; ironing a layer either way; a wall never up,
  and on a surface sloped more than `zaa_minimize_perimeter_height`
  degrees let down the slope by half its width, faded in over the
  20 degrees above. The heights are smoothed along the line (three
  points either side), a line that moves less than 10 µm all along is
  left flat, and points in line with the two before them in three
  dimensions are dropped. The writer lays a contoured line point to
  point with a height on every move and no arcs, leaves it unthinned,
  lays each move as much as the line is tall there (the mean of its
  two ends' heights; ironing keeps its flow) and, with
  `zaa_speed_scaling`, that much slower, never faster than the line's
  own speed nor slower than 10 mm/s, and held to the hot end's limit,
  a new speed only where the height has moved 5 % from the one that
  set the last. With `zaa_dont_alternate_fill_direction` fill and
  ironing keep one direction on every layer. A wall of changing width
  is contoured as a whole loop, where the reference contours each of
  its pieces of one width by itself. As in the reference nothing is
  contoured with `spiral_mode` or `dithering_local_z_mode` on, and
  each says so. Four things are not as in the reference, each a
  mistake there: a contoured wall is closed with the seam gap and no
  scarf (the reference lays a scarf's way up and way down flat along
  the contour at full flow, the joint twice); ironing keeps its speed
  (the reference slows it for its height over the ironing line's own,
  down to 10 mm/s); a wipe after a contoured line goes back along it
  at the heights it was laid at (the reference wipes at the height
  the line ended at, back up a slope through what it has just laid);
  and a travel never goes down on the way, the nozzle coming down to
  a line that starts lower once it is there (the reference travels
  straight from one height to the other). One is noted: where a layer
  is slowed to cool, a contoured move slowed for its height is held to
  the layer's cap like any other line, where the reference slows such
  moves in proportion. With `zaa_enabled` off a file is byte for byte
  what it was.
- G-code: `slice --gcode out.gcode` orders each layer (skirt and brim,
  each part's brim whole before the next, then island by island: an
  island's walls from the inside out, each beside the last, then the
  gap fill and the fill that lie in it), holds speeds to the
  hot end's melt rate, sets acceleration by feature, and reports filament,
  weight and time. Following the reference:
  - Seams: every point of every outermost wall, as it is written
    (without arcs thinned to `resolution`, which is done to the walls
    of the whole print first), is weighed as a place
    for its seam before the file is written, as the reference's
    SeamPlacer weighs it, object by object and in each object's own
    coordinates. First, how far the wall's line hangs out over the
    layer beneath there (the middle of the line outside that layer's
    outline, with 0.65 of the line's width, less the layer's height):
    of two points either of which hangs out, the one that hangs out
    less is the better. Then whether the point lies hidden inside the
    layer's own outline, more than half a mm, which it can where two
    materials meet. Then a score of two parts. One is how much of the
    part's outside can be seen from there: 30000 points are scattered
    over each object's mesh, 25 rays are cast from each over the half
    of the sky it faces, and a point is seen less by a 25th for each
    ray that runs into the part; a place on a wall takes the measure
    of the points within reach of it (`edge-visibility`, parallel
    across the points and the same at any thread count; volumes cut
    out of an object, its negative parts, are let through). The other
    is the angle the wall turns through there, taken between the
    points an arm of the nozzle's width either side, least where it
    folds into the part and most on a straight run; it counts for 0.6
    beside the first, and whole for the nearest corner. With
    `seam_position` at `aligned` the best score takes the
    seam; at `nearest` the distance from where the nozzle is counts
    beside it, and the seam is picked as the wall is laid; at `back`
    the point furthest back takes it whatever its score (and nothing
    is measured of what is seen); `aligned_back`, `aligned_front`,
    `left` and `right` lean to that side of the bed, a point of the
    surface that faces the other way counting as easier seen by up to
    as much again; `random` picks a
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
    behind another. Walls stop `seam_gap` short of their start. For
    every position but `nearest` and `random` the seams are then lined
    up as the reference lines them up: taken best first, each seam not
    yet in a line gathers one, upward and downward, of the seam or a
    point no worse by much (0.3 of the score, a tenth of the line's
    width of overhang, 1.5 mm at the back) within four line widths on
    each next layer; a line of six or more has a smooth curve fitted
    through it (a cubic B-spline of one stretch to every 4 mm of line,
    each seam counting for more the sharper its corner), and each seam
    is moved to the curve, the less the sharper its corner, a corner of
    55 degrees or more not at all. What differs by intent: where a
    line stops short of the top layer the reference, turning to look
    downward, steps down twice and leaves the layer beneath the line's
    first seam out of it (a slip; a line of twenty seams or more is
    seldom put right by its second tries), and here that layer is in
    the line; a wall is
    measured against everything on the layer beneath, not its own
    object's layer alone; a loop opened at a seam that is not one of
    its points is opened at the nearest point of the loop, where the
    reference takes the nearest of its stretches that rest; and a mesh
    is measured for visibility as it is, where the reference first
    thins one of more than 16000 triangles to that many by collapsing
    its short edges, for speed, so that on such a part the points
    scattered are not the reference's own and the measure differs by
    what the thinning loses.
  - Seams where parts meet (`seam_prefer_part_joints`, on unless turned
    off), at the aligned positions (`aligned`, `aligned_back`,
    `aligned_front`, `left`, `right`; not `back`, `nearest` or
    `random`): an object of several parts (two bodies, a multi-colour
    model built of parts) has lines on its outside where one part meets
    another, and a seam hides in such a line. Each part's mesh is taken
    by itself (embossed text and pictures are left out), and every
    triangle of it that does not face up or down and comes within
    0.5 mm of another part is cut into pieces no wider than a spacing
    (0.15 mm, widened to keep to 300000 pieces); the middle of a piece
    is a point of the joint where it lies within 0.2 mm of the other
    part's surface, facing it, or just inside it. An object that is the
    only copy of its object on the plate, and that is not the same as
    another object, counts other objects stood against it as parts too.
    A point of an outer wall within a line's width and half the spacing
    of a joint point is on the joint, and an edge of the wall near one
    is given a place every 0.2 mm along it, since a joint crosses a long
    edge far from its corners. The middle of each run of joint points
    round a wall (a point hidden inside the print where the run has
    any, as where two materials meet, else a sharp inward corner, else
    the middle by length; a point that hangs out splits a run, and a
    wall on a joint all the way round has none) beats any other point,
    is followed from layer to layer when seams are lined up, counts
    three more in the curve fitted through a line of seams, and is
    moved to that curve by half a line's width at most. Points are
    found triangle by triangle and kept in order, so the joints are
    the same at any thread count; the search for them runs beside the
    visibility measure. A port of SeamPlacer.cpp `gather_part_joints`,
    `mark_part_joint_centers` and their place in the comparators and in
    `align_seam_points`. Not as the reference: of two triangles of the
    other part equally near a point (its nearest place is on their
    shared edge), the one of the lower number is taken, where the
    reference takes whichever its tree reaches first, so near the edges
    of a part the points differ (on plate 2 of `mcbrim_test.3mf`
    159500 points an object here, 148500 there); and it also leaves
    other objects out when it prints object by object, which this
    engine does not do, and asks that two objects it takes for the same
    have the same settings, which are not read here.
  - Seams painted on or off (`paint_seam` on a project's triangles,
    read as colour paint is read, from any part of the object): at every
    position, a point of an outer wall with a triangle painted to keep
    seams off within a line's width of it is blocked, else one with a
    triangle painted to have them within that is wanted; an edge of the
    wall with a wanted triangle within its own length of its first point
    is given a place every 0.2 mm; and of each wall's longest stretch of
    wanted points the middle one, or the middle of its sharp corners,
    is marked. That point beats every other at the aligned positions; at
    every position a wanted point beats one not painted, which beats a
    blocked one, before anything else is weighed (joints, overhangs and
    the score), so a seam goes where it is painted on even at the back
    or anywhere at random. When seams are lined up the marked point is
    followed from layer to layer, and a wanted point counts three more
    in the fitted curve and is moved to it by at most three fifths of
    the way. A port of SeamPlacer.cpp `gather_enforcers_blockers`,
    `is_enforced`, `is_blocked`, the end of
    `process_perimeter_polygon`, and the paint's place in the
    comparators and in `align_seam_points`.
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
    beneath, as the reference measures it (ExtrusionProcessor.hpp). A
    stretch whose middle is further out than that outline grown by half
    a nozzle hangs clear of it and is an overhang wall, printed as a
    bridge (`detect_overhang_wall`), and is cut out of the wall where
    the reference cuts it. One partly supported slows between the four
    overhang speeds by how much of its width has something under it
    (`enable_overhang_speed`). It is measured at its own points, where
    it crosses the outline beneath, at probes inside a line of 2 mm or
    more and at up to two cuts beside the ends of such a line, and
    nowhere else; a line goes at the slower of its two ends, and a
    speed within 1 mm/s of the last one written is not written.
    Bridges, overhang walls and stretches past `overhang_fan_threshold`
    get `overhang_fan_speed` where that is more than the layer's fan.
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
  - Paths without arcs: with arc fitting off, every wall, fill line
    and support line is thinned to `resolution` before it is written,
    as the reference thins them (PrintObject.cpp
    `simplify_extrusion_path`, with its own Douglas-Peucker,
    MultiPoint.cpp `_douglas_peucker`): a point within `resolution` of
    the line between those kept is left out, and with a `resolution`
    of nothing only a point exactly on that line. Sparse infill and
    supports are held to `resolution` like the rest; their looser
    limits are for arcs. A wall loop is thinned from its own first
    point round to that point again, which therefore always stays;
    the seam is chosen among the points left and the loop opened
    there, and nothing is thinned after that. A wall whose width
    changes is thinned piece by piece as it is cut into its widths.
    Skirt, brim and prime tower are written as made, as there. Two
    things are not as in the reference: a wall is thinned whole and
    then measured against the layer beneath, where the reference
    first cuts it where it hangs clear and thins each stretch; and a
    loop no wider than `resolution` is left as made, where the
    reference lays the two points left of it as a line out and back.
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
  - Tree (`tree(auto)`, which the U1 profiles use): a port of the
    reference's organic trees (TreeSupport3D.cpp, TreeModelVolumes.cpp),
    what `tree(auto)` with the default style builds. Overhangs as
    TreeSupport::detect_overhangs finds them (sharp tails, cantilevers,
    small clusters dropped); roofs under them and tips along the lines a
    support fill would have there; every branch carried down as the whole
    area its centre could be in, kept out of the part and out of wherever
    it could not get down from (collision and avoidance for each radius,
    worked out up front), areas that meet merged; centres picked from
    the bed up, standing on the part where the bed is out of reach;
    centres smoothed and pushed clear of the part as balls; each branch
    built as a tube of circles with round ends and sliced, so it thickens
    towards the bed and spreads to 7.5 mm there. Branches are hollow
    loops, each started from a hook laid inside it
    (SupportCommon.cpp `tree_supports_generate_paths`); a foot on the
    part is dense; on the bed the trunks spread out in steps by
    `raft_first_layer_expansion`. The code is in
    `crates/edge-support/src/tree/`, one module a step.
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
    was pulled when last put away (on a printer of one nozzle, as far as
    the change pulled it back: the one retraction is shared, Extruder.cpp
    `m_share_retracted`). The change's retraction is one, the wipe within
    it (`retract_for_toolchange`). Where the tool is changed over the
    prime tower it is changed where the change's block starts, at the
    layer's height with the filament pushed back first, as the reference
    does (`append_tcr2`). The next tool starts warming `preheat_time`
    ahead by the file's own timing, looking back no further than the last
    time that tool was taken up or the start of the print, and a cooldown
    of it on the way is taken out (`; removed M104`), so a tool wanted
    again within `preheat_time` is not cooled at all (GCodeProcessor.cpp
    `process_line_T`); a tool not needed again is switched off. After the
    change G-code the fans go back to the layer's speeds.
  - Filament presets are named one for each filament (`--filament`
    given again for filament 2, and so on). One alone is used for every
    filament, and a filament past the last named takes the last; a note
    says so either way. Many filament presets hold two values of a
    setting, one for a standard nozzle and one for a high-flow nozzle
    (`filament_flow_support`, or a Bambu preset's
    `filament_extruder_variant`); each filament takes its own preset's
    value for the flow it prints at (`filament_volume_type`, standard
    unless set), and every per-filament list comes out one entry per
    filament, as the reference's command line writes it given presets by
    name (`edge_config::variants`, whose doc says what the reference
    does). A project the reference's application saved with those
    values side by side (`filament_flow_step_size`) is read the same way,
    and a printer laid out per extruder and nozzle variant (Bambu's H2D)
    has each extruder read its own nozzle's retraction values. A
    filament set to high flow whose preset has no high-flow values, and
    a process preset's own high-flow speeds, which are not followed, are
    noted. A model printed in a
    filament its settings say nothing of gets no G-code and a sentence
    saying which. Printer scripts are given a bed temperature for every
    filament.
  - A project's filament list can be changed (`Model::add_filament`,
    `remove_filament`, `set_filament_colour`; the desktop app's "Add one
    filament", "Remove last filament", "Delete", "Merge with"). A filament
    added is the last one again (its preset and, for a project on its own
    settings, every one of its values; `edge_config::filaments::reslot`
    lays the project's lists out for the new list); one taken away
    renumbers what printed in the filaments after it, parts, objects,
    painted colours, plates' orders and filament changes at layers, as the
    reference's `on_filaments_delete` does. The flushing volumes are kept
    the size of the list, a new or recoloured filament's worked out from
    the colours (`edge_engine::flush`, FlushVolCalc.cpp). Whether there is
    a prime tower is the process's `enable_prime_tower`, as there: the
    reference turns nothing on when a second filament arrives.
  - A printer of one nozzle (`single_extruder_multi_material`: the
    Centauri Carbon, Bambu Lab's A1) is given the flushing volume from
    the filament put away to the new one at each change (`flush_length`,
    `first_flush_volume`, `flush_length_1` to `_4`, GCode.cpp
    `set_extruder`), so its own G-code purges what the reference's does
    (`M6211 … L28.4374`, `M620.10 A1 … L137.614`), and changes filament
    as the reference's tower does there (WipeTower2.cpp, Print.cpp
    `_make_wipe_tower`): where the project purges in the tower
    (`purge_in_prime_tower`, off in the Centauri Carbon's profile and
    ignored on a Bambu Lab printer) each change lays down that volume
    times `flush_multiplier` (at least `filament_minimal_purge_on_wipe_tower`)
    and each block is planned from its own volume; otherwise the tower
    takes `prime_volume`, the printer's own G-code purging the flush. The
    tools on a layer are taken in the order that flushes least where the
    reference orders them so (purging in the tower, or a Bambu Lab
    printer). The new filament's temperature is set with `M104` and not
    waited for: in the tower before the change, on the first layer and
    wherever it differs from the last (`toolchange_Unload`), or after the
    change with no tower; a Bambu Lab printer's tower heats the nozzle
    back after its change G-code cooled it (`M104 T0 S220 N0`). The
    change G-code is given `max_layer_z` as the highest layer begun.
    The flushing volumes and multiplier are set in the desktop app's
    "Flushing volumes" dialog, and a project's own come over a printer
    profile's. Not done, and said in a note: ramming, cooling and
    loading the filament on the tower (`enable_filament_ramming`, which
    neither printer uses), and flushing into infill, objects or support.
  - Which prime tower a print gets is chosen as the reference chooses it
    (Print.cpp `wipe_tower_type`): a printer whose `printer_model` (or,
    failing that, preset name) starts "Bambu Lab", or one set to
    `wipe_tower_type` `type1`, gets the reference's Type1 tower,
    WipeTower.cpp, Bambu Studio's; every other printer the Type2 tower,
    WipeTower2.cpp, described below. The Type1 tower (`edge-tower`'s
    `bambu.rs`) is laid out as the reference's, in single precision as
    there: every tool change a block of whole rows across the tower
    holding `prime_volume` (the change G-code flushes; the tower only
    primes), in the order the changes come; the first layer's rows 15%
    fuller, and a row over the open part of the layer below laid as
    thick as a 0.2 mm layer's; each row faster than the one before up
    to 80 mm/s (`wipe_tower_max_purge_speed` at most), from a third of
    it, or of the first layer's speed; the wall laid by a layer's first
    change before its block, from a corner that goes round the tower a
    layer at a time, and after the block the frame round the depth no
    change uses and lines across it 10 mm apart (solid on the first
    layer); a layer with no change laid whole by the nozzle in hand;
    a ribbed tower squared to the purge area it had at the width set,
    and a tower too shallow for its height spread to depth, or with
    ribs, given longer ribs; the brim round the wall, a loop narrower
    each layer up to 3 mm, "auto" (-1) by the print's height; the wall
    and brim thinned to 0.1 mm, or fitted with arcs within it where
    `enable_arc_fitting` is on. Its moves are written as the reference's
    tower writer writes them: travel inside the tower at the speed in
    force, `G4 S0` when the new filament is in and when the block is
    done, no acceleration and no temperature of its own (the change
    G-code's, and its "reheat before wipe"), the filament changed where
    the nozzle is and the tower reached after. Bambu Studio's names
    (`prime_tower_rib_wall`, `prime_tower_rib_width`,
    `prime_tower_extra_rib_length`, `prime_tower_fillet_wall`,
    `prime_tower_max_speed`) are read for it, as the reference reads
    them from a Bambu project. Not done, each said in a note where set:
    a smooth timelapse's tower on every layer, sparse layers combined
    into one (`wipe_tower_sparse_layers_combination`), the tower's own
    acceleration (`prime_tower_acceleration`) and tower interfaces
    (`wipe_tower_interface_*`, `enable_tower_interface_features`). Not
    as the reference, on purpose: a plain (rectangle) wall is laid at the
    tower's wall speed, where the reference writes it with no speed and
    it goes at whatever the change G-code left in force (30 mm/s on an
    A1); and a layer whose every change is to a soluble or support
    filament lays its frame and wall after its first block, where the
    reference lays them before the change with the nozzle put away.
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
    stopped (`toolchange_Unload`, `set_toolchange`), pushed out at
    `ramming_pressure_advance_value` where `enable_change_pressure_when_wiping`
    asks (`SET_PRESSURE_ADVANCE`, `M572` or `M900` by the firmware, not
    set back by the tower, as there). The wall is left
    open for 2.5 mm where each new tool comes in, and the tool comes in
    that way (`wipe_tower_wall_gap`), lays 3 mm, is pulled back and irons
    the tower flat in a square spiral round where it is
    (`filament_tower_ironing_area`) before it lays its first row. Each
    row of a block goes down faster than the one before, from a third of
    the speed the tower is laid at (the first layer's, or
    `wipe_tower_max_purge_speed` held to the infill's) up to all of it
    (`toolchange_Wipe`); with `wipe_tower_stagger_toolchange_start` each
    change starts its rows somewhere else in its block, in turn. The
    tower's lines lay their own cross-section, no flow ratio applied, as
    the reference's tower writes them. A tower whose saved place no
    longer fits it is moved onto the bed as the reference's command line
    moves it (`estimate_wipe_tower_polygon`), and said in a note.
- Printer profiles: the machine's start, end and layer-change G-code and the
  filament's start and end G-code are expanded from their templates, with
  filament-level retraction overrides applied. For the Snapmaker U1 projects
  in the reference, the expanded start G-code is identical to the
  reference's on every line. A bare model exports with a printer's own
  G-code when `--machine`, `--process` and `--filament` name its presets.
- G-code set at layers (the layers slider's "Add Pause", "Add Custom
  G-code", "Add Custom Template" and "Change Filament", kept per plate in
  `Metadata/custom_gcode_per_layer.xml`): read, kept with the plate
  through edits, and written back in the reference's own layout, numbers
  at 17 digits as it writes them (the pressure advance pattern's file is
  written back to the byte). At the layer an item is set at (the layer
  whose top is nearest, the highest item where several are; ToolOrdering.cpp
  `assign_custom_gcodes`), after the layer change, the second layer's
  temperatures and before anything is printed, a pause writes
  `;PAUSE_PRINT` and the printer's `machine_pause_gcode`, a template
  `;CUSTOM_GCODE` and the printer's `template_custom_gcode`, both expanded
  with what the reference gives them (not the layer's own number or
  height, which a template is refused for naming, as there), and custom
  G-code `;CUSTOM_GCODE` and its text as it is (GCode.cpp
  `emit_custom_gcode_per_print_z`). A filament change, on a project of
  several filaments whose plate prints in one of them, prints every layer
  from it up in the filament named, with the tool change, temperatures
  and prime tower any change of filament has here (ToolOrdering.cpp's
  `extruder_override`); on a U1 and a Centauri Carbon the change comes at
  the reference's layer with the reference's `change_filament_gcode`.
  What the reference leaves out is left out and said in a note: a filament
  change on a plate of several filaments or a project of one, a colour
  change (the reference writes no `M600` and an empty `;CUSTOM_GCODE` in
  its place, which is not copied), an item below the
  first layer or beaten to a layer by a higher one; a pause on a printer
  with no pause G-code is written as the reference writes it and said.
  The time estimate stops the head at a pause, as the reference's does
  (`simulate_st_synchronize`); the pause itself takes no time a file can
  know. On a 20 mm cube with a pause, a custom line, a template and a
  filament change, on the U1 and on the Centauri Carbon, the lines written
  at each layer are the reference's; the reference's own files with them,
  timed here, come to its own time. Not as the reference: its G-code
  writer rewrites the feed rates inside a custom block (a `G1 F1800`
  already in force taken out, leaving an empty line); here custom G-code
  is written exactly as given. A Bambu Lab printer's file has Bambu
  Studio's tags (`; PAUSE_PRINTING`, `; CUSTOM_GCODE`), as it has for
  every part of the file (below).
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

**Changing filament on one nozzle, against the C++ slicer** (2026-11-12,
`edge-parity`, the reference's file read the same way). Before is the
engine as it was; after, this pass. Tower length is the prime tower's
path; flushed is the purge each file asks of the printer
(`M6211`/`M620.10 … L`, mm of filament per filament).

| project | tower m, reference / before / after | flushed mm, reference / after | temperatures at changes, before / after | time, reference / before / after |
| --- | --- | --- | --- | --- |
| `twotone` 2 filaments, Centauri Carbon (its own profile: the printer purges) | 15.39 / 15.72 / 15.72 | 284.4 + 521.4 / the same | `M109 S210 T1` at every change / `M104 S210` on the first layer only, as the reference | 23m 13s / 22m 29s / 22m 21s |
| the same, `purge_in_prime_tower` on | 29.76 / 15.72 / 30.23 | the same | as above | 26m 21s / 22m 29s / 25m 31s |
| cube grown to 2 filaments, Centauri Carbon | 34.27 / 35.10 / 35.10 | 710.9 + 1303.4 / the same | as above | 46m 56s / 45m 04s / 44m 39s |
| figurine, 4 filaments (3 used), Bambu Lab A1 | 296.37 / 313.04 / 313.61 | 30297.5 + 11422.0 + 7473.6 / the same (before: 29109.2 + 12688.8 + 8005.7) | 380 waits `M109 S220 Tn` / none; 380 `M104 T0 S220 N0` reheats, as the reference | 14h 45m / 15h 08m / 14h 41m |

The changes are now the reference's in number and order on all four
(the figurine had 380 against 381 and another order from the 190th on).
With the matrix changed in the "Flushing volumes" dialog (filament 1 to
2 doubled, the multiplier at 0.5) both slicers move together: `twotone`
with the tower purging, placed where it fits (`wipe_tower_y` 150),
29.76 → 73.77 m in the reference and 30.23 → 75.03 m here, 26m 04s →
41m 19s against 25m 12s → 40m 13s; the figurine's flush of filament 1
30297.5 → 14260.7 mm against → 14157.2, 14h 45m → 13h 16m against
14h 41m → 13h 06m (here 379 changes against 381: the order differs on
two layers). The gaps left then (rows and the way in, a one-nozzle
tower 2% too long, the totals, a tower that outgrew its place) are
closed in the next pass, below; the Bambu Lab tower (WipeTower.cpp),
then laid out as WipeTower2's and 6% longer, is its own since (below).

**The prime tower against the C++ slicer** (2026-11-12, `edge-parity`;
both files timed here, each the way its printer plans it). Before is
`main` at a6d0c69, after is this pass. Tower length and extrusion are
the tower's lines; tower time is what its moves take, the way in
included. The U1 fixtures are made with `tools/make_fixture.py` from
`mcbrim_test.3mf`'s settings (`halves`, `inset` with their second part in
filament 2; `twotone` again with the stagger on, and with every filament
rammed).

| project | tower m, ref / before / after | tower E mm, ref / before / after | tower time s, ref / before / after | print, ref / before / after |
| --- | --- | --- | --- | --- |
| lettered plate 1, U1 | 25.89 / 25.89 / 25.89 | 983.9 / 964.2 / 983.9 | 539 / 417 / 516 | 1h07m42 / 1h02m22 / 1h04m00 |
| lettered plate 2, U1 | 39.87 / 39.87 / 39.87 | 1515.5 / 1485.2 / 1515.5 | 892 / 646 / 840 | 43m38 / 41m03 / 44m17 |
| lettered plate 3, U1 | 39.87 / 39.87 / 39.87 | 1515.5 / 1485.2 / 1515.5 | 898 / 653 / 847 | 43m43 / 41m10 / 44m24 |
| `twotone`, U1 | 19.74 / 19.74 / 19.74 | 750.2 / 735.1 / 750.1 | 384 / 304 / 362 | 13m46 / 12m45 / 13m44 |
| `halves`, U1 | 41.84 / 41.84 / 41.84 | 1590.3 / 1558.3 / 1590.1 | 818 / 614 / 759 | 23m30 / 20m49 / 23m14 |
| `inset`, U1 | 35.23 / 35.23 / 35.23 | 1338.9 / 1312.0 / 1338.8 | 687 / 522 / 641 | 21m51 / 19m48 / 21m47 |
| `twotone`, U1, staggered | 19.71 / 19.74 / 19.71 | 749.1 / 735.1 / 749.0 | 395 / 304 / 368 | 13m57 / 12m45 / 13m51 |
| `twotone`, U1, rammed | 18.65 / 18.65 / 18.65 | 772.7 / 757.1 / 772.6 | 374 / 323 / 381 | 13m49 / 12m50 / 13m48 |
| `twotone`, Centauri Carbon | 15.39 / 15.72 / 15.39 | 584.8 / 567.8 / 584.8 | 293 / 234 / 286 | 23m25 / 22m21 / 23m13 |
| the same, purging in the tower | 29.76 / 30.23 / 29.76 | 1131.0 / 1092.3 / 1130.9 | 481 / 422 / 480 | 26m15 / 25m12 / 26m10 |
| cube grown to 2 filaments, CC | 34.27 / 35.10 / 34.27 | 1302.5 / 1268.0 / 1302.4 | 647 / 506 / 635 | 47m10 / 44m39 / 46m47 |
| figurine, Bambu Lab A1 | 296.37 / 313.61 / 313.61 | 11307.2 / 11680.8 / 11919.2 | 5194 / 4810 / 5721 | 14h43m / 14h41m / 14h56m |

Where the tower's time went, in order of size: the ironing as a nozzle
comes in through the gap (about 2.2 s a change: 58 s of lettered plate
1's 122 s, 102 s of plate 2's 246 s), the rows at the reference's rising
speeds (a third, three eighths, eleven twenty-fourths, seven eighths of
the speed, then 50 mm/min a row more: 41 s and 92 s), the one-nozzle ramming width
(the Centauri Carbon's tower a line wider and 2% longer), the flow ratio
the tower's lines were laid with (2% of the U1's tower plastic, 5% of
the Centauri Carbon's), and the staggered start where it is asked for.
On the three lettered plates nothing but the tower moves: every other
feature's lines are the same, line for line; travel grows 0.8 to 1.4 m
(the ironing and the way in), and the totals at the end of the file
with the tower's plastic. What is left of the time between the files on
those plates is outside the tower. The figurine's tower is now 10% slower
than the reference's: its rows are at Bambu Lab's speeds, but on
WipeTower2's layout, which is 6% longer (WipeTower.cpp lays a block a
filament, by each filament's `filament_prime_volume` and adhesion
category, with a perimeter round each, its rows into the wall by
`wipe_tower_wall_infill_overlap` and spaced by `prime_tower_infill_gap`,
first-layer rows 15% fuller and rows over the open part of the layer
below laid as a thick bridge; not ported, and said in a note).

The file's filament totals now count what the printer's own G-code
flushes, as the reference's do: the Centauri Carbon's `M6211` purges
and Bambu Lab's flush between `; FLUSH_START` and `; FLUSH_END`, each
the new filament's (`twotone` on the Centauri Carbon, filament 1:
1290.65 mm there, 763.40 before, 1247.78 now; the rest is the part).
A print that never changes filament is left as it was, the start
G-code's 200 mm purge on the Centauri Carbon not counted, where the
reference counts it. The twotone cube purging in the tower with one
flush doubled and the multiplier at 0.5 was refused as off the bed; the
tower is now moved from 15 x 220 to 15 x 192.194, as there, and is the
reference's length (73.77 m). Still not done: the tower turned end for
end on alternate layers.

**A prime tower without sparse layers** (`wipe_tower_no_sparse_layers`,
2026-11-12). Both towers leave out the layers with no tool change and
stand only as tall as the layers they lay, each layer's top the sum of
the layers laid below (WipeTower.cpp `compute_compacted_wipe_tower_z`);
a smooth timelapse keeps every layer. Bambu's tower (WipeTower.cpp)
starts on the first layer with a change, which is solid and has the full
brim; the other (WipeTower2.cpp) keeps the print's first layer as its
own, laid or not, and lays its lines and frame at the first layer's
speed while no more than one change has been made, as there. The writer
takes the nozzle over the tower at the layer's height, straight down to
the tower's layer, back up before the change G-code and down after it,
and straight up before it leaves for the part (GCode.cpp `append_tcr`,
`append_tcr2`); a test checks that no move made below the layer crosses
the part. A plate where the toolhead would then meet what is printed is
refused with the reference's words, before slicing from the tower's
estimated size and each object's height, and after from what the tower
lays and how far each object rises above it (Print.cpp
`compacted_wipe_tower_clearance_valid`,
`validate_compacted_wipe_tower_clearance`): the nozzle's cone, the head's
radius, the rod in the tower's band across the bed and the lid.
`edge-parity` now reports how many layers each file lays the tower on,
how tall it stands and each layer's height. On `bands` (a post with two
bands of a second filament, `tools/make_fixture.py`) on a U1, a P1S and
a Voron 2.4 of one nozzle: the tower on 4 / 4 layers, up to 0.800 / 0.800
mm, each layer at the same height as the reference's; with the setting
off nothing changes. The U1 refused with the tower at x 222 and accepted
at 226, as the reference. Not as the reference, on purpose (each said
in the code, the reference's being plain mistakes): the brim's chamfer steps a loop for each
layer of the tower laid, not for each of the print's layers; Bambu's
tower keeps its first laid layer solid where the tower is spread for its
height; a row's or a ram's layer below is the tower's layer laid last;
and the spiral the clearance allows for takes `travel_slope` in degrees,
which makes the check 2.3 mm stricter on a U1 (the reference accepts the
tower at x 224 and 225, here refused).

**The way to the prime tower, and the retractions round it** (2026-11-13,
`edge-parity`). The reference's own tower (WipeTower2) lays its frame,
fill, wall and brim with moves of its own: straight, without pulling the
filament back or lifting, at the speed in force, each closed line from
its corner nearest the nozzle, the wall as one with the filament pulled
back only across each gap in it (WipeTower2.cpp `finish_layer`,
`generate_path`); this engine went from line to line as for any travel,
with a retraction and a lift each time, and now goes as the tower does.
The way to the tower goes round the box of its first layer, grown by
2 mm, to the gap in its wall (GCode.cpp `generate_path_to_wipe_tower`,
`travel_to_tower_gap`), for a tool changed in place, a tool going over
the tower to be changed, and onto the tower on a layer with no change;
Bambu's tower, whose nozzle the reference takes in straight, is not
routed. Not as the reference, on purpose: it goes in level with where the
block starts, not through the gap, so it crosses the printed wall beside
the gap (0.8 mm off it on `splitbands`' layer 15), and it takes the
shorter way round by the sum of the squares of the stretches; here it
goes in through the gap, the shorter way by length. A tool changed over
the tower (`tool_change_on_wipe_tower`, the first change on a layer, one
nozzle, a rammed filament) is changed where the reference changes it, in
the corner of the space its block cleans and at the layer's height, the
filament pushed back first (`append_tcr2`): it was changed 0.5 mm away,
at the first row's start, and 0.4 mm up, still lifted. The change's
retraction is one, the wipe within it, as there (it was a travel's and
then the rest), and with one nozzle it carries on to the filament taken
up (Extruder.cpp `m_share_retracted`). The next tool's preheat is placed
as the reference's post-processor places it, the cooldown of a tool
wanted again within `preheat_time` taken out (`; removed M104`), and the
fans after a change are the layer's (they were the layer below's: on a
P1S's second layer a change turned the part fan off for the rest of it).
Fixtures: `splitbands` (`tools/make_fixture.py`, mcbrim_test.3mf's U1
settings, ramming off) with `wipe_tower_no_sparse_layers` (ns) and
`tool_change_on_wipe_tower` (on) each 0 and 1; mcbrim_test.3mf plates 1
to 3; the P1S and A1 `twotone`. Retractions, travel to the tower (from
the part, the change included) and back, in m, and the print's time as
each file gives it, reference / `main` / now:

| project | retractions | to the tower, m | back, m | time | template lines differing, `main` / now |
| --- | --- | --- | --- | --- | --- |
| `splitbands`, ns0 on0 | 239 / 390 / 235 | 9.0 / 7.2 / 7.6 | 7.5 / 7.7 / 7.9 | 17m59 / 15m54 / 15m42 | 803 / 756 |
| `splitbands`, ns0 on1 | 248 / 400 / 244 | 9.0 / 7.2 / 7.6 | 7.5 / 7.7 / 7.9 | 18m00 / 15m54 / 15m43 | 709 / 670 |
| `splitbands`, ns1 on0 | 236 / 218 / 208 | 2.1 / 2.0 / 2.3 | 1.7 / 1.7 / 1.7 | 14m47 / 15m13 / 15m14 | 523 / 513 |
| `splitbands`, ns1 on1 | 245 / 228 / 218 | 2.1 / 2.0 / 2.3 | 1.7 / 1.7 / 1.7 | 14m48 / 15m13 / 15m15 | 439 / 429 |
| mcbrim_test plate 1 | 2188 / 2120 / 2081 | 4.6 / 5.0 / 5.8 | 2.6 / 2.6 / 2.6 | 1h05m01 / 1h06m59 / 1h06m59 | 8768 / 8791 |
| mcbrim_test plate 2 | 848 / 986 / 947 | 7.6 / 6.9 / 8.7 | 5.6 / 4.8 / 4.7 | 42m32 / 44m12 / 44m21 | 3396 / 3271 |
| mcbrim_test plate 3 | 846 / 987 / 948 | 7.8 / 8.6 / 10.3 | 8.3 / 6.4 / 6.2 | 42m37 / 44m19 / 44m27 | 3440 / 3317 |
| `twotone`, P1S | 185 / 206 / 185 | 27.7 / 27.7 / 27.7 | 2.8 / 2.8 / 2.8 | 50m47 / 50m21 / 50m20 | 292 / 143 |
| `twotone`, A1 | 227 / 228 / 227 | 23.1 / 23.1 / 23.1 | 2.8 / 2.9 / 2.9 | 52m02 / 51m42 / 51m42 | 679 / 572 |

By kind of move the retractions on the tower are now the reference's
(`splitbands` ns0: 20 / 175 before / 24 there, the 4 left being gaps
crossed there where the wall is begun elsewhere here; mcbrim plates
56 / 56 and 100 / 100), as are those to it, from it, between islands of
`splitbands` and round a change. What is left: on `splitbands` with no
sparse layers the reference's 27 more are its old descent over the
part, each with a retraction and a push back (aceRage/EdgeSlicer#1020,
not copied); on the mcbrim plates the rest is between the part's own
lines (gap fill, bottom surface, supports: 1949 here, 2056 there on
plate 1; 725 / 626 on plate 2), not near the tower, so plate 1's total
is further from the reference's though every kind of move at the tower
is nearer. The tower's lines are still laid nearest first, not in the
reference's order, and the tower is not turned end for end on alternate
layers as the reference's is (WipeTower2.cpp `m_internal_rotation`), so
on every other layer the nozzle comes in on the other side; with that,
the way round and the travel to the tower differ in length (longer here
on the mcbrim plates). The print times differ outside the tower (walls
and fill at other speeds); the reference's own files timed here are as
before (18m00, 14m48, 1h05m02, 42m32, 42m38, 51m21, 52m31), so the
P1S's 34 s and the A1's 29 s over the reference's own figure (about
1.5 s a change) are still not found.

**Bambu Lab's prime tower against the C++ slicer** (2026-11-12,
`edge-parity`; both files timed here, each the way its printer plans
it). Before is `main` at 76a7430, which laid every tower as WipeTower2's;
after is the Type1 tower (WipeTower.cpp). The A1 fixtures are made with
`tools/make_fixture.py` from the figurine's settings: `twotone`, and a
four-block `fourtone` (filaments 1 to 4, 8, 6, 4 and 10 mm tall), once
with its ribbed wall and once with a plain one; the U1's `twotone` is
set to `wipe_tower_type` `type1`. Tower length and plastic are the
tower's lines; tower time is what its moves take, the way in
included, which the reference counts from the end of the change G-code
and this engine from the tower's first line, so the two differ there by
the move from the printer's purge to the tower. Straight lines the same
is how many of the reference's straight tower lines this file has too,
end for end, at the same feed rate and plastic a millimetre.

| project | tower m, ref / before / after | tower E mm, ref / before / after | tower s, ref / before / after | print, ref / before / after | straight lines the same |
| --- | --- | --- | --- | --- | --- |
| figurine, A1, 380 changes | 296.37 / 313.61 / 296.26 | 11307.2 / 11919.2 / 11307.6 | 5194 / 5721 / 5056 | 14h43m27 / 14h56m10 / 14h39m33 | 19434 of 20138 |
| `twotone`, A1 | 13.97 / 15.46 / 13.96 | 541.1 / 587.5 / 541.1 | 244 / 286 / 236 | 44m03 / 44m27 / 43m17 | 1190 of 1222 |
| `fourtone`, A1 | 54.40 / 58.51 / 54.38 | 2082.6 / 2223.6 / 2082.5 | 1057 / 1116 / 989 | 2h57m28 / 2h57m09 / 2h54m12 | 3257 of 3371 |
| `fourtone`, A1, plain wall | 54.64 / 58.51 / 54.64 | 2085.4 / 2223.6 / 2085.4 | 1198 / 1116 / 1025 | 2h59m49 / 2h57m09 / 2h54m40 | 2714 of 2920 |
| `twotone`, U1 set to Type1 | 15.69 / 19.74 / 15.69 | 599.4 / 750.1 / 599.4 | 322 / 340 / 304 | 12m44 / 13m21 / 12m45 | 3448 of 4024 |

The rows, frames and lines across the open depth are the reference's
line for line on every layer of all five; the tower stands where the
reference's does (the four-filament A1 cube's moved from 15 x 220 to
15 x 217.415, as there) and takes its ground to the hundredth. What
differs: the wall's arcs on the A1 (fitted here from the corner the
nozzle starts at, there from the outline's own first corner, so the
same wall is cut into arcs a little differently; the brim's arcs bulge
a few hundredths further out there), a few thinned wall corners on the
U1 (the outline's first corner is not the same one in the two clipping
libraries), the plain wall laid at its speed, 90 mm/s, where the
reference's goes at the 30 mm/s the change G-code leaves in force (most
of the plain-wall cube's 173 s), and the figurine's first layer, where
the tower was laid at the first layer's acceleration until travel there
took `initial_layer_travel_acceleration` (6000 mm/s² on the A1). Taken
layer by layer, the time on the tower's own lines and the travel round
it agree to the second; the rest of the tower times above is the move in.
What is left of the whole print's time is in the change G-code: the
reference's A1 change pulls the filament back 18 mm before the cut and
pushes it on again (`M620.11 S1 ... E-18`, its filament's
`filament_long_retractions_when_cut`), which this engine's left out
(`M620.11 S0`), about 2 s a change; done since (below). Slicing is no slower: the G-code
stage of the figurine 0.94 to 1.23 s before, 1.10 to 1.19 s after, on a
busy machine, and the lettered plates' within the same spread.

**Bambu Lab's printers: the file and its templates** (2026-11-12,
`edge-parity`'s template check). A printer whose `printer_model` (or,
failing that, preset name) starts "Bambu Lab" gets its file in Bambu
Studio's form, as the reference writes it (GCode.cpp `_do_export`,
GCodeProcessor.cpp `Reserved_Tags`): the tags its firmware and Bambu
Handy read (`; FEATURE:`, `; CHANGE_LAYER`, `; Z_HEIGHT:`,
`; LAYER_HEIGHT:`, `; LINE_WIDTH:`, `; WIPE_START`, `; PAUSE_PRINTING`),
the time in the header (`; model printing time: …; total estimated time:
…`), the settings at the top, the spaghetti detector opened before the
first layer and closed after the last (`M981`), power-loss recovery from
the second layer (`M1003`, and Marlin 2's `M413` elsewhere, by
`enable_power_loss_recovery`), the first layer's scan where asked
(`scan_first_layer`, `M976`), a tool change on the tower framed as the
tower writes it (`; CP TOOLCHANGE START`, the material, `M220 S100`,
`; WIPE_TOWER_START` / `_END`, the filament's end G-code before the
filament is pulled back). Every Marlin or RepRapFirmware printer is told
its limits before the start G-code (`M201`, `M203`, `M204`, `M205`,
`emit_machine_limits_to_gcode`), and every file has the reference's
`;_SET_FAN_SPEED_CHANGING_LAYER` marker after each layer change. Positions
are taken back by `extruder_offset` (0x2 on the P1 and X1). The
timelapse G-code (`time_lapse_gcode`, dropped before) goes where the
reference puts it: as the layer starts, or on a bed-slinger such as the A1
before the first tower work of the layer, else once the first island with
fill has its walls, else at the layer's end, the filament pulled back
first and the nozzle put back at its height after, its moves counted in
the layer's time for cooling. Templates are given what Bambu's read:
`first_layer_center_no_wipe_tower`, `in_head_wrap_detect_zone`,
`min_vitrification_temperature`, `is_extruder_used`,
`travel_point_1_x` to `_3_y` (the way past the objects to the cutter,
`get_path_of_change_filament`), `first_layer_print_min`/`_size` from the
reference's first-layer hull (the skirt's, out to its last loop even with
no loops, the islands, the brim, the tower), `toolchange_count` counting
the first tool taken up, each filament's own retraction laid over the
printer's (`filament_long_retractions_when_cut` and the rest, so the A1
and X1 now pull the filament back before the cut, `M620.11 S1 … E-18`),
a list of choices as its number (`z_hop_types[…] == 0`), and every
number as a C++ stream writes it (six significant digits: `F523.843`).
A Bambu Lab printer of two nozzles (H2D, H2C) is refused: which nozzle
each filament prints with is Bambu Studio's grouping, not done. The time
estimate reads what the reference's does: `G28` as a move home, Bambu's
`G29` as 260 s once, `M191` above 40 °C as 720 s, Bambu's own tool
commands (`T1000`, `T255`) as nothing, and the waits of a Bambu Lab
printer's end G-code as after the print.

Projects made with `tools/make_project.py` from the BBL profiles, the
part centred where the reference's command line moves it and the
flushing volumes set to those it works out; lines of template G-code
that differ, of the layer changes, timelapse, tool changes, start and
end, before / after:

| project | template lines differing, `main` / now | print, reference / `main` / now | reference's file timed here |
| --- | --- | --- | --- |
| cube, P1S | 519 / 7 | 14m40 / 12m25 / 14m50 | 14m47 |
| cube, X1 Carbon | 625 / 7 | 15m46 / 15m26 / 15m56 | 15m53 |
| cube, A1 | refused (`first_layer_center_no_wipe_tower`) / 218 | 19m32 / - / 19m47 | 19m33 |
| `twotone`, P1S | 2198 / 292 | 50m47 / 48m07 / 50m21 | 51m21 |
| `twotone`, X1 Carbon | 2304 / 292 | 51m53 / 51m08 / 51m27 | 52m28 |
| `twotone`, A1 | refused / 679 | 52m02 / - / 51m52 | 52m31 |
| cube and `twotone`, H2D | refused (`min_vitrification_temperature`) / refused (two nozzles) | | |

What is left: on the cubes the order of a feature's tag and its `M204`;
around a tool change, the reference's repeated `G1 Z` lines on the move
to the tower (its writer's, after a change G-code that leaves the head's
place unclear; they move nothing), and its wipe before the change at its
own speed and length (`G1 E-1.853 F12232.572` there, `E-1.88 F15000`
here); the tower's own comments on a layer with no change
(`; CP EMPTY GRID`); on the A1 the fan speeds the cooling
works out, and, above the tower, the timelapse picture the reference
leaves out (below). Not done, and listed by the check as left out: the
objects labelled for skipping (`M624`/`M625`, `; printing object`, and
Klipper's `EXCLUDE_OBJECT_*`, which the U1's file lacks too), progress at
each change of the figure (`M73`, here at each layer). The reference's
own files timed here come to within 7 s on the cubes and 35 s on the
two-filament prints (about 1.5 s a tool change, not yet found).

A C++ error this turned up: above the prime tower's top the reference
marks every layer but the last as having a tower (ToolOrdering.cpp
`collect_extruders`' last pass, the loop that keeps tower layers within
`max_layer_height`, about line 1316), though the Type1 tower lays nothing
there, so the A1's timelapse picture, which is taken before the tower's
work on such a layer, is never taken: on `twotone` (30 layers, the tower
20) layers 21 to 29 have none. Not copied: here every layer has its
picture.

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
followed. A plate's own settings in a project ("Customize current plate":
its bed type, print sequence, vase mode, later layers' order) are laid over
the project's for that plate's slice, as the reference lays a plate's
config over the whole; `print_sequence` "by object" is noted and not
followed: every object is printed together, layer by layer. On the first layer two tools whose smallest pieces are the
same to a millionth count as the same, and the lower number goes first;
the reference compares exactly, so where it has identical parts in
several filaments its order among them can differ (plate 3 of
`mcbrim_test.3mf` starts 1, 5, 4, 3, 2 there and 1, 3, 4, 5, 2 here). The tower has no
cone wall, which is noted when the profile asks for it. The tower is not
turned end for end on alternate layers. A filament with no tool of its own (the fifth on a
four-tool printer) is written as the reference writes it, with a warning.
Filaments mixed from others are not read.

Painted colours are a first working version. A project's `paint_color`
is read, triangles cut up as the paint cuts them (a port of
`TriangleSelector::deserialize`), and each painted object is shared out
between filaments as the reference's `segmentation_by_painting` shares
it. The sides are a
port of the reference's stages, listed at the top of edge-paint's
`side` module: each painted triangle is cut by the layer's plane and
the cut put onto the edges of the outline within 5 micrometres and 30
degrees of it, each ring is cut into lines of one colour with short
runs given to the colours about them (`colorize_contours`,
`filter_colorized_polygon`), and the layer is divided along the edges
of the Voronoi diagram of those lines that part two colours, traced
through the reference's graph (`MMU_Graph`, `extract_colored_segments`).
What is painted on a top or bottom face is carried through the shell
layers behind it, in by a wall and a space, where it wins over what the
sides say; with classic walls, islands of paint on a face too narrow for
a wall's first loop are dropped. How deep a colour reaches follows
`paint_depth_mode` (`edge_paint::depth`): unlimited, the default, takes
all a painted surface is nearest; walls (`paint_depth_walls`) or
millimetres (`paint_depth_mm`) cut each colour back to a band along the
outline, a thin part keeping half of itself, with the band notched on
even layers (`mmu_segmented_region_interlocking_depth`), and carry paint
on a top or bottom face as deep as the band, a flat face no deeper than
its shell. Islands of the part's own filament left between two painted
claims too narrow to print go to the colour beside them. Where one
colour meets another above or below, each gets a solid skin of its own
(`paint_depth_solid_interfaces`, as `interface_shells`, which is
followed too); the reference does that for the painted object, here it
is done for the whole plate. The band is worked out from the plate's
walls, where the reference takes the widest over an object's regions.
An old project's `mmu_segmented_region_max_width` is read as a depth in
millimetres. Two parts of an object rarely meet exactly: their meshes
follow the same curve with different corners, and cracks a few
micrometres wide are left between them. This engine closes those
cracks when it slices. The reference does not, so where paint makes
both sides of a crack one filament it
lays walls round the crack, a slit of two loops in the middle of a
solid area. That is not copied. Beam interlocking (`interlocking_beam`),
`paint_infill_override` off and `top_color_penetration_layers` are noted
and not followed. Seams
painted on or off are followed (G-code, Seams, below), and so are
supports painted on or off (Regions, below). Painted fuzzy skin is not
read; a project that has it painted on says so when it is sliced. Paint
is read through one reader of the codes (`edge_mesh::selector::code`),
and paint made with the desktop app's painting tools goes into the model
(`Model::set_paint`) and is saved as the reference saves it.

Not done in G-code: the older tree styles (`tree_slim`,
`tree_strong`, `tree_hybrid`: organic-style trees are built, with a note),
rafts, brims inside holes, draft shields, seams put by Precise Seam
parts (read as helpers, not printed, with a note), a fan speed of its own for bridges over infill, walls
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
not ported. Overhangs are measured as the reference measures them, at the
same places (see "Walls over overhangs" below), but only walls are: the
reference measures the lines of a bridge too, and slows a wall over lines
of the layer beneath that it expects to curl
(`slowdown_for_curled_perimeters`), which is not estimated here.
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
reaches 0.8 mm, a bridge over infill is turned the other way from the
reference's on about one layer in five where the vote between the two
directions of a grid is close (below, under Bridges over infill),
narrow parts of top and
bottom surfaces get no gap fill
(`gap_fill_target`), and eight of the
reference's sparse patterns are not laid (below). Gap closing runs across objects, so two separate objects closer than twice
the radius merge; the reference closes within one object only.

Clipping is clipper2-rs, a port of Clipper2. The reference uses Clipper 6,
so regions agree in shape and area but not vertex for vertex.

Supports are a first working version, not a port: the grid of normal
supports is always square to the bed, a support's lines are not sheltered
from retraction, bridges are supported like any overhang, and where a brim
and a support meet on the first layer neither gives way. Tree supports
are a port of the reference's organic trees. What still differs: the
support under bridges shorter than `max_bridge_length` is kept (the
reference leaves it out, which needs the walls and the fill's bridges;
a note says so); the older tree styles are not ported (organic trees are
built, with a note); a tip wider than the smallest roof does not turn
the branch's top layers back into roof (EdgeSlicer's
`recover_pending_branch_roofs`); `smooth_outward` is a closing; the
areas are merged in groups of neighbours ordered as the reference's
bounding box tree orders them, not by that tree itself; a branch carried
down onto the part is cut on each layer by that layer's collision and
its foot printed, where the reference cuts the first layer down by the
collision of the layer above and leaves the foot unprinted: the same
with dense bottom layers, while without them the reference's feet sit on
the part (the bottom gap is held here in both, 0.2 mm on the 3DBenchy
and on a slope); and one of the reference's mistakes is not copied, in
its overhang clusters (a second overhang on a layer joining a cluster is
never supported). The smoothing pushes a ball clear of the part by the
square of the distance, as the reference does (it reads like a slip,
but moved the distance itself the balls swing back and forth instead of
settling, and the support comes no further from the part). On `ledge` the reference holds up a 0.37 mm overhang round
the frame that its own small-overhang rule drops on a plain block
(`lip`, nothing in either); why was not found, and here it is dropped.

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

Then the printer's templates, line for line, layer by layer
(`templates.rs`): the start and end G-code, the layer change, the
timelapse and a tool change on the tower up to its rows, the moves the
templates make kept and the toolpaths' left out, with the differing
stretches and how often each kind of line differs. Left out as different
by design or not done: progress (`M73`), the objects' labels for
skipping, the width and height tags, and a feed rate already in force
inside custom G-code, which the reference's writer drops.

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

Time from loading to the G-code written, best of 5 on all 18 cores,
2026-11-11, against the reference's own work as its log has it: the
3DBenchy (`benchy_u1.3mf`) 0.63 s against 2.77 s; `mcbrim_test.3mf`
plate 1 (16 layers, eight painted objects of 85,000 triangles, seven
filaments, supports, a prime tower) 1.65 s against 2.66 s, plates 2
and 3 0.57 s and 0.55 s against 1.31 s each. Plate 1 took 2.8 s before
a pass for speed, which measures in its stages (`edge slice --timings`)
the first island's offset of the layer above kept for the rest, travel
inside the part asked of the cells it is sorted into, every filament's
layers and every object's visibility worked out side by side, the
triangles' corners kept in the leaves of the ray tree, and the seams'
visibility measure made beside the slicing; none of it moves the
G-code. On one thread plate 1 takes 15.2 s (16.7 s before): painting
6 s, the visibility measure 3.4 s, finding where parts meet 1.2 s.

By feature, in metres of extruded path, reference against this engine:

| Project | Overhang wall | Internal bridge | Internal solid fill | Gap fill |
| --- | --- | --- | --- | --- |
| `cylinder_drawcut.3mf` | none | 11.22 / 11.22 | 61.04 / 61.04 | 0.02 / 0.02 |
| `twopart_groups.3mf` | 0.16 / 0.16 | 1.16 / 1.16 | 6.38 / 6.38 | none |
| `mushroom` fixture | 0.23 / 0.23 | 1.99 / 1.99 | 10.93 / 10.93 | none |
| `mcbrim_test.3mf` plate 2 | | 2.60 / 2.66 | 24.29 / 24.58 | |
| `mcbrim_test.3mf` plate 1 | 0.10 / 0.06 | | | 8.28 / 9.98 |
| `mcbrim_test.3mf` plate 2 | 0.03 / 0.03 | | | 4.95 / 5.03 |

Supports and the automatic brim are compared on parts written by
`tools/make_fixture.py`, with normal supports turned on:

| Part | Support body | Dense layers | Brim |
| --- | --- | --- | --- |
| `mushroom` (a cap on a stem), normal | -1.6%, same extents | -3.6% | none in either |
| `shelf` (a shelf over a plate), normal | -1.5%, same extents | -5.0% | none in either |
| `tower` (6 x 6 x 80 mm) | | | same length, same reach (5.63 mm) |
| `bar` (200 x 10 x 30 mm, PLA) | | | same length, same reach (7.42 mm) |

Normal support lines come as close to the part as the reference's
(0.54 mm). They put support on the same layers as the reference,
with the same layers of air above and below. Tree supports are compared
below.

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
tool on their top layer). The plate takes 1.4 s to slice on 18 threads (0.7 s of it
the painting: eight objects of 85,000 triangles each), and 1.65 s with
its G-code.

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
simplified. A diagram that comes out broken, or does not come out, is
built again for the outline turned by a few odd angles, as the
reference does. The reference's outlines are in their object's own
coordinates, measured from the middle of the object's box, and its
diagrams are made there; this engine's outlines are in the plate's, so
the diagram is made of the outline measured from the middle of its own
box, its vertices rounded to the grid there and then moved back, and
an outline is turned about that middle. Where on the plate an object
stands then makes no difference to a diagram. It did: the Voron design
cube's notched layers, 49 to 79 mm from the origin with two edges a
nanometre out of line, gave a broken first diagram (in Boost.Polygon's
library as well, fed the same numbers) and were built turned. Over the
reference projects and the parts compared here that was 27 outlines in
36 325, all but two of them that cube's; about their own middle all
36 325 come out sound the first time. On the cube the walls moved
nearer the C++ slicer's (with the U1's presets, over the layers that
changed, the walls' lengths are 0.50 mm from the reference's in all,
where they were 0.83 mm); elsewhere a wall here and there moved by a
few hundredths of a millimetre, as near the reference's after as
before. The rest of the skeleton is still worked out in the plate's
coordinates, and a node of it can lie a nanometre or two from where
the reference's arithmetic would put it. With the
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

One thing does not match. At a margin of nothing
(`hole_to_polyhole_threshold = 0`, which both slicers take as a
ten-thousandth of a millimetre) this engine converts the tube's hole
below the tube and the reference does not: whether its points keep to
one distance within 0.0002 mm is decided by rounding that the two do not
share.

With both options off the same 300 walls were up to 0.17 mm from the
reference's: the kinks closing left in an outline cut with the
crossings in it, which every layer is now cut without (Status, above).
Measured again on 2026-11-11, each corner of a loop here against the
nearest corner of the reference's loop: above the first layer the
furthest is 0.0022 mm (the eight-sided hole; 290 of the 292 loops are
within 0.0015 mm), where it was 1.2 mm on that hole, which had a ninth
point part-way along one side, and over 0.01 mm on 274 of them. On the
first layer the furthest is 0.03 mm, on the eight-sided hole again, as
it was: the first layer alone is also shrunk for the elephant's foot,
and the cut does not move that. The walls'
length over the whole part is the reference's to the rounding of the
table (16.36 m of outer wall and 16.22 m of inner in both; the inner
was 16.23 m here), and every layer's is within 0.05% of the
reference's, where the first was 0.6% over.

With both options off the G-code of the parts made of boxes (the
`shelf`, `mushroom`, `tower` and `bridge` fixtures, with the U1's
presets and the Centauri Carbon's) is byte for byte what it was before
every layer was cut this way; the round parts' and 3DBenchy's is not.
With both on, the parts made of boxes are line for line what they are
with both off.

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

Z contouring, 2026-11-11, on parts from `tools/make_fixture.py` with the
U1's presets (a 0.2 mm layer, `zaa_min_z` 0.05 mm) but where the
Centauri Carbon's are named (arcs on), and on 3DBenchy with the U1's.
`wedge` rises 4 degrees, `dome` is a cap 3 mm high on a post, `ramp`
climbs 7.6 degrees to a flat and `holed` is the wedge with a hole down
through it. Lengths and filament are the C++ slicer's and this engine's;
a contoured move is an extruding move that says a height off its layer's
top. The heights compared are paired: each of the reference's moves of a
feature against the nearest of this engine's on the same layer, within
0.05 mm across, at the middle of the reference's move, the 95th centile
and the most; the plastic a millimetre likewise.

| Part, options | Top surface, m | Outer wall, m | Contoured moves, top and walls | Heights apart, 95% and most | Plastic a mm apart, 95% | Filament, mm | Time |
|---|---|---|---|---|---|---|---|
| wedge, off | 1.57 / 1.84 | 2.15 / 2.15 | | | | 611.5 / 610.9 | 5m 10s / 5m 41s |
| wedge | 1.57 / 1.87 | 2.17 / 2.17 | 3783 / 3783, 150 / 150 | 0, 0.0005 mm | 0.2% | 598.3 / 595.5 | 5m 6s / 5m 42s |
| and `zaa_min_z` 0.08 | 1.57 / 1.87 | 2.16 / 2.16 | 3795 / 3795, 150 / 150 | 0, 0.0005 mm | 0.1% | 609.4 / 604.1 | 5m 10s / 5m 41s |
| and fill in one direction | 1.57 / 1.87 | 2.17 / 2.17 | 3783 / 3783, 150 / 150 | 0, 0.0005 mm | 0.2% | 598.5 / 595.1 | 5m 7s / 5m 42s |
| and ironing | 1.57 / 1.87 | 2.17 / 2.17 | 3783 / 3783, 150 / 150; ironing 7869 / 7869 | 0, 0.0005 mm; ironing 0, 0 | 0.2% | 602.7 / 599.9 | 6m 59s / 7m 34s |
| dome, off | 0.40 / 0.43 | 1.46 / 1.46 | | | | 390.7 / 392.4 | 4m 9s / 4m 24s |
| dome | 0.43 / 0.47 | 1.48 / 1.48 | 1314 / 1315, 2638 / 2615 | 0.0005, 0.014 mm | 1.3% | 381.0 / 382.4 | 4m 21s / 4m 27s |
| and `zaa_min_z` 0.02 | 0.46 / 0.47 | 1.49 / 1.49 | 660 / 661, 2945 / 2936 | 0.0005, 0.016 mm | 1.0% | 377.7 / 379.8 | 4m 26s / 4m 29s |
| and walls let down past 10 degrees | 0.43 / 0.47 | 1.48 / 1.48 | 1314 / 1315, 2902 / 2871 | 0.0005, 0.019 mm | 1.3% | 375.3 / 376.7 | 4m 28s / 4m 27s |
| and no speed scaling | 0.43 / 0.47 | 1.48 / 1.48 | 1314 / 1315, 2638 / 2615 | 0.0005, 0.014 mm | 1.3% | 381.0 / 382.4 | 4m 13s / 4m 27s |
| and ironing | 0.43 / 0.47 | 1.48 / 1.48 | 1314 / 1315, 2638 / 2615; ironing 15986 / 14093 | 0.0005, 0.014 mm; ironing 0.0055, 0.023 | 1.3% | 383.4 / 384.7 | 5m 12s / 5m 23s |
| and `arachne` | 0.40 / 0.48 | 1.48 / 1.48 | 1581 / 1581, 2240 / 2231 | 0.0008, 0.008 mm | 3.4% | 381.8 / 382.9 | 4m 20s / 4m 28s |
| and the Centauri Carbon's, arcs | 0.85 / 1.57 | 1.50 / 1.50 | 5026 / 5022, 3865 / 3841 | | | 386.7 / 385.9 | 4m 42s / 5m 34s |
| ramp, off | 1.81 / 2.11 | 2.01 / 2.01 | | | | 653.8 / 653.3 | 4m 41s / 5m 9s |
| ramp | 1.84 / 2.11 | 2.01 / 2.01 | 4147 / 4150, 267 / 267 | 0, 0.0005 mm | 0.0% | 641.2 / 640.8 | 4m 43s / 5m 10s |
| and `arachne` | 1.84 / 2.11 | 2.01 / 2.01 | 4147 / 4150, 267 / 219 | 0.0083, 0.0085 mm | 9.9% | 641.5 / 641.0 | 4m 43s / 5m 11s |
| holed, off | 2.18 / 2.57 | 3.16 / 3.16 | | | | 882.1 / 881.6 | 6m 31s / 7m 19s |
| holed | 2.19 / 2.61 | 3.18 / 3.18 | 5263 / 5263, 272 / 272 | 0, 0.0005 mm | 0.3% | 863.6 / 864.4 | 6m 30s / 7m 24s |
| wedge, scarf seams on all | 1.57 / 1.87 | 2.64 / 2.43 | 3783 / 3783, 488 / 345 | | | 603.6 / 595.5 | 5m 8s / 5m 43s |
| 3DBenchy, off | 3.45 / 3.87 | 32.78 / 32.78 | | | | 3767.6 / 3774.1 | 38m 43s / 41m 48s |
| 3DBenchy | 3.54 / 3.83 | 32.59 / 32.73 | 6649 / 5339, 6992 / 7056 | 0.0023, 0.054 mm | 2.4% | 3737.4 / 3754.1 | 39m 4s / 42m 7s |

Where the top surface is the same lines in both files (the wedge, the
ramp, the holed wedge) every contoured move is the reference's to the
thousandth of a millimetre written, with the same plastic: the band, the
smoothing, the 10 µm the line must move and the points dropped come to
the same points. Where they differ it is the lines that differ, not
their heights: with contouring off the two already lay the top surfaces
of these parts differently (the reference's 1.57 m of the wedge's top
against 1.84 m here, and with the Centauri Carbon's presets on the dome
0.83 m against 1.52, too few lines alike to pair; since measured again
under Top surfaces on slopes, where they are the same), and on 3DBenchy the
reference contours 6649 top surface moves and this engine 5339 of a top
laid otherwise, the pairs within 0.0023 mm in 95 of 100. A wall of
changing width (`arachne`) is contoured here as a whole loop and in the
reference piece by piece, each piece of one width smoothed by itself, so
near where its pieces meet the two differ by up to 0.009 mm. With
`zaa_minimize_perimeter_height` at its 35 degrees the dome's walls, on a
slope of 28 degrees at most, are not let down (the same file as at 0);
at 10 degrees they are, in both, and take 6 mm less filament. With
`zaa_dont_alternate_fill_direction` the wedge's top lines run at 45
degrees on every layer in both files, and its solid fill never at 135.
No arc is fitted to a contoured move in either file.

Every extruding move in all of these files is between the slicing plane,
`zaa_min_z` above the layer beneath, and `zaa_min_z` above its own
layer's top, ironing a layer above it, in both slicers. On the wedge 27
of the reference's 109 wipes end lower than the line they wipe back over
was laid there, by up to 0.04 mm, and on the dome 32 of 286 by up to
0.10 mm; here none does. With scarf seams on all walls the reference
lays 71 mm of the wedge's contoured outer walls twice, the scarf's way
up and its way down both flat along the contour, over seven layers;
here those walls are closed with the seam gap and have no scarf (the
note says so). On the dome with ironing the reference irons 83 mm at
10 mm/s and more at other speeds between that and its own, where with
contouring off it irons all of it at the ironing speed; here all of it
is at the ironing speed. With speed scaling, a contoured move is slowed
in step with its height in both: with layers not slowed to cool
(`slow_down_for_layer_cooling` off) the speeds paired on the wedge and
the dome are within 4.4% in 95 of 100, as the speeds of the walls are
with contouring off. Where layers are slowed to cool they part: the
reference slows contoured moves by the layer's factor whatever their
speed, and this engine holds them to the layer's cap like any other
line (a note says so); on 3DBenchy's layers from 11 mm up the
reference's contoured top moves average about 4000 mm/min and this
engine's about 7200.

The reference's own file, timed here as the reference plans it, comes
to its own time to the second, or a second over, on every file (a
contoured move is read as the move it is, its length in three
dimensions). With `zaa_enabled` off the G-code is byte for byte what it
was before any of this on these parts, the other fixtures, 3DBenchy and
five of the reference's test projects (19 files), and with it on a file is the
same on one thread as on all of them and from one run to the next.
What it costs (`edge slice`, best of 7, off and on in turn, on a Mac
mini busy with other work, so the processor time is the steadier
figure): 3DBenchy 1.14 s and 1.24 s, 4.9 and 5.4 s of processor time;
the dome 0.17 s both, 0.49 and 0.51 s; the holed wedge 0.12 and 0.13 s,
0.26 and 0.28 s. Most of it is building the tree of the parts'
triangles (225,154 on 3DBenchy), which is now made on as many threads
as there are; the rays, cast no further than a surface could move a
point, take some 35 ms there.

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

| | loops | before | overhangs | comparators | visibility | lined up |
| --- | --- | --- | --- | --- | --- | --- |
| `aligned` | 1021 | 2.6%, 12.3% | 3.2%, 12.9% | 14.8%, 25.7% | 34.5%, 39.5% | 40.0%, 56.4% |
| `aligned_back` | 1011 | 13.2%, 31.1% | 13.5%, 31.4% | 18.0%, 41.5% | 27.3%, 36.5% | 46.3%, 69.9% |
| `back` | 1011 | 69.8%, 91.6% | 70.1%, 91.9% | 71.2%, 87.4% | 71.2%, 87.4% | 72.5%, 94.0% |
| `nearest` | 1019 | 14.6%, 22.7% | 15.0%, 23.1% | 16.6%, 24.7% | 25.1%, 32.2% | 25.1%, 32.2% |
| `random` | 1016 | 0.0%, 7.4% | 0.1%, 7.4% | 23.2%, 30.0% | 23.2%, 30.0% | 23.2%, 30.0% |
| Benchy | 745 | 35.1%, 41.7% | 43.1%, 52.8% | 24.6%, 31.7% | 64.0%, 80.9% | 49.9%, 61.3% |
| every loop, with the three plates | 6235 | 20.6%, 31.9% | 21.9%, 33.5% | 26.6%, 38.3% | 37.4%, 46.7% | 39.8%, 53.6% |

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
straight.

Visibility (how much of the part's outside can be seen from each place,
by rays cast from 30000 points on its mesh): the Benchy goes from 24.6%
within 0.05 mm to 64.0% and from 31.7% within 1 mm to 80.9%, its mean
distance from the reference's seams from 5.3 mm to 1.4; the mushroom
and the fins at `aligned_back` from 27.5% and 62.5% to all of their
seams, the mushroom at `aligned` from none to 72.5%, the `holes` part
at `aligned` from none to 27.1% and at `nearest` from 18.5% to 32.4%.
The `back` and `random` do not go by what is seen and are as they
were. It costs time: 30000 points and 750000 rays an object, 0.17 s
more on the Benchy (0.48 s to 0.65 s for the whole run, best of 7),
0.54 s on plate 1 of `mcbrim_test.3mf` with its many objects (2.30 s to
2.85 s), 0.02 s on the `holes` part.

Lined up (the reference's lines of seams, each with its fitted curve):
the cube at `aligned_back` goes from the other back corner on every
layer to the reference's on all 100, as it does at `aligned_front`,
`left` and `right`; the mushroom at `aligned` from 72.5% to every seam;
the comb at `back` from 1 seam of 15 to all; the round parts come
within 1 mm where they were up to 10 mm off (tube at `aligned` and
`aligned_back`, cylinder at `aligned_back`, 100% each from none; the
funnel at `back` and `aligned_back` 70% and 74% from none), and within
0.05 mm on about half the tube's loops. They could not do better
then: this engine's round walls had twice the reference's corners, so
the points a seam may sit on were not the same points, and the
cylinder at `aligned`, whose every point is as good as another, had its
seam 10 mm round from the reference's (with main merged in they do;
see below). The same holds for the frame, the steps
and the `holes` slab, where the reference's seam is on a straight side
at a point of its outline that this engine's outline does not have
(where two of the boxes the part is built of meet; a seam can only
sit on a point of its wall or be drawn between such points, so the
reference's outline has one there). The two blocks of `twotone` are
two parts of one object, and there the reference puts both seams in
the middle of the side where the parts meet, which is
`seam_prefer_part_joints`, on unless turned off and not done then (it
is now: see below); the
plates of `mcbrim_test.3mf`, objects of several parts, are the same
case, and it is why they hardly agree (4.7%, 1.5% and 1.5% of their
loops within 0.05 mm).

The Benchy loses 14 points on the way, and that is a toss-up, not a
miss. It is its own mirror image, and of its 745 loops 457 have their
seam within 1 mm of the reference's, 196 within 1 mm of the mirror
image of the reference's, and 92 elsewhere (603, 53 and 89 before the
seams were lined up). Which of two mirrored places is taken is settled
by which has the better score, and the two differ only by the scatter
of the sample points; the Benchy's mesh has more than 16000 triangles,
so the points are not the reference's own (Status, above), and a whole
line of seams goes the way its best seam does. On a part of fewer
triangles the points are the reference's: the cube's four leaning
positions each pick the reference's corner of two.

The reference's skipped layer (Status, above) is in its files, and the
figures show it: with the slip copied, for the measure only, `aligned`
comes to 49.2% and 60.8% (40.0% and 56.4% without), the steps at
`aligned` to 75.4% from 53.8%, the `holes` part at `aligned` to 36.3%
from 16.4%, and the Benchy to 51.9% and 63.8%. It is not copied.

At `aligned_front`, `left` and `right`, on the cube, the `holes` part,
the mushroom and the tube (651 loops each): 31.0% and 45.6%, 38.9% and
61.3%, 45.9% and 68.0%. Travel and retractions hardly move with any of
this: on the Benchy 26.96 m and 1520 in the reference's file, 26.49 m
and 1707 here; on the `holes` part at `aligned` 17.35 m and 747, 16.84 m
and 738. With scarf seams on, the `holes` part's wall length per layer
is off by 0.03% on average, the most 0.6% on the first layer, and both
loops of the layer at 10.45 mm still start where the reference's do.
Lining seams up costs no time that shows: the Benchy's run is 0.65 s
before it and after.

With main merged in (layers cut without flat crossings, so a ring is
its mesh's corners; walls thinned to `resolution` before they are
written, and the seams now planned on the thinned loops, so a seam's
candidates are the points that are written), 2026-11-11, on the same
parts and the bent `curve`, which the script now pairs (the reference
stands it 0.73 mm further back than the file has it, being centred by
its bounds; the script moves this engine's walls onto the reference's
where the two have the same size). Main as it was, against main with
the seams:

| | loops | main | with the seams |
| --- | --- | --- | --- |
| `aligned` | 1051 | 6.3%, 9.2% | 42.9%, 58.3% |
| `aligned_back` | 1051 | 12.3%, 28.9% | 52.7%, 69.9% |
| `back` | 1051 | 76.6%, 92.2% | 88.2%, 94.3% |
| `nearest` | 1051 | 22.7%, 30.0% | 42.8%, 46.7% |
| `random` | 1051 | 0.1%, 7.1% | 25.7%, 31.3% |
| `aligned_front`, `left`, `right` | 1953 | 25.0%, 27.1% | 46.1%, 64.3% |
| Benchy | 744 | 33.9%, 40.7% | 49.5%, 62.2% |
| the three plates | 412 | 1.9%, 1.9% | 2.4%, 5.1% |
| every loop | 8364 | 23.8%, 31.1% | 47.0%, 58.6% |

(On the old base with the same script the seams came to 39.4% and
54.9% of every loop.) The round parts gain most: the tube is within
0.05 mm on all 80 loops at every position but `nearest` and `random`,
where it was within 1 mm and on about half; the cylinder at `back` and
`aligned_back` on all 50, where it was within 0.5 mm. The cylinder at
`aligned` is 4.8 mm off on every layer, and that is the mirror image
of the reference's seam across the part's middle (10 mm off before):
every point of a round wall scores about the same, and which of the two
mirrored places wins is settled by the scatter of what can be seen.
The funnel is where it was, 76% and 72% within 1 mm at `aligned_back`
and `back`: both slicers line its seams up the back, but the
reference's line leans 2 mm across the part's 50 layers and this
engine's stands upright, the two crossing at about the 43rd layer
(within 0.05 mm there); at `aligned` its seams are elsewhere
altogether, 10 mm off on average. The `curve` is within 1 mm on every
loop at `aligned`, `aligned_back` and `back` (0.5 mm, 0.16 mm and
none), as on main. Travel and retractions on the Benchy: 26.96 m and
1520 in the reference's file, 29.00 m and 1708 on main, 27.39 m and
1727 with the seams; on the `holes` part at `aligned` 17.35 m and 747,
17.08 m and 752, 17.41 m and 754. With scarf seams on the `holes` part
(`seam_slope_type=all`, `seam_slope_inner_walls=1`) wall length per
layer is off by less than 0.005% on average and by less than 0.05% on
any layer, where main is off by 0.21% on average and 16.8% on the layer at
10.45 mm. Time, best of 5: the Benchy 0.49 s on main and 0.67 s with
the seams, plate 1 of `mcbrim_test.3mf` 2.54 s and 3.05 s, nearly all
of it the visibility measure.

Seams where parts meet, 2026-11-11, on the same parts with two more of
two parts each (`halves`, a round post cut in two, and `inset`, a block
with a post pushed half into its side) at every position: the parts of
`tools/make_fixture.py` but the round ones are boxes put together, and
so are objects of parts too. Main against main with the joints:

| | loops | main | with the joints |
| --- | --- | --- | --- |
| the three plates of `mcbrim_test.3mf` | 412 | 2.4%, 5.1% | 9.7%, 31.8% |
| `twotone`, every position | 250 | 43.2%, 44.4% | 76.8%, 78.0% |
| `halves` and `inset`, every position | 550 | 39.5%, 40.2% | 64.9%, 65.6% |
| parts of boxes and parts, the aligned positions | 2610 | 25.1%, 47.2% | 45.2%, 66.9% |
| every loop | 8914 | 46.6%, 57.5% | 52.8%, 64.5% |

At `aligned` and `aligned_back` the two blocks of `twotone` go from 16%
of their seams within 0.05 mm to all of them, the seams now in the
middle of the side where the blocks meet; `halves` from none to all,
`inset` from 67% to all, the comb from 60% and 33% to all, the frame
from 48% to 98% (all within 1 mm), the steps from 54% and 52% to 77%
and 99%, and the `holes` slab at its five aligned positions by 8 to 12
points each (25.1% from 16.4% at `aligned`). What is left on the steps
is a square hole whose four corners are as good as each other, and on
the `holes` slab what was left before. At `back`, `nearest` and
`random`, which the reference does not move to joints, and on every
part of one part (the Benchy, the cube, the round parts, the fins, the
mushroom) the file is byte for byte what it was on main.

The plates of `mcbrim_test.3mf` (objects of three parts, an arm and a
blade and an inlay, in three filaments) go from 4.1%, 1.5% and 1.5% of
their loops within 0.05 mm to 14.2%, 7.6% and 6.8%, and within 1 mm
from 8.8%, 3.0% and 3.0% to 45.3%, 24.2% and 24.2%. The rest is the
reference's cracks between parts (Status, above). On these plates the
blade's wall runs along the arm for most of its length, a joint run of
some 450 points whose middle is taken among the points hidden inside
the print; this engine closes the cracks, so all of them are hidden and
the middle is the run's, the same on every layer, where in the
reference's layers a crack here and there leaves stretches of the run
not hidden, and its middle is that of what is left, which wanders from
layer to layer (on plate 2, layer 1, 16 mm along the blade from this
engine's; on layer 3 at the blade's tip). The reference's 535 loops
against this engine's 262 on plate 1 are the walls it lays round those
cracks. Travel and retractions: plate 1 61.31 m and 2185 in the
reference's file, 56.78 m and 1972 on main, 58.95 m and 1978 with the
joints; plate 2 35.31 m and 843, 31.50 m and 945, 35.57 m and 991;
plate 3 37.89 m and 841, 34.64 m and 946, 38.76 m and 991; `twotone` at
`aligned` 8.54 m and 164, 7.58 m and 153, 7.59 m and 154; the `holes`
slab at `aligned` 17.35 m and 747, 17.54 m and 763, 17.54 m and 727.
Time, best of 5: the Benchy as it was (0.63 s to 0.67 s on either), plate 1 of
`mcbrim_test.3mf` 2.93 s and 3.07 s on main, 2.96 s and 2.97 s with the
joints (the 0.13 s search for its 24 parts' joints runs beside the
0.6 s visibility measure, and 0.05 s more goes on the walls' extra
places), the `holes` slab 0.12 s and 0.14 s (32 parts, 0.04 s of
joints, more than the visibility measure there).

Seams painted on or off, 2026-11-11: no project the reference carries
or tests with has seams painted, so `tools/make_fixture.py` paints two
(`enforced`: the round post with a strip of its side 20 degrees wide
painted to have the seam, toward the front left; `blocked`: the cube
with its right and back sides painted to keep it off), and the reference
follows the paint in both (its seams in the strip, and at the cube's
front left corner). At each of the five positions, main against the
joints alone against the paint:

| | loops | main | joints | paint |
| --- | --- | --- | --- | --- |
| `enforced`, `blocked` at `aligned`, `aligned_back`, `back` | 450 | none | none | all, to the micron |
| `enforced` at `nearest` | 50 | 0%, 2% | 0%, 2% | 14%, 14% (all within 2.9 mm) |
| `enforced` at `random` | 50 | 0%, 2% | 0%, 2% | 0%, 56% (all within 3.7 mm) |
| `blocked` at `nearest` | 100 | 1%, 1% | 1%, 1% | all |
| `blocked` at `random` | 100 | 0%, 2% | 0%, 2% | 48%, 54% |
| every loop, with these | 9664 | 43.0%, 53.1% | 48.7%, 59.5% | 55.0%, 66.1% |

At `nearest` on the post this engine's seams are at the strip's other
edge, 2.9 mm round, where the nozzle comes from elsewhere (its travel
1.39 m in the reference's file and 1.99 m here, 89 retractions and
112); at `random` they scatter within the strip as the reference's do,
but by a figure that differs in the last digit (Comparators, above).
Nothing else moves: on every other part the file is byte for byte what
it was with the joints. Travel and retractions at `aligned`: the post
1.46 m and 109 in the reference's file, 1.80 m and 113 on main, 2.08 m
and 111 with the paint; the cube 3.19 m and 201, 3.93 m and 200, 5.13 m
and 201; the travel here is not looked into. Time: none that shows (the Benchy 0.62 s and
0.65 s with the joints, 0.64 s and 0.66 s with the paint; plate 1 of
`mcbrim_test.3mf` 2.91 s and 3.01 s, 3.04 s and 3.09 s).

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

Tree supports, 2026-11-11, after the port of the organic trees, against
the C++ slicer (U1 presets unless said; support and dense layers in m,
from the G-code; "overlap" is the intersection over union of the support
area, every layer rasterised at 0.1 mm, the part placed alike):

| Part | Reference | Before the port | Now | Dense, ref / now | Overlap |
| --- | --- | --- | --- | --- | --- |
| owner's figurine (1.4 M triangles, A1) | 111.09 | 163.4 | 123.74 | 2.18 / 1.38 | 0.42 (0.33 before) |
| `mushroom` | 22.85 | 21.67 | 23.89 | 1.91 / 1.79 | 0.38 |
| `mushroom`, Centauri Carbon | 14.18 | 10.90 | 15.16 | 1.83 / 1.72 | 0.32 |
| `shelf` | 4.00 | 4.16 | 4.57 | 0.62 / 0.57 | 0.25 |
| `shelf`, Centauri Carbon | 3.83 | 2.68 | 3.85 | 0.54 / 0.52 | 0.26 |
| `shelf`, build plate only | 6.74 | 0.00 | 6.95 | 0.30 / 0.28 | 0.37 |
| `shelf`, no interface layers | 3.93 | 9.31 | 3.92 | 0 / 0 | 0.23 |
| `shelf`, gaps of 0.4 mm | 3.88 | 4.03 | 4.28 | 0.62 / 0.59 | 0.41 |
| `bridge` | 7.97 | 7.17 | 8.97 | 0.69 / 0.69 | 0.50 |
| `funnel`, 45 degrees | 11.86 | 15.25 | 11.03 | 0 / 0 | 0.42 |
| 3DBenchy | 40.57 | 81.07 | 39.92 | 0.65 / 0.56 | 0.28 |
| `mcbrim_test.3mf` plate 1 | 1.34 | 2.10 | 0.95 | 0 / 0.13 | 0.58 |
| `ledge` | 6.83 | 1.16 | 1.75 | 0.19 / 0.15 | 0.19 |

On the figurine the number of separate support outlines is the
reference's within one or two on every layer looked at (9 and 9 on the
first, 16 and 17 a hundred layers up, 19 and 21 at two hundred), and
per layer the support is mostly 5 to 20% over it, more near the top
(175 against 122 mm 360 layers up). No support line lies
over any of the part's lines in any of these; lines come within 0.25 mm
of an outer wall less often than the reference's (3260 pixels against
5143 on the figurine, 2026-11-12). Support with nothing under it (no support or part
within 0.3 mm on the layer below) is 62.8 mm2 over the figurine's 381
layers against the reference's 41.9 (668.5 before the port). The foot of
a branch on the part is dense on its own layer and the one above; the
reference prints the same two layers, its foot a layer lower and left
unprinted. Feet on the part are 0.2 mm over it (the bottom gap set) on
the 3DBenchy, `shelf` (U1 and Centauri Carbon) and `slopeshelf` (U1)
with two dense bottom layers and with none; the reference's are too with
two, and with none sit on the part on the 3DBenchy and `shelf`
(2026-11-12). With `tree_support_branch_distance_organic` 3 or 5 the
`shelf` is unchanged in both, as its tips are set along its roof's lines.

The support stage on the figurine takes 784 ms against 1084 ms before
the port (best of five, the two builds taking turns on a busy machine),
and 93 against 109 ms on `mcbrim_test.3mf` plate 1 with supports on.
`EDGE_TREE_TIMES` set prints each step's time.

Arcs, 2026-11-11, on three round parts from `tools/make_fixture.py`
(`cylinder`, `tube`, `curve`) with the Centauri Carbon's presets, which
have arc fitting on. The length of every feature is what it was as
straight moves, to the rounding of the table, and the spiral lifts are
as many (49, 39 and 29). Fill agrees arc for arc where the two engines
fill alike: sparse infill 252 arcs against the reference's 252 on the
cylinder and 382 against 382 on the tube, the bottom surface 22 against
22 and 63 against 63, and each of the cylinder's layers of solid fill 27
or 28 in both. That took one correction besides the
arcs: with arc fitting on, what is left for infill was outlined at a
fifth of `resolution` like the walls, where the reference outlines it at
`resolution` itself, so the fill's turns along a round wall had twice
the corners and every one of them became an arc. Walls are arcs for the
same share of their length (98 to 99% on the round parts, 56% on the
`curve`), and in as many pieces wherever the reference's fitter makes
no mistake: 198 against 198 on the cylinder, 239 against 278 on the
tube, 295 against 325 on the `curve`.
On the `curve` the arcs are the reference's own, centre for centre (the
80 mm bow, the half-round end, the 1.5 mm corners), bar one: on every
layer the reference has the outer wall's half-round end in two arcs,
the first stopping two corners short of the end, which is where its
mistake with a middle point due east stops an arc. The 39 it has more
on the tube are the same mistake: the inner wall round the hole is cut
in two due north of its middle on 39 of the 40 layers. On the
cylinder and the tube the walls were in more pieces than the
reference's (277 and 404) until every layer was cut without the
crossings inside flat faces (Status, above; these figures are from
after it). A sliced ring kept one point that was no corner, where its
outline started, between two corners of the mesh, and lost the corner
next to it; an arc cannot be carried round past such a point, so a ring
was two arcs, or three, before its seam cut one of them. A ring is now
its corners and nothing else and starts at the reference's corner: one
arc and one straight side, the arc cut in two by the seam. The seam of
a round wall is not where the reference puts it, which cuts the arc
somewhere else but into as many pieces, unless it falls on the ring's
start and cuts nothing: on the `holes` fixture that happens here on
49 loops (the 12 mm hole on 29 layers, the tube on 20), and with the
reference's mistake on 48 loops of the boss and the tube that part's
outer walls are 581 arcs against 678.
Tree supports are arcs for 94% of their length
against 90% (1825 against 2719 on the `shelf`, whose supports are 30%
shorter here, as above). On the box-shaped parts nothing changed but
the spiral lifts, and four arcs on the `mushroom` where a support's
outline turns a corner; the reference has five to eight arcs there,
each a dot of narrow solid fill it lays as a loop, which this engine
does not lay: its triangle of loops there is a stroke larger and has
a second loop where the reference's has the dot (Narrow solid areas,
above).

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
off bridges and bridges over infill are the reference's length to
0.1% (laid from edge to edge, as they were, bridges were 2 to 6% over
and bridges over infill up to 5%); `gaps`, whose bridges over infill
are small and are run out to the sparse lines that carry them, was 33%
under until they were run out here too, and is now the reference's
length to 0.1 mm. Besides the
features shown only the solid and sparse fill a bridge takes the place
of moves, in either slicer, and with the options off the file is byte
for byte what it was.

| Option | Part | Feature | Off | On |
| --- | --- | --- | --- | --- |
| `bridge_line_width` 0.3 | `bridge` | Bridge | 0.67 / 0.67 | 1.05 / 1.05 |
| | | Bridge over infill | 1.01 / 1.01 | 1.30 / 1.30 |
| | `shelf` | Bridge | 0.42 / 0.42 | 0.66 / 0.66 |
| `bridge_expansion_extra` 2, 6, 50 | `bridge`, all solid (`top_shell_layers=16`) | Bridge | 0.69 / 0.69 | 0.77 / 0.77, 0.94 / 0.94, 1.02 / 1.02 |
| `bridge_expansion_extra` 3 | `mushroom`, all solid | Bridge | 2.02 / 2.02 | 2.14 / 2.14 |
| `dont_filter_internal_bridges` `limited` | `gaps` | Bridge over infill | 2.58 / 2.58 | 2.58 / 2.58 |
| `dont_filter_internal_bridges` `nofilter` | `gaps` | Bridge over infill | 2.58 / 2.58 | 2.88 / 2.88 |
| | a wedge 2 to 9.5 mm tall, top at 14° | Bridge over infill | 0.00 / 0.00 | 4.16 / 4.10 |
| `enable_extra_bridge_layer` `external_bridge_only` | `bridge` | Bridge | 0.67 / 0.67 | 1.33 / 1.33 |
| | `mushroom` | Bridge | 1.97 / 1.97 | 3.94 / 3.94 |
| `enable_extra_bridge_layer` `internal_bridge_only` | `cube` | Bridge over infill | 0.84 / 0.84 | 1.68 / 1.68 |
| | `gaps` | Bridge over infill | 2.58 / 2.58 | 5.14 / 5.13 |

Where the two columns agree they agree to a tenth of a millimetre or
two in a metre (the deck's bridge 666.2 mm in both, doubled 1332.3;
the mushroom's 1969.9 and 3939.9; the cube's bridge over infill 838.8
and 1677.7).

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

With no filter the bridges over infill appear on the same layers as
the reference's (every second layer under a sloping top: a bridge is
not laid over one just beneath it) and are its length to 1.5% (the
wedge's 4.10 m against 4.16 m; 2.42 m before they were run out to the
sparse lines that carry them). `limited` adds nothing on `gaps` in
either slicer now: the bridges on the first solid layer, run out,
already cover what it would find on the next. On the plain parts
(cube, deck, mushroom) neither value changes anything in either
slicer.

Bridges over infill, before and after they were run out to the sparse
lines that carry them and turned as the reference turns them
(PrintObject.cpp `bridge_over_infill`), in metres of path, C++ slicer /
this engine before / after, with the Snapmaker U1's presets but where
`_cc` says the Centauri Carbon's; the Benchy and `mcbrim_test.3mf`
plates as saved:

| Part | Bridge over infill | Internal solid fill | Sparse fill | Filament, mm |
| --- | --- | --- | --- | --- |
| `gaps` | 2.58 / 1.73 / 2.58 | 11.29 / 11.25 / 11.30 | 10.22 / 10.38 / 10.22 | 1423.3 / 1393.0 / 1423.8 |
| `holes` | 5.41 / 4.73 / 5.40 | 29.67 / 29.61 / 29.65 | 22.16 / 22.29 / 22.16 | 3314.1 / 3287.4 / 3311.5 |
| `steps` | 2.74 / 2.36 / 2.74 | 13.83 / 13.82 / 13.84 | 7.35 / 7.41 / 7.35 | 1300.5 / 1287.1 / 1300.8 |
| `shelf` | 2.29 / 2.10 / 2.29 | 12.06 / 12.07 / 12.07 | 2.93 / 2.96 / 2.93 | 981.3 / 975.1 / 981.5 |
| `gaps_cc` | 2.35 / 1.80 / 2.36 | 11.30 / 11.28 / 11.32 | 11.16 / 11.31 / 11.17 | 1400.2 / 1378.4 / 1401.6 |
| `holes_cc` | 4.86 / 4.52 / 4.87 | 29.73 / 29.57 / 29.60 | 23.85 / 24.04 / 23.98 | 3238.2 / 3225.1 / 3240.6 |
| `steps_cc` | 2.47 / 2.25 / 2.46 | 13.83 / 13.83 / 13.85 | 8.27 / 8.30 / 8.27 | 1275.1 / 1265.5 / 1275.2 |
| Benchy | 3.48 / 2.43 / 3.63 | 21.35 / 20.23 / 20.80 | 16.24 / 16.56 / 16.23 | 3767.6 / 3718.4 / 3773.4 |
| `mcbrim_test.3mf` plate 1 | 3.02 / 3.01 / 3.59 | 50.51 / 49.41 / 48.85 | 5.62 / 6.93 / 6.89 | 6886.5 / 6809.0 / 6814.2 |
| `mcbrim_test.3mf` plate 2 | 2.60 / 2.66 / 2.66 | 24.29 / 24.52 / 24.52 | 2.72 / 2.72 / 2.72 | 4462.7 / 4463.3 / 4463.3 |

`cube`, `frame`, `tube`, `mushroom` and `counterbore` (whose bridges over
infill cover whole layers, with nothing to run out) are unchanged and
the reference's length to 0.1%, as are `cylinder_drawcut.3mf` and
`twopart_groups.3mf`. Bridge over infill, `gaps` / `holes`, with one
setting changed:

| Setting | `gaps` | `holes` |
| --- | --- | --- |
| `sparse_infill_pattern` rectilinear | 2.28 / 1.73 / 2.29 | 5.18 / 4.74 / 5.17 |
| gyroid | 2.18 / 1.73 / 2.18 | 5.33 / 4.73 / 5.29 |
| cubic | 2.70 / 1.73 / 2.70 | 5.39 / 4.73 / 5.39 |
| honeycomb | 2.75 / 1.73 / 2.74 | 5.26 / 4.73 / 5.25 |
| `sparse_infill_density` 10% | 2.75 / 1.73 / 2.75 | 5.55 / 4.73 / 5.53 |
| 40% | 2.23 / 1.73 / 2.22 | 5.14 / 4.73 / 5.14 |
| `dont_filter_internal_bridges` `limited` | 2.58 / 1.85 / 2.58 | 5.43 / 4.86 / 5.42 |
| `nofilter` | 2.88 / 2.42 / 2.88 | 6.81 / 6.02 / 6.71 |
| `thick_internal_bridges` 0 | 3.50 / 2.34 / 3.48 | 7.29 / 6.39 / 7.29 |
| `enable_extra_bridge_layer` `internal_bridge_only` | 5.14 / 3.46 / 5.13 | 10.79 / 9.46 / 10.77 |
| `top_shell_layers` 1, `top_shell_thickness` 0 | 0.00 / 0.00 / 0.00 | 0.22 / 0.00 / 0.00 |
| `layer_height` 0.12 | 2.62 / 1.72 / 2.63 | 5.37 / 4.73 / 5.39 |

Patch by patch, on the layers where the solid fill beneath them is the
same in both, the bridges cover the reference's area: on `gaps` at
z = 5.25 one patch of 560.6 mm² in both (it was four strips, 214.9 mm²
in all) at 135° in both, its lines' ends on the reference's to 0.3 mm
on average; at z = 8.25 five patches of 490.5 mm² in both. `holes` at
z = 5.25 is 1847.2 mm² against 1848.0 (1645.7 before), the ends 0.04
mm from the reference's on average; `shelf`, `steps` (z = 9.25) and the
Benchy's at z = 5.65, 7.65 and 19.65 the same to 0.3%. Over the 56
parts and settings compared, the direction of each layer's bridges
over infill is the reference's on 138 of 170 layers (56 before). The
rest are close votes: on a part laid out symmetrically about the
pattern's origin, as all of these are, a point of a patch's outline
can be exactly as near a line of each of a grid's two directions, and
which is taken is the reference's search tree's to say; three of the
five patches on `gaps` at z = 8.25 are turned as the reference turns
them, and `steps` at 5.25, `frame` and `tube` are each turned the
other way. Where the surfaces differ the bridges follow: on the Benchy
the reference has a band of solid fill one layer lower (14.05 against
14.25 mm, 213 against 266 mm of bridge) and a different top surface
round the cabin roof; on plate 1 the sparse fill beneath, 23% more of
it here, gives its bridges other directions and 19% more length;
with `top_shell_layers` 1 `holes` has half the reference's solid fill
under its tops (39 against 78 mm at z = 5.85) and none of it is
bridged where the reference bridges 220 mm, and `steps` has a sliver
of solid fill the reference has not, which is bridged here (167 mm).

It costs 10 ms of the Benchy's 100 ms of telling surfaces apart and 24
of plate 1's 241 (best of seven); a whole slice of either is within
the noise of what it was (0.64 to 0.70 s and 3.0 to 3.15 s, best of
five, before and after). Layers whose bridges cannot lie on one
another are worked on at once, and each patch grows and cuts only the
fill near it.

`counterbore_hole_bridging`, against the C++ slicer with the same
settings (C++ slicer / this engine). The part is `tools/make_fixture.py`'s
`counterbore` with the Snapmaker U1's presets: a slab of eight cells,
each with a hole that is wide for the first 3 mm and narrow above
(round in round 3 in 6, 4 in 8 and 5 in 12 mm, 4 in 12 off the middle,
round over hexagonal, square over round, hexagonal over square) and one
plain hole straight through. The layers are the first of the narrow
part and the one above it, cut at 3.15 mm where nothing else is said
(the first layer is 0.25 mm, so 0.2 mm layers do not divide the 3 mm),
at 3.10 mm with a 0.2 mm first layer, which does, and at 3.19 mm with
0.28 mm layers. Lengths are of line laid over all eight cells; the
last column is in how many of the seven holes that narrow the bridges
run the same way in both, within a degree.

| | walls on that layer, mm | bridge on it, mm | solid fill on the layer above, mm | bridges the same way |
| --- | --- | --- | --- | --- |
| off | 753 / 753 | 1540 / 1535 | 1464 / 1467 | |
| `partiallybridge` | 1060 / 1060 | 838 / 847 | 1717 / 1719 | 7 |
| `sacrificiallayer` | 1035 / 1035 | 1771 / 1771 | 1765 / 1790 | 3 |
| `partiallybridge`, Arachne | 1039 / 1039 | 838 / 841 | 1680 / 1689 | 6 |
| `sacrificiallayer`, Arachne | 1033 / 1033 | 1771 / 1771 | 1755 / 1774 | 3 |
| `partiallybridge`, `hole_to_polyhole` | 1062 / 1060 | 854 / 871 | 1711 / 1711 | 4 |
| `sacrificiallayer`, `hole_to_polyhole` | 1040 / 1040 | 1783 / 1778 | 1764 / 1775 | 1 |
| `partiallybridge`, 0.2 mm first layer | 1060 / 1060 | 838 / 847 | 1717 / 1719 | 7 |
| `sacrificiallayer`, 0.2 mm first layer | 1035 / 1035 | 1771 / 1771 | 1765 / 1790 | 3 |
| `partiallybridge`, 0.28 mm layers | 1056 / 1060 | 882 / 879 | 1783 / 1757 | 5 |
| `sacrificiallayer`, 0.28 mm layers | 1033 / 1033 | 1840 / 1840 | 1825 / 1827 | 3 |

The solid fill on the layer above is the roofs over the wide holes,
each an area of its own, and with a sacrificial layer round ones with
nothing narrow in them: their lines are fitted as the reference's are
(on that layer 343, 131 and 77 mm of line 0.424, 0.430 and 0.450 mm
wide in the reference and 349, 131 and 87 mm of 0.424, 0.430 and 0.43
to 0.47 here, all of it 0.42 before), each line where the reference's
is. The lines themselves come to 1270 mm against the reference's 1280;
the 1.4% the layer is over is in the joins along the roofs' edges and
round the narrow holes, which the two go about in different orders,
and in the loops of the narrow parts: 520 mm against 485 (515 before
the lines were fitted).

The walls of that layer are the reference's: they go round the wide
hole, stepping in a wall's width where a bridge rests on its edge.
With a sacrificial layer they come to the same length in every cell of
every row, to 0.05 mm; partly bridged to within 0.5 mm, but for the
six-sided wide hole with 0.28 mm layers, where a bridge reaches a
corner of the hexagon here and stops a detector's strip short of it
there (82.3 mm against 78.9), and two polyholes whose slot runs
another way (1.1 mm). Over the whole file no layer's walls are off by more than 0.4%,
and the walls over air that the option is there to be rid of (243 mm
on that layer with it off) are gone in both. The plain hole is
untouched. The layers next to that one change in both slicers the same
way, their fill now lying under or over its walls round the wide hole
and being made solid: a ring round the wide hole on the four layers
under it (224 mm a layer in the reference and 256 here, partly
bridged) and more solid fill on the two above (253 and 742 mm more
than with the option off in the reference, 243 and 735 here). Further
up only where the seams fall on the narrow holes' walls changes, and
further down nothing does. With `enable_extra_bridge_layer` on as well
both lay the second bridge on the layer above in the six cells whose
roof is wide enough for one, the same way in five of them (2.50 m of
bridge in the file against 2.52 m partly bridged, 3.49 against 3.49
with a sacrificial layer). On 3DBenchy with the same presets one layer
changes in both, at 34.25 mm, the same in either mode: 43.9 mm more
bridge and 39.3 mm less wall over air in the reference, 43.9 and 38.4
here.

The bridges are laid as the reference lays them, and where they run
the same way they come to its length: with a sacrificial layer 1771.3 mm
in both (the roof is covered whole, whichever way), and with the option
off 1535.4 mm against 1540.4, cell by cell within 0.15 mm in six
(231.4, 131.2, 182.0, 304.0, 273.4 and 332.8 mm in the reference) and
4.7 mm short in the seventh, the 3 mm hole in the 6 mm one, which the
two span different ways (22.5° here, 112.5° there). A row that the
narrow hole parts is laid round the side of the hole on the first row
to meet it and the last, as there. Partly bridged they are 1.1% over
(847.4 mm against 838.2): five cells are within 0.8 mm, and in two (the
round hole over the hexagonal one and the square one over the round)
one of the two strips beside the slot runs a degree off the
reference's and the lines in it fall differently (106.7 mm against
100.7, 103.1 against 99.1). They were 12% over partly bridged, 4% with
a sacrificial layer and 5.5% with the option off, when every bridge
line that was inside the area at all was laid, from edge to edge:
eight lines where the reference has seven in each of the two beside
the hexagonal hole in the square one.

Partly bridged, the bridges run along the slot in both slicers, and
the slot runs the same way in all seven cells with the presets as they
are and with a 0.2 mm first layer, and in four to six of the seven in
the other rows. Where it does not, the two ways span the same: a round
hole in a round one is spanned as well any way round, wherever in it
the hole is, and a square or six-sided one as well along any of its
sides. The choice between equals is made by the last nanometre of the
outline, in the reference as here, and the two clippers do not share
that. With a sacrificial layer the cover is held all round and is laid
the short way across, and a round, square or six-sided cover has none,
so there the last column says little; a wide hole 14 by 6 mm, tried
apart, is covered across the 6 mm in both, and partly bridged across
the 6 mm in both.

Different from the reference on purpose, both marked in the code. The
reference does this to every part of a region with nothing under it
that can be spanned, so long as the region has a hole anywhere
(`!surface->expolygon.holes.empty()`); here the hole has to be on the
edge of that part, which is what makes it a hole that narrows. A deck
across two posts with a small hole drilled into one post loses the
walls of its first layer in the reference (118 mm of wall over air
with the option off, none with it on, in either mode) and keeps them
here; a table top on four legs with such a hole has, partly bridged,
only the strips from leg to leg printed in its first layer in the
reference (1179 mm of bridge where 3472 mm is laid with the option
off, or with it on and no hole), the layer after going down on air
between them, and is left as it was here. And a sacrificial layer is
made only where the wide hole's outline is convex, which the reference
tells by whether any corner of the outline, simplified, turns the
wrong way at all: simplifying keeps the outline's first point, and
where that is in the middle of a straight side rounding leaves it a
nanometre off, to either side. The hexagonal hole's first point lies a
nanometre inside here; a corner counts as turning the wrong way only
from a tenth of a micron.

Noted and not done: with `bridge_angle` set and
`align_infill_direction_to_model` on, the reference turns that
direction with the model, whose turn on the bed is not known here.

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

Paths thinned to `resolution`, 2026-11-11, with the U1's presets (arcs
off, `resolution` 0.012) on parts from `tools/make_fixture.py`, a
Benchy and the three plates of `tests/mcbrim_test.3mf`. Until now a
path was thinned only while arcs were fitted to it, and otherwise
written with every point it was made with. Counted are the extruding
`G1` moves of each file: the reference's, this engine's before and
after.

| part | wall moves: reference / before / after | all extruding moves | wall length a layer, most it differs by, mm: before / after | file, kB |
|---|---|---|---|---|
| `cylinder` | 6343 / 12449 / 6431 | 8774 / 15139 / 8915 | 0.036 / 0.002 | 393 / 581 / 388 |
| `tube` | 15215 / 15157 / 15140 | 19830 / 20912 / 19715 | 0.296 / 0.295 | 731 / 767 / 728 |
| `curve` | 4143 / 4705 / 4109 | 6432 / 7585 / 6632 | 0.554 / 0.539 | 297 / 322 / 289 |
| `holes` | 28803 / 28713 / 28670 | 45428 / 49302 / 47110 | 4.925 / 4.920 | 1857 / 1919 / 1847 |
| `funnel` | 7275 / 7466 / 7213 | 10487 / 13447 / 12094 | 0.037 / 0.003 | 458 / 529 / 487 |
| Benchy | 49982 / 64729 / 61185 | 75299 / 108530 / 91290 | 19.308 / 19.287 | 3438 / 4332 / 3780 |
| `mcbrim_test.3mf` plate 1 | 85037 / 82041 / 79974 | 172588 / 191458 / 164570 | 859.4 / 859.5 | 6298 / 6448 / 5604 |
| plate 2 | 40733 / 41888 / 40816 | 89823 / 106818 / 91187 | 12.490 / 12.393 | 3181 / 3586 / 3099 |
| plate 3 | 40733 / 41917 / 40817 | 89823 / 106774 / 91162 | 12.653 / 12.519 | 3184 / 3587 / 3101 |
| `cylinder`, `resolution=0.05` | 3174 / 3261 / 3252 | 4903 / 5268 / 5069 | 0.007 / 0.007 | 261 / 260 / 252 |
| `curve`, `resolution=0.05` | 2114 / 2228 / 2094 | 4092 / 4760 / 4387 | 0.025 / 0.017 | 217 / 222 / 209 |
| `cylinder`, `resolution=0` | 12679 / 12768 / 12768 | 16406 / 16742 / same file | 0.001 / 0.001 | 685 / 778 / 778 |
| `curve`, `resolution=0` | 8439 / 11201 / 11201 | 11397 / 15038 / same file | 0.642 / 0.642 | 484 / 630 / 630 |
| `cylinder`, Arachne | 6360 / 6336 / 6336 | 8783 / 9036 / 8816 | 0.111 / 0.111 | 394 / 394 / 385 |
| `curve`, Arachne | 3383 / 3477 / 3412 | 5648 / 6234 / 5905 | 0.021 / 0.021 | 275 / 277 / 267 |
| `holes`, Arachne | 20708 / 20797 / 20497 | 36203 / 40453 / 38217 | 0.232 / 0.231 | 1573 / 1632 / 1559 |
| Benchy, Arachne | 57778 / 71697 / 69180 | 84068 / 116149 / 104415 | 7.503 / 7.499 | 3880 / 4616 / 4241 |
| `cylinder`, `seam_slope_type=all` | 7365 / 14507 / 7460 | 9796 / 17197 / 9944 | 0.037 / 0.002 | 434 / 672 / 433 |
| `funnel`, scarf seams | 8841 / 9174 / 8903 | 12053 / 15155 / 13784 | 0.049 / 0.049 | 520 / 602 / 559 |
| `curve`, scarf seams | 5084 / 5834 / 4983 | 7373 / 8714 / 7506 | 0.506 / 0.540 | 329 / 361 / 321 |
| Benchy, scarf seams | 72399 / 91267 / 86980 | 97712 / 135068 / 117085 | 19.804 / 19.810 | 4384 / 5524 / 4936 |
| `cube` | 796 / 796 / 796 | 3729 / 4069 / same file | 0 / 0 | 328 / 314 / 314 |
| `cube`, Arachne | 796 / 796 / 796 | 3739 / 4069 / same file | 0 / 0 | 331 / 315 / 315 |
| `cube`, scarf seams | 4063 / 4063 / 4063 | 6996 / 7336 / same file | 0 / 0 | 443 / 438 / 438 |
| `frame` | 561 / 501 / 501 | 6083 / 7007 / same file | 0 / 0 | 331 / 316 / 316 |
| `steps` | 556 / 526 / 526 | 4896 / 5711 / 5707 | 0 / 0 | 330 / 302 / 302 |
| `mushroom`, supports | 556 / 556 / 556 | 5484 / 5650 / 5648 | 0 / 0 | 353 / 338 / 337 |

The cylinder's rings are 64 or 65 moves in the reference and 65 here,
where they were 128: a corner of its outer wall, 9.79 mm out, is
11.8 µm from the line between its neighbours, so every other one goes.
(The tube's outer rings lie further out, where a corner stands 14 µm
off, and both engines keep them all; the rings of its hole, and the
Arachne walls of all these parts, came thinned already.)
Fill moved most: on the Benchy gap fill is 6680 moves against the
reference's 6501 (it was 12967), sparse infill 4630 against 4594
(6612) and the inner walls 21255 against 20866 (23270); on the tube
solid fill is 1400 against 1409 (1952) and sparse infill 2659 against
2684 (3133); with gyroid fill the cylinder is 14871 moves against
14680 (21400). On plates 2 and 3 of the lettered project the walls
are 40816 and 40817 moves against 40733 (41888 and 41917). With
`resolution` at nothing no point of a round part lies
exactly on a line and the files are the same as before. With arc
fitting on, every file is byte for byte what it was (five of the
parts above, the Benchy and the three plates).

On the parts made of boxes the walls do not move. `cube` and `frame`
are the same file; `steps` and `mushroom` lose four moves and two,
each where a fill line or a support line turns through a stub 2 to
19 µm long, whose first corner goes as it does in the reference. (In
`steps` the fan also dips to 99% on four layers, and in other files
on other layers than before. That is an error of this engine's, there
before this and not in the reference: the fan's place between its
two speeds is worked out in double precision here and in single
there, and with both speeds at 100% the sum here can land a hair
under and be cut to 99. Which layers it lands on moves with any
change in a layer's time; `main` has it on eight layers of the
Benchy.) What is left between the two engines is upstream of this. On the Benchy the outer wall is 39659
moves against 28799 because a wall measured against the layer beneath
is laid in the pieces it was measured in, also where neighbouring
pieces run on at one speed; with `enable_overhang_speed` off it is
27304 against 27686. (Since closed: 28525 against 28799, under "Walls
over overhangs" below.) On plate 1 the engines lay different lengths of
wall on ten layers of sixteen, which is where its 859 mm comes from;
on the other six the walls are 16055 moves against 16074 (16334).
And a sliced ring here keeps a point that is no corner, as under
"Arcs" above, so a ring is one move longer than the reference's, the
wrong one of two corners is sometimes kept beside that point (the
tube's layer extents differ by up to 0.014 mm on six layers, where
they differed by up to 0.004), and the ring is thinned from a different corner:
of the cylinder's 128 this engine keeps the even ones and the
reference the odd, which is the 0.012 mm their layer extents still
differ by. Tried with the layers cut so that a ring is exactly its
mesh corners (a change not yet merged), the cylinder's walls are 6336
moves against 6343 and with scarf seams 7365 against 7365, the tube's
15205 against 15215, and the layer extents of both agree to the
micron.

Slicing is no slower, to what could be measured with other work
running on the machine: thinning takes about what writing fewer moves
saves. Plate 1 of `mcbrim_test.3mf`, best of 7: 3.63 s before and
3.60 s after on all cores, 15.32 s and 15.40 s of processor time on
one thread; the Benchy 2.94 s and 2.91 s on one thread.

Walls over overhangs, 2026-11-11, with the U1's presets (arcs off) on
a Benchy, plate 1 of `tests/mcbrim_test.3mf`, parts from
`tools/make_fixture.py` and five larger ones made for this, whose
layers are long enough that neither slicer slows them to cool: `lean`
(a square post 80 mm across whose sides lean out 0.06 to 0.30 mm a
layer), `twist` (square plates each turned 1.5 degrees on the last),
`overstep` (plates each reaching 0.05 to 0.8 mm further out on one
side), `notch` (a plate over a block with notches in its side) and a
funnel 60 mm across. Until now a wall was probed against the layer
beneath every 2 mm and halved down to 0.25 mm wherever its two ends
printed differently, and written in those pieces. Where a hull leans
out a little all the way round every line of it printed differently
at its two ends, so it was written in quarter-millimetre moves: four
for each line of a Benchy's hull. The reference measures a wall at its
own points and adds a point only where the wall crosses the outline
beneath, where a probe inside a line of 2 mm or more prints
differently from what is either side of it, and beside the end of
such a line that hangs out (ExtrusionProcessor.hpp
`estimate_points_properties`); each line then goes at the slower of
its two ends, and the writer leaves a speed alone that is within
1 mm/s of the last (GCode.cpp `_extrude`). That is ported, in the
reference's single precision, with the cut between wall and overhang
wall made where the reference makes it (PerimeterGenerator.cpp: at
the layer beneath grown by half a nozzle with mitred corners, where
this engine went by the distance, which rounds them; the places are
found from the lines that grown outline is made of, without growing
it) and each stretch measured as a path of its own: an overhang wall
by the bridge's width and against the inner walls' speed, a wall by
its own.

| part | outer wall moves: reference / before / after | inner wall moves | overhang wall moves | overhang wall, mm |
|---|---|---|---|---|
| Benchy | 28799 / 39659 / 28525 | 20866 / 21279 / 21110 | 317 / 269 / 348 | 193.3 / 174.1 / 192.5 |
| Benchy, Arachne walls | 26706 / 38140 / 26517 | 30771 / 30760 / 30586 | 301 / 241 / 321 | 191.8 / 173.0 / 191.4 |
| Benchy, scarf seams | 51126 / 65281 / 50336 | 20866 / 21279 / 21110 | 407 / 328 / 433 | 232.0 / 205.0 / 227.2 |
| Benchy, arcs fitted | 20867 / 31626 / 20891 | 9486 / 9840 / 9744 | 210 / 192 / 220 | 193.9 / 173.7 / 192.5 |
| Benchy, `enable_overhang_speed` off | 28705 / 27585 / 28420 | 20862 / 21065 / 21106 | 317 / 259 / 348 | 193.3 / 173.4 / 192.5 |
| `mcbrim_test.3mf` plate 1 | 47638 / 44149 / 44155 | 37343 / 35772 / 35742 | 56 / 39 / 46 | 101.0 / 70.5 / 75.8 |
| `holes` | 14527 / 14450 / 14456 | 14266 / 14154 / 14154 | 10 / 6 / 9 | 36.4 / 34.9 / 36.3 |
| `shelf` | 303 / 301 / 303 | 299 / 299 / 299 | 6 / 8 / 6 | 96.0 / 95.7 / 96.0 |
| `twist` | 385 / 1504 / 384 | 493 / 1493 / 492 | 112 / 224 / 112 | 915.1 / 895.8 / 914.3 |
| `lean` | 409 / 390 / 373 | 258 / 236 / 236 | 0 / 0 / 0 | |
| `overstep` | 238 / 233 / 237 | 222 / 221 / 221 | 6 / 4 / 6 | 120.1 / 119.6 / 120.0 |
| `notch` | 419 / 420 / 417 | 415 / 431 / 428 | 10 / 13 / 10 | 34.0 / 31.7 / 34.0 |

With the overhang speeds off the reference still cuts a wall where
the overhang fan comes on and goes off, which is why that row moves
too. `cube`, `steps`, `frame`, `cylinder`, `tube` and `curve`, which
overhang nowhere, are the files they were to the byte, also with
Arachne walls, scarf seams and arcs; so is `mushroom`, whose cap
hangs clear all the way round. The small `funnel`, every layer of
which is slowed to cool to one speed in both files, is 3723 outer
wall moves against 3767 (3753).

Where the speed changes, and to what. A run is a stretch of a wall
loop written at one feed rate under one name; loops are paired by
layer and place, and a run's end counts as the same in both files
when it is within 0.3 mm with feed rates within 3% either side. The
last two columns walk every paired loop in steps of 0.2 mm and
compare the feed rates there, whatever points each file has, over the
length that goes slower than the wall's own speed in either file.

| part | runs: reference / before / after | run ends at the same place with the same speeds: before / after | same place, other speeds | only in the reference | only here | overhanging length within 3% of the reference's feed rate: before / after | laid under another name, mm |
|---|---|---|---|---|---|---|---|
| Benchy | 4136 / 7356 / 4003 | 735 / 1461 | 1402 / 664 | 542 / 554 | 3765 / 424 | 40.3% / 81.0% | 18.2 / 0.6 |
| `twist` | 683 / 3005 / 695 | 0 / 588 | 513 / 16 | 91 / 0 | 2413 / 12 | 61.2% / 99.1% | 20.3 / 0.0 |
| `lean`, seam at the back | 252 / 269 / 252 | 0 / 133 | 0 / 0 | 133 / 0 | 150 / 0 | 93.1% / 100.0% | 0.0 / 0.0 |
| `overstep`, seam at the back | 139 / 134 / 139 | 1 / 30 | 10 / 0 | 19 / 0 | 14 / 0 | 97.4% / 100.0% | 0.6 / 0.0 |
| `notch` | 93 / 102 / 93 | 2 / 44 | 22 / 0 | 20 / 0 | 29 / 0 | 56.7% / 100.0% | 2.3 / 0.0 |
| `shelf` | 161 / 161 / 161 | 0 / 12 | 8 / 0 | 4 / 0 | 4 / 0 | 95.3% / 100.0% | 0.4 / 0.0 |
| `holes` | 828 / 825 / 826 | 2 / 12 | 17 / 18 | 13 / 2 | 10 / 0 | 91.0% / 94.4% | 1.5 / 0.0 |

On the parts made of boxes, where both slicers have the same walls to
measure, the files now agree move for move along every overhang, to
the micron and the feed rate: `shelf`, `notch`, and `lean` and
`overstep` with `seam_position` at `back`, which puts both slicers'
seams at one corner. On layer 52 of `lean`, for one: 91.296 mm of
three sides at F1980, the side that ends at the seam at F1920, and
0.967 mm either side of each corner, where the wall is further from
the layer beneath, at F1260, in both. What is left on the others
comes from outside this:

- The wall's own speed. This engine used to leave the flow ratios out
  of what the hot end can melt and to write a measured wall's speed as
  a whole number of mm/s (F9540 against F9741.893). Both now follow
  the reference (below, under "Print time against the reference").
- Cooling. 218 of the Benchy's 240 layers are slowed to cool in the
  reference's file and 223 in this engine's, and a slowed layer's top
  speed is a few per cent lower here, because the reference counts a
  slowed layer as longer than it is (below), so a move held by cooling
  in one file and by an overhang in the other differs by that. On the
  20 layers neither slows, 88.7% of the overhanging length is within
  3% (42.1% before).
- The points a loop has. A round outline is thinned from another
  corner here (above), and the reference thins each stretch of a wall
  between the cuts by itself where this engine thins the loop whole,
  so a hull's lines begin and end in other places: each still goes at
  the slower of its ends, by the same rule, but the ends are not the
  same points, which is most of the Benchy's "other speeds". Where a
  wall runs alongside the outline beneath at about half a nozzle out,
  as the Benchy's stern does on layer 45, ten microns either way move
  the cut between wall and overhang wall by half a millimetre.
- The seam. A loop's last move stops short of the seam by the seam's
  gap, and the reference may start a loop where a wall turns into an
  overhang wall, which is a point of the loop there and not here. The
  line that ends at the seam is measured to where it stops, so its
  speed goes by where the seam is.
- Arachne walls with `thick_bridges` off. The reference lays an
  overhang wall of an Arachne loop at the wall's own width and flow
  (VariableWidth.cpp `thick_polyline_to_multi_path` keeps the bridge's
  line only when it is a round thread), where its classic walls get
  the bridge's 0.369 mm at `bridge_flow`; and since a wall is measured
  by its width, the first and last move of such a stretch go at
  43 mm/s where the bridge speed is 50. Here an overhang wall is the
  bridge's line whichever way the walls were made, so `twist` with
  Arachne walls is 94.1% within 3% where it is 99.1% with classic.
- Arcs. With arcs fitted the files agree as without (`twist` 99.5%,
  `notch` 100% with the Centauri Carbon's presets), but for the wall
  beside the seam on a part that overhangs all round: the reference
  moves the point either side of the seam 7 µm out onto the arc it
  cut there, and on a funnel 60 mm across that is 74 mm/s against 82
  for 2 mm each side.

Two things the reference does are not copied. After an overhang its
first move back at full speed is written at the whole number of mm/s
its measuring rounds to (F9720) and only the next at the wall's speed
(F9741.893), or none if the path ends there, because it asks whether
the speed has changed by more than 1 mm/s before it asks whether the
wall is back at its own; here it is back at its own speed at once.
And where it compares a crossing with the point before it, it reads
that point through a reference into a list it is adding to
(ExtrusionProcessor.hpp `prev_point`), which is freed memory once the
list has grown; here it is the point.

Print time does not move by more than seconds: the speeds along an
overhang were near enough before, in pieces. The Benchy is 41 m 13 s
as the reference plans it and 42 m 33 s as Klipper does, before and
after, against the reference's 38 m 43 s (its file timed here: 38 m
43 s and 39 m 51 s); `twist` 35 m 00 s before and 35 m 08 s after
against 32 m 53 s, `lean` 53 m 04 s and 53 m 06 s against 48 m 29 s,
plate 1 62 m 37 s both against 65 m 01 s. Those gaps are not in the
walls over overhangs, which agree on `lean` to the move. Where they are is below, under "Print time against the
reference".

Writing is no slower, to what can be measured with other work running
on the machine: measuring a wall at its own points asks less of the
layer beneath than probing it did, and telling where it hangs clear
asks a little more. On one thread, best of 8, the Benchy's G-code is
written in 131 ms before and 137 ms after, of 1.92 s of processor
time for the whole of it both times; `holes` in 36 ms and 42 ms;
plate 1 of `mcbrim_test.3mf` in 554 ms and 550 ms. The files are the
same at 1, 4 and 18 threads.

### Regions

Fixtures from `tools/make_fixture.py` on the U1 presets, against the C++
slicer: metres of each feature, the reference's, then this engine's
before regions were followed and after.

| Fixture | Feature | Reference | Before | After |
| --- | --- | ---: | ---: | ---: |
| cube, object: 4 walls, 40% | inner wall / sparse | 21.34 / 27.18 | 7.44 / 14.36 | 21.34 / 27.17 |
| block and post, post: 5 walls, 50% gyroid | inner wall / sparse | 8.72 / 5.94 | 3.72 / 5.42 | 8.75 / 5.93 |
| block and post, post's speeds | filament mm | 750.0 | 696.9 | 749.4 |
| cube, object: brim 8 mm | brim | 1.66 | 0 | 1.66 |
| modifier, right half 50% | sparse / outer wall | 27.03 / 7.82 | 14.36 / 7.82 | 27.03 / 7.82 |
| modifier, right half 4 walls | inner / outer wall / sparse | 20.85 / 11.65 / 11.93 | 7.44 / 7.82 / 14.36 | 20.85 / 11.65 / 12.34 |
| two modifiers overlapping | inner wall / sparse | 20.85 / 21.69 | 7.44 / 14.36 | 20.85 / 21.69 |
| height range 5-10 mm: 3 walls, 60% | inner wall / solid / sparse | 9.23 / 5.10 / 21.99 | 7.44 / 4.68 / 14.36 | 9.23 / 5.11 / 21.86 |
| modifier, right half speeds | filament mm | 1415.9 | 1206.9 | 1416.3 |
| mushroom, blocker, normal | support | 24.14 | 23.75 | 23.75 |
| mushroom, blocker, tree | support / interface | 11.26 / 0.93 | 23.89 / 1.79 | 12.38 / 0.90 |
| leaning block, enforcer, normal | support | 3.23 | 0 | 3.25 |
| leaning block, enforcer, tree | support / interface | 2.72 / 1.48 | 0 / 0 | 3.22 / 1.00 |
| leaning block, painted, normal | support / interface | 4.25 / 1.32 | 0 / 0 | 4.68 / 0 |
| leaning block, painted, tree | support / interface | 6.08 / 3.25 | 0 / 0 | 5.41 / 2.20 |
| leaning block, enforcer, `tree(manual)` | support / interface | 2.72 / 1.48 | 0 / 0 | 3.22 / 1.00 |

Walls are the reference's to the metre on every layer of every
modifier and range fixture; on the height range the solid fill the
range's deeper walls ask of the layers next to it is there layer by
layer. Left: with four walls in a narrow modifier region, sparse lines
there are joined along the region's edge where the reference leaves
them apart (+3.4% of sparse); travel between regions is up to a fifth
shorter here, the order of regions within a layer being this engine's;
painted supports come out within 11% of the reference's, the painted
faces projected a little differently from `slice_mesh_slabs`. The
reference's shipped projects with modifiers or part settings either do
not slice in the reference from the command line (`override_paint_*`,
`post_process_project`), stop here on a printer template variable not
yet known (`auto_pa_line_*`), or set nothing this engine follows
(EdgeSlicer support groups in `support_corpus`, noted; `stabilizer_v1`,
stabilizers): their output is what it was. One of them,
`support_corpus/twopart_groups_oversupport.3mf`, has a part that sets
`over_support_*`, region-level, so the part is a region of its own: its
walls are now the reference's on every layer (3.82% apart before, 0.00%
now), filament 615.3 mm against 615.1 (606.5 before).

### Top surfaces on slopes

2026-11-11, on parts from `tools/make_fixture.py` with the U1's presets
(and the Centauri Carbon's where `_cc` is named; with one option changed
where it is named: `oowt0` is `only_one_wall_top` off, `tsl1_t0`
`top_shell_layers` 1 with `top_shell_thickness` 0, `lh012` a 0.12 mm
layer), 3DBenchy, the lettered plates and the owner's figurine. Each cell
is the C++ slicer / this engine before / this engine now, in millimetres
of path; the time is as each file states it (this engine's as Klipper
plans it, above).

| Part | Top surface | Bottom surface | Internal solid | Sparse | Bridge | Internal bridge | Filament, mm | Time |
|---|---|---|---|---|---|---|---|---|
| wedge | 1567 / 1836 / 1567 | 1135 / 1135 / 1135 | 7431 / 7090 / 7420 | 2407 / 2431 / 2428 |  | 1667 / 1678 / 1679 | 611.5 / 610.9 / 612.4 | 5m 10s / 5m 31s / 5m 20s |
| holed | 2185 / 2566 / 2184 | 1582 / 1575 / 1575 | 10561 / 10008 / 10480 | 3709 / 3753 / 3753 |  | 2269 / 2341 / 2306 | 882.1 / 881.6 / 882.5 | 6m 31s / 7m 3s / 6m 46s |
| ramp | 1810 / 2110 / 1810 | 1339 / 1339 / 1339 | 8334 / 7957 / 8333 | 2440 / 2444 / 2444 |  | 1819 / 1828 / 1824 | 653.8 / 653.3 / 654.2 | 4m 41s / 4m 59s / 4m 48s |
| dome | 398 / 427 / 399 | 851 / 850 / 850 | 5929 / 5869 / 5914 | 1607 / 1608 / 1607 |  | 210 / 216 / 211 | 390.7 / 392.4 / 392.3 | 4m 9s / 4m 21s / 4m 20s |
| mushroom | 2280 / 2280 / 2280 | 143 / 143 / 143 | 10930 / 10932 / 10935 | 5972 / 5776 / 5775 | 1970 / 1970 / 1970 | 1993 / 1993 / 1993 | 1002.2 / 995.8 / 995.8 | 10m 32s / 10m 34s / 10m 34s |
| steps | 2874 / 2874 / 2874 | 2159 / 2159 / 2159 | 13832 / 13839 / 13840 | 7345 / 7345 / 7345 |  | 2740 / 2740 / 2745 | 1300.5 / 1300.8 / 1301.0 | 10m 11s / 10m 29s / 10m 29s |
| holes | 5962 / 5966 / 5962 | 4316 / 4297 / 4297 | 29671 / 29649 / 29651 | 22162 / 22164 / 22160 | 162 / 162 / 162 | 5407 / 5397 / 5407 | 3314.1 / 3311.5 / 3311.9 | 23m 32s / 24m 22s / 24m 22s |
| cube | 983 / 983 / 983 | 728 / 728 / 728 | 4676 / 4680 / 4680 | 14509 / 14357 / 14357 |  | 839 / 839 / 839 | 1211.9 / 1206.9 / 1206.9 | 14m 59s / 15m 18s / 15m 18s |
| frame | 1608 / 1608 / 1608 | 962 / 962 / 962 | 7062 / 7066 / 7066 | 4250 / 4250 / 4250 |  | 1204 / 1204 / 1204 | 1034.4 / 1034.5 / 1034.5 | 7m 19s / 7m 31s / 7m 31s |
| tube | 803 / 803 / 803 | 520 / 512 / 512 | 3591 / 3567 / 3567 | 3194 / 3194 / 3194 |  | 624 / 624 / 624 | 577.9 / 578.1 / 578.1 | 6m 46s / 7m 2s / 7m 2s |
| funnel | 1112 / 1112 / 1112 | 175 / 175 / 175 | 4216 / 4150 / 4150 | 2657 / 2657 / 2657 |  | 773 / 774 / 774 | 440.5 / 441.4 / 441.4 | 6m 56s / 7m 4s / 7m 4s |
| curve | 1596 / 1596 / 1596 | 1211 / 1211 / 1211 | 7689 / 7642 / 7642 | 4617 / 4619 / 4619 |  | 1385 / 1386 / 1386 | 744.3 / 744.4 / 744.4 | 5m 49s / 5m 55s / 5m 55s |
| wedge_cc | 1452 / 1752 / 1452 | 1098 / 1098 / 1098 | 7593 / 7260 / 7578 | 2700 / 2700 / 2700 |  | 1447 / 1508 / 1508 | 595.5 / 597.5 / 597.8 | 5m 20s / 5m 50s / 5m 30s |
| holed_cc | 2058 / 2482 / 2058 | 1533 / 1528 / 1528 | 10696 / 10243 / 10670 | 3818 / 3808 / 3808 |  | 2038 / 2101 / 2101 | 850.3 / 851.8 / 852.3 | 6m 52s / 7m 42s / 7m 14s |
| ramp_cc | 1701 / 2000 / 1701 | 1283 / 1283 / 1283 | 8606 / 8282 / 8617 | 2575 / 2571 / 2571 |  | 1492 / 1527 / 1527 | 628.3 / 629.7 / 630.1 | 5m 3s / 5m 33s / 5m 13s |
| dome_cc | 832 / 1524 / 832 | 829 / 829 / 829 | 5843 / 4925 / 5769 | 1624 / 1624 / 1624 |  | 98 / 98 / 98 | 390.5 / 389.4 / 389.8 | 4m 37s / 5m 29s / 4m 51s |
| holes_cc | 5803 / 5859 / 5803 | 4176 / 4169 / 4169 | 29732 / 29598 / 29665 | 23849 / 23978 / 23978 | 166 / 166 / 166 | 4861 / 4868 / 4868 | 3238.2 / 3240.6 / 3240.7 | 20m 43s / 22m 30s / 22m 26s |
| cube_cc | 975 / 975 / 975 | 699 / 699 / 699 | 4676 / 4680 / 4680 | 14061 / 14061 / 14061 |  | 760 / 760 / 760 | 1155.3 / 1155.4 / 1155.4 | 9m 51s / 10m 6s / 10m 6s |
| wedge_oowt0 | 1649 / 1649 / 1649 | 1135 / 1135 / 1135 | 7131 / 7088 / 7111 | 2407 / 2431 / 2428 |  | 1664 / 1670 / 1672 | 608.2 / 608.0 / 608.6 | 5m 7s / 5m 16s / 5m 16s |
| wedge_tsl1_t0 | 1566 / 1567 / 1567 | 1135 / 1135 / 1135 | 2796 / 2797 / 2797 | 3395 / 3395 / 3395 |  |  | 433.8 / 433.8 / 433.8 | 4m 30s / 4m 37s / 4m 37s |
| wedge_tsl3_t0 | 1567 / 1836 / 1567 | 1135 / 1135 / 1135 | 4631 / 4293 / 4617 | 2838 / 2862 / 2859 |  | 1667 / 1678 / 1679 | 540.0 / 539.9 / 540.8 | 5m 1s / 5m 21s / 5m 10s |
| wedge_arachne | 1572 / 1869 / 1572 | 1135 / 1135 / 1135 | 7446 / 7105 / 7437 | 2417 / 2423 / 2428 |  | 1677 / 1681 / 1672 | 612.3 / 611.8 / 612.3 | 5m 10s / 5m 32s / 5m 19s |
| wedge_lh012 | 1557 / 2015 / 1557 | 1135 / 1135 / 1135 | 12419 / 11983 / 12517 | 4057 / 4038 / 4050 |  | 1367 / 1375 / 1401 | 602.6 / 603.0 / 605.5 | 7m 29s / 7m 58s / 7m 41s |
| wedge_lh028 | 1614 / 1826 / 1614 | 1135 / 1135 / 1135 | 7856 / 7638 / 7855 | 1345 / 1345 / 1345 |  | 1470 / 1481 / 1477 | 689.0 / 689.0 / 689.4 | 4m 22s / 4m 39s / 4m 30s |
| dome_tsl1_t0 | 267 / 262 / 267 | 851 / 850 / 850 | 2194 / 2086 / 2168 | 2206 / 2237 / 2218 |  | 58 / 0 / 23 | 283.1 / 279.1 / 282.0 | 4m 1s / 4m 12s / 4m 12s |
| dome_arachne | 377 / 445 / 377 | 851 / 850 / 850 | 5970 / 5869 / 5961 | 1616 / 1608 / 1607 |  | 210 / 217 / 214 | 391.1 / 392.9 / 393.1 | 4m 9s / 4m 23s / 4m 20s |
| dome_lh028 | 966 / 1020 / 965 | 851 / 850 / 850 | 5730 / 5504 / 5598 | 906 / 920 / 917 |  | 313 / 394 / 388 | 459.5 / 459.5 / 460.0 | 3m 33s / 3m 47s / 3m 46s |
| holes_tsl1_t0 | 5880 / 5874 / 5880 | 4316 / 4297 / 4297 | 12041 / 12019 / 12020 | 25642 / 25691 / 25689 | 162 / 162 / 162 | 220 / 0 / 0 | 2671.9 / 2662.9 / 2663.1 | 20m 32s / 21m 13s / 21m 13s |
| holes_arachne | 5888 / 5948 / 5888 | 4329 / 4310 / 4310 | 29029 / 28946 / 29016 | 22569 / 22569 / 22565 | 162 / 162 / 162 | 5411 / 5437 / 5448 | 3298.5 / 3297.4 / 3297.7 | 23m 35s / 24m 20s / 24m 17s |
| steps_tsl1_t0 | 2874 / 2867 / 2874 | 2159 / 2159 / 2159 | 5329 / 5342 / 5330 | 9025 / 9001 / 9025 |  | 0 / 167 / 0 | 983.5 / 989.8 / 983.5 | 9m 8s / 9m 29s / 9m 24s |
| wedge_zaa | 1567 / 1866 / 1567 | 1135 / 1135 / 1135 | 7481 / 7048 / 7383 | 2492 / 2481 / 2472 |  | 1517 / 1551 / 1561 | 598.3 / 595.5 / 596.8 | 5m 6s / 5m 31s / 5m 19s |
| dome_zaa | 427 / 467 / 426 | 851 / 850 / 850 | 5749 / 5670 / 5758 | 1677 / 1677 / 1653 |  | 146 / 152 / 216 | 381.0 / 382.4 / 385.5 | 4m 21s / 4m 24s / 4m 22s |
| benchy | 3453 / 3867 / 3445 | 460 / 517 / 517 | 21347 / 20801 / 21423 | 16244 / 16233 / 16181 | 1599 / 1611 / 1608 | 3480 / 3630 / 3490 | 3767.6 / 3773.3 / 3772.1 | 38m 43s / 40m 50s / 40m 26s |
| mc1 | 8965 / 9025 / 9016 | 8776 / 8773 / 8773 | 50507 / 48846 / 48872 | 5623 / 6891 / 6891 | 220 / 362 / 354 | 3020 / 3586 / 3586 | 6886.5 / 6814.2 / 6814.4 | 1h 5m 1s / 1h 2m 54s / 1h 2m 54s |
| mc2 | 4450 / 4507 / 4507 | 4389 / 4415 / 4415 | 24290 / 24524 / 24525 | 2721 / 2718 / 2718 | 190 / 191 / 190 | 2598 / 2657 / 2657 | 4462.7 / 4463.3 / 4463.3 | 42m 32s / 41m 18s / 41m 18s |
| owner's figurine, m | 0.48 / 0.39 / 0.46 | 2.88 / 2.88 / 2.88 | 69.35 / 26.95 / 69.33 | 111.24 / 114.16 / 111.32 | 1.06 / 0.97 / 1.09 | 0.67 / 1.10 / 0.72 | | |

What moved, in the order it was found:

- A top surface was grown into the fill beside it by its whole reach at
  once, and so jumped the inner wall between a strip of top under
  `only_one_wall_top` and the solid fill inside that wall: on every
  layer of a slope a second strip of top was laid over the inner wall
  (30 mm a layer on the wedge, 17% of its top). Tops, bottoms and
  bridges now grow in the reference's waves, within each kind of fill
  taken whole.
- Bambu Studio's `enabled` for `ensure_vertical_shell_thickness` was
  taken for a value not ported, and the figurine got no vertical shells.
- With walls kept thick the faces a shell is made from are grown by a
  twentieth of a line first; without it the faces of a slope's layers,
  a hair apart, were opened away as strips, and the solid under a
  sloped top began a layer higher (3DBenchy's deck at z 14.05).
- A face that is its own whole shell is held at its edges.
- Each front of a wave is thinned of points within a tenth of a micron
  of the line through their neighbours before it grows again. The
  clipping library here leaves the two ends of a corner that hardly
  turns a nanometre apart where the reference's leaves one point, and
  each wave rounded those scraps again: the Benchy's second layer came
  to 61,000 points and the whole Benchy took 11 s instead of 0.6 s.
  Lengths move by under a millimetre (the Benchy's internal solid
  21422 to 21423 mm, the dome's 5913 to 5914). A test in
  `edge-infill` fails if a round bridge grown in waves comes back
  heavier than its curve; `edge slice --timings` shows the stages.

The wedge's contoured top (`zaa_enabled`) is now the reference's move
for move: 3783 top surface moves in both files where there were 4113
here, every one paired with the reference's at the same height.

What still differs. On 3DBenchy's roof, between z 37.6 and 39.3, the
bridges over infill fall on alternate layers from the reference's (the
totals over those layers agree within 1%). With `top_shell_layers` 1 the
reference lays the fill under the boss on the `holes` slab as an internal
bridge (220 mm) and this engine as sparse fill. With `zaa_enabled` on the
dome, the layers being sliced 0.05 mm off, a band of internal bridge falls
a layer lower than the reference's (216 mm against 146), where without
contouring it falls on the same layer. On lettered plate 1, whose walls
under the lettering are not the reference's (below), internal solid is
3% short and sparse fill 23% over on the layers under the letters.

### Auto orient

`edge orient <model> --command-line` measures each object as the
reference's `--orient 1` does and prints its lines ("orientation: …, cost:
…", "best: …"), to set beside the reference's own. On 3DBenchy, the seven handy models, the 16
turned and mirrored arms of `mcbrim_test.3mf` and eight shapes made for it
(a tilted box, an L standing up and lying, a T on its post, a mushroom
upright and tilted, a wedge, a cube on its edge) the directions tried are
the reference's, and every cost but one is its own to the bit (checked
against the reference's expressions built with its Eigen); the one is the
area of the convex hull on the bed, whose triangles are this engine's hull
and not qhull's, which moves it by up to a few mm² on some directions.
The direction chosen is the reference's for every object but those where
two directions cost exactly the same (a T on either side, a box on its top
or its bottom), where the reference's pick is the order of its hash map and
this engine takes the first found. Benchy is oriented in about 18 ms here
(56 ms on one thread); the reference's command line takes about 150 ms.

## Print time against the reference

2026-11-11, U1 presets unless said. Each file is timed here by
`edge-time` the way the reference's estimator plans (the reference's
own file comes to its own time that way), and the difference is put
down to where it is spent: `cargo run --release -p edge-time --example
lines -- file.gcode` gives the seconds of every line, which are then
summed by layer and by kind (extruding by feature, travel, wipe,
retraction, the step down after a lift, tool changes). Seconds,
reference / before / after, and retractions in brackets:

| Part | Reference | Before | After |
| --- | --- | --- | --- |
| Benchy | 2322.8 (1519) | 2432.6, +4.73% (1709) | 2381.2, +2.51% (1524) |
| cube | 899.5 (199) | 908.8, +1.03% | 907.7, +0.90% |
| cube, Centauri Carbon | 591.3 (200) | 591.8, +0.09% | 593.6, +0.39% |
| `lean` | 2909.4 (120) | 2945.4, +1.24% | 2903.0, -0.22% |
| `twist` | 1973.8 (106) | 2012.6, +1.97% (102) | 1974.5, +0.04% (102) |
| `tower` (6 mm square, 80 mm tall) | 1478.4 (800) | 1487.5, +0.62% | 1485.4, +0.47% |
| `mcbrim_test.3mf` plate 1 | 3901.4 (2188) | 3689.6, -5.43% (1976) | 3615.4, -7.33% (1843) |

What moved it, largest first:

- Gap fill between walls was laid as a stage of its own before the
  fill, the nozzle going from one gap to the next across the part. In
  the reference each line of it is a fill area of its own, chained
  with the others (Fill.cpp `make_fills`, `thin_fills`): the Benchy's
  retractions 1709 to 1524 against 1520, 28 s.
- Travel acceleration. The reference sets it for every travel, short
  hops too, and before the retraction, so the wipe goes at it; only a
  short travel into an outer or overhang wall keeps that wall's
  (GCode.cpp `travel_to`). Here short hops kept whatever was set: the
  Benchy's hops between top surface lines went at 2000 mm/s², 31 s of
  them. 22 s on the Benchy, 14 s on `twist`.
- What the hot end can melt is worked out with the flow ratios
  (GCode.cpp `_extrude`, `_mm3_per_mm`), and a measured wall's own
  speed is written as it is, not to the whole mm/s: the outer wall at
  F9742 against F9540, and on `lean` every feature's feed rate the
  reference's. 38 s on `lean`, 24 s on `twist`, little on the Benchy
  and the cube, most of whose layers are slowed to cool.
- A layer slowed to cool leaves the prime tower at its speed, as the
  reference does (none of the parts above slows a layer with a tower).

What is left:

- Cooling. The reference counts a slowed layer as longer than it is.
  CoolingBuffer.cpp `parse_layer_gcode` copies the last position,
  extruder included, before it zeroes the extruder for relative E, so
  a line that sets only the feed rate (the `G1 F` that starts every
  path) is read as the previous line's extrusion again: after a
  deretraction, a 1.5 mm move at the path's speed. The layers it
  slows come out short of `slow_down_layer_time`, the cube's 7.91 s of
  8 (its counting, done over in a script, gives 8.009 with the extra
  moves, 7.912 without), the Benchy's upper layers about 7.4 s. Not
  copied: here a slowed layer takes `slow_down_layer_time`. On the
  Benchy that is 51 s of the reference's layer time at the speeds
  written, which is most of what is left on it (its walls, gap fill
  and sparse fill are 42 s longer here), and all of the cube's 8 s.
- The Benchy's surfaces: 3445 mm of top surface against 3453 and
  3490 mm of internal bridge against 3480 (3867 and 3630 before the
  top surfaces were made the reference's, under Top surfaces on slopes
  above, which took 24 s off its time); what is left is its roof from
  layer 184 to 198, where bridges over infill fall on other layers;
  and on layer 212 a run of short open inner walls laid end to end in
  the reference and back and forth here (3 s).
- The parts of `mcbrim_test.3mf` are lettering whose walls are not the
  reference's, so its time is no measure of this.
- The time written in this engine's file for a Klipper printer is
  Klipper's plan of it (above, under Print time), which is longer than
  the reference's estimate of the same file: the Benchy is 39 m 41 s
  as the reference plans it and 40 m 50 s as Klipper does (the
  reference's file 38 m 43 s and 39 m 51 s), so its file says 40 m 50 s
  where the reference's says 38 m 43 s.

## Licence

AGPL-3.0, the same as EdgeSlicer, so algorithms may be ported from it.
`vendor/cpp_map` is another's work under MIT or Apache-2.0, which this
licence can take in; both texts are kept beside it.

The desktop app's translations (`apps/desktop/src/api/i18n/*.json`) are
made by `tools/make_translations.py` from the reference's catalogs
(`reference/EdgeSlicer/localization/i18n/<language>/*.po`), the work of
EdgeSlicer's, OrcaSlicer's, Bambu Studio's and PrusaSlicer's translators.
The catalogs carry no licence of their own: their headers say they are
"distributed under the same license as the PACKAGE package", which is
EdgeSlicer's AGPL-3.0 (`reference/EdgeSlicer/LICENSE.txt`), this
repository's own.
