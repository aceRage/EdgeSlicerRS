use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{Flow, FlowError, LineWidths};
use edge_geom::scaled;

use crate::{FillParams, Shells};

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
    pub widths: LineWidths,
    /// Fill over air: its plastic as a share of a normal line's, whether
    /// it is a round thread instead of a flattened line, and how closely
    /// its lines are laid, from 0 to 1.
    pub bridge_flow: f64,
    pub thick_bridges: bool,
    pub bridge_density: f64,
    /// The same for solid fill laid over sparse fill.
    pub internal_bridge_flow: f64,
    pub thick_internal_bridges: bool,
    pub internal_bridge_density: f64,
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
            widths: LineWidths {
                line_width: unset,
                initial_layer_line_width: unset,
                nozzle_diameter: 0.4,
            },
            bridge_flow: 1.0,
            thick_bridges: false,
            bridge_density: 1.0,
            internal_bridge_flow: 1.0,
            thick_internal_bridges: true,
            internal_bridge_density: 1.0,
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
            internal_bridge_flow: number("internal_bridge_flow", defaults.internal_bridge_flow)?,
            thick_internal_bridges: flag(
                "thick_internal_bridges",
                defaults.thick_internal_bridges,
            )?,
            internal_bridge_density: share(
                "internal_bridge_density",
                defaults.internal_bridge_density,
            )?,
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
        let internal_bridge =
            solid.bridging(self.internal_bridge_flow, self.thick_internal_bridges);
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
}
