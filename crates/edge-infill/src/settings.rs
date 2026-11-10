use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{Flow, FlowError, LineWidths};
use edge_geom::scaled;

use crate::{Anchoring, FillParams, Shells};

/// Sparse fill patterns this engine can lay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// One set of parallel lines, turned 90° each layer.
    Lines,
    /// Two crossing sets of lines on every layer.
    Grid,
}

/// The settings infill depends on, read once per print. Defaults are the
/// reference's (PrintConfig.cpp).
#[derive(Clone, Debug, PartialEq)]
pub struct InfillSettings {
    pub top_shell_layers: usize,
    pub bottom_shell_layers: usize,
    pub sparse_infill_density_percent: f64,
    pub pattern: Pattern,
    /// The configured sparse pattern when it is one not implemented here,
    /// in which case `pattern` is the grid used in its place.
    pub unsupported_pattern: Option<String>,
    /// Degrees counter-clockwise from +x.
    pub infill_direction: f64,
    pub solid_infill_direction: f64,
    /// In mm².
    pub minimum_sparse_infill_area: f64,
    /// Longest stretch of wall two sparse lines are joined along; a
    /// percentage is of the sparse line width, and zero joins nothing.
    pub infill_anchor_max: FloatOrPercent,
    pub sparse_infill_line_width: FloatOrPercent,
    pub internal_solid_infill_line_width: FloatOrPercent,
    pub top_surface_line_width: FloatOrPercent,
    pub outer_wall_line_width: FloatOrPercent,
    pub inner_wall_line_width: FloatOrPercent,
    /// How many walls there are, which sets how far a surface reaches
    /// into solid fill to anchor.
    pub wall_loops: usize,
    pub widths: LineWidths,
    /// Fill over air: its plastic as a share of a normal line's, whether
    /// it is a round thread instead of a flattened line, and how closely
    /// its lines are laid, from 0 to 1.
    pub bridge_flow: f64,
    pub thick_bridges: bool,
    pub bridge_density: f64,
    /// Solid fill laid over sparse fill: whether it is a round thread,
    /// and how closely its lines are laid. Its thread is sized by
    /// `bridge_flow`, as the reference sizes it; `internal_bridge_flow` is
    /// applied to the plastic when the G-code is written.
    pub thick_internal_bridges: bool,
    pub internal_bridge_density: f64,
    /// Whether top surface, bottom surface and internal solid lines are
    /// joined end to end (`top_surface_pattern`, `bottom_surface_pattern`,
    /// `internal_solid_infill_pattern`): `monotonicline` leaves them
    /// separate, `monotonic` and `rectilinear` join them.
    pub top_joined: bool,
    pub bottom_joined: bool,
    pub solid_joined: bool,
    /// Settings present in the input that are not honoured, each with
    /// what is done instead.
    pub unsupported: Vec<String>,
}

impl Default for InfillSettings {
    fn default() -> Self {
        let unset = FloatOrPercent::Value(0.0);
        Self {
            top_shell_layers: 4,
            bottom_shell_layers: 3,
            sparse_infill_density_percent: 20.0,
            pattern: Pattern::Grid,
            unsupported_pattern: Some("crosshatch".into()),
            infill_direction: 45.0,
            solid_infill_direction: 45.0,
            minimum_sparse_infill_area: 15.0,
            infill_anchor_max: FloatOrPercent::Value(20.0),
            sparse_infill_line_width: unset,
            internal_solid_infill_line_width: unset,
            top_surface_line_width: unset,
            outer_wall_line_width: unset,
            inner_wall_line_width: unset,
            wall_loops: 2,
            widths: LineWidths {
                line_width: unset,
                initial_layer_line_width: unset,
                nozzle_diameter: 0.4,
            },
            bridge_flow: 1.0,
            thick_bridges: false,
            bridge_density: 1.0,
            thick_internal_bridges: true,
            internal_bridge_density: 1.0,
            top_joined: false,
            bottom_joined: true,
            solid_joined: true,
            unsupported: Vec::new(),
        }
    }
}

impl InfillSettings {
    /// Reads what is present; anything absent keeps its default. Lists are
    /// per extruder and the first extruder's entry is used.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let width = |key: &str| {
            settings
                .float_or_percent(key)
                .map(|found| found.unwrap_or(FloatOrPercent::Value(0.0)))
        };
        let count = |key: &str, default: usize| -> Result<usize, ConfigError> {
            settings.integer(key)?.map_or(Ok(default), |found| {
                usize::try_from(found).map_err(|_| ConfigError::BadValue {
                    key: key.into(),
                    found: found.to_string(),
                    expected: "zero or more",
                })
            })
        };
        let (pattern, unsupported_pattern) = match settings
            .texts("sparse_infill_pattern")
            .and_then(|p| p.first())
        {
            None => (defaults.pattern, defaults.unsupported_pattern),
            Some(name) => match name.as_str() {
                "grid" => (Pattern::Grid, None),
                "line" | "rectilinear" | "zig-zag" => (Pattern::Lines, None),
                other => (Pattern::Grid, Some(other.to_owned())),
            },
        };
        // A share of full density, written as a percentage.
        let share = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(match settings.float_or_percent(key)? {
                Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => {
                    (percent / 100.0).clamp(0.1, 1.0)
                }
                None => default,
            })
        };
        let number = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(settings.float(key)?.unwrap_or(default))
        };
        let flag = |key: &str, default: bool| -> Result<bool, ConfigError> {
            Ok(settings.flag(key)?.unwrap_or(default))
        };
        // Asked for and not done: said, never passed over.
        let mut unsupported = Vec::new();
        for (key, instead) in [
            ("bridge_angle", "each bridge is turned to span its own gap"),
            (
                "internal_bridge_angle",
                "bridges over infill run across the lines beneath them",
            ),
            (
                "bridge_expansion_extra",
                "bridges reach the usual distance onto what is beside them",
            ),
            ("bridge_line_width", "bridges are as wide as the solid fill"),
        ] {
            if let Some(FloatOrPercent::Value(value) | FloatOrPercent::Percent(value)) =
                settings.float_or_percent(key)?
                && value != 0.0
            {
                unsupported.push(format!("{key} \"{value}\": {instead}"));
            }
        }
        // Straight lines are all this engine lays on a surface; any other
        // pattern is laid as joined lines and said so.
        let mut joined = |key: &str, default: bool| match settings
            .texts(key)
            .and_then(|pattern| pattern.first())
            .map(String::as_str)
        {
            None => default,
            Some("monotonicline") => false,
            Some("monotonic" | "rectilinear") => true,
            Some(other) => {
                unsupported.push(format!(
                    "{key} \"{other}\": laid as straight lines joined at their ends"
                ));
                true
            }
        };
        let top_joined = joined("top_surface_pattern", defaults.top_joined);
        let bottom_joined = joined("bottom_surface_pattern", defaults.bottom_joined);
        let solid_joined = joined("internal_solid_infill_pattern", defaults.solid_joined);
        let density = match settings.float_or_percent("sparse_infill_density")? {
            // A percentage whether or not the sign is written.
            Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => percent,
            None => defaults.sparse_infill_density_percent,
        };
        Ok(Self {
            top_shell_layers: count("top_shell_layers", defaults.top_shell_layers)?,
            bottom_shell_layers: count("bottom_shell_layers", defaults.bottom_shell_layers)?,
            sparse_infill_density_percent: density,
            pattern,
            unsupported_pattern,
            infill_direction: settings
                .float("infill_direction")?
                .unwrap_or(defaults.infill_direction),
            solid_infill_direction: settings
                .float("solid_infill_direction")?
                .unwrap_or(defaults.solid_infill_direction),
            minimum_sparse_infill_area: settings
                .float("minimum_sparse_infill_area")?
                .unwrap_or(defaults.minimum_sparse_infill_area),
            infill_anchor_max: settings
                .float_or_percent("infill_anchor_max")?
                .unwrap_or(defaults.infill_anchor_max),
            sparse_infill_line_width: width("sparse_infill_line_width")?,
            internal_solid_infill_line_width: width("internal_solid_infill_line_width")?,
            top_surface_line_width: width("top_surface_line_width")?,
            outer_wall_line_width: width("outer_wall_line_width")?,
            inner_wall_line_width: width("inner_wall_line_width")?,
            wall_loops: count("wall_loops", defaults.wall_loops)?,
            widths: LineWidths {
                line_width: width("line_width")?,
                initial_layer_line_width: width("initial_layer_line_width")?,
                nozzle_diameter: settings
                    .floats("nozzle_diameter")?
                    .and_then(|nozzles| nozzles.first().copied().flatten())
                    .unwrap_or(defaults.widths.nozzle_diameter),
            },
            bridge_flow: number("bridge_flow", defaults.bridge_flow)?,
            thick_bridges: flag("thick_bridges", defaults.thick_bridges)?,
            bridge_density: share("bridge_density", defaults.bridge_density)?,
            thick_internal_bridges: flag(
                "thick_internal_bridges",
                defaults.thick_internal_bridges,
            )?,
            internal_bridge_density: share(
                "internal_bridge_density",
                defaults.internal_bridge_density,
            )?,
            top_joined,
            bottom_joined,
            solid_joined,
            unsupported,
        })
    }

    /// What classifying surfaces needs for layers of this height.
    pub fn shells(&self, layer_height: f64) -> Result<Shells, FlowError> {
        let nozzle = self.widths.nozzle_diameter;
        let outer = Flow::from_widths(
            &self.widths,
            self.outer_wall_line_width,
            Flow::auto_width(nozzle),
            layer_height,
            false,
        )?;
        Ok(Shells {
            top_layers: self.top_shell_layers,
            bottom_layers: self.bottom_shell_layers,
            sparse_density: self.sparse_infill_density_percent / 100.0,
            minimum_sparse_area: self.minimum_sparse_infill_area,
            opening: scaled(outer.width / 10.0),
        })
    }

    /// How far outward-facing surfaces reach into the fill beside them
    /// on layers of this height (`LayerRegion::process_external_surfaces`).
    pub fn anchoring(&self, layer_height: f64) -> Result<Anchoring, FlowError> {
        let nozzle = self.widths.nozzle_diameter;
        let flow = |width| {
            Flow::from_widths(
                &self.widths,
                width,
                Flow::auto_width(nozzle),
                layer_height,
                false,
            )
        };
        let (outer, inner) = (
            flow(self.outer_wall_line_width)?,
            flow(self.inner_wall_line_width)?,
        );
        let solid = flow(self.internal_solid_infill_line_width)?;
        // With no walls there is nothing to reach past.
        let (shell, least) = if self.wall_loops > 0 {
            (
                0.5 * outer.width + outer.spacing + inner.spacing * (self.wall_loops - 1) as f64,
                inner.spacing,
            )
        } else {
            (1e-4, 1e-4)
        };
        Ok(Anchoring {
            into_sparse: scaled(least),
            into_solid: scaled(shell * std::f64::consts::SQRT_2),
            closing: scaled(0.55 * 0.65 * 1.05 * solid.spacing),
            minimum_sparse_area: self.minimum_sparse_infill_area,
        })
    }

    /// What filling a layer of this height needs.
    pub fn params(&self, layer_height: f64, first_layer: bool) -> Result<FillParams, FlowError> {
        let nozzle = self.widths.nozzle_diameter;
        let flow =
            |width, auto| Flow::from_widths(&self.widths, width, auto, layer_height, first_layer);
        let sparse = flow(self.sparse_infill_line_width, Flow::auto_width(nozzle))?;
        let solid = flow(
            self.internal_solid_infill_line_width,
            Flow::auto_width(nozzle),
        )?;
        let bridge = solid.bridging(self.bridge_flow, self.thick_bridges);
        let internal_bridge = solid.bridging(self.bridge_flow, self.thick_internal_bridges);
        Ok(FillParams {
            solid,
            bridge,
            bridge_spacing: bridge.spacing / self.bridge_density,
            internal_bridge,
            internal_bridge_spacing: internal_bridge.spacing / self.internal_bridge_density,
            // A top surface left unset is drawn at the nozzle's own width.
            top: flow(self.top_surface_line_width, nozzle)?,
            sparse,
            density: self.sparse_infill_density_percent / 100.0,
            pattern: self.pattern,
            sparse_angle: self.infill_direction.to_radians(),
            solid_angle: self.solid_infill_direction.to_radians(),
            sparse_link_max: self.infill_anchor_max.resolve(sparse.width).max(0.0),
            top_joined: self.top_joined,
            bottom_joined: self.bottom_joined,
            solid_joined: self.solid_joined,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_settings_keep_reference_defaults() {
        let settings = InfillSettings::from_settings(&Settings::default()).unwrap();
        assert_eq!(settings, InfillSettings::default());
        let params = settings.params(0.2, false).unwrap();
        assert_eq!(
            (params.solid.width, params.top.width, params.density),
            (0.45, 0.4, 0.2)
        );
        let shells = settings.shells(0.2).unwrap();
        assert_eq!(
            (shells.top_layers, shells.bottom_layers, shells.opening),
            (4, 3, 45_000)
        );
    }

    #[test]
    fn surface_patterns_say_whether_lines_are_joined() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // Absent, the reference's defaults: separate on top, joined below.
        let unset = read("{}").unwrap();
        assert_eq!(
            (unset.top_joined, unset.bottom_joined, unset.solid_joined),
            (false, true, true)
        );
        let set = read(
            r#"{"top_surface_pattern": "monotonic", "bottom_surface_pattern": "monotonicline",
                "internal_solid_infill_pattern": "rectilinear"}"#,
        )
        .unwrap();
        assert_eq!(
            (set.top_joined, set.bottom_joined, set.solid_joined),
            (true, false, true)
        );
        assert!(set.unsupported.is_empty());
        // A pattern not laid here is said, and laid as joined lines.
        let other = read(r#"{"top_surface_pattern": "concentric"}"#).unwrap();
        assert!(other.top_joined);
        assert!(other.unsupported[0].starts_with("top_surface_pattern \"concentric\""));
    }

    #[test]
    fn reads_shells_density_pattern_and_widths() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        let settings = read(
            r#"{"top_shell_layers": "5", "bottom_shell_layers": "0", "sparse_infill_density": "15%",
                "sparse_infill_pattern": "line", "infill_direction": "30", "top_surface_line_width": "0.42",
                "sparse_infill_line_width": "0.5", "nozzle_diameter": ["0.6"], "minimum_sparse_infill_area": "10"}"#,
        )
        .unwrap();
        assert_eq!(
            (settings.top_shell_layers, settings.bottom_shell_layers),
            (5, 0)
        );
        assert_eq!(
            (settings.pattern, settings.unsupported_pattern.as_deref()),
            (Pattern::Lines, None)
        );
        let params = settings.params(0.3, false).unwrap();
        assert_eq!(
            (params.top.width, params.sparse.width, params.density),
            (0.42, 0.5, 0.15)
        );
        assert!((params.solid.width - 0.675).abs() < 1e-12);
        assert!((params.sparse_angle - 30f64.to_radians()).abs() < 1e-12);

        let gyroid = read(r#"{"sparse_infill_pattern": "gyroid"}"#).unwrap();
        assert_eq!(
            (gyroid.pattern, gyroid.unsupported_pattern.as_deref()),
            (Pattern::Grid, Some("gyroid"))
        );
        assert!(read(r#"{"top_shell_layers": "-2"}"#).is_err());
        assert!(read(r#"{"sparse_infill_density": "lots"}"#).is_err());
    }

    #[test]
    fn bridge_settings_not_honoured_are_said() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        assert!(
            read(
                r#"{"bridge_angle": "0", "bridge_line_width": "0", "bridge_expansion_extra": "0"}"#
            )
            .unwrap()
            .unsupported
            .is_empty()
        );
        let set = read(r#"{"bridge_angle": "30", "bridge_line_width": "0.5"}"#).unwrap();
        assert_eq!(set.unsupported.len(), 2);
        assert!(set.unsupported[0].starts_with("bridge_angle \"30\""));
    }
}
