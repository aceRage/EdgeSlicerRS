use std::f64::consts::PI;

use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{Flow, FlowError, LineWidths};
use edge_geom::{Coord, scaled};

use crate::{
    Anchoring, BridgeDirection, FillParams, InternalBridgeFilter, NarrowFill, OverInfill, Shells,
    WallsAround,
};

/// Sparse fill patterns this engine can lay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// One set of parallel lines, turned 90° each layer (`rectilinear`).
    Lines,
    /// One set of parallel lines, the same way on every layer
    /// (`alignedrectilinear`).
    AlignedLines,
    /// Two crossing sets of lines on every layer.
    Grid,
    /// Three sets of lines 60° apart, all through the same points.
    Triangles,
    /// Three sets of lines 60° apart, the third moved half its pitch,
    /// leaving hexagons between triangles (`tri-hexagon`).
    Stars,
    /// Three sets of lines 60° apart that move sideways as the layers go
    /// up, making cubes stood on a corner.
    Cubic,
    /// Two sets of lines square to each other, each doubled and moving
    /// apart and together as the layers go up (`quartercubic`).
    QuarterCubic,
    /// Straight lines one way for some layers, then the other, turning
    /// from one to the other through zigzags (`crosshatch`).
    CrossHatch,
    /// Waves that swing between two directions as the layers go up.
    Gyroid,
    /// Hexagons, turned a third of the way round on each layer.
    Honeycomb,
    /// Two sets of lines the same way that lean against each other as
    /// the layers go up (`lateral-lattice`).
    LateralLattice,
    /// Zigzags along one axis, then the other, that open and close as
    /// the layers go up: cells with eight sides and six (`3dhoneycomb`).
    Honeycomb3D,
    /// Honeycomb stood on edge: one set of lines that splits in two and
    /// closes again as the layers go up (`lateral-honeycomb`).
    LateralHoneycomb,
    /// Waves along the diagonals that open and close as the layers go up,
    /// after the Schwarz D surface (`tpmsd`).
    TpmsD,
    /// One line that visits every square of a grid over the object
    /// (`hilbertcurve`).
    Hilbert,
    /// One spiral out from the middle of the object, in short chords
    /// (`archimedeanchords`).
    Archimedean,
    /// One spiral of eight-pointed stars out from the middle of the
    /// object (`octagramspiral`).
    Octagram,
    /// Lines turned 90° each layer, joined on the same side layer after
    /// layer (`zigzag`).
    ZigZag,
    /// Lines turned 90° each layer, each direction's moved a step
    /// sideways every second layer (`crosszag`).
    CrossZag,
}

/// Which bridges get a second layer of bridge laid over them
/// (`enable_extra_bridge_layer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraBridgeLayer {
    /// `disabled`, the default.
    None,
    /// `external_bridge_only`: bridges over air.
    External,
    /// `internal_bridge_only`: bridges over infill.
    Internal,
    /// `apply_to_all`.
    All,
}

impl ExtraBridgeLayer {
    /// Whether bridges over air get one.
    pub fn external(self) -> bool {
        matches!(self, Self::External | Self::All)
    }

    /// Whether bridges over infill get one.
    pub fn internal(self) -> bool {
        matches!(self, Self::Internal | Self::All)
    }
}

/// The settings infill depends on, read once per print. Defaults are the
/// reference's (PrintConfig.cpp).
#[derive(Clone, Debug, PartialEq)]
pub struct InfillSettings {
    pub top_shell_layers: usize,
    pub bottom_shell_layers: usize,
    /// The least a top shell and a bottom shell are to measure, in mm
    /// (`top_shell_thickness`, `bottom_shell_thickness`): layers are
    /// added to the count until it is reached. Zero leaves it to the
    /// count, and so does a count of zero, which is no shell at all.
    pub top_shell_thickness: f64,
    pub bottom_shell_thickness: f64,
    pub sparse_infill_density_percent: f64,
    pub pattern: Pattern,
    /// The configured sparse pattern when it is one not implemented here,
    /// in which case `pattern` is the grid used in its place.
    pub unsupported_pattern: Option<String>,
    /// Degrees counter-clockwise from +x.
    pub infill_direction: f64,
    pub solid_infill_direction: f64,
    /// Whether the fill keeps one direction on every layer rather than
    /// turning a quarter on each odd one: Z contouring's
    /// `zaa_dont_alternate_fill_direction`, which counts only with
    /// `zaa_enabled` (Fill.cpp `Layer::make_fills`, FillBase.cpp
    /// `_infill_direction`).
    pub steady_direction: bool,
    /// How far the two sets of a lateral lattice lean, in degrees
    /// (`lateral_lattice_angle_1` and `_2`).
    pub lateral_lattice_angles: (f64, f64),
    /// How far a cross zag's lines move sideways every second layer, in
    /// mm (`infill_shift_step`).
    pub infill_shift_step: f64,
    /// How steeply a lateral honeycomb's walls lean, in degrees from
    /// upright (`infill_overhang_angle`).
    pub infill_overhang_angle: f64,
    /// In mm².
    pub minimum_sparse_infill_area: f64,
    /// Longest stretch of wall two sparse lines are joined along; a
    /// percentage is of the sparse line width, and zero joins nothing.
    pub infill_anchor_max: FloatOrPercent,
    /// How far a sparse line's end runs along the wall when it is joined
    /// to no other, in mm or as a share of the line spacing.
    pub infill_anchor: FloatOrPercent,
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
    /// How much further a bridge reaches into solid fill and into a top
    /// or bottom surface beside it than a surface otherwise does, in mm
    /// (`bridge_expansion_extra`).
    pub bridge_expansion_extra: f64,
    /// The width set for bridges and bridges over infill, in mm
    /// (`bridge_line_width`); none leaves a round thread as thick as the
    /// nozzle's bore and a flattened line as wide as solid fill.
    pub bridge_line_width: Option<f64>,
    /// Solid fill laid over sparse fill: whether it is a round thread,
    /// and how closely its lines are laid. Its thread is sized by
    /// `bridge_flow`, as the reference sizes it; `internal_bridge_flow` is
    /// applied to the plastic when the G-code is written.
    pub thick_internal_bridges: bool,
    pub internal_bridge_density: f64,
    /// How small bridges over infill are sorted out
    /// (`dont_filter_internal_bridges`).
    pub internal_bridge_filter: InternalBridgeFilter,
    /// The direction bridges over infill are laid in, in degrees, where
    /// it is not zero (`internal_bridge_angle`); with
    /// `relative_bridge_angle` it turns the direction found instead.
    pub internal_bridge_angle: f64,
    pub relative_bridge_angle: bool,
    /// Which bridges get a second layer (`enable_extra_bridge_layer`).
    pub extra_bridge_layer: ExtraBridgeLayer,
    /// Whether top surface, bottom surface and internal solid lines are
    /// joined end to end (`top_surface_pattern`, `bottom_surface_pattern`,
    /// `internal_solid_infill_pattern`): `monotonicline` leaves them
    /// separate, `monotonic` and `rectilinear` join them.
    pub top_joined: bool,
    pub bottom_joined: bool,
    pub solid_joined: bool,
    /// Whether each is swept from one side to the other, no line laid
    /// between two already down: so for `monotonic` and `monotonicline`,
    /// not for `rectilinear`.
    pub top_ordered: bool,
    pub bottom_ordered: bool,
    pub solid_ordered: bool,
    /// Which solid fill has what its lines leave bare filled with lines
    /// of whatever width fits (`gap_fill_target`), and the shortest such
    /// line worth laying, in mm (`filter_out_gap_fill`).
    pub gap_fill_target: crate::GapFillTarget,
    pub filter_out_gap_fill: f64,
    /// Whether fill under and over the walls of nearby layers is made
    /// solid (`ensure_vertical_shell_thickness` at `ensure_all`).
    pub ensure_vertical_shells: bool,
    /// How closely solid fill inside the part is laid on the second
    /// layer, from 0 to 1 (`elefant_foot_layers_density`), and how many
    /// layers from there it takes to come back to full
    /// (`elefant_foot_compensation_layers`, which is also how many
    /// layers are shrunk for the elephant's foot).
    pub elephant_foot_density: f64,
    pub elephant_foot_layers: usize,
    /// Whether the narrow parts of solid fill inside the part are told
    /// from the rest and filled with loops along their outline
    /// (`detect_narrow_internal_solid_infill`).
    pub detect_narrow_solid: bool,
    /// How far short of where it began such a loop stops, in mm or as a
    /// share of the nozzle (`seam_gap`), and the smallest nozzle on the
    /// printer, in mm, which sets how thin a loop may be.
    pub seam_gap: FloatOrPercent,
    pub smallest_nozzle: f64,
    /// Settings present in the input that are not honoured, each with
    /// what is done instead.
    pub unsupported: Vec<String>,
}

/// Whether fill keeps one direction on every layer: with Z contouring on
/// and `zaa_dont_alternate_fill_direction` set (Fill.cpp, where each
/// filler's `dont_alternate_fill_direction` is set from the two). Only
/// the turn from layer to layer goes; a bridge keeps its own direction.
pub fn steady_direction(settings: &Settings) -> Result<bool, ConfigError> {
    Ok(settings.flag("zaa_enabled")?.unwrap_or(false)
        && settings
            .flag("zaa_dont_alternate_fill_direction")?
            .unwrap_or(false))
}

impl Default for InfillSettings {
    fn default() -> Self {
        let unset = FloatOrPercent::Value(0.0);
        Self {
            top_shell_layers: 4,
            bottom_shell_layers: 3,
            top_shell_thickness: 0.6,
            bottom_shell_thickness: 0.0,
            sparse_infill_density_percent: 20.0,
            pattern: Pattern::CrossHatch,
            unsupported_pattern: None,
            infill_direction: 45.0,
            solid_infill_direction: 45.0,
            steady_direction: false,
            lateral_lattice_angles: (-45.0, 45.0),
            infill_shift_step: 0.4,
            infill_overhang_angle: 60.0,
            minimum_sparse_infill_area: 15.0,
            infill_anchor_max: FloatOrPercent::Value(20.0),
            infill_anchor: FloatOrPercent::Percent(400.0),
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
            bridge_expansion_extra: 0.0,
            bridge_line_width: None,
            thick_internal_bridges: true,
            internal_bridge_density: 1.0,
            internal_bridge_filter: InternalBridgeFilter::Filter,
            internal_bridge_angle: 0.0,
            relative_bridge_angle: false,
            extra_bridge_layer: ExtraBridgeLayer::None,
            top_ordered: true,
            bottom_ordered: true,
            solid_ordered: true,
            gap_fill_target: crate::GapFillTarget::Nowhere,
            filter_out_gap_fill: 0.0,
            top_joined: false,
            bottom_joined: true,
            solid_joined: true,
            ensure_vertical_shells: true,
            elephant_foot_density: 1.0,
            elephant_foot_layers: 1,
            detect_narrow_solid: true,
            seam_gap: FloatOrPercent::Percent(10.0),
            smallest_nozzle: 0.4,
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
        // From nothing up, as the reference allows.
        let thickness = |key: &str, default: f64| -> Result<f64, ConfigError> {
            match settings.float(key)? {
                Some(found) if found.is_nan() || found < 0.0 => Err(ConfigError::BadValue {
                    key: key.into(),
                    found: found.to_string(),
                    expected: "zero or more",
                }),
                found => Ok(found.unwrap_or(default)),
            }
        };
        let (pattern, unsupported_pattern) = match settings
            .texts("sparse_infill_pattern")
            .and_then(|p| p.first())
        {
            None => (defaults.pattern, defaults.unsupported_pattern),
            Some(name) => match name.as_str() {
                "grid" => (Pattern::Grid, None),
                "line" | "rectilinear" | "zig-zag" => (Pattern::Lines, None),
                "alignedrectilinear" => (Pattern::AlignedLines, None),
                "triangles" => (Pattern::Triangles, None),
                "tri-hexagon" => (Pattern::Stars, None),
                "cubic" => (Pattern::Cubic, None),
                "quartercubic" => (Pattern::QuarterCubic, None),
                "crosshatch" => (Pattern::CrossHatch, None),
                "gyroid" => (Pattern::Gyroid, None),
                "honeycomb" => (Pattern::Honeycomb, None),
                "lateral-lattice" => (Pattern::LateralLattice, None),
                "zigzag" => (Pattern::ZigZag, None),
                "3dhoneycomb" => (Pattern::Honeycomb3D, None),
                "lateral-honeycomb" => (Pattern::LateralHoneycomb, None),
                "tpmsd" => (Pattern::TpmsD, None),
                "hilbertcurve" => (Pattern::Hilbert, None),
                "archimedeanchords" => (Pattern::Archimedean, None),
                "octagramspiral" => (Pattern::Octagram, None),
                "crosszag" => (Pattern::CrossZag, None),
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
        if let Some(FloatOrPercent::Value(value) | FloatOrPercent::Percent(value)) =
            settings.float_or_percent("bridge_angle")?
            && value != 0.0
        {
            unsupported.push(format!(
                "bridge_angle \"{value}\": each bridge is turned to span its own gap"
            ));
        }
        // What the profile asks of a sparse pattern that is not done.
        if settings
            .integer("fill_multiline")?
            .is_some_and(|lines| lines > 1)
        {
            unsupported
                .push("fill_multiline: each line of a sparse pattern is laid once".to_owned());
        }
        if pattern == Pattern::Gyroid && settings.flag("gyroid_optimized")? == Some(true) {
            unsupported
                .push("gyroid_optimized \"1\": the gyroid is laid as plain waves".to_owned());
        }
        if settings
            .text("sparse_infill_rotate_template")
            .is_some_and(|template| !template.trim().is_empty())
        {
            unsupported.push(
                "sparse_infill_rotate_template: sparse fill keeps to infill_direction".to_owned(),
            );
        }
        if pattern == Pattern::Honeycomb3D && settings.flag("infill_complete_top")? == Some(true) {
            unsupported.push(
                "infill_complete_top \"1\": the tops of 3D honeycomb cells are left open"
                    .to_owned(),
            );
        }
        if matches!(pattern, Pattern::ZigZag | Pattern::CrossZag)
            && settings.flag("symmetric_infill_y_axis")? == Some(true)
        {
            unsupported.push(
                "symmetric_infill_y_axis \"1\": the zigzag is not mirrored about the object's middle"
                    .to_owned(),
            );
        }
        if settings.text("sparse_infill_pattern") == Some("line") {
            unsupported.push(
                "sparse_infill_pattern \"line\": laid as rectilinear, joined along the walls"
                    .to_owned(),
            );
        }
        // Straight lines are all this engine lays on a surface; any other
        // pattern is laid as joined lines and said so.
        // (joined, swept in order)
        let mut joined = |key: &str, default: (bool, bool)| match settings
            .texts(key)
            .and_then(|pattern| pattern.first())
            .map(String::as_str)
        {
            None => default,
            Some("monotonicline") => (false, true),
            Some("monotonic") => (true, true),
            Some("rectilinear") => (true, false),
            Some(other) => {
                unsupported.push(format!(
                    "{key} \"{other}\": laid as straight lines joined at their ends"
                ));
                (true, false)
            }
        };
        let (top_joined, top_ordered) = joined(
            "top_surface_pattern",
            (defaults.top_joined, defaults.top_ordered),
        );
        let (bottom_joined, bottom_ordered) = joined(
            "bottom_surface_pattern",
            (defaults.bottom_joined, defaults.bottom_ordered),
        );
        let (solid_joined, solid_ordered) = joined(
            "internal_solid_infill_pattern",
            (defaults.solid_joined, defaults.solid_ordered),
        );
        // The reference has a different way of laying shells for each
        // other value; only this one is ported. Bambu Studio's projects
        // say "enabled" for it, and older ones "1", which the reference
        // reads as `ensure_all` (Format/BambuKeyAliases.cpp
        // `make_enum_aliases`, PrintConfig.cpp `handle_legacy`).
        let ensure_vertical_shells = match settings
            .texts("ensure_vertical_shell_thickness")
            .and_then(|found| found.first())
            .map(String::as_str)
        {
            None | Some("ensure_all" | "enabled" | "1") => true,
            Some(other) => {
                unsupported.push(format!(
                    "ensure_vertical_shell_thickness \"{other}\": shells are the layers under a top and over a bottom, and no more"
                ));
                false
            }
        };
        // From half to all of it: the reference will not slice a project
        // that says anything else ("not in range [50,100]").
        let elephant_foot_density =
            match settings.float_or_percent("elefant_foot_layers_density")? {
                Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => {
                    if !(50.0..=100.0).contains(&percent) {
                        return Err(ConfigError::BadValue {
                            key: "elefant_foot_layers_density".into(),
                            found: percent.to_string(),
                            expected: "from 50% to 100%",
                        });
                    }
                    percent / 100.0
                }
                None => defaults.elephant_foot_density,
            };
        let density = match settings.float_or_percent("sparse_infill_density")? {
            // A percentage whether or not the sign is written.
            Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => percent,
            None => defaults.sparse_infill_density_percent,
        };
        Ok(Self {
            top_shell_layers: count("top_shell_layers", defaults.top_shell_layers)?,
            bottom_shell_layers: count("bottom_shell_layers", defaults.bottom_shell_layers)?,
            top_shell_thickness: thickness("top_shell_thickness", defaults.top_shell_thickness)?,
            bottom_shell_thickness: thickness(
                "bottom_shell_thickness",
                defaults.bottom_shell_thickness,
            )?,
            sparse_infill_density_percent: density,
            pattern,
            unsupported_pattern,
            infill_direction: settings
                .float("infill_direction")?
                .unwrap_or(defaults.infill_direction),
            lateral_lattice_angles: (
                settings
                    .float("lateral_lattice_angle_1")?
                    .unwrap_or(defaults.lateral_lattice_angles.0),
                settings
                    .float("lateral_lattice_angle_2")?
                    .unwrap_or(defaults.lateral_lattice_angles.1),
            ),
            infill_shift_step: settings
                .float("infill_shift_step")?
                .unwrap_or(defaults.infill_shift_step),
            infill_overhang_angle: settings
                .float("infill_overhang_angle")?
                .unwrap_or(defaults.infill_overhang_angle),
            solid_infill_direction: settings
                .float("solid_infill_direction")?
                .unwrap_or(defaults.solid_infill_direction),
            steady_direction: steady_direction(settings)?,
            minimum_sparse_infill_area: settings
                .float("minimum_sparse_infill_area")?
                .unwrap_or(defaults.minimum_sparse_infill_area),
            infill_anchor_max: settings
                .float_or_percent("infill_anchor_max")?
                .unwrap_or(defaults.infill_anchor_max),
            infill_anchor: settings
                .float_or_percent("infill_anchor")?
                .unwrap_or(defaults.infill_anchor),
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
            // From nothing to 999 mm, as the reference allows.
            bridge_expansion_extra: match settings.float("bridge_expansion_extra")? {
                Some(extra) if !(0.0..=999.0).contains(&extra) => {
                    return Err(ConfigError::BadValue {
                        key: "bridge_expansion_extra".into(),
                        found: extra.to_string(),
                        expected: "from 0 to 999",
                    });
                }
                found => found.unwrap_or(defaults.bridge_expansion_extra),
            },
            bridge_line_width: edge_extrusion::bridge_line_width(settings)?,
            thick_internal_bridges: flag(
                "thick_internal_bridges",
                defaults.thick_internal_bridges,
            )?,
            internal_bridge_density: share(
                "internal_bridge_density",
                defaults.internal_bridge_density,
            )?,
            internal_bridge_filter: match settings.text("dont_filter_internal_bridges") {
                None => defaults.internal_bridge_filter,
                Some("disabled") => InternalBridgeFilter::Filter,
                Some("limited") => InternalBridgeFilter::Limited,
                Some("nofilter") => InternalBridgeFilter::NoFilter,
                Some(other) => {
                    return Err(ConfigError::BadValue {
                        key: "dont_filter_internal_bridges".into(),
                        found: other.to_owned(),
                        expected: "disabled, limited or nofilter",
                    });
                }
            },
            internal_bridge_angle: number("internal_bridge_angle", defaults.internal_bridge_angle)?,
            relative_bridge_angle: flag("relative_bridge_angle", defaults.relative_bridge_angle)?,
            extra_bridge_layer: match settings.text("enable_extra_bridge_layer") {
                None => defaults.extra_bridge_layer,
                Some("disabled") => ExtraBridgeLayer::None,
                Some("external_bridge_only") => ExtraBridgeLayer::External,
                Some("internal_bridge_only") => ExtraBridgeLayer::Internal,
                Some("apply_to_all") => ExtraBridgeLayer::All,
                Some(other) => {
                    return Err(ConfigError::BadValue {
                        key: "enable_extra_bridge_layer".into(),
                        found: other.to_owned(),
                        expected: "disabled, external_bridge_only, internal_bridge_only or apply_to_all",
                    });
                }
            },
            top_ordered,
            bottom_ordered,
            solid_ordered,
            gap_fill_target: match settings.text("gap_fill_target") {
                None => defaults.gap_fill_target,
                Some("everywhere") => crate::GapFillTarget::Everywhere,
                Some("topbottom") => crate::GapFillTarget::TopBottom,
                Some("nowhere") => crate::GapFillTarget::Nowhere,
                Some(other) => {
                    return Err(ConfigError::BadValue {
                        key: "gap_fill_target".into(),
                        found: other.to_owned(),
                        expected: "everywhere, topbottom or nowhere",
                    });
                }
            },
            filter_out_gap_fill: number("filter_out_gap_fill", defaults.filter_out_gap_fill)?,
            top_joined,
            bottom_joined,
            solid_joined,
            ensure_vertical_shells,
            elephant_foot_density,
            // Fill.cpp takes it for one where it is set to less.
            elephant_foot_layers: settings
                .integer("elefant_foot_compensation_layers")?
                .map_or(defaults.elephant_foot_layers, |layers| {
                    usize::try_from(layers).unwrap_or(0).max(1)
                }),
            detect_narrow_solid: flag(
                "detect_narrow_internal_solid_infill",
                defaults.detect_narrow_solid,
            )?,
            seam_gap: settings
                .float_or_percent("seam_gap")?
                .unwrap_or(defaults.seam_gap),
            // Whichever tool lays them (`min_physical_nozzle_diameter`).
            smallest_nozzle: settings
                .floats("nozzle_diameter")?
                .and_then(|nozzles| nozzles.into_iter().flatten().reduce(f64::min))
                .unwrap_or(defaults.smallest_nozzle),
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
        let solid = Flow::from_widths(
            &self.widths,
            self.internal_solid_infill_line_width,
            Flow::auto_width(nozzle),
            layer_height,
            false,
        )?;
        Ok(Shells {
            top_layers: self.top_shell_layers,
            bottom_layers: self.bottom_shell_layers,
            top_thickness: self.top_shell_thickness,
            bottom_thickness: self.bottom_shell_thickness,
            sparse_density: self.sparse_infill_density_percent / 100.0,
            minimum_sparse_area: self.minimum_sparse_infill_area,
            opening: scaled(outer.width / 10.0),
            ensure: self.ensure_vertical_shells.then(|| scaled(solid.spacing)),
            outer_spacing: outer.scaled_spacing(),
            // The width of the walls: the outer one's and every inner
            // one's (`offset_distance` in `detect_surfaces_type`).
            second_bridge: if self.extra_bridge_layer.external() {
                let inner = Flow::from_widths(
                    &self.widths,
                    self.inner_wall_line_width,
                    Flow::auto_width(nozzle),
                    layer_height,
                    false,
                )?;
                Some(
                    outer.scaled_width()
                        + inner.scaled_width() * self.wall_loops.saturating_sub(1) as Coord,
                )
            } else {
                None
            },
        })
    }

    /// The one set of walls a layer of this height would have were it
    /// all in one filament, or none with no walls
    /// (`PrintObject::discover_vertical_shells`).
    pub fn walls_around(&self, layer_height: f64) -> Result<Option<WallsAround>, FlowError> {
        if self.wall_loops == 0 {
            return Ok(None);
        }
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
        Ok(Some(WallsAround {
            depth: scaled(
                0.5 * (outer.width + outer.spacing) + inner.spacing * (self.wall_loops - 1) as f64,
            ),
            spacing: scaled(outer.spacing.min(inner.spacing)),
        }))
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
            bridge_extra: scaled(self.bridge_expansion_extra),
            closing: scaled(0.55 * 0.65 * 1.05 * solid.spacing),
            minimum_sparse_area: self.minimum_sparse_infill_area,
        })
    }

    /// What laying bridges over infill needs on layers of this height
    /// (`PrintObject::bridge_over_infill`).
    pub fn over_infill(&self, layer_height: f64) -> Result<OverInfill, FlowError> {
        let params = self.params(layer_height, false)?;
        // The round thread of a thick bridge, whether or not bridges
        // over infill are laid thick (`bridging_flow(frSolidInfill, true)`).
        let thread = params
            .solid
            .bridging_sized(self.bridge_flow, true, self.bridge_line_width)?;
        let direction = self.infill_direction.to_radians();
        Ok(OverInfill {
            spacing: params.solid.scaled_spacing(),
            filter: self.internal_bridge_filter,
            thread_spacing: thread.scaled_spacing(),
            thread_width: thread.scaled_width(),
            thread_height: thread.height,
            direction: match self.pattern {
                Pattern::Hilbert | Pattern::Octagram => BridgeDirection::AcrossPattern(direction),
                Pattern::Honeycomb3D | Pattern::CrossHatch => {
                    BridgeDirection::Fixed((self.infill_direction + 45.0) * 2.0 * PI / 360.0)
                }
                _ => BridgeDirection::FromLines,
            },
            // PrintObject.cpp, "Preserve the user's absolute or relative
            // internal bridge angle override": only one above zero counts.
            set_angle: (self.internal_bridge_angle > 0.0).then(|| {
                (
                    self.internal_bridge_angle.to_radians(),
                    self.relative_bridge_angle,
                )
            }),
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
        // `LayerRegion::bridging_flow`, with `bridge_line_width` where it
        // is set. Nothing bridges on the bed, and the first layer may be
        // too tall for a line of that width: there the width is left out
        // (Print.cpp `Print::validate` checks it against the print's
        // layer height alone).
        let width = if first_layer {
            None
        } else {
            edge_extrusion::check_bridge_line_width(
                self.bridge_line_width,
                layer_height,
                self.thick_bridges,
                self.thick_internal_bridges,
            )?;
            self.bridge_line_width
        };
        let bridge = solid.bridging_sized(self.bridge_flow, self.thick_bridges, width)?;
        let internal_bridge =
            solid.bridging_sized(self.bridge_flow, self.thick_internal_bridges, width)?;
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
            alternate: !self.steady_direction,
            // Both are shares of the line spacing when given as percentages
            // (Fill.cpp), and an anchor is never longer than a join may be.
            sparse_link_max: self.infill_anchor_max.resolve(sparse.spacing).max(0.0),
            sparse_anchor: self
                .infill_anchor
                .resolve(sparse.spacing)
                .min(self.infill_anchor_max.resolve(sparse.spacing))
                .max(0.0),
            origin: edge_geom::Point2::from_mm(0.0, 0.0),
            z: 0.0,
            lattice_leans: (
                self.lateral_lattice_angles.0.to_radians(),
                self.lateral_lattice_angles.1.to_radians(),
            ),
            cross_step: self.infill_shift_step,
            overhang_lean: self.infill_overhang_angle.to_radians(),
            top_joined: self.top_joined,
            bottom_joined: self.bottom_joined,
            solid_joined: self.solid_joined,
            top_ordered: self.top_ordered,
            bottom_ordered: self.bottom_ordered,
            solid_ordered: self.solid_ordered,
            gap_fill: self.gap_fill_target,
            gap_shortest: self.filter_out_gap_fill,
            elephant_foot_density: self.elephant_foot_density,
            elephant_foot_layers: self.elephant_foot_layers,
            // The gap is a share of the nozzle the fill is laid with
            // (Fill.cpp, `loop_clipping`).
            narrow: self.detect_narrow_solid.then(|| NarrowFill {
                nozzle: self.smallest_nozzle,
                seam_gap: self.seam_gap.resolve(nozzle).max(0.0),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ensuring_all_of_the_vertical_shell_is_followed() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        let all = read(r#"{"ensure_vertical_shell_thickness": "ensure_all"}"#).unwrap();
        assert!(all.shells(0.2).unwrap().ensure.is_some());
        assert!(all.unsupported.is_empty());
        let moderate = read(r#"{"ensure_vertical_shell_thickness": "ensure_moderate"}"#).unwrap();
        assert_eq!(moderate.shells(0.2).unwrap().ensure, None);
        assert!(moderate.unsupported[0].starts_with("ensure_vertical_shell_thickness"));
        // Bambu Studio's word for it, and the old switch, are the same.
        for same in ["enabled", "1"] {
            let found = read(&format!(
                r#"{{"ensure_vertical_shell_thickness": "{same}"}}"#
            ))
            .unwrap();
            assert!(found.shells(0.2).unwrap().ensure.is_some(), "{same}");
            assert!(found.unsupported.is_empty(), "{same}");
        }
        for other in ["partial", "disabled", "0"] {
            let found = read(&format!(
                r#"{{"ensure_vertical_shell_thickness": "{other}"}}"#
            ))
            .unwrap();
            assert_eq!(found.shells(0.2).unwrap().ensure, None, "{other}");
        }
        // Absent, it is the reference's default.
        let absent = InfillSettings::from_settings(&Settings::default()).unwrap();
        assert!(absent.shells(0.2).unwrap().ensure.is_some());
    }

    #[test]
    fn narrow_solid_areas_are_told_apart_unless_that_is_turned_off() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // On unless turned off, as in the reference, and nothing is
        // said of it either way.
        let unset = read("{}").unwrap();
        assert!(unset.detect_narrow_solid && unset.unsupported.is_empty());
        let on = read(r#"{"detect_narrow_internal_solid_infill": "1"}"#).unwrap();
        assert!(on.detect_narrow_solid && on.unsupported.is_empty());
        let off = read(r#"{"detect_narrow_internal_solid_infill": "0"}"#).unwrap();
        assert!(!off.detect_narrow_solid && off.unsupported.is_empty());
        assert_eq!(off.params(0.2, false).unwrap().narrow, None);
        // The loops stop the seam gap short, a share of the nozzle the
        // fill is laid with, and are no thinner than the smallest
        // nozzle on the printer allows.
        let set = read(r#"{"seam_gap": "15%", "nozzle_diameter": ["0.6", "0.4", "0.8"]}"#).unwrap();
        let narrow = set.params(0.2, false).unwrap().narrow.unwrap();
        assert!((narrow.seam_gap - 0.09).abs() < 1e-12);
        assert_eq!(narrow.nozzle, 0.4);
        let stock = unset.params(0.2, false).unwrap().narrow.unwrap();
        assert!((stock.seam_gap - 0.04).abs() < 1e-12);
        assert_eq!(stock.nozzle, 0.4);
    }

    #[test]
    fn the_density_of_solid_fill_over_the_first_layer_is_read() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // Full, and for one layer, unless set.
        let unset = read("{}").unwrap();
        assert_eq!(
            (unset.elephant_foot_density, unset.elephant_foot_layers),
            (1.0, 1)
        );
        assert_eq!(unset.params(0.2, false).unwrap().solid_density(1), 1.0);
        let set = read(
            r#"{"elefant_foot_layers_density": "70%", "elefant_foot_compensation_layers": "3"}"#,
        )
        .unwrap();
        assert_eq!(
            (set.elephant_foot_density, set.elephant_foot_layers),
            (0.7, 3)
        );
        assert!(set.unsupported.is_empty());
        let params = set.params(0.2, false).unwrap();
        assert!((params.solid_density(1) - 0.7).abs() < 1e-12);
        assert!((params.solid_density(3) - 0.9).abs() < 1e-12);
        assert_eq!(params.solid_density(4), 1.0);
        // Less than one layer is taken for one, as in the reference.
        let none = read(
            r#"{"elefant_foot_layers_density": "50%", "elefant_foot_compensation_layers": "0"}"#,
        )
        .unwrap();
        assert_eq!(
            (none.elephant_foot_density, none.elephant_foot_layers),
            (0.5, 1)
        );
        // Under half and over everything are refused, as the reference
        // refuses them.
        assert!(read(r#"{"elefant_foot_layers_density": "30%"}"#).is_err());
        assert!(read(r#"{"elefant_foot_layers_density": "49.9%"}"#).is_err());
        assert!(read(r#"{"elefant_foot_layers_density": "120%"}"#).is_err());
        assert!(read(r#"{"elefant_foot_layers_density": "100%"}"#).is_ok());
    }

    #[test]
    fn the_fills_own_gap_fill_is_read() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // Nowhere unless asked for, as in the reference.
        let unset = read("{}").unwrap();
        assert_eq!(unset.gap_fill_target, crate::GapFillTarget::Nowhere);
        let set =
            read(r#"{"gap_fill_target": "topbottom", "filter_out_gap_fill": "0.4"}"#).unwrap();
        let params = set.params(0.2, false).unwrap();
        assert_eq!(
            (params.gap_fill, params.gap_shortest),
            (crate::GapFillTarget::TopBottom, 0.4)
        );
        assert_eq!(
            read(r#"{"gap_fill_target": "everywhere"}"#)
                .unwrap()
                .gap_fill_target,
            crate::GapFillTarget::Everywhere
        );
        assert!(read(r#"{"gap_fill_target": "sometimes"}"#).is_err());
    }

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
        // The reference's 0.6 mm under a top, and no least over a bottom.
        assert_eq!((shells.top_thickness, shells.bottom_thickness), (0.6, 0.0));
    }

    #[test]
    fn reads_how_thick_the_shells_are_to_be() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        let settings = read(
            r#"{"top_shell_layers": "6", "top_shell_thickness": "1.0",
                "bottom_shell_layers": "4", "bottom_shell_thickness": "0.8"}"#,
        )
        .unwrap();
        assert_eq!(
            (
                settings.top_shell_thickness,
                settings.bottom_shell_thickness
            ),
            (1.0, 0.8)
        );
        let shells = settings.shells(0.16).unwrap();
        assert_eq!((shells.top_layers, shells.bottom_layers), (6, 4));
        assert_eq!((shells.top_thickness, shells.bottom_thickness), (1.0, 0.8));
        assert!(settings.unsupported.is_empty());
        // Zero leaves it to the count; less than nothing is no thickness.
        let unset = read(r#"{"top_shell_thickness": "0", "bottom_shell_thickness": "0"}"#).unwrap();
        assert_eq!(
            (unset.top_shell_thickness, unset.bottom_shell_thickness),
            (0.0, 0.0)
        );
        assert!(read(r#"{"top_shell_thickness": "-1"}"#).is_err());
        assert!(read(r#"{"bottom_shell_thickness": "-0.2"}"#).is_err());
        assert!(read(r#"{"top_shell_thickness": "thick"}"#).is_err());
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
        // Both monotonic patterns are swept in order; rectilinear is not.
        assert_eq!(
            (unset.top_ordered, unset.bottom_ordered, unset.solid_ordered),
            (true, true, true)
        );
        assert_eq!(
            (set.top_ordered, set.bottom_ordered, set.solid_ordered),
            (true, true, false)
        );
        assert!(set.unsupported.is_empty());
        // A pattern not laid here is said, and laid as joined lines.
        let other = read(r#"{"top_surface_pattern": "concentric"}"#).unwrap();
        assert!(other.top_joined && !other.top_ordered);
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

        // Every pattern laid here is known by the reference's name for it.
        for (name, pattern) in [
            ("grid", Pattern::Grid),
            ("rectilinear", Pattern::Lines),
            ("alignedrectilinear", Pattern::AlignedLines),
            ("triangles", Pattern::Triangles),
            ("tri-hexagon", Pattern::Stars),
            ("cubic", Pattern::Cubic),
            ("quartercubic", Pattern::QuarterCubic),
            ("crosshatch", Pattern::CrossHatch),
            ("gyroid", Pattern::Gyroid),
            ("honeycomb", Pattern::Honeycomb),
            ("3dhoneycomb", Pattern::Honeycomb3D),
            ("lateral-honeycomb", Pattern::LateralHoneycomb),
            ("lateral-lattice", Pattern::LateralLattice),
            ("zigzag", Pattern::ZigZag),
            ("crosszag", Pattern::CrossZag),
            ("hilbertcurve", Pattern::Hilbert),
            ("archimedeanchords", Pattern::Archimedean),
            ("octagramspiral", Pattern::Octagram),
            ("tpmsd", Pattern::TpmsD),
        ] {
            let found = read(&format!(r#"{{"sparse_infill_pattern": "{name}"}}"#)).unwrap();
            assert_eq!((found.pattern, found.unsupported_pattern), (pattern, None));
        }
        // One that is not is laid as a grid, and said.
        let other = read(r#"{"sparse_infill_pattern": "lightning"}"#).unwrap();
        assert_eq!(
            (other.pattern, other.unsupported_pattern.as_deref()),
            (Pattern::Grid, Some("lightning"))
        );
        // With none set it is the reference's default.
        assert_eq!(read("{}").unwrap().pattern, Pattern::CrossHatch);
        // What is asked of a pattern and not done is said.
        let asked = read(
            r#"{"sparse_infill_pattern": "gyroid", "gyroid_optimized": "1", "fill_multiline": "2",
                "sparse_infill_rotate_template": "0,90"}"#,
        )
        .unwrap();
        for key in [
            "fill_multiline",
            "gyroid_optimized",
            "sparse_infill_rotate_template",
        ] {
            assert!(
                asked.unsupported.iter().any(|note| note.starts_with(key)),
                "{key} is not noted"
            );
        }
        for (asked, key) in [
            (
                r#"{"sparse_infill_pattern": "3dhoneycomb", "infill_complete_top": "1"}"#,
                "infill_complete_top",
            ),
            (
                r#"{"sparse_infill_pattern": "zigzag", "symmetric_infill_y_axis": "1"}"#,
                "symmetric_infill_y_axis",
            ),
            (
                r#"{"sparse_infill_pattern": "crosszag", "symmetric_infill_y_axis": "1"}"#,
                "symmetric_infill_y_axis",
            ),
        ] {
            let notes = read(asked).unwrap().unsupported;
            assert!(notes.iter().any(|note| note.starts_with(key)), "{asked}");
        }
        // Neither is said of a pattern it does not touch.
        assert!(
            read(r#"{"sparse_infill_pattern": "grid", "infill_complete_top": "1", "symmetric_infill_y_axis": "1"}"#)
                .unwrap()
                .unsupported
                .is_empty()
        );
        // What shapes the lateral patterns and the cross zag is read.
        let leaning = read(
            r#"{"sparse_infill_pattern": "lateral-lattice", "lateral_lattice_angle_1": "-30",
                "lateral_lattice_angle_2": "60", "infill_shift_step": "0.6", "infill_overhang_angle": "50"}"#,
        )
        .unwrap()
        .params(0.2, false)
        .unwrap();
        assert!((leaning.lattice_leans.0 + 30f64.to_radians()).abs() < 1e-12);
        assert!((leaning.lattice_leans.1 - 60f64.to_radians()).abs() < 1e-12);
        assert_eq!(leaning.cross_step, 0.6);
        assert!((leaning.overhang_lean - 50f64.to_radians()).abs() < 1e-12);
        let line = read(r#"{"sparse_infill_pattern": "line"}"#).unwrap();
        assert_eq!(line.pattern, Pattern::Lines);
        assert!(
            line.unsupported
                .iter()
                .any(|note| note.contains("\"line\""))
        );
        assert!(
            read(r#"{"sparse_infill_pattern": "crosshatch"}"#)
                .unwrap()
                .unsupported
                .is_empty()
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
        // Bridges over infill are laid at `internal_bridge_angle` now, so
        // only `bridge_angle` is said.
        let set = read(r#"{"bridge_angle": "30", "internal_bridge_angle": "45"}"#).unwrap();
        assert_eq!(set.unsupported.len(), 1);
        assert!(set.unsupported[0].starts_with("bridge_angle \"30\""));
        let over = set.over_infill(0.2).unwrap();
        assert_eq!(over.set_angle, Some((45f64.to_radians(), false)));
        let turned = read(r#"{"internal_bridge_angle": "45", "relative_bridge_angle": "1"}"#)
            .unwrap()
            .over_infill(0.2)
            .unwrap();
        assert_eq!(turned.set_angle, Some((45f64.to_radians(), true)));
        assert_eq!(
            read("{}").unwrap().over_infill(0.2).unwrap().set_angle,
            None
        );
    }

    #[test]
    fn bridges_take_the_width_set_for_them() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // Left at zero, as it is by default, nothing changes and nothing is said.
        let unset = read(r#"{"bridge_flow": "0.8", "thick_bridges": "0"}"#).unwrap();
        let zero =
            read(r#"{"bridge_flow": "0.8", "thick_bridges": "0", "bridge_line_width": "0"}"#)
                .unwrap();
        assert_eq!(unset.bridge_line_width, None);
        assert_eq!(zero.params(0.2, false), unset.params(0.2, false));
        assert!(zero.unsupported.is_empty());
        // Set, a flattened bridge is that wide before `bridge_flow` thins
        // it, and a round thread over infill that thick.
        let set = read(
            r#"{"bridge_flow": "0.8", "thick_bridges": "0", "bridge_line_width": "0.3",
                "bridge_density": "80%"}"#,
        )
        .unwrap();
        assert!(set.unsupported.is_empty());
        let params = set.params(0.2, false).unwrap();
        let flat = Flow::new(0.3, 0.2, 0.4).unwrap().bridging(0.8, false);
        assert_eq!(params.bridge, flat);
        assert!((params.bridge_spacing - flat.spacing / 0.8).abs() < 1e-12);
        let thread = 0.3 * 0.8f64.sqrt();
        assert!((params.internal_bridge.width - thread).abs() < 1e-12);
        assert!((params.internal_bridge.height - thread).abs() < 1e-12);
        assert!((params.internal_bridge_spacing - (thread + 0.05)).abs() < 1e-12);
        // Nothing else is touched.
        let plain = unset.params(0.2, false).unwrap();
        assert_eq!(
            (params.solid, params.top, params.sparse),
            (plain.solid, plain.top, plain.sparse)
        );
        // A percentage is of the nozzle.
        let share = read(r#"{"bridge_line_width": "75%", "thick_bridges": "1"}"#).unwrap();
        assert!((share.params(0.2, false).unwrap().bridge.width - 0.3).abs() < 1e-12);
        // Wider than the nozzle is refused when read; no wider than the
        // layer is tall when layers are made, unless every bridge is a
        // round thread. The first layer, where nothing bridges, is not
        // judged and is left as it was.
        assert!(read(r#"{"bridge_line_width": "0.45"}"#).is_err());
        let thin = read(r#"{"bridge_line_width": "0.2", "thick_bridges": "0"}"#).unwrap();
        assert!(thin.params(0.2, false).is_err());
        assert_eq!(thin.params(0.25, true), unset_first(&thin));
        let threads = read(
            r#"{"bridge_line_width": "0.15", "thick_bridges": "1", "thick_internal_bridges": "1"}"#,
        )
        .unwrap();
        assert!((threads.params(0.2, false).unwrap().bridge.width - 0.15).abs() < 1e-12);
    }

    #[test]
    fn how_small_bridges_over_infill_are_sorted_out_is_read() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // Filtered unless asked otherwise, as in the reference.
        assert_eq!(
            read("{}").unwrap().internal_bridge_filter,
            InternalBridgeFilter::Filter
        );
        for (value, filter) in [
            ("disabled", InternalBridgeFilter::Filter),
            ("limited", InternalBridgeFilter::Limited),
            ("nofilter", InternalBridgeFilter::NoFilter),
        ] {
            let found = read(&format!(r#"{{"dont_filter_internal_bridges": "{value}"}}"#)).unwrap();
            assert_eq!(found.internal_bridge_filter, filter);
            assert!(found.unsupported.is_empty());
            assert_eq!(found.over_infill(0.2).unwrap().filter, filter);
        }
        assert!(read(r#"{"dont_filter_internal_bridges": "1"}"#).is_err());
        // Solid lines 0.45 mm wide; a thick bridge's thread is the
        // nozzle's 0.4 mm across, whatever the layer height. (How deep it
        // sinks was counted in layers here once; it is now measured
        // against the layers' own heights.)
        let unset = read("{}").unwrap();
        let over = unset.over_infill(0.2).unwrap();
        assert_eq!(
            (over.spacing, over.thread_spacing),
            (
                unset.params(0.2, false).unwrap().solid.scaled_spacing(),
                450_000
            )
        );
        assert_eq!((over.thread_width, over.thread_height), (400_000, 0.4));
        assert_eq!(unset.over_infill(0.1).unwrap().thread_height, 0.4);
        // A thread thinned by `bridge_line_width` is thinner every way.
        let thin = read(r#"{"bridge_line_width": "0.3", "thick_bridges": "1"}"#).unwrap();
        let over = thin.over_infill(0.1).unwrap();
        assert_eq!(over.thread_spacing, 350_000);
        assert_eq!((over.thread_width, over.thread_height), (300_000, 0.3));
        // Which way it runs goes by the pattern beneath.
        let pattern = |name: &str| {
            read(&format!(
                r#"{{"sparse_infill_pattern": "{name}", "infill_direction": "30"}}"#
            ))
            .unwrap()
            .over_infill(0.2)
            .unwrap()
            .direction
        };
        assert_eq!(pattern("grid"), BridgeDirection::FromLines);
        assert_eq!(pattern("gyroid"), BridgeDirection::FromLines);
        assert_eq!(
            pattern("hilbertcurve"),
            BridgeDirection::AcrossPattern(30f64.to_radians())
        );
        assert_eq!(
            pattern("3dhoneycomb"),
            BridgeDirection::Fixed(75.0 * 2.0 * PI / 360.0)
        );
        assert_eq!(pattern("crosshatch"), pattern("3dhoneycomb"));
    }

    #[test]
    fn which_bridges_get_a_second_layer_is_read() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // None unless asked for.
        let unset = read("{}").unwrap();
        assert_eq!(unset.extra_bridge_layer, ExtraBridgeLayer::None);
        assert_eq!(unset.shells(0.2).unwrap().second_bridge, None);
        for (value, which, external, internal) in [
            ("disabled", ExtraBridgeLayer::None, false, false),
            (
                "external_bridge_only",
                ExtraBridgeLayer::External,
                true,
                false,
            ),
            (
                "internal_bridge_only",
                ExtraBridgeLayer::Internal,
                false,
                true,
            ),
            ("apply_to_all", ExtraBridgeLayer::All, true, true),
        ] {
            let found = read(&format!(r#"{{"enable_extra_bridge_layer": "{value}"}}"#)).unwrap();
            assert_eq!(found.extra_bridge_layer, which);
            assert_eq!(
                (which.external(), which.internal()),
                (external, internal),
                "{value}"
            );
            assert_eq!(found.shells(0.2).unwrap().second_bridge.is_some(), external);
            assert!(found.unsupported.is_empty());
        }
        assert!(read(r#"{"enable_extra_bridge_layer": "1"}"#).is_err());
        assert!(read(r#"{"enable_extra_bridge_layer": "both"}"#).is_err());
        // A bridge worth a second layer is wider than its walls twice
        // over: the outer wall's width and each inner wall's.
        let walls = read(
            r#"{"enable_extra_bridge_layer": "apply_to_all", "wall_loops": "3",
                "outer_wall_line_width": "0.42", "inner_wall_line_width": "0.45"}"#,
        )
        .unwrap();
        assert_eq!(walls.shells(0.2).unwrap().second_bridge, Some(1_320_000));
        let none = read(
            r#"{"enable_extra_bridge_layer": "apply_to_all", "wall_loops": "0",
                "outer_wall_line_width": "0.42"}"#,
        )
        .unwrap();
        assert_eq!(none.shells(0.2).unwrap().second_bridge, Some(420_000));
    }

    #[test]
    fn a_bridge_s_extra_reach_is_read() {
        let read = |json: &str| InfillSettings::from_settings(&Settings::from_json(json).unwrap());
        // None unless asked for, and then nothing changes.
        let unset = read("{}").unwrap();
        assert_eq!(unset.bridge_expansion_extra, 0.0);
        assert_eq!(unset.anchoring(0.2).unwrap().bridge_extra, 0);
        let zero = read(r#"{"bridge_expansion_extra": "0"}"#).unwrap();
        assert_eq!(zero.anchoring(0.2), unset.anchoring(0.2));
        // Set, it is added to a bridge's reach and to nothing else.
        let set = read(r#"{"bridge_expansion_extra": "2.5"}"#).unwrap();
        assert!(set.unsupported.is_empty());
        assert_eq!(
            set.anchoring(0.2).unwrap(),
            Anchoring {
                bridge_extra: 2_500_000,
                ..unset.anchoring(0.2).unwrap()
            }
        );
        // Outside what the reference allows, or not a number.
        assert!(read(r#"{"bridge_expansion_extra": "-1"}"#).is_err());
        assert!(read(r#"{"bridge_expansion_extra": "1000"}"#).is_err());
        assert!(read(r#"{"bridge_expansion_extra": "far"}"#).is_err());
        assert!(read(r#"{"bridge_expansion_extra": "999"}"#).is_ok());
    }

    /// The first layer's parameters with no bridge width set.
    fn unset_first(settings: &InfillSettings) -> Result<FillParams, FlowError> {
        InfillSettings {
            bridge_line_width: None,
            ..settings.clone()
        }
        .params(0.25, true)
    }
}
