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
- **added** — built in the pass that wrote the row (first pass: Prepare,
  Preview, Device, the project strip; second pass: the sidebar's printer
  block, "Sidebar: Printer" in section 2, and section 8; third pass: the 3D
  views, sections 9 to 11).
- **differs** — built, but on purpose not as the C++ does it; the cell says how.
- **not yet** — not built, with what it waits for. Nothing in this column is
  shown in the page as a control that does nothing.

Everything the page asks for goes through `Session::call` (`src/api.rs`), or
through its twin `Session::call_bytes` for what is answered in bytes. The
calls are listed in section 7.

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

Read from `Sidebar::Sidebar` and `Sidebar::update_all_preset_comboboxes` in
Plater.cpp, `PlaterPresetComboBox` in PresetComboBoxes.cpp, `Tab::select_preset`
in Tab.cpp, and `PresetBundle.cpp` / `Preset.cpp` / `AppConfig.cpp` under
`libslic3r/`. How printers are added and removed, and what is kept between
runs, is section 8.

| C++ | Where | Here |
| --- | --- | --- |
| Section "Printer", folded and unfolded by its title bar (chevron, printer icon, "Printer"; "Click to collapse / expand") | `Sidebar::Sidebar` l.2385–2403, l.2614 | **had** |
| Title bar, right: the gear "settings", which opens Printer Selection (`run_wizard(RR_USER, SP_PRINTERS)`) | l.2588–2591 | **added**: the gear, with the same tooltip |
| Title bar, right: "Synchronize nozzle information" (a U1 or a Bambu printer only): reads the nozzles of the connected printer; "Printer not connected. Please go to the home page or the device page to connect the printer.", "Set Nozzle Diameter", "Nozzle settings synchronized successfully" | l.2406–2577, l.4668 | **not yet**: needs a printer connection (section 4) |
| The printer row: a framed panel (green frame on hover), a 48 px picture of the printer, the preset list; a click anywhere on it opens the list | l.2637–2705 | **added**: the frame (the app's accent on hover) and the picture beside the list |
| The picture: `<printer_model>_cover.png` from the vendor's profiles folder (Snapmaker's from `resources/images` by the preset's name), "printer_placeholder" when there is none | `Sidebar::update_printer_thumbnail` l.12170 | **added**: the cover from the profiles folder for every vendor (`printer_cover`), a grey box when there is none. A project on its own settings shows its printer's picture when that model is among those added |
| The list holds every visible printer preset, in groups with these titles, in this order: "Project-inside presets", "User presets", "System presets". Each group is in order of name (`std::map`: by code unit, capitals first). A system printer is listed once for each `printer_model`, under the model's name, whatever its nozzles | `PlaterPresetComboBox::update`, PresetComboBoxes.cpp l.1528–1749 | **added**: one list (`printers`), the same groups, titles and order; "System presets" has a row for each model added, by model name. Before: "Vendor" and "Printer", listing every preset of every vendor. **not yet**: "User presets" (there are no user presets; the group is left out while it is empty, as in the C++) |
| "Project-inside presets": the presets a project file carries that are not the vendor's own | l.1656–1663, l.1722–1730 | **differs**: the row is the printer the project was saved with (`printer_settings_id`) and stands for the project's own settings as a whole, which is how this engine slices a project. It replaces the first pass's "Sliced with: the project's own settings / presets chosen here". Picking it goes back to those settings, the project's own bed type among them; picking a row of "System presets" slices with presets |
| A preset is listed only if visible; a system printer is visible only when its vendor, model and nozzle are among the printers added (`AppConfig` "models"); with one nozzle of a model added, all of them are | `Preset::set_visible_from_appconfig` Preset.cpp l.1060, `PresetBundle::load_installed_printers` l.2127, `install_missing_variants_for_enabled_models` l.2100 | **added** (section 8). A printer a project is on that is not among those added is listed while it is in force, as the C++ makes a preset visible when it is selected (Plater.cpp l.18619) |
| The row shown as selected; the list's tooltip is the preset's own name; a modified preset has "* " before its name; typing filters the list | l.1595–1599, l.1769–1778, `Preset::label` Preset.cpp l.899 | **added**: the selection and the tooltip. Nothing modifies a preset here, so there is no mark. Typing jumps to a row, as a list of the system's does |
| Two vendors with a model of the same name are one row (the group is keyed by name) | l.1636–1641 | **differs**: a row each, the vendor in the row's tooltip. One row that picks whichever preset comes first is not what a list of printers is for |
| Picking a model: the preset of that model for the nozzle the project is on, else for 0.4, else the first by name; when the model changes, the nozzles last used on its rack | `Plater::priv::on_select_preset` l.18536–18648, `PresetBundle::get_similar_printer_preset` l.3141, `printer_preset_for_remembered_nozzles` l.4047 | **added** (`printer_select`). **not yet**: the rack's remembered nozzles (the engine has no rack) |
| At the foot of the list: "Select/Remove printers (system presets)" opens Printer Selection; "Create printer" opens "Create Printer/Nozzle". Picked, they leave the list on the printer it was on | l.1751–1762, `OnSelect` l.1188–1222, Plater.cpp l.18547 | **added**: "Select/Remove printers (system presets)". **not yet**: "Create printer" (section 8); it is not listed |
| In the list's own row: the edit icon (opens the printer's settings), "Connect to printer" (opens "Devices"), "The machine has been connected and is currently in working mode" | Plater.cpp l.2659–2686, PresetComboBoxes.cpp l.1907–1985 | **not yet**: needs a settings editor, and a printer connection (section 4) |
| Leaving a printer or filament preset with unsaved changes asks first ("Unsaved Changes") | `Tab::select_preset` Tab.cpp l.9037–9105 | not needed: presets are not edited |
| **Another printer, the process**: the one last used with that printer ("Remember printer configuration", on unless turned off); else the one of the same name (alias: the name up to "@") cut for it; else the printer's `default_print_profile`; else the first that fits | `PresetBundle::update_selections` l.2432, `update_compatible` l.7171 (`PreferedPrintProfileMatch` l.7213), Tab.cpp l.9196–9306 | **added** (`printer_select`), in that order, a process that still fits staying before the search by name. **not yet**: the profile remembered for each layer height, the preference itself |
| **Another printer, the process settings changed by hand** are carried over: "Kept %1% process settings from \"%2%\" (revertable)." | `capture_print_settings_carry`, `apply_print_settings_carry` Tab.cpp l.9096, l.9314 | **had**, in effect: the four settings set over the preset stay set when the printer changes. **not yet**: the notice, reverting |
| **Another printer, the filaments**: each one that fits stays; one that does not is replaced by the one of the same name cut for the new printer, else by one of the same `filament_type`, the printer's `default_filament_profile` before others. The filaments last used with the printer are read back and then put aside again for the project's (Tab.cpp l.9251) | `update_compatible` l.7275–7357, Tab.cpp l.9235–9306 | **added**, the same rule for each slot (`Shelf::filament_type` gives the material). A slot with nothing in it yet, as in a new project, takes what was last used in it with that printer, else the printer's usual. **differs**: every slot's usual filament is the printer's first `default_filament_profile`, the engine handing out one |
| **Another printer, the number of filaments**: the larger of the printer's nozzles and the project's filaments; it never shrinks | `TabPrinter::extruders_count_changed` Tab.cpp l.7355, l.3005 | **not yet**: there is a filament for each the file prints in, as before (the engine prints the filaments the file assigns; see "Add one filament") |
| **Another printer, the bed**: the plate takes the printer's bed and height, objects are moved to keep their place on it, every plate's slice is dropped | Plater.cpp l.18554–18703, `on_config_change` l.29200 | **added**: the bed and height (`plate_view`), fitted to the stage anew; the slices are dropped (**had**). **not yet**: moving the objects; they stay where the file puts them |
| "Bed type", a label that opens the wiki page on bed types, and a list: the six plates ("Smooth Cool Plate", "Engineering Plate", "Smooth High Temp Plate", "Textured PEI Plate", "Textured Cool Plate", "Cool Plate (SuperTack)"); a Snapmaker U1's own four ("Textured PEI Plate", "Smooth PEI Plate", "Graphic Effect Plate", "Cool Plate"); or a U1's seven with `support_multi_bed_types` | l.2725–2786, `update_all_preset_comboboxes` l.4464–4487, `libslic3r/PrintConfig.cpp` l.1284–1327 | **had**: the same lists and labels (`settled`, `bed_types`). **not yet**: "Graphic Effect Plate" (the engine has no bed temperature for it); the link (the page loads nothing from the network) |
| The list is enabled for a Bambu printer, a U1, or `support_multi_bed_types`. Any other printer is on its own default, shown greyed | l.4503, l.4555–4559 | **had** the rule. **added**: such a printer is put on its default when it is selected |
| A printer's default bed type: the preset's `default_bed_type`, by name or number; else by model: Cool Plate for `BL-P001`, `BL-P002`, `C13`, Textured PEI Plate for `SM_U1`, else High Temp Plate | `Preset::get_default_bed_type` Preset.cpp l.1168 | **added**. **differs**: the model's own file is read for `default_bed_type` too, after the preset. That is where Bambu's profiles give it and the C++ never looks, so its Bambu printers start on the High Temp Plate whatever the profile says (reported as an error of the C++) |
| The bed type is remembered for each printer preset, by its name (`orca_presets[].curr_bed_type` in the app's config, and once more under `app`): saved when it is picked and whenever the selection is written; read when a printer is selected, unless a project is being loaded, whose own bed type then holds. The first time a printer is used its default is saved at once | l.4508–4554, `on_select_bed_type` l.18515, `PresetBundle::export_selections` l.2718, `load_selections` l.2566 | **added**: kept for each printer preset in the app's settings (section 8), read back when the printer is selected, in this run or a later one; a project opened with its own settings keeps its own. Before: not remembered. **differs**: kept by name ("Cool Plate"), not by the number of the C++ list; no second copy under `app`, which the C++ keeps for what reads it elsewhere; a default is not written until the printer is selected by hand |
| A bed type remembered for a U1 that is not one of its four is brought to one of them: Cool Plate for the SuperTack, Textured PEI Plate for the rest | l.4531–4537 | **added**. For any other printer a remembered plate that is not in its list gives the printer's default; the C++ shows the first row and slices with the plate that is not listed |
| Picking a bed type marks the project modified and drops the slices of the plates without one of their own | `Plater::priv::on_select_bed_type` l.18465 | **had** (`project_set`): every slice is dropped, a plate having no bed type of its own here |
| A bed type for one plate ("Same as Global Plate Type" and the printer's plates), reset with a notice when another printer does not take it ("Plate bed types reset") | `PartPlate::get_bed_type` PartPlate.cpp l.196, PlateSettingsDialog.cpp l.394, `Plater::normalize_bed_types` l.21433 | **not yet**: the engine has one bed type for a slice (see "Plates") |
| Nozzles. One nozzle: "Nozzle" with "Diameter", a list of the sizes the model has presets for ("0.4mm"), which picks the preset; off with fewer than two. Two: "Left Nozzle" / "Right Nozzle". "Flow": "Standard" / "High Flow". "Rack": "Edit" / "Add", "Sync" | `update_nozzle_settings` l.11606–11959 | **added**: "Nozzle", "Diameter" with the sizes of the model's presets, picking among them (`printer_select` with `variant`); off with fewer than two, and for a project's own printer. Before: the size as text. **not yet**: "Flow", "Rack" |
| Several nozzles that may differ (a U1's T1 to T4, a Bambu H2D's L and R): "Nozzles", a tile for each with its size and flow ("0.4", "Std"), "Settings ›"; a tile's menu sets that tool's size in the project and the printer preset stays | l.5720–5819, `Widgets/NozzleStrip.cpp` l.336, `libslic3r/NozzleStripModel.cpp` | **added**, read only: "Nozzles", a tile for each nozzle of the preset with its size. **differs**: "Diameter" is there for these printers too and picks the model's preset for that size, all tools alike, the engine having no size of its own for a tool. **not yet**: a size for one tool, flow, "Settings ›", the tile's menu |
| — | — | The build volume ("270 × 270 × 270 mm") under the nozzles is this app's own line (**had**) |

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
| A 3D view of every plate with the objects on it, orbit / pan / zoom, selection | `View3D`, `GLCanvas3D` | **added**, in 3D: section 9. Where the browser has no WebGL2, the view from above the first pass built is kept: the bed of the printer chosen with its grid and each object's outline (its convex hull), name and size, drag to move, wheel to zoom, "Fit part" [Z] and "Fit bed" |
| Main toolbar: "Add" [Ctrl+I], "Add plate", "Auto orient all/selected objects" [Q], "Arrange all objects" [A] with its options, "Split to objects", "Split to parts", "Variable layer height", "Assembly View" | `GLCanvas3D::_init_main_toolbar` l.9274 | **not yet**: the engine does not move, add or split anything; a file is sliced as it was saved |
| Gizmos: "Move" [M], "Rotate" [R], "Scale" [S], "Lay on face" [F], "Cut" [C], "Mesh Boolean" [B], "Support Painting" [L], "Seam painting" [P], "Paint-on fuzzy skin" [H], "Color Painting" [N], "Texture displacement", "Emboss" [T], "SVG", "Measure" [U], "Assemble" [Y], "Simplify", "Brim Ears" [E], "Sculpt", "Edit", "Fillet / chamfer (CAD)" | `Gizmos/GLGizmosManager.cpp::init` l.224 | **not yet** |
| Object list: plates, objects, parts, instances, layer ranges; columns "Name", printable, "Fila.", paint marks; the object, part and plate menus | `GUI_ObjectList.cpp`, `GUI_Factories.cpp` | **not yet** as a list. The names of the objects on the plate are on the bed and in `project` (`plates[].objects`) |
| View options: "Realistic View", "X-Ray", "Center of Mass", "Show 3D Navigator", Ctrl+0..6 views | `GLCanvas3D.cpp` l.8297, View menu | **added**: the navigator and the views (section 9). **not yet**: the three view options |

### Plates

| C++ | Where | Here |
| --- | --- | --- |
| Every plate shown on the canvas; a click selects it | `PartPlateList`, `Plater::select_plate_by_hover_id` l.30481 | **added**: in the 3D scene, every plate where it stands, a click selecting it (section 9); and the column of plate numbers of the first pass (two plates or more), in Prepare and in Preview |
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
| The sliced toolpaths in 3D over transparent shells of the objects | `Preview::load_print_as_fff` (GUI_Preview.cpp l.659), `GCodeViewer` | **added**, in 3D: section 10, from the moves of the G-code (`toolpath`, `toolpath_moves`). Without WebGL2: one layer from above, drawn from the engine's own lines (`layer`), as before |
| View type list, in this order: "Line Type", "Filament", "Speed", "Actual Speed", "Acceleration", "Jerk", "Layer Height", "Line Width", "Flow", "Actual Flow", "Layer Time", "Layer Time (log)", "Fan Speed", "Temperature", "Pressure Advance" | `GCodeViewer::update_by_mode` l.1272, `get_view_type_string` l.106 | **added**: "Line Type", "Filament", "Layer Height", "Line Width" (first pass), and in 3D "Speed", "Flow", "Fan Speed", "Temperature" where the print was read back from its G-code (section 10). **not yet**: the views that need each move's time or planned speed ("Actual Speed", "Acceleration", "Jerk", "Actual Flow", the layer times, "Pressure Advance") |
| The view type a print opens in: "Automatic" (Filament with several filaments, else Line Type) or "Last used" ("Default preview view type") | `apply_default_view_type` l.1248, Preferences.cpp l.2038 | **differs**: the last one used in that project, kept with it in the session (`view`); a new project starts in "Line Type". **not yet**: the preference |
| Legend, top right: fold button, G-code window button, the list | `render_legend` l.4120 | **added**: fold button and list. **not yet**: the G-code window |
| "Line Type" rows in the order of the roles, only those in the print, each with the C++ colour; columns "Line Type", "Time", "%", "Usage", "Display"; a click on a row hides or shows it | l.4300–4645, `libvgcode/src/ViewerImpl.cpp` l.302 | **added**: the C++ order and colours (before: this app's own), a "Display" mark on each row, a click on the row toggles it (**had** as checkboxes). **not yet**: "Time", "%" and "Usage" for each feature; the roles the engine never makes ("Ironing", "Custom", "Support transition", "Wall over support", "Bottom surface over support") |
| "Supported overhangs" / "Unsupported overhangs" overlays | l.4600 | **not yet** |
| Option rows: "Travel", "Wipe", "Retract", "Unretract", "Seams", "Filament changes" | l.4624, `append_option_item` l.4497 | **added** in 3D (section 10) |
| "Filament" view: a row for each filament used with "Model", "Support", "Flushed", "Tower", "Total"; "Filament change times" | l.4417–4864 | **added**: a row for each filament with its colour, to hide or show (**had**), and "Filament change times:". **not yet**: the split of each filament's use |
| "Total estimation": "Total Filament:", "Model Filament:", "Cost:", "Prepare time:", "Model printing time:", "Total time:", "Show stealth mode" | l.5277–5390 | **added** in the legend: "Total Filament:" (m and g) and "Total time:", with how the time was worked out (**had** in the sidebar, as "Filament", "Time", "Travel"). **not yet**: model filament apart from support and tower, cost, prepare time, stealth mode |
| Vertical layer slider at the right: two thumbs (a range of layers), the layer's number over its height beside the thumb, hover shows a layer's time | `IMSlider.cpp` l.890–1230, `get_label` l.1723 | **added** in 3D: two thumbs, the same labels, one layer mode [L] (section 10). Without WebGL2, the first pass's slider: one thumb, always one layer. **not yet**: layer times |
| Up / Down move a layer; five with Shift or Ctrl; the mouse wheel over the slider likewise | `GLCanvas3D` l.5866, `IMSlider.cpp` l.1636 | **added** (before: arrows by one, PageUp / PageDown by ten; those two keys now go round the projects with Ctrl, as in the C++) |
| "Jump to Layer" (the slider's menu; Shift+G): "Please enter the layer number (min - max):" | `IMSlider::render_add_menu` l.1512 | **added**: the slider's menu and Shift+G |
| The slider's other menu entries: "Add Pause", "Add Custom G-code", "Add Custom Template", "Change Filament"; ticks with their delete and edit | l.1512, `render_edit_menu` l.1569 | **not yet**: the engine has no per-layer custom G-code |
| "Compare slices (A/B)" | IMSlider.cpp l.1300 | **not yet** |
| Horizontal move slider, the tool marker, the position window with its properties | `update_moves_slider` l.2331, GCodeViewer.cpp l.393–760 | **added** in 3D: the slider, the marker (an arrow) and the window's two lines (section 10). **not yet**: the table of a move's properties, a vendor's hotend model |
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
| Z: zoom to the objects | **added**: "Fit in all view" in 3D (section 9); "Fit part" from above |
| Ctrl+S, Ctrl+Shift+S, Ctrl+I, Ctrl+G, Ctrl+Shift+G, Ctrl+P (Preferences), "?" (the list of shortcuts) | **not yet** |
| Left / Right, Home / End (the move slider), L (one layer mode), C (G-code window) | **added** in 3D: all but C (no G-code window) |
| A, Shift+A, Q, Shift+Q, the gizmo keys, Ctrl+0..6, Ctrl+A / D / Z / Y / C / V / X, Del, 1–9, arrows to move objects | **added** in 3D: Ctrl+0..7 (the views), Ctrl+A, Esc, I and O, Ctrl+E (section 9). **not yet**: the rest, Prepare does not edit |

## 7. What the page asks for

Every call takes JSON and answers JSON, but the four at the end of the table,
which answer bytes (`Session::call_bytes`; section 11); an error is a sentence
to show. A call about a project takes `project` (its id) and means the one
shown when it is left out; one about a plate takes `plate` and means the plate
the project is on.

| Call | Takes | Answers |
| --- | --- | --- |
| `startup` | — | `{ model }`: a file named when the app was started |
| `library`, `set_profiles { dir }` | | `{ dir, vendors }`. The folder set is kept for the next run |
| `printers` | `project?` | `{ first_run, installed: [{ vendor, brand, model, name, cover, variants: [{ variant, machine }] }], selected: { source: "preset" \| "project", vendor, machine, model, variant, added } \| null, project_printer, last: { vendor, machine } \| null, settings: { file, problem } }` |
| `printer_catalogue` | — | `{ models: [{ vendor, brand, model, name, nozzle_diameter, x, y, z, toolheads, cover, enabled }], unread }` |
| `printer_cover` | `vendor`, `model` | `{ data }`: the picture as a `data:` URL, or null |
| `printers_save` | `project?`, `models: [{ vendor, model }]` | as `printers`, and `added`, `removed` (`[{ vendor, model }]`), `switched: [{ project, name, from, to }]`. An error with none: "At least one printer must be selected." |
| `printers_default` | `project?` | as `printers_save`: adds the Snapmaker U1 when no printer is added |
| `printer_select` | `project?` and `vendor` with `model` (one of those added), `variant?`; or `variant` alone (another nozzle of the model the project is on); or neither (the printer a project starts with); `initial?` | as `project` |
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
| `project_set` (what is set for `bed_type`, `process` or `filaments` is kept with the printer the project is on) | `project?` and any of `plate`, `tab` (`prepare` / `preview`), `view` (a small object), `source` (`project` / `presets`), `vendor`, `machine`, `process`, `filaments`, `bed_type`, `layer_height`, `first_layer_height`, `wall_loops`, `closing_radius` (null to clear), `initial` (these are what it starts with) | as `project`. A change to what it is sliced with drops its slices and stops one that is running |
| `settled` | `project?` | `{ printer: { source, name, vendor, model, variant, nozzle_diameter, bed, height, gcode_flavor, host_type, print_host, print_host_webui, has_api_key, bed_types { enabled, in_force, choices } }, process: { name, layer_height, first_layer_height, wall_loops, closing_radius } }`, either null when nothing is chosen |
| `plate_view` | `project?`, `plate?` | `{ project, plate, bed, height, objects: [{ name, outline, box, height }] }` |
| `slice` | `project?`, `plate?` or `all: true` | `{ project, run }`; `status` says how it goes |
| `cancel` | `project?` | — |
| `status` | `project?` | `{ project, run, state, label, step, steps, plate, at, of, error }`, `state` being idle, slicing, done, failed or cancelled |
| `summary` | `project?`, `plate?` | as before, and `plate`, `widths`, `heights` |
| `layer` | `project?`, `plate?`, `index` | as before |
| `export` | `project?`, `plate?`, `path` | `{ path, bytes }` |
| `scene` | `project?` | the project for Prepare in 3D: `{ project, key, bare, bed { box, shape, height, model, texture }, plates: [{ id, name, origin }], plate, colours, objects: [{ name, vertices, triangles, bytes, filaments }], placings: [{ object, plate, printable, matrix (16 numbers, column by column), box }] }`. `key` changes only when the file does |
| `toolpath` | `project?`, `plate?` | a sliced plate's moves, before they are asked for: `{ source: "gcode" \| "lines", moves, move_size, at_once, layers { first, z, height }, features, filaments, counts { travel, line, wipe, retract, unretract, seam, tool_change }, box, plate, run }` |
| `scene_mesh` (bytes) | `project?`, `object` | an object's mesh, `EDGM`: 24 bytes of header, vertices (`f32` × 3), triangles (`u32` × 3), a mark byte for each triangle when they differ |
| `bed_model` (bytes) | `project?` | the vendor's model of the printer's bed, as `scene_mesh` |
| `bed_texture` (bytes) | `project?` | the vendor's picture of the bed: the SVG or PNG file as it is |
| `toolpath_moves` (bytes) | `project?`, `plate?`, `from`, `count` | a stretch of moves, `EDGT`: 24 bytes of header, then 32 bytes a move (where it ends, speed, height, width, turn, temperature, layer, what it is, feature, filament, fan) |

Replaced by these: `open` (now `project_open` and `project`), and the model,
plate and presets that `slice` used to take with each call (now the project's,
set with `project_set`).

## 8. Printers: adding, removing, and what is kept between runs

Read from `WebGuideDialog.cpp/.hpp` (`GuideFrame`), the pages under
`resources/web/guide/` (`24/`, `21/`, `22/`, `js/printer_table.js`,
`js/data_table.js`, `../data/text.js`), `GUI_App.cpp` (`run_wizard`,
`config_wizard_startup`), `libslic3r/AppConfig.cpp`, `libslic3r/PresetBundle.cpp`,
`libslic3r/PrinterModelSpecs.cpp`, `CreatePresetsDialog.cpp` and `Tab.cpp`.

### Where it is opened from

| C++ | Where | Here |
| --- | --- | --- |
| "Select/Remove printers (system presets)" at the foot of the printer list, the gear in the Printer title bar, and "Add/Remove presets" in the preset cog's menu all open the guide on its printer page alone (`BBL_MODELS_ONLY`, page `24`): a window of 1000 × 720 that can be resized, no title | PresetComboBoxes.cpp l.1207, l.1515, Plater.cpp l.2590, `GUI_App::run_wizard` l.9592–9643, WebGuideDialog.cpp l.227–280 | **added**: the list's entry and the gear open "Printer Selection", a dialog of that size. **not yet**: resizing it; the cog's menu (there is no cog) |
| The whole guide (Help > "Setup Wizard", and the first run): "Welcome to EdgeSlicer" / "Get Started" (page `1`), "Please select your login region" (`11`), "Printer Selection" with "Back" / "Next" (`21`), "Filament Selection" with "Back" / "Finish" (`22`) | `GuidePage`, WebGuideDialog.hpp l.61, pages `1`, `11`, `21`, `22` | **differs**: the printer page only, on the first run too (below). **not yet**: the welcome page, the region (it is for the vendor's account, which this app has none of), the filament page (below) |

### Printer Selection

| C++ | Where | Here |
| --- | --- | --- |
| Title "Printer Selection"; one table, a row for each printer model of every vendor, all its nozzles together; the body is drawn only where it is in sight | `24/index.html`, `js/printer_table.js` | **added**: the table, every row built at once (393 of them), the pictures fetched as their rows come into sight |
| The rows come from every vendor's `machine_model_list`: `model` (the name in the list), `name` (the model file's), `vendor`, `nozzle_diameter` ("0.2;0.4;0.6"), `cover` (`<model>_cover.png` in the vendor's folder), `materials`; a model whose file is missing is left out | `GuideFrame::LoadProfileFamily` l.1389–1435 | **added**: `Library::models` in `edge-engine` reads the same, and `printer_catalogue` hands it to the page |
| Vendors: the filament library, Snapmaker, then the user's installed vendors and the rest in the order the folder is read in | `GuideFrame::LoadProfileData` l.1169–1273; `VENDOR_PRIORITY` in printer_table.js | **differs**: Snapmaker's, then the others by vendor name: the same list, in the same order on every machine |
| Columns "Enabled", the picture, "Brand" (`BBL` is "Bambu Lab", `Custom` "Custom Printer", `Other` "Orca colosseum"), "Model", "X mm", "Y mm", "Z mm", "Toolheads"; a value that is not known is "—" | printer_table.js l.23–77 | **added** |
| X, Y, Z and toolheads are those of the model's preset for a 0.4 nozzle, else for the first nozzle it lists, the shortest name among several ("Bed width, from the printer's default (0.4 mm nozzle) preset", "Bed depth, …", "Printable height, …", "Number of extruders / toolheads") | `MachinePresetIndex::specs_for_model`, `libslic3r/PrinterModelSpecs.cpp` l.135; WebGuideDialog.cpp l.1509 | **added** (`Library::models`: `bed`, `height`, `nozzles`), with the same tooltips |
| A printer that is added is ticked when the page opens; one tick stands for every nozzle of the model | `SaveProfileData` l.1283–1313, printer_table.js l.104 | **added**; an added model's name is also in bold |
| A click anywhere on a row ticks or unticks it | `onBodyClick`, data_table.js l.507 | **added** |
| Search box "Search brand or model": every word typed must be in the brand, the vendor or the model, capitals not counting | data_table.js l.110, l.182; printer_table.js l.106 | **added** |
| A funnel on each column ("Filter Brand"): a list of values to tick for "Enabled" ("Enabled" / "Not enabled"), "Brand" (with "Find…") and "Toolheads", with "Select all" / "Select none"; "Contains…" for "Model"; "At least" / "At most" for the sizes ("In mm. Printers without this value are hidden while a limit is set.") | data_table.js | **added**, each applied with "OK" or Enter. The "Enabled" filter keeps the rows it showed when it was set, so a row does not leave as it is ticked |
| A chip for each filter in force with ✕ ("Remove this filter"), and "Clear search and filters" | data_table.js l.258–289 | **added** |
| A click on a column's title sorts by it: up, down, then the order given ("Click to sort") | data_table.js | **added**; a printer without the value sorted by comes last |
| "{n} printers · {on} enabled", or "{shown} of {n} shown · {on} enabled"; "All / Clear all: every printer", or "All / Clear all: the {shown} shown" | data_table.js l.89–105 | **added** |
| "All" and "Clear all" tick and untick the rows shown ("Applies to the {shown} printers shown by the search and filters ({on} of them enabled)") | `setShown`, data_table.js l.417 | **added** |
| Nothing left: "No printer matches the search and filters." | data_table.js | **added** |
| "Confirm" and "Cancel". Confirm with nothing ticked: a notice "error", "At least one printer must be selected.", "ok" | `24/index.html`, `ConfirmSelect` 24.js l.68 | **added**. **differs**: the sentence is shown in the dialog, above the buttons, not in a notice of its own |
| Escape closes what is open, innermost first: a filter, the notice, the dialog (as Cancel) | 24.js l.93–102 | **added** |

### Confirm, Cancel and the first run

| C++ | Where | Here |
| --- | --- | --- |
| Confirm keeps the models ticked, every nozzle of each, as the printers added (`AppConfig::set_vendors`); the list is built anew from what the page sends | `SaveProfile` l.676–758, `apply_config` l.788–957 | **added** (`printers_save`). A printer added from another profiles folder was not on the page to be unticked, and stays added |
| The printer selected afterwards: the first model newly added (Snapmaker's before other vendors', then by name) on its 0.4 nozzle, or its smallest without one; with nothing added, the printer that was selected | `apply_config` l.854–897, `PresetBundle::default_printer_variant` l.297, `load_selections` l.2506–2563 | **added**, for the project shown. **differs**: a project sliced with its own settings stays on them (the C++, which has one selection, would move it) |
| The printer that was selected has been removed: another nozzle of its model that is still added, else the first printer added, by preset name; with none at all, "Default Printer". The open project simply goes to that printer; nothing is asked | `PresetCollection::select_preset_by_name` Preset.cpp l.4114–4161 | **added**, for every open project that was on a removed printer, each named in the status line afterwards (`switched`): the printer last selected if it is still added, else the first by preset name. **differs**: of that first model the 0.4 preset is taken, not the first by name, which is the 0.2 one; where the C++ picks a preset of a model elsewhere (`get_similar_printer_preset`) it says that is not wanted |
| With changes to a preset unsaved: "Configuration package changed" (keep or save) | `apply_config` l.821–828 | not needed: presets are not edited |
| Removing a vendor's last printer leaves its profiles on the disk | l.839–852 | the same: the profiles folder is only read |
| The guide opens by itself at startup when there is no config file, when only the default printers are there, or when it was never finished | `GUI_App::config_wizard_startup` l.9874, `Utils/StartupWizardLogic.hpp` l.40 | **added**: with no printer added, Printer Selection opens by itself when the page starts and when another profiles folder is chosen |
| No printer is ticked beforehand on a first run (but a model that is the only one there is) | `SaveProfileData` l.1315; no default in the page | **added**: none is |
| The first run's guide has no Cancel and no close box; closed all the same with only default printers there, the Snapmaker U1 with a 0.4 nozzle is added and selected, with "Snapmaker PLA SnapSpeed" | `GuideFrame::run` l.1037–1053, PresetBundle.cpp l.291–294 | **added** (`printers_default`): Cancel or Escape on a first run adds the Snapmaker U1 and starts the project on its 0.4 preset. **differs**: the dialog has its Cancel on a first run too; the filament is the one the U1's profile names as usual, there being no list of filaments added. Profiles without a Snapmaker U1 add nothing, and the list offers only "Select/Remove printers (system presets)" |

### What is kept, and where

| C++ | Where | Here |
| --- | --- | --- |
| One file, `EdgeSlicer.conf` (JSON), in the application's data folder: `~/Library/Application Support/EdgeSlicer`, `%APPDATA%\EdgeSlicer`, `~/.config/EdgeSlicer`, or `data_dir` beside the executable | `AppConfig::config_path` l.1916, GUI_App.cpp l.2927 | **added**: `settings.json` of this app's own (`src/api/store.rs`), in `~/Library/Application Support/EdgeSlicerRS` (macOS), `%APPDATA%\EdgeSlicerRS` (Windows), `$XDG_CONFIG_HOME/EdgeSlicerRS` or `~/.config/EdgeSlicerRS` (elsewhere), or in the folder `EDGE_DESKTOP_DATA` names. The C++ application's file is neither read nor written. Written whole or not at all; a file that cannot be read is said and set aside, not written over; what another build wrote in it is kept |
| The printers added: `"models": [{ "vendor", "model", "nozzle_diameter": "0.2;0.4" }]` | `AppConfig::save` l.1264–1281, `load` l.883 | **added**, in the same layout |
| The printer selected: `presets.machine`; what was last used with each printer: `orca_presets: [{ machine, process, filament, filament_01…, curr_bed_type, filament colours, flush volumes }]`, written whenever the selection changes | `PresetBundle::export_selections` l.2687–2786 | **added**: `presets { vendor, machine }` and `printers: [{ machine, vendor, process, filaments, curr_bed_type }]`, written when a printer is selected by hand and when a process, a filament or a bed type is chosen. **not yet**: colours and flush volumes (the page sets neither) |
| The filaments added: `"filaments": [...]`; process presets hidden; `firstguide.finish`; `app.region` | `AppConfig::save` l.1224, `SaveProfile` l.687 | **not yet**: every filament and process made for a printer is offered (below) |
| The profiles in use: the vendors installed into the data folder's `system`, else `resources/profiles` | `LoadProfileData` l.1194–1252 | **added**: the profiles folder chosen in the window is kept (`app.profiles_dir`); `EDGE_PROFILES` comes before it. Before: it was forgotten when the app closed |
| The page's own: the strip's width and folding, the last file opened | — | **had**, kept by the page (`localStorage`). The printer, process and filaments last used were kept there too; they are in `settings.json` now, and the old entries are no longer read |

### Not built: filaments, creating and deleting printers

| C++ | Where | Here |
| --- | --- | --- |
| "Filament Selection" (the guide's last page): the filaments of the printers ticked, to tick by "Printer", "Filament type" and "Vendor", the models' `default_materials` ticked when none is ("Do you want to use default filament ?"); kept as the `filaments` section; a filament list shows only those | `22/22.js`, `SaveProfile` l.704–715, `Preset::set_visible_from_appconfig` | **not yet**: a filament list here offers every filament preset made for the printer. It waits for nothing in the engine (`PrinterModel::default_materials` is there); it is its own pass |
| "Add/Remove filaments" at the foot of each filament list opens the Filament manager (`run_preset_manager`), not the guide | PresetComboBoxes.cpp l.1756, GUI_App.cpp l.9598 | **not yet**, with the above |
| "Create printer" opens "Create Printer/Nozzle": "Create Printer" or "Create Nozzle for Existing Printer"; vendor and model ("Select Vendor", "Select Model", "Input Custom Vendor", "Input Custom Model", "Can't find my printer model"), "Nozzle Diameter", "Bed Shape", "Printable Space", "Origin", "Hot Bed STL", "Hot Bed SVG", "Max Print Height"; then "Create from Template" or "Create Based on Current Printer" with the filament and process presets to make; "Printer Created Successfully" | `CreatePrinterPresetDialog`, CreatePresetsDialog.cpp l.1769–3490 | **not yet**: it writes user presets, which neither the engine (`edge-config` reads vendor profiles only) nor this app has |
| Deleting a user printer preset (the delete button of the printer's settings, "Delete this preset"): never a system or default one; "Are you sure to delete the selected preset?"; "%d Filament Preset and %d Process Preset is attached to this printer. Those presets would be deleted if the printer is deleted."; "Presets inherited by other presets cannot be deleted!"; then the next preset by name is selected and the project goes to it | `Tab::delete_preset` Tab.cpp l.10030–10159, l.8978 | **not yet**: there are no user presets and no settings editor |


## 9. Prepare: the 3D scene

Read from `GLCanvas3D.cpp/.hpp`, `Camera.cpp/.hpp`, `3DBed.cpp`, `2DBed.cpp`,
`PartPlate.cpp/.hpp`, `3DScene.cpp`, `Selection.cpp`, `KBShortcutsDialog.cpp`,
`MainFrame.cpp` (the View menu), `Preferences.cpp`, `libslic3r/Color.hpp` and
the shaders under `resources/shaders/140`. (`CameraPopup.cpp` is the printer's
live camera, not this one. There is no `CoordAxes.cpp` in this fork: the axes
are `Bed3D::Axes`.)

The scene is drawn on WebGL2 by the page's own code, `ui/gl/` (no library):
`math.js`, `camera.js`, `shaders.js`, `renderer.js`, `scene.js`, `stage.js`,
`controls.js`. Where the browser has no WebGL2 the stage says so and keeps
the view from above it had before ("The bed" in section 2). Colours are the
reference's own numbers; each is named in `scene.js` with where it is from.

### The camera

| C++ | Where | Here |
| --- | --- | --- |
| Looks at a target from a distance (1000 mm to start with); the zoom is pixels to a millimetre at the target, in perspective as in orthographic view: the perspective frustum is sized from the zoom, not from an angle | `Camera::apply_projection` l.246, `DefaultDistance` | **added** (`camera.js` `matrices`) |
| Near and far planes hug the scene: 10 mm of margin, at least 50 mm apart, the near plane no closer than 100 mm (the eye is moved back) | `calc_tight_frustrum_zs_around` l.474 | **added** |
| Zoom kept between a fifth of what fits the scene and 250 | `set_zoom` l.101, Camera.hpp l.168 | **added** |
| A step of zoom is a tenth: `zoom / (1 - step * 0.1)`, at most four steps at once | Camera.hpp l.86 `update_zoom` | **added** |
| "Use Perspective View" / "Use Orthogonal View" (`use_perspective_camera`, on), "Auto Perspective" ("Automatically switch between orthographic and perspective when changing from top/bottom/side views.", off) | MainFrame.cpp l.3779–3878, `Camera::auto_type` l.66 | **added**: the three, in the stage's "View" menu, kept by the page |
| Views: "iso" (from the front right, 45° up), "top" (up is +Y), "bottom" (up is -Y), "front", "rear", "left", "right" (up is +Z), "topfront" and "plate" (from the front, 45° up) | `Camera::select_view` l.112, `set_default_orientation` l.750 | **added** |
| The window starts at "topfront" with the bed in sight | MainFrame.cpp l.1391 | **added**: when a project is first shown, and when its bed is another size |
| Orbit keeps +Z up and stops straight above and straight below (zenit -90° to 90°) | `rotate_on_sphere_with_target` l.394 | **added** |
| "Use free camera" (a trackball), a 3Dconnexion mouse | `rotate_local_around_target` l.452, `Mouse3DController` | **not yet**: no preferences dialog; no device support in a page |
| Fit: a box fills the view with a margin of 1.25 (objects, a plate) or 2 (the bed) | `calc_zoom_to_bounding_box_factor` l.515, GLCanvas3D.cpp l.1780 | **added** |
| One camera for Prepare and Preview; going from one to the other keeps the view | Plater.cpp l.18251 | **added**: one camera for each project, shared by its two tabs. Before: a view from above for each tab |

### The mouse

| C++ | Where | Here |
| --- | --- | --- |
| Left button dragged: "Rotate View", 0.8° a pixel, about the middle of what is selected, else of the objects on the plate shown, else of the plate; with Ctrl or Cmd about the point of the bed in the middle of the view | `on_mouse` l.6642–6716, `TRACKBALLSIZE` | **added**. **differs**: a drag that starts on an object turns the view too; in the C++ it moves the object, which nothing here can do yet |
| Right or middle button dragged: "Pan View" | l.6718–6747 | **added**. **differs**: what is under the pointer on the plane through the target stays under it. The C++ measures the move on the near plane, so in perspective the scene lags the pointer (listed with the errors found) |
| Mouse wheel: "Zoom View"; "Zoom to mouse position" (off), "Reverse mouse zoom" (off) | `on_mouse_wheel` l.6007 | **added**, both in the "View" menu. **differs**: zooming to the pointer starts on, as the view from above did |
| "Orbit speed multiplier", "Swap pan and rotate mouse buttons", "Camera style": "Touchpad" (Alt to turn, Shift to move) | Preferences.cpp l.1998–2029 | **not yet**: no preferences |
| Pinch to zoom, two fingers to move and to turn | l.6136–6169 | **added**: the pinch (a browser sends it as a wheel). **not yet**: the other two |
| A click on an object selects it; Ctrl adds it to the selection or takes it away; a click on nothing lets go of everything (not with Shift held); a right click selects | l.6423–6551, l.6749–6858 | **added** |
| A click on a plate selects the plate and lets go of the objects | Plater.cpp l.30481 | **added** |
| Alt: "Select a part"; Shift: "Select objects by rectangle"; a double click opens the object's settings; the right button's menus | l.6423–6551, l.6871 | **not yet**: parts are not told apart, and there are no object settings or menus |
| Dragging an object moves it on the bed | l.6553–6608 | **not yet**: the engine does not move objects |
| Hovering changes no colour (the hover colours are commented out) | 3DScene.cpp l.362 | the same |
| What is under the pointer is found by casting a ray at the meshes | `_picking_pass` l.9819, `SceneRaycaster` | **differs**: the one pixel under the pointer is drawn with every object and plate in a colour of its own, and read back (`renderer.js` `pick`) |

### Keys

| C++ | Here |
| --- | --- |
| Ctrl+0 "Camera view - Default" (the plate view with the bed in sight), Ctrl+1 "Top", Ctrl+2 "Bottom", Ctrl+3 "Front", Ctrl+4 "Behind", Ctrl+5 "Left side", Ctrl+6 "Right side", Ctrl+7 the plate from the front and above | **added** (and Cmd with them; a browser keeps Cmd+digit for its own tabs), and as "Default View", "Top", "Bottom", "Front", "Rear", "Left", "Right" in the "View" menu |
| Z "Fit in all view": the plate view, with what is selected in sight, else the bed | **added** ("Fit part" [Z] before, in the view from above) |
| I "Zoom in", O "Zoom out" | **added** |
| Esc "Deselect all" | **added** |
| Ctrl+A "Select all objects" | **added**: those on the plate shown |
| Ctrl+E "Show Labels" | **added**: the names of the objects on the plate shown |
| Tab "Switch between Prepare/Preview" | **had** |
| Arrows, PgUp / PgDn, Del, Ctrl+C / V / X / Z / Y / D / K, 1–9, A, Q, the gizmo keys, "?" | **not yet**: Prepare does not edit |

### The bed

| C++ | Where | Here |
| --- | --- | --- |
| The vendor's model of the bed (an STL named by `bed_model` in the printer model's own file, beside the vendor's presets), its middle at the middle of the printable area, 0.45 mm under the ground, lit, in (0.3255, 0.337, 0.337), dark (0.255, 0.255, 0.283) | `Bed3D::render_model` l.650, `update_model_offset` l.615, `libslic3r/Preset.cpp` l.5017 | **added**: `scene` names it, `bed_model` gives the mesh |
| Without a model: the bed's shape, flat, a shade darker (×0.8), 0.04 mm under the ground | `render_default` l.709 | **added** |
| The vendor's picture of the bed (`bed_texture`, an SVG or a PNG) over the plate shown, at most 2048 pixels a side | `PartPlate::render_logo` l.738 | **added**: `bed_texture` gives the file as it is; the browser draws it |
| For a Bambu printer: the bed type's own pictures and the calibration lines | l.800–850 | **not yet** |
| A custom model or picture set in the printer's settings (`bed_custom_model`, `bed_custom_texture`) | Plater.cpp l.29302 | **not yet**: presets are not edited |
| Axes from the origin: X (255, 60, 91), Y (100, 200, 24), Z (47, 136, 233), a tenth of the build volume's greatest side long, unlit | `Bed3D::Axes::render` l.208 | **added** |
| From under the bed nothing of it is drawn but the grid, in (0.8, 0.8, 0.8, 0.4) | `render_internal` l.375, `render_grid` | **added** |

### Plates

| C++ | Where | Here |
| --- | --- | --- |
| The plates stand in a grid ⌈√n⌉ wide, a fifth of a bed apart, rows towards -Y | `compute_colum_count`, `compute_shape_position` l.3910, `LOGICAL_PART_PLATE_GAP` | **added**: `scene` gives each plate's origin for the bed of the printer chosen (`edge_engine::plate_position`); an object goes with its plate when the printer is another |
| Every plate that is not the one shown is filled (0.82, dark (0.384, 0.384, 0.412)); the one shown is the bed itself | `render_background` l.656 | **added** |
| A grid from the plate's origin: every 10 mm (20, 50 or 100 on a bed 600, 1200 or 6000 mm across), every fifth line bold, and the bed's outline; (0.89), dark (0.431, 0.431, 0.463); the plate shown (0.5294, 0.5451, 0.5333), dark (0.298, 0.298, 0.3333) | `calc_gridlines` l.476, `render_grid` l.995, 2DBed.cpp l.27 | **added**; the bold lines are two pixels wide, drawn as quads (WebGL has no wide lines) |
| The plate's number, "01" to "36", at its bottom right, and its name ("Untitled" without one) at its top left, in (242, 117, 78) | `calc_vertex_for_number` l.545, `generate_plate_name_texture` l.2264 | **added** |
| Icons beside each plate: "Remove current plate (if not last one)", "Auto orient objects on current plate", "Arrange objects on current plate", "Lock current plate", "Customize current plate", "Move plate to the front", "Print history", "Edit current plate name" | `render_icons` l.1154 | **not yet**: none of what they do can be done yet (section 2, "Plates") |
| Height limits for printing by object, the areas a printer keeps clear, the stripes of a two-nozzle bed | l.858, l.968, `calc_height_limit` | **not yet** |

### Objects

| C++ | Where | Here |
| --- | --- | --- |
| An object is shaded triangle by triangle (each face its own normal) by two lights fixed to the camera: ambient 0.3, top light 0.48, front light 0.18, a little gloss | `resources/shaders/140/gouraud.vs`, `gouraud.fs` | **added** (`shaders.js` `mesh`) |
| Its colour is its filament's (`filament_colour`); black is lifted to (0.2, 0.2, 0.2) so that it can be shaded | `update_colors_by_extruder` l.1442, `adjust_color_for_rendering` l.143 | **added**. A filament the project has no colour for takes one of the C++ object colours in turn (`GLVolume::MODEL_COLOR`) |
| Colours painted on an object show on it | `GLVolume::simple_render` l.570 | **added**: `scene_mesh` gives the triangles as the paint cuts them, each with its filament |
| Negative parts (0.3, 0.3, 0.3, 0.4), modifiers (1, 1, 0, 0.6), support blockers (1, 0.3, 0.3, 0.4) and enforcers (0.3, 0.3, 1, 0.4), seen through | 3DScene.cpp l.217–220 | **added** |
| A copy that is not to be printed: (0, 0, 0, 0.5) | `UNPRINTABLE_COLOR` | **added** |
| What reaches out of the plate's print volume is a third darker | gouraud.fs l.199 | **added** for a bed's box. **not yet**: a round bed's own outline |
| The background turns (0.753, 0.192, 0.039) while an object is off its plate | GLCanvas3D.cpp l.10727 | **not yet** |
| What is selected: an outline and a faint grey over it, and the corners of its box (a fifth of each edge, white); "glow" (the default) or "thin" (`selection_highlight_style`) | GLCanvas3D.cpp l.2060–2740, `resources/shaders/140/selection_composite.fs`, Selection.cpp l.2779 | **added**: the "thin" style and the corners. **not yet**: the glow, and the lighter colours of a painted object that is selected |
| A mirrored copy is drawn inside out | 3DScene.cpp `is_left_handed` | **added** |
| "Show Labels" (Ctrl+E): each object's name over it, a thicker border when selected | GLCanvas3D.cpp l.1120 | **added**, as the page's own text. **differs**: placed by the object's top as it is seen; the C++ places a label without dividing by depth, so it drifts (listed with the errors found). **not yet**: "Sequence#" |
| "Show 3D Navigator": a cube that turns with the camera, "Front", "Back", "Top", "Bottom", "Left", "Right"; a click on a face goes to that view | `_render_3d_navigator` l.8189 (ImGuizmo) | **added**: a cube of the page's own with the six faces. **not yet**: its edges and corners, the axes' letters |
| A round "Fit in all view" button beside it | l.8300 | **added** |
| "View options": "X-Ray", "Realistic View", "Center of Mass"; wireframe; ambient occlusion and FXAA; shadows | l.8300–8445 | **not yet** |
| Contours where an object sinks under the bed | `SinkingContours` | **not yet** |

## 10. Preview: the 3D view

Read from `GCodeViewer.cpp/.hpp`, `LibVGCode/LibVGCodeWrapper.cpp`,
`libvgcode/` (all of it), `GUI_Preview.cpp`, `IMSlider.cpp`,
`libslic3r/GCode/GCodeProcessor.cpp` and the preview parts of
`GLCanvas3D.cpp`. Drawn by `ui/gl/toolpath.js` with the `segments` and
`options` shaders of `shaders.js`.

### Where the moves come from

| C++ | Where | Here |
| --- | --- | --- |
| The preview is the G-code read back: the processor turns the file just written into moves, each with what it is, its feature, width, height, speed, fan, temperature, filament and layer | `GCodeProcessor::process_G1` l.2900–3300, `Preview::load_print_as_fff` | **added**: `edge_engine::read_toolpath` reads the same comments (`;TYPE:`, `;WIDTH:`, `;HEIGHT:`, `;LAYER_CHANGE`, `;Z:`, `;WIPE_START`, `;WIPE_END`) and tells moves apart the same way. Before: the sliced lines of one layer, with nothing between them |
| A print that could not be written has no preview | — | **differs**: its sliced lines are shown all the same (`toolpath` says `source: "lines"`), without moves, marks, speeds or order; the reason there is no G-code is under "About this print" |
| A layer is the moves from its first line to the next layer's first line | `libvgcode/src/Layers.cpp` | **added** |
| A seam is where an outer wall's loop ends within 0.25 mm of where it began | GCodeProcessor.cpp l.3256 | **added**. Like the C++, a loop that starts as an overhang wall gets none |
| Arcs (G2, G3) | `process_G2_G3` | **added**: cut into half-millimetre moves. The engine writes none; a printer's start G-code may |
| Times, real speeds, acceleration and jerk of each move; pressure advance | the time estimator | **not yet**: `edge-time` plans the moves but does not hand a time for each back |

### How a line is drawn

| C++ | Where | Here |
| --- | --- | --- |
| Each move is a box with a rhombus for a cross-section, as wide and as high as the line laid; only the half facing the viewer is built (8 vertices, 8 triangles), one instance a move | `libvgcode/src/SegmentTemplate.cpp`, `Shaders.hpp` | **added**, the shader ported |
| A line's middle is half its height under the nozzle | ViewerImpl.cpp l.1253 | **added** |
| Each end has a tip: pointed where the path ends, turned to fill the outside of a bend where it goes on | Shaders.hpp `POINTY_CAPS` | **added**: the turn at each move is worked out by the engine and sent with it |
| Lit vertex by vertex: ambient 0.2, emission 0.15, top light 0.48, front light 0.12, gloss 0.15 | Shaders.hpp l.15–26 | **added**. The C++ has other numbers for OpenGL ES (ShadersES.hpp); the desktop ones are used |
| Moves kept in textures and looked up by number; a move's colour worked out on the processor and uploaded again when the view changes | ViewerImpl.cpp l.1246–1576 | **differs**: one buffer of 32 bytes a move, bound twice a move apart; the shader works the colour out, so changing the view, the range or what is shown uploads nothing |
| Travels and wipes are thin boxes, 0.1 mm | `DEFAULT_TRAVELS_RADIUS_MM`, `DEFAULT_WIPES_RADIUS_MM` | **added** |
| Marks (retractions, seams, tool changes) are two cones base to base, the line's width and height times 1.5 (0.75 on a Mac) | `OptionTemplate.cpp`, Shaders.hpp l.480 | **added**, the factor by platform as the C++ has it |
| Looking down, the last moves are drawn first | ViewerImpl.cpp l.2398 `reverse_order` | **not yet**: a saving, not a look |

### View types and colours

| C++ | Where | Here |
| --- | --- | --- |
| "Line Type": each feature its colour (inner wall (255, 230, 77), outer wall (255, 125, 56) and so on) | ViewerImpl.cpp l.302 `DEFAULT_EXTRUSION_ROLES_COLORS` | **had** in 2D; **added** in 3D, with "Custom" (94, 209, 148) for the lines of the printer's own start G-code |
| "Filament": each filament its colour | `ColorPrint` | **had** in 2D; **added** in 3D. A travel is its filament's colour, as in the C++ |
| "Layer Height", "Line Width": over the eleven range colours, least (11, 44, 122) to most (148, 38, 22), mixed between two and cut to whole numbers | `include/ColorRange.hpp`, `ColorRange::get_color_at` | **had** in 2D; **added** in 3D, the mixing ported |
| "Speed" ("Speed (mm/s)"), "Flow" ("Volumetric flow rate (mm³/s)"), "Fan Speed" ("Fan speed (%)"), "Temperature" ("Temperature (℃)") | GCodeViewer.cpp l.4362 | **added**, for a print read back from its G-code. Flow is the speed times what a line of that width and height takes |
| A range runs over the lines whose feature is shown (not the printer's own lines for height and width), values rounded to two digits; hiding a feature narrows it | `update_color_ranges` l.2261, `round_to_bin` | **added**, worked out by the page from the moves it holds |
| A range's legend: eleven values, greatest first (one or two when there are no more), to 2 decimals (0 for speed, fan and temperature) | `append_range` l.3948, `ColorRange::get_values` | **added**. Before: six values |
| "Actual Speed", "Acceleration", "Jerk", "Actual Flow", "Layer Time", "Layer Time (log)", "Pressure Advance" | l.1280 | **not yet**: need each move's time and planned speed |
| Travel (56, 72, 155), wipe (255, 255, 0), retraction (205, 34, 214), unretraction (73, 173, 207), seam (230, 230, 230), tool change (193, 190, 99) | ViewerImpl.cpp l.329 `DEFAULT_OPTIONS_COLORS` | **added** |
| Lower layers greyed or dimmed ("Dim lower layers in the G-code preview") | `set_layer_colors` l.1609 | not built, as in the C++: nothing there turns it on (listed with the errors found) |

### The legend

| C++ | Where | Here |
| --- | --- | --- |
| "Line Type" rows for the features in the print, in the order of their numbers; a click hides or shows | l.4312 | **had**; now from the moves, so "Custom" and "Overhang wall" are there when the print has them |
| Rows "Travel", "Wipe", "Retract", "Unretract", "Seams", "Filament changes" for what the print has, each to show or hide; only seams shown to start with | `append_option_item` l.4497, `libvgcode/src/Settings.hpp` | **added**, under "Options" in the Line Type and Filament views; "Travel" alone in the Speed view, as in the C++. What is shown is kept with the project (`view.shown`) |
| "Time", "%", "Usage" for each row; "Supported overhangs", "Unsupported overhangs" | l.4578 | **not yet** (section 3) |
| The "Filament" view lists each filament's use; it has no eyes | l.4741 | **differs**: each filament can be hidden or shown (**had**) |

### The sliders and what they show

| C++ | Where | Here |
| --- | --- | --- |
| The layers slider: upright at the right, two thumbs that bound the layers shown, so the layers below the upper thumb stay in sight; each thumb's layer number over its height beside it | IMSlider.cpp l.890–1230, `get_label` l.1756 | **added** (`controls.js` `LayerSlider`). Before: one thumb, one layer |
| A click takes up the nearer thumb; Up / Down move the one taken up (the upper to start with), five with Shift or Ctrl; the wheel over the slider likewise | GLCanvas3D.cpp l.5865, IMSlider.cpp l.1636 | **added** |
| "On/Off one layer mode of the vertical slider" [L], and its button | GUI_Preview.cpp l.538, IMSlider.cpp l.1286 | **added**. Leaving it shows every layer up to the thumb; the C++ goes back to the whole print |
| "Jump to Layer" [Shift+G] | `render_add_menu` l.1512 | **had**; moves the thumb taken up |
| The moves slider: along the bottom, how far through the layers shown the print has got; Left / Right step it (five with Shift or Ctrl), Home / End go to its ends; moving a layer thumb puts it back at the end | `update_moves_slider` l.2331, GLCanvas3D.cpp l.5865 | **added** (`MoveSlider`), one thumb, counted in moves. **not yet**: its lower thumb; one stop for each line of G-code (the C++ merges an arc's moves) |
| At the ends, Left / Right go on into the next layer | GLCanvas3D.cpp l.5880 | **not yet** |
| The nozzle while the print is stopped part of the way: the printer's own hotend model, else an arrow, white and half seen through, its tip half a millimetre over the move | GCodeViewer.cpp l.298–341 | **added**: the arrow. **not yet**: a vendor's hotend model |
| The position window: "X", "Y", "Z" in the axes' colours, "Speed: " and what the view shows ("Height: ", "Width: ", "Flow: ", "Fan: ", "Temperature: ", "Tool: "); unfolded, a table of everything about the move | l.393–791 | **added**: the two lines. **not yet**: the table |
| The objects under the print, seen through (0.2), in their filaments' colours | `load_shells` l.2752, `render_shells` l.3269 | **added**. Until a plate is sliced they are all that is shown of it |
| Only the plate shown is drawn, where it stands among the others; the axes move to it | GLCanvas3D.cpp l.3854, 3DBed.cpp `set_axes_mode` | **added**. **differs**: the camera goes with the plate, so going to another plate in Preview shows it where the last one was |
| The G-code window [C]; "Compare slices (A/B)"; ticks on the layers slider (pause, custom G-code, filament change) | l.877, IMSlider.cpp l.1300, l.1512 | **not yet** (section 3) |

## 11. How meshes and moves reach the page

They are too large for JSON. `Session::call_bytes` answers four calls in
bytes, little-endian, in layouts written out byte for byte in
`src/api/scene.rs`; the window carries them as a raw Tauri response
(`call_bytes` in `main.rs`) and `--serve` under `/bytes/<name>`.

- **An object's mesh is sent once**, in the object's own coordinates
  (`scene_mesh`: vertices, triangles by index, a mark for each triangle when
  they differ). Its copies are sixteen numbers each in `scene`. `scene.key`
  stays the same until the file is another, so a printer change, a plate
  change or coming back to a project sends no mesh again. The page gives each
  triangle its own three vertices with the face's normal (16 bytes a vertex)
  and hands them to the graphics card object by object, the plate shown first,
  drawing as each arrives.
- **Moves are sent a stretch at a time** (`toolpath_moves`, at most 262144
  moves, 8 MB), 32 bytes a move, and go to the graphics card as they are: the
  page reads none of them number by number. `toolpath.run` says which slice
  they are of. The moves of the last three plates looked at stay on the card.
- **Nothing is rebuilt to change what is seen**: the layers shown and the
  moves slider are a range of moves; the view type, the colour range and what
  is hidden are numbers given to the shader.
- **Not built**: coarser meshes for far-away objects. The largest project
  tried (1.36 million triangles) draws in about a millisecond.

Measured on the Mac mini (Apple M5 Pro, the app's browser pane, WebGL2 on
Metal through ANGLE, a 1062 × 860 stage, 60 Hz), through `--serve`, debug
build of the app with the engine optimised:

| | 20 mm cube (STL) | `3DBenchy.3mf` | `mcbrim_test.3mf`, three plates |
| --- | --- | --- | --- |
| Triangles, objects | 12, 1 | 225 154, 1 | 1 360 736, 16 |
| `scene` (JSON) | 0.5 kB | 0.5 kB | 6.2 kB |
| Meshes sent | 264 B | 4.06 MB | 46.3 MB |
| First object on screen after the project is shown | 6 ms | 0.2 s | 0.1 s (0.6 s the first time, the file being read) |
| All objects on screen | 7 ms | 0.2 s | 0.6 s (1.0 s the first time) |
| Meshes sent on coming back to the project | none | none | none |
| Moves of the plate sliced | 7 928 | 131 686 | 210 106 (plate 1) |
| Moves sent | 0.25 MB | 4.21 MB | 6.72 MB |
| On screen after `toolpath` is asked for | 10 ms | 0.09 s | 0.02–0.18 s |
| Frame while turning, Prepare / Preview | 16.7 ms (the screen's own) | 16.7 ms | 16.7 ms; drawn 16 times a frame, 19.8 ms / 16.7 ms: about 1.2 ms a draw in Prepare, under 1 ms in Preview |
