# UI parity with the C++ EdgeSlicer

What the C++ application offers in its main tabs and its project strip, what
this app had, and what it has now. It is the map later passes work from: the
C++ side is listed whole, including what is not built here.

The C++ source read is the pinned `reference/EdgeSlicer` (156e9591fe, the
merge of PR 920 "feat/multi-project-strip"). Paths are under
`src/slic3r/GUI/` unless they start with `libslic3r/`, `libvgcode/` or
`Utils/`. Strings in "quotes" are the C++ application's own English.

The last column reads:

- **had** — the app did this before the pass.
- **added** — built in this pass (first pass: Prepare, Preview, Device, the
  project strip).
- **differs** — built, but on purpose not as the C++ does it; the cell says how.
- **not yet** — not built, with what it waits for. Nothing in this column is
  shown in the page as a control that does nothing.

Everything the page asks for goes through `Session::call` (`src/api.rs`). The
calls are listed at the end.

## 1. The tab bar

| C++ | Where | Here |
| --- | --- | --- |
| A notebook along the top: Home (icon only), "Prepare", "Preview", "Showcase", "Device", "Stream", "Printers", "Project", "Calibration" (and "Layerling" when installed). Order fixed in `MainTabs::Position` | `MainFrame::init_tabpanel`, `MainFrame::update_layout`, `MainTabs.hpp` | **added**: "Prepare", "Preview", "Device", with the C++ tab icons (`resources/images/tab_3d_active.svg`, `tab_preview_active.svg`, `tab_monitor_active.svg`). **not yet**: Home, Showcase, Stream, Printers, Project, Calibration |
| Prepare, Preview and Showcase are one panel (the plater) added three times: the sidebar stays, the canvas changes | `MainFrame::update_layout` (`InsertPage` ×3), `MainTabs::hosts_plater` | **added**: Prepare and Preview share the sidebar and the stage |
| The page opened at startup is Home or Prepare ("Default Page" preference, `default_page` 0 or 1) | `GUI_App.cpp` l.1628, `Preferences.cpp` l.1995 | **differs**: Prepare, there being no Home. **not yet**: the preference |
| Selecting Preview slices the plate by itself when it holds printable objects and has no valid result; a plate that is slicing is shown as it is; an empty plate shows an empty bed with no message | `Plater::priv::set_current_panel` → `do_reslice` (Plater.cpp l.18070) | **added**: the same, on going to Preview and on picking another plate there. A plate whose slice failed is not sliced again by itself; its error is shown |
| The slice button always ends in Preview, cancelled or not | `Plater::priv::on_action_slice_plate` l.19328, `on_action_slice_all` l.19361 | **added** |
| Tab on the canvas switches between Prepare and Preview | `GLCanvas3D::on_key` l.5729 (`EVT_GLCANVAS_TAB`), KBShortcutsDialog "Switch between Prepare/Preview" | **added**: while the stage has the keys (it was clicked); elsewhere Tab moves focus as usual |
| Leaving Prepare disables undo and redo in the title bar | `init_tabpanel` page-changed handler | **not yet**: there is nothing to undo |
| In a G-code-only or exported-file session Prepare is refused with a warning | `MainFrame::preview_only_hint` | **not yet**: G-code files are not opened |
| Entering and leaving Device tells the device web view "active" / "inactive"; entering Home refreshes it | `init_tabpanel` page-changed handler | **not yet**: no web view |
| Right of the tabs: the slice button with its menu, and the print button with its menu | `MainFrame::create_side_tools` l.2482 | **added**: see section 2, "Slice and export" |
| Ctrl+1..4 for Home, Prepare, Preview, Device | `MainTabs::tab_for_field_shortcut` (no longer bound in the C++ either) | not built, as in the C++ |
| What each tab keeps: the project's own view (Prepare or Preview) is remembered per project; Home, Device and the rest are the window's | `ProjectSlot::main_tab`, `ProjectStrip::after_switch` | **added**: `tab` is kept with each project in the session; Device is the window's |

## 2. Prepare

### Sidebar: Printer

| C++ | Where | Here |
| --- | --- | --- |
| Section "Printer", folded and unfolded by its title ("Click to collapse / expand") | `Sidebar::Sidebar` l.2385, `toggle_printer_section` l.2614 | **added** |
| One printer preset list over every installed vendor, grouped "Project-inside presets", "User presets", "System presets", "Incompatible presets", with a thumbnail of the printer | `PlaterPresetComboBox(TYPE_PRINTER)`, PresetComboBoxes.cpp l.1684 | **differs**: "Vendor" then "Printer", from the vendor profiles (**had**). A project keeps "Sliced with: the project's own settings / presets chosen here" (**had**), this engine's way of saying whether the file's settings or presets are used; the C++ has no such choice because opening a project selects its presets. **not yet**: user presets, installed-vendor selection ("Select/Remove printers (system presets)", "Create printer"), thumbnails |
| "Bed type" list: the six plates, or a Snapmaker U1's own ("Textured PEI Plate", "Smooth PEI Plate", "Graphic Effect Plate", "Cool Plate"), or a U1's seven with `support_multi_bed_types`; enabled only for Bambu printers, a U1, or `support_multi_bed_types`, otherwise the preset's default, greyed | `Sidebar::update_presets` l.4464, `libslic3r/PrintConfig.cpp` l.1284 | **added**: the same lists, labels and enabled rule, from `settled` (`bed_types`). Before: a fixed list of six with "as the project or the printer has it". **differs**: the plate shown when none was chosen is the one in force (the project's, else the printer's default); the C++ remembers the last plate per printer in its app config. **not yet**: "Graphic Effect Plate" (the engine has no bed temperature for it), the remembered plate, the label's link to the wiki |
| Changing the bed type marks the project modified and drops every plate's slice | `Plater::priv::on_select_bed_type` l.18465 | **added** (`project_set`) |
| Nozzle rows: "Diameter", "Flow" ("Standard" / "High Flow"), "Rack"; several nozzles shown as tiles ("Nozzles", "Settings ›") | `update_nozzle_settings` l.11675, `Widgets/NozzleStrip.cpp` | **added**, read only: nozzle size and build volume under the bed type. **not yet**: choosing them (a preset per nozzle is picked in "Printer") |
| "Synchronize nozzle information", "Connect to printer" (opens "Devices"), the working-mode light, the settings gear (the setup wizard), the edit cog ("Click to edit preset") | l.2408, l.2587, l.2665, PresetComboBoxes.cpp l.1150 | **not yet**: needs a printer connection (section 4) and a settings editor |

### Sidebar: Filament

| C++ | Where | Here |
| --- | --- | --- |
| Section "Filament Management" ("Pellet Configuration" for pellet printers), sub-title "Filaments" | l.2882, l.3515, l.4450 | **added** |
| One row for each filament of the project: a numbered colour swatch ("Click to select filament color"), the filament preset list, the tool badge, the row menu ("Edit", "Delete", "Merge with", "Print on nozzle", "Rack slot"…) | `init_filament_combo` l.4135, `GUI_Factories.cpp::create_filament_action_menu` l.2600 | **added**: a row for filament 1 up to the highest the file prints in, with its number on the project's colour and a preset list each (before: one list for each filament the plate shown used, no colours). With the project's own settings the saved names are shown. **not yet**: changing a colour, the tool badge, the row menu |
| "Add one filament", "Remove last filament" | l.3508–3600 | **not yet**: the engine prints the filaments the file assigns |
| "Apply All" (one preset for every filament) | l.3624 | **not yet** |
| "Flushing volumes", "Color Mixing Match", "Synchronize filament list from AMS", "Sync Filament Information", "Spool Manager", "Manage filament presets" | l.2903, l.2936, l.3404, l.3508 | **not yet** |
| "Mixed Filaments": "Add Gradient", "Add Pattern", "Add Color" | l.3794 | **not yet** |

### Sidebar: Process

| C++ | Where | Here |
| --- | --- | --- |
| Section "Process" with the process preset list | `ParamsPanel::ParamsPanel` l.210, `Tab.cpp` l.300 | **added** as its own section (before: one list among the presets) |
| Pages "Quality", "Strength", "Speed", "Support", "Multimaterial", "Others", each a page of settings | `TabPrint::build`, Tab.cpp l.3441 | **added**: the four settings the engine takes over a preset, under the C++ names and pages — Quality: "Layer height", "Initial layer height", "Slice gap closing radius"; Strength: "Wall loops". Empty is "as the preset has it", whose value is shown in grey. A change marks the project modified and drops its slices, as in the C++. **not yet**: every other setting (the engine reads them from the preset; there is no editor) |
| "Global" / "Objects" switch, per-object, per-part and per-layer settings, "View all object's settings" | `ParamsPanel::create_layout` l.364, `TabPrintModel` l.5035 | **not yet** |
| "Advance" switch, search ("Search in preset"), "Compare presets", save and delete preset, favourites, "Manage process presets" | ParamsPanel.cpp l.210–257, Tab.cpp l.300–420 | **not yet** |

### The bed

| C++ | Where | Here |
| --- | --- | --- |
| A 3D view of every plate with the objects on it, orbit / pan / zoom, selection | `View3D`, `GLCanvas3D` | **added**, from above only: the bed of the printer chosen with its grid and each object's outline (its convex hull), name and size, drag to move, wheel to zoom, "Fit part" [Z] and "Fit bed". A bare model is shown in the middle, where slicing puts it. Before: nothing was shown until a model was sliced. **not yet**: the third dimension, selection, the other plates in the same view |
| Main toolbar: "Add" [Ctrl+I], "Add plate", "Auto orient all/selected objects" [Q], "Arrange all objects" [A] with its options, "Split to objects", "Split to parts", "Variable layer height", "Assembly View" | `GLCanvas3D::_init_main_toolbar` l.9274 | **not yet**: the engine does not move, add or split anything; a file is sliced as it was saved |
| Gizmos: "Move" [M], "Rotate" [R], "Scale" [S], "Lay on face" [F], "Cut" [C], "Mesh Boolean" [B], "Support Painting" [L], "Seam painting" [P], "Paint-on fuzzy skin" [H], "Color Painting" [N], "Texture displacement", "Emboss" [T], "SVG", "Measure" [U], "Assemble" [Y], "Simplify", "Brim Ears" [E], "Sculpt", "Edit", "Fillet / chamfer (CAD)" | `Gizmos/GLGizmosManager.cpp::init` l.224 | **not yet** |
| Object list: plates, objects, parts, instances, layer ranges; columns "Name", printable, "Fila.", paint marks; the object, part and plate menus | `GUI_ObjectList.cpp`, `GUI_Factories.cpp` | **not yet** as a list. The names of the objects on the plate are on the bed and in `project` (`plates[].objects`) |
| View options: "Realistic View", "X-Ray", "Center of Mass", "Show 3D Navigator", Ctrl+0..6 views | `GLCanvas3D.cpp` l.8297, View menu | **not yet** |

### Plates

| C++ | Where | Here |
| --- | --- | --- |
| Every plate shown on the canvas; a click selects it | `PartPlateList`, `Plater::select_plate_by_hover_id` l.30481 | **added**: a column of plate numbers (two plates or more) that selects the plate, in Prepare and in Preview. Before: a "Plate" list in the sidebar |
| Per plate: delete, orient, arrange, lock, "Customize current plate" (bed type, print sequence, spiral vase, filament sequences), name, "Move plate to the front", "Print history" | `PartPlate::render_icons` l.1173, `PlateSettingsDialog.cpp` | **not yet**. A plate's name is shown in its tooltip; its first-layer filament order is followed by the engine |
| "Add plate", "Duplicate Current Plate", the plate menu ("Select All", "Delete All", "Arrange", "Reload All", "Auto Rotate", "Delete Plate") | toolbar, `create_plate_menu` l.2831 | **not yet** |

### Slice and export

| C++ | Where | Here |
| --- | --- | --- |
| "Slice plate", with a menu: "Slice all", "Slice plate" (and "Filament arrangement..." on dual-nozzle Bambu printers); the button keeps the mode picked | `create_side_tools` l.2482 | **added**: the button and its menu, in the tab bar. Before: one "Slice" button in the sidebar. "Slice all" slices every plate that holds something and is not sliced yet, one after another (`slice` with `all`) |
| "Slice plate" is off while slicing, when the plate's result is valid, or when the plate cannot be sliced; "Slice all" when no plate can be | `MainFrame::get_enable_slice_status` l.2672 | **added**: the same three rules. **not yet**: the blocks for mixed filament temperatures, a cold plate and zero flow ratio |
| Progress as a notification on the canvas with the stage's text, a percentage and a cancel; "Slice ok." when done, "Slicing Canceled" when called off | `SlicingProgressNotification.cpp`, Plater.cpp l.18718, l.19108 | **added**: a note on the stage with the engine's stage, a bar, "Cancel", then "Slice ok." or "Slicing Canceled" (before: a bar in the sidebar). Cancel takes effect between stages, as before |
| The print button and its menu: "Print plate", "Print all", "Print" (send to a print host), "Send", "Send all", "Export plate sliced file" [Ctrl+G], "Export all sliced file", "Export G-code file", "Send to Multi-device", "Send to Bambu Connect"; the last one used is remembered | `print_select_type_label` l.2378, `available_print_actions` l.2397 | **added**: "Export G-code file" for the plate shown (**had** as "Export G-code…"), on when the plate is sliced and its G-code was written. **not yet**: the sliced-file (3MF) exports, and everything that sends to a printer (section 4) |
| Ctrl+R slices the plate | `MainFrame` char hook l.771 | **added** |
| Warnings and errors of a slice as notifications | `NotificationManager` | **had**, moved to Preview: "About this print" on the stage keeps the three kinds apart (what is wrong with this print; what was settled without being asked; the settings this engine does not follow yet) |

### Files

| C++ | Where | Here |
| --- | --- | --- |
| "New Project" [Ctrl+N], "Open Project..." [Ctrl+O], "Recent projects" | `MainFrame::init_menubar_as_editor` l.3385 | **added** through the strip (section 5). **not yet**: a menu bar, recent projects |
| "Save Project" [Ctrl+S], "Save Project as..." [Ctrl+Shift+S] | the same | **not yet**: the engine does not write projects |
| "Import 3MF/STL/STEP/SVG/OBJ/AMF/GLB..." [Ctrl+I] adds geometry to the project shown; dropping files on the window; "Open as project" / "Import geometry only" | l.3385, `PlaterDropTarget::OnDropFiles` l.13122 | **differs**: a file is opened as a project of its own, an STL as much as a 3MF (**had**: STL and 3MF). **not yet**: adding to a project, the other formats, dropping files |
| Export: STL, STEP, generic and Bambu 3MF, preset bundles | the same | **not yet** |

## 3. Preview

| C++ | Where | Here |
| --- | --- | --- |
| The sliced toolpaths in 3D over transparent shells of the objects | `Preview::load_print_as_fff` (GUI_Preview.cpp l.659), `GCodeViewer` | **had**, in 2D: one layer from above, drawn from the engine's own lines (`layer`). Until a plate is sliced its outlines are shown faintly. **not yet**: 3D, shells |
| View type list, in this order: "Line Type", "Filament", "Speed", "Actual Speed", "Acceleration", "Jerk", "Layer Height", "Line Width", "Flow", "Actual Flow", "Layer Time", "Layer Time (log)", "Fan Speed", "Temperature", "Pressure Advance" | `GCodeViewer::update_by_mode` l.1272, `get_view_type_string` l.106 | **added**: a list of the four the engine has the data for — "Line Type" and "Filament" (**had**, as "Colour by feature / filament"), "Layer Height" and "Line Width" (new), the last two coloured over the C++ range colours (`libvgcode/include/ColorRange.hpp`). **not yet**: the views that need the speed, flow, fan, temperature or time of each move, which the engine does not hand out |
| The view type a print opens in: "Automatic" (Filament with several filaments, else Line Type) or "Last used" ("Default preview view type") | `apply_default_view_type` l.1248, Preferences.cpp l.2038 | **differs**: the last one used in that project, kept with it in the session (`view`); a new project starts in "Line Type". **not yet**: the preference |
| Legend, top right: fold button, G-code window button, the list | `render_legend` l.4120 | **added**: fold button and list. **not yet**: the G-code window |
| "Line Type" rows in the order of the roles, only those in the print, each with the C++ colour; columns "Line Type", "Time", "%", "Usage", "Display"; a click on a row hides or shows it | l.4300–4645, `libvgcode/src/ViewerImpl.cpp` l.302 | **added**: the C++ order and colours (before: this app's own), a "Display" mark on each row, a click on the row toggles it (**had** as checkboxes). **not yet**: "Time", "%" and "Usage" for each feature; the roles the engine never makes ("Ironing", "Custom", "Support transition", "Wall over support", "Bottom surface over support") |
| "Supported overhangs" / "Unsupported overhangs" overlays | l.4600 | **not yet** |
| Option rows: "Travel", "Wipe", "Retract", "Unretract", "Seams", "Filament changes" | l.4624, `append_option_item` l.4497 | **not yet**: the lines the engine hands out for drawing leave out moves, retractions and seams |
| "Filament" view: a row for each filament used with "Model", "Support", "Flushed", "Tower", "Total"; "Filament change times" | l.4417–4864 | **added**: a row for each filament with its colour, to hide or show (**had**), and "Filament change times:". **not yet**: the split of each filament's use |
| "Total estimation": "Total Filament:", "Model Filament:", "Cost:", "Prepare time:", "Model printing time:", "Total time:", "Show stealth mode" | l.5277–5390 | **added** in the legend: "Total Filament:" (m and g) and "Total time:", with how the time was worked out (**had** in the sidebar, as "Filament", "Time", "Travel"). **not yet**: model filament apart from support and tower, cost, prepare time, stealth mode |
| Vertical layer slider at the right: two thumbs (a range of layers), the layer's number over its height beside the thumb, hover shows a layer's time | `IMSlider.cpp` l.890–1230, `get_label` l.1723 | **differs**: a vertical slider with one thumb and the same label; it is always the C++ "one layer mode" [L], a view from above showing one layer. Before: a horizontal slider. **not yet**: the lower thumb, layer times |
| Up / Down move a layer; five with Shift or Ctrl; the mouse wheel over the slider likewise | `GLCanvas3D` l.5866, `IMSlider.cpp` l.1636 | **added** (before: arrows by one, PageUp / PageDown by ten; those two keys now go round the projects with Ctrl, as in the C++) |
| "Jump to Layer" (the slider's menu; Shift+G): "Please enter the layer number (min - max):" | `IMSlider::render_add_menu` l.1512 | **added**: the slider's menu and Shift+G |
| The slider's other menu entries: "Add Pause", "Add Custom G-code", "Add Custom Template", "Change Filament"; ticks with their delete and edit | l.1512, `render_edit_menu` l.1569 | **not yet**: the engine has no per-layer custom G-code |
| "Compare slices (A/B)" | IMSlider.cpp l.1300 | **not yet** |
| Horizontal move slider, the tool marker, the position window with its properties | `update_moves_slider` l.2331, GCodeViewer.cpp l.393–760 | **not yet**: needs the order of moves within a layer |
| G-code window [C] | `GCodeWindow::render` l.877 | **not yet** |
| "Select Plate" bar at the top left: a tile for each plate with its state (not sliced, slicing, sliced, failed); a click selects the plate and slices it if it is not sliced | `GLCanvas3D::_render_imgui_select_plate_toolbar` l.11656 | **added**: the same column of plates as in Prepare, with a tick for sliced, "…" for slicing and "!" for failed; a click selects and, in Preview, slices |
| Its first tile: "Slice all" with sliced/total while some are not sliced, then "All Plates" / "Stats", which opens "Statistics of All Plates" (each filament's use, "Total time:", "Total cost") | l.11656–11730, `render_all_plates_stats` l.3295 | **added**: the tile in both states; the statistics list each plate's filament and time and their totals. **not yet**: by filament, cost |
| Canvas warnings: paths beyond the plate, beyond the height, conflicting paths | GLCanvas3D.cpp l.12998 | **differs**: the engine refuses to write G-code with toolpaths off the bed; the reason is shown under "About this print" |
| "Auto slice after changes", "Dim lower layers in the G-code preview" | Preferences.cpp l.2135, l.2046 | **not yet** |
| "Export Toolpaths as OBJ" | MainFrame.cpp l.4417 | **not yet** |

## 4. Device

| C++ | Where | Here |
| --- | --- | --- |
| The tab "Device" holds one of three panels, by the printer preset selected: Bambu's monitor (`MonitorPanel`: "Status", "Storage", "Update", "HMS") for a Bambu printer on Bambu's network; Flashforge's own pages; for every other printer a web view (`PrinterWebView`) | `MainFrame::show_device` l.1906, `Sidebar::update_all_preset_comboboxes` (Plater.cpp l.4576), `libslic3r/PresetBundle.cpp::use_bbl_device_tab` l.723 | **added**: the tab follows the printer of the project shown (its preset, or the printer the project was saved with) |
| The web view loads the printer's own page: `print_host_webui` if the preset has it, else `print_host`, with `http://` put in front; a Snapmaker U1 gets Snapmaker's bundled device page, driven over MQTT or a websocket; a device picked from the per-model "Devices" list comes before the preset's address | Plater.cpp l.4304–4430, `MainFrame::load_printer_url` l.5286, `Utils/PrintHostDevices` | **not yet**: see "What a real Device tab needs". **added**: the address is shown as text when the preset carries one ("Hostname, IP or URL", "Device UI") |
| With no address: a page "Printer Connection" — "Please set up your printer connection to view the device." | `resources/web/orca/missing_connection.html` | **added**: the same words, and that an address cannot be set here yet |
| The API key is put into every request of the page (`X-API-Key`) | `PrinterWebView::SendAPIKey` l.473 | **not yet**. The page is told only whether the preset has a key, never the key |
| Bar over the page: "Printer page" / "Storage" (the G-code files on the printer: "Name", "Print time", "Filament", "Size", "Modified"; "Print", "Delete"), and "Printer:" to pick among several printers of the model | `PrinterWebView.cpp`, `Utils/PrinterStorage.cpp` l.337 | **not yet** |
| What the tab shows of the printer itself | the preset | **added**: name, vendor, model, nozzles, bed and height, G-code flavor, bed type in force, "Host Type" |
| Where the address is set: "Physical Printer" ("Print Host upload": "Host Type", "Hostname, IP or URL", "Device UI", "Authorization Type", "API Key / Password", "HTTPS CA File", "User", "Password", "Test", "Browse ..."), and the "Devices" list for several printers of one model | `PhysicalPrinterDialog.cpp` l.127, `PrintHostDevicesDialog.cpp` | **not yet**: needs user presets, or a list of printers of this app's own |
| Where it is kept: the user's printer preset, and `<datadir>/hub/print_host_devices.json`; never the project (a saved 3MF has `print_host`, `print_host_webui`, `printhost_apikey` and the rest taken out, and they are ignored on load) | `libslic3r/Preset.cpp` l.1394, l.2710, `PresetBundle::full_config_secure` l.3220 | Read from the preset chosen only. Forty-seven vendor presets carry an address (for example Volumic's `192.168.0.60`), which is why the tab can show one today |
| Sending: "Send G-code to printer host" ("Upload", "Upload and Print", the file name, "Switch to Device tab after upload."), and "Print host upload queue" | `PrintHostDialogs.cpp` l.515, l.2358 | **not yet** |
| Bambu monitor: camera, "Printing Progress", "Pause" / "Resume" / "Stop", temperatures, AMS, speed; needs Bambu's network plug-in | `Monitor.cpp`, `StatusPanel` | **not yet**, and not planned before the print hosts |
| Next to Device: "Stream" (cameras), "Printers" (every printer at a glance), the mobile hub | `StreamPanel`, `PrintersPanel`, `RemoteHub.cpp` | **not yet** |

### What a real Device tab needs

Nothing below is built. The page loads nothing from the network and the engine
has no printer connection; this is what the C++ does over the network for the
printers this project is tested on, so the next pass can be decided.

**Requests, all to the printer's own address on the local network** (the host
of `print_host`; `X-Api-Key: <printhost_apikey>` on each when a key is set):

| For | Klipper through Moonraker (`Utils/MoonRaker.cpp`, `Utils/PrinterStorage.cpp`, `Utils/PrintHostDeviceStatus.cpp`, `RemoteControl.cpp`) | OctoPrint's interface, which is what a preset with "Host Type" `octoprint` ("Octo/Klipper") uses, Moonraker answering it too (`Utils/OctoPrint.cpp`) |
| --- | --- | --- |
| Is it there | `GET /printer/info` | `GET /api/version` (JSON with `api`; `text` starting "OctoPrint" when given) |
| Send a file | `POST /server/files/upload`, multipart: `file`, `path`, `print` = true / false | `POST /api/files/local`, multipart: `file`, `path`, `print` = true / false |
| Start it | `print=true` on the upload, or `POST /printer/print/start?filename=` | `print=true` on the upload |
| What it is doing | `GET /printer/objects/query?print_stats&...` (state, file, progress, temperatures); live over `ws://<host>/websocket` (JSON-RPC) | not used by the C++ beyond the printer's own page |
| Files on it | `GET /server/files/list?root=gcodes`, `GET /server/files/metadata?filename=`, `DELETE /server/files/gcodes/<name>` | — |
| Pause, resume, stop | `POST /printer/print/pause`, `/resume`, `/cancel`; `POST /printer/gcode/script` | — |

A Snapmaker U1 is reached the same way on port 80 by the C++ "LAN" route
(`SnapmakerLan.hpp`: `/printer/info`, `/server/files/upload`,
`/printer/print/start?filename=`), and otherwise by Moonraker's JSON-RPC over
MQTT (ports 1884 and 8883, with pairing). Finding printers is mDNS
(`_snapmaker._tcp.local`, UDP 5353). The other fifteen host types
(`Utils/PrintHost.cpp::make_print_host`) each have a class of their own.

**Three ways to do it, given "the page loads nothing from the network":**

1. **The session talks to the printer; the page still talks only to the
   session.** New calls behind `Session::call` (`printer_test`, `printer_send`,
   `printer_status`, `printer_files`) make the requests above from Rust, in a
   crate of their own beside the engine, and the Device tab draws what comes
   back. The rule stands as written, the content security policy does not
   change, `--serve` and the tests share it, and a test can stand a fake
   Moonraker on a loopback port. It needs an HTTP client in the app (and a
   websocket one for live status), and a place to keep addresses, since vendor
   presets rarely have one: a list of this app's own, as the C++ keeps
   `print_host_devices.json`. **Recommended.**
2. **A window of its own for the printer's page.** The app opens a second web
   view at the printer's address, apart from the app's page: what the C++ web
   view shows, with no monitoring code to write. The app's page still loads
   nothing, but the application then shows a page it does not control, it does
   nothing for `--serve`, and sending a file still needs option 1.
3. **The printer's page inside the app's page** (a frame, with the policy
   opened to the printer's address). This breaks the rule itself, and a
   printer's page is not written to be framed. Not recommended.

Option 1 first (test, send, status, files), then option 2 as a convenience for
the printer's own page if it is still wanted.

## 5. The project strip

Read from `ProjectStrip.cpp/.hpp`, `ProjectSessions.hpp` and their uses in
`MainFrame.cpp`, `Plater.cpp`, `GUI_App.cpp` and `Preferences.cpp`.

| C++ | Where | Here |
| --- | --- | --- |
| A column at the far left of the window, beside every tab, always shown, listing the open projects | `MainFrame::update_layout` l.1123, ProjectStrip.hpp (D1, D9) | **added**. Before: one model at a time |
| Each project is live: its own plater, with its model, plate, presets, slice, Preview, selection, camera and undo history. A switch saves and loads nothing; a slice running in a project out of sight goes on | ProjectStrip.hpp (P4), `switch_to`, `stash_active`, `show_slot` | **added**: `Session` holds every open project with its file, plate, choices of presets and settings, the slice of each plate, the tab it was on and where its Preview was left (`view`). Each slice runs on a thread of its own, so another project is looked at, or sliced, meanwhile. The page keeps each project's camera. **not yet**: selection and undo, there being none |
| The window starts with one project, "Untitled" | `ProjectStrip::ProjectStrip` | **added** |
| "New project" (File menu, Ctrl+N, the strip's "+"): a new "Untitled" right after the one shown, then "Untitled 2", "Untitled 3" (the lowest number free), on the printer of the project it was started from; a project that is itself empty and untouched is simply started again; lands in Prepare | `new_project`, `ProjectSessions::add`, `next_untitled_name` | **added** (`project_new`) |
| "Open project" (File menu, Ctrl+O, the strip's folder button): a file open already is shown instead (two spellings of a path are one file); an empty untouched project takes the file itself; otherwise a new project right after the one shown; lands in Prepare | `open_project`, `find_by_origin`, `normalized_path` | **added** (`project_open`). An STL opens the same way, the C++ taking only project files there |
| A click shows a project. On Prepare or Preview the window goes to the tab that project was on; on Home or Device it stays | `select_project`, `after_switch` | **added** (`project_select`) |
| A switch is refused, with a notice, while a job or an open tool holds the project shown ("The project cannot be switched now.") | `can_leave_active`, `Plater::project_swap_blocker` | not needed: nothing here holds a project |
| Ctrl+PgDn / Ctrl+PgUp go to the next / previous project, round the ends; on macOS also Cmd+Shift+] / [ and the Window menu ("Next Project", "Previous Project", the projects by name) | `cycle`, `neighbour_id`, MainFrame.cpp l.799, l.4268 | **added**: the keys (`project_cycle`). **not yet**: the Window menu. In a browser (`--serve`) Ctrl+N, Ctrl+O and Ctrl+PgDn belong to the browser; in the app's window they are the page's |
| At most 8 projects (2 to 16, "Maximum open projects" under Preferences > Project): at the limit, "Too many open projects" asks which to close | `make_room`, `ProjectSessions::default_max_open` | **added**: 8, with the same question (`full` in the answer of `project_new` / `project_open`). **not yet**: the preference |
| Tiles: a thumbnail of the plate the project is on, rendered again a moment after it changes; an empty bed for an empty plate; a page with a folded corner until there is a picture | `render_active_thumbnail`, `on_paint`, `classify_tile_render` | **added**: the plate from above (bed and outlines), drawn again when the plate or the printer changes; the empty bed and the page likewise |
| The name under the thumbnail on at most two lines, broken only after a space, `_`, `-` or `.`, cut short with an ellipsis; the name is the file's without folder and extension | `wrap_name_two_lines`, `name_from_path` | **added**, the rule ported |
| The active project has an accent frame | `on_paint` | **added** (this app's accent, orange, not the C++ green) |
| An orange dot (top left) marks unsaved changes | `on_poll` (`is_project_dirty`), `on_paint` | **differs**: the dot marks a project whose choices (presets in use, bed type, the process settings set here) are no longer the ones it was opened with. That is all a project can lose here: nothing edits a model and nothing saves a project |
| A badge (bottom right): going round while the project slices, a tick once every plate that holds objects is sliced; also for projects out of sight | `slice_badge`, `on_poll` (every 500 ms) | **added**, from `projects`, asked for twice a second (more often while something slices) |
| Tooltip: the name, the file, "Unsaved changes", "Slicing" | `tooltip_for` | **added** |
| A cross (top right, on hover) and the middle mouse button close a project ("Close project") | `hit_test`, `on_left_down`, `on_middle_down` | **added** |
| Closing asks first: with unsaved changes, "\"%s\" has unsaved changes. Save them before closing it?" (Yes / No / Cancel); while slicing, "\"%s\" is slicing. Stop slicing and close it?" | `close_project` | **added** (`project_close`, which answers `ask` until told `discard` / `stop`). **differs**: the first offers "Don't save" and "Cancel" and says a project cannot be saved yet |
| Closing the project shown shows the next one, else the one before; closing one out of sight changes nothing on screen; closing the last one leaves a new empty "Untitled", so the strip always has one | `successor_on_close`, `close_project` | **added** |
| A tile dragged up or down moves to where the line shows | `on_motion`, `drop_index_at`, `ProjectSessions::move` | **added** (`project_move`) |
| More tiles than fit: the wheel scrolls the strip | `on_wheel` | **added** |
| Tile menu: "Save", "Save as...", "Send...", "Close", "Close others", "Reveal in Finder" / "Show in Explorer", "Open in new window" | `show_item_menu` | **added**: "Close", "Close others". **not yet**: saving, sending, showing the file, a second window |
| Strip menu: "Show thumbnails" / "Show names only", "Default width", "Collapse the project strip" / "Show the project strip" | `show_strip_menu` | **added** |
| The right edge is dragged to widen the strip: 88 to 200, names alone at least 120; remembered | `StripLayout`, `on_motion`, `project_strip_width` | **added**, remembered by the page |
| The chevron folds the strip to a 24 wide rail: New and Open stacked, a mark for each project (filled for the active one, a ring when sliced, the unsaved dot); remembered | `set_collapsed`, `on_paint` | **added** |
| Closing the window with several projects open: one dialog, "Unsaved projects", lists every unsaved one to save, not save, or cancel | `confirm_close_all`, MainFrame.cpp l.604 | **not yet**: the window closes at once. What is lost is what the dot marks |
| "Reopen the open projects at the next start" (off unless asked for): the projects written out at exit and read back | `save_session_for_restart`, `restore_session`, `SessionManifest` | **not yet**: needs writing projects. Reloading the page (`--serve`) does find every project as it was, the session being the server's |
| Project files opened from Explorer / Finder go into the strip ("Open project files from Explorer / Finder in": "New project tab" / "The current project") | `open_files_from_shell`, GUI_App.cpp l.9202 | **added** for a file named when the app is started (`startup`, **had**). **not yet**: a file handed to an app that is already running |

## 6. Shortcuts

| C++ | Here |
| --- | --- |
| Ctrl+N "New Project", Ctrl+O "Open Project" | **added** |
| Ctrl+PgDn / Ctrl+PgUp, Cmd+Shift+] / [: next / previous project | **added** |
| Ctrl+R "Slice plate" | **added** |
| Tab: "Switch between Prepare/Preview" | **added** (on the stage) |
| Up / Down: the layer slider; Shift or Ctrl with them: five at a time | **added** (Up / Down by one **had**) |
| Shift+G: "Jump to Layer" | **added** |
| Z: zoom to the objects | **added** ("Fit part") |
| Ctrl+S, Ctrl+Shift+S, Ctrl+I, Ctrl+G, Ctrl+Shift+G, Ctrl+P (Preferences), "?" (the list of shortcuts) | **not yet** |
| Left / Right, Home / End (the move slider), L (one layer mode), C (G-code window) | **not yet**; the view is always one layer |
| A, Shift+A, Q, Shift+Q, the gizmo keys, Ctrl+0..6, Ctrl+A / D / Z / Y / C / V / X, Del, 1–9, arrows to move objects | **not yet**: Prepare does not edit |

## 7. What the page asks for

Every call takes JSON and answers JSON; an error is a sentence to show. A call
about a project takes `project` (its id) and means the one shown when it is
left out; one about a plate takes `plate` and means the plate the project is
on.

| Call | Takes | Answers |
| --- | --- | --- |
| `startup` | — | `{ model }`: a file named when the app was started |
| `library`, `set_profiles { dir }` | | `{ dir, vendors }` |
| `machines { vendor }` | | `{ machines }` |
| `presets { vendor, machine }` | | `{ processes, filaments, usual_process, usual_filament }` |
| `projects` | — | the strip: `{ active, max_open, projects: [{ id, name, path, dirty, empty, plate, tab, slicing, plates_with_objects, plates_sliced }] }` |
| `project` | `project?` | a strip entry and `file`, `plates: [{ id, name, filaments, objects, sliced, failed }]`, `sliced`, `failed`, `has_settings`, `used`, `filament_count`, `colours`, `saved { printer, process, filaments }`, `choices { source, vendor, machine, process, filaments, bed_type, layer_height, first_layer_height, wall_loops, closing_radius }`, `view`, `status` |
| `project_new` | — | the strip and `id`; or the strip and `full: true` |
| `project_open` | `path` | the strip and `id` (`already: true` when it was open); or `full: true` |
| `project_select` | `id` | the strip |
| `project_cycle` | `step` (1 or -1) | the strip |
| `project_move` | `id`, `index` | the strip |
| `project_close` | `id`, `discard?`, `stop?` | the strip and `closed: true`; or `closed: false`, `ask: "unsaved" \| "slicing"`, `name` |
| `project_set` | `project?` and any of `plate`, `tab` (`prepare` / `preview`), `view` (a small object), `source` (`project` / `presets`), `vendor`, `machine`, `process`, `filaments`, `bed_type`, `layer_height`, `first_layer_height`, `wall_loops`, `closing_radius` (null to clear), `initial` (these are what it starts with) | as `project`. A change to what it is sliced with drops its slices and stops one that is running |
| `settled` | `project?` | `{ printer: { source, name, vendor, model, variant, nozzle_diameter, bed, height, gcode_flavor, host_type, print_host, print_host_webui, has_api_key, bed_types { enabled, in_force, choices } }, process: { name, layer_height, first_layer_height, wall_loops, closing_radius } }`, either null when nothing is chosen |
| `plate_view` | `project?`, `plate?` | `{ project, plate, bed, height, objects: [{ name, outline, box, height }] }` |
| `slice` | `project?`, `plate?` or `all: true` | `{ project, run }`; `status` says how it goes |
| `cancel` | `project?` | — |
| `status` | `project?` | `{ project, run, state, label, step, steps, plate, at, of, error }`, `state` being idle, slicing, done, failed or cancelled |
| `summary` | `project?`, `plate?` | as before, and `plate`, `widths`, `heights` |
| `layer` | `project?`, `plate?`, `index` | as before |
| `export` | `project?`, `plate?`, `path` | `{ path, bytes }` |

Replaced by these: `open` (now `project_open` and `project`), and the model,
plate and presets that `slice` used to take with each call (now the project's,
set with `project_set`).
