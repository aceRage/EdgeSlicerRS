use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{Flow, FlowError, LineWidths};
use edge_geom::{Coord, scaled};

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
    /// The same on the first layer and the last, where the fill is a
    /// bottom or a top surface (`top_bottom_infill_wall_overlap`).
    pub top_bottom_infill_wall_overlap_percent: f64,
    /// Outline detail in mm below which vertices are dropped.
    pub resolution: f64,
    pub enable_arc_fitting: bool,
    /// Print the object's last layer with a single wall.
    pub only_one_wall_top: bool,
    /// Print the first layer with a single wall.
    pub only_one_wall_first_layer: bool,
    /// A top surface narrower than this keeps all its walls, in mm or as
    /// a share of the inner wall's width.
    pub min_width_top_surface: FloatOrPercent,
    /// The sparse infill's line width; zero is the nozzle's own.
    pub sparse_infill_line_width: FloatOrPercent,
    /// The one-wall options only apply where there is a solid shell behind
    /// the wall, as in the reference.
    pub top_shell_layers: usize,
    pub bottom_shell_layers: usize,
    /// Gaps between walls are filled when this speed, in mm/s, is above
    /// zero; the reference has no other switch for it.
    pub gap_infill_speed: f64,
    /// Gap lines shorter than this many mm are left out.
    pub filter_out_gap_fill: f64,
    /// Set where walls are of changing width (`wall_generator` at
    /// `arachne`, which is the reference's default); none for `classic`.
    pub arachne: Option<ArachneSettings>,
}

/// What `wall_generator = arachne` is tuned by. The percentages are of
/// the nozzle's diameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArachneSettings {
    pub min_bead_width_percent: f64,
    pub initial_layer_min_bead_width_percent: f64,
    pub min_feature_size_percent: f64,
    pub min_length_factor: f64,
    pub wall_transition_length_percent: f64,
    /// In degrees.
    pub wall_transition_angle: f64,
    pub wall_transition_filter_deviation_percent: f64,
    pub wall_distribution_count: i32,
    /// In mm.
    pub wall_maximum_resolution: f64,
    pub wall_maximum_deviation: f64,
}

impl Default for ArachneSettings {
    fn default() -> Self {
        Self {
            min_bead_width_percent: 85.0,
            initial_layer_min_bead_width_percent: 85.0,
            min_feature_size_percent: 25.0,
            min_length_factor: 0.5,
            wall_transition_length_percent: 100.0,
            wall_transition_angle: 10.0,
            wall_transition_filter_deviation_percent: 25.0,
            wall_distribution_count: 1,
            wall_maximum_resolution: 0.5,
            wall_maximum_deviation: 0.025,
        }
    }
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
            top_bottom_infill_wall_overlap_percent: 25.0,
            resolution: 0.01,
            enable_arc_fitting: false,
            only_one_wall_top: false,
            only_one_wall_first_layer: false,
            min_width_top_surface: FloatOrPercent::Percent(300.0),
            sparse_infill_line_width: FloatOrPercent::Value(0.0),
            top_shell_layers: 4,
            bottom_shell_layers: 3,
            gap_infill_speed: 30.0,
            filter_out_gap_fill: 0.0,
            arachne: Some(ArachneSettings::default()),
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
        let outside_overlap = match settings.float_or_percent("top_bottom_infill_wall_overlap")? {
            Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => percent,
            None => defaults.top_bottom_infill_wall_overlap_percent,
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
        // A percentage whether or not the sign is written.
        let percent = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(match settings.float_or_percent(key)? {
                Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => percent,
                None => default,
            })
        };
        let number = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(settings.float(key)?.unwrap_or(default))
        };
        let arachne = match settings.text("wall_generator") {
            Some("classic") => None,
            Some("arachne") | None => {
                let defaults = ArachneSettings::default();
                Some(ArachneSettings {
                    min_bead_width_percent: percent(
                        "min_bead_width",
                        defaults.min_bead_width_percent,
                    )?,
                    initial_layer_min_bead_width_percent: percent(
                        "initial_layer_min_bead_width",
                        defaults.initial_layer_min_bead_width_percent,
                    )?,
                    min_feature_size_percent: percent(
                        "min_feature_size",
                        defaults.min_feature_size_percent,
                    )?,
                    min_length_factor: number("min_length_factor", defaults.min_length_factor)?,
                    wall_transition_length_percent: percent(
                        "wall_transition_length",
                        defaults.wall_transition_length_percent,
                    )?,
                    wall_transition_angle: number(
                        "wall_transition_angle",
                        defaults.wall_transition_angle,
                    )?,
                    wall_transition_filter_deviation_percent: percent(
                        "wall_transition_filter_deviation",
                        defaults.wall_transition_filter_deviation_percent,
                    )?,
                    wall_distribution_count: settings
                        .integer("wall_distribution_count")?
                        .map_or(defaults.wall_distribution_count, |count| count as i32),
                    wall_maximum_resolution: number(
                        "wall_maximum_resolution",
                        defaults.wall_maximum_resolution,
                    )?,
                    wall_maximum_deviation: number(
                        "wall_maximum_deviation",
                        defaults.wall_maximum_deviation,
                    )?,
                })
            }
            Some(other) => {
                return Err(ConfigError::BadValue {
                    key: "wall_generator".into(),
                    found: other.to_owned(),
                    expected: "classic or arachne",
                });
            }
        };
        Ok(Self {
            arachne,
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
            top_bottom_infill_wall_overlap_percent: outside_overlap,
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
            min_width_top_surface: settings
                .float_or_percent("min_width_top_surface")?
                .unwrap_or(defaults.min_width_top_surface),
            sparse_infill_line_width: settings
                .float_or_percent("sparse_infill_line_width")?
                .unwrap_or(defaults.sparse_infill_line_width),
            top_shell_layers: count("top_shell_layers", defaults.top_shell_layers)?,
            bottom_shell_layers: count("bottom_shell_layers", defaults.bottom_shell_layers)?,
            gap_infill_speed: settings
                .floats("gap_infill_speed")?
                .and_then(|speeds| speeds.first().copied().flatten())
                .unwrap_or(defaults.gap_infill_speed),
            filter_out_gap_fill: settings
                .float("filter_out_gap_fill")?
                .unwrap_or(defaults.filter_out_gap_fill),
        })
    }

    /// What the settings ask of the walls that is not done, each with
    /// what is done instead. Nothing at present.
    pub fn notes(&self) -> Vec<String> {
        Vec::new()
    }

    /// What the generator needs for layer `index` of `layer_count`, which
    /// adds to [`Self::params`] the rules that depend on where a layer is.
    /// The object's very last layer gets the single top wall all over; a
    /// layer below it is told how to thin the walls over whatever top
    /// surface it has ([`crate::OneWallTop`]).
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
        // The first layer and the last overlap their walls by the amount
        // set for top and bottom surfaces (PerimeterGenerator.cpp, where
        // `layer_id == 0` or there are no slices above).
        if index == 0 || index + 1 == layer_count {
            params.infill_overlap_percent = self.top_bottom_infill_wall_overlap_percent;
        }
        // Below the last layer a top surface keeps only its outer wall,
        // where there is a top shell to make one of it.
        if let Some(arachne) = &mut params.arachne {
            arachne.is_top_or_bottom_layer = index == 0 || index + 1 == layer_count;
        }
        // Below the last layer a top surface keeps only its outer wall,
        // where there is a top shell to make one of it.
        if self.only_one_wall_top && self.top_shell_layers > 0 && index + 1 < layer_count {
            let infill = self.sparse_infill_line_width.resolve(self.nozzle_diameter);
            let infill = if infill > 0.0 {
                infill
            } else {
                Flow::auto_width(self.nozzle_diameter)
            };
            params.one_wall_top = Some(crate::OneWallTop {
                min_width: scaled(self.min_width_top_surface.resolve(params.inner.width)),
                infill_width: scaled(infill),
                // libslic3r.h `BRIDGE_INFILL_MARGIN`, 1 mm on a 0.4 mm nozzle.
                bridge_margin: scaled(1f64.min(self.nozzle_diameter / 0.4)),
                wall_loops: self.wall_loops,
                overlap_percent: self.top_bottom_infill_wall_overlap_percent,
            });
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
            gap_fill: (self.gap_infill_speed > 0.0).then_some(crate::GapFill {
                flow: solid_infill,
                shortest: scaled(self.filter_out_gap_fill),
            }),
            one_wall_top: None,
            arachne: self.arachne.map(|arachne| {
                // Of the nozzle's diameter, and kept in single precision,
                // as the reference keeps them (`make_paths_params`).
                let of_nozzle = |percent: f64| (percent * 0.01 * self.nozzle_diameter) as f32;
                crate::ArachneParams {
                    min_bead_width: of_nozzle(if first_layer {
                        arachne.initial_layer_min_bead_width_percent
                    } else {
                        arachne.min_bead_width_percent
                    }),
                    min_feature_size: of_nozzle(arachne.min_feature_size_percent),
                    min_length_factor: arachne.min_length_factor as f32,
                    wall_transition_length: of_nozzle(arachne.wall_transition_length_percent),
                    wall_transition_angle: arachne.wall_transition_angle as f32,
                    wall_transition_filter_deviation: of_nozzle(
                        arachne.wall_transition_filter_deviation_percent,
                    ),
                    wall_distribution_count: arachne.wall_distribution_count,
                    is_top_or_bottom_layer: false,
                    wall_maximum_resolution: (arachne.wall_maximum_resolution / 1e-6) as Coord,
                    wall_maximum_deviation: (arachne.wall_maximum_deviation / 1e-6) as Coord,
                }
            }),
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
    fn end_layers_overlap_their_walls_by_the_surface_amount() {
        let settings = WallSettings::from_settings(
            &Settings::from_json(
                r#"{"infill_wall_overlap": "15%", "top_bottom_infill_wall_overlap": "30%"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let overlap = |index| {
            settings
                .params_at(0.2, index, 10)
                .unwrap()
                .infill_overlap_percent
        };
        assert_eq!(
            [overlap(0), overlap(1), overlap(8), overlap(9)],
            [30.0, 15.0, 15.0, 30.0]
        );
        // Left unset it is the reference's quarter.
        let unset = WallSettings::default();
        assert_eq!(
            unset.params_at(0.2, 0, 10).unwrap().infill_overlap_percent,
            25.0
        );
    }

    #[test]
    fn layers_below_the_last_are_told_how_to_thin_walls_under_a_top() {
        let settings = WallSettings::from_settings(
            &Settings::from_json(
                r#"{"only_one_wall_top": "1", "top_shell_layers": "4", "wall_loops": "3",
                    "wall_generator": "classic",
                    "min_width_top_surface": "200%", "inner_wall_line_width": "0.45",
                    "sparse_infill_line_width": "0.5", "nozzle_diameter": ["0.4"],
                    "top_bottom_infill_wall_overlap": "25%"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let top = settings
            .params_at(0.2, 3, 10)
            .unwrap()
            .one_wall_top
            .unwrap();
        assert_eq!(top.min_width, scaled(0.9));
        assert_eq!(top.infill_width, scaled(0.5));
        assert_eq!((top.wall_loops, top.overlap_percent), (3, 25.0));
        // The last layer is one wall all over, with nothing to split.
        let last = settings.params_at(0.2, 9, 10).unwrap();
        assert_eq!((last.wall_loops, last.one_wall_top), (1, None));
        // Off, or with no top shell to make of it, nothing is thinned.
        let off = WallSettings {
            only_one_wall_top: false,
            ..settings
        };
        assert_eq!(off.params_at(0.2, 3, 10).unwrap().one_wall_top, None);
        let no_shell = WallSettings {
            top_shell_layers: 0,
            ..settings
        };
        assert_eq!(no_shell.params_at(0.2, 3, 10).unwrap().one_wall_top, None);
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
    fn the_wall_generator_and_what_tunes_it_are_read() {
        let read = |json: &str| WallSettings::from_settings(&Settings::from_json(json).unwrap());
        // Walls of changing width unless classic ones are asked for.
        assert_eq!(
            read("{}").unwrap().arachne,
            Some(ArachneSettings::default())
        );
        assert_eq!(
            read(r#"{"wall_generator": "classic"}"#).unwrap().arachne,
            None
        );
        assert!(read(r#"{"wall_generator": "spiral"}"#).is_err());
        let set = read(
            r#"{"wall_generator": "arachne", "min_bead_width": "70%", "initial_layer_min_bead_width": "90%",
                "min_feature_size": "20%", "wall_transition_length": "150%", "wall_transition_angle": "15",
                "wall_transition_filter_deviation": "30%", "wall_distribution_count": "2",
                "min_length_factor": "0.7", "wall_maximum_resolution": "0.3", "wall_maximum_deviation": "0.02",
                "nozzle_diameter": ["0.6"]}"#,
        )
        .unwrap();
        // Shares of the nozzle's diameter, the first layer with its own.
        let (first, later) = (
            set.params(0.2, true).unwrap().arachne.unwrap(),
            set.params(0.2, false).unwrap().arachne.unwrap(),
        );
        assert!((first.min_bead_width - 0.54).abs() < 1e-6);
        assert!((later.min_bead_width - 0.42).abs() < 1e-6);
        assert!((later.min_feature_size - 0.12).abs() < 1e-6);
        assert!((later.wall_transition_length - 0.9).abs() < 1e-6);
        assert!((later.wall_transition_filter_deviation - 0.18).abs() < 1e-6);
        assert_eq!(
            (
                later.wall_transition_angle,
                later.wall_distribution_count,
                later.min_length_factor
            ),
            (15.0, 2, 0.7)
        );
        assert_eq!(
            (later.wall_maximum_resolution, later.wall_maximum_deviation),
            (300_000, 20_000)
        );
        // The first layer and the last are told what they are.
        let ends = |index| {
            set.params_at(0.2, index, 10)
                .unwrap()
                .arachne
                .unwrap()
                .is_top_or_bottom_layer
        };
        assert_eq!((ends(0), ends(4), ends(9)), (true, false, true));
        assert!(
            read(r#"{"wall_generator": "classic"}"#)
                .unwrap()
                .params(0.2, false)
                .unwrap()
                .arachne
                .is_none()
        );

        // One wall over top surfaces is told to these walls as to the
        // classic ones.
        let asked = read(r#"{"only_one_wall_top": "1"}"#).unwrap();
        assert!(asked.notes().is_empty());
        assert!(asked.params_at(0.2, 4, 10).unwrap().one_wall_top.is_some());
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
