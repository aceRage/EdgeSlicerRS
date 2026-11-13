//! The menu bar, described once: every menu and entry of the reference's
//! (MainFrame.cpp `init_menubar_as_editor` and `generate_help_menu`, the
//! macOS application menu and Window menu), in its order and its words,
//! with each entry's key, when it can be used and whether it is ticked.
//!
//! Two things are made from this description. In the app's window it is
//! the system's own menu bar (`menubar.rs`, beside `main.rs`), which only
//! sends the id of the entry chosen to the page. Served to a browser
//! (`--serve`), which has no menu bar of its own, the page draws the same
//! menus as a bar of its own (`ui/menu.js`). Either way the page carries
//! out what an id stands for, and asks [`Session::call`] for anything it
//! needs; the keys are one table, from here, bound once (`menu.js`).
//!
//! A label is the reference's message id, as its source writes it (with
//! its `&` marks), passed through the layer; `&` is taken out for showing
//! as wxWidgets takes it out ("&&" is one "&"), and "..." is put after it
//! where the reference adds `dots`. The key is kept apart from it.
//!
//! An entry of the reference whose feature is not built is shown, off,
//! with what it waits for ([`Item::waits`]): the menu bar's exception to
//! the rule that nothing is shown that does nothing (UI_PARITY.md).
//!
//! Also here: the recent projects (the reference's `recent_projects`,
//! [`RECENT_MOST`] of them, the latest first), and "Export all objects as
//! one STL..." / "Export all objects as STLs...".

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Session, i18n::tr, lock, shown, slot};

/// The calls answered here.
pub(super) const JSON: [&str; 5] = [
    "menu",
    "recent_remove",
    "export_stl",
    "export_stls",
    "shortcuts",
];

/// How many recent projects are kept: the reference's `max_recent_count`
/// as it starts (AppConfig.cpp l.533, MainFrame.cpp `m_recent_projects(18)`).
pub const RECENT_MOST: usize = 18;

/// The name the app goes by, in "&About %s".
const APP_NAME: &str = "EdgeSlicerRS";

/// A key: with the command key (Ctrl, or Cmd on a Mac, as wxWidgets reads
/// "Ctrl+" on one), Shift, and the key itself as the browser names it
/// ("N", "0", "Delete", "Escape", "?").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub command: bool,
    pub shift: bool,
    pub key: &'static str,
}

const fn command(key: &'static str) -> Key {
    Key {
        command: true,
        shift: false,
        key,
    }
}

const fn command_shift(key: &'static str) -> Key {
    Key {
        command: true,
        shift: true,
        key,
    }
}

const fn plain(key: &'static str) -> Key {
    Key {
        command: false,
        shift: false,
        key,
    }
}

/// On which systems an entry is in the reference's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    All,
    Mac,
    Other,
}

/// When an entry can be used, from the session's state and the page's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    /// The project shown is not slicing (`can_start_new_project`,
    /// `can_open_project`, `can_add_models`).
    Idle,
    /// It has unsaved changes and is not slicing (`can_save`).
    Dirty,
    /// It has objects (`can_export_model`).
    Objects,
    /// Prepare, a project with objects (`can_select`, `can_delete_all`).
    PrepareObjects,
    /// Prepare, something selected (`can_delete`, `can_clone`, ...).
    Selected,
    /// Prepare, something copied from this project to paste.
    Pasteable,
    /// Prepare, something to take back or make again.
    Undo,
    Redo,
    /// Prepare or Preview (`can_change_view`).
    Plater,
    /// Prepare, Import: not while slicing.
    PrepareIdle,
    /// Preview.
    Preview,
    /// The plate shown is sliced and its G-code was written
    /// (`can_export_gcode`).
    Gcode,
    /// Every plate that holds something is sliced and its G-code written
    /// (`can_export_all_gcode`).
    AllGcode,
    /// There is more than one project open.
    Projects,
    /// In the app's window, not a browser (Quit, Minimize, Zoom).
    Window,
    /// The session has one of these calls (the user presets' import and
    /// export, built on another branch: looked up by name).
    Call(&'static [&'static str]),
}

/// What a ticked entry shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Perspective,
    Orthogonal,
    AutoPerspective,
    GcodeWindow,
    Navigator,
    Labels,
}

/// One entry.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub id: &'static str,
    /// The reference's message id.
    pub label: &'static str,
    /// Whether "..." follows it (the reference's `dots`).
    pub dots: bool,
    pub key: Option<Key>,
    /// The reference's help text for it, a message id (its tooltip here).
    pub tip: &'static str,
    pub when: When,
    pub check: Option<Check>,
    /// Ticked as one of a group (the camera's two views).
    pub radio: bool,
    /// Not built: what it waits for, this app's own words.
    pub waits: Option<&'static str>,
    pub os: Os,
}

const fn item(id: &'static str, label: &'static str, tip: &'static str, when: When) -> Item {
    Item {
        id,
        label,
        dots: false,
        key: None,
        tip,
        when,
        check: None,
        radio: false,
        waits: None,
        os: Os::All,
    }
}

impl Item {
    const fn dots(mut self) -> Self {
        self.dots = true;
        self
    }
    const fn key(mut self, key: Key) -> Self {
        self.key = Some(key);
        self
    }
    const fn check(mut self, check: Check) -> Self {
        self.check = Some(check);
        self
    }
    const fn radio(mut self, check: Check) -> Self {
        self.check = Some(check);
        self.radio = true;
        self
    }
    const fn waits(mut self, why: &'static str) -> Self {
        self.waits = Some(why);
        self
    }
    const fn on(mut self, os: Os) -> Self {
        self.os = os;
        self
    }
}

/// A menu or a line of one.
#[derive(Clone, Copy, Debug)]
pub enum Node {
    Item(Item),
    Menu {
        id: &'static str,
        label: &'static str,
        os: Os,
        items: &'static [Node],
    },
    Line,
    /// The recent projects, as entries of their own ("file.recent").
    Recent,
    /// The open projects (macOS's Window menu).
    Projects,
}

use Node::{Item as I, Line};

// What some entries wait for: this app's own words.
const NO_HUB: &str = "not yet: there is no Home tab";
const ONE_WINDOW: &str = "not yet: the app has one window";
const NO_ZIP: &str = "not yet: models are not read from a ZIP archive";
const NO_GCODE_CONFIG: &str = "not yet: settings are not read from a G-code file";
const NO_STEP: &str = "not yet: STEP needs a CAD kernel, which the engine has not";
const NO_BAMBU: &str = "not yet: no Bambu Studio export";
const NO_PRESETS: &str = "not yet: a vendor's system profile is not imported";
const NO_3DCONNEXION: &str = "not yet: 3D mice are not read";
const NO_OVERHANG: &str = "not yet: overhangs are not marked in the scene";
const NO_XRAY: &str = "not yet: the scene has no X-ray view";
const NO_REALISTIC: &str = "not yet: the scene has one way of shading";
const NO_MASS: &str = "not yet: centres of mass are not worked out";
const NO_COMPARE: &str = "not yet: slices are not compared";
const NO_MOBILE_HUB: &str = "not yet: there is no Mobile Hub";
const NO_THEMES: &str = "not yet: the app follows the system's dark or light";
const NO_COSTS: &str = "not yet: no prices or rates of your own are kept";
const NO_LAYOUT: &str = "not yet: the sidebar's layout is not reset";
const NO_CALIBRATION: &str = "not yet: calibration models are not built";
const NO_FOLDER: &str = "not yet: folders are not shown in the system's file manager";
const NO_LOGS: &str = "not yet: the app keeps no logs";
const NO_TIPS: &str = "not yet: there are no tips";
const NO_UPDATES: &str = "not yet: there are no releases to check for";
const NO_NETWORK_TEST: &str = "not yet: no network test";
const NO_FIREWALL: &str = "not yet: the firewall is not checked";
const NO_BAMBU_HELP: &str = "not yet: Bambu Lab printers are not sent to";
const NO_MIGRATE: &str =
    "not yet: filament presets a profile update took away are not brought back";

/// What the presets' import and export may be called (the user presets
/// are built on another branch): the first of these the session has is
/// the one the entry uses.
pub const PRESET_IMPORT: &[&str] = &["preset_import", "presets_import", "import_presets"];
pub const PRESET_EXPORT: &[&str] = &[
    "preset_export",
    "presets_export",
    "export_presets",
    "preset_bundle_export",
];

static IMPORT: &[Node] = &[
    I(item(
        "import.model",
        "Import 3MF/STL/STEP/SVG/OBJ/AMF/GLB",
        "Load a model",
        When::PrepareIdle,
    )
    .dots()
    .key(command("I"))),
    I(item(
        "import.zip",
        "Import Zip Archive",
        "Load models contained within a zip archive",
        When::Always,
    )
    .dots()
    .waits(NO_ZIP)),
    I(item(
        "import.configs",
        "Import Configs",
        "Load configs",
        When::Call(PRESET_IMPORT),
    )
    .dots()),
    I(item(
        "import.gcode_config",
        "Import Config from G-code",
        "Load print/filament/printer settings embedded in a sliced G-code file",
        When::Always,
    )
    .dots()
    .waits(NO_GCODE_CONFIG)),
];

static EXPORT: &[Node] = &[
    I(item(
        "export.stl",
        "Export all objects as one STL",
        "Export all objects as one STL",
        When::Objects,
    )
    .dots()),
    I(item(
        "export.stls",
        "Export all objects as STLs",
        "Export all objects as STLs",
        When::Objects,
    )
    .dots()),
    I(item(
        "export.step",
        "Export as STEP",
        "Export the selected objects, or all objects on the current plate, as a STEP file of solids",
        When::Objects,
    )
    .dots()
    .waits(NO_STEP)),
    I(item(
        "export.generic_3mf",
        "Export Generic 3MF",
        "Export 3mf file without using some 3mf-extensions",
        When::Objects,
    )
    .dots()),
    I(item(
        "export.bambu_3mf",
        "Export Bambu 3MF",
        "Export a project Bambu Studio opens with its settings (settings Bambu Studio does not have are left out)",
        When::Objects,
    )
    .dots()
    .waits(NO_BAMBU)),
    I(item(
        "export.bambu_open",
        "Export && Open in Bambu Studio",
        "Export a project Bambu Studio opens with its settings, then launch Bambu Studio with it",
        When::Objects,
    )
    .dots()
    .waits(NO_BAMBU)),
    I(item(
        "export.plate_sliced",
        "Export plate sliced file",
        "Export current sliced file",
        When::Gcode,
    )
    .dots()
    .key(command("G"))),
    I(item(
        "export.all_sliced",
        "Export all plate sliced file",
        "Export all plate sliced file",
        When::AllGcode,
    )
    .dots()),
    I(item(
        "export.gcode",
        "Export G-code",
        "Export current plate as G-code",
        When::Gcode,
    )
    .dots()),
    I(item(
        "export.preset_bundle",
        "Export Preset Bundle",
        "Export current configuration to files",
        When::Call(PRESET_EXPORT),
    )
    .dots()),
];

static FILE: &[Node] = &[
    I(item(
        "file.new_window",
        "New Window",
        "Start a new window",
        When::Always,
    )
    .waits(ONE_WINDOW)
    .on(Os::Mac)),
    I(item("file.new", "New Project", "Start a new project", When::Idle).key(command("N"))),
    I(item(
        "file.open",
        "Open Project",
        "Open a project file",
        When::Idle,
    )
    .dots()
    .key(command("O"))),
    Node::Menu {
        id: "file.recent",
        label: "Recent projects",
        os: Os::All,
        items: &[Node::Recent],
    },
    I(item(
        "file.start_page",
        "Start page",
        "Show the start page with recent projects on the Home tab",
        When::Always,
    )
    .waits(NO_HUB)),
    I(item(
        "file.save",
        "Save Project",
        "Save current project to file",
        When::Dirty,
    )
    .key(command("S"))),
    I(item(
        "file.save_as",
        "Save Project as",
        "Save current project as",
        When::Always,
    )
    .dots()
    .key(command_shift("S"))),
    Line,
    Node::Menu {
        id: "file.import",
        label: "Import",
        os: Os::All,
        items: IMPORT,
    },
    Node::Menu {
        id: "file.export",
        label: "Export",
        os: Os::All,
        items: EXPORT,
    },
    Line,
    // On Windows and Linux the reference keeps these in the title bar's
    // own menu, beside File; a menu bar has them here, before Quit.
    I(item("app.preferences", "Preferences", "", When::Always)
        .key(command("P"))
        .on(Os::Other)),
    I(item(
        "app.themes",
        "Themes",
        "Colours, fonts, corners and the title bar",
        When::Always,
    )
    .dots()
    .waits(NO_THEMES)
    .on(Os::Other)),
    I(item(
        "app.costs",
        "Costs",
        "Your own filament prices and machine rates, over the presets' values",
        When::Always,
    )
    .dots()
    .waits(NO_COSTS)
    .on(Os::Other)),
    I(item("file.quit", "Quit", "Quit", When::Window)
        .key(command("Q"))
        .on(Os::Other)),
];

static EDIT: &[Node] = &[
    I(item("edit.undo", "Undo", "Undo", When::Undo).key(command("Z"))),
    I(item("edit.redo", "Redo", "Redo", When::Redo).key(command("Y"))),
    Line,
    I(item(
        "edit.cut",
        "Cut",
        "Cut selection to clipboard",
        When::Selected,
    )
    .key(command("X"))),
    I(item(
        "edit.copy",
        "Copy",
        "Copy selection to clipboard",
        When::Selected,
    )
    .key(command("C"))),
    I(item("edit.paste", "Paste", "Paste clipboard", When::Pasteable).key(command("V"))),
    I(item(
        "edit.delete",
        "Delete selected",
        "Deletes the current selection",
        When::Selected,
    )
    .key(plain("Delete"))
    .on(Os::Other)),
    I(item(
        "edit.delete_all",
        "Delete all",
        "Deletes all objects",
        When::PrepareObjects,
    )
    .key(command("D"))),
    Line,
    I(item(
        "edit.clone",
        "Clone selected",
        "Clone copies of selections",
        When::Selected,
    )
    .key(command("K"))),
    Line,
    I(item(
        "edit.duplicate_plate",
        "Duplicate Current Plate",
        "Duplicate the current plate",
        When::PrepareObjects,
    )),
    Line,
    I(item(
        "edit.select_all",
        "Select all",
        "Selects all objects",
        When::PrepareObjects,
    )
    .key(command("A"))),
    I(item(
        "edit.deselect_all",
        "Deselect all",
        "Deselects all objects",
        When::Selected,
    )
    .key(plain("Escape"))),
];

static VIEW: &[Node] = &[
    I(item("view.default", "Default View", "Default View", When::Plater).key(command("0"))),
    I(item("view.top", "Top", "Top View", When::Plater).key(command("1"))),
    I(item("view.bottom", "Bottom", "Bottom View", When::Plater).key(command("2"))),
    I(item("view.front", "Front", "Front View", When::Plater).key(command("3"))),
    I(item("view.rear", "Rear", "Rear View", When::Plater).key(command("4"))),
    I(item("view.left", "Left", "Left View", When::Plater).key(command("5"))),
    I(item("view.right", "Right", "Right View", When::Plater).key(command("6"))),
    I(item("view.fit", "Fit in all view", "Fit in all view", When::Plater).key(plain("Z"))),
    Line,
    I(item(
        "view.perspective",
        "Use Perspective View",
        "Use Perspective View",
        When::Always,
    )
    .radio(Check::Perspective)),
    I(item(
        "view.orthogonal",
        "Use Orthogonal View",
        "Use Orthogonal View",
        When::Always,
    )
    .radio(Check::Orthogonal)),
    I(item(
        "view.auto_perspective",
        "Auto Perspective",
        "Automatically switch between orthographic and perspective when changing from top/bottom/side views.",
        When::Plater,
    )
    .check(Check::AutoPerspective)),
    Line,
    I(item(
        "view.gcode_window",
        "Show &G-code Window",
        "Show G-code window in Preview scene.",
        When::Preview,
    )
    .key(plain("C"))
    .check(Check::GcodeWindow)),
    I(item(
        "view.navigator",
        "Show 3D Navigator",
        "Show 3D navigator in Prepare and Preview scene.",
        When::Plater,
    )
    .check(Check::Navigator)),
    I(item(
        "view.reset_layout",
        "Reset Window Layout",
        "Reset to default window layout",
        When::Plater,
    )
    .waits(NO_LAYOUT)),
    Line,
    I(item(
        "view.3dconnexion",
        "Show 3Dconnexion Settings",
        "Show/Hide 3Dconnexion devices settings dialog",
        When::Always,
    )
    .waits(NO_3DCONNEXION)),
    Line,
    I(item(
        "view.labels",
        "Show &Labels",
        "Show object labels in 3D scene.",
        When::Plater,
    )
    .key(command("E"))
    .check(Check::Labels)),
    I(item(
        "view.overhang",
        "Show &Overhang",
        "Show object overhang highlight in 3D scene.",
        When::Plater,
    )
    .waits(NO_OVERHANG)),
    I(item(
        "view.xray",
        "X-Ray",
        "Show every object as a translucent, edge-weighted surface to see inside and behind parts.",
        When::Plater,
    )
    .waits(NO_XRAY)),
    I(item(
        "view.realistic",
        "Realistic View",
        "Render objects with Phong shading, and optionally ambient occlusion and cast shadows (Preferences > Graphics).",
        When::Plater,
    )
    .waits(NO_REALISTIC)),
    I(item(
        "view.center_of_mass",
        "Show Center of Mass",
        "Show the center of mass of each plate, object, separate part and support in Prepare and Preview. Click a marker for its weight, volume and moment of inertia.",
        When::Plater,
    )
    .waits(NO_MASS)),
    Line,
    I(item(
        "view.compare",
        "Compare Slices",
        "Compare two sliced results visually",
        When::Always,
    )
    .dots()
    .waits(NO_COMPARE)),
    I(item(
        "view.compare_previous",
        "Compare with Previous Slice",
        "Compare the last two slices of this session",
        When::Always,
    )
    .waits(NO_COMPARE)),
    Line,
    I(item(
        "view.mobile_hub",
        "Mobile Hub",
        "Open the Mobile Hub page in your browser (phone access, cameras, open windows)",
        When::Always,
    )
    .dots()
    .waits(NO_MOBILE_HUB)
    .on(Os::Mac)),
];

static FLOW_RATE: &[Node] = &[
    I(item(
        "calibration.pass1",
        "Pass 1",
        "Flow rate test - Pass 1",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.pass2",
        "Pass 2",
        "Flow rate test - Pass 2",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    Line,
    I(item(
        "calibration.yolo",
        "YOLO (Recommended)",
        "Orca YOLO flowrate calibration, 0.01 step",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.yolo_perfectionist",
        "YOLO (perfectionist version)",
        "Orca YOLO flowrate calibration, 0.005 step",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
];

static INPUT_SHAPING: &[Node] = &[
    I(item(
        "calibration.shaping_frequency",
        "Input Shaping Frequency",
        "Input Shaping Frequency",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.shaping_damping",
        "Input Shaping Damping/zeta factor",
        "Input Shaping Damping/zeta factor",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
];

static CALIBRATION: &[Node] = &[
    I(item(
        "calibration.temperature",
        "Temperature",
        "Temperature Calibration",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    Node::Menu {
        id: "calibration.flow_rate",
        label: "Flow rate",
        os: Os::All,
        items: FLOW_RATE,
    },
    I(item(
        "calibration.pressure_advance",
        "Pressure advance",
        "Pressure advance",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.retraction",
        "Retraction test",
        "Retraction test",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.stringing",
        "Stringing (temp × retraction)",
        "Combined temperature and retraction stringing grid",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.max_flowrate",
        "Max flowrate",
        "Max flowrate",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.cornering",
        "Cornering",
        "Cornering calibration",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    Node::Menu {
        id: "calibration.input_shaping",
        label: "Input Shaping",
        os: Os::All,
        items: INPUT_SHAPING,
    },
    I(item("calibration.vfa", "VFA", "VFA", When::Always).waits(NO_CALIBRATION)),
    I(item(
        "calibration.ironing",
        "Ironing flow",
        "Ironing flow calibration",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
    I(item(
        "calibration.tutorial",
        "Tutorial",
        "Calibration help",
        When::Always,
    )
    .waits(NO_CALIBRATION)),
];

static WINDOW: &[Node] = &[
    I(item("window.minimize", "Minimize", "", When::Window).key(command("M"))),
    I(item("window.zoom", "Zoom", "", When::Window)),
    Line,
    I(item("window.next", "Next Project", "", When::Projects).key(command_shift("]"))),
    I(item("window.previous", "Previous Project", "", When::Projects).key(command_shift("["))),
    Node::Projects,
];

static HELP: &[Node] = &[
    I(item(
        "help.shortcuts",
        "Keyboard Shortcuts",
        "Show the list of the keyboard shortcuts",
        When::Always,
    )
    .key(plain("?"))),
    I(item("help.wizard", "Setup Wizard", "Setup Wizard", When::Always)),
    Line,
    I(item(
        "help.config_folder",
        "Show Configuration Folder",
        "Show Configuration Folder",
        When::Always,
    )
    .waits(NO_FOLDER)),
    I(item(
        "help.migrate",
        "Migrate Filament Presets",
        "Bring back filament presets that a profile update removed from printers they were made for",
        When::Always,
    )
    .waits(NO_MIGRATE)),
    I(item(
        "help.export_logs",
        "Export Logs",
        "Export application logs as ZIP file",
        When::Always,
    )
    .waits(NO_LOGS)),
    I(item("help.tip", "Show Tip of the Day", "Show Tip of the Day", When::Always).waits(NO_TIPS)),
    I(item("help.update", "Check for Update", "Check for Update", When::Always).waits(NO_UPDATES)),
    I(item("help.import_profile", "Import Profile", "Import Profile", When::Always).waits(NO_PRESETS)),
    I(item("help.network_test", "Open Network Test", "Open Network Test", When::Always)
        .waits(NO_NETWORK_TEST)),
    I(item(
        "help.firewall",
        "Check Windows Firewall...",
        "Check whether Windows Firewall blocks printer discovery, and fix its rules",
        When::Always,
    )
    .waits(NO_FIREWALL)
    .on(Os::Other)),
    I(item(
        "help.bambu",
        "Bambu Lab Printer Help...",
        "What a Bambu Lab printer needs to show up and connect",
        When::Always,
    )
    .waits(NO_BAMBU_HELP)),
    I(item("help.about", "&About %s", "", When::Always).on(Os::Other)),
];

/// macOS's application menu: what the reference puts there.
static APP: &[Node] = &[
    I(item("app.about", "&About %s", "", When::Always).on(Os::Mac)),
    I(item("app.preferences", "Preferences", "", When::Always)
        .key(command(","))
        .on(Os::Mac)),
    I(item(
        "app.themes",
        "Themes",
        "Colours, fonts, corners and the title bar",
        When::Always,
    )
    .dots()
    .waits(NO_THEMES)
    .on(Os::Mac)),
    I(item(
        "app.costs",
        "Costs",
        "Your own filament prices and machine rates, over the presets' values",
        When::Always,
    )
    .dots()
    .waits(NO_COSTS)
    .on(Os::Mac)),
    Line,
    I(item("file.quit", "Quit", "Quit", When::Window)
        .key(command("Q"))
        .on(Os::Mac)),
];

/// The menu bar, in the reference's order. On macOS the application's
/// own menu comes first and takes the system's name for it.
pub static BAR: &[Node] = &[
    Node::Menu {
        id: "app",
        label: APP_NAME,
        os: Os::Mac,
        items: APP,
    },
    Node::Menu {
        id: "file",
        label: "File",
        os: Os::All,
        items: FILE,
    },
    Node::Menu {
        id: "edit",
        label: "Edit",
        os: Os::All,
        items: EDIT,
    },
    Node::Menu {
        id: "view",
        label: "View",
        os: Os::All,
        items: VIEW,
    },
    Node::Menu {
        id: "calibration",
        label: "Calibration",
        os: Os::All,
        items: CALIBRATION,
    },
    Node::Menu {
        id: "window",
        label: "Window",
        os: Os::Mac,
        items: WINDOW,
    },
    Node::Menu {
        id: "help",
        label: "Help",
        os: Os::All,
        items: HELP,
    },
];

/// Whether the menus are those of a Mac.
pub fn on_mac() -> bool {
    cfg!(target_os = "macos")
}

fn shows(os: Os, mac: bool) -> bool {
    match os {
        Os::All => true,
        Os::Mac => mac,
        Os::Other => !mac,
    }
}

/// Every entry of the bar, on either system, with the menu it is in.
pub fn entries() -> Vec<(&'static str, Item)> {
    fn walk(menu: &'static str, nodes: &'static [Node], out: &mut Vec<(&'static str, Item)>) {
        for node in nodes {
            match node {
                Node::Item(item) => out.push((menu, *item)),
                Node::Menu { id, items, .. } => walk(id, items, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk("", BAR, &mut out);
    out
}

/// A label as wxWidgets shows it: each `&` that marks the next letter
/// taken out, "&&" one "&".
pub fn shown_label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// An entry's words in the language of the call.
pub fn label_of(item: &Item) -> String {
    let words = tr(item.label);
    let words = if item.label.contains("%s") {
        super::i18n::fill(&words, &[&APP_NAME])
    } else {
        words
    };
    let mut shown = shown_label(&words);
    if item.dots {
        shown.push_str("...");
    }
    shown
}

/// How a key is shown: "⇧⌘S" on a Mac, "Ctrl+Shift+S" elsewhere (the
/// reference's "Ctrl+" and "Shift+", in the language), "Del" and "Esc" in
/// the language.
pub fn key_text(key: &Key, mac: bool) -> String {
    let name = match key.key {
        "Delete" => {
            if mac {
                "⌦".to_owned()
            } else {
                tr("Del")
            }
        }
        "Escape" => tr("Esc"),
        other => other.to_owned(),
    };
    if mac {
        format!(
            "{}{}{name}",
            if key.shift { "⇧" } else { "" },
            if key.command { "⌘" } else { "" }
        )
    } else {
        format!(
            "{}{}{name}",
            if key.command {
                tr("Ctrl+")
            } else {
                String::new()
            },
            if key.shift {
                tr("Shift+")
            } else {
                String::new()
            }
        )
    }
}

/// The key as the system's menu takes it (Tauri's accelerators), when the
/// window's menu bar holds it: an entry's key with the command key, but
/// Cut, Copy and Paste, which are the system's own there (so a text field
/// keeps its keys) and whose keys the page reads from the clipboard's own
/// events. Every other key is the page's to bind.
pub fn accelerator(item: &Item) -> Option<String> {
    let key = item.key?;
    if !key.command || matches!(item.id, "edit.cut" | "edit.copy" | "edit.paste") {
        return None;
    }
    let name = match key.key {
        "]" => "BracketRight",
        "[" => "BracketLeft",
        "," => "Comma",
        other => other,
    };
    Some(format!(
        "CmdOrCtrl+{}{name}",
        if key.shift { "Shift+" } else { "" }
    ))
}

/// What the page says of itself when it asks for the menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub tab: String,
    pub selected: bool,
    pub pasteable: bool,
    pub window: bool,
    pub perspective: bool,
    pub auto_perspective: bool,
    pub gcode_window: bool,
    pub navigator: bool,
    pub labels: bool,
}

impl PageState {
    pub fn of(args: &Value) -> Self {
        let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
        Self {
            tab: args
                .get("tab")
                .and_then(Value::as_str)
                .unwrap_or("prepare")
                .to_owned(),
            selected: flag("selected"),
            pasteable: flag("pasteable"),
            window: flag("window"),
            perspective: args
                .get("perspective")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            auto_perspective: flag("auto_perspective"),
            gcode_window: flag("gcode_window"),
            navigator: flag("navigator"),
            labels: flag("labels"),
        }
    }
}

/// What the session knows that decides an entry.
#[derive(Clone, Debug, Default)]
pub struct SessionState {
    pub slicing: bool,
    pub dirty: bool,
    pub objects: bool,
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub gcode: bool,
    pub all_gcode: bool,
    pub recent: Vec<(String, bool)>,
    pub projects: Vec<(u32, String)>,
    pub active: u32,
    /// The calls of [`When::Call`] the session has.
    pub calls: Vec<&'static str>,
}

/// Whether an entry can be used now.
pub fn enabled(item: &Item, page: &PageState, session: &SessionState) -> bool {
    if item.waits.is_some() {
        return false;
    }
    let prepare = page.tab == "prepare";
    let plater = prepare || page.tab == "preview";
    match item.when {
        When::Always => true,
        When::Idle => !session.slicing,
        When::Dirty => session.dirty && !session.slicing,
        When::Objects => session.objects,
        When::PrepareObjects => prepare && session.objects,
        When::Selected => prepare && page.selected,
        When::Pasteable => prepare && page.pasteable,
        When::Undo => prepare && session.undo.is_some(),
        When::Redo => prepare && session.redo.is_some(),
        When::Plater => plater,
        When::PrepareIdle => prepare && !session.slicing,
        When::Preview => page.tab == "preview",
        When::Gcode => plater && session.gcode && !session.slicing,
        When::AllGcode => plater && session.all_gcode && !session.slicing,
        When::Projects => session.projects.len() > 1,
        When::Window => page.window,
        When::Call(names) => names.iter().any(|name| session.calls.contains(name)),
    }
}

fn checked(check: Check, page: &PageState) -> bool {
    match check {
        Check::Perspective => page.perspective,
        Check::Orthogonal => !page.perspective,
        Check::AutoPerspective => page.auto_perspective,
        Check::GcodeWindow => page.gcode_window,
        Check::Navigator => page.navigator,
        Check::Labels => page.labels,
    }
}

/// The menu bar as the page and the window draw it.
pub fn tree(page: &PageState, session: &SessionState, mac: bool) -> Value {
    fn nodes(nodes: &[Node], page: &PageState, session: &SessionState, mac: bool) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for node in nodes {
            match node {
                Node::Item(item) if shows(item.os, mac) => {
                    out.push(entry(item, page, session, mac))
                }
                Node::Item(_) => {}
                Node::Menu {
                    id,
                    label,
                    os,
                    items,
                } if shows(*os, mac) => {
                    let inner = nodes_of(items, page, session, mac);
                    let enabled = match *id {
                        // As the reference: off while there is nothing
                        // in it, or the project slices.
                        "file.recent" => !session.recent.is_empty() && !session.slicing,
                        _ => true,
                    };
                    let label = if *id == "app" {
                        APP_NAME.to_owned()
                    } else {
                        shown_label(&tr(label))
                    };
                    out.push(json!({
                        "id": id,
                        "label": label,
                        "enabled": enabled,
                        "items": inner,
                    }));
                }
                Node::Menu { .. } => {}
                Node::Line => {
                    // No line first, last or twice over.
                    if out.last().is_some_and(|last| last["line"] != json!(true)) {
                        out.push(json!({ "line": true }));
                    }
                }
                Node::Recent => {
                    for (at, (path, missing)) in session.recent.iter().enumerate() {
                        // wxFileHistory's labels, the path in full
                        // (`wxFH_PATH_SHOW_ALWAYS`), numbered from 1.
                        out.push(json!({
                            "id": format!("recent:{at}"),
                            "label": format!("{} {path}", at + 1),
                            "path": path,
                            "missing": missing,
                            "enabled": !session.slicing,
                        }));
                    }
                }
                Node::Projects => {
                    if !session.projects.is_empty()
                        && out.last().is_some_and(|last| last["line"] != json!(true))
                    {
                        out.push(json!({ "line": true }));
                    }
                    for (id, name) in &session.projects {
                        out.push(json!({
                            "id": format!("project:{id}"),
                            "label": name,
                            "enabled": true,
                            "checked": *id == session.active,
                        }));
                    }
                }
            }
        }
        while out.last().is_some_and(|last| last["line"] == json!(true)) {
            out.pop();
        }
        out
    }
    fn nodes_of(items: &[Node], page: &PageState, session: &SessionState, mac: bool) -> Vec<Value> {
        nodes(items, page, session, mac)
    }
    fn entry(item: &Item, page: &PageState, session: &SessionState, mac: bool) -> Value {
        let mut label = label_of(item);
        // The step that would be taken back or made again, by its name.
        let step = match item.id {
            "edit.undo" => session.undo.as_deref(),
            "edit.redo" => session.redo.as_deref(),
            _ => None,
        };
        if let Some(step) = step {
            label = format!("{label}: {}", tr(step));
        }
        let tip = match (item.waits, item.tip) {
            (Some(waits), _) => waits.to_owned(),
            (None, "") => String::new(),
            (None, tip) => tr(tip),
        };
        json!({
            "id": item.id,
            "label": label,
            "key": item.key.map(|key| key_text(&key, mac)),
            "bind": item.key.map(|key| json!({ "command": key.command, "shift": key.shift, "key": key.key })),
            "accelerator": accelerator(item),
            "enabled": enabled(item, page, session),
            "checked": item.check.map(|check| checked(check, page)),
            "radio": item.radio,
            "tip": tip,
            "waits": item.waits.is_some(),
        })
    }
    json!({
        "mac": mac,
        "menus": nodes(BAR, page, session, mac),
    })
}

// ---- the keyboard shortcuts window ----

/// The reference's lists of keys (KBShortcutsDialog.cpp
/// `fill_shortcuts`): each section's title, then each key with what it
/// does, both message ids; a key that is one of the menu bar's is named by
/// the entry's id, so it is shown as the bar shows it. `built` says
/// whether this app does it.
struct Shortcut {
    key: Keyed,
    does: &'static str,
    built: bool,
}

enum Keyed {
    /// An entry of the bar, by its id.
    Entry(&'static str),
    Key(Key),
    /// Words (a message id), a key with the command key, or Shift.
    Words(&'static str),
    Command(&'static str),
    Shift(&'static str),
    Alt(&'static str),
}

const fn row(key: Keyed, does: &'static str, built: bool) -> Shortcut {
    Shortcut { key, does, built }
}

fn sections(mac: bool) -> Vec<(&'static str, Vec<Shortcut>)> {
    use Keyed::{Alt, Command, Entry, Key as K, Shift, Words};
    let delete = if mac {
        K(plain("Delete"))
    } else {
        Words("Del")
    };
    vec![
        (
            "Global shortcuts",
            vec![
                row(Entry("file.new"), "New Project", true),
                row(Entry("file.open"), "Open Project", true),
                row(Entry("file.save"), "Save Project", true),
                row(Entry("file.save_as"), "Save Project as", true),
                row(
                    Entry("import.model"),
                    "Import geometry data from STL/STEP/3MF/OBJ/AMF files",
                    true,
                ),
                row(
                    Entry("export.plate_sliced"),
                    "Export plate sliced file",
                    false,
                ),
                row(K(command("R")), "Slice plate", true),
                row(K(command_shift("G")), "Print plate", false),
                row(Entry("edit.cut"), "Cut", true),
                row(Entry("edit.copy"), "Copy to clipboard", true),
                row(Entry("edit.paste"), "Paste from clipboard", true),
                row(Entry("app.preferences"), "Preferences", true),
                row(
                    K(if mac {
                        command_shift("M")
                    } else {
                        command("M")
                    }),
                    "Show/Hide 3Dconnexion devices settings dialog",
                    false,
                ),
                row(Command("Tab"), "Switch table page", false),
                row(Command("PgDn"), "Next open project", true),
                row(Command("PgUp"), "Previous open project", true),
                row(delete, "Delete selected", true),
                row(
                    Entry("help.shortcuts"),
                    "Show keyboard shortcuts list",
                    true,
                ),
            ],
        ),
        (
            "Plater",
            vec![
                row(Words("Left mouse button"), "Rotate View", true),
                row(Words("Right mouse button"), "Pan View", true),
                row(Words("Mouse wheel"), "Zoom View", true),
                row(K(plain("A")), "Arrange all objects", true),
                row(Shift("A"), "Arrange objects on selected plates", true),
                row(
                    K(plain("Q")),
                    "Auto orients selected objects or all objects. If there are selected objects, it just orients the selected ones. Otherwise, it will orient all objects in the current project.",
                    false,
                ),
                row(
                    Shift("Q"),
                    "Auto orients all objects on the active plate.",
                    false,
                ),
                row(Shift("Tab"), "Collapse/Expand the sidebar", false),
                row(Command("Any arrow"), "Movement in camera space", false),
                row(Alt("Left mouse button"), "Select a part", false),
                row(
                    Command("Left mouse button"),
                    "Select multiple objects",
                    true,
                ),
                row(
                    Shift("Left mouse button"),
                    "Select objects by rectangle",
                    false,
                ),
                row(
                    Words("Arrow Up"),
                    "Move selection 10 mm in positive Y direction",
                    true,
                ),
                row(
                    Words("Arrow Down"),
                    "Move selection 10 mm in negative Y direction",
                    true,
                ),
                row(
                    Words("Arrow Left"),
                    "Move selection 10 mm in negative X direction",
                    true,
                ),
                row(
                    Words("Arrow Right"),
                    "Move selection 10 mm in positive X direction",
                    true,
                ),
                row(Shift("Any arrow"), "Movement step set to 1 mm", true),
                row(Entry("edit.deselect_all"), "Deselect all", true),
                row(
                    K(plain("1-9")),
                    "keyboard 1-9: set filament for object/part",
                    false,
                ),
                row(Entry("view.default"), "Camera view - Default", true),
                row(Entry("view.top"), "Camera view - Top", true),
                row(Entry("view.bottom"), "Camera view - Bottom", true),
                row(Entry("view.front"), "Camera view - Front", true),
                row(Entry("view.rear"), "Camera view - Behind", true),
                row(Entry("view.left"), "Camera Angle - Left side", true),
                row(Entry("view.right"), "Camera Angle - Right side", true),
                row(Entry("view.fit"), "Fit in all view", true),
                row(Entry("edit.select_all"), "Select all objects", true),
                row(Entry("edit.delete_all"), "Delete all", true),
                row(Entry("edit.undo"), "Undo", true),
                row(Entry("edit.redo"), "Redo", true),
                row(K(plain("M")), "Gizmo move", true),
                row(K(plain("R")), "Gizmo rotate", true),
                row(K(plain("S")), "Gizmo scale", true),
                row(K(plain("F")), "Gizmo place face on bed", true),
                row(K(plain("C")), "Gizmo cut", true),
                row(K(plain("B")), "Gizmo mesh boolean", false),
                // L opens "Support Painting" here: the reference's list still
                // names it for the SLA gizmo. "Color Painting" [N] is not in
                // its list.
                row(K(plain("H")), "Gizmo FDM paint-on fuzzy skin", true),
                row(K(plain("L")), "Gizmo SLA support points", true),
                row(K(plain("P")), "Gizmo FDM paint-on seam", true),
                row(K(plain("T")), "Gizmo text emboss/engrave", false),
                row(K(plain("U")), "Gizmo measure", false),
                row(K(plain("Y")), "Gizmo assemble", false),
                row(K(plain("E")), "Gizmo brim ears", false),
                row(K(plain("I")), "Zoom in", true),
                row(K(plain("O")), "Zoom out", true),
                row(Words("Tab"), "Switch between Prepare/Preview", true),
            ],
        ),
        (
            "Gizmo",
            vec![
                row(Words("Esc"), "Deselect all", true),
                row(Shift(""), "Move: press to snap by 1mm", false),
                // The painting tools (paint.js `wheel`).
                row(
                    Command("Mouse wheel"),
                    "Support/Color Painting: adjust pen radius",
                    true,
                ),
                row(
                    Alt("Mouse wheel"),
                    "Support/Color Painting: adjust section position",
                    true,
                ),
            ],
        ),
        (
            "Objects List",
            vec![
                row(
                    K(plain("1-9")),
                    "Set extruder number for the objects and parts",
                    false,
                ),
                row(Words("Del"), "Delete objects, parts, modifiers", true),
                row(Words("Esc"), "Deselect all", true),
                row(Entry("edit.copy"), "Copy to clipboard", true),
                row(Entry("edit.paste"), "Paste from clipboard", true),
                row(Entry("edit.cut"), "Cut", true),
                row(Entry("edit.select_all"), "Select all objects", true),
                row(Entry("edit.clone"), "Clone selected", true),
                row(Entry("edit.undo"), "Undo", true),
                row(Entry("edit.redo"), "Redo", true),
                row(
                    Words("Space"),
                    "Select the object/part and press space to change the name",
                    false,
                ),
                row(
                    Words("Mouse click"),
                    "Select the object/part and mouse click to change the name",
                    false,
                ),
            ],
        ),
        (
            "Preview",
            vec![
                row(
                    Words("Arrow Up"),
                    "Vertical slider - Move active thumb Up",
                    true,
                ),
                row(
                    Words("Arrow Down"),
                    "Vertical slider - Move active thumb Down",
                    true,
                ),
                row(
                    Words("Arrow Left"),
                    "Horizontal slider - Move active thumb Left",
                    true,
                ),
                row(
                    Words("Arrow Right"),
                    "Horizontal slider - Move active thumb Right",
                    true,
                ),
                row(
                    K(plain("L")),
                    "On/Off one layer mode of the vertical slider",
                    true,
                ),
                row(Entry("view.gcode_window"), "On/Off G-code window", true),
                row(Words("Tab"), "Switch between Prepare/Preview", true),
                row(Shift("Any arrow"), "Move slider 5x faster", true),
                row(Shift("Mouse wheel"), "Move slider 5x faster", true),
                row(Command("Any arrow"), "Move slider 5x faster", true),
                row(Command("Mouse wheel"), "Move slider 5x faster", true),
                row(
                    Words("Home"),
                    "Horizontal slider - Move to start position",
                    true,
                ),
                row(
                    Words("End"),
                    "Horizontal slider - Move to last position",
                    true,
                ),
            ],
        ),
    ]
}

/// The keyboard shortcuts window's lists, in the language of the call.
pub fn shortcuts(mac: bool) -> Value {
    let all = entries();
    let entry_key = |id: &str| -> String {
        all.iter()
            .find(|(_, item)| item.id == id && shows(item.os, mac))
            .or_else(|| all.iter().find(|(_, item)| item.id == id))
            .and_then(|(_, item)| item.key)
            .map(|key| key_text(&key, mac))
            .unwrap_or_default()
    };
    let prefix = |command: bool, shift: bool, alt: bool| -> String {
        if mac {
            format!(
                "{}{}{}",
                if shift { "⇧" } else { "" },
                if alt { "⌥" } else { "" },
                if command { "⌘" } else { "" }
            )
        } else {
            format!(
                "{}{}{}",
                if command { tr("Ctrl+") } else { String::new() },
                if alt { tr("Alt+") } else { String::new() },
                if shift { tr("Shift+") } else { String::new() }
            )
        }
    };
    let sections: Vec<Value> = sections(mac)
        .into_iter()
        .map(|(title, rows)| {
            let rows: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    let key = match row.key {
                        Keyed::Entry(id) => entry_key(id),
                        Keyed::Key(key) => key_text(&key, mac),
                        Keyed::Words(words) => tr(words),
                        Keyed::Command(words) => prefix(true, false, false) + &tr(words),
                        Keyed::Shift(words) => {
                            let shift = prefix(false, true, false);
                            if words.is_empty() {
                                shift.trim_end_matches('+').to_owned()
                            } else {
                                shift + &tr(words)
                            }
                        }
                        Keyed::Alt(words) => prefix(false, false, true) + &tr(words),
                    };
                    json!({ "key": key, "does": tr(row.does), "built": row.built })
                })
                .collect();
            json!({ "title": tr(title), "rows": rows })
        })
        .collect();
    json!({ "title": tr("Keyboard Shortcuts"), "sections": sections })
}

// ---- recent projects ----

/// `recent` with `path` put first, once (wxFileHistory
/// `AddFileToHistory`: two spellings of one path are one entry), at most
/// [`RECENT_MOST`].
pub fn with_recent(recent: &[String], path: &str) -> Vec<String> {
    let key = super::normalized(path, cfg!(windows));
    let mut out = vec![path.to_owned()];
    out.extend(
        recent
            .iter()
            .filter(|kept| super::normalized(kept, cfg!(windows)) != key)
            .cloned(),
    );
    out.truncate(RECENT_MOST);
    out
}

/// Writes `bytes` to `path` whole or not at all: beside it first, then
/// put in its place.
pub fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut beside = path.as_os_str().to_owned();
    beside.push(".tmp");
    let beside = PathBuf::from(beside);
    let failed = |error: std::io::Error| format!("{}: {error}", path.display());
    std::fs::write(&beside, bytes).map_err(failed)?;
    std::fs::rename(&beside, path).map_err(|error| {
        let _ = std::fs::remove_file(&beside);
        failed(error)
    })
}

impl Session {
    /// What decides the menu's entries, of the project shown and the
    /// window.
    fn menu_session(self: &std::sync::Arc<Self>) -> SessionState {
        let calls = preset_calls(self);
        let state = lock(&self.state);
        let project = &state.projects[state.active_index()];
        let marks = project.history.marks();
        let name = |key: &str| marks.get(key).and_then(Value::as_str).map(str::to_owned);
        let gcode = project
            .results
            .get(&slot(project.plate))
            .is_some_and(|done| done.gcode.is_ok());
        let sliceable = project.sliceable();
        let all_gcode = !sliceable.is_empty()
            && sliceable.iter().all(|plate| {
                project
                    .results
                    .get(&slot(*plate))
                    .is_some_and(|done| done.gcode.is_ok())
            });
        SessionState {
            slicing: project.slicing(),
            dirty: project.dirty(),
            objects: project
                .file
                .as_ref()
                .is_some_and(|file| !file.model.objects().is_empty()),
            undo: name("undo"),
            redo: name("redo"),
            gcode,
            all_gcode,
            recent: state
                .store
                .data
                .recent_projects
                .iter()
                .map(|path| (path.clone(), !Path::new(path).is_file()))
                .collect(),
            projects: state
                .projects
                .iter()
                .map(|project| (project.id, project.name.clone()))
                .collect(),
            active: state.active,
            calls,
        }
    }

    pub(super) fn menu_json(
        self: &std::sync::Arc<Self>,
        name: &str,
        args: &Value,
    ) -> Result<Value, String> {
        match name {
            "menu" => {
                let session = self.menu_session();
                let mut tree = tree(&PageState::of(args), &session, on_mac());
                tree["recent"] = json!(
                    session
                        .recent
                        .iter()
                        .map(|(path, missing)| json!({ "path": path, "missing": missing }))
                        .collect::<Vec<_>>()
                );
                tree["preset_calls"] = json!(session.calls);
                Ok(tree)
            }
            "shortcuts" => Ok(shortcuts(on_mac())),
            "recent_remove" => {
                let path = super::needed(args, "path")?.to_owned();
                let key = super::normalized(&path, cfg!(windows));
                let mut state = lock(&self.state);
                state.store.change(|data| {
                    data.recent_projects
                        .retain(|kept| super::normalized(kept, cfg!(windows)) != key);
                })?;
                Ok(json!({ "recent": state.store.data.recent_projects }))
            }
            "export_stl" => self.export_stl(args),
            "export_stls" => self.export_stls(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// Puts a project file first among the recent projects, as the
    /// reference does when a project is opened or saved (Plater.cpp
    /// `set_project_filename`): a 3MF that is there. An STL opened as a
    /// project is not one (the reference lists a model only with its
    /// "recent_models" preference, off unless asked for).
    pub(super) fn note_recent(&self, path: &Path) {
        let is_project = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"));
        if !is_project || !path.is_file() {
            return;
        }
        let path = shown(path);
        let mut state = lock(&self.state);
        let recent = with_recent(&state.store.data.recent_projects, &path);
        // Not kept, it is not said: opening or saving went well.
        let _ = state.store.change(|data| data.recent_projects = recent);
    }

    fn export_model(&self, args: &Value) -> Result<(edge_engine::Model, String), String> {
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        let file = project
            .file
            .as_ref()
            .filter(|file| !file.model.objects().is_empty())
            .ok_or("There are no objects in the project.")?;
        Ok((file.model.clone(), project.name.clone()))
    }

    /// "Export all objects as one STL...": to `path` (".stl" added to a
    /// name without it). A file that is there is not written over unless
    /// `replace` says so (the save dialog asked); else the answer is
    /// `{ exists: true, path }`. Answers `{ path, bytes, triangles, uncut }`.
    /// With `suggest: true` only `{ suggested }`, the name the dialog
    /// offers (the project's, as `get_export_file` names it).
    fn export_stl(&self, args: &Value) -> Result<Value, String> {
        let (model, name) = self.export_model(args)?;
        if args
            .get("suggest")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let stem = model
                .path()
                .and_then(Path::file_stem)
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or(name);
            let file = format!("{stem}.stl");
            let suggested = match model
                .path()
                .and_then(Path::parent)
                .filter(|dir| !dir.as_os_str().is_empty())
            {
                Some(dir) => shown(&dir.join(file)),
                None => file,
            };
            return Ok(json!({ "suggested": suggested }));
        }
        let mut path = PathBuf::from(super::needed(args, "path")?.trim());
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"))
        {
            let mut named = path.into_os_string();
            named.push(".stl");
            path = PathBuf::from(named);
        }
        let replace = args
            .get("replace")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.exists() && !replace {
            return Ok(json!({ "exists": true, "path": shown(&path) }));
        }
        let exported = edge_engine::export::model_mesh(&model);
        let bytes = edge_engine::export::stl_binary(&exported.mesh);
        write_whole(&path, &bytes)?;
        Ok(json!({
            "path": shown(&path),
            "bytes": bytes.len(),
            "triangles": exported.mesh.triangles.len(),
            "uncut": exported.uncut,
        }))
    }

    /// "Export all objects as STLs...": one file for each object, all its
    /// copies in it, in the folder `dir`, named by the object; a name
    /// taken gets "(1)", "(2)" (Plater.cpp `get_save_file`), so nothing
    /// is written over. Answers `{ files: [path], uncut }`.
    fn export_stls(&self, args: &Value) -> Result<Value, String> {
        let (model, _) = self.export_model(args)?;
        let dir = PathBuf::from(super::needed(args, "dir")?.trim());
        if !dir.is_dir() {
            return Err(format!("{}: there is no such folder", dir.display()));
        }
        let mut files = Vec::new();
        let mut uncut = Vec::new();
        for object in model.objects() {
            let exported = edge_engine::export::object_mesh(object);
            let stem = edge_engine::export::file_stem(object.name());
            let mut path = dir.join(format!("{stem}.stl"));
            let mut n = 1;
            while path.exists() {
                path = dir.join(format!("{stem}({n}).stl"));
                n += 1;
            }
            write_whole(&path, &edge_engine::export::stl_binary(&exported.mesh))?;
            files.push(shown(&path));
            uncut.extend(exported.uncut);
        }
        Ok(json!({ "files": files, "uncut": uncut }))
    }
}

/// The calls of the presets' import and export that the session has,
/// looked up once by name: each is asked `{ "probe": true }`, and one that
/// is not there answers "there is no call named …". (A call that is there
/// should answer a probe without acting on it.)
fn preset_calls(session: &std::sync::Arc<Session>) -> Vec<&'static str> {
    static FOUND: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            PRESET_IMPORT
                .iter()
                .chain(PRESET_EXPORT)
                .copied()
                .filter(|name| {
                    !matches!(
                        session.answer(name, &json!({ "probe": true })),
                        Err(error) if error == format!("there is no call named {name}")
                    )
                })
                .collect()
        })
        .clone()
}

#[cfg(test)]
mod tests;
