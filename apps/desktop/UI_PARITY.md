# UI parity with the C++ EdgeSlicer

What the C++ application offers in its main tabs and its project strip, what
this app had, and what it has now. It is the map later passes work from: the
C++ side is listed whole, including what is not built here.

The C++ source read is the pinned `reference/EdgeSlicer` (156e9591fe, the
merge of PR 920 "feat/multi-project-strip"). Paths are under
`src/slic3r/GUI/` unless they start with `libslic3r/`, `libvgcode/` or
`Utils/`. Strings in "quotes" are the C++ application's own English as
its source writes them (its message ids); the C++ shows them through its
catalog of the language chosen, English included, and so does this app
(section 16).

The last column reads:

- **had** — the app did this before the pass.
- **added** — built in the pass that wrote the row (first pass: Prepare,
  Preview, Device, the project strip; second pass: the sidebar's printer
  block, "Sidebar: Printer" in section 2, and section 8; third pass: the 3D
  views, sections 9 to 11; fourth pass: the process pages, "Sidebar:
  Process" in section 2 and section 12; fifth pass: Prepare as an editor,
  section 13; sixth pass: the filament's and the printer's pages, section
  14; seventh pass: "Lay on face", "Mirror", "Split to objects" and
  "Split to parts", in section 13; eighth pass: saving projects, the
  "Files" rows of section 2, the strip's saving in section 5, and
  section 15; ninth pass: the parts of an object, "Parts" in section 13,
  and what a saved project holds of them in section 15; tenth pass: plates
  as plates, the "Plates" rows of section 2, "Plates" in section 13 and
  what a saved project holds of them in section 15; eleventh pass: the
  G-code window and what is set at layers, in sections 3, 10 and 15;
  twelfth pass: sending to printers, the print button's "Print", the
  Device tab with the printer's own page, in sections 2, 4, 7 and 8;
  thirteenth pass: the list of filaments, "Sidebar: Filament" in section 2,
  `filament_edit` in section 7, the undo history in section 13 and what
  a saved project holds of them in section 15; fourteenth pass: wording
  and languages, section 16, and the rows that point to it; fifteenth
  pass: the menu bar, section 17, recent projects, importing OBJ and
  exporting STL in section 2 "Files", and the keys of section 6;
  sixteenth pass: user presets, the preset rows of section 2, the calls of
  section 7, "User presets" in section 8, and the marks of sections 12
  and 14);
  seventeenth pass: the painting tools, "The painting tools" in section 13,
  with the rows of sections 2, 6, 7, 9, 11 and 15 that point to it);
  eighteenth pass: settings of an object, a part and a height range,
  "Sidebar: Process" in section 2, sections 7, 12 ("On an object, a part,
  a height range"), 13 (`scene_edit`, undo, "The object list", "Parts"),
  15 and 16);
  nineteenth pass: "Cut", in section 13 ("Cut"), with the rows of
  sections 2, 6, 7, 9, 15 and 17 that point to it).
- **differs** — built, but on purpose not as the C++ does it; the cell says how.
- **not yet** — not built, with what it waits for. Nothing in this column is
  shown in the page as a control that does nothing.

  **The menu bar's exception.** The menu bar (section 17) holds every
  entry of the reference's menus, as the entry points the features will
  be reached by. An entry whose feature is not built is **shown, off**:
  it can never be chosen (nor its key used), its tooltip in the page's own
  bar says "not yet: " and what it waits for, and it is listed in section
  17. It is never on and doing nothing. Anywhere else the rule stands.

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
| Entering and leaving Device tells the device web view "active" / "inactive"; entering Home refreshes it | `init_tabpanel` page-changed handler | **differs**: the printer's page is a frame (section 4) that is first pointed at the printer when Device is looked at, and then left as it is; there is nothing to tell it |
| Right of the tabs: the slice button with its menu, and the print button with its menu | `MainFrame::create_side_tools` l.2482 | **added**: see section 2, "Slice and export" |
| The menu bar: on macOS the system's, at the top of the screen; on Windows and Linux a dropdown in the custom title bar ("File", "Edit", "View", "Preferences", "Help", calibration's menu) | `init_menubar_as_editor` l.3385, `BBLTopbar` | **added** (fifteenth pass): the system's own in the window, on every system; in a browser (`--serve`) a slim bar of the page's over the strip and the tab bar. Section 17 |
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
| The sidebar (Printer, Filament, Objects, Process) has a width of the window's choosing; it cannot be dragged wider or narrower | `Sidebar::Sidebar`, `ParamsPanel` | **differs** (the owner's choice): its right edge is dragged (or given the keys: Left, Right, Home, End; Enter or a double-click for the usual width) from 300 to 640 px, never leaving the stage less than 420 px; the stage follows it as it is dragged, and the width is kept between runs in `settings.json` (`layout_set`, section 8). `ui/STYLE.md`, "The sidebar's edge" |
| Title bar, right: the gear "settings", which opens Printer Selection (`run_wizard(RR_USER, SP_PRINTERS)`) | l.2588–2591 | **added**: the gear, with the same tooltip |
| Title bar, right: "Synchronize nozzle information" (a U1 or a Bambu printer only): reads the nozzles of the connected printer; "Printer not connected. Please go to the home page or the device page to connect the printer.", "Set Nozzle Diameter", "Nozzle settings synchronized successfully" | l.2406–2577, l.4668 | **not yet**: needs a printer connection (section 4) |
| The printer row: a framed panel (green frame on hover), a 48 px picture of the printer, the preset list; a click anywhere on it opens the list | l.2637–2705 | **added**: the picture, the model's name and the build volume as the head of one card for the printer; a click anywhere on it opens the list |
| The picture: `<printer_model>_cover.png` from the vendor's profiles folder (Snapmaker's from `resources/images` by the preset's name), "printer_placeholder" when there is none | `Sidebar::update_printer_thumbnail` l.12170 | **added**: the cover from the profiles folder for every vendor (`printer_cover`), a grey box when there is none. A project on its own settings shows its printer's picture when that model is among those added |
| The list holds every visible printer preset, in groups with these titles, in this order: "Project-inside presets", "User presets", "System presets". Each group is in order of name (`std::map`: by code unit, capitals first). A system printer is listed once for each `printer_model`, under the model's name, whatever its nozzles | `PlaterPresetComboBox::update`, PresetComboBoxes.cpp l.1528–1749 | **added**: one list (`printers`), the same groups, titles and order; "System presets" has a row for each model added, by model name. Before: "Vendor" and "Printer", listing every preset of every vendor. **added** (sixteenth pass): "User presets", the person's printer presets (section 8, "User presets") whose system printer is of a model added, each by its name with the model's picture; picked, the project goes on it (`printer_select` with `machine`). The group is left out while it is empty, as in the C++ |
| "Project-inside presets": the presets a project file carries that are not the vendor's own | l.1656–1663, l.1722–1730 | **differs**: the row is the printer the project was saved with (`printer_settings_id`) and stands for the project's own settings as a whole, which is how this engine slices a project. It replaces the first pass's "Sliced with: the project's own settings / presets chosen here". Picking it goes back to those settings, the project's own bed type among them; picking a row of "System presets" slices with presets |
| A preset is listed only if visible; a system printer is visible only when its vendor, model and nozzle are among the printers added (`AppConfig` "models"); with one nozzle of a model added, all of them are | `Preset::set_visible_from_appconfig` Preset.cpp l.1060, `PresetBundle::load_installed_printers` l.2127, `install_missing_variants_for_enabled_models` l.2100 | **added** (section 8). A printer a project is on that is not among those added is listed while it is in force, as the C++ makes a preset visible when it is selected (Plater.cpp l.18619) |
| The row shown as selected; the list's tooltip is the preset's own name; a modified preset has "* " before its name; typing filters the list | l.1595–1599, l.1769–1778, `Preset::label` Preset.cpp l.899 | **added**: the selection and the tooltip. **added** (sixteenth pass): "* " before the preset in force while something is set over it (the `*` message id and a space, as GUI_App.cpp l.8490 sets the suffix), in the printer list, each filament's and the process's. Typing jumps to a row, as a list of the system's does. The list is the page's own (`listbox` in `app.js`), so that it can show the groups, each printer's picture and the entry at its foot; its keys are those of a system list (arrows, Home, End, typing, Enter, Escape) |
| Two vendors with a model of the same name are one row (the group is keyed by name) | l.1636–1641 | **differs**: a row each, the vendor in the row's tooltip. One row that picks whichever preset comes first is not what a list of printers is for |
| Picking a model: the preset of that model for the nozzle the project is on, else for 0.4, else the first by name; when the model changes, the nozzles last used on its rack | `Plater::priv::on_select_preset` l.18536–18648, `PresetBundle::get_similar_printer_preset` l.3141, `printer_preset_for_remembered_nozzles` l.4047 | **added** (`printer_select`). **not yet**: the rack's remembered nozzles (the engine has no rack) |
| At the foot of the list: "Select/Remove printers (system presets)" opens Printer Selection; "Create printer" opens "Create Printer/Nozzle". Picked, they leave the list on the printer it was on | l.1751–1762, `OnSelect` l.1188–1222, Plater.cpp l.18547 | **added**: "Select/Remove printers (system presets)". **not yet**: "Create printer" (section 8); it is not listed |
| In the list's own row: the edit icon (opens the printer's settings), "Connect to printer" (opens "Devices"), "The machine has been connected and is currently in working mode" | Plater.cpp l.2659–2686, PresetComboBoxes.cpp l.1907–1985 | **added**: the edit icon at the right of the printer card's head, "Click to edit preset", opens the printer's pages (section 14). **not yet**: "Connect to printer" and the working-mode mark, which need a printer connection (section 4) |
| Leaving a printer, filament or process preset with unsaved changes asks first: "Transfer or discard changes", "You have changed some settings of preset \"%1%\"." and what can be done, the list of the options changed with "Old Value" and "New Value" by page and group, "Transfer", "Discard", "Save" (the save dialog, then the new preset) and closing the box | `Tab::select_preset` Tab.cpp l.9037–9105, `may_discard_current_dirty_preset` l.9441, UnsavedChangesDialog.cpp l.808–1470 | **added** (sixteenth pass; `preset_changes`, `ui/presets.js`): for each of the three lists, a filament's for its own slot; "Cancel" stays on the preset. **differs**: every option listed goes (no tick for each: the C++'s tree has one); the dialog is not asked for when a printer change only carries process settings over (the fork's own carry, below, which it does without asking either); "Remember my choice." is not offered |
| **Another printer, the process**: the one last used with that printer ("Remember printer configuration", on unless turned off); else the one of the same name (alias: the name up to "@") cut for it; else the printer's `default_print_profile`; else the first that fits | `PresetBundle::update_selections` l.2432, `update_compatible` l.7171 (`PreferedPrintProfileMatch` l.7213), Tab.cpp l.9196–9306 | **added** (`printer_select`), in that order, a process that still fits staying before the search by name. **not yet**: the profile remembered for each layer height, the preference itself |
| **Another printer, the process settings changed by hand** are carried over: "Kept %1% process settings from \"%2%\" (revertable)."; each carried value stays marked as changed and can be reverted | `capture_print_settings_carry`, `apply_print_settings_carry` Tab.cpp l.9096, l.9314 | **added**: every process setting set over the preset stays set, with the same sentence in the status line (one setting: "Kept 1 process setting from …"), and each is given back with its own reset or all with the reset in the Process title bar. **differs**: said for a change of process preset too, where the C++ asks ("Unsaved Changes": transfer, discard, save); line widths and layer heights are kept across nozzle sizes, where the C++ lets them "follow the new nozzle size" |
| **Another printer, the filaments**: each one that fits stays; one that does not is replaced by the one of the same name cut for the new printer, else by one of the same `filament_type`, the printer's `default_filament_profile` before others. The filaments last used with the printer are read back and then put aside again for the project's (Tab.cpp l.9251) | `update_compatible` l.7275–7357, Tab.cpp l.9235–9306 | **added**, the same rule for each slot (`Shelf::filament_type` gives the material). A slot with nothing in it yet, as in a new project, takes what was last used in it with that printer, else the printer's usual. **differs**: every slot's usual filament is the printer's first `default_filament_profile`, the engine handing out one |
| **Another printer, the number of filaments**: the larger of the printer's nozzles and the project's filaments; it never shrinks | `TabPrinter::extruders_count_changed` Tab.cpp l.7355, l.3005 | **not yet**: another printer leaves the list as it is. It waits for nothing now that the list can grow ("Add one filament"); it is left out because it would give every new project on a U1 four filaments, which changes what an untouched project is sliced and saved with, and wants deciding |
| **Another printer, the bed**: the plate takes the printer's bed and height, objects are moved to keep their place on it, every plate's slice is dropped | Plater.cpp l.18554–18703, `on_config_change` l.29200 | **added**: the bed and height (`plate_view`), fitted to the stage anew; the slices are dropped (**had**). **not yet**: moving the objects; they stay where the file puts them |
| "Bed type", a label that opens the wiki page on bed types, and a list: the six plates ("Smooth Cool Plate", "Engineering Plate", "Smooth High Temp Plate", "Textured PEI Plate", "Textured Cool Plate", "Cool Plate (SuperTack)"); a Snapmaker U1's own four ("Textured PEI Plate", "Smooth PEI Plate", "Graphic Effect Plate", "Cool Plate"); or a U1's seven with `support_multi_bed_types` | l.2725–2786, `update_all_preset_comboboxes` l.4464–4487, `libslic3r/PrintConfig.cpp` l.1284–1327 | **had**: the same lists and labels (`settled`, `bed_types`). **not yet**: "Graphic Effect Plate" (the engine has no bed temperature for it); the link (the page loads nothing from the network) |
| The list is enabled for a Bambu printer, a U1, or `support_multi_bed_types`. Any other printer is on its own default, shown greyed | l.4503, l.4555–4559 | **had** the rule. **added**: such a printer is put on its default when it is selected |
| A printer's default bed type: the preset's `default_bed_type`, by name or number; else by model: Cool Plate for `BL-P001`, `BL-P002`, `C13`, Textured PEI Plate for `SM_U1`, else High Temp Plate | `Preset::get_default_bed_type` Preset.cpp l.1168 | **added**. **differs**: the model's own file is read for `default_bed_type` too, after the preset. That is where Bambu's profiles give it and the C++ never looks, so its Bambu printers start on the High Temp Plate whatever the profile says (reported as an error of the C++) |
| The bed type is remembered for each printer preset, by its name (`orca_presets[].curr_bed_type` in the app's config, and once more under `app`): saved when it is picked and whenever the selection is written; read when a printer is selected, unless a project is being loaded, whose own bed type then holds. The first time a printer is used its default is saved at once | l.4508–4554, `on_select_bed_type` l.18515, `PresetBundle::export_selections` l.2718, `load_selections` l.2566 | **added**: kept for each printer preset in the app's settings (section 8), read back when the printer is selected, in this run or a later one; a project opened with its own settings keeps its own. Before: not remembered. **differs**: kept by name ("Cool Plate"), not by the number of the C++ list; no second copy under `app`, which the C++ keeps for what reads it elsewhere; a default is not written until the printer is selected by hand |
| A bed type remembered for a U1 that is not one of its four is brought to one of them: Cool Plate for the SuperTack, Textured PEI Plate for the rest | l.4531–4537 | **added**. For any other printer a remembered plate that is not in its list gives the printer's default; the C++ shows the first row and slices with the plate that is not listed |
| Picking a bed type marks the project modified and drops the slices of the plates without one of their own | `Plater::priv::on_select_bed_type` l.18465 | **had** (`project_set`): every slice is dropped, a plate having no bed type of its own here |
| A bed type for one plate ("Same as Global Plate Type" and the printer's plates), reset with a notice when another printer does not take it ("Plate bed types reset") | `PartPlate::get_bed_type` PartPlate.cpp l.196, PlateSettingsDialog.cpp l.394, `Plater::normalize_bed_types` l.21433 | **added** (tenth pass): "Bed type" in "Plate Settings" (section 13, "Plates"), its bed temperatures and `curr_bed_type` the plate's in its slice. Another printer takes away what it does not take (PlateBedTypes.cpp: any type on a Bambu printer or one with `support_multi_bed_types`, a U1's own four on a U1, none on the rest), with the reference's box and words. **differs**: the reset is a step of the undo history ("Plate bed types reset"); the reference takes no snapshot for it |
| Nozzles. One nozzle: "Nozzle" with "Diameter", a list of the sizes the model has presets for ("0.4mm"), which picks the preset; off with fewer than two. Two: "Left Nozzle" / "Right Nozzle". "Flow": "Standard" / "High Flow". "Rack": "Edit" / "Add", "Sync" | `update_nozzle_settings` l.11606–11959 | **added**: "Nozzle", "Diameter" with the sizes of the model's presets, picking among them (`printer_select` with `variant`); off with fewer than two, and for a project's own printer. Before: the size as text. **not yet**: "Flow", "Rack" |
| Several nozzles that may differ (a U1's T1 to T4, a Bambu H2D's L and R): "Nozzles", a tile for each with its size and flow ("0.4", "Std"), "Settings ›"; a tile's menu sets that tool's size in the project and the printer preset stays | l.5720–5819, `Widgets/NozzleStrip.cpp` l.336, `libslic3r/NozzleStripModel.cpp` | **added**, read only: "Nozzles", a tile for each nozzle of the preset with its size. **differs**: "Diameter" is there for these printers too and picks the model's preset for that size, all tools alike, the engine having no size of its own for a tool. **not yet**: a size for one tool, flow, "Settings ›", the tile's menu |
| — | — | The build volume ("270 × 270 × 270 mm") is this app's own line (**had**), now under the printer's name |

### Sidebar: Filament

| C++ | Where | Here |
| --- | --- | --- |
| Section "Filament Management" ("Pellet Configuration" for pellet printers), sub-title "Filaments" | l.2882, l.3515, l.4450 | **added** |
| One row for each filament of the project: a numbered colour swatch ("Click to select filament color"), the filament preset list, the tool badge, the row menu ("Edit", "Delete", "Merge with", "Print on nozzle", "Rack slot"…) | `init_filament_combo` l.4135, `GUI_Factories.cpp::create_filament_action_menu` l.2600 | **added**: a row for each filament the project has: as many as its settings describe, at least the highest anything prints in (`Model::filament_count`; before: up to the highest the file printed in), its number on its colour, its preset list (with the project's own settings, the saved names). The row's edit icon ("Click to edit preset", sixth pass). **added** (thirteenth pass): the swatch is a button, "Click to select filament color", opening a picker of the page's own (the reference's palette with white, greys and black, a square of saturation and brightness over a strip of hue, the colour typed; OK / Cancel); the row menu on the right button or the menu key: "Edit", "Delete" ("Delete this filament", off with one filament), "Merge with" (the other filaments, each by its preset's name and colour). **differs**: the reference's colour dialog also takes several colours to a filament, their make-up and a filament library's colours; one colour here. **not yet**: the tool badge, "Print on nozzle", "Rack nozzle size", "Print on", "Rack slot" (the engine has no assignment of a filament to a nozzle of the person's choosing) |
| "Add one filament" (at most 64, `MAXIMUM_FILAMENT_NUMBER`), "Remove last filament" (shown with more than one) | l.3508–3600, `Sidebar::add_filament`, `delete_filament` l.10223 | **added**, in the list's head, the same words (`filament_edit` `add`, `remove_last`). The new filament is the last one again: its preset (`filament_presets.back()`), what is set over it, and with the project's own settings all its values; its colour the next of the reference's sixteen (`get_next_color_for_filament`, one counter for the window as the reference's is static); what is flushed to and from it worked out from the colours (below). Nothing is asked, as in the reference (it asks only about mixed filaments, which are not read here). A printer with fewer tools takes a fifth filament as the reference does: `T4` is written, with the engine's warning |
| "Delete" of any filament; "Merge with" | `Sidebar::delete_filament` with an index, `change_filament` l.10069, `Plater::on_filaments_delete` l.28076, `PartPlate::on_filament_deleted` l.6798, MixedFilamentConfigRemap.hpp | **added** (`delete`, `merge`): the filaments after it are numbered one less wherever they are named: a part's and an object's filament, the colours painted on (`paint_color`, the triangles' codes rewritten), an object's, a part's and a height range's feature filaments (`wall_filament`, `support_filament`…), the project's own process settings that name one, a plate's first-layer and later-layers orders (the filament taken out of them) and its `filament_maps` entry, a filament change at a layer (one to it taken out; merged, to the filament merged into). What was in it prints in its object's filament (a part), filament 1 (an object), unpainted (a colour); merged, in the filament merged into. Its flushing volumes go with it. **differs**: the flushing volumes are taken out by row and column, where the reference keeps the first n − 1 (see "Errors found in the C++"); a filament change merged goes to the right filament (ditto); a range of later layers whose order is left empty is taken away, where the reference keeps it empty and its dialog then refuses it. **not yet**: a feature filament set by hand over a process preset (`process_set`) is not renumbered |
| "Apply All" with "Filament preset to apply to all filaments", "Set all filaments to the selected preset" | l.3624 | **added** while presets are chosen here (a project on its own settings has none to choose), its list starting at filament 1's preset. **differs** (the owner's choice): it stands above the list of filaments, between the list's head and its first row, where the reference has it under the list; behaviour, undo step and words are the reference's. **differs**: on a printer whose heads carry nozzles of different sizes the reference lists families and gives each slot the variant for its nozzle; the engine has one nozzle size for all tools, so the plain list is offered |
| Each filament's preset list in the reference's groups ("User presets", then "System presets"), compatible filaments only, "* " before one with something set over it | `PlaterPresetComboBox::update`, Preset.cpp `is_compatible_with_printer` l.943 | **added** (sixteenth pass): the person's filament presets that go with the printer (their `compatible_printers` name it or a printer it is made from; none named and no condition: every printer) before the vendor's made for it. Picking another with something set over the slot's preset asks first (above, "Sidebar: Printer"). Saving one from the slot's pages ("Save current Filament") puts the slot on it, and every other slot on the same preset with nothing of its own set, as `update_presets_from_to` moves the plater's slots. **differs**: a preset with a `compatible_printers_condition` and no list is offered where the system preset it is made from is offered: the condition is not worked out, as the vendor's own lists here go by `compatible_printers` alone |
| "Flushing volumes" (shown with more than one filament): `WipingDialog` ("Flushing volumes for filament change") with the matrix, "Re-calculate", the multiplier | l.2903, `WipeTowerDialog.cpp` | **added** (`flush_volumes`, `flush_calc`, `filament_edit` `flush`): in the list's head with more than one filament; a box for each pair from a row's filament to a column's, numbered swatches in the filaments' colours, "From"/"To" in the corner; boxes show the volume times the multiplier, cut to a whole number, as there; "Re-calculate" from the colours; "Flushing volume (mm³) for each filament pair.", "Suggestion: Flushing Volume in range [%d, %d]" (the least any filament flushes, 800); "Multiplier" with two decimals, from 0 to 3, put back with "The multiplier should be in range [%.2f, %.2f]." ("Warning"); a box under its least or over 20000, and the suggestion, in the danger colour. OK is one step ("Flushing volumes"), saved (`flush_volumes_matrix`, `flush_multiplier`), lets the slices go; a project's own volumes and multiplier come over a printer profile's (`project_config`). Seen at 900 × 560 with seven filaments, dark and light, English and German. **differs**: a box not touched keeps its volume, the multiplier at 0 makes the boxes read-only and keeps the volumes, and a box is measured against the least of the filament flushed out (see "Errors found in the C++"). **not yet**: the tip about re-calculating on a change of colour and its preference; multi-colour filaments' several colours in "Re-calculate" (AMS) |
| "Color Mixing Match", "Synchronize filament list from AMS", "Sync Filament Information", "Spool Manager", "Manage filament presets" | l.2936, l.3404, l.3508 | **not yet** |
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
| An option's label, its tooltip with "parameter name: key" under it, its unit after the field | `libslic3r/PrintConfig.cpp`, Field.cpp `get_formatted_tooltip_text` l.109 | **added**, the C++ strings, kept as message ids in the data and shown in the language (section 16). **differs**: a length that may be a percentage ("mm or %") shows "mm" after a plain number and nothing after one typed with its %. **not yet**: the wiki link behind a label (the page loads nothing from the network) |
| Controls by kind: a tick box (`coBool`), a number with its unit (`coInt`, `coFloat`, `coPercent`), a length or a percentage (`coFloatOrPercent`), a list (`coEnum`), a list that also takes a typed value (`i_enum_open`, `f_enum_open`: the infill anchors, the support interface layers), text, a box of text for G-code, scripts and notes | Field.cpp, OptionsGroup.cpp | **added**: the controls of `ui/STYLE.md`; a list is the page's own listbox; a list that takes a typed value is a field with a chevron that opens the values as a menu |
| One line with several options under one label: "Overhang speed" with its four ranges, "Bridge" with external and internal | Tab.cpp l.3703–3714 | **differs**: the line's label and tooltip, then a row for each option under it; four fields do not fit beside a label in a sidebar |
| A value for each nozzle flow variant (the speeds, accelerations and jerks: `coFloats`), picked by the variant selector at the top of the tab | `register_flow_variant_view`, Tab.cpp l.3670 | **differs**: one field, showing the first variant's value and setting every variant alike. **not yet**: a value for each variant (the engine reads the first) |
| A changed option is marked (its label orange) with an arrow that puts it back: "Click to drop current modify and reset to saved value." | Tab.cpp l.10523, `OG_CustomCtrl` | **added**: a bar of the accent on the control, the label in bold, and the arrow with the same tooltip. Here a change is the project's own over the preset (`process_set`): the preset's value is what the arrow goes back to and what an emptied field shows in grey. That is the reference's "dirty" preset (`get_edited_preset` against `get_selected_preset`, `current_is_dirty`): what is set over a preset is what "Save current …" writes into a preset of one's own, what "Transfer or discard changes" lists, and what puts "* " before the preset in its list (sixteenth pass). The C++ edits one preset for the whole window; here each project has its own changes over its presets, kept with the project, as before. **not yet**: the lock beside an option of a preset of one's own whose value is not the system preset's ("Click to reset current value and attach to the global value.") |
| How many options are changed | the preset's "* " mark, the tab's orange title | **added**: a count on each page's tab ("2 settings changed") |
| A page's groups of options ("Layer height", "Line width", "Seam", …) are always open, one under another | `TabPrint::build` (`new_optgroup`), `ConfigOptionsGroup` | **differs** (the owner's choice): each titled group folds from its title (a disclosure button with `aria-expanded`, the keys too), with the number of its settings set by hand on its title, open or folded; "Expand all groups" and "Collapse all groups" beside the search act on the page shown. Folded groups keep their rules and resets; a setting searched for opens its group for the while. Kept between runs in `settings.json` by kind, page and group by their message ids (`layout_set`, section 8); a group never folded or unfolded by hand starts as `src/api/layout.rs` `FOLDED_AT_FIRST` has it (the rarely touched: advanced groups, G-code and notes, "Special mode", experimental and per-nozzle groups, "Tree supports"). The same on a filament's, the printer's and an item's pages (`ui/STYLE.md`, "Groups that fold") |
| The pages' tabs in one row (a notebook's tabs; the pages' tree on a filament's or the printer's tab) | `TabPrint::build`, `Tab::create_preset_tab` | **differs** (the owner's choice): each tab a bounded control on a sunk track, the open one in the accent; where they do not all fit on one row at the sidebar's width they wrap onto two (or more, as few as hold them), shared out evenly and filled to the width, never scrolled or cut; the counts stay on them; Left and Right go along them in reading order across the rows. The same on a filament's, the printer's and an item's pages (`ui/STYLE.md`, "The pages' tabs") |
| "Click to reset all settings to the last saved preset." | Tab.cpp l.2229 | **added**: the arrow in the Process title bar, there while anything is changed |
| A change marks the project modified and drops the slices | `Tab::on_value_change`, `Plater::on_config_change` | **added** (**had** for the four) |
| A value out of range or not a number: "Invalid numeric.", "Value is out of range.", "Value %s is out of range. The valid range is from %d to %d.", "%s can't be a percentage"; the field goes back | Field.cpp l.386–498 | **added**, said in the status line before anything is sent, with the limit; the session holds to the same limits (`process_set`). **differs**: a length over `max_literal` is refused with its range, where the C++ asks whether a percentage was meant. **added** beyond the C++: a value in range that the engine would stop at (a 5 mm layer on a 0.4 nozzle) is refused with the engine's own reason, found by asking it |
| Options shown, hidden and greyed out by what the others are set to | `ConfigManipulation::toggle_print_fff_options` l.757–1441 | **added**: the whole function, every page's rules (`src/api/process/rules.rs`, in the C++ order; `toggle_line` hides, `toggle_field` greys out). The printer's side of them (G-code flavor, Bambu or not, tower type, extruder variants) is read from the settings a slice gets; the number of filaments is the project's. **not yet**: the values the C++ changes by itself while toggling (arc fitting off under extrusion rate smoothing, a multi-line fill back to one line, the reverse threshold to 0%); "Brim ear radius" as the label of "Brim width" with brim ears |
| Settings that need one another: "Spiral mode only works when…", "Change these settings automatically?" and the rest | `ConfigManipulation::update_print_fff_config` l.150–756 | **not yet**: the questions; a value is taken as it is set |
| "Advance" switch: off, the simple options; on, the advanced ones too; a page with nothing to show in the mode is not among the tabs. Developer options with "Develop mode" (Preferences) | ParamsPanel.cpp l.229, `ConfigOptionMode`, GUI_App.cpp l.8520, `Tab::update_visibility` | **added**: the switch, off to start with as in the C++, kept by the page; it reads "Advanced", as the C++ shows it through the `en` catalog (section 16); 54 options without it (no Speed page), 428 with. **not yet**: develop mode, so the 5 developer options are not shown |
| Search ("Search in preset"): finds options by label, jumps to the page and the option | Tab.cpp l.382–420, `Search.cpp` | **differs** (the owner's choice): the field reads "Search for setting" in English (this app's own words, `wording.txt`), here and on a filament's and the printer's pages; in any other language it reads the catalog's words for "Search in preset" ("In Profilen suchen"), and "Search for setting" where a catalog has none (`i18n::search_words`, handed to the page with the language). **added**: every word typed must be in the label, the name (`seam_slope_type`), the group or the page; the list shows label, page › group and name; Enter or a click opens the page, goes to the row and marks it for a moment; Escape clears. An option the rules hide is listed greyed, and its group is gone to |
| What the slicer does with an option | — | this app's own (**added**): a quiet ring beside an option the engine does not read with the settings as they are, a dot beside one it follows with a note, each said in the label's tooltip; nothing is hidden for it and every option can still be set (section 12) |
| The support sets row ("Support set": apply, save, delete) | Tab.cpp l.3776, `support_set_create_widget` | **not yet**: it keeps its sets in files of its own beside the user presets, which this pass does not read or write |
| "Global" / "Objects" switch (`SwitchButton`, `_L("Global")`, `_L("Objects")`); in "Objects" the pages of what the object list has selected: an object (`TabPrintObject`), a part or a modifier (`TabPrintPart`), a height range (`TabPrintLayer`); nothing selected, the panel is hidden | ParamsPanel.cpp l.222, `set_active_tab` l.581, Tab.cpp l.5027–5579, GUI_ObjectSettings.cpp `update_settings_list` l.189 | **added** (eighteenth pass, `ui/objsettings.js`, `src/api/settings/items.rs`): the switch at the head of the section, the pages of the item selected, its name beside the switch. The options an item may set are the reference's (section 12, "On an object, a part, a height range"), the reference's "Frequent" page first; the value shown is the item's own, else its parent's (part and range: the object's; object: the project's); its own is marked as a value set by hand (`.set`), with the arrow "Click to reset current value and attach to the global value."; a change makes it the item's own even when it equals the parent's (Tab.cpp l.5217, `set = true`); the rules are the global pages' (`rules.rs`) on the item's settings. Each change is one step of the history ("Change Option wall_loops", untranslated as the reference's `boost::format`); the reset of all "Reset Options". "Objects" carries the dot of what is set by hand while anything has settings of its own, as the reference colours its word. **differs**: with nothing selected, or several objects or parts, the section says so ("Select an object, a part or a height range…") where the reference hides it; several items at once (the reference's blank fields for values that differ) are not offered; the switch starts on "Global" for every project, where the reference turns to "Objects" by itself after opening a project with settings of objects (`switch_to_object_if_has_object_configs`); the preset row and its cog are hidden in "Objects": an item's own values are not edits of the preset, so they put no "* " on the list, are not what "Save current Process" saves (it saves what is set over the preset, the global pages' alone) and are not asked about when another preset is picked ("Transfer or discard changes"); the reset arrows keep their two meanings (an item's: back to what is over it; the global pages': back to the preset). What the engine does with each option set there is shown as on the global pages, from the engine's own answer (section 12): walls, shells, infill, speeds and the rest followed on an object, a part, a modifier and a height range; a layer height on an object or a range, an object's supports and seam, the feature filaments noted as not followed, with the engine's note. **not yet**: the tool selector |
| "View all object's settings" (the table icon, `ObjectTableDialog` "Object/Part Setting": a grid of every object and part with "Plate", "Name", "Printable", "Filament", "Layer height", "Perimeter", "Infill density(%)", "Support", "Brim", "Perimeter speed", each with a reset), and the menus' "Edit in Parameter Table" | ParamsPanel.cpp l.248, GUI_ObjectTable.cpp (3 350 lines), GUI_Factories.cpp l.3280 | **not yet**: a grid editor of its own size (GUI_ObjectTable.cpp is 3 350 lines, with a side window of settings); every one of its columns can be set from the item's pages now. Not shown |
| The tool selector at the top of the tab ([Plate] [T1 0.4] …) | `create_tool_selector` | **not yet** |
| "Compare presets", save ("Save current …") and delete preset, favourites, "Manage process presets" | ParamsPanel.cpp l.238, Tab.cpp l.300–420 | **added** (sixteenth pass): a cog beside the process list (and in the head of a filament's or the printer's pages) opens "Save current %s", "Rename preset" and "Delete this preset" (both for one's own preset only, as the C++ hides its delete button for a system one) and "Compare presets" (section 8, "User presets"). **differs**: the C++ has save and delete as icons in the tab's head and compare in the title bar; here they are one menu, the head being shared with the "Global / Objects" switch. A project sliced with its own settings has nothing to save: the C++ makes a preset of the project's own settings, which are one whole here (this row's "Project-inside presets" above). **not yet**: favourites, "Manage process presets" (the Process manager window), "Preset Inside Project" in the save dialog (the project would carry presets of its own in its file, which section 15's save does not write), detaching from the parent ("Detach from parent", develop mode) |
| The filament's and the printer's own pages of settings | `TabFilament::build`, `TabPrinter::build` | **added**: section 14, from the same tool, drawn by the same code |

### The bed

| C++ | Where | Here |
| --- | --- | --- |
| A 3D view of every plate with the objects on it, orbit / pan / zoom, selection | `View3D`, `GLCanvas3D` | **added**, in 3D: section 9. Where the browser has no WebGL2, the view from above the first pass built is kept: the bed of the printer chosen with its grid and each object's outline (its convex hull), name and size, drag to move, wheel to zoom, "Fit part" [Z] and "Fit bed" |
| Main toolbar: "Add" [Ctrl+I], "Add plate", "Auto orient all/selected objects" [Q], "Arrange all objects" [A] with its options, "Split to objects", "Split to parts", "Variable layer height", "Assembly View" | `GLCanvas3D::_init_main_toolbar` l.9274 | **added**: "Add", "Add plate", "Arrange all objects" with its options (section 13, "Arrange": the reference's arranger, ported), "Split to objects" and "Split to parts". **not yet**: auto orient (the engine does not do it), variable layer height, the assembly view |
| Gizmos: "Move" [M], "Rotate" [R], "Scale" [S], "Lay on face" [F], "Cut" [C], "Mesh Boolean" [B], "Support Painting" [L], "Seam painting" [P], "Paint-on fuzzy skin" [H], "Color Painting" [N], "Texture displacement", "Emboss" [T], "SVG", "Measure" [U], "Assemble" [Y], "Simplify", "Brim Ears" [E], "Sculpt", "Edit", "Fillet / chamfer (CAD)" | `Gizmos/GLGizmosManager.cpp::init` l.224 | **added**: "Move", "Rotate" and "Scale" with their number fields, and "Lay on face" [F] (section 13); "Support Painting" [L], "Seam painting" [P], "Paint-on fuzzy skin" [H] and "Color Painting" [N] (section 13, "The painting tools"); "Cut" [C]: "Planar" on a flat surface, "Pick face", a part given to the other side, "Dovetail", "Keep cut editable" / "Edit cut..." / "Copy cut to..." (section 13, "Cut"). **not yet**: "Cut"'s curved and drawn surfaces and connectors (section 13, "Cut"), "Mesh Boolean", "Texture displacement", "Emboss", "SVG", "Measure", "Assemble", "Simplify", "Brim Ears", "Sculpt", "Edit", "Fillet / chamfer (CAD)": each waits for the engine to do what it does (joining meshes, displacing a surface, text and pictures made into meshes, measuring and assembling, simplifying, brim ears in the brim) |
| Object list: plates, objects, parts, instances, layer ranges; columns "Name", printable, "Fila.", paint marks; the object, part and plate menus | `GUI_ObjectList.cpp`, `GUI_Factories.cpp` | **added**: plates, objects, parts and instances with "Name", printable and "Fila.", and the object's, the part's and the plate's menus (section 13); the mark of an item with settings of its own, "Layers" and its height ranges (eighteenth pass, section 13). **not yet**: the paint marks |
| View options: "Realistic View", "X-Ray", "Center of Mass", "Show 3D Navigator", Ctrl+0..6 views | `GLCanvas3D.cpp` l.8297, View menu | **added**: the navigator and the views (section 9). **not yet**: the three view options |

### Plates

| C++ | Where | Here |
| --- | --- | --- |
| Every plate shown on the canvas; a click selects it | `PartPlateList`, `Plater::select_plate_by_hover_id` l.30481 | **added**: in the 3D scene, every plate where it stands, a click selecting it (section 9); and the column of plate numbers of the first pass (two plates or more), in Prepare and in Preview |
| Per plate: delete, orient, arrange, lock, "Customize current plate" (bed type, print sequence, spiral vase, filament sequences), name, "Move plate to the front", "Print history" | `PartPlate::render_icons` l.1173, `PlateSettingsDialog.cpp` | **added** (tenth pass): delete, arrange, lock, "Customize current plate", the name and "Move plate to the front", as tools beside the plate shown, in its menu (its tile, its row, the plate in the scene) and kept by a save (section 13, "Plates"). **not yet**: orient (the engine has no orientation), "Print history" (sending is built, section 4, but no history of it is kept) |
| "Add plate", "Duplicate Current Plate", the plate menu ("Select All", "Delete All", "Arrange", "Reload All", "Auto Rotate", "Delete Plate") | toolbar, `create_plate_menu` l.2831 | **added**: "Add plate", "Duplicate Current Plate" (in the plate's menu, there being no Edit menu), and of the menu "Select All", "Delete All", "Arrange", "Delete Plate", "Lock" / "Unlock", "Edit Plate Name". **not yet**: "Reload All" (no edit reads an object's source file again: "Reload from disk" waits for it), "Auto Rotate" (orientation); neither is shown |

### Slice and export

| C++ | Where | Here |
| --- | --- | --- |
| "Slice plate", with a menu: "Slice all", "Slice plate" (and "Filament arrangement..." on dual-nozzle Bambu printers); the button keeps the mode picked | `create_side_tools` l.2482 | **added**: the button and its menu, in the tab bar. Before: one "Slice" button in the sidebar. "Slice all" slices every plate that holds something and is not sliced yet, one after another (`slice` with `all`) |
| "Slice plate" is off while slicing, when the plate's result is valid, or when the plate cannot be sliced; "Slice all" when no plate can be | `MainFrame::get_enable_slice_status` l.2672 | **added**: the same three rules. **not yet**: the blocks for mixed filament temperatures, a cold plate and zero flow ratio |
| Progress as a notification on the canvas with the stage's text, a percentage and a cancel; "Slice ok." when done, "Slicing Canceled" when called off | `SlicingProgressNotification.cpp`, Plater.cpp l.18718, l.19108 | **added**: a note on the stage with the engine's stage, a bar, "Cancel", then "Slice ok." or "Slicing Canceled" (before: a bar in the sidebar). Cancel takes effect between stages, as before |
| The print button and its menu: "Print plate", "Print all", "Print" (send to a print host), "Send", "Send all", "Export plate sliced file" [Ctrl+G], "Export all sliced file", "Export G-code file", "Send to Multi-device", "Send to Bambu Connect"; the last one used is remembered | `print_select_type_label` l.2378, `available_print_actions` l.2397 | **added**: "Export G-code file" for the plate shown (**had** as "Export G-code…"), on when the plate is sliced and its G-code was written. **added** (twelfth pass): the button with its menu, as the reference offers it for a printer that is not Bambu's: "Print" (send to the printer's host, section 4) and "Export G-code file"; the last one picked is remembered (by the page), "Print" not brought back for a printer that cannot be sent to (as the fork's `get_remembered_print_select`). "Print" opens "Send G-code to printer host" for the plate shown. **differs**: "Print host upload queue" is in the button's menu as well (the reference has it nowhere else either for a non-Bambu printer). File > Export > "Export G-code..." is the same export (section 17). **not yet**: the sliced-file (3MF) exports; "Print plate", "Send" and the rest are Bambu's or not offered by the fork for other printers |
| Ctrl+R slices the plate | `MainFrame` char hook l.771 | **added** |
| Warnings and errors of a slice as notifications | `NotificationManager` | **had**, moved to Preview: "About this print" on the stage keeps the three kinds apart (what is wrong with this print; what was settled without being asked; the settings this engine does not follow yet) |

### Files

| C++ | Where | Here |
| --- | --- | --- |
| "New Project" [Ctrl+N], "Open Project..." [Ctrl+O], "Recent projects" | `MainFrame::init_menubar_as_editor` l.3385 | **added** through the strip (section 5) and the File menu (section 17) |
| "Recent projects": wxFileHistory's list in the File menu, the full path of each ("1 /path/file.3mf"), the latest first; at most `max_recent_count` (18 unless set; MainFrame.cpp l.268, AppConfig.cpp l.533); kept as `recent_projects` in `app_config`, numbered "01", "02", …; a project file goes in when it is opened or saved (`set_project_filename`), a model only with the "recent_models" preference; off while the project slices or when it is empty; a file no longer there: "The project is no longer available." ("Error") | MainFrame.cpp l.3416–3434, `add_to_recent_projects` l.5024, `open_recent_project` l.5169 | **added**: kept in `settings.json` (section 8) as `recent_projects`, laid out as the reference lays it out; 18, the latest first, two spellings of one path one entry; a 3MF opened (not one open already) or saved goes first; an STL opened as a project does not (there is no "recent_models" preference). The submenu shows each path, cut at its start where it is long so its file name shows, whole in its tooltip; a file no longer there is struck through and, chosen, says "The project is no longer available." and, the box answered, is taken off the list. **differs**: the reference's box has one button, OK, but removes the entry only on Yes, so it never removes it (section 17, errors). No "Clear": the reference's menu has none (its Home page removes entries one at a time, there being no Home here) |
| "Save Project" [Ctrl+S] ("Save current project to file"), "Save Project as..." [Ctrl+Shift+S] ("Save current project as"): to the project file it was opened from, else (an STL, a new project, a project opened from a ZIP) through the save dialog, "Save file as:", "3MF files", "Untitled.3mf"; written as `SplitModel \| ShareMesh \| FullPathSources`; the project is then that file and saved; "Failed to save the project.\nPlease check whether the folder exists online or if other programs open the project file." | MainFrame.cpp l.3444–3458, Plater.cpp `save_project` l.21961, `get_export_file` l.15728 | **added** (`project_save`, `ui/save.js`): the keys and a project tile's "Save" / "Save as..." (below, section 5), the same rules and words; the file is the reference's project file (section 15). An STL opened as a project is saved as a 3MF beside it, its name offered. And (fifteenth pass) File > "Save Project" / "Save Project as..." (section 17). **differs**: in `--serve` the dialog is the page's own (a path is typed). What the file held that the save would not keep is listed and asked about first (this app's own question, the reference keeping all it reads) |
| "Import 3MF/STL/STEP/SVG/OBJ/AMF/GLB..." [Ctrl+I] adds geometry to the project shown; dropping files on the window; "Open as project" / "Import geometry only" | l.3385, `PlaterDropTarget::OnDropFiles` l.13122 | **added**: File > Import > "Import 3MF/STL/STEP/SVG/OBJ/AMF/GLB..." and "Add" [Ctrl+I] put a 3MF's, an STL's or (fifteenth pass) an OBJ's geometry in the project shown, a 3MF always as "Import geometry only" (section 13). The dialog's filter offers 3MF, STL and OBJ, what the engine reads. The strip's Open still opens a file as a project of its own (**had**), an OBJ too. **not yet**: STEP (a CAD kernel), SVG (an emboss feature), AMF (XML, rarely used), GLB; dropping files; the question which of the two is meant |
| OBJ: `v` and `f` (texture and normal numbers read past, a negative number counted back), a quad cut in two along its first corner, a face of five corners or more refused ("The file contains polygons with more than 4 vertices."), a solid wound inside out turned round; colours from `usemtl`/`mtllib` and vertex colours | `Format/OBJ.cpp` `load_obj`, `objparser.cpp` | **added** (`edge-mesh::obj`): the geometry, as the reference reads it; checked against the C++ slicer's `--info` on all 21 OBJs of its own tests (triangles and bounds, `crates/edge-mesh/tests/reference_obj.rs`). **differs**: a convex face of five corners or more is cut into a fan, as the quad is (only a concave one is refused); a face naming vertex 0 is refused (the reference takes it as its end-of-face mark and reads the faces after it wrongly). **not yet**: colours and materials |
| Export: "Export all objects as one STL...", "Export all objects as STLs...": every object, every copy, its printed parts (no modifier, blocker, enforcer), placed in the project's coordinates, in mm, binary; negative parts cut out (CGAL), else the printed parts alone with "Unable to perform boolean operation on model meshes. Only positive parts will be exported."; "as STLs": into a folder ("Choose a directory"), a file for each object named by it, "(1)", "(2)" after a name taken | Plater.cpp `export_stl` l.25936, `get_export_file` (FT_STL, "Export STL file:") | **added** (`export_stl`, `export_stls`, `edge-engine::export`): the same meshes; written whole or not at all, never over a file without being told (the window's save dialog asks; in a browser the page asks, "Confirm Save As" / "The file %s already exists\nDo you want to replace it?"); a failure says why. Checked against the C++ slicer: its `--export-stl` of `mcbrim_test.3mf` and our "as STLs" give the same 16 objects, each with the same triangle count (85,046) and bounds to 0.001 mm, and its `--info` reads our one STL as 1,360,736 triangles, the sum. **differs**: negative parts are never cut out (no solid boolean here): the objects that have them are named, with the reference's sentence; an object's name is made a file name (what a file system refuses put as `_`), where the reference writes `name` as it is and fails silently on "a/b" (section 17, errors); the copies stand where the project has them, the reference also taking each object's `origin_translation` off in "as STLs" |
| Export: STEP, generic and Bambu 3MF, sliced files (.gcode.3mf), preset bundles | the same | **not yet** (section 17 says what each waits for). `edge save` (the command line) writes the same project file as "Save Project" |

## 3. Preview

| C++ | Where | Here |
| --- | --- | --- |
| The sliced toolpaths in 3D over transparent shells of the objects | `Preview::load_print_as_fff` (GUI_Preview.cpp l.659), `GCodeViewer` | **added**, in 3D: section 10, from the moves of the G-code (`toolpath`, `toolpath_moves`). Without WebGL2: one layer from above, drawn from the engine's own lines (`layer`), as before |
| View type list, in this order: "Line Type", "Filament", "Speed", "Actual Speed", "Acceleration", "Jerk", "Layer Height", "Line Width", "Flow", "Actual Flow", "Layer Time", "Layer Time (log)", "Fan Speed", "Temperature", "Pressure Advance" | `GCodeViewer::update_by_mode` l.1272, `get_view_type_string` l.106 | **added**: "Line Type", "Filament", "Layer Height", "Line Width" (first pass), and in 3D "Speed", "Flow", "Fan Speed", "Temperature" where the print was read back from its G-code (section 10). **not yet**: the views that need each move's time or planned speed ("Actual Speed", "Acceleration", "Jerk", "Actual Flow", the layer times, "Pressure Advance") |
| The view type a print opens in: "Automatic" (Filament with several filaments, else Line Type) or "Last used" ("Default preview view type") | `apply_default_view_type` l.1248, Preferences.cpp l.2038 | **differs**: the last one used in that project, kept with it in the session (`view`); a new project starts in "Line Type". **not yet**: the preference |
| Legend, top right: fold button, G-code window button, the list | `render_legend` l.4120 | **added**: fold button and list; the G-code window's button (eleventh pass), "On/Off G-code window [C]" as its tooltip, the reference's button having none |
| "Line Type" rows in the order of the roles, only those in the print, each with the C++ colour; columns "Line Type", "Time", "%", "Usage", "Display"; a click on a row hides or shows it | l.4300–4645, `libvgcode/src/ViewerImpl.cpp` l.302 | **added**: the C++ order and colours (before: this app's own), a "Display" mark on each row, a click on the row toggles it (**had** as checkboxes). **not yet**: "Time", "%" and "Usage" for each feature; the roles the engine never makes ("Ironing", "Custom", "Support transition", "Wall over support", "Bottom surface over support") |
| "Supported overhangs" / "Unsupported overhangs" overlays | l.4600 | **not yet** |
| Option rows: "Travel", "Wipe", "Retract", "Unretract", "Seams", "Filament changes" | l.4624, `append_option_item` l.4497 | **added** in 3D (section 10) |
| "Filament" view: a row for each filament used with "Model", "Support", "Flushed", "Tower", "Total"; "Filament change times" | l.4417–4864 | **added**: a row for each filament with its colour, to hide or show (**had**), and "Filament change times:". **not yet**: the split of each filament's use |
| "Total estimation": "Total Filament:", "Model Filament:", "Cost:", "Prepare time:", "Model printing time:", "Total time:", "Show stealth mode" | l.5277–5390 | **added** in the legend: "Total Filament:" (m and g) and "Total time:", with how the time was worked out (**had** in the sidebar, as "Filament", "Time", "Travel"). **not yet**: model filament apart from support and tower, cost, prepare time, stealth mode |
| Vertical layer slider at the right: two thumbs (a range of layers), the layer's number over its height beside the thumb, hover shows a layer's time | `IMSlider.cpp` l.890–1230, `get_label` l.1723 | **added** in 3D: two thumbs, the same labels, one layer mode [L] (section 10). Without WebGL2, the first pass's slider: one thumb, always one layer. **not yet**: layer times |
| Up / Down move a layer; five with Shift or Ctrl; the mouse wheel over the slider likewise | `GLCanvas3D` l.5866, `IMSlider.cpp` l.1636 | **added** (before: arrows by one, PageUp / PageDown by ten; those two keys now go round the projects with Ctrl, as in the C++) |
| "Jump to Layer" (the slider's menu; Shift+G): "Please enter the layer number (min - max):" | `IMSlider::render_add_menu` l.1512 | **added**: the slider's menu and Shift+G |
| The slider's other menu entries, for the layer of the thumb taken up: "Add Pause" ("Insert a pause command at the beginning of this layer."), "Add Custom G-code" ("Insert custom G-code at the beginning of this layer."), "Add Custom Template" (only when the printer has `template_custom_gcode`), "Change Filament" with "Filament 1"… in their colours (with more than one filament; off unless the plate prints in one of them and is neither printed by object nor a spiral vase); greyed when printed by object | `render_add_menu` l.1512, GUI_Preview.cpp `update_layers_slider_mode` l.490 | **added** (eleventh pass; `layer_codes`, `layer_code_set`), the same entries, conditions and words. The item made is the reference's (`add_code_as_tick`, `add_custom_gcode`): a pause or a template only where nothing is set, custom G-code in place of whatever is there, a filament change over a filament change |
| A tick where something is set: marks across the track, the kind's label beside it ("Pause", "Custom", "Template", "Color"), its tooltip ("Pause: \"M600\"", "Custom G-code: \"…\"", "Custom Template: \"…\"", "Change Filament"), a delete icon at the selected tick; the slider's menu on a tick: "Delete Pause"; "Delete Custom Template"; "Edit Custom G-code", "Delete Custom G-code"; "Change Filament" and "Delete Filament Change" | `draw_ticks` l.703, `draw_custom_label_block`, `show_tooltip` l.817, `render_edit_menu` l.1569 | **added**: the same, with the reference's words. **differs**: a filament change's mark is in the filament's colour where the reference colours the track above it; the estimated time before a tick's tooltip is not shown (no layer times, see "Actual Speed"); a colour change or an unknown item (read from a file; the reference makes none) gets "Jump to Layer" alone where the reference's menu is empty |
| "Custom G-code": "Enter Custom G-code used on current layer:", a box of six lines, "OK" (off while empty) and "Cancel" | `render_input_custom_gcode` l.1323 | **added**, a dialog of the page's own (`#custom-gcode`); "Edit Custom G-code" opens it with the text that is there |
| What an edit of the ticks does: the plate's items set as a whole, the project modified, the plate's slice made invalid; not a step of the undo history (the handler takes no snapshot, and the undo stack keeps the objects, not `plates_custom_gcodes`) | Plater.cpp l.13532 (`EVT_CUSTOMEVT_TICKSCHANGED`), Model.hpp l.1727 | **added**: the same (`layer_code_set`, `layer_code_delete`); undo and redo leave the items as they are. **differs**: the plate is sliced again at once, so the change is seen; the reference waits for the slice button |
| "Compare slices (A/B)" | IMSlider.cpp l.1300 | **not yet** |
| Horizontal move slider, the tool marker, the position window with its properties | `update_moves_slider` l.2331, GCodeViewer.cpp l.393–760 | **added** in 3D: the slider, the marker (an arrow) and the window's two lines (section 10). **not yet**: the table of a move's properties, a vendor's hotend model |
| G-code window [C]: the lines round the move the moves slider is on, that line framed, each with its number, the command, its words and the comment in colours of their own, a line over 55 characters cut to 52 and "..."; under the legend at the right; shown to start with and remembered (`show_gcode_window`); not scrolled, the wheel over the moves slider moving it | `GCodeWindow::render` l.877, `load_gcode` l.839, GUI_App.cpp l.1364, AppConfig.cpp l.297 | **added** (eleventh pass): the same lines, split and cut the same way, the line found by the move as the reference finds it (the move's own line, `gcode_id`); C and the legend's button; remembered by the page. The colours are the page's (`STYLE.md`). The wheel over the window moves the moves slider as over the slider. **differs**: where the legend leaves too little room under it (fewer than eight lines), the window stands at its left; it stops above "About this print" rather than covering it. How it stays quick: section 11 |
| "Select Plate" bar at the top left: a tile for each plate with its state (not sliced, slicing, sliced, failed); a click selects the plate and slices it if it is not sliced | `GLCanvas3D::_render_imgui_select_plate_toolbar` l.11656 | **added**: the same column of plates as in Prepare, with a tick for sliced, "…" for slicing and "!" for failed; a click selects and, in Preview, slices |
| Its first tile: "Slice all" with sliced/total while some are not sliced, then "All Plates" / "Stats", which opens "Statistics of All Plates" (each filament's use, "Total time:", "Total cost") | l.11656–11730, `render_all_plates_stats` l.3295 | **added**: the tile in both states; the statistics list each plate's filament and time and their totals. **not yet**: by filament, cost |
| Canvas warnings: paths beyond the plate, beyond the height, conflicting paths | GLCanvas3D.cpp l.12998 | **differs**: the engine refuses to write G-code with toolpaths off the bed; the reason is shown under "About this print" |
| "Auto slice after changes", "Dim lower layers in the G-code preview" | Preferences.cpp l.2135, l.2046 | **not yet** |
| "Export Toolpaths as OBJ" | MainFrame.cpp l.4417 | **not yet** |

## 4. Device

| C++ | Where | Here |
| --- | --- | --- |
| The tab "Device" holds one of three panels, by the printer preset selected: Bambu's monitor (`MonitorPanel`: "Status", "Storage", "Update", "HMS") for a Bambu printer on Bambu's network; Flashforge's own pages; for every other printer a web view (`PrinterWebView`) | `MainFrame::show_device` l.1906, `Sidebar::update_all_preset_comboboxes` (Plater.cpp l.4576), `libslic3r/PresetBundle.cpp::use_bbl_device_tab` l.723 | **added**: the tab follows the printer of the project shown (its preset, or the printer the project was saved with) |
| The web view loads the printer's own page: `print_host_webui` if the preset has it, else `print_host`, with `http://` put in front; a Snapmaker U1 gets Snapmaker's bundled device page, driven over MQTT or a websocket; a device picked from the per-model "Devices" list comes before the preset's address | Plater.cpp l.4304–4430, `MainFrame::load_printer_url` l.5286, `Utils/PrintHostDevices` | **added** (twelfth pass): the printer's page at "Device UI", else "Hostname, IP or URL", `http://` put in front, in a frame of its own (below, "The printer's page in the app"); the connection kept for the printer, else the preset's. The frame is first pointed at the printer when Device is looked at. **differs**: a U1 shows its own page at its address, not Snapmaker's bundled page (that page and its MQTT pairing are not in the source). **not yet**: the "Devices" list |
| With no address: a page "Printer Connection" — "Please set up your printer connection to view the device." | `resources/web/orca/missing_connection.html` | **added**: the same words, the preset's facts, and "Connect to printer", which opens "Physical Printer" |
| The API key is put into every request of the page (`X-API-Key`) | `PrinterWebView::SendAPIKey` l.473 | **not yet**, on purpose: the key is never handed to a page the app does not control. What it would take: a page that needs the key (a Moonraker with keys on, for its own page) would be shown through the session, which would fetch the printer's page and its requests itself and add the header, a proxy for one origin; or in the window, a second web view of the printer alone with a request hook (Tauri's `on_web_resource_request`, not on every platform). The page is told only whether a key is set, never the key |
| Bar over the page: "Printer page" / "Storage" (the G-code files on the printer: "Name", "Print time", "Filament", "Size", "Modified"; "Print", "Delete"), and "Printer:" to pick among several printers of the model | `PrinterWebView.cpp`, `Utils/PrinterStorage.cpp` l.337, `PrinterStoragePanel.cpp` | **added** for a U1 (Moonraker): "Printer page" / "Storage", the files with "Name", "Size", "Modified", "Print" and "Delete" (each asked first: "Delete \"%s\" from %s?\nThis cannot be undone."), "Refresh"; "Print" refused while the printer prints, as `PrinterStorage` refuses it. The address, a button to load the page again, and "Connect to printer" in the bar. **not yet**: "Print time" and "Filament" (`/server/files/metadata` for each file), sorting, the details pane, "Printer:" (one printer for each preset here); "Storage" for the Centauri Carbon (SDCP commands 258, 259) |
| What the tab shows of the printer itself | the preset | **added**: name, vendor, model, nozzles, bed and height, G-code flavor, bed type in force, "Host Type" |
| Where the address is set: "Physical Printer" ("Print Host upload": "Host Type", "Hostname, IP or URL", "Device UI", "Authorization Type", "API Key / Password", "HTTPS CA File", "User", "Password", "Test", "Browse ..."), and the "Devices" list for several printers of one model | `PhysicalPrinterDialog.cpp` l.127, l.596, `PrintHostDevicesDialog.cpp` | **added** (twelfth pass, `ui/hosts.js`): "Physical Printer" with "Print Host upload": "Host Type" (the types this app can send to, "Octo/Klipper" and "Elegoo Link", and the one in force if it is another, marked "(not yet)"), "Hostname, IP or URL" with "Test", "Device UI", "API Key / Password" (a field that shows only that a key is set; "Clear"), "HTTPS CA File" with "Browse ..." and the reference's hint; the reference's tooltips; "Test" tries what is typed before it is kept and says "Success!" and the host's own sentence, or its failure. Opened by "Connect to printer" on the Printer card (the fork's icon, `set_connection_tooltip`) and from Device. **differs**: the icon opens this dialog; in the fork it opens "Devices" with "Edit connection..." inside. "Use the preset's connection" takes a kept connection away. There is no name to save it under (the reference saves a physical printer preset; here it is kept for the printer preset, below). **not yet**: "Authorization Type", "User", "Password" (shown by the reference for PrusaLink only, which is not built), "Browse ..." for the address (Bonjour discovery), "Devices", "Printer" (`printhost_port`, Repetier and Obico), "Ignore HTTPS certificate revocation checks" (Windows) |
| Where it is kept: the user's printer preset, and `<datadir>/hub/print_host_devices.json`; never the project (a saved 3MF has `print_host`, `print_host_webui`, `printhost_apikey` and the rest taken out, and they are ignored on load) | `libslic3r/Preset.cpp` l.1394, l.2710, `PresetBundle::full_config_secure` l.3220 | **differs**: kept for each printer preset in this app's data folder (section 8), the key in a file of its own; never in a project, which this app never writes them to. A printer without a kept connection uses its preset's (forty-seven vendor presets carry an address, for example Volumic's `192.168.0.60`) |
| Sending: "Send G-code to printer host" ("Upload", "Upload and Print", the file name, "Switch to Device tab after upload."), and "Print host upload queue" | `PrintHostDialogs.cpp` l.515, l.1532, l.2358 | **added** (twelfth pass): "Send G-code to printer host", "Upload to Printer Host with the following filename:", the name offered as the reference offers it (the printer's `filename_format` filled in, `{print_time}` with the slice's time, folded to ASCII, the stem selected), "Please do not include the special characters …", the two questions before sending ("The filename '%s' contains special characters …", "Upload filename doesn't end with \".gcode\". …"), "Unload filaments after print" for Snapmaker's tool changers (ticked as the preset's `unload_filaments_at_end`), the Centauri Carbon's "Time-lapse", "Heated Bed Leveling", "Textured Build Plate (Side A)" / "Smooth Build Plate (Side B)" (the side the project's bed type means, and "The selected bed type does not match the file. …" when another is picked; the two ticks remembered), "Switch to Device tab after upload." (remembered), "Upload", "Upload and Print", "Cancel". The bytes sent are those "Export G-code file" writes. The queue: "ID", "Progress", "Status" ("Queued", "Uploading", "Error", "Canceling", "Canceled", "Completed"), "Host", "Size", "Filename", "Message", "Cancel selected", "Show error message", "Close"; one file after another; an error is shown as it happens ("Error uploading to print host:"). **differs**: the Centauri Carbon's options are in the same dialog under its name, with the two buttons (the reference's Elegoo dialog has an "Upload and Print" tick and one button); the queue says when a print was started. **not yet**: the "Group" and "Upload to storage" lists (Repetier, PrusaLink), the QIDI, Creality and Flashforge options, a toolhead map chosen by hand (the U1 is sent each filament on its own toolhead); "Ongoing uploads" asked when the window closes |
| Bambu monitor: camera, "Printing Progress", "Pause" / "Resume" / "Stop", temperatures, AMS, speed; needs Bambu's network plug-in | `Monitor.cpp`, `StatusPanel` | **not yet**, and not planned before the print hosts |
| Next to Device: "Stream" (cameras), "Printers" (every printer at a glance), the mobile hub | `StreamPanel`, `PrintersPanel`, `RemoteHub.cpp` | **not yet** |

### Sending to a printer: what is built

The session talks to the printer; the page still talks only to the session
(option 1 of the plan this section held before; option 3, the printer's
page inside the app's page, was taken for the printer's own page alone,
isolated as below). The requests are made by `src/hosts/` in the desktop
app (not an engine crate), behind one trait (`Host`: `test`, `upload` with
a start and its progress, `files`, `delete`, `print`), by
`ureq` (HTTP, rustls) and `tungstenite` (a plain websocket). Nothing is
asked of a printer unless the person asks ("Test", "Upload", "Upload and
Print", Device); nothing looks for printers on the network.

| Host type | Reached as | "Test" | "Upload" | "Upload and Print" | Ported from |
| --- | --- | --- | --- | --- | --- |
| "Octo/Klipper" (`octoprint`), a printer that is not a U1 | OctoPrint's API at the address, `X-Api-Key` | `GET api/version`: `api` in it, `text` (if any) starting "OctoPrint"; Moonraker's OctoPrint compatibility answers it | "Test", then `POST api/files/local`: `print`, `path`, `file` | the same with `print=true` | `Utils/OctoPrint.cpp` |
| "Octo/Klipper" with a printer model naming "Snapmaker" and "U1" | Moonraker's own API at the address (port 80 on a U1), `X-Api-Key` if a key is set | `GET server/info` (`result`), `GET access/info` (a printer that wants a login is refused) | `POST server/files/upload`: `print=false`, `root=gcodes`, `file` | `GET printer/objects/query?print_stats` first (printing or paused: refused, nothing sent), the upload, `GET server/files/metadata` (the size must be the file's), `print_stats` again, the toolhead map (`SET_PRINT_EXTRUDER_MAP`… each filament on its own toolhead, `SET_PRINT_USED_EXTRUDERS`, `SET_PRINT_PREFERENCES … [END_UNLOAD_FILAMENT=[…]]`) by `POST printer/gcode/script?script=`, then `POST printer/print/start?filename=` | `GUI/SnapmakerLan.cpp`, `GUI/RemoteSend.cpp` l.2390, `GUI/SnapmakerTaskConfig.cpp`. The fork's own desktop path for a U1 is Snapmaker's bundled page over MQTT (`send_flow_for`), whose upload code is not in the source |
| "Elegoo Link" (`elegoolink`), Centauri Carbon | its web server at the address; its websocket at `ws://<host>:3030/websocket` (SDCP) | OctoPrint's test, else `GET /` naming "ELEGOO" | OctoPrint's upload when it answers as OctoPrint; else `POST uploadFile/upload` in parts of 1 MiB (`Check`, `S-File-MD5`, `Offset`, `Uuid`, `TotalSize`, `File`), each answered `code: "000000"` | SDCP command 0 first (not idle: refused, nothing sent), the upload, command 0 each second while the printer checks the file (`CurrentStatus` 8, at most 60 s), command 128 (`Filename "/local/<name>"`, `Calibration_switch`, `PrintPlatformType`, `Tlp_Switch`), its `Ack` read (the reference's sentence for each) | `Utils/ElegooLink.cpp`, `Utils/ElegooStartPrint.hpp`. The Centauri Carbon 2 (`ElegooCc2`) is refused: not yet |

Everything is timed (10 s to connect, 15 s for a question, 60 s for each
write of a file and 120 s for the answer to it, 60 s for a print to
start); a file being sent is called off between pieces of 64 KiB.

The other host types of the reference (`PrintHost.cpp::make_print_host`),
not yet: PrusaLink and PrusaConnect (`OctoPrint.cpp`, digest
authorisation; about 600 lines), Duet (279), FlashAir (233), AstroBox
(175), Repetier (295, groups and printers), MKS (142, with `TCPConsole`),
ESP3D (175), CrealityPrint (604), Obico (219, a cloud), Flashforge (796),
SimplyPrint (492, a cloud with OAuth). None is used by the printers this
project is tested on; each is a class behind the same trait.

### The printer's page in the app

What is allowed, and why. `CLAUDE.md` says the page "loads nothing from the
network": that is about the app's own code and assets, and stays so. The
printer's own page, at the address the person typed, is the feature, and is
the one thing from the network the page shows:

- It is a frame (`#device-frame`) of another origin than the app's page,
  so it cannot reach the page's document, its `call`, or the session's
  token; `sandbox="allow-scripts allow-same-origin allow-forms allow-popups
  allow-downloads"` (same-origin is the printer's own origin, which its page
  needs for its own requests and storage; never top navigation),
  `referrerpolicy="no-referrer"`.
- The app's own document keeps its policy: scripts, styles, connections
  and images from itself only. In `--serve` the policy is sent with the
  page and names, in `frame-src`, only the origins of the connections in
  force when the page is loaded (each kept connection and each open
  project's preset address); a connection saved to a new address loads the
  page again (`hosts.js` `framable`; the session keeps every project as it
  was). In the window the policy is Tauri's, fixed when the app is built,
  so it cannot name the printer: `frame-src http: https:` (no `data:`,
  `blob:` or other schemes), everything else as before. The window's
  commands are given to the window's own page only (`capabilities/default.json`
  names no remote URL), so a framed page cannot call them.
- The API key is never put into the frame (above, `X-API-Key`).
- The frame is pointed at the printer only when Device is looked at.

Not checked in the window itself (the macOS web view and the Windows one may
treat an `http:` frame in the app's page as mixed content); in a browser
through `--serve` it is checked.

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
| Ctrl+PgDn / Ctrl+PgUp go to the next / previous project, round the ends; on macOS also Cmd+Shift+] / [ and the Window menu ("Next Project", "Previous Project", the projects by name) | `cycle`, `neighbour_id`, MainFrame.cpp l.799, l.4268 | **added**: the keys (`project_cycle`) and, on macOS, the Window menu (section 17). In a browser (`--serve`) Ctrl+N, Ctrl+O and Ctrl+PgDn belong to the browser; in the app's window they are the page's |
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
| Strip menu: "Show thumbnails" / "Show names only", "Default width", "Collapse the project strip" / "Show the project strip" | `show_strip_menu` | **added**. "Language", which the strip's menu held while there was no Preferences, is now in Preferences (section 16, 17) |
| The right edge is dragged to widen the strip: 88 to 200, names alone at least 120; remembered | `StripLayout`, `on_motion`, `project_strip_width` | **added**, remembered by the page |
| The chevron folds the strip to a 24 wide rail: New and Open stacked, a mark for each project (filled for the active one, a ring when sliced, the unsaved dot); remembered | `set_collapsed`, `on_paint` | **added** |
| Closing the window with several projects open: one dialog, "Unsaved projects", "These projects have unsaved changes. Save the ticked ones before closing?", every unsaved one ticked, "Save" / "Don't save" / "Cancel"; with one project, "EdgeSlicer - Save": "The current project has unsaved changes, save it before continue?" (Yes / No / Cancel) | `confirm_close_all`, MainFrame.cpp l.604, Plater.cpp `close_with_confirm` l.25050 | **added** in the window: it is held open while a project is unsaved (`unsaved`), the page asks in these words ("EdgeSlicerRS - Save"), saves what is ticked and then closes it. **not yet**: "Remember my choice."; in a browser (`--serve`) nothing is asked, the session outliving the page |
| "Reopen the open projects at the next start" (off unless asked for): the projects written out at exit and read back | `save_session_for_restart`, `restore_session`, `SessionManifest` | **not yet**: projects can be written now (section 15); the preference and the manifest are not built. Reloading the page (`--serve`) does find every project as it was, the session being the server's |
| Project files opened from Explorer / Finder go into the strip ("Open project files from Explorer / Finder in": "New project tab" / "The current project") | `open_files_from_shell`, GUI_App.cpp l.9202 | **added** for a file named when the app is started (`startup`, **had**). **not yet**: a file handed to an app that is already running |

## 6. Shortcuts

| C++ | Here |
| --- | --- |
| The keys are one table, the menu bar's (`src/api/menu.rs`), bound once (`ui/menu.js`): in the window the system's menu bar holds every key with Ctrl (Cmd on a Mac) but Cut, Copy and Paste, and sends its id; the page reads the rest, and in a browser all of them. A key typed into a text field is the field's, but New, Open, Save, Save as, Quit and the next and previous project; Cut, Copy and Paste are the field's, or the text's when text is selected. The bindings that were in `app.js`, `save.js`, `scene-edit.js` and `gl/stage.js` (N, O, S, Shift+S, Cmd+Shift+] / [, Z, Ctrl+Z, Y, I, C, V, K, Del, Ctrl+0..6, Ctrl+A, Ctrl+E, Esc, C) moved there. "Keyboard Shortcuts" [?] lists them (section 17) | **added** (fifteenth pass) |
| Ctrl+N "New Project", Ctrl+O "Open Project" | **added** |
| Ctrl+PgDn / Ctrl+PgUp, Cmd+Shift+] / [: next / previous project | **added** |
| Ctrl+R "Slice plate" | **added** |
| Tab: "Switch between Prepare/Preview" | **added** (on the stage) |
| Up / Down: the layer slider; Shift or Ctrl with them: five at a time | **added** (Up / Down by one **had**) |
| Shift+G: "Jump to Layer" | **added** |
| Z: zoom to the objects | **added**: "Fit in all view" in 3D (section 9); "Fit part" from above |
| Ctrl+S "Save Project", Ctrl+Shift+S "Save Project as..." | **added** (and Cmd on a Mac), anywhere in the page but in a dialog; in a browser they are taken from the browser's own |
| Ctrl+P (Cmd+, on a Mac): Preferences; "?": the list of shortcuts; Ctrl+D: "Delete all"; Ctrl+X: "Cut"; Ctrl+Q: Quit (Windows and Linux; Cmd+Q in the application menu on a Mac) | **added** (fifteenth pass) |
| Ctrl+G ("Export plate sliced file"), Ctrl+Shift+G ("Print plate") | **not yet**: the sliced file is not written; "Print plate" is Bambu's. Ctrl+G's entry is shown, off |
| Left / Right, Home / End (the move slider), L (one layer mode), C (G-code window) | **added** in 3D, C in the eleventh pass |
| A, Shift+A, Q, Shift+Q, the gizmo keys, Ctrl+0..6, Ctrl+A / D / Z / Y / C / V / X, Del, 1–9, arrows to move objects | **added** in 3D: Ctrl+0..7 (the views), Ctrl+A, Esc, I and O, Ctrl+E (section 9); A, Shift+A, M, R, S, F, C, Ctrl+Z, Ctrl+Y, Ctrl+C, Ctrl+V, Ctrl+K, Ctrl+I, Del and the arrows (section 13; C is "Cut" in Prepare and the G-code window in Preview, as in the reference, and Up / Down move the cut plane while "Cut" is open); L, P, H, N (the painting tools), and while one is open its own keys: C, S, F, T, H, G for its tools, 1–9 and 1 then 0–6 for the colour tool's filaments, Ctrl and Alt with the wheel (section 13, "The painting tools"; where each is bound, section 17). **not yet**: Q and Shift+Q, the other gizmos' keys, 1–9 (the filament of what is selected, with no painting tool open) |

## 7. What the page asks for

Every call takes JSON and answers JSON, but the seven at the end of the table,
which answer bytes (`Session::call_bytes`; section 11); an error is a sentence
to show. A call about a project takes `project` (its id) and means the one
shown when it is left out; one about a plate takes `plate` and means the plate
the project is on.

| Call | Takes | Answers |
| --- | --- | --- |
| `startup` | — | `{ model }`: a file named when the app was started |
| `language` | `system?` (the system's preferred languages, the page's `navigator.languages`) | `{ language, catalog, languages: [{ code, name }], table, search }`: the language the page is shown in (chosen and kept at the first run, section 16), its table as `tools/make_translations.py` wrote it, and the words of the settings pages' search fields in it (section 16) |
| `language_set` | `language` (a code of `languages`) | as `language`; kept. An error for a code not listed |
| `layout` | — | `{ sidebar_width, sidebar_min, sidebar_max, stage_min, groups: [{ kind, page, group, open }], folded_at_first: [{ kind, page, group }] }`: the page's layout as it was left (the sidebar's width dragged to, null for the usual one; the settings pages' groups folded or unfolded by hand) and the limits and first folds it holds to (`src/api/layout.rs`) |
| `layout_set` | `sidebar_width?` (whole pixels from `sidebar_min` to `sidebar_max`, or null); `groups?` (`[{ kind, page, group, open }]`: `kind` process, filament, printer or item, `page` and `group` message ids of that kind's pages, `open` true, false or null for as at first) | as `layout`; kept at once. Refused out of range, or for a group the pages have not got, nothing changed |
| `library`, `set_profiles { dir }` | | `{ dir, vendors }`. The folder set is kept for the next run |
| `printers` | `project?` | `{ first_run, installed: [{ vendor, brand, model, name, cover, variants: [{ variant, machine }] }], selected: { source: "preset" \| "project", vendor, machine, model, variant, added } \| null, project_printer, last: { vendor, machine } \| null, settings: { file, problem }, user: [{ name, vendor, model, variant, base }] }` (the person's printer presets; `selected.user` says the one in force is one of them) |
| `printer_catalogue` | — | `{ models: [{ vendor, brand, model, name, nozzle_diameter, x, y, z, toolheads, cover, enabled }], unread }` |
| `printer_cover` | `vendor`, `model` | `{ data }`: the picture as a `data:` URL, or null |
| `printers_save` | `project?`, `models: [{ vendor, model }]` | as `printers`, and `added`, `removed` (`[{ vendor, model }]`), `switched: [{ project, name, from, to }]`. An error with none: "At least one printer must be selected." |
| `printers_default` | `project?` | as `printers_save`: adds the Snapmaker U1 when no printer is added |
| `printer_select` | `project?` and `vendor` with `model` (one of those added), `variant?`; or `vendor` with `machine` (a printer preset of one's own); or `variant` alone (another nozzle of the model the project is on); or neither (the printer a project starts with); `initial?` | as `project`; and `notice { title, text }` when plates' own bed types the printer does not take were taken away ("Plate bed types reset"); so does `project_set` given `source`, `vendor` or `machine` |
| `machines { vendor }` | | `{ machines }` |
| `presets { vendor, machine }` | | `{ processes, filaments, user_processes, user_filaments, usual_process, usual_filament }`: the vendor's made for the printer (for a printer preset of one's own, for the system printer it is made from) and the person's that go with it |
| `projects` | — | the strip: `{ active, max_open, projects: [{ id, name, path, dirty, empty, plate, tab, revision, slicing, plates_with_objects, plates_sliced }] }`. `revision` counts the edits to the project's model |
| `project` | `project?` | a strip entry and `file`, `plates: [{ id, name, filaments, objects, sliced, failed, locked, own { bed_type, print_sequence, spiral_mode, first_layer_sequence, other_layers_sequence: [{ begin, end (null: "End"), order }] } (null or empty: the project's), customized (whether it sets any of them) }]`, `sliced`, `failed`, `has_settings`, `used`, `filament_count` (as many filaments as its settings describe, at least the highest anything prints in), `colours` (each filament's; null for a model with no settings whose filaments were never changed), `saved { printer, process, filaments }`, `choices { source, vendor, machine, process, filaments, bed_type, over { key: value }, filament_over [{ key: value }], printer_over { key or key#n: value }, layer_height, first_layer_height, wall_loops, closing_radius }` (`over` is every process setting set over the preset, `filament_over` each filament's own from filament 1, `printer_over` the printer's; the four after them are four process settings under their older names), `view`, `edit { revision, undo, redo }` (what there is to take back and to make again, by name, or null), `status`. All of it is of the model as it stands, edits included |
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
| `user_presets`, `presets_reload` | — | `{ dir, printer, process, filament: [{ name, inherits }], unread: [{ file, reason }] }`: the person's presets (section 8, "User presets"), as read at the start or read again from the disk; `unread` the files there that could not be taken as presets, left as they are |
| `preset_check` | `project?`, `kind` (`process`, `filament`, `printer`), `filament?` (from 1), `name?`, `preset?` (the one renamed), `rename?` | `{ kind, name, suggested, current, level (ok, warn, invalid), message, title }`: what the save or rename dialog says of a name as it is typed, in the reference's words and order (`validate_preset_name`); `suggested` is "<name> - Copy" for a system preset |
| `preset_save` | `project?`, `kind`, `filament?`, `name`, `accept?` | as `project`, and `saved: true`, `preset`: the project's preset of that kind with what is set over it, saved as one of the person's and chosen in its place with nothing over it. Or `{ saved: false, ask }` when the name is one of theirs (`accept: true` saves over it). Refused with the reason: a name that cannot be had, a project on its own settings, a session with no data folder |
| `preset_rename` | `project?`, `kind`, `preset`, `name` | as `project`: every open project and what is remembered with each printer name it so. Refused: a system preset ("System presets cannot be renamed."), a name taken |
| `preset_delete` | `project?`, `kind`, `preset`, `confirm?` | without `confirm`, `{ deleted: false, title, ask }` (the reference's question, "Used by:"); with it, as `project` and `deleted: true`. Refused: a system preset, one others inherit (the reference's message with their names) |
| `preset_changes` | `project?`, `kind`, `filament?` | `{ kind, filament, name, count, pages: [{ title, extruder, groups: [{ title, rows: [{ key, label, old, new }] }] }] }`: what is set over the preset, old and new, by page and group |
| `preset_compare` | `project?`, `kind`, `vendor?`, `all?`, `left?`, `right?` | `{ kind, vendor, system, user }` (what can be compared: those offered for the project's printer, or with `all` every one of the kind); with `left` and `right`, also `count` and `pages` as `preset_changes` (`old` the left's value, `new` the right's) |
| `preset_import` | `path` or `paths` (`.json`, `.zip`, `.orca_printer`, `.orca_filament`), `overwrite?`, `keep_untrusted?` | `{ imported: true, printer, filament, process, passed, removed, title, message, presets }`; or `{ imported: false, title, ask, conflicts }` ("Overwrite config", before saving over one of theirs: `overwrite` true or false answers it), or `{ imported: false, title, ask, remove, keep, untrusted }` ("Untrusted settings in file": `keep_untrusted` answers it). For the menu bar to call: no entry of the page's own |
| `preset_export` | `path?` (a folder), `type` (`printer_bundle`, `filament_bundle`, `printer`, `filament`, `process`), `names?`, `replace?` | without `path`, the choices `{ types: [{ type, title, note }], printer_bundle, filament_bundle, printer, filament, process }`; with it, `{ exported: true, files, message }`, or `{ exported: false, file, ask }` when the zip is there (`replace` answers it). For the menu bar to call |
| `settled` | `project?` | `{ printer: { source, name, vendor, model, variant, nozzle_diameter, bed, height, gcode_flavor, host_type, print_host, print_host_webui, has_api_key, bed_types { enabled, in_force, choices } }, process: { name, layer_height, first_layer_height, wall_loops, closing_radius } }`, either null when nothing is chosen |
| `plate_view` | `project?`, `plate?` | `{ project, plate, bed, height, objects: [{ name, outline, box, height }] }` |
| `slice` | `project?`, `plate?` or `all: true` | `{ project, run }`; `status` says how it goes. The model is sliced as it stands, not the file. A plate with an object over its edge is refused in the reference's words |
| `cancel` | `project?` | — |
| `status` | `project?` | `{ project, run, state, label, step, steps, plate, at, of, error }`, `state` being idle, slicing, done, failed or cancelled |
| `summary` | `project?`, `plate?` | as before, and `plate`, `widths`, `heights`. `gaps` ends with each setting set by hand that the slice did not read |
| `layer` | `project?`, `plate?`, `index` | as before |
| `export` | `project?`, `plate?`, `path` | `{ path, bytes }`; written whole or not at all (fifteenth pass; it was written in place) |
| `print_host` | `project?` | the connection of the project's printer: `{ printer (the preset's name), model, from ("kept", "preset"), connection { host_type, print_host, print_host_webui, printhost_cafile, has_api_key }, host_types [{ value, label, built }], built, can_send, why_not, can_start, storage, device_url, route ("octoprint", "snapmaker", "elegoolink", …), offers { elegoo, unload { default } \| null }, problem }`; `{ printer: null, host_types }` without a printer. Never the key |
| `print_host_set` | `project?`, `host_type`, `print_host`, `print_host_webui`, `printhost_cafile`, `apikey?` (a new key; "" or null clears it; left out keeps it); or `forget: true` (back to the preset's) | as `print_host`. Refused, nothing kept: a host type that is not one, an address that cannot be read |
| `print_host_test` | `project?` and the fields of `print_host_set` (as typed; what is left out is as kept) | `{ ok, message }`: the host's own sentence ("Connection to OctoPrint is working correctly.", "Could not connect to Moonraker: HTTP 401: Unauthorized") |
| `print_send_name` | `project?`, `plate?` | `{ name }`: the name "Send G-code to printer host" offers |
| `print_send` | `project?`, `plate?`, `name`, `start`, `options { unload_at_end, timelapse, leveling, smooth_plate }` | `{ id, queue }`: the plate's G-code (the bytes `export` writes) put in the queue |
| `print_queue` | — | `{ jobs: [{ id, name, host, printer, size, start, state (queued, uploading, error, canceling, canceled, completed), said, progress (0–100), stage, message }], busy }` |
| `print_cancel` | `id` | as `print_queue`: a job waiting is not sent, one going is stopped |
| `print_queue_clear` | — | as `print_queue`, without the jobs that are over |
| `printer_files` | `project?` | `{ files: [{ name, size, modified }] }`, newest first; an error for a printer whose files cannot be listed here |
| `printer_file_delete` | `project?`, `name` | as `printer_files` |
| `printer_file_print` | `project?`, `name` | `{ started: true }`; refused while the printer prints |
| `scene` | `project?` | the project for Prepare in 3D: `{ project, key, bare, bed { box, shape, height, model, texture }, plates: [{ id, name, origin, locked }], plate, colours, objects: [{ id, name, mesh, vertices, triangles, bytes, filaments, filament, parts }], placings: [{ id, object, plate, printable, matrix (16 numbers, column by column), box, over }] }`. An object also has `own_filament` (the filament it is set to itself, or null: what its parts set to "default" print in), `part_list: [{ id, name, kind (part, negative, modifier, support_blocker, support_enforcer, precise_seam), filament, own (the part's own filament, or null for "default"), matrix (16 numbers, column by column: the part's own coordinates to the object's), box (round the part's own mesh), triangles ([first, how many] of the object's mesh), split (whether "Split" can be done to the part), settings (how many settings of its own it has that a part's pages offer) }]`, `settings` (as many of the object's own that an object's pages offer), `ranges: [{ min_z, max_z, layer_height, settings (its own besides its layer height) }]` (its height ranges, lowest first) and `split: { objects, parts }` (whether "Split to objects" and "Split to parts" can be done to it). A part's `id` stays its own through every edit that leaves it there. An object's `id` and a copy's (a placing's) `id` stay theirs through every edit; `mesh` is the number of the object's mesh, the same for as long as the mesh is; `filament` is the object's own when its parts agree; `over` says a copy reaches over the edge of its plate |
| `scene_edit` | `project?`, `op` and what it takes (section 13) | `{ project, scene, done, made, note, part }`: the project and its scene as they then are, the edit's name in the history, the copies it made, something to tell the person or null, and the part it made (`part_add`) or null |
| `item_settings` | `project?`, `object`, and `part` (a part's or a modifier's id) or `range` (a height range's place among the object's, from 0) | the settings of that object, part or height range: `{ project, kind: "process", item { kind (object, part, range), object, part, range, name, heights { min_z, max_z } \| null }, keys (what it may set, in the pages' order), values { key: value } (in force from over it: the object's for a part or a range, the project's for an object), set { key: value } (its own), changed, hidden, off (the rules, on its own settings), unread, unread_note (one sentence for all: what the engine does with them), why { key: sentence } (when they differ), notes, counts { page: number set } ("Frequent" among them), refused }`. Its own are changed by `scene_edit` (`item_set` and the rest, section 13) |
| `undo`, `redo` | `project?` | `{ project, scene, done }`; an error when there is nothing to take back or to make again |
| `filament_edit` | `project?`, `op`: `add` (`colour?`, "#RRGGBB"; else the next of the reference's palette), `remove_last`, `delete` (`filament`), `merge` (`filament`, `into`), `colour` (`filament`, `colour`), `apply_all` (`preset`) | as `scene_edit`: `{ project, scene, done }`, `done` the step's name ("Add one filament", "Remove last filament", "Delete this filament", "Merge with", "Change Filament Color", "Apply All"), or null when nothing changed. Refused, nothing changed: the last filament, a 65th, a filament the project has not got, a merge into itself, a colour that is not `#RRGGBB`, "Apply All" on a project on its own settings |
| `scene_faces` | `project?`, `copy` | the faces "Lay on face" offers for a copy, largest first: `{ project, copy, faces: [{ normal, area, drawn }] }`, `normal` outward in the object's own coordinates (what `lay_on_face` takes), `area` in mm² at the printed size, `drawn` the outline to draw over the face (rounded and lifted as the reference draws it), in the object's own coordinates |
| `cut_section` | `project?`, `copy`, `plane` (sixteen numbers, column by column: the plane's x, y and normal, then its middle, in the scene) | `{ project, copy, lines, above, below, reach }`: where the plane cuts the copy's printed parts (six numbers a line, in the scene), how far the copy reaches over and under the plane along its normal (the grip's tooltip), and the box round it in the plane's own axes, least and greatest corners ("Build Volume"); with `cap: true`, `cap` too: the cut face as triangles (nine numbers each, in the scene), for the connectors. A bare model is measured where it is shown (section 13, "Cut") |
| `cut_pick` | `project?`, `copy`, `from`, `to` (a line of sight in the scene) | `{ point, normal, corners }` of the face of the copy's printed parts it meets first, or null ("Pick face") |
| `cut_parts` | `project?`, `copy`, `plane`, the cut's options | `{ count, upper, ranges, offset }`: the pieces the cut would make, the side each is on and its triangles in `cut_parts_mesh` (bytes, EDGM), for a piece to be given to the other side |
| `cut_connector_mesh` | `type`, `style`, `shape`, `snap?` | bytes (EDGM): a connector's shape at unit size, as the cut makes it, for the tool to draw |
| `cut_draw_hits` | `project?`, `copy`, `plane`, `rays` (each `[x, y, z, dx, dy, dz]` in the scene) | `{ hits, snap }`: where each ray first meets the copy's printed parts, a sample `[x, y, z, nx, ny, nz, facet]` in the plane's frame, or null for a miss or a face met from inside (`draw_sample_at`); `snap` the radius a stroke closes the loop within |
| `cut_draw` | `project?`, `copy`, `plane`, `drawn` (the chain and the sweep, as `scene_edit` `cut` takes it), `thickness?`, `thickness_from?`, `op?` (`append` with `stroke` and `at`, `close` with `path`, `finish_open`, `points` with `points`) | `{ drawn, done, path, closed, valid, error, empty, folds, clipped, flips, snap, surface }`: the chain after the step, the finished line put back onto the copy, why there is no cut surface, which side it leaves empty, the panel's notes, and the surface's triangles in the plane's frame (`refresh_draw_stroke`) |
| `cut_recipe` | `project?`, `copy`, `onto?` | how the copy's object was cut when it was kept editable, set up on `onto` (the copy itself when not given): `{ plane, mode, thickness, thickness_from, keep_upper, keep_lower, to_parts, place_on_cut_upper, place_on_cut_lower, flip_upper, flip_lower, groove }` ("Copy cut to...") |
| `toolpath` | `project?`, `plate?` | a sliced plate's moves, before they are asked for: `{ source: "gcode" \| "lines", moves, move_size, at_once, layers { first, z, height }, features, filaments, counts { travel, line, wipe, retract, unretract, seam, tool_change }, box, plate, run, gcode_lines }` (`gcode_lines`: how many lines the G-code has, 0 without one) |
| `gcode_lines` | `project?`, `plate?`, `from` (a line, from 1), `count` (at most 2048) | `{ total, from, lines: [text] }`: the G-code's lines as the G-code window shows them (no new line; over 55 characters cut to 52 and "...") |
| `layer_codes` | `project?`, `plate?` | what is set at the plate's layers and what the slider offers there: `{ project, plate, items: [{ print_z, kind (pause, custom, template, filament, color, unknown), extruder, color, extra }], pause, template (the printer's `machine_pause_gcode` and `template_custom_gcode`, as text), colours (each filament's), change_filament (null with one filament, else whether it can be picked), sequential }` |
| `layer_code_set` | `project?`, `plate?`, `print_z` (a layer's top, as `toolpath` gives it), `kind` (`pause`, `custom` with `extra`, `template`, `filament` with `extruder`) | as `layer_codes`, and `detail` (as `project`). An item added or changed as the reference's slider does it; the project unsaved; the plate's slice let go. Refused, nothing changed: a layer the slice has none at, a pause or a template where something is set, custom G-code with no text, a template the printer has none of, a filament change not offered, a plate printed by object |
| `layer_code_delete` | `project?`, `plate?`, `print_z` | as `layer_code_set`; an error when nothing is set there |
| `menu` | what the page knows: `tab`, `selected`, `pasteable`, `window` (the app's window, not a browser), and the scene's `perspective`, `auto_perspective`, `gcode_window`, `navigator`, `labels` | the menu bar as it stands, in the language (section 17): `{ mac, menus: [{ id, label, enabled, items: [{ id, label, key (as shown), bind { command, shift, key }, accelerator (the window bar's, or null where the page binds the key), enabled, checked (null for none), radio, tip, waits } \| { line: true } \| a menu]}], recent: [{ path, missing }], preset_calls }`. In the window `call` also makes the system's menu bar of it (`src/menubar.rs`) |
| `shortcuts` | — | `{ title, sections: [{ title, rows: [{ key, does, built }] }] }`: "Keyboard Shortcuts", the reference's lists with the keys as this system shows them |
| `recent_remove` | `path` | `{ recent }`: the recent projects without it, kept |
| `export_stl` | `project?`, `path` (".stl" added), `replace?`; or `suggest: true` | `{ path, bytes, triangles, uncut }`; `{ exists: true, path }` when a file is there and `replace` was not given; with `suggest`, `{ suggested }`. An error with no objects: "There are no objects in the project." |
| `export_stls` | `project?`, `dir` | `{ files, uncut }`: a file for each object, named by it, none written over |
| `paint_open` | `project?`, `copy`, `kind` (`supports`, `seam`, `fuzzy_skin`, `color`), `filaments` (how many, for `color`) | opens a painting tool on the copy's object, its printed parts' paint read into the session's selectors: `{ project, object, parts: [{ id, triangles, filament }], max_state, ms }`. One tool at a time; an object with no printed part is refused |
| `paint_close` | — | `{}`: the tool's selectors let go |
| `scene_mesh` (bytes) | `project?`, `mesh` (its number in `scene`) or `object` (its place in the list) | an object's mesh, `EDGM`: 24 bytes of header, vertices (`f32` × 3), triangles (`u32` × 3), a mark byte for each triangle when they differ |
| `bed_model` (bytes) | `project?` | the vendor's model of the printer's bed, as `scene_mesh` |
| `bed_texture` (bytes) | `project?` | the vendor's picture of the bed: the SVG or PNG file as it is |
| `toolpath_moves` (bytes) | `project?`, `plate?`, `from`, `count` | a stretch of moves, `EDGT`: 24 bytes of header, then 32 bytes a move (where it ends, speed, height, width, turn, temperature, layer, what it is, feature, filament, fan) |
| `toolpath_lines` (bytes) | `project?`, `plate?`, `from`, `count` | the line of the G-code each of a stretch of moves was read from, `EDGL`: 24 bytes of header as `toolpath_moves`', then a `u32` a move (from 1) |
| `paint_mesh` (bytes) | — | the painted parts' own triangles, unpainted, in the object's coordinates: a `u32` count and a `u32` number of parts, then 48 bytes a triangle (three vertices of `renderer.js`'s 16 bytes: `f32` x, y, z, the normal as three `i8`, a mark: the part's filament) |
| `paint_event` (bytes) | `project?`, `matrix` (the copy's), `eye`, `clip` (null or `{ normal, offset }`), `op` (`hover`, `paint`, `commit`, `erase_all`, `gap`, `gap_fill`, `unmark`, `refresh`) and what it takes (`api/paint.rs`): `ray` / `rays` (origin and direction in the world, the newest first), `state`, `tool`, `radius`, `height`, `angle`, `highlight`, `area`, `name` | a `u32` length and that much JSON (`{ hit { part, facet, point, normal }, all, states, marked, note, done, edited, band, blocks, ms }`, padded to 4), then blocks of painted pieces: for each a `u32` part, a `u32` block (1024 of the part's triangles), a `u32` count and that many triangles of 48 bytes, their mark the state (128 more for what a fill has marked). A block sent replaces the page's; with `all` every block is sent. `commit` and the panel's buttons make the step of the history and answer `edited` (as `scene_edit` does) |

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
| With changes to a preset unsaved: "Configuration package changed" (keep or save) | `apply_config` l.821–828 | not needed: what is set over a preset is the project's, and stays with it through a change of printers added |
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
| A printer's connection: in its physical printer preset (`print_host`, `print_host_webui`, `printhost_apikey`, `printhost_cafile`, …), and several printers of one model in `<datadir>/hub/print_host_devices.json`, keys and passwords there in plain text (PrintHostDevices.cpp l.173) | `PhysicalPrinterDialog::OnOK`, `Utils/PrintHostDevices.cpp` | **differs**: a connection is kept for each printer preset by its name (user printer presets, sixteenth pass, do not carry one yet), in `print_hosts.json` beside `settings.json` (`src/hosts/kept.rs`), under the reference's keys so a later pass can take each entry over as a physical printer of that preset; renaming a printer preset of one's own does not carry its connection yet. The key is kept apart, in `print_host_secrets.json`, made readable by this user alone (0600; on Windows the folder is the user's own `%APPDATA%`), and is never answered to the page, logged, put in an error or in a project. Not the system's keychain: a build of this app is not signed, so macOS would ask each new build for leave to read its own entry, and the keychain would not follow `EDGE_DESKTOP_DATA`; it can take the file's place behind `Kept::key` and `Kept::set` |
| The language: `app.language`, the canonical name of the one chosen ("de_DE"); at the first run the system's, written at once | GUI_App.cpp `load_language` l.8279, Preferences.cpp l.274 | **added**: `app.language` in `settings.json`, the same name. **differs**: a system language with no catalog is not kept (the reference keeps it and then shows the bare message ids); English is |
| The send dialog's choices: `recent` (`open_device_tab_post_upload`, `elegoolink_timelapse`, `elegoolink_heated_bed_leveling`, …) | `PrintHostDialogs.cpp` l.2998 | **differs**: kept by the page (`localStorage`), as the print button's last action |
| The recent projects: `recent_projects` | AppConfig.cpp `set_recent_projects` l.1642 | **added**: `recent_projects` in `settings.json`, `{ "01": path, "02": path, … }` as the reference keeps them, read back in the order of their numbers (section 2, "Files") |
| — | — | **differs** (the owner's choice; this app's own): the sidebar's width as it was dragged, `layout.sidebar_width` in `settings.json`, and the settings pages' groups folded or unfolded by hand, `layout.groups` (`{ kind: { page: { group: true or false } } }`, by message ids), kept by the session (`layout_set`) so the window and a browser (`--serve`) find them alike; nothing is kept for the usual width or a group never touched |
| The page's own: the strip's width and folding, the last file opened | — | **had**, kept by the page (`localStorage`). The printer, process and filaments last used were kept there too; they are in `settings.json` now, and the old entries are no longer read |

### User presets

Read from PresetBundle.cpp (`load_user_presets`, `import_presets`), Preset.cpp
(`PresetCollection::load_presets`, `save_current_preset`, `Preset::save`,
`save_info`, `load_info`, `rename_user_preset`, `delete_current_preset`,
`validate_preset_name`, `is_compatible_with_printer`), Tab.cpp (`save_preset`,
`delete_preset`, `may_discard_current_dirty_preset`), SavePresetDialog.cpp,
UnsavedChangesDialog.cpp (and its `DiffPresetDialog`), PresetComboBoxes.cpp,
PresetManagerDialog.cpp and CreatePresetsDialog.cpp (`ExportConfigsDialog`).

| C++ | Where | Here |
| --- | --- | --- |
| A person's presets live in the data folder: `user/<user id, or "default" without an account>/machine`, `filament`, `process`, each `<name>.json` with `<name>.info` beside it; one that inherits from nothing in `<kind>/base/` | `load_user_presets` PresetBundle.cpp l.1090, `path_from_name` Preset.cpp l.4402 | **added** (`edge_config::user`, `src/api/presets.rs`): the same layout and format under this app's data folder (`EDGE_DESKTOP_DATA`, else the folder `settings.json` is in), `user/default/`. A folder copied from the C++ application's data folder (its `user/default`), or into it, is read by both: checked with files written by the C++'s code as written out by hand in the tests, and the other way with the C++ build of the pinned commit (below). **differs**: only `default`; a folder of an account's id is not read (there is no account here) |
| A preset's JSON: only what differs from the preset it inherits (`Preset::save` with the parent's config, keys the reference does not know left out, a flow-variant list of more than one kept), its name twice (`name` and `print_settings_id` / `filament_settings_id` / `printer_settings_id`), `inherits`, `from: "User"`, `is_custom_defined: "0"`, `version` (the vendor's, "02.03.03.14" written "2.3.3.14"); every value text or a list of texts; the keys in order, four spaces a level, a new line at the end (nlohmann `dump(4)`) | `Preset::save` Preset.cpp l.797, `ConfigBase::save_to_json` Config.cpp l.1977 | **added**, written so; a test compares the text byte for byte with what the C++ writes. A text of several lines is written as the text (the reference writes a string option raw) |
| The `.info`: `sync_info`, `user_id`, `setting_id`, `base_id` (the parent system preset's `setting_id`), `updated_time`, a `key = value` line each; "null" reads as nothing; "hold" is not written | `save_info` l.754, `load_info` l.712 | **added**. **differs**: a new preset gets the time it is saved; the reference writes 0 and fills it in at its next start |
| A file is written beside and renamed over (`write_file_atomically`); one that does not parse is moved to a quarantine folder (upstream deleted it) | Preset.cpp l.1735, `PresetQuarantine.cpp` | **added**: written whole or not at all. **differs**: one that does not parse is left where it is and listed (`user_presets` `unread`); nothing is moved or deleted |
| A file of another kind in a folder, or a second file of a name already read, is passed over; a file with no version that reads is passed over too | l.1745–1766 | **added** for the first two, each listed as unread with why. **differs**: a file without a version is read |
| A preset of one's own is laid over the preset it inherits, which may be another of one's own; one of the name of a system preset is not loaded | l.1772–1822 | **added**: followed through any chain into the vendor's presets; a name the vendor has is the vendor's |
| Offered where its `compatible_printers` name the printer, or, for a printer of one's own, the system printer it was made from (one step up only: "FIXME only check the parent now"); none named and no condition: every printer | `is_compatible_with_printer` Preset.cpp l.943, `is_compatible_with_parent_printer` l.933 | **added**, the whole chain up. **differs**: a `compatible_printers_condition` is not worked out (the vendor's lists here go by the list alone): such a preset is offered where the system preset it is made from is |
| "Save preset": "Save %s as", the name ("<name> - Copy" for a system preset), checked as it is typed: "Name is invalid;" with "illegal characters:" and `<>[]:/\|?*"`, "Name is unavailable." (the default presets' names), "Overwriting a system profile is not allowed.", "Preset \"%1%\" already exists." (or "… and is incompatible with the current printer.") with "Please note that saving will overwrite this preset.", empty, a space first or last; OK off while it cannot be had | SavePresetDialog.cpp l.208–270, Preset.cpp `validate_preset_name` l.3488 | **added** (`preset_check`, `preset_save`), the same checks in the same order. A filament made from a system preset that names no printer is made for the printer it is saved on (Orca's rule). Saving over one of one's own keeps its file and its `.info` ("update") |
| "User Preset" / "Preset Inside Project", "Use for every printer", "Detach from parent" | SavePresetDialog.cpp l.140–205 | **not yet**: a preset inside the project (section 15's save would have to carry presets of the project's own); detaching (develop mode) |
| Renaming one's own preset (the managers): the same checks, refused for a system preset; written under the new name before the old files go; presets laid over it follow | `rename_user_preset` Preset.cpp l.3384, PresetManagerDialog.cpp l.441 | **added** (`preset_rename`), from the lists' cog menu ("Rename preset"). What is set over it in a project stays set over it, so the C++'s refusal of a preset with unsaved changes is not needed. **not yet**: renaming a filament's family at once |
| Deleting (above, "Not built"'s row of printers, for every kind) | `Tab::delete_preset` | **added** (`preset_delete`) |
| Import ("Import Configs") and export ("Export Preset Bundle": `.orca_printer`, `.orca_filament`, zips of printer, filament or process presets) | PresetBundle.cpp `import_presets`, CreatePresetsDialog.cpp `ExportConfigsDialog` | **added** in the session (`preset_import`, `preset_export`, `edge_config::bundle`), for the menu bar to call. **differs**: asked once for the whole import (overwrite all or none), not for each preset; a file that is not a preset stops the import before anything is written |
| Cloud sync of user presets (`sync_info`, `setting_id`), favourites, the Filament and Process managers | `PresetBundle::load_user_presets(config, my_presets)`, PresetManagerDialog.cpp | **not yet**: there is no account; the managers are their own windows |

How it was checked against the C++: the C++ build of the pinned commit
(`EdgeSlicer --datadir <a copy of this app's data folder> --list-presets`)
lists the process, filament and printer preset this app saved, each with
its `inherits` and the printers it goes with; sliced by it
(`--printer-preset "Snapmaker U1 (0.4 nozzle)" --process-preset "Strong walls"
--filament-presets "My PLA"`) its G-code has `print_settings_id = Strong
walls`, `filament_settings_id = "My PLA"`, `wall_loops = 3`,
`sparse_infill_density = 25%` and `nozzle_temperature = 212`, the values the
app saved. This app slicing the same model with those presets writes G-code
identical, but for the names, to the same model sliced with the system
presets and the same three values set by hand. The other way, the tests read
files laid out as `Preset::save` and `save_info` write them.

#### Errors found in the C++ (user presets)

| Where | What is wrong | What a person sees |
| --- | --- | --- |
| Tab.cpp l.10072 | `_L_PLURAL("The following presets inherit this preset.", "The following preset inherits this preset.", count)`: the singular and plural are the wrong way round | With one preset inheriting, "The following presets inherit this preset."; with several, "The following preset inherits this preset." Here the count picks the sentence that fits |
| Tab.cpp l.10124 | "Are you sure to %1% the selected preset?" is filled with `_L("Delete")`, a capitalised button word | "Are you sure you want to Delete the selected preset?" (kept here: the message id is the reference's and its catalogs translate it so) |
| Preset.cpp l.937 | `is_compatible_with_parent_printer` looks one step up ("FIXME only check the parent now") | A printer preset made from another of one's own is offered no process or filament made for the system printer. Here the chain is followed |
| Preset.cpp l.1763–1764 | A user preset whose `version` does not read is passed over with nothing said | The preset is gone from the lists. Here it is read |
| Preset.cpp l.746, Tab.cpp `save_preset` | A new preset's `.info` is written with `updated_time = 0`, put right at the next start | Nothing at once; the time is the next start's, not the save's. Here the save's |
| Snapmaker_Orca.cpp l.787 with l.3299 | `--printer-preset` with a printer preset of one's own writes it flattened with `"inherits": ""`, so the printer's system name is lost and every process is "not compatible" | The CLI refuses to slice ("process not compatible with printer", exit 239) with any process and a user printer |
| CreatePresetsDialog.cpp l.371 (`get_filament_name`) | `substr(0, index_at - 1)` drops the character before the "@" | "PLA@U1" is exported as "PL" (the bundle's name and the `.orca_filament` file's) |
| CreatePresetsDialog.cpp l.3966–3969, l.4082–4085 | After `initial_zip_archive`, a `continue` leaves the writer open | A broken, unfinished archive on the disk, with "Export successful" said |
| CreatePresetsDialog.cpp l.4089 with l.4562 | A filament made from a preset whose name has no "@" gets no vendor and is left out of the `.orca_filament`; the "vendor" folder is the first word after "@" | A bundle that may hold only `bundle_structure.json`, said to be exported |
| CreatePresetsDialog.cpp l.3686, l.3704 | The question calls a zip a "folder"; `find(".zip")` searches the whole path | A wrong word; a folder named with ".zip" in it puts the time stamp in the wrong place |
| PresetBundle.cpp l.1382–1392 | A preset whose parent comes later in the same archive is skipped ("can not find inherit preset") | A lower count of imported presets, nothing said of which |
| PresetBundle.cpp l.1344 | An imported file whose version does not read is skipped | As above |

### Not built: filaments, creating and deleting printers

| C++ | Where | Here |
| --- | --- | --- |
| "Filament Selection" (the guide's last page): the filaments of the printers ticked, to tick by "Printer", "Filament type" and "Vendor", the models' `default_materials` ticked when none is ("Do you want to use default filament ?"); kept as the `filaments` section; a filament list shows only those | `22/22.js`, `SaveProfile` l.704–715, `Preset::set_visible_from_appconfig` | **not yet**: a filament list here offers every filament preset made for the printer. It waits for nothing in the engine (`PrinterModel::default_materials` is there); it is its own pass |
| "Add/Remove filaments" at the foot of each filament list opens the Filament manager (`run_preset_manager`), not the guide | PresetComboBoxes.cpp l.1756, GUI_App.cpp l.9598 | **not yet**, with the above |
| "Create printer" opens "Create Printer/Nozzle": "Create Printer" or "Create Nozzle for Existing Printer"; vendor and model ("Select Vendor", "Select Model", "Input Custom Vendor", "Input Custom Model", "Can't find my printer model"), "Nozzle Diameter", "Bed Shape", "Printable Space", "Origin", "Hot Bed STL", "Hot Bed SVG", "Max Print Height"; then "Create from Template" or "Create Based on Current Printer" with the filament and process presets to make; "Printer Created Successfully" | `CreatePrinterPresetDialog`, CreatePresetsDialog.cpp l.1769–3490 | **not yet**: a printer preset of one's own is now saved from the printer's pages (section 8, "User presets"); a new model or nozzle with its bed and the presets made for it waits for this dialog |
| Deleting a user printer preset (the delete button of the printer's settings, "Delete this preset"): never a system or default one; "Are you sure to delete the selected preset?"; "%d Filament Preset and %d Process Preset is attached to this printer. Those presets would be deleted if the printer is deleted."; "Presets inherited by other presets cannot be deleted!"; then the next preset by name is selected and the project goes to it | `Tab::delete_preset` Tab.cpp l.10030–10159, l.8978 | **added** (sixteenth pass, `preset_delete`): refused for a system preset and for one others inherit; "Are you sure to %1% the selected preset?" with "Used by:" and where the open projects use it; the projects on it go to the system printer it was made from. **not yet**: "%d Filament Preset and %d Process Preset is attached…", which is for a printer that stands alone ("Create printer") |


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
| Alt: "Select a part"; Shift: "Select objects by rectangle"; a double click opens the object's settings; the right button's menus | l.6423–6551, l.6871 | **added**: Alt with a click selects the part under the pointer, Alt with Shift adds it to the parts of that copy selected or takes it away, and Alt with a drag moves the part within its object (section 13, "Parts"); the right button's menu on an object, on the parts selected and on a plate. **differs**: Shift with a click adds to what is selected, as Ctrl does. **not yet**: the rectangle, object settings |
| Dragging an object moves it on the bed | l.6553–6608 | **added** (section 13) |
| Hovering changes no colour (the hover colours are commented out) | 3DScene.cpp l.362 | the same |
| With a painting tool open, the buttons paint on the object (the left, the right, Shift with the left to erase), Ctrl with a button turns or moves the view, off the object the buttons turn and move the view; no object is dragged; the wheel with Ctrl sizes the brush, with Alt moves the section view | `GLGizmoPainterBase::on_mouse`, `gizmo_event` | **added** (section 13, "The painting tools") |
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
| Arrows, PgUp / PgDn, Del, Ctrl+C / V / X / Z / Y / D / K, 1–9, A, Q, the gizmo keys, "?" | **added**: the arrows, Del, Ctrl+C / V / Z / Y / K, A, and M, R, S, F, C of the gizmo keys (section 13). **not yet**: PgUp / PgDn (turning what is selected), Ctrl+X, Ctrl+D, 1–9, Q, the other gizmos, "?" |

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
| Icons beside each plate: "Remove current plate (if not last one)", "Auto orient objects on current plate", "Arrange objects on current plate", "Lock current plate", "Customize current plate", "Move plate to the front", "Print history", "Edit current plate name" | `render_icons` l.1154 | **added** (tenth pass): a column of the page's own tools at the far right corner of the plate shown, with these tooltips ("Unlock current plate" on a locked one; a dot of the accent on "Customize current plate" when the plate sets anything, as the reference draws its changed icon). **differs**: beside the plate shown only, not every plate; HTML over the scene, placed as the scene is drawn and hidden while the corner is out of sight. **not yet**: orient, "Print history" |
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
| The G-code window [C]; ticks on the layers slider (pause, custom G-code, template, filament change) | l.877, IMSlider.cpp l.1512 | **added** (section 3) |
| "Compare slices (A/B)" | IMSlider.cpp l.1300 | **not yet** (section 3) |

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
- **The G-code window holds no file.** Each move's line of the G-code is
  read with the moves, once a slice (`Toolpath::lines`), and the session
  keeps where each line of the file ends; the page asks for the moves'
  lines once a print, a stretch at a time (`toolpath_lines`, 4 bytes a
  move: 0.7 MB for the 180 473 moves of `mcbrim_test.3mf`'s plate 1, in
  about 50 ms), and only when the window is on. The text comes as JSON in
  blocks of 256 lines (`gcode_lines`, about 20 ms each through `--serve`
  with a debug build) as they come into sight: two blocks at most on
  their way at once, so a slider dragged fast asks only for what is then
  in sight, the next block asked for while it is quiet, the last 48
  blocks kept. The window's rows are made once and filled again; drawing
  the window with the scene, for a move anywhere on that plate, took
  1.3 ms (median of 300, 4.9 ms at most) in the app's browser pane.
- **Nothing is rebuilt to change what is seen**: the layers shown and the
  moves slider are a range of moves; the view type, the colour range and what
  is hidden are numbers given to the shader.
- **Paint is the session's.** While a painting tool is open the session
  holds a selector for each printed part (`edge_mesh::selector`, the
  reference's `TriangleSelector`) and a tree of boxes round its triangles
  (`edge_mesh::raycast`). The page sends the lines of sight under the
  pointer (`paint_event`, a pixel apart along a drag) and gets back, in
  bytes, the painted pieces of the blocks of 1024 triangles whose paint
  changed, which replace what it drew for those blocks; the object's own
  triangles come once (`paint_mesh`). The alternative was the selector in
  the page: the same port in JavaScript, a second tree of boxes built in
  the page from the mesh it already has, and the codes sent to the
  session to be saved. It was not taken, by what was measured: the work
  per event is small enough that the round trip is the cost, and the
  codes stay where they are saved. See the table below.
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

Painting the figurine (`jolteon_cmp.3mf`: one object of five parts,
1 411 438 triangles; the largest part 696 152), the same machine. In the
engine, release build (`EDGE_PAINT_BENCH=… cargo test --release -p
edge-engine --test paint -- --ignored`), on the largest part: its
neighbours and normals 30 ms, its tree of boxes 96 ms, a selector 5 ms;
a line of sight 1 µs (400 cast); a dragged stroke's step (a capsule
between two hits) 0.15 ms on average and 1.0 ms at most with a 1 mm
brush, 0.66 ms and 1.8 ms with a 5 mm one; writing the part's codes
0.7 ms; a smart fill 6 ms and putting it on 1.2 ms. Through the page
(`--serve`, the app's own crate in a debug build, the engine optimised):

| | Figurine |
| --- | --- |
| Opening "Color Painting" (`paint_open`: five selectors and trees) | 0.60 s |
| The object's own triangles (`paint_mesh`) | 67.7 MB, 0.95 s; the tool is open 2.1 s after the key |
| A step of a stroke, three lines of sight (`paint_event` `paint`): in the session / round trip | 5–6 ms / 6–8 ms (20 steps); 6 to 10 blocks of 1024 triangles sent again |
| The button let go (`commit`: the codes written into the model, a step of the history, the scene answered) | 0.16 s, after the stroke |

A step takes less than a frame of the screen, so a stroke follows the
pointer; pointer moves that come while one is on its way go together in the
next (a line of sight a pixel apart from where the last ended).

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

What is set on these pages is the project's own over its preset; since
the sixteenth pass it is also what the reference calls the "dirty" preset:
"* " before the preset in its list, listed by "Transfer or discard
changes" when another preset is picked, and written into a preset of one's
own by "Save current Process" (section 2, "Sidebar: Process"; section 8,
"User presets"). The values a preset of one's own shows are its own laid
over the system preset it is made from.

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
| Others | 51 | 28 | 1 (`exclude_object`) | 22 | 13 | 22 |
| All | 433 | 224 | 1 | 208 | 57 | 181 |

Turned on, three more are read with a note: `spiral_mode`, `raft_layers`,
`draft_shield`; and `print_sequence` (read since the tenth pass) has one at
"by object", which is not followed: every object is printed together, layer
by layer. `zaa_enabled` turned on is followed, and its
`zaa_speed_scaling` then has a note while layers are slowed to cool.

Nothing is said of an option the reference's rules have greyed out: it is
off, whoever reads it. Some settings are read only when another turns them on
(the Arachne widths with `wall_generator` at `arachne`, the support patterns
with supports on); those are hidden or greyed by the rules while they are
off, and the answer is asked again after every change.

The same is said with the print. A real slice notes what it reads too
(`prepare_noted`), and "About this print" lists, after the engine's notes,
each setting set by hand that nothing read: `hollow_interior "1": not read by
the engine; it changes nothing in this print`.

### On an object, a part, a height range

What an item may set is the reference's per-item tabs' (Tab.cpp
`TabPrintObject`, `TabPrintPart`, `TabPrintLayer`):
`Preset::print_options()` met with `PrintObjectConfig` and
`PrintRegionConfig` for an object, with `PrintRegionConfig` and
`part_support_keys()` for a part or a modifier, with `layer_height` and
`PrintRegionConfig` for a height range; of those, what the process pages
show. `tools/make_settings_pages.py` reads the lists from PrintConfig.hpp
(the two `PRINT_CONFIG_CLASS_DEFINE`s), Preset.cpp
(`s_Preset_print_options`) and PrintConfig.cpp (`part_support_keys`), and
the "Frequent" page from `TabPrintModel::build`, into
`settings_pages.json` (`process.items`):

| Item | Options | Among them |
| --- | --- | --- |
| Object | 333 | every option of a region and the object's own: layer height, line widths, seam, supports, brim, raft, the walls, shells, infill, speeds |
| Part, modifier | 187 | a region's (walls, shells, infill, speeds…) and 13 of the 17 part support keys on the pages; no layer height, no supports on or off |
| Height range | 175 | a region's and `layer_height` |

A negative part, a support blocker or enforcer and a Precise Seam part have
none (GUI_ObjectList.cpp `add_settings_item`). Simple and advanced are the
global pages' mode.

What the engine does with an item's option is said by one function of the
session, `items::item_followed` (`src/api/settings/items.rs`), from the
engine's own answer (`edge_engine::settable`, `edge_engine::unfollowed_note`,
`crates/edge-engine/src/regions.rs`): where the engine notes the option
set on that kind of item as not followed, its note, in the words "About
this print" gives after a slice ("layer_height: set on an object, not
followed there; the plate's value is used"), as the ring of section 12;
where it follows it there, nothing (followed); else a ring with "not read
by the engine set on" that kind. The engine's answer, by family (README,
"Regions"):

| Options | Object | Part, modifier | Height range |
| --- | --- | --- | --- |
| walls, shells, solid and sparse infill, their patterns, speeds, accelerations, flows, line widths, fuzzy skin, ironing | followed | followed | followed |
| `layer_height` | noted, not followed (everything on a plate is cut at the same heights) | not offered | noted, not followed |
| brim type, width, gap | followed | not offered | not offered |
| supports on or off and their options, `seam_position`, elephant foot, XY compensation, closing radius | noted, not followed | not offered (the part support keys: noted) | not offered |
| wall, sparse and solid infill filaments | noted (each prints in the region's own filament) | noted | noted |
| holes to polyholes, `make_overhang_printable`, Z contouring, scarf seams, bridges over sparse fill | noted | noted | noted |

The two lists of what may be set where, the pages' (the reference's tabs,
`settings_pages.json`) and the engine's (`PrintObjectConfig` and
`PrintRegionConfig`), agree but in two ways. `fuzzy_skin_skip_overhangs`
(object, part, range) and `independent_support_layer_height` (object) are
in the engine's and not the pages': they are not in
`Preset::print_options()`, so the reference's tabs do not offer them,
while its slicer reads them from a file that sets them; the pages follow
the reference's tabs and offer neither. The 13 part support keys of
`part_support_keys()` on the pages (`support_interface_top_layers` and the
rest; EdgeSlicer's support groups) are object-level in the engine's list:
the reference's tab offers them on a part, so the pages do, and the engine
notes them set there as not followed, which is what it does.

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
| An object is made of volumes (`ModelVolume`): model parts, negative volumes, modifiers, support blockers and enforcers (and Precise Seam's), each with its own mesh, its transformation within the object, name, config (`extruder` among it) and paint; every instance of the object has them alike | `libslic3r/Model.hpp` | **added**: each part of a `Shape` has a number that stays its own (`Shape::part_ids`), and the model edits parts (`Model::set_part_filament`, `rename_part`, `remove_parts`, `set_part_transforms`, `mirror_parts`, `split_part`, `set_part_kind`, `add_part`, `add_generic`, `add_from`). The 3MF reader now gives a part's own filament (`Part::own_filament`), the settings its metadata carries (`Part::settings`, kept and not read by a slice) and the object's own filament (`Object::filament`). A part moved within its object is sliced where it then is, in its filament; an untouched project slices as before, byte for byte |
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
| `arrange` | `plate?`, or `all: true`, or `copies: [id]` | arranges one plate (the one shown when none is named), every plate, or the copies named around what else stands on their plates, with the options beside the Arrange button and the project's settings (see "Arrange" below); `note` says what fitted no plate ("Arranging is done but there are unpacked items. Reduce spacing and try again.") and what has no size ("Object %s has zero size and can't be arranged.") | "Arrange" |
| `mirror` | `copies`, `axis` (`x`, `y`, `z`) | mirrors the copies along that axis of the scene, about the middle of the box round them, and stands them on the bed; the mirror is kept in each copy's transform | "Mirror Object" |
| `split_objects` | `object` | "Split to objects": an object for each piece of the object's one mesh, or for each of its printed parts; each has a copy wherever the object had one, on the same plates, standing where its piece stood (brought down to the bed when it was in the air); `made` is the new copies. `note` says when modifiers or negative parts were left out. Refused, nothing changed: "The selected object couldn't be split." | "Split to Objects" |
| `split_parts` | `object` | "Split to parts": the object's one part becomes a part for each piece of its mesh. Refused: "The target object contains only one part and can not be split." | "Split to parts" |
| `lay_on_face` | `copy`, `normal` (three numbers, a face's from `scene_faces`) | turns the copy so that face points straight down, about the middle of its box, and stands it on the bed | "Gizmo-Place on Face" |
| `part_filament` | `object`, `parts: [id]`, `filament` (from 1; 0 or null: "default", the object's) | a part's or a modifier's own filament | "Change Filament" |
| `part_rename` | `object`, `part`, `name` | | "Rename Part" |
| `part_delete` | `object`, `parts: [id]` | takes parts out; refused for the last printed part: "Deleting the last solid part is not allowed." When one part is left, its own filament becomes the object's | "Delete part" |
| `part_transform` | `object`, `parts: [{ id, matrix }]` (each part's own coordinates to the object's), `name?` | sets where parts stand within their object, in every copy of it, then stands each copy on the bed again | as `transform` |
| `part_mirror` | `copy`, `parts: [id]`, `axis` | mirrors parts along an axis of the scene, as they stand in that copy, about the middle of the box round them | "Mirror Object" |
| `part_split` | `object`, `part` | "Split" of a part: a part for each piece of its mesh, in its place, each given the filament it printed in as its own. Refused: "The target object contains only one part and can not be split." | "Split to parts" |
| `part_type` | `object`, `parts: [id]`, `kind` (`part`, `negative`, `modifier`, `support_blocker`, `support_enforcer`, `precise_seam`) | "Change type"; refused for the last printed part: "The type of the last solid object part is not to be changed." | "Change part type" |
| `part_add` | `object`, `kind`, and `path` (a file, "Load...") or `shape` (`Cube`), `copy?` | adds a part; `part` in the answer is its number | "Load Part" / "Load Modifier", "Add primitive" |
| `plate_rename` | `plate?`, `name` (empty: none; at most 250 characters) | names a plate; the same name again is no step (`done` null) | "Rename Plate" |
| `plate_lock` | `plate?`, `value` | locks or unlocks a plate | "lock partplate" |
| `plate_settings` | `plate?` and, each null or left out for the project's, `bed_type` (one the printer takes), `print_sequence` (`by layer`, `by object`), `spiral_mode` (true, false), `first_layer_sequence` (filament numbers, each once), `other_layers_sequence` (`[{ begin, end (null: "End"), order }]`, layers from 2) | what the plate sets for itself ("Customize current plate"); refused, nothing changed, with "Please input layer value (>= 2)." and the like | "Plate Settings" (the reference takes no snapshot) |
| `plate_front` | `plate?` | "Move plate to the front": the plate is plate 1 and the one shown, the others one back each | "move plate to the front" |
| `plate_duplicate` | `plate?` | "Duplicate Current Plate": a plate after the last with a new object for each object with copies on the plate, each copy where it stood on its plate; the new plate is shown; `made` is its copies | "Duplicate Current Plate" (the reference takes no snapshot) |
| `cut` | `copy`, `plane` (as `cut_section` takes it; the normal points into the upper part), `keep_upper`, `keep_lower`, `place_on_cut_upper`, `place_on_cut_lower`, `flip_upper`, `flip_lower`, `to_parts`, `thickness` (0 to 20 mm), `thickness_from` (`centred`, `above`, `below`), `sides` (a piece given to the other side), `groove` (a dovetail, with its `init`), `editable` ("Keep cut editable", on unless false), `amend` ("Edit cut": one step with putting the cut back), each left out as the tool starts | the copy's object cut by the plane (section 13, "Cut"); `made` is the copies of what it made; `note` says how many of the cut's outlines could not be closed on an open mesh. Refused, nothing changed: "Select at least one object to keep after cutting.", "Cut plane is placed out of object" (Cut to parts with nothing on either side), a plane that is not one | "Cut by Plane" |
| `cut_edit` | `copy` | the halves of the cut that made the copy's object put back as the object was (section 13, "Cut"); `made` is its copy, `recipe` the cut as `cut_recipe` answers it with `missing` (fewer halves left than the cut made). Refused, nothing changed, for an object whose cut was not kept editable or cannot be made here | "Edit cut" |
| `item_set` | `object`, `part?` or `range?`, `key`, `value` (null: given back to what is over it) | an object's, a part's or a height range's own setting (Tab.cpp `TabPrintModel::on_value_change`): checked as the global pages check it, kept in the shape the value over it has (a list for each nozzle flow stays one), written as the reference serializes it. Refused, nothing changed: a key that kind of item may not set (`layer_height` on a part: "layer_height cannot be set on a part"), a value the option does not take, a negative part, a support blocker or enforcer or a Precise Seam part. The same value again is no step | "Change Option `key`" |
| `item_reset` | `object`, `part?` or `range?` | takes away every setting of the item's own that its pages offer (`reset_model_config`; the list's mark) | "Reset Options" |
| `range_add` | `object`, `range?` | without `range`, "Height range Modifier" on an object without ranges: 0 to 2 mm, the object's layer height (else the project's) and filament 0 (`get_default_layer_config`); one with ranges is left as it is. With `range`, "Add height range" after that range (`add_layer_range_after_current`): after the last, 2 mm on its top; into a range starting at its top, that range split as the reference splits it; into a gap of a layer or more, the gap. Refused where there is no room ("There is no room for another height range after this one.", the reference doing nothing without a word) | "Add layers"; "Add height range" (the reference's snapshot is unnamed) |
| `range_set` | `object`, `range`, `min_z?`, `max_z?` | a range's heights (`edit_layer_range`): a bottom at or over the top takes the top half a millimetre over it, as GUI_ObjectLayers.cpp does; a top at or under the bottom, a range over another's heights, a height that is not a number or is under the bed are refused | "Edit height range" (the reference's is unnamed) |
| `range_delete` | `object`, `range` | | "Remove height range" |

`arrange` leaves a locked plate as it is when every plate is arranged, and
refuses one alone: "This plate is locked.\nCannot auto-arrange on this
plate." Arranging every plate leaves alone too a plate whose own print
sequence is not the project's, as the reference does (ArrangeJob.cpp
`prepare_all`).

### Undo and redo

| C++ | Where | Here |
| --- | --- | --- |
| A snapshot of the model, the plates and the selection is taken before each change, named for the change; Ctrl+Z and Ctrl+Y (Cmd+Shift+Z on a Mac) and the arrows in the title bar go back and forth, each with the list of steps under it | `Plater::take_snapshot`, `Plater::undo` / `redo`, `Utils/UndoRedo.cpp` | **added**: `undo` and `redo`, for each project its own history (`src/api/edit.rs`). A step is the model before the change; a model is cheap to keep because the meshes are shared. Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z, and two buttons whose tooltip names the step ("Undo [Ctrl+Z]", then the step). **differs**: the buttons are in Prepare's toolbar, there being no title bar. **not yet**: the list of steps to jump several at once |
| The history is kept while it fits in a tenth of the machine's memory, at most 1 GB | UndoRedo.cpp `StackImpl` l.551 | **differs**: the last 200 steps |
| A new change forgets what was undone | `Stack::take_snapshot` | **added** |
| Selecting is a step too ("Selection-Add Object") | Selection.cpp | **differs**: only edits are steps; what is selected stays selected while it is still there |
| The settings are not in the history: a preset or a setting changed is not undone by Ctrl+Z | `Plater::priv::take_snapshot` (the model, the plates, the gizmos; the presets are not serialized) | the same: the history holds the model only. A process setting set by hand, a preset or a bed type is neither a step nor taken back by one, and an edit does not touch it (tested). An object's, a part's and a height range's own settings are the model's, and each change of them is a step, as the reference snapshots them ("Change Option %s", "Reset Options", "Add layers", "Remove height range") |
| The filament list is not in the history either: adding, removing, deleting, merging, a colour and "Apply All" take no snapshot, while what a deletion renumbers in the model is in the model the history holds | Plater.cpp `on_filaments_change`, `on_filaments_delete`, `Sidebar::delete_filament`, `on_filament_color_changed` | **differs**: each is one step ("Add one filament", "Remove last filament", "Delete this filament", "Merge with", "Change Filament Color", "Apply All"), the model and the filaments' presets and what is set over them taken back together, and the filament changes at layers with them. Undoing a deletion in the reference gives the objects their old numbers against the shorter list, which is what keeping them apart comes to. Picking one row's preset stays as before, no step, as in the reference |
| Undoing back to the state that was saved leaves the project unmodified | `Plater::is_project_dirty` | **added**: the project is unsaved while its model is not the one it was opened with |

### Selection

| C++ | Where | Here |
| --- | --- | --- |
| A click selects an instance; Ctrl adds or takes away; a click on nothing or Esc lets go; Ctrl+A selects all | GLCanvas3D.cpp l.6423–6551 | **had** (section 9); now by the copies' `id`, so what is selected survives every edit that leaves it there. Shift adds as Ctrl does |
| Del or Backspace deletes what is selected | `Plater::remove_selected` l.25120, GUI_ObjectList.cpp l.4586 | **added** |
| The list and the canvas show one selection | `ObjectList::update_selections`, `update_selections_on_canvas` | **added**, both ways, parts as well as objects |
| Selection modes: instances or volumes, one at a time; Alt picks the volume mode for a click (`set_volume_selection_mode(evt.AltDown() ? Volume : Instance)`), Alt+Shift adds a volume of the same instance or takes it away | GLCanvas3D.cpp l.6444–6510, `Selection::add_volumes` | **added**: Alt with a click selects the part under the pointer (picked by drawing that copy's parts each in a colour of its own), Alt+Shift adds or takes away a part of the same copy, a click without Alt selects the object again. The part is marked as the object is, its outline and the corners of its box. Parts are of one object; while they are selected, nothing else is |
| The right button: the object's menu ("Delete", "Clone", "Printable", "Rename", "Fill bed with copies", "Set as an individual object", "Mirror", "Split", the settings…) | `GUI_Factories.cpp` l.649–1181 | **added**: "Delete" [Del], "Clone" [Ctrl+K], "Printable", "Rename", and the "Split" and "Mirror" sub-menus (below). **not yet**: the rest (see "Left for later") |
| Ctrl+C / Ctrl+V copy and paste objects; Ctrl+K "Clone" asks how many | `Plater::copy_selection_to_clipboard` l.29941, `paste_from_clipboard`, `clone_selection` l.29965 | **added**: Ctrl+C notes what is selected, Ctrl+V puts a copy of each on the plate shown, Ctrl+K one beside each. **differs**: Ctrl+K makes one clone at a press and asks nothing; the clipboard is the page's, not the system's. **not yet**: Ctrl+X |
| A clone or a paste is put on a free place found by the arranger | `Plater::find_new_position` l.16128, `Selection::paste_objects_from_clipboard` l.3384 | **differs**: the free place nearest what it was made of, by the boxes round the copies, 6 mm apart; beside the plates when the plate has no room |

### The object list

In the sidebar, under "Objects" (`ui/objects.js`). In the C++ the list is
what the Process section shows when its switch is on "Objects"; here it has a
section of its own, always shown, and its selection is what the Process
section's "Objects" shows the settings of. It stands before the Process
section, where the C++ shows it too (at the head of Process).

| C++ | Where | Here |
| --- | --- | --- |
| A tree: each plate ("Plate 1", with its name), under it the objects on it, under an object its parts, its instances when it has several, its layer ranges; what is on no plate under "Outside" | `ObjectDataViewModel.cpp` l.523, l.563 | **added**: plates, objects, instances ("Instance 1"), "Outside"; an object of several parts lists each part under it, before its instances: the mark of its kind, its name, what it is in the tooltip ("Part", "Negative Part", "Modifier", "Support Blocker", "Support Enforcer", "Precise Seam"; a part that is not printed is quiet and slanted) and, for a part or a modifier, its filament. A file without plates lists its objects alone. **not yet**: layer ranges |
| A part's row: a click selects the volume; Ctrl and Shift add parts of the same object; mixing kinds is refused with "Selection conflicts" ("If the first selected item is an object, the second should also be an object.", "If the first selected item is a part, the second should be a part in the same object.") | `ObjectList::selection_changed`, `check_last_selection` l.5619, `fix_multiselection_conflicts` | **added**: the same rules and words, said in the status line; nothing changes. A part is selected as it stands in the first copy the row is under |
| A part's "Fila.": "default" and each filament (`BitmapChoiceRenderer`); "default" follows the object's filament; a part without its own shows the object's number; the object's own choice clears its parts' (`set_extruder`) | GUI_ObjectList.cpp l.777–810, l.1205–1226, ExtraRenderers.cpp l.320 | **added**: a list on a part's and a modifier's row, "default" first; closed, it shows the number the part then prints in. The object's list still sets every part, as before |
| Renaming a part: a double click on its name, or F2 ("Rename Part") | `ObjectList::rename_item`, `update_name_in_model` l.1250 | **added**, as an object's name |
| Columns "Name", printable (a mark to click), visibility, "Fila." (a list of the project's filaments, each with its colour), the marks for support paint, colour paint, sinking, settings | GUI_ObjectList.cpp l.445–510 | **added**: "Name", printable, "Fila.". An object whose parts print in several filaments shows "…" until one is chosen for all of it. **not yet**: hiding an object, the marks |
| A click selects; Ctrl and Shift select several | `ObjectList::selection_changed` | **added** |
| A double click on the name, or F2, renames in place; an empty name leaves the name as it was; `<>:/\|?*"` are not allowed | `ObjectList::rename_item` l.6307, `update_name_in_model` l.1233 | **added**: Enter takes the name, Escape or an empty name leaves it. **not yet**: the characters that are not allowed (nothing is written under an object's name yet) |
| The plate's row: a click shows the plate; its menu | `ObjectList::OnContextMenu`, `create_plate_menu` | **added**: a click shows the plate; the right button opens "Select All", "Delete All", "Arrange", "Delete Plate"; a cross at the row is "Remove current plate (if not last one)" |
| Dragging rows to reorder, or onto another plate | `ObjectList::OnDrop` | **not yet** |
| An item's settings: the reference's settings rows under an item are gone (`NEW_OBJECT_SETTING`, GUI_ObjectSettings.hpp l.9); an item with settings of its own has an icon in the editing column ("Click the icon to reset all settings of the object"), a click on which takes them all away without asking ("Reset Options") | GUI_ObjectList.cpp `add_settings_item` l.4180, l.699, l.1582; ObjectDataViewModel.cpp l.219 | **added** (eighteenth pass): the mark after the name of an object, a part or a range with settings of its own that its pages offer, the same tooltip, a click taking them away (one step). A range's layer height, which every range carries, does not count |
| Copying an item's settings (Ctrl+C / Ctrl+V on a settings row, `copy_settings_to_clipboard`) | GUI_ObjectList.cpp l.1379–1418 | not built, as in the reference: the rows they need are no longer made, so nothing reaches it (listed with the errors found) |
| "Layers" under an object, and a row for each height range: "Range 0.00-2.00 (mm)"; a click on "Layers" shows the ranges' heights, on a range its settings; a range's "Fila." | ObjectDataViewModel.cpp l.133, l.161; GUI_ObjectList.cpp `add_layer_root_item`, `add_layer_item` | **added**: "Layers" after the parts, each range under it a step in; a click selects the object in the scene and the row in the list. **not yet**: a range's filament in "Fila." (it is kept: filament 0, "default"), the range drawn on the object in the scene, the right button's menu on a range (the reference has none) |

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
| With parts selected (the volume mode) the gizmos move, turn and size the parts within their object; `do_move` / `do_rotate` / `do_scale` set each volume's transformation and bring a flying instance down | GLCanvas3D.cpp `do_move` l.7029–7110, `do_rotate` l.7205 | **added**: Move (with an arrow along Z for a part: up or down within its object), Rotate and Scale act on the parts selected, and so does dragging with Alt; letting go makes one `part_transform`. While a drag lasts only the parts move on screen, in every copy of the object. "Lay on face" stays the object's (off while parts are selected), and so does which plate a copy is on |

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
| The panel's title: "Object Operations" for an instance, "Volume Operations" for one part, "Group Operations" for several; a volume's scale and turn are its own, within its object | `GizmoObjectManipulation::update_settings_value` l.87–160 | **added**: the three titles; for a part "Scale" and "Absolute" are the part's own, "Size" as it is printed, and its "Position" Z can be typed. The list of coordinates is "World" / "Object", as the C++'s ("Part" is commented out there) |

### Split to objects, Split to parts

Read from `GLCanvas3D.cpp` (`_init_main_toolbar` l.9376), `GUI_Factories.cpp`
(the "Split" sub-menu l.2404), `Plater.cpp` (`split_object` l.16161,
`split_volume`, `can_split` l.20456), `GUI_ObjectList.cpp` (`split` l.2933,
`is_splittable` l.3488, `get_volume_by_item` l.3467), `libslic3r/Model.cpp`
(`ModelObject::split` l.2419, `ModelVolume::split` l.3186),
`libslic3r/TriangleMesh.cpp` (`TriangleMesh::split`) and
`libslic3r/MeshSplitImpl.hpp`. The engine has the pieces of a mesh
(`edge_mesh::split`) and the edits (`Model::split_to_objects`,
`Model::split_to_parts`).

| C++ | Where | Here |
| --- | --- | --- |
| The main toolbar's "Split to objects" and "Split to parts", after "Arrange" | `_init_main_toolbar` l.9376–9395 | **added**, with the same tooltips, after "Arrange" |
| The object's menu: "Split" ("Split the selected object"), a sub-menu of "To objects" ("Split the selected object into multiple objects"), "To parts" ("Split the selected object into multiple parts"), "By painted colour"; before "Mirror" | `GUI_Factories.cpp` l.2404–2418 | **added**: "Split" with "To objects" and "To parts", the same words. **not yet**: "By painted colour" (it is not shown) |
| On for one object selected (one row of the list): "Split to objects" when the object has more than one part (of any kind), or its one part's mesh is of several pieces; "Split to parts" when it has one part and that is of several pieces | `can_split`, `ObjectList::is_splittable` | **added**, the same rules (`split` in `scene`), the selection being copies of one object. The answer for a mesh is worked out once and kept with its shape, as the C++ caches `m_is_splittable` |
| The pieces of a mesh: triangles joined across shared edges run the other way round (`create_face_neighbors_index`: an edge pairs with the first later triangle that runs it back and has it free); the pieces in the order of their first triangle, each walked from it as the reference walks it; a piece enclosing a negative volume turned the right way out | `MeshSplitImpl.hpp` `its_split`, `TriangleMesh::split` | **added** (`edge_mesh::split`), the same rule and order. **differs**: the pairs are worked out in one pass in order, where the C++ works them out on several threads at once (see "Errors found in the C++") |
| Split to objects: an object of one part becomes an object for each piece of three triangles or more, named after the object with `_1`, `_2`…; an object of several parts an object for each printed part, named after the part, with its colour paint; modifiers and negative parts are dropped. Each new object takes every instance of the old one; the old one is removed; they are added at once, "kept where they stand", and dropped onto the bed (`ensure_on_bed`, "auto_drop_on_import" being on unless turned off); the new objects are selected; "The selected object couldn't be split." when one object would come of it | `ModelObject::split`, `split_object`, `load_model_objects` l.15569 | **added**, in that order in the list where the old object was, each copy on the plate its old copy was on. **differs**: a whole part keeps its seam paint as well as its colour paint, the geometry being the same (the C++ copies the colour only); dropping the modifiers says "All non-solid parts (modifiers) were deleted", the notice the C++ has commented out, rather than nothing |
| Split to parts: the part becomes a part for each piece, named after it with `_1`, `_2`…, in its filament, its paint let go; a piece whose convex hull is flat is dropped; "The target object contains only one part and can not be split." | `ObjectList::split`, `ModelVolume::split` | **added**; the object stays where it is |
| One snapshot: "Split to Objects", "Split to parts"; the slices of the plates the object is on let go | `split_object`, `ObjectList::split` | **added** |
| The object list shows the parts after "Split to parts" | `ObjectList::split` | **added**: the parts under the object, read only (see "The object list") |

### Mirror

Read from `GUI_Factories.cpp` (`append_menu_items_mirror` l.2028),
`Plater.cpp` (`mirror` l.16123, `can_mirror` l.20500), `GLCanvas3D.cpp`
(`mirror_selection` l.4375, `do_mirror` l.7414) and `Selection.cpp`
(`mirror` l.1797, `scale_and_translate` l.1715).

| C++ | Where | Here |
| --- | --- | --- |
| The object's menu: "Mirror" ("Mirror object"), a sub-menu of "Along X axis", "Along Y axis", "Along Z axis" ("Mirror along the X axis" and the rest), after "Split" and before "Delete"; on while what is selected is of one instance (`can_mirror`: `is_from_single_instance`, and no cut object) | `GUI_Factories.cpp` l.2028, l.2421 | **added**, the same words and tooltips, before "Delete"; off unless one copy is selected. The page's menus take a sub-menu now (`showMenu` in `app.js`): it opens beside its entry by the pointer, a click or Right, and closes with Left or Escape. There is no toolbar button for it, as there is none in the C++ |
| A mirror is a scale of -1 along the axis, in the world's coordinates, about the middle of what is selected, kept in the instance's transform (a "mirror" of its `Transformation`); the mesh is not changed. `do_mirror("Mirror Object")` takes one snapshot and stands each instance on the bed again, one that was sunk staying sunk | `Selection::mirror`, `do_mirror` | **added** (`scene_edit` `mirror`, `Model::mirror`): the mirror is the copy's transform, the mesh is the file's, one undo step, stood on the bed as a turn is |
| Slicing a mirrored instance: the mesh is transformed and, the transform being left-handed, its triangles are turned round (`TriangleMesh::transform` with `is_left_handed`), so it is still a solid | `libslic3r/TriangleMesh.cpp` | **had**: `Mesh::append` reverses the winding under a mirroring transform; painted triangles are turned the same way (`Project::painted`). Tested: an L mirrored along X slices, layer by layer, to the mirror image of its walls, each wall still anticlockwise, its first layer where it was; along Z it is upside down and stands on the bed |
| The number fields of a mirrored object: its scale is positive, its mirror apart | `Geometry::Transformation` | **added**: the fields take a mirrored object as mirrored along its own X and then turned, so its scale reads 100% and its angles are those of the turn |
| The other instances of the object are mirrored too | `synchronize_unselected_instances` | **differs**: only the copy selected, as Rotate and Scale here |

### Lay on face

Read from `Gizmos/GLGizmoFlatten.cpp`, `libslic3r/LayOnFace.cpp`,
`Selection.cpp` (`flattening_rotate`) and `GLCanvas3D.cpp` (`do_rotate`).
The engine has the convex hull of a mesh (`edge_mesh::hull`, an
incremental hull of its own where the reference has qhull) and the faces
and the turn (`edge_engine::lay_on_face`); the page draws and picks the
faces (`ui/scene-edit.js`).

| C++ | Where | Here |
| --- | --- | --- |
| "Lay on face" [F] in the gizmos' column, on for one whole instance (`is_selection_supported`) | `GLGizmosManager.cpp` l.227, `GLGizmoFlatten.cpp` l.166 | **added**: the tool in Prepare's toolbar beside Scale, "Lay on face [F]", off unless one copy is selected; F and its button open and close it, Esc closes it |
| The faces: the convex hull of the object's model parts, its triangles grouped by normal (to a thousandth) across shared edges, each group's outline taken round in the plane; dropped: a single triangle with a side under the least side, an outline under the least area or with an inner angle under 1°. The gizmo's limits go by the printed size: every face of 5 mm² or more, else the 12 largest down to max(0.01 mm², 1e-4 × diagonal²), the least side min(1 mm, 5% of the diagonal) | `LayOnFace.cpp` `hull_planes` l.18, `LayOnFaceFilter::for_gizmo` l.167 | **added** (`scene_faces`), the same grouping and limits at the copy's size and turn. The hull is cached with the object's shape (an affine map keeps a hull a hull) |
| Each face drawn over the object, a tenth of the way in towards its middle, its corners rounded (ten passes), lifted min(0.1 mm, 1% of the diagonal), in front of the object and hiding one another; the one under the pointer lighter (`FLATTEN_COLOR`, `FLATTEN_HOVER_COLOR`) | `update_planes` l.477, `on_render` l.327 | **added**: the same outline from the engine, the same colours |
| A click on a face turns the object so the face points down, then `do_rotate("Gizmo-Place on Face")` stands it on the bed: one undo step | `on_mouse` l.90, `lay_on` l.107 | **added**, as the button goes down, as in the C++. **differs**: the turn is about the middle of the copy's box; the reference's is about the instance's origin, which it keeps in the middle of the object's geometry, where a model read here may have its origin anywhere |
| The panel: "Convex hull" / "Face under cursor", "Click a highlighted face to lay the object on it.", "Hold Alt to pick any face under the pointer." | `on_render_input_window` l.386 | **added**: the panel titled "Lay on face" with the first hint, and "No resting faces found on the convex hull." when there are none. **not yet**: "Face under cursor" and Alt (section 13, "Left for later"); the hint about Alt is not shown, Alt doing nothing here |
| The other instances of the object take the same turn | `Selection::synchronize_unselected_instances` | **differs**: as with Rotate here, only the copy selected turns |

### Cut

Read from `Gizmos/GLGizmoCut.cpp/.hpp` (the fork's: 11 734 lines, with its
curved and drawn surfaces, flexi joints, push-fit pins, TAC joints and
re-editable cuts), `GLGizmosManager.cpp`, `KBShortcutsDialog.cpp`,
`libslic3r/CutUtils.cpp/.hpp` (`Cut::perform_with_plane`,
`process_solid_part_cut`, `process_modifier_cut`, `post_process`,
`reset_instance_transformation`, `finalize`), `libslic3r/Model.cpp`
(`clone_for_cut`, `assign_copy`, `invalidate_cut`),
`libslic3r/TriangleMeshSlicer.cpp` (`cut_mesh`, `triangulate_slice`),
`libslic3r/CurvedCut.cpp` (`curved_cut_thickness_faces`),
`libslic3r/CutRecipe.hpp` (`cut_plane_center_allowed`),
`Plater.cpp` (`apply_cut_object_to_model`) and `Format/bbs_3mf.cpp`
(`cut_information.xml`, `cut_recipe.xml`). The mesh's side is
`edge_mesh::cut` (and `edge_mesh::earcut` for the caps), the model's
`Model::cut` and `Model::cut_section` (`crates/edge-engine/src/cut.rs`),
the session's `scene_edit` `cut` and `cut_section`, the page's
`ui/cut.js`.

The modes the fork offers: "Planar" (a plane; its surface "Flat", or
"Curved", a height field over the plane bent by control points, or
"Draw", a ruled strip along a line drawn on the model), and "Dovetail"
(a tongue and groove along the plane, "Tongue and groove" in PrusaSlicer:
"Depth", "Width", "Flap Angle", "Groove Angle", "Count", "Gap"). Built:
"Planar" with the flat, the curved and the drawn surface, "Pick face", a part
given to the other side, "Dovetail", "Keep cut editable" with "Edit
cut..." and "Copy cut to...", and the connectors "Plug", "Dowel",
"Snap" and "Pin". Not built: the fork's "Flexi" and "TAC Joint"
connectors, and connectors on a curved or drawn surface (below).

| C++ | Where | Here |
| --- | --- | --- |
| "Cut" [C] in the gizmos' column, for one whole instance (`on_is_activable`: `is_single_full_instance`); C again or Esc closes it | `GLGizmosManager.cpp` l.228, `GLGizmoCut.cpp` l.6291, `on_init` (`m_shortcut_key`) | **added**: "Cut [C]" in Prepare's toolbar after "Lay on face", off unless one copy is selected; C and its button open and close it, Esc closes it. Another copy selected while it is open moves it to that copy; parts selected close it |
| The mesh cut: each model part cut in the instance's frame (`instance_matrix * volume_matrix`), the upper and lower meshes with their caps (`cut_mesh`, `triangulate_slice`) | `CutUtils.cpp` l.529 `process_volume_cut`, `TriangleMeshSlicer.cpp` l.2450 | **differs**, on purpose: `edge_mesh::cut` builds the halves so that they are always closed (the reference finds a cap's corners by nearness and adds a vertex where it finds none, which leaves open edges: see "Errors found in the C++"). A corner within 1e-4 mm of the plane is on it; a crossed edge is cut once for both its triangles; corners of the cut nearer than 1e-4 mm are one (the reference's own duplicate rule); the cap is laid only over the edges the cut opened, followed round as loops (where loops touch, the one turning furthest left), holes put in the smallest outline round them, and triangulated by ears (after Mapbox's earcut) using every corner of the loops and no other, no ear taken that does not turn. A mesh open where it is cut keeps its opening, and the session says so ("The object is open where it was cut: %d of the cut's outlines could not be closed.") |
| "Cut position" ("Z:", the plane's middle's height), "Reset cutting plane" (↺, off at the start); "Build Volume" (the object's size in the plane's axes) | `render_cut_plane_input_window` l.8935, `render_move_center_input` l.876, `render_build_size` l.8302 | **added**, each a step of the tool's own ("Move cut plane", "Reset cutting plane"). The plane's middle goes where the reference lets it: while the object reaches past it by more than half a millimetre either way, or nearer the object's middle (`set_center_pos`, `cut_plane_center_allowed`) |
| The plane: a square 1.5 times the object's bounding sphere's radius a side (half), (0.9, 0.9, 0.9, 0.5), pink (1, 0.8, 0.8, 0.5) when the cut cannot be made; drawn twice with a thickness; the object cyan above it and magenta under it (the canvas's colour clip plane); the section's outline over it | `render_cut_plane` l.1284, `init_picking_models` l.6665, `apply_color_clip_plane_colors`, `render_clipper_cut` | **added**: the same colours (the scene's, not the page's accent); the section is the session's (`cut_section`), asked for as the plane moves, one call at a time. Only the printed parts take the two tints; modifiers and the other helpers are drawn as they are |
| The grip: a yellow stalk half the radius out along the normal, a ball (yellow under the pointer, else grey) 32 px across at 0.35, a quarter larger while dragged; two red cones (about the plane's x) and two green (about its y) | `render_cut_plane_grabbers` l.5700 | **added**, the same sizes and colours, over the object (the depth cleared for them) |
| Dragging the ball, or the plane, moves the plane along its normal, by whole millimetres with Shift; a cone turns it about the plane's own x or y, by eighths of a turn between a third and two thirds of the grip's reach and by 5° about the reach itself; "X: 30.0°" while turning; the heights of the two parts on the ball's tooltip ("12.50 mm (Object A)"; "Part A" with Cut to parts) | `dragging_grabber_move` l.6351, `dragging_grabber_rotation` l.6389, `get_tooltip` l.288 | **added**. The tooltip is the stage's own (`title`) |
| A click on the plane flips it ("Click to flip the cut plane\nDrag to move the cut plane"); "Flip cut plane" | `on_mouse` l.330, `flip_cut_plane` l.8415 | **added**: half a turn about the plane's own x, the plane's tooltip in those words. The fork's "Flip cut plane" button is commented out (`render_flip_plane_button`), so it is not shown either |
| Shift with a drag: "Draw cut line"; the plane laid through the line and the line of sight, its middle the object's brought onto it ("Cut by line") | `process_cut_line` l.11349, `m_shortcuts_cut` | **added**. **differs**: a line under 3 pixels is ignored (the reference measures 3 units at the near plane) |
| Up / Down: the plane 1 mm along its normal ("Move cut plane") | `GLGizmosManager.cpp` l.1121 `shift_cut` | **added** |
| "Mirror cut" X / Y / Z ("Mirror the whole cut - plane, surface and connectors - about the object's centre on the world X axis"), the side that was upper staying upper | `render_mirror_buttons` l.8648, `mirror_cut` l.8604 | **added**, a step of the tool's own |
| "Thickness" (0 to 20 mm, "Width of the band of material the cut removes, …"), "Remove from" "Centred" / "Above" / "Below", and its note; started at 0 each time the tool opens | `render_cut_thickness_input` l.5646, `on_set_state` | **added**: the engine cuts each half at its own face (`CutOptions::faces`) |
| "Reset cut" ("Reset cutting plane and remove connectors") | l.9006 | **added**: the plane (there are no connectors) |
| "After cut": "Upper part" / "Lower part" each with its colour, "Keep", "Place on cut", "Flip" (the last two exclusive, off without "Keep"); "Cut to parts" (keeps both, as they stood); kept from one cut to the next | l.9066–9124 | **added** |
| "Perform cut": off while nothing is kept; the warnings "Warning: Select at least one object to keep after cutting.", "Warning: Cut plane is placed out of object" | `can_perform_cut` l.9805, `render_input_window_warning_lines` l.9269 | **added**, the same words; a plane past the object warns and still cuts, as in the reference |
| The plane's moves are snapshots of the gizmo ("Move cut plane", "Rotate cut plane", "Flip cut plane", "Cut by line", "Mirror cut on X" …) in the project's undo history | `Plater::TakeSnapshot` with `SnapshotType::GizmoAction` | **differs**: they are the tool's own while it is open: Ctrl+Z (and the Edit menu's Undo) takes back the plane's last move, then, with none left, the project's last step. They go when the tool closes, the project's history holding the model only (above, "Undo and redo") |
| "Cut by Plane": one snapshot; the tool closes first; the object is replaced by what is made, which is selected; the project is modified; the plates' slices go | `perform_cut` l.10751, `Plater::apply_cut_object_to_model` l.25405 | **added** (`scene_edit` `cut`). **differs**: the objects made stand where the object stood in the list (the reference adds them at the end), as "Split to objects" here does |
| What each part becomes: a model part is cut, each piece keeping its name (with `_A` / `_B` for Cut to parts), filament and settings; a negative part, modifier, support blocker or enforcer goes whole, its place put into its own transform, to each side its box reaches (all of them to the one object with Cut to parts), and is left out of a side whose printed material its box does not meet | `process_solid_part_cut` l.841, `add_cut_volume` l.496, `process_modifier_cut` l.818, `delete_extra_modifiers` | **added**. **differs**: a side with no printed part makes no object (see "Errors found in the C++") |
| The copies: the cut copy's turn and size go into the halves' meshes, which stand where it stood; "Place on cut" turns a part to lie on its cut face, "Flip" upside down; every copy is then put on the bed | `reset_instance_transformation` l.868, `post_process` l.987, `load_model_objects` | **added**. **differs**: the other copies of the object keep their own turn and size relative to the cut one; the reference gives them their own turn about z on top of the cut copy's turn, which is in the meshes (see "Errors found in the C++"). The turn is about the middle of the part's box, the reference's about the instance's origin |
| The halves' own settings, height ranges, variable layer heights and brim ears are the object's (`clone_for_cut` copies them, at the object's old heights) | `Model.cpp` l.2293, l.1399–1402 | **added**, as the reference: kept as they were, at the object's heights (listed with the errors: on a part that was lifted or turned they no longer fit) |
| Paint: the fork carries every kind over the cut (`KeepPaint`, "painting survives a cut, always": `save_paintings`, `restore_painting` by where the faces are) | `Cut::finalize` l.1034 | **differs**: a triangle that is whole on a side keeps its codes as they were; a piece of a painted triangle takes the state at its middle whole; the cap is unpainted, as the reference leaves a cut face. A copy that was mirrored carries its pieces' paint the same way, its whole triangles too |
| The cut information: a plain cut's halves are not cut objects (`InvalidateCutInfo` while the object has no cut id): no `cut_information.xml` entry; a cut with connectors or a dovetail, both parts kept, gives its results one cut id (`check_sum` 2 and one for each dowel, the connectors counted); with "Keep cut editable" (on unless turned off) a `cut_recipe.xml` with the uncut mesh | `perform_cut` l.10902, `update_object_cut_id` l.10006 | **added** (`ObjectKept::cut_id`, `PartKept::connector`), written as the reference writes it (section 15); "Edit cut..." takes a cut's dowels with its halves, as the reference takes every object of the cut id |
| "Mode": "Planar", "Dovetail" | `render_cut_mode_combo` l.718 | **added**, as two joined buttons (STYLE.md: a choice of one of two), kept from one cut to the next |
| "Dovetail": the groove drawn over the plane (`its_make_groove_plane`), "Groove" with "Depth" and "Width" each with its tolerance, "Flap Angle" (30°–120°), "Groove Angle" (0°–15°), "Multiple" with "Count" (1–20), "Gap" (off with one) and the "Spacing" they come to, each with its reset; its grabbers: a move across the plane (x), along it when the groove is angled (y), a turn about the normal; the upper part's tongue cyan and the lower part's groove magenta; "Perform cut" off with "Warning: Cut plane with groove is invalid" while a side of a dovetail misses the object (`has_valid_groove`) | `render_cut_plane_input_window` l.8935, `update_plane_model`, `has_valid_groove`, CutUtils.cpp `perform_with_groove` l.2218–2427 | **added**. The cut is the reference's own run of plane cuts (the slab either side of the plane, each dovetail's sides and flaps, the tolerances), each piece closed, the pieces of a side one part, the objects `_A` and `_B`; no mesh boolean is needed. Checked: closed halves adding up to the block less the tolerance's gap, the slice at the groove showing a gap of the tolerance, the C++ slicer reading both halves as manifold with those volumes. **differs**: the depth it starts at goes by the object's size; the reference's goes by its grabbers' size, which is fixed on the screen, so it changes with the zoom (section 17). Paint is not carried over a dovetail's pieces |
| "Surface": "Flat", "Curved" (and "Draw", below); "Show halves": "Upper" / "Lower": "Visible", "Ghost", "Hidden" with its note | `render_curved_surface_inputs` l.2891, `render_side_visibility_inputs` l.2851, `curved_cut_side_alpha` | **added** (`curved.js`): the surface as two joined buttons; each half drawn whole, a quarter there, or not at all (the mesh shader's alpha a side, as gouraud.fs), the cut unchanged |
| "Curved": a grid of control points over the plane fitted to the part's section (a point every 10 mm, at least five a side, half sizes 15% or 5 mm past the part), blended by Catmull-Rom (straight with two a side), level past its edges; "Control points" (one count with "Square", or across and down) and the grid sampled again; a handle dragged along the plane's normal, its neighbours following within "Bend radius" (1.5 times the spacing to start), less the further they are with "Falloff"; the right button on a handle snaps it onto the part along the normal, with Shift its neighbours too; "Smooth" (one pass, half way to the neighbours' mean), "Reset surface", the notes and tooltips, "Cut face updating…", "The surface does not cross the part on the upper side; …" | `render_curved_surface_inputs`, `curved_on_mouse` l.2729, `curved_snap_apply`, CurvedCut.cpp (`CurvedCutSheet`, `curved_cut_fit_projection_extent`, `curved_cut_default_grid`, `curved_cut_snap_distance`, `falloff_weight`) | **added**: the sheet's sums in the page and in `edge_engine::curved`; a drag, a snap, Smooth, Reset and the grid each a step of the tool's own undo; the plane's moves refit the sheet, a bent one keeping its shape over the new extent, resampled from the sheet as the person last set it, as the reference does (`publish_reference`), so fit after fit does not drift. **not yet**: the sheet's "Bend radius" circle and the snap's marker in the scene; connectors on a bent surface ("Add connectors" is off there) |
| The cut by a curved surface: the closed solid under the surface (a slab reaching past the part every way, sampled 128 a side), the lower half the part's intersection with it and the upper the part less it; with a thickness, a slab at each face; a half the boolean gives nothing for recovered as the part less the other; a side the surface leaves empty not refused but said ("Cut: the surface does not cross the part on %1%, so that half is empty."); a bent sheet wins over pieces given to sides; while the surface is bent the halves are worked out and shown in place of the part | CurvedCut.cpp `curved_cut_lower_slab` l.636, `cut_with_solid` l.746, `curved_cut_split` l.859, GLGizmoCut.cpp `perform_cut` l.10912 | **added** (`curved::lower_slab`, `curved::cut_with_solid`, `Model::cut` with `CutOptions::sheet`, `cut_preview`, `cut_preview_mesh`, `cut_snap`): the booleans are `edge_mesh::boolean`'s, the part prepared once a copy and kept between previews, the preview asked for last stopping the one before (`Cancel`); a boolean that refuses leaves the object as it was with "The cut could not be made." and why. Checked: each half closed, the halves' volumes adding up to the part (to 1e-2 mm³), with a thickness short of it by the band, an empty side made and said; sliced; the recipe (kind 1 and its `<sheet>`) read back here and by the C++ slicer (`--info` manifold, `--export-3mf` writing the sheet back), and "Edit cut..." takes it. A boolean can leave corners a fraction of a micron apart, which single precision folds to a triangle of no area once the half is placed on the bed: corners within 1e-4 mm are made one (`edge_mesh::tidy::weld_close`), the triangles that lose a corner taken out, the half kept only if it stays closed; the test checks the halves as placed. Paint is not carried over a curved cut |
| "Draw": the cut by a line drawn on the model (DrawCut.cpp): an open line a ruled strip along it, its ends carried "Extension" past it, leaning by "Angle" (and the skirt's own "Extension angle"), swept along the surface's normal, the line of sight or an axis ("Direction"), "Depth" in and across to a flat core; a closed loop a band, a skirt and that core, taking out the piece it goes round; the line resampled every millimetre, smoothed and put back onto the model; an open line's extension stopped short of the rest of the part; "Draw a longer line on the model.", "The line crosses itself.", "The line leaves the model.", "The line does not make a usable cut surface.", "The stroke does not separate the part."; the cut the part less the swept solid and the part with it; a part the line leaves whole going whole to its side; in the recipe (kind 2, its `draw_*` and `<stroke>`) | DrawCut.cpp (`DrawCutStroke::finish`, `DrawCutChain`, `draw_cut_build_cutter`, `draw_cut_split` l.3290, `draw_cut_empty_sides` l.3163), CutRecipe.hpp (`recipe_stroke_to_chain`, `migrate_draw_v2`), GLGizmoCut.cpp `refresh_draw_stroke` l.3329, `reproject_draw_stroke_on_mesh` l.3454, `compute_draw_extension_clearance` l.3206, l.5556 | **added** (`edge_engine::drawn`, `Model::cut` with `CutOptions::drawn`): what the tool works out before a cut (`draw_cut_line`: finished, put back onto the model, the angle dropped for a loop with no consistent outside, the clamp) is done again by the engine from the recipe, so a cut made again is the one the tool showed; the halves by `edge_mesh::boolean`, welded as the curved cut's; a boolean that refuses leaves the object as it was. Checked: a line across a block 3 mm off its middle makes closed halves of 12 and 18 parts in 30, sliced; a line ending 4 mm short of a second block has its end's extension clamped to 4.9 mm and its cutter kept out of that block; the recipe read back here and by the C++ slicer (`--info` both halves manifold, 10 800 and 7 200 mm³; `--export-3mf` writing the stroke's 41 points and `draw_through_all` back). **differs**: the core plate is triangulated by the engine's own ears over `edge_clip::union`, where the reference uses its own; the clamp stops at a face the ray goes into, not at the first face met, and a part the line leaves whole is found by the cutter's own corners as well (both in "Errors found in the C++"); the loop's closing radius goes by the box of the parts in the plane's frame, the reference's by the gizmo's box |
| "Draw" in the tool: a press on the model starts the line, or carries it on from one of its marked ends ("That stroke did not start at either end of the line, so it was not added. …", "The line is already closed. …"); the stroke drawn as the pointer moves, a pixel at a time (at most 256 a move), its short skims onto another face taken out; letting go near the other end closes the loop (the end lit green beforehand); "Close loop" (the ends joined across the surface as it is seen, "The two ends cannot be joined across the surface from this view. …"), "Cut along the line", "Clear line"; "Edit points": a handle dragged along the surface, taken away by the right button, added by Shift with a click on a span; Esc out of editing or clearing the line, Ctrl+Z / Ctrl+Y a stroke at a time; the panel's notes (how to go on, "Closed loop: …" / "Open line: …", the empty side, the folds, the clipped extension, the loop with no consistent outside); the plane and its grips hidden, the line, its ends and the surface drawn; "Perform cut" off while the line cannot cut or leaves a side empty; "Through all" hidden and off (`DrawThroughAllHidden`); "Mirror cut" turning the line over; "Edit cut..." on a drawn cut opening the tool with its line | `render_draw_surface_inputs` l.5199, `draw_on_mouse` l.4371, `draw_interpolate_to` l.3288, `close_draw_chain_on_surface` l.3662, `commit_draw_points` l.3530, `render_draw_stroke` l.4833, `render_draw_chain_endpoints` l.4938, `can_perform_cut` l.9839, CutRecipe.cpp `cut_recipe_mirrored` | **added** (`drawn.js`, `cut_draw`, `cut_draw_hits`): the session finds where the pointer meets the model (the copy's parts in the plane's frame, kept while the copy and the plane stay as they are) and keeps the sums; the page keeps the chain and its undo; while the line can cut, its halves are worked out by the booleans as a bent sheet's are and shown in place of the part. Checked in the page: a stroke across a block used open and cut, a loop of four strokes closing on the last (a plug at Angle 60), a stroke away from the ends refused, undo and redo, a point dragged and one taken away, "Close loop" across an open line (the line then crossing itself, said), "Edit cut..." bringing a drawn cut back with its line, "Mirror cut" on Y. **differs**: an edit of the points keeps an open line open (the reference closes it, see "Errors found in the C++"); the halves are the booleans' rather than the reference's colour field over the part (`update_draw_field_texture`). **not yet**: connectors on a drawn surface ("Add connectors" is off there), the section of the drawn surface while they are placed (`render_draw_section`) |
| "Add connectors" / "Edit connectors" (off unless both parts are kept as objects of their own): the cut face shown with what is over the plane hidden; a click on the face adds a connector, a click chooses one (Shift adds to the choice, Alt takes off it), a drag moves it, the right button takes it off, Ctrl+A chooses all, Delete takes the chosen off; "Type" ("Plug", "Dowel", "Snap"), "Style" ("Prism", "Frustum", a plug's only), "Shape" ("Triangle", "Square", "Hexagon", "Circle", not a snap's), "Depth" and "Size" each with its tolerance, "Rotation", a snap's "Bulge" and "Space"; each change to the chosen ones and the next; "Remove connectors", "Confirm connectors", "Cancel" (takes them all off); "Warning: Invalid connectors detected" with what is out of the cut contour, out of the object, overlapped, and "Perform cut" off meanwhile; with connectors "Keep" is forced on, "Cut to parts" and "Pick face" off, "Reset cut" takes them off; the cut: a plug or a snap a part of the lower object with its hole (the snap's a round one) a negative part of the upper, wider and deeper by the tolerances and through the face by 0.05 mm, a dowel an object of its own ("name-Dowel-Connector-n") with a hole each side; a cut's thickness added to each; the results one cut in `cut_information.xml` (its id, `check_sum`, the connectors by part) and in the recipe | `render_connectors_input_window` l.7147, `add_connector` l.11413, `gizmo_event` l.11540, `check_and_update_connectors_state` l.9617, `apply_connectors_in_model` l.9898, `apply_cut_connectors` l.11692, `get_connector_mesh`, CutUtils.cpp `process_connector_cut` l.711, `apply_tolerance` l.451, TriangleMesh.cpp `its_make_cylinder`, `its_make_frustum`, `its_make_frustum_dowel`, `its_make_snap` | **added** ("Plug", "Dowel", "Snap"; `connectors.js`, `edge_mesh::connector`, `Model::cut` with `CutOptions::connectors`): the same shapes (each closed and wound outward, checked), placings, holes and tolerances, the same colours. Checked: sliced, a plug's hole is wider than the plug by the radius tolerance (0.2 mm) and deeper by the height tolerance; the C++ slicer reads the file as its own (its `--info` manifold, `--export-3mf` writing the same cut information and recipe back), and "Edit cut..." puts the connectors back. **differs**: a connector's mesh is not centred on its box across the plane (the reference's `add_volume` puts a triangle's middle a quarter of its radius off where it is drawn), and the outline checked against the cut contour is turned as it is drawn (section 17). The fields are number fields with a reset each; the reference has sliders and resets the snap's two only. A dowel's frustum (two cones) is cut when a file asks for it, but, as in the reference, the panel makes a dowel a prism. **not yet**: the fork's "Flexi" and "TAC Joint"; a box drawn to choose connectors (Shift or Alt with a drag); the hovered connector's colour, which the pane does not show for want of hover events (the code has it) |
| "Pin" (the fork's push-fit pin): "Preset" ("S", "M", "L", "Auto": the largest the section takes, "No preset fits this cut; S is used. Check the fit."; "Custom" once a field is changed), "Diameter", "Same length both sides" with "Length" or "Upper length" / "Lower length", "Same clearance both sides" with "Clearance" or "Upper clearance" / "Lower clearance", "Groove for the lip", "Lip" with "Lip height", "Lip width", "Lip position", "Cutout" ("None", "Flat", "Slot", "Flat + slot") with "Flat depth", "Flats on both sides", "Slot width", "Slot length", "Print" ("Auto (lying when it can)", "Upright") and how it will print, "Advanced" with "End chamfer", "Mouth chamfer", "Depth clearance", "Pointed hole end", "Rotation", a drawing of the pin, why it cannot be made and what may go wrong, each field with its tooltip; the cut: the pin an object of its own in its print pose, in a row past the object ("name-Pin-n"), a hole in each half from its face (a mouth flare, a groove for the lip, room at the bottom, a cone or a flat end), a cut's thickness added; "%1% pin(s) added beside the object (print lying flat)." and the rest; a pin that cannot be made refused | `render_pushfit_inputs` l.7905, `render_pushfit_profile_inset` l.7824, PushFit.cpp (`pushfit_pin_mesh`, `pushfit_hole_profile`, `pushfit_auto_size`, `pushfit_validate`, `pushfit_warnings`), CutUtils.cpp `process_pushfit_cut` l.569, `add_separate_parts` l.945 | **added** (`edge_engine::pushfit`, `cut_pin`): the same shapes, presets, checks and placing; the flats cut off by planes (the reference's fallback when its boolean fails), the slit by `edge_mesh::boolean` (a pin is made without it only if the boolean fails, and the note says so). Every pin, flat and slit of each preset and each hole tested closed; sliced, the upper half's hole is the pin's radius and its clearance. The pin's shape is written on its parts in `cut_information.xml` (`processed`, `pushfit_*`) and in the recipe; the C++ slicer reads the file as its own and writes it back, and "Edit cut..." puts the pin back. The checks and warnings are the reference's own sentences, which it does not translate either. The holes are drawn round a pin while it is placed, faint (`render_pushfit_hole_envelopes`) |
| "Pick face" ("Click a model face to set the cut plane flush with it; use Cut position to offset"), "Click a face of the model." while armed | `apply_picked_facet` l.8375, l.9022–9029 | **added** (`cut_pick`, the session's ray tree): the face under the pointer lit, a click lays the plane on it, its middle where the click was; refused where the plane would miss the object |
| A part given to the other side by the right button ("Right-click a part to assign it to the other side") | `process_contours`, `PartSelection`, `perform_by_contour` | **added** (`cut_parts`, `cut_parts_mesh`): the pieces worked out as the plane is placed, each drawn in its side's colour, a right click on one moving it to the other side; the cut then makes one part of each side's pieces, the objects `_A` and `_B`; "Cut to parts" off meanwhile; refused when every piece is on one side |
| "Keep cut editable" (on unless turned off, with its tooltip): the halves carry how they were cut and the object as it was, written as `Metadata/cut_recipe.xml` and `Metadata/cut_recipe/<sha256>.bin` | `build_recipe_from_gizmo` l.10089, `object_part_mesh`, bbs_3mf.cpp `_add_cut_recipe_file_to_archive` | **added** (`edge-3mf` `recipe`): the reference's schema (version 8) and blob ("ESCUTMSH", f32 corners, u32 triangles), the blob named by its SHA-256 and written once for both halves; a recipe read is written back as it was. The uncut mesh is kept in the halves' coordinates (the copy's turn and size in it), so `instance_linear` is written as the identity; one the reference wrote is turned by its `instance_linear` when read. Checked: the C++ slicer reads ours and writes it back with the same hash, and its file reads back here with "Edit cut..." working |
| "Edit cut..." ("Edit the cut that produced this object") in the object's menu: the halves taken out, the uncut object put in their place, the tool opened as the cut was made ("Editing an existing cut. Perform cut replaces both halves.", "One half of this cut is missing …"); Cancel puts the halves back; one snapshot for the whole | GUI_ObjectList.cpp `edit_cut` l.3583, GLGizmoCut.cpp `begin_reedit`, `cancel_reedit`, `render_reedit_notice` | **added** (`scene_edit` `cut_edit`, `cut` with `amend`): one undo step, "Edit cut", for putting back and cutting again; closing the tool without cutting takes that step back. **differs**: the halves are those carrying the same cut on the copy's plate, where the reference goes by the cut id, which a plain cut takes away, so it put back the chosen half alone and said the other was missing (section 17). The uncut object takes the halves' helpers (each once) and settings; the reference's has its mesh alone. A recipe of a drawn cut, or one with connectors of the fork's "Flexi" or "TAC Joint", is refused with a reason; its plugs, dowels and snaps come back as they were placed |
| "Copy cut to..." ("Set this cut up on an object, ready to cut it the same way"): every object, "1. name (parts)", " (this object)" on the one selected; the tool opened on it with the cut set up in its own place | GUI_Factories.cpp `append_menu_item_copy_cut` l.2194, `cut_target_label`, `copy_cut_to` l.3755 | **added** (`cut_recipe` with `onto`), the same labels and tooltips |

How it was checked:

- `crates/edge-mesh/tests/cut.rs`: cubes across, through corners, edges
  and faces, missed; cylinders at heights and slants; a tube (a hole in
  each cap); two shells and two touching along an edge; a hollow box; a
  thickness; an open box and an open sheet; 72 planes through a fine
  cylinder and 200 at random through a sphere plus one through each ring
  of its corners. Each half closed and wound alike (every edge run once
  each way), no triangle of no area, the volumes adding up to the whole.
- `crates/edge-engine/tests/cut.rs`: a block cut (volumes, bounds, both
  on the bed, on its plate), keeping one side and laying it on a slanted
  cut (the cap flat on the bed, its area the section's), Flip, Cut to
  parts (`block_A`, `block_B`, the object's height), the helpers' sides,
  settings and height ranges, paint, a turned copy, a thickness, and the
  reference's 3DBenchy (`resources/handy_models/3DBenchy.3mf`) at seven
  planes, through its cabin and at slants: closed, volumes to 1e-6 of
  the whole, no triangle of no area made (the Benchy has 11 of its own,
  which the halves share). Its halves, the upper stood on the bed, slice
  (`edge_slice::slice`) to the whole's outlines on each side of cuts at
  9, 25 and 33.6 mm: as many outlines, areas within 1e-5, no open chain.
- The C++ slicer has no cut on its command line (`Snapmaker_Orca.cpp`
  l.4990: "Cut operation is not supported yet"), and its tests of the cut
  are of the fork's curved cuts and joints, not of numbers for a plane.
  So what this cut makes was given to it instead: the Benchy cut at 30 mm
  in the page and saved; its `--info` reads two objects, each
  "manifold = yes", one part, 38 470 and 188 760 facets, 2 112.66 and
  13 437.80 mm³ (the whole: 15 550.38; the sum 15 550.46, the difference
  its float sums); `--export-3mf` writes them back as two objects of the
  same facets, which this reader reads; `edge-parity` on the saved file:
  150 of 150 layers at the same heights, the walls within 0.03 % a
  layer. The cube cut at 7.5 mm: 20 facets each, 5 000 and 3 000 mm³,
  manifold. Cut to parts (a slant through the Benchy): one object, its
  two parts, manifold, 15 550.35 mm³, back through `--export-3mf` as one
  object of two parts.
- The session (`src/api/edit/tests.rs`): `cut_section` on the Benchy and
  on a bare STL (measured where it is shown), the refusal, "Cut by
  Plane" as one step that undoes and redoes, both parts on the bed,
  saved and read back by another session to the same triangles, Cut to
  parts, and the halves sliced.
- In the page (`--serve`, 1280 × 820 and 900 × 560, dark and light,
  English and German): C opened it on a 20 mm cube; the ball dragged by
  the pointer (10 → 13.75 mm); a cone dragged (the plane turned 40°, the
  section and the tints following); a click on the plane flipped it;
  "Cut position" typed (6.5, then 7.5); "Mirror cut" Y; Ctrl+Z took back
  a plane move; "Reset cutting plane"; "Place on cut" ticked; "Perform
  cut": two objects, both selected, the project unsaved, "Cut by Plane"
  to undo; Ctrl+Z and Ctrl+Shift+Z; A arranged them; "Slice plate"
  sliced both; saved. The Benchy cut through its cabin at 30 mm with
  "Flip" (an 18 mm upper part upside down beside a 30 mm hull after A),
  Up twice (24 → 26 mm), a thickness of 2 mm with "Remove from", Cut to
  parts (`3DBenchy_A`, `3DBenchy_B` under the object in the list). The
  line drawn with Shift was made through the page's own handlers
  (`grab`, `drag`, `drop` with Shift), the pane's drag not carrying the
  key; so was selecting the copy before C on a server started anew, and
  the camera's turn for the pictures.

### The main toolbar

Over the stage, in the middle (`.toolbar`, STYLE.md). A tool that cannot be
used now is off, not hidden.

| C++ | Where | Here |
| --- | --- | --- |
| "Add" [Ctrl+I]: a file dialog; the file's geometry joins the project | `_init_main_toolbar` l.9312, `Plater::add_file` l.24872 | **added**: an STL or a 3MF, into an empty project as much as into one with objects; what is added is selected. Each object keeps its turn and size and is stood on the bed at the free place nearest the middle of the plate shown |
| "Add plate" | l.9321, `PartPlateList::create_plate` l.4378 | **added**: at most 36 (PartPlate.hpp `MAX_PLATE_COUNT`); the new plate is the one shown, and what stood beside the plates where it comes to stand is on it |
| Removing a plate: its objects are not deleted, they are left beside the plates that remain; the last plate stays | `PartPlateList::delete_plate` l.4531, `compute_origin_for_unprintable` l.3900 | **added**, the same place beside the plates. Which plate is shown afterwards is the reference's rule |
| "Arrange all objects" [A], "Arrange objects on selected plates" [Shift+A], with a popup of options ("Spacing", "Auto rotate for arrangement", "Allow multiple materials on same plate", "Avoid extrusion calibration region", "Align to Y axis") | l.9352, `_render_arrange_menu` l.8057, `ArrangeJob.cpp` | **added**, the reference's arranger (below). A click on the tool opens the options under the toolbar, as the reference's toggled item does (`#arrange-panel`, pressed while open, Escape closes it): "Spacing", a slider 0 to 100 with its number (0.05 a step, never under 0), "0 means auto spacing.", "Auto rotate for arrangement", "Allow multiple materials on same plate", "Avoid extrusion calibration region" (only for a Bambu Lab printer that scans its first layer), "Align to Y axis" (off and greyed while turning is on), then "Arrange" (every plate) and "Reset". Kept at once in the app's settings under the reference's keys of its `arrange` section (`min_object_distance_fff` / `_fff_seq_print`, `enable_rotation_fff` / `_fff_seq_print` by the project's print sequence, `allow_multi_materials_on_same_plate`, `avoid_extrusion_cali_region`); "Align to Y axis" is not kept and follows the printer (an i3's: on), as Plater.cpp sets it; "Reset" takes the kept ones away. A arranges every plate; Shift+A the plate shown, as the reference does. **differs**: Shift+A with objects selected arranges those around what else stands on their plates, which stays where it is (the reference's `prepare_selected`, which no key or menu of its reaches) |
| "Split to objects", "Split to parts" | l.9376–9395 | **added** (below) |
| "Auto orient", "Variable layer height", "Assembly View" | l.9331–9400 | **not yet** |

### Arrange

The reference arranges with libnest2d (`deps_src/libnest2d`) driven by
`libslic3r/Arrange.cpp` and `Jobs/ArrangeJob.cpp`; this is ported in
`crates/edge-arrange` (no UI) and `crates/edge-engine/src/arrange.rs`
(`Model::arrange_with`), with the no-fit polygons in `edge-clip`
(`nfp_convex`, `minkowski_sum_convex`, `inner_fit_rectangle`).

| C++ | Where | Here |
| --- | --- | --- |
| Each object is the convex hull of its model parts on the ground; with the spacing at 0 ("auto") it keeps 1 mm round it, 6 mm with normal supports, 24 mm (half each with tree supports anywhere) with tree supports, supports counting only where an overhang or paint would make them; a spacing given is halved onto each; never more than the bed allows | ModelArrange.cpp `get_instance_arrange_poly`, `instance_may_get_support`, Arrange.cpp `update_selected_items_inflation` | **added** (`edge_arrange::outline`, `may_get_support`, `support_allowance`) |
| The bed shrunk 1 mm (5 printing by object) and by the skirt's distance and loops; by object, grown back by half the clearance and every object kept the extruder's clearance radius apart (a nozzle's width for objects under the nozzle's height); the objects ordered and checked against the rod and lid heights | `update_arrange_params`, `seq_print_min_obj_distance`, `objfunc` l.612 | **added**, compared with the C++ on 8 parts of rising height: the same two plates, every part within 0.00 mm of the C++'s |
| Items taken largest first (priority, hotter bed, first filament, area; by object shorter first); each tried on every bed open, the best score taken, a bed opened when none takes it; an item no empty bed takes put on none; a large one tried again against the bed's excluded areas alone | libnest2d `firstfit.hpp`, `selection_boilerplate.hpp` | **added**. **differs**: items alike in all of these keep the order they were given in (the C++ sorts unstably); a bed is opened at most once more for an item an empty bed did not take (the C++ can open beds without end: see the errors below) |
| The first item to the bed's corner; each next where the no-fit polygons of what is on the bed (and the bed's own inner fit) leave room, searched from every corner of the free region along its outline with NLopt's subplex, scored by `objfunc` (distance to the pile's middle and the bed's corner, the pack's density, lining up with a like neighbour, like heights near each other, filament changes few, materials not mixed when not allowed); with fixed items, scored from the bed's middle or the prime tower's (`fixed_overfit`); the four turns 0, 45, 90, 135° tried when turning is on, after the turn of the smallest box (rotating calipers) | `nfpplacer.hpp` `_trypack`, Arrange.cpp `objfunc`, `fill_config`, `rotcalipers.hpp` | **added**: the optimiser is NLopt 2.x's `sbplx`/`nldrmd` on one variable ported step for step (`edge_arrange::minimize`), the searches from the corners run side by side and the first best taken, so any thread count gives the same answer |
| The pile put in the middle of the bed (or at `best_object_pos`), moved clear of fixed items to the nearest place they leave | `finalAlign` | **added**. **differs**: the box lined up is that of what was placed, and only that moves (the C++ counts fixed objects in it and moves them too: see the errors below) |
| Every plate: all but the locked plates and those printed in their own sequence pooled; a prime tower's ground on each bed where the print will have one; the bed's excluded areas; plates added for what does not fit; the empty plates at the end taken away; the objects listed in the order placed | `prepare_all`, `prepare_wipe_tower`, `finalize`, `rebuild_plates_after_arrangement` | **added**. **differs**: the prime tower's ground is estimated from its width, the prime volume, the filaments, the layer height and the brim, for a box or rib wall (WipeTowerEstimate.cpp's Type 2 case), where the C++ takes the tower drawn in its scene or its own fuller estimate; a locked plate left empty is kept |
| One plate: what is on it, every other plate left as it is; refused on a locked plate | `prepare_partplate` | **added**. **differs**: what the plate cannot hold goes on plates added after the last, one for each bed (see the errors below) |
| Not arranged: what is set not to print, what has no size, what is taller than the printer | `m_unprintable`, `check_unprintable` | **differs**: left where it stands; the C++ moves it past the last bed |
| What no bed holds: past the last plate | `postprocess_arrange_polygon` | **added**, beside the plates, one after another, and said |

Compared with the C++ on fixtures (its `--arrange 1 --export-3mf`, the
scripts in the PR): cubes of one size, cubes of several sizes, long, round
and L-shaped parts of distinct sizes, 30 parts for a plate of 25, and
parts of rising height printed by object come out the same to the
hundredth of a millimetre, on the same plates. Where parts are alike in
area (the C++'s order among them is unstable) or with turning on (boxes of
alike area on an outline grown a nanometre differently by Clipper 6 and
Clipper2 tip the tie), the order or the turns differ and the piles with
them; the plates, the pile's box, the gaps (never under the spacing) and
nothing off the bed agree. 50 parts arrange in about 50 ms.

In the served page (`--serve`, 800 × 600, dark and light), with
`mcbrim_test.3mf` on the U1's presets: the Arrange tool opened its
options; "Arrange" laid every plate's 16 objects on plate 1 and took
plates 2 and 3 away, Undo brought the three plates back; plate 2 locked
from its tile's menu and A pressed: plate 2's four left where they stood,
the other twelve on plate 1, plate 3 taken away; an object moved onto
another and Shift+A with it selected put it back in the free place, the
others unmoved; "Spacing" 60 typed and "Arrange": plates 3, 4 and 5 added,
the locked plate left as it was, the 60 kept in the app's settings under
`min_object_distance_fff`; "Reset" gave 0 back.

### Parts

Read from `GUI_ObjectList.cpp` (`load_subobject`, `load_modifier`,
`load_generic_subobject`, `create_mesh`, `del_subobject_from_object`,
`change_part_type`, `set_volume_type`, `split`, `set_extruder`),
`GUI_Factories.cpp` (`create_bbl_part_menu`, the part menu the list
shows; `part_menu`; `append_menu_items_add_volume`;
`append_menu_item_change_filament`), `ExtraRenderers.cpp`
(`BitmapChoiceRenderer`), `GLCanvas3D.cpp` (`on_mouse`, `do_move`),
`Selection.cpp` (`center`, `drop`, `mirror`) and
`Gizmos/GizmoObjectManipulation.cpp`.

| C++ | Where | Here |
| --- | --- | --- |
| The part menu: "Delete", "Edit text", "Repair", "Simplify", "Center", "Drop", "Mirror", "Hide"/"Show", "Separate", "Split" ("To objects", "To parts", "By painted colour"), "Edit in Parameter Table", "Change type", "Reload from disk", "Replace with STL", "Edit in Blender", "Edit in FreeCAD", "Fillet / chamfer", "Export as one STL", then "Change Filament", "Override paint", the support groups, "Precise Seam Type" | GUI_Factories.cpp l.2539–2585, l.2998 | **added**: "Delete", "Center", "Drop" (both "Move Object" steps), "Mirror" ▸ "Along X axis"…, "Split" ▸ "To objects" (off, as the C++ has it for a part) and "To parts", "Change type", "Change Filament" ("Default" and each filament by its preset's name; "Set Filament for selected items" for several; only with more than one filament, for parts and modifiers). **differs**: "Change type" is a sub-menu of the six kinds, where the C++ asks in a dialog ("Choose part type", "Type:"); "Split" is on whenever "To parts" can be done (see "Errors found in the C++"). **not yet**: the rest (each needs something the model does not have: text and SVG parts, mesh repair and simplifying, hiding, a part made an object of its own, the table of "Edit in Parameter Table", reloading from a file, exporting). "Rename" is not in the C++'s shown part menu either; a double click or F2 renames |
| "Height range Modifier" (no tooltip) after them, and "Edit Process Settings" after "Printable" | GUI_Factories.cpp l.967, l.3262 | **added** (eighteenth pass): "Height range Modifier" gives an object without ranges one from 0 to 2 mm, selects "Layers" and turns the Process section to "Objects"; "Edit Process Settings" turns it to "Objects" for what is selected. **not yet**: "Color Pattern...", "Edit in Parameter Table" (the table, section 2) |
| "Add part", "Add negative part", "Add modifier", "Add support blocker", "Add support enforcer", "Add precise seam" in the object's menu, each "Load..." and the generic shapes "Cube", "Cylinder", "Sphere", "Cone", "Disc", "Torus", text, SVG… | GUI_Factories.cpp l.426, l.659, l.922 | **added**: the first five, each with "Load..." (an STL's or a 3MF's printed geometry, as it stands in its file, named after it: "Load Part" / "Load Modifier") and "Cube" (a tenth of the bed's greater side, square to the bed, beside the right front corner of the object's box at its foot: "Add primitive", named "Generic-Cube"). The part made is selected. **not yet**: "Add precise seam", "Height range Modifier", "Color Pattern...", and "Cylinder", "Sphere", "Cone", "Disc", "Torus" and the rest, which wait for `edge_mesh::primitives` to make them (it makes boxes only) |
| A new model part takes the object's `extruder` as its own; any other kind none | GUI_ObjectList.cpp l.2416–2421, l.2529–2534 | **added** |
| Deleting a part: not the last solid one ("Deleting the last solid part is not allowed."); when one volume is left, its config goes to the object | l.2856–2913 | **added**, the filament going to the object; a part's other settings stay with the part (the model keeps no settings of an object's) |
| Changing a part's type: the last solid part keeps its type ("The type of the last solid object part is not to be changed."); the volumes are sorted by type after | `change_part_type` l.5819, `set_volume_type` l.5930 | **added**. **differs**: a part keeps its place in the list; the reference sorts the volumes by type |
| "Split" of a part: a part for each piece, named with `_1`, `_2`…, each given the original's filament (`extruder_id`) | `ModelVolume::split` Model.cpp l.3186 | **added**, the pieces in the part's place, keeping its transform (see the error at l.3014 below) |
| Modifiers carry settings for the region inside them; support blockers and enforcers say where supports go | `PrintObject`, `SupportMaterial` | **added** (the engine's regions, `crates/edge-engine/src/regions.rs`, aceRage/EdgeSlicerRS#79): a modifier's settings are laid over what of its object's parts lies inside it, a blocker takes supports out, an enforcer adds them; what is not followed is said in a note. A modifier's settings are set on its pages ("Objects", a part or modifier selected) since the eighteenth pass, and they are kept through every edit, listed and drawn see-through in their colours |

### Plates

Read from `PartPlate.cpp` (`render_icons`, `set_bed_type`, `set_print_seq`,
`set_spiral_vase_mode`, `set_first_layer_print_sequence`,
`set_other_layers_print_sequence`, `PartPlateList::move_plate_to_index`,
`duplicate_plate`, `lock_plate`, `preprocess_arrange_polygon`),
`PlateSettingsDialog.cpp`, `Plater.cpp` (`select_plate_by_hover_id`,
`open_platesettings_dialog`, `rename_plate`, `duplicate_plate`,
`normalize_bed_types`), `GUI_Factories.cpp` (`create_plate_menu`,
`plate_menu`, `append_menu_item_locked`, `append_menu_item_plate_name`),
`MainFrame.cpp` (the Edit menu), `Jobs/ArrangeJob.cpp`,
`libslic3r/PlateBedTypes.cpp` and `libslic3r/Format/bbs_3mf.cpp`. The
engine's side is `crates/edge-engine/src/plates.rs`; the page's
`ui/plates.js`.

| C++ | Where | Here |
| --- | --- | --- |
| Icons down the right of every plate, each acting on its plate after showing it | `render_icons` l.1154, `select_plate_by_hover_id` l.30481 | **added** for the plate shown (see section 9, "Plates"): remove, arrange, lock, customize, name, to the front. **not yet**: orient, "Print history" |
| The plate's menu (the object list's plate row; the right button on a plate): "Select All", "Delete All", "Arrange", "Reload All", "Auto Rotate", "Delete Plate", "Add Primitive", "Add Handy models", "Add Custom Models", "Add Models", then "Lock" / "Unlock", "Edit Plate Name" | `create_plate_menu` l.2831, `plate_menu` l.3149 | **added**, with their help text as tooltips, on the plate's tile (top left of the stage), its row in the object list and the plate in the scene, by the right button or Shift+F10 / the menu key; after them "Customize current plate", "Move plate to the front" (off for plate 1) and "Duplicate Current Plate" ("Duplicate the current plate"), which are the icons' and the Edit menu's in the reference. **not yet**, not shown: "Reload All", "Auto Rotate", the four ways to add a model (one "Add" [Ctrl+I] is in the toolbar) |
| "Edit Plate Name": "Plate name", at most 250 characters, OK / Cancel; Enter is OK; an empty name is none | `PlateNameEditDialog` PlateSettingsDialog.cpp l.679 | **added**. The name stands at the plate's top left in the scene ("Untitled" without one, **had**), in its tile's tooltip and at its row ("Plate 2 (name)") |
| Lock: a locked plate's copies are left where they are by "Arrange all", which arranges the rest; arranging a locked plate alone is refused: "This plate is locked.\nCannot auto-arrange on this plate." | `preprocess_arrange_polygon` PartPlate.cpp l.5407, ArrangeJob.cpp l.198, l.407 | **added**. A lock is drawn on the plate's tile and at its row; the lock tool is pressed while it is locked |
| "Plate Settings": "Plate name"; "Bed type" ("Same as Global Plate Type" and the printer's plates; off for a printer without a choice); "Print sequence" ("Same as Global Print Sequence", "By Layer", "By Object"); "Spiral vase" ("Same as Global", "Enable", "Disable"); "First layer filament sequence" ("Auto" / "Customize" with the filaments to drag into order); "Other layer filament sequence" ("Auto" / "Customize": ranges "Layer" [from] "to" [to or "End"], each with its order, added and taken off by + and −); "Please input layer value (>= 2)." | PlateSettingsDialog.cpp l.371–520, `open_platesettings_dialog` Plater.cpp l.30400 | **added**, the same rows and words, the orders as chips of the filaments' colours dragged into place or moved with Left and Right. The ranges are put in order of their first layer, as the reference sorts them. **differs**: "Enable" asks nothing and changes no other setting, where the reference asks first and sets what vase mode needs ("Spiral mode only works when…", the same as the process page's, **not yet** in section 2); a range's end before its start is refused, where the reference moves the end |
| What each setting does when sliced | BackgroundSlicingProcess.cpp `apply` (`new_config.apply(*plate->config())`) | the plate's own settings are laid over everything else for its slice, a plate read from a file's as much as one set here (`Loaded::plate_settings`). **Bed type**: followed: the plate's bed temperatures and `curr_bed_type`. **Print sequence**: read, but "by object" is not followed: the print says "print_sequence "by object": printing object by object is not followed; every object is printed together, layer by layer" (the same as for the project's own). **Spiral vase**: as the project's own: noted as not followed ("vase mode is not followed…") and turning off what it turns off (arc fitting, scarf seams). **First layer filament sequence**: followed (**had** for a file's). **Other layer filament sequence**: noted as not followed ("other_layers_print_sequence: above the first layer tools are taken in the order that wastes least") |
| "Move plate to the front": the plate becomes the first with what stands on it, the others one back; it is then the one shown | `move_plate_to_index` l.4879 | **added**. Every plate's slice from the first to the one moved is let go, a plate's number being in what is sliced |
| "Duplicate Current Plate": a new plate after the last with a copy of each object on the plate, where it stood | `PartPlateList::duplicate_plate` l.4461 | **added**, the plate shown afterwards. **differs**: an object with copies on other plates is copied with its copies on this plate only (see "Errors found in the C++"); one undo step |
| Another printer that does not take a plate's bed type: it is taken away, "Plate bed types reset" | `normalize_bed_types` l.21433 | **added** (section 2) |
| Keys for plates | KBShortcutsDialog.cpp | Shift+A "Arrange objects on selected plates" (**had**); the reference has no other key for a plate. A plate is selected by a click on it, its tile or its row (**had**) |

### The painting tools

Read from `Gizmos/GLGizmoPainterBase.cpp/.hpp`, `GLGizmoFdmSupports.cpp`,
`GLGizmoSeam.cpp`, `GLGizmoFuzzySkin.cpp`, `GLGizmoMmuSegmentation.cpp`,
`GLGizmosManager.cpp` and `libslic3r/TriangleSelector.cpp/.hpp`. The page's
side is `ui/paint.js` (and `paint.css`), the session's `src/api/paint.rs`,
the selector `crates/edge-mesh/src/selector/` and the model's paint
`Model::set_paint` (section 13 above, and section 15).

| C++ | Where | Here |
| --- | --- | --- |
| The selector: each triangle of a part split as finely as the stroke needs (sides longer than the edge limit halved, the limit a fifth of the brush's radius and at most 0.05 mm, 0.1 mm for a height range), a state for each piece, pieces put back whole where all of a triangle's are alike, the free ones collected when half the list is free | `TriangleSelector::select_patch`, `split_triangle`, `remove_useless_children`, `garbage_collect` | **added**: `edge_mesh::selector::Selector`, the same tree, corners shared with the neighbours across a split side, the same limits. Codes the C++ wrote read back the same (174 386 in the reference's projects, `edge-3mf/tests/paint_codes.rs`), and codes written here are written back the same by the C++ |
| The cursors: "Circle" (what is under a disc as the camera sees it, only faces turned to the camera), "Sphere" (a ball), a drag a capsule of either between two points, "Height range" (a band of the world's heights), what the section view cuts away left out; a mesh scaled unevenly tested in the world | `Cursor`, `Circle`, `Sphere`, `Capsule2D`, `Capsule3D`, `HeightRange` | **added** (`selector::Cursor`) |
| Smart fill: from the triangle under the pointer across edges where the faces turn by no more than "Smart fill angle"; on hover the faces it would take are marked lighter, the press paints them | `seed_fill_select_triangles`, `seed_fill_apply_on_triangles`, `get_seed_fill_color` | **added** |
| Bucket fill (the colour tool's "Fill", with "Edge detection" its angle, else any angle), "Triangle" (the piece under the pointer) | `bucket_fill_select_triangles` | **added**. **differs**: a "Triangle" dragged paints the triangle it is let go on; the reference leaves it marked (listed below) |
| "Gap fill": groups of pieces of one state smaller than "Gap area" shown in the least state round them; "Perform" paints them so | `TriangleSelectorPatch::update_triangles_per_patch`, `update_selector_triangles` | **added** (`Selector::gaps`, `fill_gaps`) |
| A stroke: the pointer's way since the last event sampled a pixel apart, the hits cut into runs on one part, a run on one triangle reduced to its ends, a longer one simplified to a tenth of the brush's radius, capsules between the points kept | `get_projected_mouse_positions` | **added**. **differs**: the points kept are the hits; the reference moves them onto a plane through three of them and looks for the nearest triangle again |
| What the pointer is on: a ray cast at each printed part, the nearest hit not cut away by the section view | `update_raycast_cache`, `MeshRaycaster` | **added**: in the session, a tree of boxes for each part (`edge_mesh::raycast`) |
| The buttons: left paints the tool's state, right its other one ("Block …"; the colour tool has none and the right button moves the view; fuzzy skin's right erases), Shift with the left erases; Ctrl with a button is the view's; a press off the object is the view's | `gizmo_event`, `on_mouse`, `get_left_button_state_type`, `get_right_button_state_type` | **added** |
| One step of the history when the button is let go, named as each tool names it ("Enforce supports", "Block supports", "Unselect all"; "Enforce seam", "Block seam", "Remove selection"; "Add fuzzy skin", "Remove fuzzy skin"; "Painted using: Filament %1%", "Remove painted color"); "Erase all painting" and "Perform" one step each ("Reset selection"; the colour tool's "Perform" "Gap fill") | `gizmo_event` (`LeftUp`), `handle_snapshot_action_name`, the panels | **added**. **differs**: a stroke that changed nothing is no step (the reference takes one each time) |
| Undo and redo while a tool is open: the tool reads the paint back from the model | `on_load`, `update_from_model_object` | **added** |
| The ring of the brush round the pointer ("Circle"), a ball at the point under the pointer ("Sphere", blue with the left button, red with the right), the band's outlines on the object ("Height range") | `render_cursor_circle`, `render_cursor_sphere`, `render_cursor_height_range` | **added** |
| The object while painting: its own triangles in the neutral grey (supports, seam, fuzzy skin) or each part in its filament's colour (colours), the paint over them: enforcers green {0.5, 1, 0.5}, blockers red {1, 0.5, 0.5}, stabilizer points purple, filaments in their colours; what a fill marks 1.25 times lighter. The object's other copies hidden | `TriangleSelectorGUI::enforcers_color`, `blockers_color`, `TriangleSelectorPatch`, `InstancesHider` | **added**, drawn with the page's own shading. The reference's enforcers are green in this fork (its ball cursor is blue), and so they are here |
| "Section view": a plane square to the line of sight cutting the object from the camera's side, its place from 0 to 1 across the copy's bounding sphere, kept square to where the camera looked when it began; "Reset direction"; Alt with the wheel by 0.01 | `ObjectClipper::set_position_by_ratio`, `get_clipping_plane_in_volume_coordinates` | **added**: the scene's shader cuts the object at the plane (one dot product a pixel), and painting and the pointer's hits leave what it cuts away. **differs**: the sphere's middle is the copy's box's, not the instance's origin (an object read here may have its origin anywhere). **not yet**: the cut face drawn over the cut (`render_cut`) |
| **"Support Painting" [L]**: "Tool type" "Circle", "Sphere", "Fill", "Gap Fill" (keys C, S, F, G); "Pen size" (0.4 to 8 mm, Ctrl with the wheel by 0.2); "Smart fill angle"; "Gap area" and "Perform"; "On overhangs only" (paints only what faces down more steeply than the threshold); "Threshold Angle", whose overhangs are shaded red, what touches the bed light blue; "Vertical" / "Horizontal" (a drag held to the line it began on); "Section view"; "Erase all painting"; what the buttons do | `GLGizmoFdmSupports::on_render_input_window` | **added**. **not yet**: "Paint stabilizer points" (the engine has no stabilizers), "Threshold Overlap", "Simulate Auto Supports" and "By layer (before slicing)" (they need the support generator's own rule and its preview of the layers; "Threshold Angle" here is the one the panel sets, starting at 30°) |
| **"Seam painting" [P]**: "Brush shape" "Circle", "Sphere" (C, S); "Brush size"; "Vertical"; "Section view"; "Erase all painting" | `GLGizmoSeam::on_render_input_window` | **added**. **not yet**: "Auto-paint seam" (it slices the object's walls to find where the seam would go, which waits for the walls to be asked for alone) |
| **"Paint-on fuzzy skin" [H]**: "Circle", "Sphere", "Triangle", "Fill" (smart fill); "Brush size"; "Smart fill angle"; "Section view"; "Erase all painting"; "Warning: Fuzzy skin is disabled, painted fuzzy skin will not take effect!" with "Enable painted fuzzy skin for this object" | `GLGizmoFuzzySkin::on_render_input_window` | **added**: the tools; and (eighteenth pass) the warning, shown while the object's own `fuzzy_skin`, else the project's, is "Disabled", and its link, which sets the object's own to "None (allow paint)" as the reference does. **differs**: the link is a step of the history ("Change Option fuzzy_skin"); the reference sets the object's config without a snapshot. The panel says, in the engine's words, what of the paint it does not follow |
| **"Color Painting" [N]**: "Filaments" (a button of each filament's colour with its number, "Shortcut Key 1"…; keys 1 to 9, and 1 then 0 to 6 for 10 to 16), "Selected filament n / count"; "Circle", "Sphere", "Triangle", "Height Range", "Fill", "Gap Fill" (C, S, T, H, F, G); "Pen size", "Height range" (0.1 to 8 mm), "Edge detection" with "Smart fill angle", "Gap area" with "Perform"; "Vertical" / "Horizontal"; "Section view"; "Erase all painting" | `GLGizmoMmuSegmentation::on_render_input_window`, `on_number_key_down` | **added**. **not yet**: "Spread" and "Airbrush" (EdgeSlicer's own brushes, with their own fields), "Remap filaments", "Color Pattern...", "Toggle Wireframe" |
| The panel's list of what the buttons and keys do | `show_tooltip_information` | **added**, under the fields, in the reference's words |
| The keys: L, P, H, N open and close the tools; while one is open, its own (C, S, F, T, H, G, the colour tool's 1–9), Ctrl and Alt with the wheel; Esc closes it | `GLGizmosManager::handle_shortcut`, `on_key`, KBShortcutsDialog.cpp | **added**. L, P, H, N are bound with the other gizmos' letters in `scene-edit.js` (`TOOLS`; none of the menu bar's, section 17) and listed, built, in "Keyboard Shortcuts" (its rows "Gizmo FDM paint-on fuzzy skin", "Gizmo SLA support points" for L, "Gizmo FDM paint-on seam", and the Gizmo list's two wheel rows); a tool's own keys mean something only while it is open, so they stay in its handler (`paint.js` `key`, asked first by `scene-edit.js` `key`), as the reference's gizmo takes them before the manager; the wheel in the stage's (`stage.js`). Esc is the menu's "Deselect all", which closes a tool first. Undo and Redo, by their keys or the Edit menu, take back a stroke whole, the tool open. A key typed in a field reaches the field |

What the engine does with paint made here, checked against the C++ slicer
(`edge-parity` on the file saved from the page, the reference's build of the
pinned commit):

- **Seams**: a strip painted down `cylinder` (`tools/make_fixture.py`)
  with "Seam painting": the C++ re-exports the file with the same 19 codes,
  and both slicers put every layer's seam in the strip, 229.6° to 231.5°
  round the post, the same on every layer (unpainted, both put it at 346°).
- **Colours**: a cube in filament 1 of a seven-filament project
  (`cube` with `mcbrim_test.3mf`'s settings), its top filled with filament 4
  and a stroke of filament 3 over it: both slicers make 13 tool changes; on
  the top millimetre the C++ lays 116 / 116 / 343 mm of filament 1 / 3 / 4,
  this engine 116 / 112 / 336 (what remains is the engine's own sharing out
  of colours, `edge-paint`, not the paint).
- **Supports**: followed (the engine's regions read the triangles' kept
  `paint_supports`, which `Model::set_paint` writes; `Part::support_paint`
  now reads them as every kind is read, only of the mesh the part has). On
  `mushroom` (`tools/make_fixture.py`, `enable_support=1`) the underside of
  the cap filled with a blocker in the page ("Fill", the right button's
  state; two triangles, `paint_supports="8"`): unpainted the C++ lays
  3189 mm of support on 49 layers and this engine 3433 mm on 49; with the
  blocker both lay none, and the walls are the same to 0.00 %. The same
  cap filled with an enforcer with supports switched off: neither slicer
  lays any (an enforcer does not turn supports on).
- **Fuzzy skin**: kept and saved (the C++ reads it back), not followed by
  this engine yet; the tool's panel says so in the engine's words ("the
  project has fuzzy skin painted on it, which is not followed: the settings
  alone decide"), as a slice does, while the model has it.

What an edit does to paint (Model.cpp, followed): a mirror, a scale, a
turn, a move and "Lay on face" keep it (only the transformation changes:
`ModelVolume::mirror`, `scale`); "Split to parts" and "Split" of a part
start every piece unpainted (`ModelVolume::split` resets the facets of the
first and makes the others without them); "Split to objects" of an object
of several parts keeps each part's (`ModelObject::split` assigns them); a
part deleted takes its paint with it; a filament deleted renumbers the
colours painted after it and leaves those of the one deleted unpainted
(as before). `convert_from_imperial` (which scales the mesh and keeps the
paint) has no counterpart here yet.

### Left for later

| C++ | Waits for |
| --- | --- |
| "Lay on face": its second mode, "Face under cursor" (any flat face under the pointer, recessed ones too, and Alt to use it for a moment), and laying one part of an object on a face | the first needs the page to grow a flat face from the triangle under the pointer (`grow_flat_face`) and to warn of a recessed one; the second, the faces of one part (`scene_faces` for a part) and a turn of the part within its object |
| "Fill bed with copies" (`GUI_Factories.cpp` l.1146, `FillBedJob.cpp`) | the reference's arrangement: it asks the arranger how many more fit |
| Sinking an object under the bed; the arrow along Z; "Drop" | the engine cutting what is under the bed away (it slices from an object's lowest point) |
| "Auto orient", "Mesh Boolean", "Simplify" | mesh operations the engine does not have |
| "Split" > "By painted colour" ("Convert the part's colour painting into separate parts, one per filament") | cutting a mesh along its painted borders, which the engine does only layer by layer while slicing |
| The other instances of an object taking a mirror or a turn of the object too | instances kept alike, which no tool here does yet (a part's edits are every instance's, as in the C++) |
| "Edit in Parameter Table" (the table of section 2), "Separate" / "Set as an individual object", copying and pasting parts | the table is a piece of work of its own; copying parts in the model. A modifier's and a part's own settings are set on their pages (eighteenth pass) |
| "Center" and "Drop" for an object; "Hide" / "Show" of a part | nothing; not built in this pass |
| Saving the project with its edits | **added**: section 15 |
| The glow round what is selected, the red background while something is over the edge, the ticks of the rotate ring | nothing; they are looks, left for a pass on the stage's own drawing |
| Rectangle selection (Shift with a drag), dragging rows of the list | nothing; not built in this pass |
| A plate's "Auto orient objects on current plate", "Auto Rotate" | orientation in the engine (`Orient.cpp`) |
| "Reload All" ("reload all from disk") | reading an object's source file again (`source_file` is kept with it, section 15, but nothing reads it): the part menu's "Reload from disk" first |
| "Print history" | sending to a printer (section 4); a file's `print_history` is kept with its plate |
| The plate's tools beside every plate, not only the one shown | nothing; the page shows the plate shown's, where the keys and the pointer are |
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

The seventh pass ("Lay on face", "Mirror", "Split"): in the engine,
`crates/edge-mesh/src/hull.rs` (the hull of a cube, of points inside and on
its faces, an octahedron, a tetrahedron, 200 points on a sphere, a cloud of
2000 points checked face by face and twice for the same answer, flat and
straight sets refused), `split.rs` (one piece, two that touch at a corner,
one inside out, edges wound alike), `crates/edge-engine/src/lay_on_face.rs`
(a box's six faces in order of area, the printed size, the filter past the
twelve largest, the turn down, the drawn outline) and
`crates/edge-engine/tests/tools.rs` (a box laid on a side and on its top, a
wedge on its slanted face; mirrored along each axis the copy is mirrored and
what is sliced stays a solid of the same volume, where it stood; two shells
to parts and to objects, named and placed as the reference names and places
them; a piece in the air brought down). In the session
(`src/api/edit/tests.rs`): a box laid on its side slices 30 mm tall where it
stands, one undo step; an L mirrored along X slices, layer by layer, to the
mirror image of its outer walls, each still anticlockwise, the first layer
where it was, and along Z upside down on the bed; two shells split to parts
and to objects slice to the same box of toolpaths as the one object did, two
undo steps.

In the served page (the app's browser pane, 1280 × 800, dark and light), by
the mouse and the keys: a Benchy selected, F, the 52 faces of its hull drawn
over it, the one under the pointer lit, a click on its roof's face laid it
on it ("Gizmo-Place on Face" to undo), sliced: 173 layers, 34.65 mm, on the
roof, and Cmd+Z put it back; an L selected in the list, the object's menu,
"Mirror" > "Along X axis" (the sub-menu by the pointer, the tooltips the
C++ help text), the tower at the other end, sliced and looked at in
Preview; two shells in one STL: "Split to parts" from the toolbar (the list
then shows "twoshell.stl_1" and "twoshell.stl_2" under the object, "Split to
parts" off), the menu's "Split" > "To objects" ("To parts" off), two objects
selected, sliced, both printed where they stood; one of them laid on its
side in light. The browser pane's right click did not open the page's menus,
on the stage or in the list (it does not seem to send the page a second
button), so the object's menu was opened by a `contextmenu` event on the
list's row, or by `GL.edit.menu`, and then gone through with the mouse. The screenshots are under `target/ui-shots/tools/`.

The eighth pass (parts): in the engine, `crates/edge-3mf/src/tests.rs`
(an object's and a part's own filament and a part's settings read) and
`crates/edge-engine/tests/parts.rs` (a part's own filament printed and given
back to "default"; a part moved, lifted, both lifted and brought down; the
last solid part kept; a part made a modifier and a negative part; a part of
two pieces split in its place; a part mirrored about its middle, still a
solid; a part renamed and a cube added beside the object). In the session
(`src/api/edit/tests.rs`): `twopart_groups.3mf`'s second part in filament 2
slices with both filaments and undo gives the bytes back; its third part
moved 15 mm slices elsewhere and undo gives the bytes back; a cube modifier,
a part from a file, renamed, split, retyped, deleted (the last solid part
refused both ways), mirrored, and the seven steps taken back by name. The
CLI's G-code is the starting commit's byte for byte for the three plates of
`mcbrim_test.3mf`, `twopart_groups.3mf`, the `halves` and `inset`
fixtures and eight more projects (`twotone` fails alike in both: its
settings name one filament).

In the served page (the app's browser pane, 1280 × 820, dark and light),
with `mcbrim_test.3mf` and the `twotone` fixture on the U1's presets: a part
of `mcbrim_test.3mf` selected in the list (its outline and box in the
scene), its filament list ("default", 1 to 7) and another filament picked;
`twotone`'s first part selected by Alt and a click in the scene, given
filament 2 from the part menu ("Change Filament"), the scene's colour
changed, sliced: Preview's "Filament" view showed one filament; undone with
the toolbar, sliced again: two filaments and a prime tower. The part moved
76 mm by the Move arrow ("Volume Operations"), lifted 5 mm by typing Z,
sliced (55 layers, to 11.05 mm); deleted with Del (the object then in
filament 2), undone with Ctrl+Z; a two-shell STL added by "Add part" ▸
"Load...", split with the toolbar's "Split to parts", one piece made a
modifier ("Change type", see-through, the print's note under "About this
print"); "Add modifier" ▸ "Cube"; a part renamed by a double click; turned
by 45° typed into "Relative"; mirrored from its menu, sliced; each step
undone back to an unmodified project. Two parts selected with Cmd in the
list ("Group Operations"); an object then Cmd-clicked gave "Selection
conflicts". The browser pane's own clicks with Cmd and with Alt and a drag
did not always reach the page, so Alt with a drag was run through the
editor's own `grab`, `drag` and `drop` (the part moved 48.74 mm, "Move
Object"). The screenshots are under the session's scratch folder,
`ui_parts/shots/`.

The tenth pass (plates): in the engine, `crates/edge-engine/tests/plates.rs`
(above, and: a locked plate left alone by arranging every plate and refused
alone, the others arranged; a plate left alone by `arrange_leaving`; a
plate's own settings in its slice's settings and its stamp, a name and a
lock not; bed types another printer does not take taken away; what cannot
be set refused, nothing changed). In the session
(`src/api/edit/tests.rs`): `mcbrim_test.3mf` with plates 1 and 2 sliced:
renamed (both slices kept; the same name again no step), locked (kept),
arranged alone (refused in the reference's words) and all (the locked
plate's copies where they were), customized (plate 1's slice let go, plate
2's kept; a range from layer 1 refused), plate 3 to the front, duplicated,
and the seven steps taken back by name to an unmodified project; on the
U1's presets a plate's "Engineering Plate" refused, "Cool Plate" kept by
another U1 nozzle and taken away by an Elegoo Neptune 4 with the notice,
one undo step. The CLI's G-code is the starting commit's byte for byte for
the three plates of `mcbrim_test.3mf`, `twopart_groups.3mf` and the
`cube`, `tower`, `bridge`, `halves`, `mushroom` fixtures (`twotone` fails
alike in both).

In the served page (`--serve`, the app's browser pane, 1280 × 820 dark and
900 × 560 light), with a copy of `mcbrim_test.3mf`: plate 1 renamed "Arms
front" from its tool beside it (the name on the plate, at its row, the
unsaved dot); locked from its tool (the lock on its tile and row, the tool
pressed); "Arrange all objects" from the toolbar (plate 1 where it was,
plate 2 laid out); its arrange tool (the status line: "This plate is
locked. Cannot auto-arrange on this plate."); the tile's menu by the right
button; "Customize current plate" from it: "Smooth Cool Plate", "By Layer",
the first layer's order with filament 7 dragged and moved by Left to the
front, the other layers customized, a second range left empty and OK
refused with "Please input layer value (>= 2).", the range taken off with −
and OK; sliced: 45 °C bed, `curr_bed_type = Cool Plate`, the
other-layers note under "About this print"; plate 3 moved to the front
from its tile's menu, plate 2 duplicated (plate 4, eight copies); the six
steps undone by name to an unmodified project and redone; saved as, closed
and opened again: the four plates as left (the C++ slicer re-exporting
that file shows them the same); switched to an Elegoo Neptune 4: the
"Plate bed types reset" box, undone. The browser pane's right button does
not reach the page, so the tile's menu was opened by a `contextmenu` event
and then gone through with the mouse. The screenshots are under the
session's scratch folder, `ui_plates/shots/`.

### Errors found in the C++
- TriangleMeshSlicer.cpp l.2398–2406 (`triangulate_slice`): each corner
  of a cap triangle is looked for among the cut's vertices by nearness
  (`is_equal`), and where none is found a new vertex is added. A corner
  added so, or matched to the wrong one of two near vertices, leaves the
  cap not sharing its edges with the side: the half has open edges. The
  person sees nothing on a Mac or Linux; on Windows only, a box asks
  "non-manifold edges be caused by cut tool, do you want to fix it now?"
  (GLGizmoCut.cpp l.10966) and sends the part to Windows' repair. The
  cut here shares every cap corner with the side by construction
  (`edge_mesh::cut`).
- CutUtils.cpp l.2050–2052 (`perform_with_plane`; the same at l.1736,
  l.1924): `post_process` adds a side to the result when it has any
  volume, before `delete_extra_modifiers` takes away the modifiers that
  meet none of its printed material. A plane within half a millimetre
  above the object's printed parts (which `cut_plane_center_allowed`
  lets it be) with a modifier reaching above it makes an upper "half" of
  that modifier alone, which is then deleted: an object with no volume
  reaches the model and the object list. Here a side with no printed
  part makes no object.
- CutUtils.cpp l.868–906 (`reset_instance_transformation`): the cut
  instance's turn and size are put into the halves' meshes, and every
  other instance is given only its own turn about z (its size dropped,
  `get_matrix_no_scaling_factor`). Another copy of an object whose cut
  copy was turned about z comes out turned by both, and one sized
  otherwise comes out the cut copy's size: the copies of the halves no
  longer stand as their object's did. Here each keeps its turn and size
  relative to the cut copy.
- Model.cpp l.1399–1402 (`assign_copy`, by `clone_for_cut` l.2293): the
  halves take the object's height ranges, variable layer heights and
  brim ears at the object's heights. The upper half, stood on the bed,
  or a half laid on its cut or flipped, has them where the object had
  them, not where its material now is: a range's settings land on
  another part of it. Kept the same here, there being no one right way
  to move them for a tilted cut; listed.
- GLGizmoCut.cpp l.6635 with l.975–983 (`update_bb`,
  `get_grabber_mean_size`): the dovetail's depth (and with it its width,
  four times the depth) starts at half the grabbers' mean size, which
  with `ENABLE_FIXED_GRABBER` is `32 * INV_ZOOM`, a size on the screen.
  The groove a person is first offered depends on how far the camera is
  zoomed when the tool opens, not on the object: zoomed in on a large
  part it is a sliver, zoomed out on a small one it is wider than the
  part. Here it goes by the object's size, as the size-based branch of
  the same function does.
- GUI_ObjectList.cpp l.3621 (`edit_cut`) with GLGizmoCut.cpp l.10902
  (`InvalidateCutInfo`) and l.10518: "Edit cut..." finds the halves of
  a cut by their cut id, but a cut without connectors or a dovetail (the
  common one) takes the halves' cut id away, so only the half chosen is
  taken out. The other stays on the bed, the panel says "One half of
  this cut is missing from the project", and "Perform cut" makes both
  halves again: the person ends up with three pieces, one of them twice.
  Here the halves are those carrying the same recipe on that plate.
- GLGizmoCut.cpp l.10069–10081 (`object_part_mesh`) with l.10575–10580
  (`begin_reedit`): the uncut mesh is kept in the object's coordinates
  (the volumes' matrices only), and the plane in the instance's turned
  and sized ones less its move; the stand-in is put back with a half's
  instance, which after a cut has no turn or size (they went into the
  halves' meshes, `reset_instance_transformation`). For an object that
  was turned or sized when it was cut, "Edit cut..." shows it unturned
  and unsized under a plane still in the turned frame: the cut made
  again is not where the first was. (`instance_linear` is kept, but used
  only to carry the painting.) Here the uncut mesh is kept in the
  halves' coordinates.
- GLGizmoCut.cpp l.11692–11712 (`apply_cut_connectors`): a connector's
  mesh is added with `add_volume`, which centres it on its box, while the
  tool draws it uncentred at the point placed. A triangle's box is not
  centred on its middle, so a triangular plug and its hole come out a
  quarter of the radius away from where they were drawn and placed. Here
  only the height is centred.
- GLGizmoCut.cpp l.9515–9530 (`is_outside_of_cut_contour`): the outline
  checked against the cut contour leaves the connector's "Rotation" out,
  so a turned triangle or square is passed or refused as if unturned: one
  that pokes out of the part once turned is cut through the outside of
  it. Here the outline is checked as drawn.
- DrawCut.cpp l.3163–3181 (`draw_cut_empty_sides`): which side a drawn
  cut leaves empty is asked of the mesh's own corners alone. A loop drawn
  on a face of a coarse mesh (a box's top) holds none of them, so the
  panel says "The stroke does not separate the part" and "Perform cut"
  is off for a plug that does separate it. Here a side still empty is
  also asked of the cutter's own corners that lie well inside the part.
- DrawCut.cpp l.2083–2109 (`draw_cut_core_face`): the core's face is
  worked out without the mesh, while the cutter (`draw_cut_build_cutter`)
  turns a loop's core normal by the mesh where the samples' normals
  cancel (a loop over a strongly curved patch). There the face the panel
  measures and draws can be the other way round from the one cut. Here
  both are given the mesh.
- GLGizmoCut.cpp l.3235–3240 (`compute_draw_extension_clearance`,
  `clearance_along`): an open line's extension stops at the first face
  its ray meets. A line drawn on a flat face lies in that face's plane,
  so the ray off its end runs along the face and meets the face's own
  edge: the end's extension is clamped to that edge and never reaches
  past the silhouette, which is what it is there for, and the strip's
  end then lies on the part's side face. Here only a face the ray goes
  into (the rest of the part, met from outside) stops it.
- GLGizmoCut.cpp l.3559 (`commit_draw_points`): an edit of the line's
  points makes the chain closed whatever it was ("Editing is only
  reachable on a VALID stroke, which means a CLOSED chain"), but a line
  used open with "Cut along the line" is valid too: moving one of its
  points turns it into a loop. Here an open line stays open.
- WipeTowerDialog.cpp l.783–800 (`read_matrix_values`): OK reads every
  box back and divides it by the multiplier, though each box holds the
  volume times the multiplier cut to a whole number, so a dialog opened
  and closed with OK changes every volume (418 at 0.3 is shown 125 and
  saved 416.67). A person sees the purge creep down each time.
- WipeTowerDialog.cpp l.791–794 and l.448: with the multiplier at 0 OK
  saves a matrix of zeros, and a box typed in stores its value over 0
  (infinity). Raising the multiplier again does not bring the volumes back.
- WipeTowerDialog.cpp l.688 (`update_warning_texts`): a box from filament
  j to filament i is measured against filament i's least flush
  (`m_min_flush_volume[i]`, the one flushed in), where "Re-calculate"
  (l.753) and the least's own meaning (Plater.cpp `get_min_flush_volumes`:
  what stays in the nozzle of the filament flushed out) take filament j's.
  With filaments whose least differs a box is marked wrong or right wrongly.

Arranging (each left out of the port; none worked around):

- `libslic3r/Arrange.cpp` l.1013 (`_arrange`): with "Auto rotate for
  arrangement" the turn of the smallest box is measured on the outline as
  the object is already turned and then set as the item's whole turn,
  applied to the outline not turned: an object that was turned on the bed
  is laid at a turn that does not give it its smallest box.
- `Arrange.cpp` l.835 and l.849 (`AutoArranger::operator()`, `preload`):
  the count of items to place takes in the fixed items twice (the prime
  towers of all 36 beds, the excluded areas), so the share left to place,
  which weighs the pile's density, is far too small whenever a print has
  a prime tower: piles are laid out less tightly than the objective means.
- `Arrange.cpp` l.1079 and l.1154 (`process_arrangeable`, `arrange`): an
  outline of fewer than three corners is dropped from the list, and the
  results are then written back by position, so every object after it
  gets the next one's place and plate.
- `Arrange.cpp` l.816 (`sortfunc`) with `std::sort`: two items with
  different filament lists starting with the same filament are neither
  before nor after each other whatever their areas, which is no strict
  order; items alike in everything land in an order the library chooses,
  differently on each system.
- libnest2d `placers/nfpplacer.hpp` l.178 and l.222 (`EdgeCache`): a free
  region's outline is walked from its first corner to its last and not
  back, and its last corner is no starting point, so the places along one
  side of every free region are never tried: an object can miss the gap it
  would fit.
- `nfpplacer.hpp` l.1077 and l.1172 (`finalAlign`): fixed objects (not
  virtual ones) are counted in the box of the pile that is lined up and
  are moved with it, though their moves are thrown away: with objects
  that stay (placing loose objects from the hub) the arranged ones are
  lined up as if those moved too, off the middle.
- libnest2d `selections/firstfit.hpp` l.120: the item itself is searched
  on each bed, and each search leaves it at the turn it found best, which
  the next bed's search starts from: with turning on, the turns tried on a
  later plate depend on the plate before.
- `firstfit.hpp` l.169–192: an item that fits an empty bed but not beside
  the prime tower every new bed carries (the check for what fits no bed,
  `remove_unpackable_items`, leaves the tower out) opens a new bed, fails,
  and opens another without end (`best_bed_id == MAX_NUM_PLATES` l.139 is
  never true): arranging such a print never finishes.
- libnest2d `utils/rotcalipers.hpp` l.319 (`fitIntoBoxRotation`): the
  previous box is never moved along as the calipers turn, so the turn is
  always bisected from no turn at all; an object too long for the bed is
  not turned to fit when the turn it needs is not between 0 and the first
  box narrower than the bed.
- `slic3r/GUI/PartPlate.cpp` l.5647 (`postprocess_bed_index_for_current_plate`):
  arranging one plate, every bed beyond the first is given the one place
  after the last plate, so what two or more beds' worth did not fit is put
  there piled on itself.
- `PartPlate.cpp` l.6180 (`rebuild_plates_after_arrangement`): an empty
  plate at the end is taken away even when it is locked.

- `KBShortcutsDialog.cpp` l.260–262: the list of shortcuts names L "Gizmo
  SLA support points", the key that opens "Support Painting" in an FFF
  printer's window, and leaves out N, which opens "Color Painting" (its
  `m_shortcut_key` is `WXK_CONTROL_N`). The window tells the person of a
  gizmo there is not and not of one there is. Here the rows are the
  reference's (L under its name) and N is in the tool's tooltip.

- `Gizmos/GLGizmoPainterBase.cpp` l.1080–1095 (`gizmo_event`, `LeftUp`): the
  colour tool's "Triangle" (and any pointer tool) paints at each position
  what was marked at the one before, and the release takes its snapshot
  without painting what is marked last. Drag "Triangle" across a model and
  the triangle the button is let go on stays marked, lighter, and is not
  painted. Here it is painted with the step.
- `Gizmos/GLGizmoPainterBase.cpp` l.1088–1091: a snapshot is taken on every
  release, whether or not the stroke changed anything, so painting over
  paint already there, or a click that paints nothing new, leaves a step in
  the undo history that undoes nothing. Here no step is made.
- `Gizmos/GLGizmoFdmSupports.cpp` l.1326–1341: the support tool's step names
  ("Enforce supports", "Block supports", "Unselect all") are written without
  `_L` ("BBS remove _L()"), so the undo history shows them in English in
  every language; "Unselect all" is not in the catalog at all. Here they are
  passed through the catalog like the other tools'.
- `libslic3r/MeshSplitImpl.hpp` l.238–268 (`create_face_neighbors_index`),
  which the selector's neighbours come from: run on several threads, with
  BBS's check of whether the other triangle's side is taken, so on a mesh
  with sides shared by more than two triangles which pairs are made depends
  on the threads' timing, and paint can spread (or a split side be shared)
  differently from one opening of a tool to the next. Here the pairs are
  made in one pass, in order (as `edge_mesh::split` already did).

- `PartPlate.cpp` l.4461–4487 (`PartPlateList::duplicate_plate`): the
  objects to copy come from `get_objects_on_this_plate` (l.2423), which
  pushes the object once for each of its instances on the plate, and each
  is copied whole (`m_model->add_object(*object)`), every instance with it,
  and every instance registered on the new plate and moved by the plates'
  offset. An object with two copies on the plate comes out as two objects
  of two copies each, two on top of the other two; an object with a copy
  on another plate too has that copy moved by the offset, off any plate.
  Here a new object is made for each object, with its copies on the plate
  only.
- `Plater.cpp` l.30697 (`duplicate_plate`) and l.30428–30476 (the "Plate
  Settings" handler) take no snapshot, so neither can be undone, where
  renaming, locking and moving a plate can.
- `GUI_Factories.cpp` l.3409–3412 (`append_menu_item_locked`): the menu's
  "Lock" / "Unlock" calls `plate->lock` directly: no snapshot and the
  project is not marked modified, where the icon's lock takes "lock
  partplate" (Plater.cpp l.30617).
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
- `libslic3r/MeshSplitImpl.hpp` l.239–262: `create_face_neighbors_index`
  pairs triangles in `execution::for_each` on several threads, and each
  iteration writes the entry of the *other* triangle
  (`neighbors[other_face][vertex_index] = face_idx`) while another thread
  may be reading or writing that same entry for its own triangle: a data
  race. On a closed, manifold mesh every edge has one partner, so the
  answer comes out the same; where three or more triangles share an edge,
  which of them are paired (and so how "Split to objects" and "Split to
  parts" divide the mesh) can change from one run to the next. Here the
  pairs are worked out in one pass, in order.
- `GUI_Factories.cpp` l.2576–2577 (`create_bbl_part_menu`, the part menu
  the object list shows): the "Split" sub-menu is enabled by
  `can_split(true) || can_split_by_color()`, but for a part
  `ObjectList::is_splittable(true)` is always false (GUI_ObjectList.cpp
  l.3496, `type == itVolume`), so unless the part is painted the whole
  sub-menu is off and its "To parts", enabled by `can_split(false)`, can
  never be reached from a part's menu. Here "Split" is on when "To parts"
  can be done.
- `GUI_ObjectList.cpp` l.2892–2903 (`del_subobject_from_object`): after the
  last volume's config is applied to the object it is reset, and only then
  is `last_volume->config.has("extruder")` asked, which is always false; the
  block is dead (the extruder already went over with `apply`).
- `Model.cpp` l.3253–3262 (`ModelVolume::split`): the loop that drops
  volumes whose convex hull is degenerate runs over all of the object's
  volumes, not only the new pieces, so splitting one part can delete another,
  unrelated flat part of the same object. Here only the pieces are checked.
- `libslic3r/Model.cpp` l.3014: `ModelVolume::center_geometry_after_creation`
  moves the part by the shift of its mesh as it is (`translate(shift)`),
  not by that shift turned and sized by the part's own transform. Called
  by `ModelVolume::split` (l.3186) after it sets the offset to zero and
  before it adds the old offset back, it puts each piece of a part that
  has a turn or a size of its own in the wrong place; a part as loaded
  (no turn of its own) is not affected. Here a part's pieces keep the
  part's transform and their own coordinates, so they stay where they were.
- `MainFrame.cpp` l.1407–1409: going to another project calls
  `reset_model_config()` on the object, part and range tabs to let go of
  the last project's selection, but that function (Tab.cpp l.5171) erases
  every option of the tab's keys from the configs it holds, which are the
  previous project's items: whatever object, part or range was selected
  there loses all its settings of its own, with a "Reset Options" step
  taken on the wrong project. The comment says only the pointers were
  meant to go.
- `GUI_ObjectList.cpp` l.7455–7461 (macOS, `OnEditingStarted`): a click on
  the editing column's icon always resets `get_model_tab(vol_idx >= 0)`,
  without the plate and range cases the other platforms have (l.1588–1593):
  on a range's row it takes away all the settings of the object the range
  is of, and leaves the range's.
- `GUI_ObjectList.cpp` l.5030–5073 (`edit_layer_range`) and
  GUI_ObjectLayers.cpp l.129: heights typed are taken with no check
  against the other ranges (`ranges[new_range] = config`): a range typed
  onto another's heights replaces it, ranges may overlap (the slicer trims
  them without a word), and a top equal to the bottom is taken (only a
  top under the bottom is refused), a range of no height. Here they are
  refused with the reason.
- `GUI_ObjectLayers.cpp` l.460–466 (`LayerRangeEditor::get_value`): a
  negative height says "Invalid numeric." and is then returned and applied
  all the same: a range starting under the bed. Here it is refused.
- `libslic3r/Slicing.cpp` l.259 (`layer_height_profile_from_ranges`):
  `option("layer_height")->getFloat()` with no check, and the 3MF reader
  (bbs_3mf.cpp l.3864–3920) gives a range only what its file says: a
  range written without a layer height (by another program, or by hand)
  is a null pointer when sliced. Here every range is made with one, as the
  reference makes them (`get_default_layer_config`).
- `GUI_ObjectList.cpp` l.3459–3461 (`get_default_layer_config`): the
  object's `extruder`, else the print preset's `opt_float("extruder")`
  (a process preset has none: a null pointer), is worked out and never
  used, the range being given 0.
- `ParamsPanel.cpp` l.713–736 (`notify_object_config_changed`): only
  objects and parts are looked at, so a project whose only settings of
  their own are a height range's does not mark "Objects", and is not
  turned to "Objects" when opened. Here a range counts.
- `GUI_ObjectList.cpp` l.1379–1418 and l.1735–1765: copying and pasting an
  item's settings works on settings rows (`itSettings`), which are no
  longer made (`NEW_OBJECT_SETTING`): the code cannot be reached. Pasting
  height ranges (l.1349–1371) takes no snapshot, and `emplace` leaves a
  range already at those heights as it was.
- `Tab.cpp` l.5552–5579 (`TabPrintLayer::notify_changed`,
  `update_custom_dirty`): showing a range's pages writes the object's
  layer height into a range without one, a change to the model with no
  step of the history and no unsaved mark.

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
| The pages' tree with the changed ones marked; "Advance"; search | Tab.cpp | **added**, as for the process (section 2): tabs with a count (wrapping onto rows), groups that fold, search ("Search for setting" in English, as the process's: section 2) over the kind's options (an extruder's under each "Extruder n"), the one "Advance" switch for all pages, as the reference has one mode |

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

Since the sixteenth pass a filament's or the printer's preset may be one
of the person's (section 8, "User presets"): the head of the panel carries
the cog of section 2 ("Save current Filament", "Save current Machine",
"Rename preset", "Delete this preset", "Compare presets"), what is set here
is the "dirty" preset those save, and the lists show "* " while anything is
set.

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
| Detaching from the system preset ("Detach from parent") | develop mode, which the app has not got. Saving, renaming, deleting, comparing, importing and exporting are built (sixteenth pass, section 8) |
| A Bambu Lab printer's own prime tower (WipeTower.cpp, Bambu Studio's); on a printer of one nozzle, ramming, cooling and loading on the tower (`enable_filament_ramming`), flushing into infill, objects or support; a tower moved where it fits once its flush makes it deeper | the rest of the one-nozzle tower port. Done on `feat/single-nozzle-purge`: the flush purged into the tower where `purge_in_prime_tower` asks, blocks planned from each change's volume, tools ordered by the flush, `M104` without a tool number and no wait, the Bambu reheat, `max_layer_z` at a change (README, "Changing filament on one nozzle") |
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
| `Metadata/model_settings.config` | written afresh from the model: each object's name and filament (`extruder`), each part's name, `subtype`, own filament (`extruder`, none for "default") and, once it has moved, `matrix` (see "Parts" below). Kept as the file had them, where the part is still what it was read as: every other metadata and setting of each object and part (per-object and per-part settings, modifiers' settings, `matrix`, `source_*`, Precise Seam's type, embossed text and SVG descriptions, `mesh_stat`), each plate's own (`locked`, `bed_type`, `print_sequence`, `first_layer_print_sequence`, `spiral_mode`, `filament_map_mode`, `filament_maps` with one entry for each filament, `print_history`…), a setting changed here (the lock, "Plate Settings") set in place or added where the reference writes it and one given back to the project taken out (`PlateKept::with`), a plate added here written as the reference writes a new one until something is set on it (`PlateKept::fresh`), each copy's place in the assembly view while its object has all its copies, and each copy's `identify_id` (the number the reference labels it with in G-code) while it has. Plates are numbered as they stand, each with its name and its copies |
| `Metadata/project_settings.config` | the settings, below |
| `Metadata/slice_info.config` | written afresh: the header alone, as the reference writes a project saved unsliced |
| `Metadata/layer_config_ranges.xml` (height ranges and their settings) | an object's ranges kept as the file has them until they are edited here; then written as the reference writes them (`_add_layer_config_ranges_file_to_archive`: Boost's XML with each tag on a line, a range's `min_z` and `max_z`, an `<option opt_key>` for each of its settings in order of key), under the object's place as it stands (eighteenth pass). **differs**: the file ends with a new line, which the reference's does not have, as before this pass (the writer's own, for kept ranges too) |
| `Metadata/layer_heights_profile.txt` (variable layer height), `brim_ear_points.txt`, `cut_information.xml` (cut connectors) | kept, each object's line or block written under its place in the list as it stands, so they follow an object when one before it is deleted; a clone carries its original's, as the reference's clone does. A cut connector names its part by the part's place among the object's parts, which is written as it now stands (a part before it deleted, one added after it); once the connector's part itself is deleted or split, the object's cut information is not written, and that is asked about first |
| `Metadata/custom_gcode_per_layer.xml` (pauses, custom G-code, templates, filament changes at layers) | read (`edge_3mf::CustomGcode`, by plate, plate 1's for a file without plates as the reference reads it) and written afresh in the reference's layout: Boost's XML with a new line between tags, `top_z` to 17 digits, values escaped as Boost escapes them (new lines kept), a plate with no items still written when it has an entry, an undefined mode written `MultiExtruder`; the pressure advance pattern's file is written back to the byte. Each plate's go with it (deleting a plate before it renumbers them with it). Edited from the slider (section 3). A file laid out otherwise (PrusaSlicer's `custom_gcodes_per_print_z`), which the reference does not read, is kept as it was while nothing is set here |
| `Metadata/plate_<n>.png`, `_small`, `plate_no_light_<n>`, `top_<n>`, `pick_<n>` | kept for each plate while it is as it was read (`PlateStamp`: what is on it and where, its filaments and first-layer order), under its number now; a plate that has changed is saved without pictures, which would show it as it was. **not yet**: drawing new ones |
| `Metadata/process_settings_<n>.config`, `filament_settings_<n>.config`, `machine_settings_<n>.config` (presets the file carries) | kept while the project is saved with its file's own settings; with presets chosen here they are not, and that is asked about first |
| An object cut here ("Cut", section 13) | each part a new object (or a part of the one, with Cut to parts) with its own mesh, written as any part made here; no `cut_information.xml` entry, what the reference writes for its own cut without connectors; a cut with connectors or a dovetail an entry for each object it made, `<cut_id id check_sum connectors_cnt>` and each connector's part (`volume_id`, `type`, `r_tolerance`, `h_tolerance`), as `_add_cut_information_file_to_archive` writes them; a file's connector parts are read as such; with "Keep cut editable" (the default) the recipe and the uncut mesh, as the reference writes them (next row). The objects made carry the object's settings, height ranges, layer heights and brim ears (`ObjectKept::cut_half`), not its place in the file, its cut information, its copies' assembly view or labels. Checked: the C++ slicer's `--info` and `--export-3mf` read the parts back whole and manifold (section 13, "Cut") |
| `Metadata/cut_recipe.xml` and its meshes `Metadata/cut_recipe/<sha256>.bin` | read with each object (a mesh whose name is not its SHA-256, or a recipe of no known kind, left out as the reference leaves it); written afresh under each object's place now, a recipe read written back as it was (its own element, the half's own matrices with it), the meshes once each (section 13, "Cut") |
| `edgeslicer_height_patterns.json`, `Metadata/showcase/` | not read here and kept by each object's place in the list: kept as they were while every object of the file is at its place; once one is deleted they are not kept, and that is asked about first |
| Every other entry (`Auxiliaries/`, embossing SVGs, CAD bodies, airbrush fields, texture layers, image fill…) | kept byte for byte |
| `Metadata/plate_<n>.gcode` and `.md5`, `plate_<n>.json`, `filament_sequence.json`, `_rels/model_settings.config.rels` | not written: a slice saved with the project. A saved project is unsliced, as the reference's "Save Project" writes it |
| An entry whose name reaches outside the package (`../`, a drive, a leading `/`) | not written, and said before saving |

The same project gives the same bytes: the order is fixed and every entry
carries one time, `SOURCE_DATE_EPOCH` when it is set, else 1980-01-01. A
file is written to `<path>.tmp` and moved into place once whole, so what was
at `path` is left as it was when writing fails.

### Parts

A part keeps what its file held of it in one place, `edge_3mf::PartKept`
(its `<part>` entries in order, its `subtype` as written, what its mesh's
triangles carry, and what it was read as); `Part::settings()` lists its own
settings from there. What a part was read as (its name, kind, own
filament, place in its object, mesh, place among the parts) tells the
writer what has been edited since: an entry the edit did not touch is
written as the file had it, so an unedited project is written as before.

| Edit | What the saved project holds | Read back by the reference as |
| --- | --- | --- |
| A part given a filament of its own; put back to "Default" | its `extruder`, set in place or added after its metadata; for "Default" none (`ModelVolume::extruder_id` then takes the object's) | the part's volume in that filament; "Default" prints in the object's |
| An object given a filament | the object's `extruder`; its parts' own taken away, as the C++ clears them | every part in the object's filament |
| A part renamed | its `name`, in its place among the entries | the volume's name |
| A part deleted | its `<part>`, mesh and entries gone with it; the others' places close up (they are written in order). The last part of an object left alone gives its own settings to the object, and its filament, as the C++ applies the last volume's config to the object (GUI_ObjectList.cpp `del_subobject_from_object`) | the object without it |
| A part moved, turned, scaled, mirrored | its component's `transform` (what places it) and its `matrix` (the C++ reads it only for "Reload from disk", `source.transform`) | the volume where it stands, mirrored where it is |
| A part split | each piece a part of the part's kind with its settings and its filament as its own, named `_1`, `_2`…, without its `source_*`, embossed text, `matrix` or `mesh_stat` of the whole, and without the paint, as Model.cpp `ModelVolume::split` makes each piece a copy of the part with the source and facets reset | the pieces, each with the part's settings |
| A part's kind changed | its `subtype`; made a Precise Seam part, `modifier_part` with `precise_seam_type` "precise_seam_center" (the type the C++ gives one) and its settings kept under `precise_seam_config:`; made something else, those undone | the volume of that kind |
| A part added (a file's, or a cube) | a `<part>` with its name, kind, `matrix`, filament when it has its own, and `mesh_stat` | the added volume |

What each triangle carries (paint) is written only with the mesh it was
read with, or painted on (`PartKept::painted_on`): a split part's pieces are
new meshes and carry none, as in the C++.

| Paint made here | What the saved project holds | Read back by the reference as |
| --- | --- | --- |
| A part painted with a tool (section 13, "The painting tools") | each triangle's code for that kind (`paint_supports`, `paint_seam`, `paint_color`, `paint_fuzzy_skin`), in the reference's order on the triangle, what else it carries as it was; a triangle with nothing left carries nothing; the older `paint_fuzzy` is written as `paint_fuzzy_skin` once fuzzy skin is painted again | the same paint: re-exported with `--export-3mf`, the C++ writes the same codes back (19 seam codes of a strip painted on `cylinder`, 3 colour codes of a cube, byte for byte), and slices with them (section 13) |
| A part painted on that had no file (an STL's) | a `PartKept` made for it, with the codes and the mesh they are of | as above |
| Paint given back as it was read | the file's bytes: every painted project of the reference's written with its parts repainted from their own codes is the file written without (`edge-3mf/tests/paint_codes.rs`) | |

### The settings

| Project | Written |
| --- | --- |
| On its file's own settings, nothing set over them | the file's `project_settings.config` as it was, and the presets it carries |
| On its file's own settings, with process, filament or printer settings or the bed type set here | the file's own, each setting set here taking the value the engine settles it to (`edge_engine::own_settings`), everything else as the file has it; what was set is added to the file's `different_settings_to_system`; the file's presets go with it |
| On presets chosen here | what a slice reads (`edge_engine::settled`: the presets with everything they inherit, the file's own settings under them, what was set over them, the reference's defaults for the rest), only the reference's own options and the names (`printer_settings_id`, `print_settings_id`, `filament_settings_id`), a value from the defaults' table in the text a project holds (its escapes taken out); `different_settings_to_system` lists, for the process, each filament and the printer, the settings set here (`edge_engine::project_settings`) |
| No printer chosen (an STL, an empty project) | none: the reference opens it on the presets it is on |
| Its filaments changed here (added, taken away, merged, recoloured) | on its own settings, the file's own laid out for the list (`edge_config::filaments::reslot`: every filament's list one entry for each, the added one the last one's, a deleted one's gone, `different_settings_to_system` and `inherits_group` with them, the process's feature filaments renumbered), `filament_colour`, `flush_volumes_matrix` and `flush_volumes_vector` the size of the list; on presets, the same three in the settings saved. Each plate's `filament_maps` one entry for each filament, a deleted filament's taken out. What the model renumbers (parts, objects, paint, height ranges, plates' orders, filament changes at layers) is written as it stands |

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
deleted; the cut information of an object whose connector part was deleted
or split; an entry whose name reaches outside the file. A part's own edits
lose nothing else: what a deleted part held goes with it, and a split
part's settings go with each piece. The reference never
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
- Parts (ninth pass). `crates/edge-engine/tests/save.rs`: on
  `twopart_groups.3mf` (a part with settings of its own), a two-part
  project built in the test (a painted part of two pieces with settings,
  a part with a filament of its own) and `mcbrim_test.3mf` (painted
  parts): a part's own filament and "Default", an object's filament, a
  rename, deleting one part and all but one, moving, turning, scaling and
  mirroring, splitting, each kind and Precise Seam and back, a cube and a
  file added; each saved, opened again, is the edited model (names, kinds,
  filaments, transforms to the bit, meshes, paint, settings), slices to the
  same G-code, and saves twice to the same bytes, with nothing asked.
  `crates/edge-3mf/tests/round_trip.rs`: a cut connector follows its part
  and one deleted is asked about. `src/api/save/tests.rs`: parts edited
  through `scene_edit`, saved as, are as left when the file is opened in
  another session; undo after the save marks the project unsaved. The
  same edits on the fixtures `twotone`, `halves`, `inset` and on
  `mcbrim_test.3mf`, through the engine (17 edits each, every one the same
  model and the same G-code after saving), and the CLI's G-code of the
  lettered plates, `twopart_groups`, the fixtures and a Benchy byte for
  byte what main's binary writes.
- The C++ slicer (headless, `edge-parity`) on each saved file, with a
  project of seven filaments: a part given filament 2 is printed in it
  (twotone, a twopart part: filament 2 used for that part's volume), put
  back to "Default" and an object's filament print in the object's; a
  part moved, turned or scaled has its toolpaths where it now is (5 mm
  further in X for a move); a part deleted is gone (its filament no
  longer used); a negative part, a modifier, a blocker and an enforcer
  are not printed; a Precise Seam part is read as one and, made a part
  again, printed as before; a split part's pieces print in its filament
  with its settings (the same material as before the split); a cube
  modifier adds nothing printed and a part added from a file is printed;
  on `mcbrim_test.3mf` plate 1 a part deleted, moved or made negative
  changes the filament used as it does here. A file added at a place
  partly off the bed is refused by both slicers alike.

- Plates (tenth pass). `crates/edge-engine/tests/plates.rs`: on
  `mcbrim_test.3mf` a plate renamed (with `&` and `<` in the name),
  locked, given its own bed type, print sequence, vase mode, first and
  later layers' order; a plate added and then set; a plate moved to the
  front; a plate duplicated: each saved, opened again, is as it was left
  (names, locks, settings, what stands on each plate to a ten-thousandth
  of a millimetre), saved twice to the same bytes, its metadata in the
  reference's keys and order; every setting given back to the project
  saves to the bytes of the unedited project. `src/api/save/tests.rs`: the
  same through `scene_edit`, saved as and opened in another session. The
  C++ slicer (headless) on each saved file: re-exported with
  `--export-3mf`, its `model_settings.config` has the plates as left
  (names, `locked`, `bed_type`, `print_sequence`,
  `first_layer_print_sequence`, `other_layers_print_sequence` and its
  count, `spiral_mode`, each plate's copies); `edge-parity` on the plate
  moved to the front prints what the original's third plate prints, the
  first layer's toolpaths over the same box (7.97 to 185.36 by 55.19 to
  268.23), and on the duplicated plate what the original plate prints;
  on a plate given "Cool Plate" it heats the bed to 45 °C with
  `curr_bed_type = Cool Plate`, as this engine does; on a plate given "By
  Object" it reads the plate's own ("plate print by object, set from plate
  self") and refuses the plate for objects too close for it, which this
  engine prints layer by layer with its note.

- Settings of an object, a part and a height range (eighteenth pass).
  `crates/edge-3mf/src/own.rs`: ranges written as the reference lays them
  out and read back, a setting set in place, added after the metadata and
  taken away. `crates/edge-engine/tests/item_settings.rs`: on
  `twopart_groups.3mf` an object's `wall_loops`, a part's
  `sparse_infill_density` and two ranges (given out of order) saved and
  opened again as set, `layer_config_ranges.xml` to the byte, saved twice
  the same bytes, and all of it given back saves to the bytes of the
  untouched file; a bare STL's object given a setting keeps its name;
  overlapping ranges, a range ending where it starts, under the bed or not
  a number, and settings on a helper part refused. `src/api/settings/items/tests.rs`:
  the options each kind offers; an object, a part and a range each given
  their own through `scene_edit` (the global value untouched, the rules on
  the item's settings, out-of-range and wrong-kind refusals), saved as and
  opened in another session as left, the ten steps undone by name;
  `mcbrim_test.3mf` with plates 1 and 2 sliced, an object of plate 2 given
  5 walls: plate 1 keeps its slice. In the served page (1280 × 820 and
  900 × 560 dark, 1280 × 820 light) on the `halves` fixture: "Objects",
  the object selected, wall loops typed to 5 (marked, "Frequent 1" and
  "Strength 1" on the tabs, the list's mark, the dot on "Objects", the
  unsaved dot), "Global" showing 2 unmarked; the arrow (back to 2, "Change
  Option wall_loops" to undo) and the toolbar's undo (5 again); Part 1
  selected, its pages without layer height or supports, its infill typed
  to 60%; "Height range Modifier" from the object's menu (0 to 2 mm,
  "Layers" selected), its top typed to 3, "Add height range" (3 to 5),
  the first range selected, its layer height typed to 0.1; saved as,
  closed, opened again: the marks, the ranges and each value as left.
  The C++ slicer, re-exporting that file headless (`--export-3mf`), writes
  `layer_config_ranges.xml` as this app wrote it (but the last new line)
  and the object's `wall_loops` 5 and the part's `sparse_infill_density`
  60% in `model_settings.config`; slicing it (`--slice 1`), its project
  header still says `wall_loops = 2`, `sparse_infill_density = 15%`, but
  it prints the first 3 mm in 0.1 mm layers (64 layers, 50 before),
  3.7 times the inner wall (7.35 mm of filament a layer, 1.98 before: four
  inner walls) and more sparse infill (9.57 mm a layer, 5.37 before) where
  the 60% part is. The CLI's G-code and `edge save` are the starting
  commit's byte for byte for the three plates of `mcbrim_test.3mf`,
  `twopart_groups.3mf` and the `cube`, `tower`, `bridge`, `inset`,
  `halves` fixtures (`twotone` refused alike). With the engine's regions
  (merged since): `edge-parity` on that file, this engine and the C++
  slicer print the same inner walls above the range (7.315 mm of filament
  a layer each, at 4.45, 6.45 and 8.45 mm; 1.96 for the file without the
  object's 5 walls) and nearly the same sparse infill (9.563 here, 9.581
  there; 5.37 without the part's 60%); the C++ prints 64 layers, the first
  3 mm at the range's 0.1 mm, and this engine 50, noting "layer_height: set
  on height range 0-3 mm of \"Object_1\", not followed there; the plate's
  value is used". `src/api/settings/items/tests.rs` slices
  `twopart_groups.3mf` through `scene_edit` with the three set and finds
  more inner wall, more sparse infill and that note. In the served page:
  an object's wall loops 5 (no ring: followed), its layer height and
  supports with the engine's note; Part 1's infill 60%; "Add Modifier" ›
  "Cube", moved over Part 2, given 1 wall on its own pages; "Height Range
  Modifier", the range's layer height 0.1 with its note; sliced from the
  button: 50 layers, the inner walls a layer 4.28 mm (the modifier's one
  wall over half the object), "About this print" listing the range's
  layer height as not followed.

- What is set at layers (eleventh pass). `crates/edge-3mf`: the pressure
  advance pattern's file written back to the byte, and a pause added to
  another plate written in the reference's layout and read back.
  `crates/edge-engine/tests/save.rs`: a pause and a custom line set on
  `twopart_groups.3mf` are written at their layers, saved, opened again
  and sliced to the same G-code, and stay with their plate when one
  before it is deleted. `src/api/ticks/tests.rs`: each edit through the
  calls, the project unsaved and the plate's slice let go, the G-code
  exported with them, the menu's refusals, undo and redo leaving them,
  saved and opened in another session. In the served page on a 20 mm cube
  (U1, two filaments, the plate in filament 1): "Add Pause" at layer 10,
  "Add Custom G-code" at layer 20 (typed in the dialog, then "Edit Custom
  G-code"), "Change Filament" › "Filament 2" at layer 30, each sliced
  again at once; `;PAUSE_PRINT` and `M600` seen in the G-code window at
  layer 10 and in the exported file; "Delete Pause" from the tick's
  button; saved, closed, opened again with the custom line and the
  filament change. The C++ slicer given the saved file writes the pause,
  the custom lines and the change to filament 2 at the same layers as
  this engine.

- The filament list (thirteenth pass). `crates/edge-engine/tests/filaments.rs`:
  on `mcbrim_test.3mf` filament 3 deleted (every part, object and painted
  triangle renumbered as the reference's remap does, the lists and the
  colours without it, the flushing volumes 6 × 6), saved and opened again
  the same model and the same G-code on its three plates; filament 2
  merged into 6; a filament added, saved, opened again with eight; a bare
  STL given the lists of its filaments without settings of its own.
  `src/api/filaments/tests.rs`: each edit through the session, each a
  step taken back and made again, the G-code of a one-filament model the
  same bytes after adding a filament and taking it back; deleting and
  removing on a copy of `mcbrim_test.3mf`, saved, opened in another
  session, plate 1 sliced to the same G-code there; colours and "Apply
  All" saved and read back. The C++ slicer, headless, re-exporting each
  file saved in the page (`--export-3mf`): `filament_settings_id`,
  `filament_colour`, `flush_volumes_matrix`, `flush_volumes_vector` and
  the per-filament lists as left (two filaments on the U1 and on the
  Centauri Carbon; six after deleting filament 3 of the lettered project),
  and this engine slices the C++'s re-export of the deleted project to the
  same bytes as the file saved here, paint included.
  In the served page (1280 × 820 dark, 900 × 700 light): a two-part cube
  on the U1 given a second filament with "Add one filament" (teal, the
  palette's first), part 2 given filament 2 in the object list, its colour
  picked black in the picker and its preset "Generic PETG @U1 0.4
  nozzle"; sliced, Preview showed the prime tower and 50 filament
  changes; saved as, and the same on the Centauri Carbon; on the lettered
  project the row menu of filament 3, "Merge with" listing the six others,
  "Delete" taking it (six rows, plates' filaments renumbered), saved,
  undone from the toolbar (seven again, "Delete this filament" to redo);
  "Apply All" with PETG and undone. Screenshots in the pass's scratch
  folder.

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
- `bbs_3mf.cpp` l.9463 with l.6048–6052 and l.6151–6152: a part's
  `matrix` is written as `volume->get_matrix() * source.transform`, and
  read back into `source.transform` while the component's transform
  places the volume; so every save and open multiplies the part's
  placement into its `matrix` once more (a part at Z 11 is written at 22
  the next time). Only "Reload from disk" reads it. Found by reading; here
  a moved part's `matrix` is where it stands.
- `bbs_3mf.cpp` l.9274–9276: `_add_project_embedded_presets_to_archive`
  builds a header line into `out` that is never used.
- `bbs_3mf.cpp` l.8039: the cover's `rel-5` line is written without the
  space the others have (wording only).
- `Model::plates_custom_gcodes` is keyed by a plate's index and nothing
  renumbers it when a plate is deleted (`PartPlateList::delete_plate`;
  the only writers are Plater.cpp l.13536, l.22649 and the importer):
  deleting plate 1 leaves plate 2's pauses keyed to index 1, so they are
  now plate 1's no longer and land on the plate that took index 1 next.
  Here they go with their plate.
- ToolOrdering.cpp l.2294–2297 (`assign_custom_gcodes`): every
  `ToolChange` item is passed over by the loop that finds a layer's item,
  so `tool_changes_as_color_changes` (l.2285, used l.2319) is never acted on: a
  filament change on a printer of one filament does nothing, and says
  nothing. Here the same is done and said in a note.
- GCode.cpp l.6900–6928: what a colour change (`M600`, `color_change_gcode`)
  writes is behind `#if 0`, so a colour change read from a project is
  written as an empty `;CUSTOM_GCODE` block. Here nothing is written, with
  a note; the empty block is not copied (aceRage/EdgeSlicer#979).
- PresetBundle.cpp l.6364–6388 (`update_multi_material_filament_presets`):
  when a filament is deleted the flushing matrix keeps its first n − 1
  rows and columns and the vector drops its last pair, whichever filament
  went, so deleting filament 2 of 3 leaves filament 3 with filament 2's
  volumes (to it and from it). A person sees the wrong numbers in
  "Flushing volumes" and, on a printer that purges, the wrong purge.
- Plater.cpp l.28187–28198 (`on_filaments_delete`, "update customize
  gcode"): merging a filament, only the first filament change to it (the
  element `std::remove_if` leaves at its new end) is given the target, the
  others stay in the unspecified tail it leaves, and the loop after numbers
  that first one down once more (the target's number is already the one
  after the deletion), so a "Change Filament" at a layer to a merged
  filament goes to the filament after the one meant, and others may be
  lost or doubled.
- Plater.cpp l.13876 (`get_next_color_for_filament`): the colour counter is
  a function `static`, so the colour a new filament gets depends on how
  many were added before in the whole run, in any project (followed here,
  as it is harmless).
- PartPlate.cpp l.3771–3779: `need_update_data` is set for every plate
  that has later layers' orders, whether or not one names the filament
  deleted, so each is written back (harmless).
- GCode.cpp l.2689–2697: the custom G-code set at layers is not checked
  for reserved tags (`validate_custom_gcode`), though the reference's
  G-code viewer and estimator read its own tags back from the file.

## 16. Wording and languages

The C++ application never shows a string as its source writes it: every
`_L("…")` and `L("…")` (and the labels, tooltips, units and list labels of
`PrintConfig.cpp`, the pages and groups of `Tab.cpp`) is a message id that
gettext looks up in the catalog of the language chosen, English included.
The `en` catalog words 520 of its 4,466 strings otherwise than the source
("Advance" reads "Advanced", "Lay on face" "Lay on Face", "Top Solid
Layers" "Top solid layers"). This app copied the message ids and showed
them raw, so its English was wrong in those places and no other language
was offered. Now it shows them as the C++ does.

### Where the words come from

| C++ | Where | Here |
| --- | --- | --- |
| The catalogs: `localization/i18n/<language>/Snapmaker_Orca_<language>.po`, made into `resources/i18n/<language>/EdgeSlicer.mo` by `msgfmt --check-format` at build time; 20 languages (ca cs de en es fr hu it ja ko lt nl pl pt_BR ru sv tr uk zh_CN zh_TW), about 4,500 to 5,100 entries each | CMakeLists.txt `gettext_po_to_mo` | **added**: `tools/make_translations.py` reads every `.po` (multi-line strings, escapes, `msgctxt`, plurals with the header's `Plural-Forms`; fuzzy and obsolete entries left out, as gettext leaves them) and writes `src/api/i18n/<language>.json`: JSON, an entry to a line, of the entries gettext would use and that say something other than their message id; the plural rule worked out once for n = 0 to 199 (a digit each, `100 + n % 100` past that, which the tool checks holds for every rule here), so neither side evaluates C. 20 tables, 8.0 MiB in all: `en` 77 KiB (522 entries), the others 219 KiB (ja, 2,536) to 667 KiB (ru, 4,222). Built into the binary like the settings pages; one language is read (`serde_json`, a few milliseconds) the first time it is asked for. Run the tool again at a pin bump; `--check` fails when a table is not what the catalogs give, `--report` lists what was left out as broken |
| `wxTranslations` with the catalog of the language; the message id when the catalog has none; English loads `en` (wxWidgets takes the message ids to be US English, and English here is `en_GB`) | GUI_App.cpp `load_language` l.8279–8492 | **added**: `src/api/i18n.rs` (`tr`, `trc` with a context, `trn` with a plural, `fill` for the placeholders `%1%`, `%1$d`, `%s`, `%d`, `%.2f`) and `ui/i18n.js` (`t`, `tc`, `tn`, `fmt`), the same behaviour on both sides |
| The language of what the application composes: gettext's, one for the process | | **added**: the session's. `Session::call` runs each call in it, so an error or a notice a call composes is in the page's language; a slice's thread and an upload take the language of the call that started them; anything else (a test, the CLI, the engine) is English as the C++ shows it, the `en` catalog over the message ids. The engine crates have no translations: their notes and errors are in English, as they were |
| Right-to-left languages are refused ("…which is not supported") | GUI_App.cpp l.8389 | none of the 20 is one; nothing to do |
| Slovak is given the Czech catalog | GUI_App.cpp l.8407 | **not yet**: no Slovak system language is matched to Czech |

### Choosing the language, and keeping it

| C++ | Where | Here |
| --- | --- | --- |
| At the first run (no `language` in `app_config`): the system's language (`wxLocale::GetSystemLanguage`, on Windows the user's locale), kept at once as `language`; a language with no catalog shows the bare message ids | GUI_App.cpp l.8285–8340 | **added**: the page sends the system's preferred languages (`navigator.languages`, which the window's web view takes from the system; the session falls back on `LC_ALL`, `LC_MESSAGES`, `LANG`), and the first of them is taken where the reference has a catalog for it (`de-AT` finds German, `zh-Hant`, `zh-TW`, `zh-HK` Traditional Chinese, `pt-BR` Portuguese; `pt-PT` none, as in the reference), and kept. **differs**: a system language with no catalog is English, and nothing is kept, so a catalog that comes later is found; the reference keeps `fi_FI` and shows message ids, not even the English catalog's words |
| Preferences > General > "Language": English and the languages with a catalog, sorted by their English names, shown by the names Preferences gives them ("Deutsch", "Français", "日本語", "中文(简体)", "中文(繁体)", "Português (Brasil)", "Czech", "Turkish", "Ukrainian", "Catalan" …) | Preferences.cpp l.164–242, l.1925–1966 | **added**: Preferences (the menu bar's, section 17: the application menu on a Mac [Cmd+,], File on Windows and Linux [Ctrl+P]), "General", "Language": a list of the same 20 names in the same order, the one in force chosen; OK keeps another. Until the fifteenth pass it was at the end of the project strip's menu, the app having no Preferences then |
| A change asks about an unsaved project, then "Switching the language requires application restart.\n\nDo you want to continue?" ("Language selection"), keeps the language and rebuilds the whole window | Preferences.cpp l.274–310 | **differs**: the language is kept (`language_set`) and the page loads itself again in it at once; the projects stay open in the session as they are, so nothing is asked and nothing is lost |
| `language` kept in `app_config` (`EdgeSlicer.conf`) | Preferences.cpp l.304 | **added**: `app.language` in `settings.json` (section 8), the same canonical name ("de_DE"); found again at the next start |

### What goes through the catalogs

- **The settings pages**: `settings_pages.json` keeps the message ids (one
  file for every language); the page puts an option's label, full label,
  tooltip and list labels, a group's title and a line's label and tooltip
  in the language as it loads the pages, and a page's title, an
  extruder's "Extruder %d" and a unit when it shows them. Of the pages'
  1,607 distinct texts, 1,082 are message ids of the catalog; the rest are
  numbers, material names and options newer than the catalog, shown as
  they are. The search searches the words as shown.
- **The page**: every string of `ui/*.js` that is the C++'s goes through
  `t()` as its message id (255 distinct literals; 222 message ids of
  `Snapmaker_Orca.pot`, 33 of the C++'s newer words); what the session
  hands over that the C++ names (the line types, view types, undo steps,
  bed types, host types, the upload queue's states, the option rules'
  line labels) is put through `t()` where it is shown. A tooltip with its
  keys ("Undo [Ctrl+Z]") is the message id and the keys, as the C++ builds
  it; a menu entry with its keys ("Delete  Del") likewise.
- **The static page** (`index.html`): `i18n.js` puts every text, title,
  label and placeholder that is a message id in the language before
  anything else of the page runs (128 distinct; 72 message ids, 20 newer
  words, 36 the app's own).
- **The session** (`src/`): what a call composes in the C++'s words goes
  through `tr()` (23 distinct literals: 12 message ids, 11 newer words):
  the bed types notice, "An object is laid over the boundary of plate…",
  "Enter Custom G-code used on current layer:", "Please input layer value
  (>= 2).", the settings' "Value %s is out of range. The valid range is
  from %d to %d.", "Value is out of range.", "Invalid numeric.", "%s can't
  be a percentage" (each after the option's label, in the language), and
  the printers' connections' messages (PrintHost.cpp, OctoPrint.cpp,
  MoonRaker.cpp, ElegooLink.cpp: "Connection timed out…", "Could not
  connect to OctoPrint", "Error uploading to print host", "Error code:
  %d"…), composed as the C++ composes them ("%s: %s\n\n%s").

The search fields of the settings pages are the one place where English
and the other languages are worded apart, by the owner's choice: English
reads this app's own "Search for setting", every other language the
catalog's words for the reference's "Search in preset" (or "Search for
setting" where its catalog has none). The session picks them
(`i18n::search_words`, through `tr()`) and hands them to the page with the
language (`search`), and `i18n.js` puts them on every field marked
`data-search-words`.

`src/api/i18n/wording.txt` lists what is shown and is not a message id:

- **newer** (62): the C++'s own `_L` words that are newer than its
  catalog (the project strip, the printer's storage page, the filament
  list's head, the object manipulation panel, the connection messages):
  they go through the layer like any message id, and read in English in
  every language until a pin bump brings catalogs that have them.
- **own** (139): this app's own words, with no counterpart in the C++ (41
  on the static page, 98 in the scripts): the browser's path prompts,
  "About this print", the notes on what the engine follows, the printer's
  page in Device, Printer Selection's column tips, the connection notes.
  They are shown in English in every language.

The session's own sentences (the errors of a call given what it should
not be, "there is no project 3", the notes of a slice, the connection
notes "The printer asks for a login…") are English too and are not
listed one by one. Words the C++ shows without translating them
("ElegooLink not detected", "Could not parse server response", "Instance
%d", a `_devL` string) are left as they are.

The eighteenth pass's words (settings of an object, a part and a height
range): the switch's "Global" / "Objects", "Frequent" (a page title, put
in the language as the others are), "Layers", "Range", "mm", "Height
range Modifier", "Edit Process Settings", "Add height range", "Remove
height range", "Invalid numeric.", "Object", "Part", "Modifier", "Click
to reset current value and attach to the global value." and "Click the
icon to reset all settings of the object" go through `t()` as message
ids, and the height's "Invalid numeric." through `tr()`; an item's pages
are the global pages' data, already in the language. A range's row is
built of pieces as the reference builds it (ObjectDataViewModel.cpp
l.161: `_(L("Range")) + " %.2f-%.2f " + "(" + _("mm") + ")"`), so it
reads as the C++'s does in every language (German: "Reichweite
0.00-3.00 (mm)", the catalog's word). The undo steps' names are message
ids put in the language where the page shows them; "Change Option %s"
stays English, as the C++ composes it with `boost::format`. This pass's
own words (the hints, the "not read by the engine set on" of an option the engine gives no note of, the session's refusals
of heights and keys) are listed under `[own]`. The engine's refusals in
`edge-engine` (`item_settings.rs`) are English, as the engine's are, and
the session checks first in its own listed words but for one: a project
with the reference's height colour patterns.

`every_word_shown_is_a_message_id_or_listed` (`src/api/i18n/tests.rs`)
reads every `t()`, `tc()`, `tn()` of the scripts, every `tr()` of the
session and every word of the static page, and fails on one that is
neither a message id of the template nor listed; and on a listed entry
that is a message id now, or shown nowhere any more. A form typed as the
English reads (the `en` catalog's "Layer height (mm)" for "Layer Height
(mm)") fails it, since other languages would not find it: the audit put
back four ("Layer Height (mm)", "Line Width (mm)" in the 2D and 3D
legends, "Fan Speed (%)", "Total Estimation") and the search's placeholder
("Search in preset").

### English, as the C++ shows it

With the `en` catalog, 168 strings this app shows read otherwise than
they did, as the C++ reads them; among the labels and menus: "Advance" →
"Advanced", "Lay on face" → "Lay on Face", "Add part" → "Add Part", "Add
negative part" → "Add Negative Part", "Add modifier" → "Add Modifier",
"Add support blocker" / "enforcer" → "Add Support Blocker" / "Enforcer",
"Along X axis" → "Along X Axis" (and Y, Z), "Change type" → "Change Type",
"To objects" / "To parts" → "To Objects" / "To Parts", "Object
Operations" → "Object operations" (and Volume, Group), "Jump to Layer" →
"Jump to layer", "Slice ok." → "Slice complete", "Select all" → "Select
All", "arrange current plate" → "Arrange current plate", "Initial layer
height" → "First layer height" (and the other "Initial layer" options),
"Support filament" → "Filament for Supports", "Detect thin wall" →
"Detect thin walls",
"De-retraction Speed" → "Deretraction speed", "Bed exclude area" →
"Excluded bed area", "%s can't be a percentage" → "%s can’t be a
percentage"; the rest are the options' tooltips and labels of the
settings pages.

The settings' messages also changed in English on their own account:
"Invalid numeric. It is a whole number." is "Invalid numeric." as
Field.cpp has it; a value out of a one-sided range reads "Value is out of
range. (≥ 0)" where it read "Value -1 is out of range. It cannot be less
than 0." (the C++'s words, the limit after them); "Could not connect to
Moonraker: …" is followed by the reference's note "Note: Moonraker version
at least 1.1.0 is required.".

### Fonts and layout

The page uses the system's fonts (`--font-ui`: `system-ui`,
`-apple-system`, "Segoe UI"), and sets `lang` on the page to the language
chosen, so that the system picks the right CJK face: on a Mac PingFang SC
or TC for Chinese and Hiragino Sans for Japanese; on Windows "Segoe UI"
should fall back on Microsoft YaHei, JhengHei and Yu Gothic (not seen).
Chinese and Japanese were checked to render in Chromium (`--serve`) on
the Mac; the window (WebKit) was not looked at in them.
German, French, Japanese and Simplified Chinese were checked at 1280 × 820
and 900 × 560, dark and light, in the sidebar, every process page, the
filament's and the printer's pages, the object menu, the strip's menu,
"Plate Settings", "Physical Printer" and "Send G-code to printer host":
nothing runs out of its box or past the window. Two things were cut
badly and are fixed: a list's closed face in an option row (now up to 58 %
of the row, 210 px, the label wrapping beside it: "Obere und untere
Oberflächen", "Surfaces supérieure et inférieure"), and "Plate
Settings"' lists ("Identique à la séquence d'impression globale"; the
dialog 640 px, its controls 310 px). What is still cut with an ellipsis
is so by design and whole in its tooltip (a preset's name, a long list
value in the narrowest sidebar). A tab row that does not fit scrolls, as
it did.

### Not yet

- The reference's web pages (Printer Selection, the guide) translate
  themselves from `resources/web/data/text.js`, tables of their own for
  13 languages, not the catalogs. Printer Selection here is in English in
  every language but for its buttons; "At least one printer must be
  selected." is that table's `t37`.
- `tc()` (a message id with a context) is not called yet: no string the
  page shows has a context in the C++; it is there and tested.
- The native file dialogs' titles and filters (`main.rs` `pick`: "Save
  file as:", "3MF files") are in English: `pick` is Tauri's command, not
  the session's, and is not given the language yet. They are message
  ids and can follow.
- The developer's own diagnostics (the page's console, WebGL errors) and
  the session's own sentences stay English.
- Slovak to Czech (above).

### Errors found in the catalogs

- `nl/Snapmaker_Orca_nl.po` l.2121, 2127, 2136, 2311, 2320, 5763, 5773,
  8345 and `sv/Snapmaker_Orca_sv.po` l.2091, 2097, 2106, 2278, 2285, 5704,
  5710, 8206: plural entries whose forms are the singular and the plural
  run together with "@" between them ("%1$d fouten hersteld@%1$d fouten
  hersteld"), so each form holds the placeholder twice or ends in "@";
  the C++ would show "@" in the message, or format it with one value for
  two placeholders. Left out here (the message id is shown).
- `de/Snapmaker_Orca_de.po` l.2449: "Plate" is translated "Aktuelle Platte
  drucken" ("Print current plate"), the translation of another string;
  every "Plate 2" of the German C++ reads "Aktuelle Platte drucken 2".
  Left out here (`WRONG` in the tool). Its "Print" is "aktuelle Platte
  drucken" too, which reads oddly on a menu entry but is not plainly
  another string's; kept.
- `de/Snapmaker_Orca_de.po` l.1705: "Language" is "Spache" (for
  "Sprache"), so the German C++ titles its Preferences' list so, and so
  does the strip's menu here. A misspelling, not another string's
  translation: kept, for the catalog to be mended where it is.
- `Snapmaker_Orca.pot` l.16281: the `msgstr` of "EdgeSlicer integrates
  some features from the 3D printing open-source community…" is a string
  that is never closed (`msgstr "`), which gettext's own tools refuse.
  Only the template; the catalogs do not have the entry.
- No entry of any catalog has placeholders that differ from its message
  id's but for the plurals above (`--report`).


## 17. The menu bar

Read from `MainFrame.cpp` (`init_menubar_as_editor` l.3385–4345,
`generate_help_menu` l.3104, `add_common_view_menu_items` l.3233, the
`can_*` rules l.2142–2355, `open_recent_project` l.5169),
`KBShortcutsDialog.cpp` and `GUI_Factories.cpp`. `init_menubar_as_gcodeviewer`
(the G-code viewer's own menus) is not this app's.

### How it is made

The menus are described once, in the session (`src/api/menu.rs`): each
entry's id, its message id as the C++ source writes it (with its `&`
marks; shown without them, "&&" as "&", as wxWidgets shows them, with
"..." where the reference adds `dots`), its key, when it can be used (the
reference's `can_*`, from the session's state and the page's), whether it
is ticked, its help text (the tooltip), and for an entry that is not built
what it waits for. `menu` answers it in the session's language. In the
app's window `main.rs` makes the system's menu bar of that answer
(`src/menubar.rs`, Tauri's menu API: at the top of the screen on macOS, in
the window elsewhere), made again when its make-up changes (another
language, the recent or the open projects) and otherwise only set where an
entry changed; an entry chosen sends its id to the page (the event
`menu`). In a browser (`--serve`) the page draws the same answer as a bar
of its own (`ui/menu.js`, `ui/menu.css`). Either way the page does what
the id stands for (`Menu.run`). The page asks for the menu again whenever
what decides it may have changed, at most every half second.

The keys: section 6. Cut, Copy and Paste in the window's Edit menu are the
system's own entries, so a text field keeps them (the page reads the
clipboard's events they send for objects); on macOS the application
menu's Services, Hide, Hide Others and Show All, and Minimize and Zoom,
are the system's too.

The menus follow the reference's per system: macOS has the application
menu (About, Preferences [Cmd+,], Themes, Costs, Quit) and the Window
menu; Windows and Linux have Preferences [Ctrl+P], Themes and Costs (the
reference's title-bar menu, beside File) in File before Quit [Ctrl+Q], and
"About" in Help. "Delete selected" [Del] is Windows' and Linux's only, as
in the reference (`#if 0` on macOS).

### Every entry

"wired" is an entry that does what the reference's does, with what the
app had; "built" is one built in this pass; "off" is shown, never usable,
its tooltip "not yet: …" saying what it waits for.

| Menu | Entry (message id) | Key | Here |
| --- | --- | --- | --- |
| EdgeSlicerRS (macOS) | "&About %s" | | built: "About EdgeSlicerRS", a box with the app's name, what it follows and its licence |
| | "Preferences" | Cmd+, | built: "Preferences", General, "Language" (section 16) |
| | "Themes..." | | off: the app follows the system's dark or light |
| | "Costs..." | | off: no prices or rates of your own are kept |
| | "Quit" | Cmd+Q | wired: asks about unsaved projects as closing the window does (section 5), then closes it; off in a browser |
| File | "New Window" (macOS) | | off: the app has one window |
| | "New Project" | Ctrl+N | wired (`project_new`); off while the project slices |
| | "Open Project..." | Ctrl+O | wired (`project_open`); off while slicing |
| | "Recent projects" ▸ | | built (section 2, "Files"); off with none, or while slicing |
| | "Start page" | | off: there is no Home tab |
| | "Save Project" | Ctrl+S | wired (`project_save`); on when the project has unsaved changes and does not slice (`can_save`) |
| | "Save Project as..." | Ctrl+Shift+S | wired |
| | Import ▸ "Import 3MF/STL/STEP/SVG/OBJ/AMF/GLB..." | Ctrl+I | wired to "Add" (section 13), Prepare, not while slicing; reads 3MF, STL and OBJ (built: OBJ). STEP, SVG, AMF, GLB: not yet, and not in the dialog's filter |
| | Import ▸ "Import Zip Archive..." | | off: models are not read from a ZIP archive |
| | Import ▸ "Import Configs..." | | wired (sixteenth pass): the page's `presetImport()` picks a file (`.json`, `.zip`, `.orca_printer`, `.orca_filament`, the reference's filter and "Select profile to load:"), calls `preset_import`, asks the reference's questions it answers with ("Untrusted settings in file": "Remove (recommended)" / "Keep"; "Overwrite config": "Yes to All" / "No to All") and shows "Import result" (section 8, "User presets"). **differs**: no "Yes" / "No" for each preset, the session asking once for all |
| | Import ▸ "Import Config from G-code..." | | off: settings are not read from a G-code file |
| | Export ▸ "Export all objects as one STL..." | | built (section 2, "Files"); on with objects |
| | Export ▸ "Export all objects as STLs..." | | built; on with objects |
| | Export ▸ "Export as STEP..." | | off: STEP needs a CAD kernel |
| | Export ▸ "Export Generic 3MF..." | | off: the project file is written in the reference's own layout only. The reference's "generic" 3MF is its project file with every mesh in the one model file (`export_3mf` without `SplitModel`), which `edge-3mf::write` does not write; that is its own change to the writer |
| | Export ▸ "Export Bambu 3MF...", "Export && Open in Bambu Studio..." | | off: no Bambu Studio export |
| | Export ▸ "Export plate sliced file..." | Ctrl+G | off: the sliced file (.gcode.3mf) is not written |
| | Export ▸ "Export all plate sliced file..." | | off: the same |
| | Export ▸ "Send plate to Bambu Connect", "Send all plates to Bambu Connect" | | not shown: the reference takes them out for a printer not Bambu's, and sending to Bambu is not this app's |
| | Export ▸ "Export G-code..." | | wired: the print button's "Export G-code file" (section 2); on when the plate shown is sliced and its G-code written |
| | Export ▸ "Export Preset Bundle..." | | wired (sixteenth pass): "Export Preset Bundle", the reference's five kinds ("Printer config bundle(.orca_printer)" and its note, "Filament bundle(.orca_filament)", "Printer presets(.zip)", "Filament presets(.zip)", "Process presets(.zip)"), the presets of the kind each ticked, "Export", then a folder; a zip already there is asked about in the reference's words; "Export successful" with the files written (`presetExport()`, `preset_export`) |
| | "Preferences", "Themes...", "Costs..." (Windows, Linux) | Ctrl+P | as in the application menu |
| | "Quit" (Windows, Linux) | Ctrl+Q | as above |
| Edit | "Undo", "Redo" | Ctrl+Z, Ctrl+Y | wired (section 13), with the step's name: "Undo: Delete", "Redo: Import Object" (the step's message id, through the layer); Prepare. Cmd+Shift+Z makes again as well (`scene-edit.js`) |
| | "Cut", "Copy", "Paste" | Ctrl+X, C, V | wired: copies of objects, as Ctrl+C / Ctrl+V and the object's menu did; Cut is Copy then delete. Pasted into the project they were copied from (section 13: on the plate shown). **not yet**: copying between projects (it needs the model to take another model's objects with their own settings) |
| | "Delete selected" (Windows, Linux) | Del | wired |
| | "Delete all" | Ctrl+D | built: every object of the project, one step to undo, as the reference's `delete_all_objects_from_model` |
| | "Clone selected" | Ctrl+K | wired. **differs**: the key is shown on every system; the reference shows it on macOS only (the binding is everywhere) |
| | "Duplicate Current Plate" | | wired (section 13, "Plates") |
| | "Select all" | Ctrl+A | wired: the objects on the plate shown |
| | "Deselect all" | Esc | wired: a tool that is open is closed first, as Esc did |
| View | "Default View", "Top", "Bottom", "Front", "Rear", "Left", "Right" | Ctrl+0..6 | wired (section 9); Prepare and Preview. Ctrl+7 (the plate from above) stays the scene's own, being in no menu |
| | "Fit in all view" | Z | wired |
| | "Use Perspective View", "Use Orthogonal View" | | wired: the scene's camera (it was in the scene's View menu, where it stays) |
| | "Auto Perspective" | | wired |
| | "Show &G-code Window" | C | wired; Preview |
| | "Show 3D Navigator" | | wired |
| | "Reset Window Layout" | | off: the sidebar's layout is not reset |
| | "Show 3Dconnexion Settings" | | off: 3D mice are not read |
| | "Show &Labels" | Ctrl+E | wired |
| | "Show &Overhang", "X-Ray", "Realistic View", "Show Center of Mass" | | off: each waits for the scene to draw it |
| | "Compare Slices...", "Compare with Previous Slice" | | off: slices are not compared |
| | "Mobile Hub..." (macOS) | | off: there is no Mobile Hub |
| Calibration | "Temperature", "Flow rate" ▸ ("Pass 1", "Pass 2", "YOLO (Recommended)", "YOLO (perfectionist version)"), "Pressure advance", "Retraction test", "Stringing (temp × retraction)", "Max flowrate", "Cornering", "Input Shaping" ▸ ("Input Shaping Frequency", "Input Shaping Damping/zeta factor"), "VFA", "Ironing flow", "Tutorial" | | off: calibration models are not built (the reference has the menu in the macOS bar and a title-bar menu elsewhere) |
| Account (macOS) | sign-in for Snapmaker, Bambu Lab, Flashforge | | not shown: the cloud is not this task |
| Window (macOS) | "Minimize" [Cmd+M], "Zoom" | | the system's own, in the window; off in a browser |
| | "Next Project", "Previous Project" | Cmd+Shift+] / [ | wired (`project_cycle`); on with two projects |
| | the open projects, the one shown ticked | | built: choosing one shows it |
| Help | "Keyboard Shortcuts" | ? | built (its "Gizmo cut" [C] row built in the nineteenth pass): the reference's lists (KBShortcutsDialog.cpp: "Global shortcuts", "Plater", "Gizmo", "Objects List", "Preview"; its "Sculpt Gizmo" list is left out, there being no sculpting), each key as this system shows it, the menu bar's keys taken from its table; what this app does not do is dimmed |
| | "Setup Wizard" | | wired: Printer Selection (section 8; the app's guide is that page alone) |
| | "Show Configuration Folder" | | off: folders are not shown in the system's file manager |
| | "Migrate Filament Presets", "Import Profile" | | off: "Migrate Filament Presets" brings back system filament presets a profile update took away, and "Import Profile" imports a vendor's system profile (`PresetUpdater::import_system_profile`); neither is about a person's presets, which are built (sixteenth pass) |
| | "Export Logs" | | off: the app keeps no logs |
| | "Show Tip of the Day", "Check for Update", "Open Network Test", "Check Windows Firewall..." (Windows), "Bambu Lab Printer Help..." | | off, each with its reason |
| | "&About %s" (Windows, Linux) | | built, as above |
| Publish ("3D Models") | | | not shown: commented out in the reference |

### The user presets' calls

The user presets are built (sixteenth pass, section 8). "Import Configs..." and
"Export Preset Bundle..." are on only when the session has one of these
calls, looked up by name when the menu is first asked for (each is sent
`{ "probe": true }`, and one the session does not have answers "there is
no call named …"; a call that is there should answer a probe without
acting): for import `preset_import`, `presets_import`, `import_presets`;
for export `preset_export`, `presets_export`, `export_presets`,
`preset_bundle_export`. The entry then calls the page's own `presetImport()`
/ `presetExport()` where that branch defines one, else the call with
`{ path }` (a configs file picked) or `{ dir }` (a folder picked). The
session has `preset_import` and `preset_export`, each of which answers a
probe with `{ "probe": true }` and does nothing; the page defines
`presetImport()` and `presetExport()`. "Import Profile" (Help) stays off:
in the reference it imports a vendor's system profile
(`PresetUpdater::import_system_profile`), not a person's presets.
"Compare presets" has no menu entry in the reference (ParamsPanel's
button only); here it is in the preset lists' cog (section 2).

### How it was checked

In a browser (`--serve`, dark and light, 1280 × 820 and 900 × 560, English
and German): every menu opened by the pointer and the keys; File's New,
Open, Save as, the next and previous project; a project opened from
"Recent projects" after the server was started again; a recent file that
was gone said so and was taken off; an OBJ imported (96 triangles, 24 × 24
× 20 mm, as the C++ reads it); the objects exported as one STL (written,
asked about when there, a missing folder said) and as STLs; G-code
exported from File > Export after a slice; Edit's Copy, then Paste by
Cmd+V, Undo by Cmd+Z, the entries reading "Undo: Paste From Clipboard",
"Redo: Import Object"; the views by Cmd+1 and Cmd+3; "Keyboard
Shortcuts" by "?"; Preferences to Deutsch and back; a tooltip of an entry
that is off; letters, "?", Delete and Escape typed in a text field reached
it and nothing else. The window: built (`tools/make_app.sh`), the menu's
description and the window bar's make-up unit-tested; the system's bar
itself was not clicked through.

### Errors found in the C++

- MainFrame.cpp l.5187–5188, `open_recent_project`: the box for a recent
  project that is gone has one button (`wxOK | wxYES_DEFAULT`) but the
  entry is removed only when it answers `wxID_YES`, which it never does.
  The person is told "The project is no longer available." each time they
  choose it, and it stays in the list for good.
- AppConfig.cpp l.1642–1655 and l.1628: the recent projects are kept
  under "01", "02", … in a `std::map` of strings and read back in that
  map's order, so with `max_recent_count` over 99 (it takes up to 10000,
  MainFrame.cpp l.4349) entry "100" is read between "10" and "11": after a
  restart the list is out of order.
- Plater.cpp l.26100–26105 and l.26150 ("Export all objects as STLs"):
  the file is named by the object's name as it is, though the same file
  sanitises it for "Export part as STL" (l.26160); an object named
  "bracket / v2" (or with ":" on Windows) is written nowhere, and
  `store_stl` (Format/STL.cpp l.42–50, "FIXME returning false even if
  write failed") returns true whatever happened, so nothing is said: the
  person finds files missing. `stl_write_binary` (admesh stl_io.cpp l.130)
  also writes in place, so a failed write leaves a part of a file.
- objparser.cpp l.238–241: a face naming vertex 0 (OBJ counts from 1)
  becomes index -1, which OBJ.cpp l.78 and l.121 take as the end of a
  face; such a file is read with its faces cut in the wrong places rather
  than refused as "The file contains invalid vertex index.".
- Snapmaker_Orca.cpp l.8236 (the command line's `--export-stl` /
  `--export-stls`): `find_last_of(".stl")` looks for any one of the
  characters '.', 's', 't', 'l', not for the extension, so an object whose
  name ends in s, t or l ("MC Arms") is written as `obj_1_MC Arms` with no
  extension, which the reference's own `--info` then refuses ("Unknown
  file format").
- MainFrame.cpp l.3708–3713 (macOS): "Clone selected" shows Ctrl+K but
  hands the 3D view the key M; on Windows and Linux (l.3612) the entry
  shows no key, while the shortcuts window lists Ctrl+K for it.
