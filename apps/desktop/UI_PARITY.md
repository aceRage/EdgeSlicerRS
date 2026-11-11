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
  views, sections 9 to 11; fourth pass: the process pages, "Sidebar:
  Process" in section 2 and section 12; fifth pass: Prepare as an editor,
  section 13; sixth pass: the filament's and the printer's pages, section
  14; seventh pass: saving projects, the "Files" rows of section 2, the
  strip's saving in section 5, and section 15).
- **differs** — built, but on purpose not as the C++ does it; the cell says how.
- **not yet** — not built, with what it waits for. Nothing in this column is
  shown in the page as a control that does nothing.

Parity here is of function: what can be done, with which options, and in
which words. It is not of looks. The app is designed dark mode first, with
light kept working, in a modern web style that is streamlined and spare,
nearer the newer PrusaSlicer look than Bambu Studio's. A row's "Here"
column says what is offered, never that it is styled as the C++ styles it.
How it is styled (the tokens, the controls, the dos and don'ts) is in
`ui/STYLE.md`, and the whole page follows it: the project strip, the tab
bar, the sidebar, Printer Selection, the stage and what lies over it in
Prepare and Preview, Device and the dialogs. The restyle changed no
control, wording, call or key; the sidebar's sections now stand in the
order Printer, Filament Management, Objects, Process (the object list was
after the process pages), with each section's title, and the Process
section's head, kept in sight as the sidebar scrolls.

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
| Leaving Prepare disables undo and redo in the title bar | `init_tabpanel` page-changed handler | **added**: undo and redo are Prepare's (section 13); in Preview and Device their buttons are not shown and their keys do nothing |
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
| The printer row: a framed panel (green frame on hover), a 48 px picture of the printer, the preset list; a click anywhere on it opens the list | l.2637–2705 | **added**: the picture, the model's name and the build volume as the head of one card for the printer; a click anywhere on it opens the list |
| The picture: `<printer_model>_cover.png` from the vendor's profiles folder (Snapmaker's from `resources/images` by the preset's name), "printer_placeholder" when there is none | `Sidebar::update_printer_thumbnail` l.12170 | **added**: the cover from the profiles folder for every vendor (`printer_cover`), a grey box when there is none. A project on its own settings shows its printer's picture when that model is among those added |
| The list holds every visible printer preset, in groups with these titles, in this order: "Project-inside presets", "User presets", "System presets". Each group is in order of name (`std::map`: by code unit, capitals first). A system printer is listed once for each `printer_model`, under the model's name, whatever its nozzles | `PlaterPresetComboBox::update`, PresetComboBoxes.cpp l.1528–1749 | **added**: one list (`printers`), the same groups, titles and order; "System presets" has a row for each model added, by model name. Before: "Vendor" and "Printer", listing every preset of every vendor. **not yet**: "User presets" (there are no user presets; the group is left out while it is empty, as in the C++) |
| "Project-inside presets": the presets a project file carries that are not the vendor's own | l.1656–1663, l.1722–1730 | **differs**: the row is the printer the project was saved with (`printer_settings_id`) and stands for the project's own settings as a whole, which is how this engine slices a project. It replaces the first pass's "Sliced with: the project's own settings / presets chosen here". Picking it goes back to those settings, the project's own bed type among them; picking a row of "System presets" slices with presets |
| A preset is listed only if visible; a system printer is visible only when its vendor, model and nozzle are among the printers added (`AppConfig` "models"); with one nozzle of a model added, all of them are | `Preset::set_visible_from_appconfig` Preset.cpp l.1060, `PresetBundle::load_installed_printers` l.2127, `install_missing_variants_for_enabled_models` l.2100 | **added** (section 8). A printer a project is on that is not among those added is listed while it is in force, as the C++ makes a preset visible when it is selected (Plater.cpp l.18619) |
| The row shown as selected; the list's tooltip is the preset's own name; a modified preset has "* " before its name; typing filters the list | l.1595–1599, l.1769–1778, `Preset::label` Preset.cpp l.899 | **added**: the selection and the tooltip. Nothing modifies a preset here, so there is no mark. Typing jumps to a row, as a list of the system's does. The list is the page's own (`listbox` in `app.js`), so that it can show the groups, each printer's picture and the entry at its foot; its keys are those of a system list (arrows, Home, End, typing, Enter, Escape) |
| Two vendors with a model of the same name are one row (the group is keyed by name) | l.1636–1641 | **differs**: a row each, the vendor in the row's tooltip. One row that picks whichever preset comes first is not what a list of printers is for |
| Picking a model: the preset of that model for the nozzle the project is on, else for 0.4, else the first by name; when the model changes, the nozzles last used on its rack | `Plater::priv::on_select_preset` l.18536–18648, `PresetBundle::get_similar_printer_preset` l.3141, `printer_preset_for_remembered_nozzles` l.4047 | **added** (`printer_select`). **not yet**: the rack's remembered nozzles (the engine has no rack) |
| At the foot of the list: "Select/Remove printers (system presets)" opens Printer Selection; "Create printer" opens "Create Printer/Nozzle". Picked, they leave the list on the printer it was on | l.1751–1762, `OnSelect` l.1188–1222, Plater.cpp l.18547 | **added**: "Select/Remove printers (system presets)". **not yet**: "Create printer" (section 8); it is not listed |
| In the list's own row: the edit icon (opens the printer's settings), "Connect to printer" (opens "Devices"), "The machine has been connected and is currently in working mode" | Plater.cpp l.2659–2686, PresetComboBoxes.cpp l.1907–1985 | **added**: the edit icon at the right of the printer card's head, "Click to edit preset", opens the printer's pages (section 14). **not yet**: "Connect to printer" and the working-mode mark, which need a printer connection (section 4) |
| Leaving a printer or filament preset with unsaved changes asks first ("Unsaved Changes") | `Tab::select_preset` Tab.cpp l.9037–9105 | not needed: presets are not edited |
| **Another printer, the process**: the one last used with that printer ("Remember printer configuration", on unless turned off); else the one of the same name (alias: the name up to "@") cut for it; else the printer's `default_print_profile`; else the first that fits | `PresetBundle::update_selections` l.2432, `update_compatible` l.7171 (`PreferedPrintProfileMatch` l.7213), Tab.cpp l.9196–9306 | **added** (`printer_select`), in that order, a process that still fits staying before the search by name. **not yet**: the profile remembered for each layer height, the preference itself |
| **Another printer, the process settings changed by hand** are carried over: "Kept %1% process settings from \"%2%\" (revertable)."; each carried value stays marked as changed and can be reverted | `capture_print_settings_carry`, `apply_print_settings_carry` Tab.cpp l.9096, l.9314 | **added**: every process setting set over the preset stays set, with the same sentence in the status line (one setting: "Kept 1 process setting from …"), and each is given back with its own reset or all with the reset in the Process title bar. **differs**: said for a change of process preset too, where the C++ asks ("Unsaved Changes": transfer, discard, save); line widths and layer heights are kept across nozzle sizes, where the C++ lets them "follow the new nozzle size" |
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
| — | — | The build volume ("270 × 270 × 270 mm") is this app's own line (**had**), now under the printer's name |

### Sidebar: Filament

| C++ | Where | Here |
| --- | --- | --- |
| Section "Filament Management" ("Pellet Configuration" for pellet printers), sub-title "Filaments" | l.2882, l.3515, l.4450 | **added** |
| One row for each filament of the project: a numbered colour swatch ("Click to select filament color"), the filament preset list, the tool badge, the row menu ("Edit", "Delete", "Merge with", "Print on nozzle", "Rack slot"…) | `init_filament_combo` l.4135, `GUI_Factories.cpp::create_filament_action_menu` l.2600 | **added**: a row for filament 1 up to the highest the file prints in, with its number on the project's colour and a preset list each (before: one list for each filament the plate shown used, no colours). With the project's own settings the saved names are shown. **added** (sixth pass): the row's edit icon ("Click to edit preset", the row menu's "Edit"), which opens that filament's pages (section 14). **not yet**: changing a colour (the colour picker), the tool badge, the rest of the row menu |
| "Add one filament", "Remove last filament" | l.3508–3600 | **not yet**: the engine prints the filaments the file assigns |
| "Apply All" (one preset for every filament) | l.3624 | **not yet** |
| "Flushing volumes", "Color Mixing Match", "Synchronize filament list from AMS", "Sync Filament Information", "Spool Manager", "Manage filament presets" | l.2903, l.2936, l.3404, l.3508 | **not yet** |
| "Mixed Filaments": "Add Gradient", "Add Pattern", "Add Color" | l.3794 | **not yet** |

### Sidebar: Process

The pages are drawn from data, not written: `src/api/settings_pages.json`
(its `process` part), made from the pinned source by
`tools/make_settings_pages.py` (Tab.cpp
`TabPrint::build` for the pages, groups and lines; `libslic3r/PrintConfig.cpp`
for each option; the defaults table of `tools/make_defaults.py`). After a pin
bump: `tools/make_defaults.py <EdgeSlicer executable>`, then
`tools/make_settings_pages.py`. Section 12 has the numbers and what the engine
does with each option.

| C++ | Where | Here |
| --- | --- | --- |
| Section "Process" with the process preset list | `ParamsPanel::ParamsPanel` l.210, `Tab.cpp` l.300 | **added** as its own section (first pass) |
| Pages "Quality", "Strength", "Speed", "Support", "Multimaterial", "Others", each a page of option groups; a row of tabs picks the page | `TabPrint::build`, Tab.cpp l.3441 | **added**: the six pages with every group and every option in the C++ order, 433 options. A page's controls are built when it is first opened. Before: four settings ("Layer height", "Initial layer height", "Slice gap closing radius", "Wall loops") |
| An option's label, its tooltip with "parameter name: key" under it, its unit after the field | `libslic3r/PrintConfig.cpp`, Field.cpp `get_formatted_tooltip_text` l.109 | **added**, the C++ strings. **differs**: a length that may be a percentage ("mm or %") shows "mm" after a plain number and nothing after one typed with its %. **not yet**: the wiki link behind a label (the page loads nothing from the network) |
| Controls by kind: a tick box (`coBool`), a number with its unit (`coInt`, `coFloat`, `coPercent`), a length or a percentage (`coFloatOrPercent`), a list (`coEnum`), a list that also takes a typed value (`i_enum_open`, `f_enum_open`: the infill anchors, the support interface layers), text, a box of text for G-code, scripts and notes | Field.cpp, OptionsGroup.cpp | **added**: the controls of `ui/STYLE.md`; a list is the page's own listbox; a list that takes a typed value is a field with a chevron that opens the values as a menu |
| One line with several options under one label: "Overhang speed" with its four ranges, "Bridge" with external and internal | Tab.cpp l.3703–3714 | **differs**: the line's label and tooltip, then a row for each option under it; four fields do not fit beside a label in a sidebar |
| A value for each nozzle flow variant (the speeds, accelerations and jerks: `coFloats`), picked by the variant selector at the top of the tab | `register_flow_variant_view`, Tab.cpp l.3670 | **differs**: one field, showing the first variant's value and setting every variant alike. **not yet**: a value for each variant (the engine reads the first) |
| A changed option is marked (its label orange) with an arrow that puts it back: "Click to drop current modify and reset to saved value." | Tab.cpp l.10523, `OG_CustomCtrl` | **added**: a bar of the accent on the control, the label in bold, and the arrow with the same tooltip. Here a change is the project's own over the preset (`process_set`), not an edit to the preset: the preset's value is what the arrow goes back to and what an emptied field shows in grey |
| How many options are changed | the preset's "* " mark, the tab's orange title | **added**: a count on each page's tab ("2 settings changed") |
| "Click to reset all settings to the last saved preset." | Tab.cpp l.2229 | **added**: the arrow in the Process title bar, there while anything is changed |
| A change marks the project modified and drops the slices | `Tab::on_value_change`, `Plater::on_config_change` | **added** (**had** for the four) |
| A value out of range or not a number: "Invalid numeric.", "Value is out of range.", "Value %s is out of range. The valid range is from %d to %d.", "%s can't be a percentage"; the field goes back | Field.cpp l.386–498 | **added**, said in the status line before anything is sent, with the limit; the session holds to the same limits (`process_set`). **differs**: a length over `max_literal` is refused with its range, where the C++ asks whether a percentage was meant. **added** beyond the C++: a value in range that the engine would stop at (a 5 mm layer on a 0.4 nozzle) is refused with the engine's own reason, found by asking it |
| Options shown, hidden and greyed out by what the others are set to | `ConfigManipulation::toggle_print_fff_options` l.757–1441 | **added**: the whole function, every page's rules (`src/api/process/rules.rs`, in the C++ order; `toggle_line` hides, `toggle_field` greys out). The printer's side of them (G-code flavor, Bambu or not, tower type, extruder variants) is read from the settings a slice gets; the number of filaments is the project's. **not yet**: the values the C++ changes by itself while toggling (arc fitting off under extrusion rate smoothing, a multi-line fill back to one line, the reverse threshold to 0%); "Brim ear radius" as the label of "Brim width" with brim ears |
| Settings that need one another: "Spiral mode only works when…", "Change these settings automatically?" and the rest | `ConfigManipulation::update_print_fff_config` l.150–756 | **not yet**: the questions; a value is taken as it is set |
| "Advance" switch: off, the simple options; on, the advanced ones too; a page with nothing to show in the mode is not among the tabs. Developer options with "Develop mode" (Preferences) | ParamsPanel.cpp l.229, `ConfigOptionMode`, GUI_App.cpp l.8520, `Tab::update_visibility` | **added**: the switch, off to start with as in the C++, kept by the page; 54 options without it (no Speed page), 428 with. **not yet**: develop mode, so the 5 developer options are not shown |
| Search ("Search in preset"): finds options by label, jumps to the page and the option | Tab.cpp l.382–420, `Search.cpp` | **added**: every word typed must be in the label, the name (`seam_slope_type`), the group or the page; the list shows label, page › group and name; Enter or a click opens the page, goes to the row and marks it for a moment; Escape clears. An option the rules hide is listed greyed, and its group is gone to |
| What the slicer does with an option | — | this app's own (**added**): a quiet ring beside an option the engine does not read with the settings as they are, a dot beside one it follows with a note, each said in the label's tooltip; nothing is hidden for it and every option can still be set (section 12) |
| The support sets row ("Support set": apply, save, delete) | Tab.cpp l.3776, `support_set_create_widget` | **not yet**: it writes files of user presets |
| "Global" / "Objects" switch, per-object, per-part and per-layer settings, "View all object's settings"; the tool selector at the top of the tab ([Plate] [T1 0.4] …) | `ParamsPanel::create_layout` l.364, `TabPrintModel` l.5035, `create_tool_selector` | **not yet**: the engine reads one set of settings for a plate |
| "Compare presets", save ("Save current …") and delete preset, favourites, "Manage process presets" | ParamsPanel.cpp l.238, Tab.cpp l.300–420 | **not yet**: there are no user presets (`edge-config` reads vendor profiles only); what is changed is kept with the project while it is open |
| The filament's and the printer's own pages of settings | `TabFilament::build`, `TabPrinter::build` | **added**: section 14, from the same tool, drawn by the same code |

### The bed

| C++ | Where | Here |
| --- | --- | --- |
| A 3D view of every plate with the objects on it, orbit / pan / zoom, selection | `View3D`, `GLCanvas3D` | **added**, in 3D: section 9. Where the browser has no WebGL2, the view from above the first pass built is kept: the bed of the printer chosen with its grid and each object's outline (its convex hull), name and size, drag to move, wheel to zoom, "Fit part" [Z] and "Fit bed" |
| Main toolbar: "Add" [Ctrl+I], "Add plate", "Auto orient all/selected objects" [Q], "Arrange all objects" [A] with its options, "Split to objects", "Split to parts", "Variable layer height", "Assembly View" | `GLCanvas3D::_init_main_toolbar` l.9274 | **added**: "Add", "Add plate" and "Arrange all objects" (section 13; the arrangement is a plain one, not the reference's). **not yet**: auto orient, splitting (the engine does neither), variable layer height, the assembly view, the arrangement's options |
| Gizmos: "Move" [M], "Rotate" [R], "Scale" [S], "Lay on face" [F], "Cut" [C], "Mesh Boolean" [B], "Support Painting" [L], "Seam painting" [P], "Paint-on fuzzy skin" [H], "Color Painting" [N], "Texture displacement", "Emboss" [T], "SVG", "Measure" [U], "Assemble" [Y], "Simplify", "Brim Ears" [E], "Sculpt", "Edit", "Fillet / chamfer (CAD)" | `Gizmos/GLGizmosManager.cpp::init` l.224 | **added**: "Move", "Rotate" and "Scale" with their number fields (section 13). **not yet**: the rest; "Lay on face" waits for the convex hull of a mesh in three dimensions, which nothing here works out |
| Object list: plates, objects, parts, instances, layer ranges; columns "Name", printable, "Fila.", paint marks; the object, part and plate menus | `GUI_ObjectList.cpp`, `GUI_Factories.cpp` | **added**: plates, objects and instances with "Name", printable and "Fila.", and the object's and the plate's menus (section 13). **not yet**: parts, layer ranges, the paint marks, settings of an object |
| View options: "Realistic View", "X-Ray", "Center of Mass", "Show 3D Navigator", Ctrl+0..6 views | `GLCanvas3D.cpp` l.8297, View menu | **added**: the navigator and the views (section 9). **not yet**: the three view options |

### Plates

| C++ | Where | Here |
| --- | --- | --- |
| Every plate shown on the canvas; a click selects it | `PartPlateList`, `Plater::select_plate_by_hover_id` l.30481 | **added**: in the 3D scene, every plate where it stands, a click selecting it (section 9); and the column of plate numbers of the first pass (two plates or more), in Prepare and in Preview |
| Per plate: delete, orient, arrange, lock, "Customize current plate" (bed type, print sequence, spiral vase, filament sequences), name, "Move plate to the front", "Print history" | `PartPlate::render_icons` l.1173, `PlateSettingsDialog.cpp` | **added**: delete and arrange (section 13). **not yet**: the rest. A plate's name is shown in its tooltip; its first-layer filament order is followed by the engine |
| "Add plate", "Duplicate Current Plate", the plate menu ("Select All", "Delete All", "Arrange", "Reload All", "Auto Rotate", "Delete Plate") | toolbar, `create_plate_menu` l.2831 | **added**: "Add plate", and of the menu "Select All", "Delete All", "Arrange" and "Delete Plate" (section 13). **not yet**: "Duplicate Current Plate", "Reload All", "Auto Rotate" |

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
| "Save Project" [Ctrl+S] ("Save current project to file"), "Save Project as..." [Ctrl+Shift+S] ("Save current project as"): to the project file it was opened from, else (an STL, a new project, a project opened from a ZIP) through the save dialog, "Save file as:", "3MF files", "Untitled.3mf"; written as `SplitModel \| ShareMesh \| FullPathSources`; the project is then that file and saved; "Failed to save the project.\nPlease check whether the folder exists online or if other programs open the project file." | MainFrame.cpp l.3444–3458, Plater.cpp `save_project` l.21961, `get_export_file` l.15728 | **added** (`project_save`, `ui/save.js`): the keys and a project tile's "Save" / "Save as..." (below, section 5), the same rules and words; the file is the reference's project file (section 15). An STL opened as a project is saved as a 3MF beside it, its name offered. **differs**: no menu bar was added: the keys and the tile's menu are where the C++ has them too, and a File menu of two entries would be a bar for little; in `--serve` the dialog is the page's own (a path is typed). What the file held that the save would not keep is listed and asked about first (this app's own question, the reference keeping all it reads) |
| "Import 3MF/STL/STEP/SVG/OBJ/AMF/GLB..." [Ctrl+I] adds geometry to the project shown; dropping files on the window; "Open as project" / "Import geometry only" | l.3385, `PlaterDropTarget::OnDropFiles` l.13122 | **added**: "Add" [Ctrl+I] puts an STL's or a 3MF's geometry in the project shown, a 3MF always as "Import geometry only" (section 13). The strip's Open still opens a file as a project of its own, an STL as much as a 3MF (**had**). **not yet**: the other formats, dropping files, the question which of the two is meant |
| Export: STL, STEP, generic and Bambu 3MF, preset bundles | the same | **not yet**. `edge save` (the command line) writes the same project file as "Save Project" |

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
| Each project is live: its own plater, with its model, plate, presets, slice, Preview, selection, camera and undo history. A switch saves and loads nothing; a slice running in a project out of sight goes on | ProjectStrip.hpp (P4), `switch_to`, `stash_active`, `show_slot` | **added**: `Session` holds every open project with its file, plate, choices of presets and settings, the slice of each plate, the tab it was on and where its Preview was left (`view`). Each slice runs on a thread of its own, so another project is looked at, or sliced, meanwhile. The page keeps each project's camera and what is selected in it; each project has its own undo history in the session (section 13) |
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
| An orange dot (top left) marks unsaved changes | `on_poll` (`is_project_dirty`), `on_paint` | **added**: the dot marks a project whose model is not the one it was opened or last saved with (an edit taken back by undo is no change), or whose choices (presets in use, bed type, the settings set here) are no longer those it was opened or saved with. Saving clears it |
| A badge (bottom right): going round while the project slices, a tick once every plate that holds objects is sliced; also for projects out of sight | `slice_badge`, `on_poll` (every 500 ms) | **added**, from `projects`, asked for twice a second (more often while something slices) |
| Tooltip: the name, the file, "Unsaved changes", "Slicing" | `tooltip_for` | **added** |
| A cross (top right, on hover) and the middle mouse button close a project ("Close project") | `hit_test`, `on_left_down`, `on_middle_down` | **added** |
| Closing asks first: with unsaved changes, "\"%s\" has unsaved changes. Save them before closing it?" (Yes / No / Cancel); while slicing, "\"%s\" is slicing. Stop slicing and close it?" | `close_project` | **added** (`project_close`, which answers `ask` until told `discard` / `stop`), the three answers: Yes saves (asking where for a project with no project file; a save called off keeps it open) |
| Closing the project shown shows the next one, else the one before; closing one out of sight changes nothing on screen; closing the last one leaves a new empty "Untitled", so the strip always has one | `successor_on_close`, `close_project` | **added** |
| A tile dragged up or down moves to where the line shows | `on_motion`, `drop_index_at`, `ProjectSessions::move` | **added** (`project_move`) |
| More tiles than fit: the wheel scrolls the strip | `on_wheel` | **added** |
| Tile menu: "Save", "Save as...", "Send...", "Close", "Close others", "Reveal in Finder" / "Show in Explorer", "Open in new window" | `show_item_menu` | **added**: "Save", "Save as...", "Close", "Close others". **not yet**: sending, showing the file, a second window |
| Strip menu: "Show thumbnails" / "Show names only", "Default width", "Collapse the project strip" / "Show the project strip" | `show_strip_menu` | **added** |
| The right edge is dragged to widen the strip: 88 to 200, names alone at least 120; remembered | `StripLayout`, `on_motion`, `project_strip_width` | **added**, remembered by the page |
| The chevron folds the strip to a 24 wide rail: New and Open stacked, a mark for each project (filled for the active one, a ring when sliced, the unsaved dot); remembered | `set_collapsed`, `on_paint` | **added** |
| Closing the window with several projects open: one dialog, "Unsaved projects", "These projects have unsaved changes. Save the ticked ones before closing?", every unsaved one ticked, "Save" / "Don't save" / "Cancel"; with one project, "EdgeSlicer - Save": "The current project has unsaved changes, save it before continue?" (Yes / No / Cancel) | `confirm_close_all`, MainFrame.cpp l.604, Plater.cpp `close_with_confirm` l.25050 | **added** in the window: it is held open while a project is unsaved (`unsaved`), the page asks in these words ("EdgeSlicerRS - Save"), saves what is ticked and then closes it. **not yet**: "Remember my choice."; in a browser (`--serve`) nothing is asked, the session outliving the page |
| "Reopen the open projects at the next start" (off unless asked for): the projects written out at exit and read back | `save_session_for_restart`, `restore_session`, `SessionManifest` | **not yet**: projects can be written now (section 15); the preference and the manifest are not built. Reloading the page (`--serve`) does find every project as it was, the session being the server's |
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
| Ctrl+S "Save Project", Ctrl+Shift+S "Save Project as..." | **added** (and Cmd on a Mac), anywhere in the page but in a dialog; in a browser they are taken from the browser's own |
| Ctrl+G, Ctrl+Shift+G, Ctrl+P (Preferences), "?" (the list of shortcuts) | **not yet** (Ctrl+I is "Add", section 13) |
| Left / Right, Home / End (the move slider), L (one layer mode), C (G-code window) | **added** in 3D: all but C (no G-code window) |
| A, Shift+A, Q, Shift+Q, the gizmo keys, Ctrl+0..6, Ctrl+A / D / Z / Y / C / V / X, Del, 1–9, arrows to move objects | **added** in 3D: Ctrl+0..7 (the views), Ctrl+A, Esc, I and O, Ctrl+E (section 9); A, Shift+A, M, R, S, Ctrl+Z, Ctrl+Y, Ctrl+C, Ctrl+V, Ctrl+K, Ctrl+I, Del and the arrows (section 13). **not yet**: Q and Shift+Q, the other gizmos' keys, Ctrl+D, Ctrl+X, 1–9 (the filament of what is selected) |

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
| `projects` | — | the strip: `{ active, max_open, projects: [{ id, name, path, dirty, empty, plate, tab, revision, slicing, plates_with_objects, plates_sliced }] }`. `revision` counts the edits to the project's model |
| `project` | `project?` | a strip entry and `file`, `plates: [{ id, name, filaments, objects, sliced, failed }]`, `sliced`, `failed`, `has_settings`, `used`, `filament_count`, `colours`, `saved { printer, process, filaments }`, `choices { source, vendor, machine, process, filaments, bed_type, over { key: value }, filament_over [{ key: value }], printer_over { key or key#n: value }, layer_height, first_layer_height, wall_loops, closing_radius }` (`over` is every process setting set over the preset, `filament_over` each filament's own from filament 1, `printer_over` the printer's; the four after them are four process settings under their older names), `view`, `edit { revision, undo, redo }` (what there is to take back and to make again, by name, or null), `status`. All of it is of the model as it stands, edits included |
| `project_new` | — | the strip and `id`; or the strip and `full: true` |
| `project_open` | `path` | the strip and `id` (`already: true` when it was open); or `full: true` |
| `project_select` | `id` | the strip |
| `project_cycle` | `step` (1 or -1) | the strip |
| `project_move` | `id`, `index` | the strip |
| `project_close` | `id`, `discard?`, `stop?` | the strip and `closed: true`; or `closed: false`, `ask: "unsaved" \| "slicing"`, `name` |
| `project_save` | `project?`, `path?`, `accept?`, `suggest?` | saves the project as a project file (section 15): to `path`, else to the 3MF it was opened from (`.3mf` is added to a path without it). The strip and `saved: true`, `path`; the project then goes by that file and is saved. Or the strip and `saved: false` with `ask: "path"`, `name`, `suggested` (it has no project file: ask where, offering that path), or with `ask: "losses"`, `name`, `losses` (sentences: what its file held that would not be kept; nothing was written; `accept: true` writes it all the same). With `suggest: true` only `{ suggested }`, for "Save as..." |
| `unsaved` | — | `{ unsaved: [{ id, name, path }], projects }`: the projects with unsaved changes, for the question asked before the window closes |
| `project_set` (what is set for `bed_type`, `process` or `filaments` is kept with the printer the project is on) | `project?` and any of `plate`, `tab` (`prepare` / `preview`), `view` (a small object), `source` (`project` / `presets`), `vendor`, `machine`, `process`, `filaments`, `bed_type`, `layer_height`, `first_layer_height`, `wall_loops`, `closing_radius` (null to clear), `initial` (these are what it starts with) | as `project`. A change to what it is sliced with drops its slices and stops one that is running |
| `process_pages` | — | the process pages as the reference has them: `{ pages: [{ title, groups: [{ title, lines: [{ keys, label?, tooltip? }] }] }], options: { key: { label, tooltip, type (bool, int, float, percent, float_or_percent, enum, string, strings), unit?, min?, max?, max_literal?, values?, labels?, open?, mode (simple, advanced, developer), list?, multiline?, code?, full_width?, height?, default } } }`. The same for every project; asked for once |
| `process` | `project?` | `{ project, source, name, values { key: value } (by the preset, or the project's own settings, or the default), set { key: value } (what is set over them), counts { page: number set }, changed, hidden: [key], off: [key] (the reference's rules), unread: [key] (what a slice with these settings does not read), notes { key: [sentence] } (the engine's notes), refused (why the engine would stop, or null) }` |
| `process_set` | `project?` and `key` with `value` (null gives it back to the preset), or `reset: true` (gives all back) | as `process`. An error, with nothing changed, for a value the option does not take or the engine would stop at. A change drops the project's slices |
| `settings_pages` | `kind` (`process`, `filament`, `printer`) | as `process_pages`, for that kind. A printer's page with `per: "extruder"` ("Extruder %d") stands for one page for each extruder; a page with `flavors` is shown for those G-code flavors only. An option may also have `vector` (a value for each filament or extruder), `modes` (a value for the normal mode, then the silent), `override` (`printer` or `process`) with `of` (a filament's own value of that option, "nil" while off), `widget` (set in a dialog of the reference's own), `own` (the tab's own, `extruders_count`); a line may have `full` (the row is named with the option's `full_label`) and `labels` (a name for an option on that line) |
| `settings` | `project?`, `kind`, `filament` (from 1; for `filament`) | as `process`, and for a filament or the printer: `filament`, `filaments`, `extruders`, `pages` (the titles as they stand, "Extruder 1" …), `away` (pages not shown for this G-code flavor), `labels` (a line's name by the rules, by its first option), `fallback` (what an override that is off stands for), `fixed` (why an option is not set here), `gcode_flavor`. An extruder's own option is named `key#n`, n from 0, in every list |
| `settings_set` | `project?`, `kind`, `filament?`, and `key` (or `key#n`) with `value`, or `reset: true` (that kind's, and that filament's, all back) | as `settings`. Refused with the reason, nothing changed: a value the option does not take; "nil" for what is not an override; an extruder or a filament the project has not got; an option set in a dialog of the reference's own, or that is the preset's and not a filament's; a value the engine would stop at with the block printed in that filament |
| `settled` | `project?` | `{ printer: { source, name, vendor, model, variant, nozzle_diameter, bed, height, gcode_flavor, host_type, print_host, print_host_webui, has_api_key, bed_types { enabled, in_force, choices } }, process: { name, layer_height, first_layer_height, wall_loops, closing_radius } }`, either null when nothing is chosen |
| `plate_view` | `project?`, `plate?` | `{ project, plate, bed, height, objects: [{ name, outline, box, height }] }` |
| `slice` | `project?`, `plate?` or `all: true` | `{ project, run }`; `status` says how it goes. The model is sliced as it stands, not the file. A plate with an object over its edge is refused in the reference's words |
| `cancel` | `project?` | — |
| `status` | `project?` | `{ project, run, state, label, step, steps, plate, at, of, error }`, `state` being idle, slicing, done, failed or cancelled |
| `summary` | `project?`, `plate?` | as before, and `plate`, `widths`, `heights`. `gaps` ends with each setting set by hand that the slice did not read |
| `layer` | `project?`, `plate?`, `index` | as before |
| `export` | `project?`, `plate?`, `path` | `{ path, bytes }` |
| `scene` | `project?` | the project for Prepare in 3D: `{ project, key, bare, bed { box, shape, height, model, texture }, plates: [{ id, name, origin }], plate, colours, objects: [{ id, name, mesh, vertices, triangles, bytes, filaments, filament, parts }], placings: [{ id, object, plate, printable, matrix (16 numbers, column by column), box, over }] }`. An object's `id` and a copy's (a placing's) `id` stay theirs through every edit; `mesh` is the number of the object's mesh, the same for as long as the mesh is; `filament` is the object's own when its parts agree; `over` says a copy reaches over the edge of its plate |
| `scene_edit` | `project?`, `op` and what it takes (section 13) | `{ project, scene, done, made, note }`: the project and its scene as they then are, the edit's name in the history, the copies it made, and something to tell the person or null |
| `undo`, `redo` | `project?` | `{ project, scene, done }`; an error when there is nothing to take back or to make again |
| `toolpath` | `project?`, `plate?` | a sliced plate's moves, before they are asked for: `{ source: "gcode" \| "lines", moves, move_size, at_once, layers { first, z, height }, features, filaments, counts { travel, line, wipe, retract, unretract, seam, tool_change }, box, plate, run }` |
| `scene_mesh` (bytes) | `project?`, `mesh` (its number in `scene`) or `object` (its place in the list) | an object's mesh, `EDGM`: 24 bytes of header, vertices (`f32` × 3), triangles (`u32` × 3), a mark byte for each triangle when they differ |
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
| "Select/Remove printers (system presets)" at the foot of the printer list, the gear in the Printer title bar, and "Add/Remove presets" in the preset cog's menu all open the guide on its printer page alone (`BBL_MODELS_ONLY`, page `24`): a window of 1000 × 720 that can be resized, no title | PresetComboBoxes.cpp l.1207, l.1515, Plater.cpp l.2590, `GUI_App::run_wizard` l.9592–9643, WebGuideDialog.cpp l.227–280 | **added**: the list's entry and the gear open "Printer Selection", a dialog of 1120 × 780 at the most, less in a smaller window (all of a window narrower than 760). **not yet**: resizing it; the cog's menu (there is no cog) |
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
| Left button dragged: "Rotate View", 0.8° a pixel, about the middle of what is selected, else of the objects on the plate shown, else of the plate; with Ctrl or Cmd about the point of the bed in the middle of the view | `on_mouse` l.6642–6716, `TRACKBALLSIZE` | **added**. A drag that starts on an object moves the object, as in the C++ (section 13) |
| Right or middle button dragged: "Pan View" | l.6718–6747 | **added**. **differs**: what is under the pointer on the plane through the target stays under it. The C++ measures the move on the near plane, so in perspective the scene lags the pointer (listed with the errors found) |
| Mouse wheel: "Zoom View"; "Zoom to mouse position" (off), "Reverse mouse zoom" (off) | `on_mouse_wheel` l.6007 | **added**, both in the "View" menu. **differs**: zooming to the pointer starts on, as the view from above did |
| "Orbit speed multiplier", "Swap pan and rotate mouse buttons", "Camera style": "Touchpad" (Alt to turn, Shift to move) | Preferences.cpp l.1998–2029 | **not yet**: no preferences |
| Pinch to zoom, two fingers to move and to turn | l.6136–6169 | **added**: the pinch (a browser sends it as a wheel). **not yet**: the other two |
| A click on an object selects it; Ctrl adds it to the selection or takes it away; a click on nothing lets go of everything (not with Shift held); a right click selects | l.6423–6551, l.6749–6858 | **added** |
| A click on a plate selects the plate and lets go of the objects | Plater.cpp l.30481 | **added** |
| Alt: "Select a part"; Shift: "Select objects by rectangle"; a double click opens the object's settings; the right button's menus | l.6423–6551, l.6871 | **added**: the right button's menu on an object and on a plate (section 13). **differs**: Shift with a click adds to what is selected, as Ctrl does. **not yet**: parts are not told apart, the rectangle, object settings |
| Dragging an object moves it on the bed | l.6553–6608 | **added** (section 13) |
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
| Arrows, PgUp / PgDn, Del, Ctrl+C / V / X / Z / Y / D / K, 1–9, A, Q, the gizmo keys, "?" | **added**: the arrows, Del, Ctrl+C / V / Z / Y / K, A, and M, R, S of the gizmo keys (section 13). **not yet**: PgUp / PgDn (turning what is selected), Ctrl+X, Ctrl+D, 1–9, Q, the other gizmos, "?" |

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
| Icons beside each plate: "Remove current plate (if not last one)", "Auto orient objects on current plate", "Arrange objects on current plate", "Lock current plate", "Customize current plate", "Move plate to the front", "Print history", "Edit current plate name" | `render_icons` l.1154 | **differs**: removing a plate and arranging it are in the object list (the cross at a plate's row) and in the plate's menu, not drawn beside the plate (section 13). **not yet**: the rest |
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
  they differ). Its copies are sixteen numbers each in `scene`. A mesh is
  kept by the page under the project's `key` and the mesh's own number
  (`objects[].mesh`), which stays the same whatever is done to the object's
  copies, so a printer change, a plate change, coming back to a project, or
  an edit that moves, clones or takes away copies sends no mesh again. An
  object given another filament has another mesh, its colours being part of
  it; a clone has its original's. The page gives each
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

## 12. The process pages: the options, and what the engine does with them

### Where the pages come from

`tools/make_settings_pages.py` reads three things and runs nothing (the
process's pages here; the filament's and the printer's in section 14):

- `TabPrint::build` (Tab.cpp l.3441–4032) for the pages, the groups and the
  lines in order: `add_options_page`, `new_optgroup`,
  `append_single_option_line("key")`, an `Option` shaped before it is
  appended (`is_code`, `multiline`, `full_width`, `height`), and a `Line` of
  several options with its own label and tooltip. Commented-out lines are not
  read. A group that holds only a widget of the C++'s own (the support sets)
  has no options and is left out.
- `PrintConfigDef` (PrintConfig.cpp) for each option: `label`, `full_label`,
  `tooltip`, `sidetext`, `min`, `max`, `max_literal`, `mode` (simple unless
  set), `enum_values` and `enum_labels` (pushed, inserted at the head, or
  copied from another option), `gui_type` (a list that takes a typed value),
  `ratio_over`, `multiline`, `readonly`.
- `crates/edge-config/src/reference_defaults.txt` for the defaults, which is
  the reference's own account of them.

`--check` fails when the file is not what the source gives. The session
includes the file (`src/api/settings.rs`), hands it to the page
(`process_pages`) and checks values against it (`process_set`), so the page
and the session cannot disagree about a limit or a list.

| Page | Groups | Options | Simple | Advanced | Developer |
| --- | --- | --- | --- | --- | --- |
| Quality | Layer height, Line width, Seam, Precision, Ironing, Z contouring, Wall generator, Walls and surfaces, Bridging, Overhangs | 104 | 6 | 97 | 1 |
| Strength | Walls, Top/bottom shells, Hollowing, Infill, Advanced | 73 | 17 | 56 | 0 |
| Speed | Initial layer speed, Other layers speed, Overhang speed, Layer time control, Travel speed, Acceleration, Junction Deviation, Jerk(XY), Advanced | 64 | 0 | 64 | 0 |
| Support | Support, Raft, Support filament, Support ironing, Over-support surfaces, Advanced, Tree supports, Side stabilizers | 78 | 11 | 67 | 0 |
| Multimaterial | Prime tower, Filament for Features, Ooze prevention, Flush options, Advanced, Color Mixing (Experimental) | 63 | 6 | 54 | 3 |
| Others | Skirt, Brim, Special mode, Fuzzy Skin, G-code output, Change extrusion role G-code, Post-processing Scripts, Notes | 51 | 14 | 36 | 1 |

### What the engine does with an option

The page keeps no list of followed settings. `edge_engine::followed` answers
for the settings as they stand: it slices a 12 mm block with exactly what a
model would be sliced with (the presets, what is set over them, a project's
own settings under both), writes its G-code, and says

- which settings were asked for by name on the way (`edge_config::Reads`,
  which `Settings` notes into; going through all of them, as the settings
  block at the end of a G-code file does, asks for none);
- the notes the stages gave (`Sliced::gaps`: the same sentences "About this
  print" shows);
- why the slice stopped, if it did.

An option is then one of three things, shown beside its label:

| The engine's answer | Shown as |
| --- | --- |
| Read, no note about it | nothing: it is followed |
| A note that starts with its name | a dot; the tooltip starts "Followed with a note:" and gives the engine's sentence |
| Not read | a ring, and the label a shade quieter; the tooltip says the engine does not read it with the settings as they are, so changing it changes nothing in the print |

What the engine answers for the Snapmaker U1's "0.20mm Standard" process as
it is (the 12 mm block, one filament), page by page. "Unread and in sight"
are the options a ring is drawn beside: unread, and neither hidden nor
greyed out by the rules. "Unread either way" are those still unread with
supports, a raft, ironing, fuzzy skin, a brim, a prime tower, a scarf seam,
Arachne walls, gyroid fill and the rest of the switches turned on: the ones
no stage reads at all.

| Page | Options | Read | Read, with a note | Unread | Unread and in sight | Unread either way |
| --- | --- | --- | --- | --- | --- | --- |
| Quality | 104 | 84 | 0 | 20 | 1 | 6 |
| Strength | 73 | 33 | 0 | 40 | 13 | 37 |
| Speed | 64 | 33 | 0 | 31 | 10 | 31 |
| Support | 78 | 25 | 0 | 53 | 2 | 43 |
| Multimaterial | 63 | 21 | 0 | 42 | 18 | 42 |
| Others | 51 | 27 | 1 (`exclude_object`) | 23 | 14 | 23 |
| All | 433 | 223 | 1 | 209 | 58 | 182 |

Turned on, four more are read with a note: `zaa_enabled`, `spiral_mode`,
`raft_layers`, `draft_shield`.

Nothing is said of an option the reference's rules have greyed out: it is
off, whoever reads it. Some settings are read only when another turns them on
(the Arachne widths with `wall_generator` at `arachne`, the support patterns
with supports on); those are hidden or greyed by the rules while they are
off, and the answer is asked again after every change.

The same is said with the print. A real slice notes what it reads too
(`prepare_noted`), and "About this print" lists, after the engine's notes,
each setting set by hand that nothing read: `hollow_interior "1": not read by
the engine; it changes nothing in this print`.


## 13. Prepare as an editor

Read from `GLCanvas3D.cpp` (`_init_main_toolbar`, `on_mouse`, `do_move`,
`do_rotate`, `do_scale`), `Gizmos/GLGizmoMove.cpp`, `GLGizmoRotate.cpp`,
`GLGizmoScale.cpp`, `Gizmos/GizmoObjectManipulation.cpp`,
`GUI_ObjectList.cpp`, `ObjectDataViewModel.cpp`, `GUI_Factories.cpp`,
`PartPlate.cpp`, `Plater.cpp`, `Selection.cpp`, `Utils/UndoRedo.cpp`,
`libslic3r/Arrange.cpp` and `Jobs/ArrangeJob.cpp`.

Before this pass Prepare showed a file; the engine sliced the file as it was
saved. Now each open project holds a model that can be changed, and a slice
is of that model.

### The model, in the engine and in the session

| C++ | Where | Here |
| --- | --- | --- |
| A project is a `Model` of `ModelObject`s, each with `ModelInstance`s that carry a transform, and a `PartPlateList` that says which instance is on which plate | `libslic3r/Model.hpp`, `PartPlate.cpp` | **added**: `edge_engine::Model` (`crates/edge-engine/src/model.rs`): plates, objects, each object's copies. It knows nothing of a window. `Model::open` reads a file, `Model::place` gives what `prepare` takes |
| An instance is kept in the canvas's coordinates; when the plates are laid out anew (another bed, a plate more or fewer) every instance is moved with its plate | `PartPlateList::update_all_plates_pos_and_size`, `PartPlate::set_pos_and_size` | **differs**: a copy is kept in its own plate's coordinates and belongs to a plate or to none, so nothing has to be moved when the plates are laid out anew. The scene's coordinates (the first plate at 0, 0, the others in the reference's grid for the printer's bed) are worked out when the scene is asked for, and an edit is given in them |
| — | — | **added**: an untouched model slices to the bytes its file does. `Model::place` and `Opened::place` share one path (`load::placed`), each plate keeping the origin the file measures it from; checked to the bit for every plate of `mcbrim_test.3mf`, for `twopart_groups.3mf` and for a bare STL (`tests/model.rs`), and the G-code the app exports for an unedited project is the CLI's byte for byte |
| A bare model (an STL) is put in the middle of the bed when it is added | `Plater::load_files`, the arranger | **added**: a bare model is shown and sliced in the middle of the bed, as before, and its first edit gives it that place for good (`Model::settle`, `Loaded::placed`), with a plate to stand on. A file without plates gets its one plate the same way |
| An instance that has moved stays on its plate while its box reaches over any of the plate; else it goes to the first plate it reaches over; else it is on none | `PartPlateList::notify_instance_update` l.5026, `PartPlate::intersect_instance` l.2557 | **added** (`Model::set_transforms`). **differs**: by the box round its printed material on the ground; the reference takes the box of its convex hull, height included |
| What is on no plate is not sliced | `PartPlate::obj_to_instance_set`, the "unprintable plate" | **added**: it is in no plate's list (`plates[].objects`), counts for no plate's filaments, and the list has it under "Outside" |
| An instance over the edge of its plate stops the plate from being sliced: "An object is laid over the boundary of plate or exceeds the height limit. Please solve the problem by moving it totally on or off the plate, and confirming that the height is within the build volume." | `Plater::validate_current_plate` l.30333, GLCanvas3D.cpp l.4920 | **added**: the same words over the stage while the plate shown has such an object (`placings[].over`), and from `slice`, which refuses the plate. What reaches out of the plate is drawn darker (**had**). **not yet**: the height limit |
| What was moved, turned or sized is stood on the bed again: an instance that stood on the bed stands on it after a turn or a new size; one left in the air after a move is brought down; one that was sunk stays sunk | `GLCanvas3D::do_move` l.7029, `do_rotate` l.7153, `do_scale` l.7296 | **added** (`Model::stand_in_scene`). **not yet**: sinking an object under the bed by moving it down, which the engine would not cut off at the bed |
| A change to what is on a plate lets go of that plate's slice and no other | `PartPlate::update_slice_result_valid_state` | **added**: each plate has a `PlateStamp` (what its slice depends on: the shapes on it, where each stands, whether it is printed, the plate's place and first-layer order); an edit lets go of the slices whose stamp changed and calls a running slice off. A name is not in it: renaming keeps a slice. **differs**: taking a plate away numbers the later ones anew, and their slices go, the plate's number being in what is sliced (the prime tower's place) |

### `scene_edit`

One call makes every edit: `scene_edit { project?, op, … }`. It answers
`{ project, scene, done, made, note }`, so the page shows what came of it
without asking again. What cannot be done is an error and changes nothing.
The names in the last column are the reference's own for the step in its
undo history.

| `op` | Takes | Does | Step |
| --- | --- | --- | --- |
| `transform` | `copies: [{ id, matrix }]` (sixteen numbers, column by column, in the scene's coordinates), `name?` | stands each copy where its matrix puts it, then settles its plate and stands it on the bed | `name`: "Move Object" (a drag), "Gizmo-Move", "Gizmo-Rotate", "Gizmo-Scale", "Set Position", "Set Orientation", "Set Scale", "Tool Move" (the arrow keys) |
| `delete` | `copies: [id]` | takes copies away; an object whose last copy goes, goes | "Delete selected" |
| `rename` | `object`, `name` | | "Rename Object" |
| `printable` | `copies: [id]`, `value` | | "Set Printable" / "Set Unprintable" |
| `filament` | `object`, `filament` (from 1) | every printed part of the object takes it, as the reference clears the parts' own choice; what is painted on it stays | "Change Filament" |
| `add` | `path`, `plate?` | adds an STL's or a 3MF's objects to the plate (the one shown): "Import geometry only" | "Import Object" |
| `clone` | `copies: [id]`, `plate?`, `name?` | a new object for each, alike in shape, name, turn and size | "Clone", or "Paste From Clipboard" |
| `add_plate` | — | a plate after the last, which is then the one shown | "add partplate" |
| `delete_plate` | `plate?` | takes a plate away, not the last one | "delete partplate" |
| `arrange` | `plate?`, or `all: true` | lays out one plate, or every plate | "Arrange" |

### Undo and redo

| C++ | Where | Here |
| --- | --- | --- |
| A snapshot of the model, the plates and the selection is taken before each change, named for the change; Ctrl+Z and Ctrl+Y (Cmd+Shift+Z on a Mac) and the arrows in the title bar go back and forth, each with the list of steps under it | `Plater::take_snapshot`, `Plater::undo` / `redo`, `Utils/UndoRedo.cpp` | **added**: `undo` and `redo`, for each project its own history (`src/api/edit.rs`). A step is the model before the change; a model is cheap to keep because the meshes are shared. Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z, and two buttons whose tooltip names the step ("Undo [Ctrl+Z]", then the step). **differs**: the buttons are in Prepare's toolbar, there being no title bar. **not yet**: the list of steps to jump several at once |
| The history is kept while it fits in a tenth of the machine's memory, at most 1 GB | UndoRedo.cpp `StackImpl` l.551 | **differs**: the last 200 steps |
| A new change forgets what was undone | `Stack::take_snapshot` | **added** |
| Selecting is a step too ("Selection-Add Object") | Selection.cpp | **differs**: only edits are steps; what is selected stays selected while it is still there |
| The settings are not in the history: a preset or a setting changed is not undone by Ctrl+Z | `Plater::priv::take_snapshot` (the model, the plates, the gizmos; the presets are not serialized) | the same: the history holds the model only. A process setting set by hand, a preset or a bed type is neither a step nor taken back by one, and an edit does not touch it (tested) |
| Undoing back to the state that was saved leaves the project unmodified | `Plater::is_project_dirty` | **added**: the project is unsaved while its model is not the one it was opened with |

### Selection

| C++ | Where | Here |
| --- | --- | --- |
| A click selects an instance; Ctrl adds or takes away; a click on nothing or Esc lets go; Ctrl+A selects all | GLCanvas3D.cpp l.6423–6551 | **had** (section 9); now by the copies' `id`, so what is selected survives every edit that leaves it there. Shift adds as Ctrl does |
| Del or Backspace deletes what is selected | `Plater::remove_selected` l.25120, GUI_ObjectList.cpp l.4586 | **added** |
| The list and the canvas show one selection | `ObjectList::update_selections`, `update_selections_on_canvas` | **added**, both ways |
| The right button: the object's menu ("Delete", "Clone", "Printable", "Rename", "Fill bed with copies", "Set as an individual object", "Mirror", "Split", the settings…) | `GUI_Factories.cpp` l.649–1181 | **added**: "Delete" [Del], "Clone" [Ctrl+K], "Printable", "Rename". **not yet**: the rest (see "Left for later") |
| Ctrl+C / Ctrl+V copy and paste objects; Ctrl+K "Clone" asks how many | `Plater::copy_selection_to_clipboard` l.29941, `paste_from_clipboard`, `clone_selection` l.29965 | **added**: Ctrl+C notes what is selected, Ctrl+V puts a copy of each on the plate shown, Ctrl+K one beside each. **differs**: Ctrl+K makes one clone at a press and asks nothing; the clipboard is the page's, not the system's. **not yet**: Ctrl+X |
| A clone or a paste is put on a free place found by the arranger | `Plater::find_new_position` l.16128, `Selection::paste_objects_from_clipboard` l.3384 | **differs**: the free place nearest what it was made of, by the boxes round the copies, 6 mm apart; beside the plates when the plate has no room |

### The object list

In the sidebar, under "Objects" (`ui/objects.js`). In the C++ the list is
what the Process section shows when its switch is on "Objects"; here it has a
section of its own, the per-object settings that go with that switch not
being built. It stands before the Process section, where the C++ shows it
too (at the head of Process).

| C++ | Where | Here |
| --- | --- | --- |
| A tree: each plate ("Plate 1", with its name), under it the objects on it, under an object its parts, its instances when it has several, its layer ranges; what is on no plate under "Outside" | `ObjectDataViewModel.cpp` l.523, l.563 | **added**: plates, objects, instances ("Instance 1"), "Outside". A file without plates lists its objects alone. **not yet**: parts, layer ranges |
| Columns "Name", printable (a mark to click), visibility, "Fila." (a list of the project's filaments, each with its colour), the marks for support paint, colour paint, sinking, settings | GUI_ObjectList.cpp l.445–510 | **added**: "Name", printable, "Fila.". An object whose parts print in several filaments shows "…" until one is chosen for all of it. **not yet**: hiding an object, the marks |
| A click selects; Ctrl and Shift select several | `ObjectList::selection_changed` | **added** |
| A double click on the name, or F2, renames in place; an empty name leaves the name as it was; `<>:/\|?*"` are not allowed | `ObjectList::rename_item` l.6307, `update_name_in_model` l.1233 | **added**: Enter takes the name, Escape or an empty name leaves it. **not yet**: the characters that are not allowed (nothing is written under an object's name yet) |
| The plate's row: a click shows the plate; its menu | `ObjectList::OnContextMenu`, `create_plate_menu` | **added**: a click shows the plate; the right button opens "Select All", "Delete All", "Arrange", "Delete Plate"; a cross at the row is "Remove current plate (if not last one)" |
| Dragging rows to reorder, or onto another plate | `ObjectList::OnDrop` | **not yet** |

### Move, Rotate, Scale

The handles are drawn in the 3D scene by the page (`ui/scene-edit.js`), in
the axes' own colours; a handle under the pointer is lit. While a drag
lasts only what is seen changes; letting go makes one `scene_edit`.

| C++ | Where | Here |
| --- | --- | --- |
| "Move" [M]: arrows along X, Y and Z from the middle of what is selected; a drag moves along the arrow; Shift moves in steps of 1 mm | `GLGizmoMove3D::on_render`, `calc_projection` l.319, GLGizmoMove.hpp `m_snap_step` | **added**: the arrows along X and Y, the step with Shift. **not yet**: the arrow along Z. A copy lifted is stood on the bed again, as in the C++, so the arrow could only sink it, which the engine does not cut off |
| Dragging the object itself moves it across the bed, wherever a tool is open or not; across to another plate, it is that plate's | GLCanvas3D.cpp l.6553–6608, `do_move(L("Move Object"))` l.6763 | **added**: across the plane at the height of the middle of what is held |
| The arrows move what is selected 10 mm; with Shift 1 mm | KBShortcutsDialog.cpp, GLCanvas3D.cpp l.5669 (`do_move(L("Tool Move"))`) | **added**. Presses that come faster than the session answers are added up, so none is lost |
| "Rotate" [R]: a ring for each axis round the selection, the radius the bounding sphere's and 5 mm; a drag turns about the axis; with the pointer between a third and two thirds of the radius the turn goes by eighths, from the ring to a tenth beyond it by 5° | `GLGizmoRotate::on_dragging` l.102, l.211–217 | **added**, with those regions. One object turns about its own middle, several about the middle of them all. **not yet**: the scale of ticks drawn round the ring, the readout beside the pointer (the fields show the angle as it goes) |
| "Scale" [S]: grips at the foot of the box, two for X and two for Y, one above for Z, and four at the corners for all three at once; the size goes with how far the pointer is drawn out from the middle; Shift goes by 5%; Ctrl keeps the far side where it is ("Single sided scaling") | `GLGizmoScale3D::update_grabbers_data` l.220, `calc_ratio`, `do_scale_along_axis` l.414 | **added**: the grips along the object's own axes and at its corners, the steps with Shift; several objects have the corners only and grow about their middle. **not yet**: single sided scaling |
| Esc closes the tool before it lets go of what is selected; the tool's key again closes it | `GLGizmosManager::on_key` | **added** |

### The number fields

A panel under the toolbar while a tool is open, titled "Object Operations"
as the reference's is.

| C++ | Where | Here |
| --- | --- | --- |
| Move: a list "World" / "Object" ("Coordinate system used for transform actions."), X, Y, Z in the axes' colours, "Position" in mm | `do_render_move_window` l.795 | **added**. **differs**: in the world's coordinates X and Y are the middle of the box round what is selected, where the reference gives the instance's own origin (a model read here is not moved to stand about its origin), and Z is its lowest point, which cannot be typed; in the object's coordinates a number typed is how far to go along the object's own axis |
| Move: "Align" / "Distribute" with their anchors and origins, "Drop" ("Move the selection along Z so that its lowest point touches the bed."), "Hide other plates while moving" | l.990–1210 | **not yet**. Nothing is left in the air to drop |
| Rotate: "World", X, Y, Z, "Relative" in degrees with "Reset current rotation to the value when open the rotation tool.", "Absolute" with "Reset current rotation to real zeros." | `do_render_rotate_window` l.1506 | **added**: both rows and both resets; "Absolute" for one object. The angles are the reference's (about X, then Y, then Z) |
| Scale: "World" / "Object", "Scale" in percent with its reset, "Size" in mm, "uniform scale" | `do_render_scale_input_window` l.1721 | **added**. An object that is turned is sized as a whole in the world's coordinates, as in the C++ |
| Inches for millimetres ("in") | `m_imperial_units` | **not yet**: no preferences |
| What is typed is a step of its own: "Set Position", "Set Orientation", "Set Scale" | `change_position_value` l.286 and on | **added** |

### The main toolbar

Over the stage, in the middle (`.toolbar`, STYLE.md). A tool that cannot be
used now is off, not hidden.

| C++ | Where | Here |
| --- | --- | --- |
| "Add" [Ctrl+I]: a file dialog; the file's geometry joins the project | `_init_main_toolbar` l.9312, `Plater::add_file` l.24872 | **added**: an STL or a 3MF, into an empty project as much as into one with objects; what is added is selected. Each object keeps its turn and size and is stood on the bed at the free place nearest the middle of the plate shown |
| "Add plate" | l.9321, `PartPlateList::create_plate` l.4378 | **added**: at most 36 (PartPlate.hpp `MAX_PLATE_COUNT`); the new plate is the one shown, and what stood beside the plates where it comes to stand is on it |
| Removing a plate: its objects are not deleted, they are left beside the plates that remain; the last plate stays | `PartPlateList::delete_plate` l.4531, `compute_origin_for_unprintable` l.3900 | **added**, the same place beside the plates. Which plate is shown afterwards is the reference's rule |
| "Arrange all objects" [A], "Arrange objects on selected plates" [Shift+A], with a popup of options ("Spacing", "Auto rotate for arrangement", "Allow multiple materials on same plate", "Avoid extrusion calibration region", "Align to Y axis") | l.9352, `ArrangeJob.cpp` | **differs**: both keys and the button (every plate), but **this is not the reference's arrangement**: see below. **not yet**: the options |
| "Auto orient", "Split to objects", "Split to parts", "Variable layer height", "Assembly View" | l.9331–9400 | **not yet** |

### Arrange: what the reference does, and what a port would take

The reference arranges with libnest2d (`deps_src/libnest2d`, some 13 000
lines) driven by `libslic3r/Arrange.cpp` (1 169 lines) and
`Jobs/ArrangeJob.cpp` (905). Each object is its outline on the bed, grown by
half the spacing and its brim; an item is placed where the no-fit polygon of
its outline against what is already placed allows (`_NofitPolyPlacer`), the
candidates along that polygon's edges scored by an objective of the
reference's own (`AutoArranger::objfunc` l.452: how near the pile's middle,
how well it lines up with its neighbours, a bonus for objects of like height
and filament, penalties for the excluded and the unwanted regions and for the
prime tower) and refined by a numerical optimiser; items are taken largest
first (`_FirstFitSelection`) and those that do not fit go on to the next
plate, a new one being added when needed. `ArrangeJob` gathers the items,
leaves locked plates alone, takes the spacing from the printer when it is not
given, keeps the clearances of printing by object, and puts the result back
as moves of the instances.

What stands here instead (`Model::arrange`) lays the copies of a plate out in
rows by the boxes round them, the deepest first, 6 mm apart, the whole set in
the middle of the bed; nothing is turned; what a plate has no room for is put
beside the plates and the person is told how many that was. It is the same
every time and keeps things on the bed and apart, and that is all it is.

A port would take, in this order: (1) outlines instead of boxes, which the
engine can already make (`edge_geom::convex_hull` of a copy's vertices on the
ground); (2) no-fit polygons, for convex outlines a Minkowski sum, which
belongs in `edge-clip` beside the clipping library; (3) the placer with the
reference's objective, which is the bulk of `Arrange.cpp` and can be ported
as it stands; (4) the optimiser along the polygon's edges (libnest2d's own,
a few hundred lines once the rest is there); (5) several plates as bins and
the options. Steps 1 to 3 give arrangements that can be compared with the
reference's on fixtures; the comparison needs the reference run with
`--arrange`, which `edge-parity` does not do yet.

### Left for later

| C++ | Waits for |
| --- | --- |
| "Lay on face" [F] (`GLGizmoFlatten.cpp`: the faces of the object's convex hull, a click lays one on the bed) | the convex hull of a mesh in three dimensions, in the engine; the page has only the mesh |
| "Fill bed with copies" (`GUI_Factories.cpp` l.1146, `FillBedJob.cpp`) | the reference's arrangement: it asks the arranger how many more fit |
| Sinking an object under the bed; the arrow along Z; "Drop" | the engine cutting what is under the bed away (it slices from an object's lowest point) |
| "Mirror", "Auto orient", "Split to objects" / "Split to parts", "Cut", "Mesh Boolean", "Simplify" | mesh operations the engine does not have |
| Parts in the list, a filament for one part, modifiers and their settings, "Set as an individual object" | per-object and per-part settings, which the engine does not read (section 12) |
| Saving the project with its edits | **added**: section 15 |
| The glow round what is selected, the red background while something is over the edge, the ticks of the rotate ring | nothing; they are looks, left for a pass on the stage's own drawing |
| Rectangle selection (Shift with a drag), dragging rows of the list | nothing; not built in this pass |
| A printer with another bed moving every object to keep its place | deciding the rule: here a copy keeps its place on its own plate, and what was on no plate stays where it stood beside the first |

### How it was checked

In the engine: `crates/edge-engine/tests/model.rs` (an untouched model is its
file to the bit; each edit, plate membership, standing on the bed, adding,
cloning, arranging). In the session: `src/api/edit/tests.rs` (a moved object
is sliced where it stands and the toolpaths' box moves with it; undo gives
the bytes it sliced to before; only the plates an edit touches lose their
slice; plates added and removed; geometry added to an empty project, cloned,
arranged and sliced; a process setting is left alone by undo).

In the served page (`--serve`, the app's browser pane, 1280 × 800, dark and
light, and 700 and 1920 wide), with `mcbrim_test.3mf`, a bare STL and an
empty project, by the mouse and the keys: selecting in the scene and in the
list; dragging a Move arrow (a step of "Gizmo-Move", the X field going from
87.28 to 127.14); Ctrl+Z and the Redo button; dragging the Z ring (two
objects turned by 30° in the 5° region); typing a relative rotation of 25°
about X (the object stood on the bed again) and resetting it; dragging a
corner grip (142.84%) and typing a size of 20 mm; dragging an object from
plate 1 onto plate 2 (the list and `plates[].objects` followed), slicing
plate 2 and looking at Preview, where its toolpaths are among the plate's
others; deleting three objects picked with Shift in the list; renaming by a
double click; printable off (the object goes dark); another filament from
the list's own list; "Add" into an empty project, Ctrl+K, Ctrl+C and Ctrl+V,
"Add plate", an object typed half over the edge of plate 2 (the warning, and
"Slice plate" refused in the same words), "A", the object's menu, removing
plate 2 at its row (its object then under "Outside"); a bare STL dragged
(it gets a plate), moved by the arrow keys, sliced: the toolpaths' box
(190.44 to 247.96 by 79.81 to 93.79) is the object's (190.23 to 248.17 by
79.60 to 94.00) less half a line. The screenshots are under
`target/ui-shots/edit/`.

### Errors found in the C++

- `3DScene.cpp` l.351: `#ifdef ENABBLE_OUTSIDE_COLOR` (misspelt) opens a
  block whose `else {` is closed under `#ifdef ENABLE_OUTSIDE_COLOR` at
  l.360. Neither name is defined anywhere, so the colour for what is outside
  the plate is dead code today; defining the second, which is what a reader
  would do to turn it on, leaves the braces unbalanced.
- `PartPlate.cpp` l.2514: `PartPlate::check_outside` works out the
  instance's convex hull (`Polygon hull = instance->convex_hull_2d();`) on
  every call and never uses it; the one that is used is worked out again at
  l.2535. It runs for every instance each time anything moves.
- `GizmoObjectManipulation.cpp` l.1010: the tooltip reads "Holded down Ctrl
  key continuously and click by left mouse button…" (wording only).

## 14. The filament's and the printer's pages

Read from `TabFilament::build` and `add_filament_overrides_page`
(Tab.cpp l.5668–6358), `TabFilament::toggle_options` and
`update_filament_overrides_page` (l.6756, l.5820), `TabPrinter::build_fff`,
`build_kinematics_page` and `build_unregular_pages` (l.7028–8003),
`TabPrinter::toggle_options` (l.8382), and `PrintConfigDef`.

### Where the pages come from

`tools/make_settings_pages.py` (it was `make_process_pages.py`) writes all
three kinds into `src/api/settings_pages.json`; the process's part is what
it was, byte for byte. Beside what section 12 says it reads, it follows an
`Option` or a `Line` of any name, the label an option is given on its line
("Target", "Minimal"), a line whose tooltip is an option's, a line with a
widget of the reference's own in an option's place (`create_line_with_widget`),
the `for` over a list of keys the overrides page hands its helpers to, the
printer's `append_option_line` in its loops over the axes, the extruder
count the tab defines for itself, the field heights a function names, and,
in `PrintConfigDef`, the machine limits made in a loop over the axes and the
filament's retraction overrides made from the printer's. The printer's pages
are put in the order `build_unregular_pages` leaves them in for a Marlin,
Klipper, RepRapFirmware or Repetier printer, the flavors its
`is_marlin_flavor` names.

| Kind | Page | Groups | Options | Simple | Advanced | Developer |
| --- | --- | --- | --- | --- | --- | --- |
| Filament | Filament | Basic information, Flow ratio and Pressure Advance, Print chamber temperature, Print temperature, Bed temperature, Volumetric speed limitation | 47 | 27 | 17 | 3 |
| | Cooling | Cooling for specific layer, Part cooling fan, Auxiliary part cooling fan, Exhaust fan | 23 | 15 | 8 | 0 |
| | Setting Overrides | Retraction, Ironing | 23 | 4 | 19 | 0 |
| | Advanced | Filament start G-code, Change extrusion role G-code, Filament end G-code | 3 | 0 | 3 | 0 |
| | Multimaterial | Wipe tower parameters, Tool change parameters with single extruder MM printers, Tool change parameters with multi extruder MM printers | 23 | 0 | 23 | 0 |
| | Dependencies | Compatible printers, Compatible process profiles | 4 | 0 | 4 | 0 |
| | Notes | Notes | 1 | 0 | 1 | 0 |
| | All | | 124 | 46 | 75 | 3 |
| Printer | Basic information | Printable space, Advanced, Cooling Fan, Extruder Clearance, Adaptive bed mesh, Accessory, End of print | 36 | 4 | 28 | 4 |
| | Machine G-code | File header, Machine start, Machine end, Printing by object, Before layer change, Layer change, Timelapse, Change filament, Change extrusion role, Pause, Template Custom G-code | 11 | 0 | 11 | 0 |
| | Multimaterial | Single extruder multi-material setup, Wipe tower, Per-nozzle settings (experimental), Single extruder multi-material parameters, Advanced | 23 | 0 | 23 | 0 |
| | Extruder n (one for each) | Size, Layer height limits, Position, Retraction, Z-Hop, Retraction when switching material | 25 | 2 | 21 | 2 |
| | Motion ability | Advanced, Resonance Compensation, Speed limitation, Acceleration limitation, Jerk limitation | 26 | 14 | 12 | 0 |
| | Notes | Notes | 1 | 0 | 1 | 0 |
| | All | | 122 | 20 | 96 | 6 |

Groups that hold only a widget of the reference's own have no options and
are left out: "Values per nozzle (experimental)" on the filament's first
page, "Nozzle rack (experimental)" on an extruder's, and the
"Nozzles in this project" line ("Extruder settings…") of an extruder's
"Size"; so are the price, machine-rate and makeup notes.

### Where they are in the page

| C++ | Where | Here |
| --- | --- | --- |
| The filament's and the printer's settings are tabs of their own beside the process's (`ParamsPanel`, or the parameters dialog), opened from a filament row's edit icon or "Edit" in its menu, and the printer row's edit icon ("Click to edit preset") | Plater.cpp l.4167, l.2659, GUI_Factories.cpp l.2606 | **differs**: a panel that takes the sidebar's place, with "Back to the sidebar" (and Escape), opened by the edit icon of a filament's row or of the printer card. The sidebar is wider while it is open (`clamp(340px, 30vw, 520px)`: 340 px at 900, 520 at 1920), the plate staying in sight to be sliced and looked at; under 761 px it is the one column the sidebar is. Chosen over a dialog: a dialog would cover the plate the change is for, and the process's pages already live in the sidebar, so the three kinds are one renderer in one column |
| A filament's tab shows the one filament preset being edited, picked in its own list | `Tab::create_preset_tab` | **differs**: the project's filament n, shown over its preset; the filaments' numbered swatches at the panel's head go from one's pages to another's |
| The pages' tree with the changed ones marked; "Advance"; search | Tab.cpp | **added**, as for the process (section 2): tabs with a count, search over the kind's options (an extruder's under each "Extruder n"), the one "Advance" switch for all pages, as the reference has one mode |

### What is set, and how it reaches the slice

| C++ | Here |
| --- | --- |
| Editing changes the edited preset, saved or discarded ("Unsaved Changes") | **differs**: as for the process, a change is the project's own over the preset (`Choices::filament_over`, one map for each filament; `Choices::printer_over`), kept with the project, through a change of preset and of printer (the status line says "Kept n filament n settings from …"), reset per option or all of a kind (and filament) at once. It drops the slices and marks the project changed |
| A filament preset holds one filament's values | **added**: a filament's value is that filament's alone: the engine's `Overrides::filaments[n]` sets entry n of a list with a value for each filament, every other entry as it was (`crates/edge-engine/src/job.rs` `set_entry`) |
| An extruder's page shows that extruder's entry of the printer's lists (`append_single_option_line(key, "", extruder_idx)`), or its variant slot on a per-variant printer (`extruder_field_index`) | **added**: options named `key#n`; `Overrides::extruders[n]` sets entry n. **differs**: no variant slots; entry n is extruder n, which is how the engine reads the lists (`PrintSettings::for_filament`). A printer preset with per-variant lists (`printer_extruder_variant`) is not read so by the engine either |
| A value for each nozzle flow (Standard, High Flow), picked by the variant switch over the pages (`register_flow_variant_view`) | **differs**, as on the process pages: the field shows the value the engine reads for the filament (its entry, or the list's last), and sets that entry. **not yet**: a value for each flow; the engine reads none apart |
| The machine limits: a column for the normal mode and, with `silent_mode`, one for the silent (`append_option_line`) | **differs**: the normal mode's alone (entry 0), the silent's kept; the row is named in full ("Maximum speed X"). **not yet**: the silent column |
| The overrides page: each option's tick box; off, the field shows the printer's value (the process's for ironing) and the option is "nil" | **added**: the tick box before the label, the field greyed with the value in force while off ("Off: the printer's value for this filament's extruder is used."); on, it starts from that value. The rules for which can be ticked are the reference's |
| G-code boxes; "Edit Custom G-code" (the placeholders dialog) | **added**: a box of text in the fixed-width face, shown unescaped and held escaped, as profiles hold G-code. **not yet**: the placeholders dialog, and the check of the G-code as it is typed (`validate_custom_gcode`) |
| Set in a dialog of its own: "Printable area" (Bed Shape), the ramming ("Set…", RammingDialog), "Compatible printers" / "Compatible process profiles" (a checklist) | **differs**: shown, greyed, with the reason in the tooltip; not offered |
| "Extruders" (`extruders_count`), which adds or removes extruder pages | **differs**: shown, not offered: the number of extruders is the printer preset's; another printer is picked in the Printer section |
| A filament's "Compatible printers condition", "Compatible process profiles condition" | **differs**: shown, not offered: they decide which presets are offered with which, which matters to a saved preset only |
| What a value may be (Field.cpp) | **added**, as for the process: refused in the page and by the session, with the reason; a point is "XxY" |
| What the engine would stop at | **added**: refused with the engine's reason, asked with the block printed in the filament being set |
| A setting set by hand that nothing read | **added**: with the print's notes, named with whose it is: `fan_max_speed (filament 1) "60": not read by the engine; it changes nothing in this print` |

### The rules

`src/api/settings/filament_rules.rs` ports `TabFilament::toggle_options`
and `update_filament_overrides_page`; `printer_rules.rs` ports
`TabPrinter::toggle_options`, each extruder's rules once for each extruder,
and the "Wipe tower" group's own rule ("Enable pressure change when
wiping"). The U1's plates are named its way ("Cool Steel Plate", "Smooth
PEI Plate") and only its plates are shown, as the rules have it.
**not yet**: what the reference changes by itself while it toggles
(`initial_layer_fan_speed` to -1, `manual_filament_change` off, the
question that turns wiping off under firmware retraction), the input
shaper's list cut to the flavor, the checks run as a value is typed
(`check_nozzle_temperature_range`, `check_chamber_minimal_temperature`,
`check_adaptive_pressure_advance_model`: questions); "Motion ability" for
another flavor than those named is hidden, and the pages keep their order.

### What the engine does with them

As in section 12, from the engine's own answer (`edge_engine::followed_in`,
the block printed in the filament asked about). For `mcbrim_test.3mf` on
the Snapmaker U1 (0.4 nozzle) with "Panchroma PLA Matte @U1", filament 1
and extruder 1:

| Kind | Page | Options | Read | Read, with a note | Not read | Not read and in sight | Not set here |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Filament | Filament | 47 | 43 | 0 | 4 | 4 | 0 |
| | Cooling | 23 | 21 | 0 | 2 | 0 | 0 |
| | Setting Overrides | 23 | 22 | 0 | 1 | 1 | 0 |
| | Advanced | 3 | 2 | 0 | 1 | 1 | 0 |
| | Multimaterial | 23 | 14 | 0 | 8 | 7 | 1 |
| | Dependencies | 4 | 0 | 0 | 0 | 0 | 4 |
| | Notes | 1 | 1 | 0 | 0 | 0 | 0 |
| | All | 124 | 103 | 0 | 16 | 13 | 5 |
| Printer | Basic information | 36 | 3 | 0 | 32 | 30 | 1 |
| | Machine G-code | 11 | 5 | 0 | 6 | 6 | 0 |
| | Multimaterial | 23 | 6 | 0 | 16 | 14 | 1 |
| | Extruder 1 | 25 | 17 | 0 | 8 | 6 | 0 |
| | Motion ability | 26 | 16 | 0 | 10 | 4 | 0 |
| | Notes | 1 | 0 | 0 | 1 | 1 | 0 |
| | All (one extruder) | 122 | 47 | 0 | 73 | 62 | 2 |

### How it was checked

In the engine: `job.rs` (filament 2's and extruder 1's own entries, a list
made long enough). In the session: `src/api/settings/preset_tests.rs` (the
pages as the reference has them; filament 2's temperature in the G-code's
moves and filament 1's untouched; extruder 2's retraction, a start G-code
and a Z speed limit in the G-code; refusals; an override on and off; the
rules and the engine's answer) and the rules' own tests.

In the served page (1280, 900, 700 and 1920 wide, dark and light), with
`mcbrim_test.3mf` (seven filaments, the model in filaments 2 to 7) on the
U1: filament 2's "Other layers" typed to 245 (marked, counted, its reset
arrow); then, by the calls the page makes, filament 1's flow ratio 0.9 and
fan 70, filament 3's retraction override 2.2, the printer's start G-code
with a marker line, extruder 1's retraction 1.9 and the Z speed limit 14.
Sliced and exported: `M104 T1 S245` / `M109 T1 S245`, eleven
`G1 E-2.20000` retractions (none before), every Z move at `F840` (14 mm/s;
`F1200` before), the marker line, and in the settings' list
`nozzle_temperature = 220,245,220,…`, `filament_flow_ratio = 0.9,0.98,…`,
`fan_max_speed = 70,100,…`, `retraction_length = 1.9,1.5,1.5,1.5`,
`machine_max_speed_z = 14,12`. Reset, sliced again: every one of them back
to the preset's (`nozzle_temperature = 220,220,…`, no `S245`). The
screenshots are under `target/ui-shots/presets/`.

### Not yet

| C++ | Waits for |
| --- | --- |
| Saving, renaming, deleting user presets; "Save current …"; detaching from the system preset | user presets, which `edge-config` does not read or write |
| "Compare presets" | the same |
| Preset bundles, import and export | the same |
| The filament colour picker ("Click to select filament color"), "Add one filament", "Remove last filament" | the engine printing filaments the file does not assign |
| AMS sync, "Sync Filament Information" | a printer connection (section 4) |
| The physical printer and print host dialog | a list of printers of this app's own (section 4) |
| The dialogs above (bed shape, ramming, compatible presets, Extruder settings, the G-code placeholders) | each its own piece of work |
| A value for each flow variant; the silent mode's limits | the engine reading them apart |

### Errors found in the C++

- `Tab.cpp` l.6908: `toggle_line("chamber_temperatures", …)` names an option
  that does not exist (it is `chamber_temperature`), so the chamber's line
  is never hidden for a printer without chamber control; the same name at
  l.6144 never matches, so `check_chamber_temperature` never runs.
- `Tab.cpp` l.8471 and l.8493: `toggle_option("retract_length", …)` names an
  option that does not exist (`retraction_length`), so the length is never
  greyed under firmware retraction, which is what the comment above it says
  it is for.
- `Tab.cpp` l.8545: `toggle_line("retraction_distances_when_cut#0", …)` is
  extruder 1's line on every extruder's page.
- `Tab.cpp` l.6860–6862: with several bed types, `cool_plate_temp` is shown
  twice and `supertack_plate_temp` not at all, so its half of the line keeps
  whatever the last printer left it as.
- `TabPrinter::build_unregular_pages` (l.7663): "Multimaterial" goes in two
  from the end, which with "Motion ability" left out (a flavor not in
  `is_marlin_flavor`) puts it, and the extruder pages after it, before
  "Machine G-code".
- The engine, not the C++: `edge_engine::prepare` (`for_filaments`, job.rs)
  merges per filament only the lists a filament preset has one entry of; a
  preset with a value for each flow variant (two entries: most of the U1's,
  `nozzle_temperature ["220", "220"]`) keeps the first preset's list, so
  filament 2 reads filament 1's High Flow value and filament 3 on read
  filament 1's last, whatever their own presets say. Left as it is in this
  pass (it changes what an untouched project slices to); the pages show and
  set what the engine reads.

## 15. What a saved project holds

Read from `libslic3r/Format/bbs_3mf.cpp` (`_BBS_3MF_Exporter`, and
`_BBS_3MF_Importer` for what the reference reads back),
`libslic3r/PresetBundle.cpp` (`full_fff_config`, `full_config_secure`,
`load_config_file_config`), `libslic3r/Preset.cpp` (`load_external_preset`)
and `libslic3r/Config.cpp` (`save_to_json`). The writer is
`crates/edge-3mf/src/write.rs`, what is kept from a file
`crates/edge-3mf/src/kept.rs`, the model's side
`crates/edge-engine/src/save.rs`, the session's `src/api/save.rs`, the
page's `ui/save.js`; `edge save` on the command line writes the same file.

### The package

The reference's "Save Project" layout (`SplitModel | ShareMesh`), entry by
entry, in a fixed order:

| Entry | Here |
| --- | --- |
| `[Content_Types].xml`, `_rels/.rels` | written afresh, as the reference writes them; `_rels/.rels` names the cover (`Auxiliaries/.thumbnails/…` when the file had one, else the first plate's picture while that is written) |
| `3D/3dmodel.model`: the header, an object of components for each object, the build (each copy: transform, printable) | written afresh. The header keeps every `<metadata>` the file had (title, designer, licence, description…), with `Application` "EdgeSlicer-2.4.9.0" (the pinned reference's version, `edge_3mf::REFERENCE_VERSION`, checked against the submodule by a test), `BambuStudio:3mfVersion` 1, the dates and the designer's id empty as the reference writes them. The reference reads a file as a whole project (plates, settings, presets) only when its `Application` names EdgeSlicer, Snapmaker Orca or Bambu Studio, so the reference's own name is the one written. Transforms are written as the shortest text that reads back to the same number (the reference writes 9 digits) |
| `3D/Objects/<name>_<n>.model`, `3D/_rels/3dmodel.model.rels` | written afresh: a file for each object, each mesh of it that no earlier part has (the same mesh painted alike is written once and named from every part that has it, as `ShareMesh` does); vertices as the shortest text that reads back to the same `f32`; each triangle with every attribute it carried as the file had it (`paint_color`, `paint_seam`, `paint_supports`, `paint_fuzzy_skin`, texture layers, `face_property`). A `/` or another character a path cannot hold in an object's name is written `_` in the file's name (the reference writes the name as it is) |
| `Metadata/model_settings.config` | written afresh from the model: each object's name and filament and each part's (`extruder`: the object's when its printed parts agree, each part's own only where it differs), each part's `subtype`. Kept as the file had them: every other metadata and setting of each object and part (per-object and per-part settings, modifiers' settings, `matrix`, `source_*`, Precise Seam's type, embossed text and SVG descriptions, `mesh_stat`), each plate's own (`locked`, `bed_type`, `print_sequence`, `first_layer_print_sequence`, `spiral_mode`, `filament_map_mode`, `filament_maps` with one entry for each filament, `print_history`…), each copy's place in the assembly view while its object has all its copies, and each copy's `identify_id` (the number the reference labels it with in G-code) while it has. Plates are numbered as they stand, each with its name and its copies |
| `Metadata/project_settings.config` | the settings, below |
| `Metadata/slice_info.config` | written afresh: the header alone, as the reference writes a project saved unsliced |
| `Metadata/layer_heights_profile.txt` (variable layer height), `layer_config_ranges.xml` (height ranges and their settings), `brim_ear_points.txt`, `cut_information.xml` (cut connectors) | kept, each object's line or block written under its place in the list as it stands, so they follow an object when one before it is deleted; a clone carries its original's, as the reference's clone does |
| `Metadata/custom_gcode_per_layer.xml` (pauses, colour changes, custom G-code at layers) | kept, each plate's under its number as it stands |
| `Metadata/plate_<n>.png`, `_small`, `plate_no_light_<n>`, `top_<n>`, `pick_<n>` | kept for each plate while it is as it was read (`PlateStamp`: what is on it and where, its filaments and first-layer order), under its number now; a plate that has changed is saved without pictures, which would show it as it was. **not yet**: drawing new ones |
| `Metadata/process_settings_<n>.config`, `filament_settings_<n>.config`, `machine_settings_<n>.config` (presets the file carries) | kept while the project is saved with its file's own settings; with presets chosen here they are not, and that is asked about first |
| `Metadata/cut_recipe.xml` and its meshes, `edgeslicer_height_patterns.json`, `Metadata/showcase/` | not read here and kept by each object's place in the list: kept as they were while every object of the file is at its place; once one is deleted they are not kept, and that is asked about first |
| Every other entry (`Auxiliaries/`, embossing SVGs, CAD bodies, airbrush fields, texture layers, image fill…) | kept byte for byte |
| `Metadata/plate_<n>.gcode` and `.md5`, `plate_<n>.json`, `filament_sequence.json`, `_rels/model_settings.config.rels` | not written: a slice saved with the project. A saved project is unsliced, as the reference's "Save Project" writes it |
| An entry whose name reaches outside the package (`../`, a drive, a leading `/`) | not written, and said before saving |

The same project gives the same bytes: the order is fixed and every entry
carries one time, `SOURCE_DATE_EPOCH` when it is set, else 1980-01-01. A
file is written to `<path>.tmp` and moved into place once whole, so what was
at `path` is left as it was when writing fails.

### The settings

| Project | Written |
| --- | --- |
| On its file's own settings, nothing set over them | the file's `project_settings.config` as it was, and the presets it carries |
| On its file's own settings, with process, filament or printer settings or the bed type set here | the file's own, each setting set here taking the value the engine settles it to (`edge_engine::own_settings`), everything else as the file has it; what was set is added to the file's `different_settings_to_system`; the file's presets go with it |
| On presets chosen here | what a slice reads (`edge_engine::settled`: the presets with everything they inherit, the file's own settings under them, what was set over them, the reference's defaults for the rest), only the reference's own options and the names (`printer_settings_id`, `print_settings_id`, `filament_settings_id`), a value from the defaults' table in the text a project holds (its escapes taken out); `different_settings_to_system` lists, for the process, each filament and the printer, the settings set here (`edge_engine::project_settings`) |
| No printer chosen (an STL, an empty project) | none: the reference opens it on the presets it is on |

`different_settings_to_system` is written truthfully, as the reference
reads it (Preset.cpp `load_external_preset`): when it opens the file it
takes every setting of a preset that is not listed from the preset the file
names, so a setting listed is one set here and a value the engine settled
otherwise for an unlisted one is the preset's again. Written as JSON with
keys in order and four spaces to a level (`save_to_json`), with `name`
"project_settings", `from` "project", `version`. `print_host`,
`print_host_webui`, `printhost_apikey`, `printhost_cafile`,
`printhost_user`, `printhost_password` and `printhost_port` are never
written (`full_config_secure`), whichever way the settings come (tested).

### Asked before saving

What the file held that a save would not keep is listed, and nothing is
written until the person says to: the presets it carries when presets are
chosen here; what goes by objects' places (above) once an object of the
file is deleted; the assembly view of an object some of whose copies were
deleted; an entry whose name reaches outside the file. The reference never
asks this, keeping everything it reads; the words are this app's. `edge
save` lists the same and writes only with `--accept-losses`.

### How it was checked

- `crates/edge-3mf/tests/round_trip.rs`: every 3MF the reference ships or
  tests with (36), read, written and read again, is the same project:
  objects, names, parts, kinds, filaments, meshes to the bit, colour and
  seam paint, what every triangle carries, copies, plates, settings (but
  what reaches a printer); written again from what was read back, the same
  bytes. A plain file's layout, the paint kept on its triangle, a plate's
  own settings, the assembly view, a slice and a name reaching outside
  left out; a printer's address left out; the version claimed is the
  pinned reference's.
- `crates/edge-engine/tests/save.rs`: every reference project saved and
  opened again places to the bit on every plate; `mcbrim_test.3mf` and
  `twopart_groups.3mf` slice to the same bytes; `mcbrim_test.3mf` with a
  copy moved, one deleted, an object given filament 5 and renamed slices
  from the saved file as it did in memory, plate by plate, keeps the
  pictures of no plate it changed and all of those it did not; cut
  information follows its objects when the first is deleted; a variable
  layer height is kept; settings settled here name what was set by hand;
  a bare STL is saved standing in the middle of the bed.
- `src/api/save/tests.rs`: saving over the file opened, the unsaved mark
  cleared and back with undo; an STL asked where, saved as a 3MF with the
  presets and a process setting; the losses asked about first; an empty
  project saved.
- Both slicers on every reference project and five fixtures
  (`tools/make_fixture.py`: cube, enforced, blocked, halves, twotone),
  each plate of the original and of the file `edge save` wrote
  (`SOURCE_DATE_EPOCH=1`): this engine's G-code is the same to the byte for
  every one (those it refuses, it refuses alike); the C++ slicer's is the
  same to the byte for `mcbrim_test.3mf` (three plates), `pa_pattern`,
  `auto_pa_line_dual`, `auto_pa_line_single`, `cylinder_drawcut`,
  `stabilizer_v1`, `onepart_group` and the four `twopart_groups*`. For the
  five fixtures, which are not the reference's own projects (no
  `Application`), every move is the same and only the settings listed in
  the G-code's comments differ: the reference reads a project of its own
  through its presets, which lays a U1's lists out per nozzle
  (`accel_to_decel_enable = 1,1`). The rest it slices in neither form
  headless (no printer in the file, or files it refuses).
- In the served page (1280 × 820, dark and light), `mcbrim_test.3mf`
  (a copy) with a copy moved, one deleted, an object given filament 5 and
  `wall_loops` set to 5: saved by the tile's "Save" and by Cmd+S, the dot
  gone and the path said; "Save as..." with the page's dialog to a path
  typed without `.3mf`, the tile renamed; closing it unsaved asked "Save
  them before closing it?" with Yes / No / Cancel. The C++ slicer, given
  the saved file, prints the moved copy where it was moved (its first
  layer's lines there, none left where it stood), plate 2 without the
  deleted copy (its filament 6 no longer used), plate 3's object in
  filament 5 (4.71 g where it was 2.26 g), and `wall_loops = 5`. A move
  that put the copy into its neighbour was refused by the C++ slicer for
  conflicting paths, as it should be; this engine does not check that.
- Not checked: the window's own close question (the window was not
  opened in this pass; the page's side and the session's are as above).

### Not yet

- Drawing a plate's pictures, and the project's cover.
- "Export plate sliced file", "Export all sliced file" (a project with its
  G-code), and "Export Bambu 3MF".
- Reading back, or keeping, what a project is saved with beyond the file:
  "Reopen the open projects at the next start", recent projects, crash
  backups.

### Errors found in the C++

- `bbs_3mf.cpp` l.7954: the small thumbnail is checked with
  `if (png_data != nullptr)`, the large one's pointer (freed at l.7912),
  where `small_png_data` is meant; a small picture that failed to encode is
  then written from a null pointer.
- `bbs_3mf.cpp` l.7945: the small thumbnail's pixel index is
  `i / sw * W + j / sh` with `i` stepping by `sh` and `j` by `sw`: rows and
  columns swapped. Harmless while the picture is square (512 to 128), wrong
  for any other.
- `bbs_3mf.cpp` l.8302: an object's file is named `3D/Objects/<name>_<id>.model`
  from its name as it is, so a `/` in a name makes a folder of it (and `..`
  climbs one).
- `bbs_3mf.cpp` l.9274–9276: `_add_project_embedded_presets_to_archive`
  builds a header line into `out` that is never used.
- `bbs_3mf.cpp` l.8039: the cover's `rel-5` line is written without the
  space the others have (wording only).
