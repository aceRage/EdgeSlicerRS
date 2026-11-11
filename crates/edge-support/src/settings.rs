use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{Flow, FlowError, LineWidths};

/// How supports are built (`support_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Columns straight down, squared off to a grid and filled with lines.
    Normal,
    /// Thin branches from many points under the overhang that lean
    /// together into trunks on their way down.
    Tree,
}

/// The settings supports depend on. Defaults are the reference's
/// (PrintConfig.cpp). Lengths are in mm, angles in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct SupportSettings {
    pub enabled: bool,
    pub style: Style,
    /// A surface rising at less than this angle from level is held up;
    /// zero to go by `threshold_overlap` instead.
    pub threshold_angle: f64,
    /// How much of a wall must rest on the layer below for it to need no
    /// support, when no angle is set.
    pub threshold_overlap: FloatOrPercent,
    /// Only hold up what has nothing of the part beneath it.
    pub on_build_plate_only: bool,
    pub remove_small_overhang: bool,
    /// Gap beside a support, from the part on the same layer.
    pub xy_distance: f64,
    /// The same on the first layer, where the support spreads out.
    pub first_layer_gap: f64,
    /// Air left between the top of a support and what it holds up, and
    /// between the part and a support resting on it.
    pub top_z_distance: f64,
    pub bottom_z_distance: f64,
    /// Gap between the lines of the sparse body of a support.
    pub base_spacing: f64,
    /// Direction of those lines; the dense top runs across them.
    pub angle: f64,
    /// How many layers at the top of a support are dense, the contact included.
    pub interface_top_layers: usize,
    /// Gap between the lines of the dense top.
    pub interface_spacing: f64,
    /// How many layers at the foot of a support that stands on the part
    /// are dense, and the gap between their lines.
    pub interface_bottom_layers: usize,
    pub bottom_interface_spacing: f64,
    /// Grows every overhang sideways before it is supported.
    pub expansion: f64,
    /// On the first layer a support is this dense and this much wider.
    pub first_layer_density: f64,
    pub first_layer_expansion: f64,
    /// Tree supports: how thick a branch is at its tip and once grown, how
    /// much thicker it gets on the way down (an angle), how far it may
    /// lean from upright (usually, and at most), how thickly tips are set
    /// under an overhang, and how many walls a branch has (0 for one, two
    /// where it is thick).
    pub tree_tip_diameter: f64,
    pub tree_branch_diameter: f64,
    pub tree_branch_diameter_angle: f64,
    pub tree_angle_slow: f64,
    pub tree_branch_angle: f64,
    pub tree_top_rate: f64,
    pub tree_wall_count: usize,
    pub support_line_width: FloatOrPercent,
    pub line_width: FloatOrPercent,
    pub initial_layer_line_width: FloatOrPercent,
    pub outer_wall_line_width: FloatOrPercent,
    pub nozzle_diameter: f64,
    /// Settings present in the input that are not honoured, each with what
    /// is done instead.
    pub unsupported: Vec<String>,
}

impl Default for SupportSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            style: Style::Normal,
            threshold_angle: 30.0,
            threshold_overlap: FloatOrPercent::Percent(50.0),
            on_build_plate_only: false,
            remove_small_overhang: true,
            xy_distance: 0.35,
            first_layer_gap: 0.2,
            top_z_distance: 0.2,
            bottom_z_distance: 0.2,
            base_spacing: 2.5,
            angle: 0.0,
            interface_top_layers: 3,
            interface_spacing: 0.5,
            interface_bottom_layers: 0,
            bottom_interface_spacing: 0.5,
            expansion: 0.0,
            first_layer_density: 0.9,
            first_layer_expansion: 2.0,
            tree_tip_diameter: 0.8,
            tree_branch_diameter: 2.0,
            tree_branch_diameter_angle: 5.0,
            tree_angle_slow: 25.0,
            tree_branch_angle: 40.0,
            tree_top_rate: 0.3,
            tree_wall_count: 0,
            support_line_width: FloatOrPercent::Value(0.0),
            line_width: FloatOrPercent::Value(0.4),
            initial_layer_line_width: FloatOrPercent::Value(0.4),
            outer_wall_line_width: FloatOrPercent::Value(0.4),
            nozzle_diameter: 0.4,
            unsupported: Vec::new(),
        }
    }
}

impl SupportSettings {
    /// Reads what is present; anything absent keeps its default.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let mut unsupported = Vec::new();
        let number = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(settings.float(key)?.unwrap_or(default))
        };
        let width = |key: &str, default| {
            settings
                .float_or_percent(key)
                .map(|found| found.unwrap_or(default))
        };
        let flag = |key: &str, default: bool| -> Result<bool, ConfigError> {
            Ok(settings.flag(key)?.unwrap_or(default))
        };
        let count = |key: &str, default: usize| -> Result<usize, ConfigError> {
            Ok(settings
                .integer(key)?
                .map_or(default, |found| usize::try_from(found).unwrap_or(0)))
        };

        let wanted = flag("enable_support", defaults.enabled)?;
        let mut enabled = wanted;
        let mut style = defaults.style;
        if wanted {
            match settings.text("support_type") {
                None | Some("normal(auto)") => {}
                Some("tree(auto)") => style = Style::Tree,
                Some(other) => {
                    unsupported.push(format!(
                        "support_type \"{other}\": supports only where painted are not \
                         implemented; NO supports are generated"
                    ));
                    enabled = false;
                }
            }
        }
        if enabled {
            // A style this engine has one answer for is noted when the
            // profile names another.
            match (style, settings.text("support_style")) {
                (_, None | Some("default")) | (Style::Normal, Some("grid")) => {}
                (Style::Tree, Some("organic")) => {}
                (Style::Normal, Some(other)) => unsupported.push(format!(
                    "support_style \"{other}\": normal supports are squared off to a grid"
                )),
                (Style::Tree, Some(other)) => unsupported.push(format!(
                    "support_style \"{other}\": tree supports are built one way, as round \
                     branches"
                )),
            }
        }
        if enabled {
            let text = |key: &str| settings.text(key);
            // Present and not a value that means the default behaviour.
            for (key, plain, instead) in [
                (
                    "support_base_pattern",
                    &["default", "rectilinear"][..],
                    "supports are filled with straight lines",
                ),
                (
                    "support_interface_pattern",
                    &["auto", "rectilinear"][..],
                    "the dense top is straight lines across the support's",
                ),
                ("raft_layers", &["0"][..], "no raft is printed"),
                (
                    "enforce_support_layers",
                    &["0"][..],
                    "the first layers are supported like the rest",
                ),
                (
                    "bridge_no_support",
                    &["0"][..],
                    "bridges are supported like any overhang",
                ),
                (
                    "independent_support_layer_height",
                    &["0"][..],
                    "support layers are the part's layers",
                ),
                (
                    "support_filament",
                    &["0"][..],
                    "supports are printed in the part's filament",
                ),
                (
                    "support_interface_filament",
                    &["0"][..],
                    "the dense top is printed in the part's filament",
                ),
                // With it go `support_ironing_pattern`, `_flow`,
                // `_spacing`, `_inset`, `_speed` and `_direction`.
                (
                    "support_ironing",
                    &["0"][..],
                    "the dense top of a support is not ironed",
                ),
            ] {
                if let Some(value) = text(key)
                    && !plain.contains(&value)
                {
                    unsupported.push(format!("{key} \"{value}\": {instead}"));
                }
            }
        }
        // A share of full density, written as a percentage.
        let density = match settings.float_or_percent("raft_first_layer_density")? {
            Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => {
                percent / 100.0
            }
            None => defaults.first_layer_density,
        };
        Ok(Self {
            enabled,
            style,
            threshold_angle: number("support_threshold_angle", defaults.threshold_angle)?,
            threshold_overlap: width("support_threshold_overlap", defaults.threshold_overlap)?,
            on_build_plate_only: flag("support_on_build_plate_only", false)?,
            remove_small_overhang: flag(
                "support_remove_small_overhang",
                defaults.remove_small_overhang,
            )?,
            xy_distance: number("support_object_xy_distance", defaults.xy_distance)?,
            first_layer_gap: number("support_object_first_layer_gap", defaults.first_layer_gap)?,
            top_z_distance: number("support_top_z_distance", defaults.top_z_distance)?,
            bottom_z_distance: number("support_bottom_z_distance", defaults.bottom_z_distance)?,
            base_spacing: number("support_base_pattern_spacing", defaults.base_spacing)?,
            angle: number("support_angle", defaults.angle)?,
            interface_top_layers: count(
                "support_interface_top_layers",
                defaults.interface_top_layers,
            )?,
            interface_spacing: number("support_interface_spacing", defaults.interface_spacing)?,
            // -1 stands for "as many as at the top".
            interface_bottom_layers: match settings.integer("support_interface_bottom_layers")? {
                Some(-1) => count(
                    "support_interface_top_layers",
                    defaults.interface_top_layers,
                )?,
                Some(layers) => usize::try_from(layers).unwrap_or(0),
                None => defaults.interface_bottom_layers,
            },
            bottom_interface_spacing: number(
                "support_bottom_interface_spacing",
                defaults.bottom_interface_spacing,
            )?,
            expansion: number("support_expansion", defaults.expansion)?,
            first_layer_density: density,
            first_layer_expansion: number(
                "raft_first_layer_expansion",
                defaults.first_layer_expansion,
            )?,
            tree_tip_diameter: number("tree_support_tip_diameter", defaults.tree_tip_diameter)?,
            tree_branch_diameter: number(
                "tree_support_branch_diameter_organic",
                defaults.tree_branch_diameter,
            )?,
            tree_branch_diameter_angle: number(
                "tree_support_branch_diameter_angle",
                defaults.tree_branch_diameter_angle,
            )?,
            tree_angle_slow: number("tree_support_angle_slow", defaults.tree_angle_slow)?,
            tree_branch_angle: number(
                "tree_support_branch_angle_organic",
                defaults.tree_branch_angle,
            )?,
            tree_top_rate: match settings.float_or_percent("tree_support_top_rate")? {
                Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => {
                    percent / 100.0
                }
                None => defaults.tree_top_rate,
            },
            tree_wall_count: count("tree_support_wall_count", defaults.tree_wall_count)?,
            support_line_width: width("support_line_width", defaults.support_line_width)?,
            line_width: width("line_width", defaults.line_width)?,
            initial_layer_line_width: width(
                "initial_layer_line_width",
                defaults.initial_layer_line_width,
            )?,
            outer_wall_line_width: width("outer_wall_line_width", defaults.outer_wall_line_width)?,
            nozzle_diameter: settings
                .floats("nozzle_diameter")?
                .and_then(|nozzles| nozzles.first().copied().flatten())
                .unwrap_or(defaults.nozzle_diameter),
            unsupported,
        })
    }

    fn widths(&self) -> LineWidths {
        LineWidths {
            line_width: self.line_width,
            initial_layer_line_width: self.initial_layer_line_width,
            nozzle_diameter: self.nozzle_diameter,
        }
    }

    /// A support line on a layer `height` mm thick: `support_line_width`,
    /// or the general width, or the first layer's own on the first layer
    /// (Flow.cpp `support_material_flow`).
    pub fn flow(&self, height: f64, first_layer: bool) -> Result<Flow, FlowError> {
        Flow::from_widths(
            &self.widths(),
            self.support_line_width,
            Flow::auto_width(self.nozzle_diameter),
            height,
            first_layer,
        )
    }

    /// Width of the part's outer wall, which overhangs are measured against.
    pub(crate) fn wall_width(&self, height: f64) -> Result<f64, FlowError> {
        Ok(Flow::from_widths(
            &self.widths(),
            self.outer_wall_line_width,
            Flow::auto_width(self.nozzle_diameter),
            height,
            false,
        )?
        .width)
    }
}
