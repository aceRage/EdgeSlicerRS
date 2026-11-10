use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{Flow, FlowError, LineWidths};
use edge_geom::scaled;

use crate::Params;

/// The settings walls depend on, read once per print. Defaults are the
/// reference's (PrintConfig.cpp).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallSettings {
    pub wall_loops: usize,
    /// Line widths: zero means "not set", a percentage is of the nozzle diameter.
    pub outer_wall_line_width: FloatOrPercent,
    pub inner_wall_line_width: FloatOrPercent,
    pub internal_solid_infill_line_width: FloatOrPercent,
    /// Used wherever a more specific width is not set.
    pub line_width: FloatOrPercent,
    /// Replaces every width on the first layer when set.
    pub initial_layer_line_width: FloatOrPercent,
    pub nozzle_diameter: f64,
    pub infill_wall_overlap_percent: f64,
    /// Outline detail in mm below which vertices are dropped.
    pub resolution: f64,
    pub enable_arc_fitting: bool,
    /// Print the object's last layer with a single wall.
    pub only_one_wall_top: bool,
    /// Print the first layer with a single wall.
    pub only_one_wall_first_layer: bool,
    /// The one-wall options only apply where there is a solid shell behind
    /// the wall, as in the reference.
    pub top_shell_layers: usize,
    pub bottom_shell_layers: usize,
}

impl Default for WallSettings {
    fn default() -> Self {
        let unset = FloatOrPercent::Value(0.0);
        Self {
            wall_loops: 2,
            outer_wall_line_width: unset,
            inner_wall_line_width: unset,
            internal_solid_infill_line_width: unset,
            line_width: unset,
            initial_layer_line_width: unset,
            nozzle_diameter: 0.4,
            infill_wall_overlap_percent: 15.0,
            resolution: 0.01,
            enable_arc_fitting: false,
            only_one_wall_top: false,
            only_one_wall_first_layer: false,
            top_shell_layers: 4,
            bottom_shell_layers: 3,
        }
    }
}

impl WallSettings {
    /// Reads what is present; anything absent keeps its default. Lists are
    /// per extruder and the first extruder's entry is used.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let width = |key: &str, default| {
            settings
                .float_or_percent(key)
                .map(|found| found.unwrap_or(default))
        };
        let overlap = match settings.float_or_percent("infill_wall_overlap")? {
            // The setting is a percentage whether or not the sign is written.
            Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => percent,
            None => defaults.infill_wall_overlap_percent,
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
        Ok(Self {
            wall_loops: count("wall_loops", defaults.wall_loops)?,
            outer_wall_line_width: width("outer_wall_line_width", defaults.outer_wall_line_width)?,
            inner_wall_line_width: width("inner_wall_line_width", defaults.inner_wall_line_width)?,
            internal_solid_infill_line_width: width(
                "internal_solid_infill_line_width",
                defaults.internal_solid_infill_line_width,
            )?,
            line_width: width("line_width", defaults.line_width)?,
            initial_layer_line_width: width(
                "initial_layer_line_width",
                defaults.initial_layer_line_width,
            )?,
            nozzle_diameter: settings
                .floats("nozzle_diameter")?
                .and_then(|nozzles| nozzles.first().copied().flatten())
                .unwrap_or(defaults.nozzle_diameter),
            infill_wall_overlap_percent: overlap,
            resolution: settings.float("resolution")?.unwrap_or(defaults.resolution),
            enable_arc_fitting: settings
                .flag("enable_arc_fitting")?
                .unwrap_or(defaults.enable_arc_fitting),
            only_one_wall_top: settings
                .flag("only_one_wall_top")?
                .unwrap_or(defaults.only_one_wall_top),
            only_one_wall_first_layer: settings
                .flag("only_one_wall_first_layer")?
                .unwrap_or(defaults.only_one_wall_first_layer),
            top_shell_layers: count("top_shell_layers", defaults.top_shell_layers)?,
            bottom_shell_layers: count("bottom_shell_layers", defaults.bottom_shell_layers)?,
        })
    }

    /// What the generator needs for layer `index` of `layer_count`, which
    /// adds to [`Self::params`] the rules that depend on where a layer is.
    /// Only the object's very last layer gets the single top wall; top
    /// surfaces lower down keep all their walls, where the reference thins
    /// the walls over each of them (`split_top_surfaces`).
    pub fn params_at(
        &self,
        layer_height: f64,
        index: usize,
        layer_count: usize,
    ) -> Result<Params, FlowError> {
        let mut params = self.params(layer_height, index == 0)?;
        let one_wall = (index == 0
            && self.only_one_wall_first_layer
            && self.bottom_shell_layers > 0)
            || (index + 1 == layer_count && self.only_one_wall_top && self.top_shell_layers > 0);
        if one_wall {
            params.wall_loops = params.wall_loops.min(1);
        }
        Ok(params)
    }

    /// What the generator needs for a layer of this height.
    pub fn params(&self, layer_height: f64, first_layer: bool) -> Result<Params, FlowError> {
        let outer = self.flow(self.outer_wall_line_width, layer_height, first_layer)?;
        let inner = self.flow(self.inner_wall_line_width, layer_height, first_layer)?;
        let solid_infill = self.flow(
            self.internal_solid_infill_line_width,
            layer_height,
            first_layer,
        )?;
        // Arc fitting needs the detail that simplifying would remove.
        let resolution =
            self.resolution.max(1e-4) * if self.enable_arc_fitting { 0.2 } else { 1.0 };
        Ok(Params {
            wall_loops: self.wall_loops,
            outer,
            inner,
            solid_infill_spacing: solid_infill.scaled_spacing(),
            infill_overlap_percent: self.infill_wall_overlap_percent,
            resolution: scaled(resolution),
        })
    }

    fn flow(
        &self,
        role_width: FloatOrPercent,
        layer_height: f64,
        first_layer: bool,
    ) -> Result<Flow, FlowError> {
        let widths = LineWidths {
            line_width: self.line_width,
            initial_layer_line_width: self.initial_layer_line_width,
            nozzle_diameter: self.nozzle_diameter,
        };
        Flow::from_widths(
            &widths,
            role_width,
            Flow::auto_width(self.nozzle_diameter),
            layer_height,
            first_layer,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_settings_keep_reference_defaults() {
        let settings = WallSettings::from_settings(&Settings::default()).unwrap();
        assert_eq!(settings, WallSettings::default());
        let params = settings.params(0.2, false).unwrap();
        assert_eq!(params.wall_loops, 2);
        assert_eq!(params.outer.width, 0.45);
        assert_eq!(params.resolution, 10_000);
    }

    #[test]
    fn widths_fall_back_from_role_to_general_to_nozzle() {
        let settings = WallSettings::from_settings(
            &Settings::from_json(
                r#"{"wall_loops": "3", "outer_wall_line_width": "0.42", "inner_wall_line_width": "0",
                    "line_width": "110%", "internal_solid_infill_line_width": "0",
                    "initial_layer_line_width": "0.5", "nozzle_diameter": ["0.6", "0.4"],
                    "infill_wall_overlap": "25%", "resolution": "0.012", "enable_arc_fitting": "1"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let params = settings.params(0.2, false).unwrap();
        assert_eq!((params.wall_loops, params.outer.width), (3, 0.42));
        assert!((params.inner.width - 0.66).abs() < 1e-12);
        assert_eq!(params.infill_overlap_percent, 25.0);
        assert_eq!(params.resolution, 2_400);

        let first = settings.params(0.25, true).unwrap();
        assert_eq!(
            (first.outer.width, first.inner.width, first.outer.height),
            (0.5, 0.5, 0.25)
        );

        let auto = WallSettings {
            nozzle_diameter: 0.8,
            ..WallSettings::default()
        };
        assert!((auto.params(0.3, false).unwrap().outer.width - 0.9).abs() < 1e-12);
    }

    #[test]
    fn end_layers_can_be_held_to_one_wall() {
        let settings = WallSettings::from_settings(
            &Settings::from_json(
                r#"{"wall_loops": "3", "only_one_wall_top": "1", "only_one_wall_first_layer": "1",
                    "top_shell_layers": "4", "bottom_shell_layers": "0"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let walls = |index| settings.params_at(0.2, index, 10).unwrap().wall_loops;
        // No bottom shell, so the first layer keeps its walls; the last has a top shell.
        assert_eq!((walls(0), walls(5), walls(9)), (3, 3, 1));
        let off = WallSettings {
            wall_loops: 3,
            ..WallSettings::default()
        };
        assert_eq!(off.params_at(0.2, 9, 10).unwrap().wall_loops, 3);
        let none = WallSettings {
            wall_loops: 0,
            only_one_wall_top: true,
            ..WallSettings::default()
        };
        assert_eq!(none.params_at(0.2, 9, 10).unwrap().wall_loops, 0);
    }

    #[test]
    fn bad_values_are_errors() {
        let read = |json: &str| WallSettings::from_settings(&Settings::from_json(json).unwrap());
        assert!(read(r#"{"wall_loops": "-1"}"#).is_err());
        assert!(read(r#"{"wall_loops": "two"}"#).is_err());
        assert!(read(r#"{"line_width": "wide"}"#).is_err());
        let thin = WallSettings {
            line_width: FloatOrPercent::Value(0.03),
            ..WallSettings::default()
        };
        assert!(thin.params(0.2, false).is_err());
    }
}
