use edge_avoid::Detour;
use edge_config::{ConfigError, FloatOrPercent, Settings, Value as Setting};
use edge_extrusion::Flow;
use edge_template::{Value, Variables};

use crate::{scarf::Scarf, seam::Seam};

/// How the nozzle rises before a travel (`z_hop_types`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lift {
    /// Straight up, then across.
    Normal,
    /// Up a ramp along the start of the travel.
    Slope,
    /// Up a turn of a spiral, then across.
    Spiral,
    /// A ramp, or a spiral where the travel starts a new layer. The
    /// reference also spirals when the ramp would cross an overhang; that
    /// is not looked at here.
    Auto,
}

/// Which wall of an island is printed first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallOrder {
    InnerOuter,
    OuterInner,
    /// The outer wall between the inner ones: those behind the first
    /// inner wall, then the outer wall, then the first inner wall. Above
    /// the first layer, which goes inner wall first.
    InnerOuterInner,
}

/// Which way round walls are laid (`wall_direction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallDirection {
    /// Counter-clockwise, round holes as round the part
    /// (PerimeterGenerator.cpp: "all walls are counter-clockwise
    /// initially").
    Auto,
    CounterClockwise,
    Clockwise,
}

/// The fan for whatever is laid over air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverhangFan {
    /// In percent.
    pub speed: f64,
    /// A wall gets it where no more than this share of its width is
    /// supported; none gives it to every outer wall. Bridges always get it.
    pub at: Option<f64>,
}

/// The settings G-code export depends on, read once per print. Defaults
/// are the reference's (PrintConfig.cpp). Speeds are in mm/s, lengths in mm,
/// temperatures in °C.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintSettings {
    pub outer_wall_speed: f64,
    pub inner_wall_speed: f64,
    pub sparse_infill_speed: f64,
    pub internal_solid_infill_speed: f64,
    pub top_surface_speed: f64,
    /// Ironing lines: the process's speed, or the filament's own where
    /// it sets one (`ironing_speed`, `filament_ironing_speed`).
    pub ironing_speed: f64,
    /// The part fan while ironing, in percent; none when ironing gets
    /// the layer's (`ironing_fan_speed` below zero).
    pub ironing_fan_speed: Option<f64>,
    pub bridge_speed: f64,
    /// Lines that fill the gaps between walls.
    pub gap_infill_speed: f64,
    /// Whether a stretch of wall with nothing under it is printed as a
    /// bridge, in its own kind of line.
    pub detect_overhang_wall: bool,
    /// Whether walls slow down as less of them is supported, and the
    /// speeds where a quarter, a half, three quarters and nearly all of
    /// the line hangs over; zero for the wall's own speed.
    pub enable_overhang_speed: bool,
    pub overhang_speeds: [FloatOrPercent; 4],
    /// With this on, the reference prints a wall with nothing under it
    /// at the last of the overhang speeds, not at the bridge's
    /// (`slowdown_for_curled_perimeters`). Slowing over edges that have
    /// curled up, which the setting is named for, is not done.
    pub slowdown_for_curled_perimeters: bool,
    /// The fan for bridges and overhangs; none when they get the layer's.
    pub overhang_fan: Option<OverhangFan>,
    /// A bridge's plastic as a share of a normal line's, and whether it
    /// is a round thread instead of a flattened line.
    pub bridge_flow: f64,
    pub thick_bridges: bool,
    /// The width set for everything laid as a bridge, in mm
    /// (`bridge_line_width`); none leaves a wall over air as wide as the
    /// inner walls.
    pub bridge_line_width: Option<f64>,
    /// May be a percentage of the outer wall's acceleration.
    pub bridge_acceleration: FloatOrPercent,
    /// Solid fill laid over sparse fill; a percentage is of the bridge's speed.
    pub internal_bridge_speed: FloatOrPercent,
    /// A wall loop no longer than a circle of this radius, in mm, is laid
    /// at `small_perimeter_speed`, which as a percentage is of the outer
    /// wall's speed; at nothing, at half that speed. A threshold of zero
    /// leaves every wall at its own speed.
    pub small_perimeter_threshold: f64,
    pub small_perimeter_speed: FloatOrPercent,
    /// Walls on the first layer.
    pub initial_layer_speed: f64,
    /// All fill on the first layer.
    pub initial_layer_infill_speed: f64,
    pub travel_speed: f64,
    /// Speed of moves that only change height: `travel_speed_z` or else the
    /// travel speed, held to the machine's Z limit.
    pub z_speed: f64,
    /// Most plastic the hot end can melt, in mm³/s; zero for no limit.
    pub filament_max_volumetric_speed: f64,

    pub retraction_length: f64,
    pub retraction_speed: f64,
    /// Zero means "same as retraction".
    pub deretraction_speed: f64,
    /// Travels shorter than this are made without retracting.
    pub retraction_minimum_travel: f64,
    pub z_hop: f64,
    pub lift: Lift,
    /// Steepness of a ramped lift, in degrees.
    pub travel_slope: f64,
    /// The nozzle is only lifted between these heights; zero for no limit.
    pub retract_lift_above: f64,
    pub retract_lift_below: f64,
    pub retract_when_changing_layer: bool,
    /// After retracting, drag the nozzle back over what it just printed.
    pub wipe: bool,
    pub wipe_distance: f64,
    /// Shares of the retraction made before and after the wipe; the rest
    /// happens during it.
    pub retract_before_wipe: f64,
    pub retract_after_wipe: f64,
    /// Absent to wipe at the speed of the line just printed.
    pub wipe_speed: Option<f64>,

    pub nozzle_diameter: f64,
    pub seam: Seam,
    /// Each inner wall starts a little further round than the wall
    /// outside it, so the seams do not lie one behind another
    /// (`staggered_inner_seams`).
    pub staggered_inner_seams: bool,
    /// How far short of its start a wall loop stops, in mm.
    pub seam_gap: f64,
    /// Which wall loops are joined on a slope instead, and how.
    pub scarf: Scarf,
    /// Curves are cut into segments that stray no further than this, in mm.
    pub resolution: f64,
    /// Whether stretches of a path that lie on a circle are written as
    /// arcs, `G2` and `G3` (`enable_arc_fitting`), within `resolution`
    /// of the path. Never in spiral vase mode, as in the reference
    /// (LayerRegion.cpp `simplify_path`).
    pub arc_fitting: bool,
    /// The bed's extent in mm: min x, min y, max x, max y.
    pub bed: Option<[f64; 4]>,

    pub filament_diameter: f64,
    pub filament_flow_ratio: f64,
    pub print_flow_ratio: f64,
    /// Further shares of the plastic for particular kinds of line
    /// (GCode.cpp `_extrude`).
    pub top_solid_infill_flow_ratio: f64,
    pub bottom_solid_infill_flow_ratio: f64,
    pub internal_bridge_flow: f64,
    pub brim_flow_ratio: f64,
    /// In g/cm³, when the filament profile gives one.
    pub filament_density: Option<f64>,

    pub nozzle_temperature: f64,
    pub nozzle_temperature_initial_layer: f64,
    /// Absent when the settings name no bed type this engine knows.
    pub bed_temperature: Option<f64>,
    pub bed_temperature_initial_layer: Option<f64>,

    pub wall_order: WallOrder,
    pub wall_direction: WallDirection,
    pub infill_first: bool,
    /// Skip retraction on moves between fill lines that stay inside the walls.
    pub reduce_infill_retraction: bool,
    /// Travel goes round the inside of the walls where a straight move
    /// would cross them (`reduce_crossing_wall`), unless the way round is
    /// longer than the straight move by more than `max_travel_detour`
    /// (`max_travel_detour_distance`; none for any length).
    pub reduce_crossing_wall: bool,
    pub max_travel_detour: Option<Detour>,
    /// Part cooling fan, as percentages: the speed on layers that take
    /// `fan_cooling_layer_time` seconds or longer, and the speed on layers
    /// as short as `slow_down_layer_time`. Between the two it is scaled.
    pub fan_min_speed: f64,
    pub fan_max_speed: f64,
    pub fan_cooling_layer_time: f64,
    pub slow_down_layer_time: f64,
    /// The fan stays off for this many layers at the start.
    pub close_fan_the_first_x_layers: usize,
    /// The layer the fan reaches full speed on, ramping up; zero for no ramp.
    pub full_fan_speed_layer: usize,
    /// On layers long enough to need no cooling, keep the fan at its
    /// minimum instead of stopping it.
    pub reduce_fan_stop_start_freq: bool,
    /// Slow layers shorter than `slow_down_layer_time` down to that long,
    /// though no line goes below `slow_down_min_speed`.
    pub slow_down_for_layer_cooling: bool,
    pub slow_down_min_speed: f64,
    pub dont_slow_down_outer_wall: bool,
    /// The speed of the walls round a circle the auto circle
    /// compensation resized, in mm/s: the filament's
    /// `circle_compensation_speed`, the speed its compensation was
    /// measured at. Zero leaves them at the walls' own speeds.
    pub circle_compensation_speed: f64,
    /// Whether the printer has a second, auxiliary fan, and its speed in
    /// percent once the first layers are down.
    pub auxiliary_fan: bool,
    pub additional_cooling_fan_speed: f64,
    /// Speed of the skirt; zero for the usual speeds.
    pub skirt_speed: f64,
    /// Speeds of a support's sparse body and of its dense top.
    pub support_speed: f64,
    pub support_interface_speed: f64,
    /// The fastest a nozzle lays plastic on the prime tower.
    pub wipe_tower_max_purge_speed: f64,

    /// Accelerations in mm/s²; zero means "leave it as it is".
    pub default_acceleration: f64,
    pub outer_wall_acceleration: f64,
    pub inner_wall_acceleration: f64,
    pub initial_layer_acceleration: f64,
    pub top_surface_acceleration: f64,
    /// Fill accelerations may be a percentage of the default.
    pub sparse_infill_acceleration: FloatOrPercent,
    pub internal_solid_infill_acceleration: FloatOrPercent,
    pub travel_acceleration: f64,
    /// Klipper also takes a deceleration limit, as a share of the acceleration.
    pub accel_to_decel_factor: Option<f64>,
    pub klipper: bool,
    /// What the machine can do, for timing the print; none without a
    /// printer profile.
    pub limits: Option<edge_time::Limits>,

    pub pressure_advance: Option<f64>,

    /// The printer's and filament's own G-code, as templates. Without a
    /// start template a generic preamble is written instead.
    pub machine_start_gcode: Option<String>,
    pub machine_end_gcode: Option<String>,
    pub before_layer_change_gcode: Option<String>,
    pub layer_change_gcode: Option<String>,
    pub filament_start_gcode: Option<String>,
    pub filament_end_gcode: Option<String>,
    /// The printer's own G-code for swapping tools.
    pub change_filament_gcode: Option<String>,
    /// How far a filament is pulled back when its tool is put away.
    pub retract_length_toolchange: f64,
    /// What a tool waiting its turn is cooled by, when the profile keeps
    /// idle nozzles from oozing; and how long before its turn it is heated
    /// again, in seconds.
    pub standby_temperature_delta: Option<f64>,
    pub preheat_time: f64,
    /// Whether the tool's temperature is waited for after a change.
    pub tool_change_temperature_wait: bool,
    /// How many tools the printer has.
    pub tools: usize,
    /// How many filaments the project is set up with, used or not.
    pub filaments: usize,
    /// Every setting, by name, for the templates to refer to.
    pub variables: Variables,
    /// Every setting as the file's settings block lists it, in order.
    /// Printers and their hosts read the material, nozzle and
    /// temperatures from there.
    pub config: Vec<(String, String)>,
    /// When the file is made, as `2026-11-10 at 07:21:37`, for its first
    /// lines; none leaves the time out.
    pub produced_at: Option<String>,

    /// Settings present in the input that export does not honour yet, each
    /// with what is done instead.
    pub unsupported: Vec<String>,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            outer_wall_speed: 60.0,
            inner_wall_speed: 60.0,
            sparse_infill_speed: 100.0,
            internal_solid_infill_speed: 100.0,
            top_surface_speed: 100.0,
            ironing_speed: 20.0,
            ironing_fan_speed: None,
            bridge_speed: 25.0,
            gap_infill_speed: 30.0,
            detect_overhang_wall: true,
            enable_overhang_speed: true,
            overhang_speeds: [FloatOrPercent::Value(0.0); 4],
            slowdown_for_curled_perimeters: false,
            overhang_fan: Some(OverhangFan {
                speed: 100.0,
                at: Some(0.05),
            }),
            bridge_flow: 1.0,
            thick_bridges: false,
            bridge_line_width: None,
            bridge_acceleration: FloatOrPercent::Percent(50.0),
            internal_bridge_speed: FloatOrPercent::Percent(150.0),
            small_perimeter_threshold: 0.0,
            small_perimeter_speed: FloatOrPercent::Percent(50.0),
            initial_layer_speed: 30.0,
            initial_layer_infill_speed: 60.0,
            travel_speed: 120.0,
            z_speed: 120.0,
            filament_max_volumetric_speed: 0.0,
            retraction_length: 0.8,
            retraction_speed: 30.0,
            deretraction_speed: 0.0,
            retraction_minimum_travel: 2.0,
            z_hop: 0.4,
            lift: Lift::Slope,
            travel_slope: 3.0,
            retract_lift_above: 0.0,
            retract_lift_below: 0.0,
            retract_when_changing_layer: false,
            wipe: false,
            wipe_distance: 1.0,
            retract_before_wipe: 1.0,
            retract_after_wipe: 0.0,
            wipe_speed: None,
            nozzle_diameter: 0.4,
            seam: Seam::Aligned,
            staggered_inner_seams: false,
            seam_gap: 0.04,
            scarf: Scarf::default(),
            resolution: 0.01,
            arc_fitting: false,
            bed: None,
            filament_diameter: 1.75,
            filament_flow_ratio: 1.0,
            print_flow_ratio: 1.0,
            top_solid_infill_flow_ratio: 1.0,
            bottom_solid_infill_flow_ratio: 1.0,
            internal_bridge_flow: 1.0,
            brim_flow_ratio: 1.0,
            filament_density: None,
            nozzle_temperature: 200.0,
            nozzle_temperature_initial_layer: 200.0,
            bed_temperature: None,
            bed_temperature_initial_layer: None,
            wall_order: WallOrder::InnerOuter,
            wall_direction: WallDirection::Auto,
            infill_first: false,
            reduce_infill_retraction: false,
            reduce_crossing_wall: false,
            max_travel_detour: None,
            fan_min_speed: 20.0,
            fan_max_speed: 100.0,
            fan_cooling_layer_time: 60.0,
            slow_down_layer_time: 5.0,
            close_fan_the_first_x_layers: 1,
            full_fan_speed_layer: 0,
            reduce_fan_stop_start_freq: false,
            slow_down_for_layer_cooling: true,
            slow_down_min_speed: 10.0,
            dont_slow_down_outer_wall: false,
            circle_compensation_speed: 200.0,
            auxiliary_fan: false,
            additional_cooling_fan_speed: 0.0,
            skirt_speed: 50.0,
            support_speed: 80.0,
            support_interface_speed: 80.0,
            wipe_tower_max_purge_speed: 90.0,
            default_acceleration: 0.0,
            outer_wall_acceleration: 0.0,
            inner_wall_acceleration: 0.0,
            initial_layer_acceleration: 0.0,
            top_surface_acceleration: 0.0,
            sparse_infill_acceleration: FloatOrPercent::Percent(100.0),
            internal_solid_infill_acceleration: FloatOrPercent::Percent(100.0),
            travel_acceleration: 0.0,
            accel_to_decel_factor: None,
            klipper: false,
            limits: None,
            pressure_advance: None,
            machine_start_gcode: None,
            machine_end_gcode: None,
            before_layer_change_gcode: None,
            layer_change_gcode: None,
            filament_start_gcode: None,
            filament_end_gcode: None,
            change_filament_gcode: None,
            retract_length_toolchange: 10.0,
            standby_temperature_delta: None,
            preheat_time: 30.0,
            tool_change_temperature_wait: true,
            tools: 1,
            filaments: 1,
            variables: variables(&Settings::default(), None, None, None, None),
            config: Vec::new(),
            produced_at: None,
            unsupported: Vec::new(),
        }
    }
}

impl PrintSettings {
    /// Reads what is present; anything absent keeps its default. Lists are
    /// per extruder or filament and the first entry is used.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        Self::for_filament(settings, 0)
    }

    /// The settings for printing with one filament, counting from 0.
    /// Lists are per filament or per tool, and that filament's entry is
    /// used, or the last entry of a list too short to have one.
    pub fn for_filament(settings: &Settings, index: usize) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let first = |key: &str| -> Result<Option<f64>, ConfigError> {
            Ok(settings
                .floats(key)?
                .and_then(|values| values.get(index).or(values.last()).copied().flatten()))
        };
        // The machine's limits are listed for its normal and quiet modes,
        // not per filament.
        let limit = |key: &str| -> Result<Option<f64>, ConfigError> {
            Ok(settings
                .floats(key)?
                .and_then(|values| values.first().copied().flatten()))
        };
        let number = |key: &str, default: f64| first(key).map(|found| found.unwrap_or(default));
        // A filament may override the printer's retraction settings; an
        // entry of `nil` there means it does not.
        let overridable = |key: &str, default: f64| -> Result<f64, ConfigError> {
            match first(&format!("filament_{key}"))? {
                Some(value) => Ok(value),
                None => number(key, default),
            }
        };
        // A template is the setting's text; for per-filament ones, the first
        // filament's. An empty one is the same as none.
        let template = |key: &str| -> Option<String> {
            settings
                .texts(key)
                .and_then(|texts| texts.get(index).or(texts.last()))
                .filter(|text| !text.trim().is_empty())
                .map(|text| unescaped(text))
        };
        let mut unsupported = Vec::new();

        let wall_order = match settings
            .texts("wall_sequence")
            .and_then(|v| v.first())
            .map(String::as_str)
        {
            None | Some("inner wall/outer wall") => WallOrder::InnerOuter,
            Some("outer wall/inner wall") => WallOrder::OuterInner,
            Some("inner-outer-inner wall") => WallOrder::InnerOuterInner,
            Some(other) => {
                unsupported.push(format!(
                    "wall_sequence \"{other}\": inner wall first is used"
                ));
                WallOrder::InnerOuter
            }
        };
        let wall_direction = match settings.text("wall_direction") {
            None | Some("auto") => WallDirection::Auto,
            Some("ccw") => WallDirection::CounterClockwise,
            Some("cw") => WallDirection::Clockwise,
            Some(other) => {
                return Err(ConfigError::BadValue {
                    key: "wall_direction".into(),
                    found: other.to_owned(),
                    expected: "auto, ccw or cw",
                });
            }
        };
        let first_text = |key: &str| settings.texts(key).and_then(|v| v.get(index).or(v.last()));
        let lift = match first_text("z_hop_types").map(String::as_str) {
            None | Some("Slope Lift") => Lift::Slope,
            Some("Normal Lift") => Lift::Normal,
            Some("Spiral Lift") => Lift::Spiral,
            Some("Auto Lift") => Lift::Auto,
            Some(other) => {
                unsupported.push(format!("z_hop_types \"{other}\": a ramped lift is used"));
                Lift::Slope
            }
        };
        let seam = match settings.text("seam_position") {
            None | Some("aligned") => Seam::Aligned,
            Some("nearest") => Seam::Nearest,
            Some("aligned_back") => Seam::AlignedBack,
            Some("aligned_front") => Seam::AlignedFront,
            Some("back") => Seam::Back,
            Some("left") => Seam::Left,
            Some("right") => Seam::Right,
            Some("random") => Seam::Random,
            Some(other) => {
                unsupported.push(format!("seam_position \"{other}\": aligned seams are used"));
                Seam::Aligned
            }
        };
        // Present and not the value that turns the feature off.
        for (key, off, instead) in [
            (
                "retract_lift_enforce",
                "All Surfaces",
                "the nozzle lifts over every surface",
            ),
            ("wipe_on_loops", "0", "walls end without the inward move"),
            (
                "wipe_before_external_loop",
                "0",
                "outer walls start without the inward move",
            ),
            (
                "exclude_object",
                "0",
                "objects are not labelled for cancelling",
            ),
            (
                "slowdown_for_curled_perimeters",
                "0",
                "walls are not slowed over edges that have curled up",
            ),
        ] {
            if let Some(value) = first_text(key)
                && value != off
            {
                unsupported.push(format!("{key} \"{value}\": {instead}"));
            }
        }
        let scarf = Scarf::read(settings, &mut unsupported)?;
        if first("internal_bridge_fan_speed")?.is_some_and(|speed| speed >= 0.0) {
            unsupported.push(
                "internal_bridge_fan_speed: bridges over infill get the overhang fan speed".into(),
            );
        }
        if first("initial_layer_fan_speed")?.is_some_and(|speed| speed >= 0.0) {
            unsupported.push(
                "initial_layer_fan_speed: the first layers follow close_fan_the_first_x_layers"
                    .into(),
            );
        }
        // A share written as a percentage, such as "100%".
        let share = |key: &str, default: f64| -> Result<f64, ConfigError> {
            match first_text(key) {
                None => Ok(default),
                Some(text) => text
                    .trim()
                    .trim_end_matches('%')
                    .parse::<f64>()
                    .map(|percent| percent / 100.0)
                    .map_err(|_| ConfigError::BadValue {
                        key: key.into(),
                        found: text.clone(),
                        expected: "a percentage",
                    }),
            }
        };
        let flag = |key: &str, default: bool| -> Result<bool, ConfigError> {
            Ok(match first_text(key).map(String::as_str) {
                None => default,
                Some("1" | "true") => true,
                Some("0" | "false") => false,
                Some(other) => {
                    return Err(ConfigError::BadValue {
                        key: key.into(),
                        found: other.into(),
                        expected: "0 or 1",
                    });
                }
            })
        };
        // This filament's entry of a list of numbers that may each be a
        // percentage.
        let listed_share =
            |key: &str, default: FloatOrPercent| -> Result<FloatOrPercent, ConfigError> {
                let Some(text) = first_text(key) else {
                    return Ok(default);
                };
                let bad = || ConfigError::BadValue {
                    key: key.into(),
                    found: text.clone(),
                    expected: "a number or a percentage",
                };
                let text = text.trim();
                match text.strip_suffix('%') {
                    Some(percent) => percent.trim().parse().map(FloatOrPercent::Percent),
                    None => text.parse().map(FloatOrPercent::Value),
                }
                .map_err(|_| bad())
            };
        let nozzle_diameter = number("nozzle_diameter", defaults.nozzle_diameter)?;
        let travel_speed = number("travel_speed", defaults.travel_speed)?;
        // The bed is the box around the corners of `printable_area`, each "XxY".
        let bed = settings.texts("printable_area").and_then(|corners| {
            let mut extent: Option<[f64; 4]> = None;
            for corner in corners {
                let (x, y) = corner.split_once('x')?;
                let (x, y): (f64, f64) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
                let [min_x, min_y, max_x, max_y] = extent.unwrap_or([x, y, x, y]);
                extent = Some([min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)]);
            }
            extent
        });

        // The bed temperature lives under a key named after the plate in use.
        let bed_key = match settings.text("curr_bed_type") {
            Some("Cool Plate") => Some("cool_plate_temp"),
            Some("Engineering Plate") => Some("eng_plate_temp"),
            Some("High Temp Plate") => Some("hot_plate_temp"),
            Some("Textured PEI Plate") => Some("textured_plate_temp"),
            Some("Textured Cool Plate") => Some("textured_cool_plate_temp"),
            Some("Supertack Plate") => Some("supertack_plate_temp"),
            Some(other) => {
                unsupported.push(format!(
                    "curr_bed_type \"{other}\": no bed temperature is set"
                ));
                None
            }
            None => None,
        };
        let (bed_temperature, bed_temperature_initial_layer) = match bed_key {
            Some(key) => (first(key)?, first(&format!("{key}_initial_layer"))?),
            None => (None, None),
        };

        Ok(Self {
            outer_wall_speed: number("outer_wall_speed", defaults.outer_wall_speed)?,
            inner_wall_speed: number("inner_wall_speed", defaults.inner_wall_speed)?,
            sparse_infill_speed: number("sparse_infill_speed", defaults.sparse_infill_speed)?,
            internal_solid_infill_speed: number(
                "internal_solid_infill_speed",
                defaults.internal_solid_infill_speed,
            )?,
            top_surface_speed: number("top_surface_speed", defaults.top_surface_speed)?,
            // "The active filament's ironing speed override wins when
            // set" (GCode.cpp `_extrude`); a filament the list has no
            // entry for keeps the process's speed, and does not borrow
            // another's (PrintConfig.hpp `ironing_filament_override`).
            ironing_speed: match settings
                .floats("filament_ironing_speed")?
                .and_then(|speeds| speeds.get(index).copied().flatten())
                .filter(|speed| *speed > 0.0)
            {
                Some(own) => own,
                None => number("ironing_speed", defaults.ironing_speed)?,
            },
            ironing_fan_speed: first("ironing_fan_speed")?.filter(|speed| *speed >= 0.0),
            bridge_speed: number("bridge_speed", defaults.bridge_speed)?,
            gap_infill_speed: number("gap_infill_speed", defaults.gap_infill_speed)?,
            detect_overhang_wall: flag("detect_overhang_wall", defaults.detect_overhang_wall)?,
            enable_overhang_speed: flag("enable_overhang_speed", defaults.enable_overhang_speed)?,
            overhang_speeds: [
                listed_share("overhang_1_4_speed", defaults.overhang_speeds[0])?,
                listed_share("overhang_2_4_speed", defaults.overhang_speeds[1])?,
                listed_share("overhang_3_4_speed", defaults.overhang_speeds[2])?,
                listed_share("overhang_4_4_speed", defaults.overhang_speeds[3])?,
            ],
            slowdown_for_curled_perimeters: flag("slowdown_for_curled_perimeters", false)?,
            overhang_fan: if flag("enable_overhang_bridge_fan", true)? {
                Some(OverhangFan {
                    speed: number("overhang_fan_speed", 100.0)?,
                    // How much of a line may hang over before the fan
                    // comes on, written as that share (GCode.cpp
                    // `overhang_fan_overlap_threshold`).
                    at: match first_text("overhang_fan_threshold").map(String::as_str) {
                        Some("0%") => None,
                        Some("10%") => Some(0.9),
                        Some("25%") => Some(0.75),
                        Some("50%") => Some(0.5),
                        Some("75%") => Some(0.25),
                        None | Some("95%") => Some(0.05),
                        Some(other) => {
                            return Err(ConfigError::BadValue {
                                key: "overhang_fan_threshold".into(),
                                found: other.into(),
                                expected: "0%, 10%, 25%, 50%, 75% or 95%",
                            });
                        }
                    },
                })
            } else {
                None
            },
            bridge_flow: number("bridge_flow", defaults.bridge_flow)?,
            thick_bridges: flag("thick_bridges", defaults.thick_bridges)?,
            bridge_line_width: edge_extrusion::bridge_line_width(settings)?,
            bridge_acceleration: listed_share("bridge_acceleration", defaults.bridge_acceleration)?,
            internal_bridge_speed: listed_share(
                "internal_bridge_speed",
                defaults.internal_bridge_speed,
            )?,
            small_perimeter_threshold: number(
                "small_perimeter_threshold",
                defaults.small_perimeter_threshold,
            )?,
            small_perimeter_speed: listed_share(
                "small_perimeter_speed",
                defaults.small_perimeter_speed,
            )?,
            initial_layer_speed: number("initial_layer_speed", defaults.initial_layer_speed)?,
            initial_layer_infill_speed: number(
                "initial_layer_infill_speed",
                defaults.initial_layer_infill_speed,
            )?,
            travel_speed,
            z_speed: {
                let travel = travel_speed;
                let wanted = first("travel_speed_z")?
                    .filter(|speed| *speed > 0.0)
                    .unwrap_or(travel);
                match limit("machine_max_speed_z")?.filter(|limit| *limit > 0.0) {
                    Some(limit) => wanted.min(limit),
                    None => wanted,
                }
            },
            filament_max_volumetric_speed: number(
                "filament_max_volumetric_speed",
                defaults.filament_max_volumetric_speed,
            )?,
            retraction_length: overridable("retraction_length", defaults.retraction_length)?,
            retraction_speed: overridable("retraction_speed", defaults.retraction_speed)?,
            deretraction_speed: overridable("deretraction_speed", defaults.deretraction_speed)?,
            retraction_minimum_travel: overridable(
                "retraction_minimum_travel",
                defaults.retraction_minimum_travel,
            )?,
            z_hop: overridable("z_hop", defaults.z_hop)?,
            lift,
            travel_slope: number("travel_slope", defaults.travel_slope)?,
            retract_lift_above: number("retract_lift_above", 0.0)?,
            retract_lift_below: number("retract_lift_below", 0.0)?,
            retract_when_changing_layer: flag(
                "retract_when_changing_layer",
                defaults.retract_when_changing_layer,
            )?,
            wipe: flag("wipe", defaults.wipe)?,
            wipe_distance: number("wipe_distance", defaults.wipe_distance)?,
            retract_before_wipe: share("retract_before_wipe", defaults.retract_before_wipe)?
                .clamp(0.0, 1.0),
            retract_after_wipe: share("retract_after_wipe", defaults.retract_after_wipe)?
                .clamp(0.0, 1.0),
            wipe_speed: if flag("role_based_wipe_speed", true)? {
                None
            } else {
                // The reference never wipes slower than 10 mm/s.
                Some(
                    settings
                        .float_or_percent("wipe_speed")?
                        .unwrap_or(FloatOrPercent::Percent(80.0))
                        .resolve(travel_speed)
                        .max(10.0),
                )
            },
            nozzle_diameter,
            seam,
            staggered_inner_seams: flag("staggered_inner_seams", false)?,
            seam_gap: settings
                .float_or_percent("seam_gap")?
                .unwrap_or(FloatOrPercent::Percent(10.0))
                .resolve(nozzle_diameter)
                .max(0.0),
            scarf,
            resolution: number("resolution", defaults.resolution)?,
            arc_fitting: flag("enable_arc_fitting", defaults.arc_fitting)?
                && !flag("spiral_mode", false)?,
            bed,
            filament_diameter: number("filament_diameter", defaults.filament_diameter)?,
            filament_flow_ratio: number("filament_flow_ratio", defaults.filament_flow_ratio)?,
            print_flow_ratio: number("print_flow_ratio", defaults.print_flow_ratio)?,
            top_solid_infill_flow_ratio: number("top_solid_infill_flow_ratio", 1.0)?,
            bottom_solid_infill_flow_ratio: number("bottom_solid_infill_flow_ratio", 1.0)?,
            internal_bridge_flow: number("internal_bridge_flow", 1.0)?,
            brim_flow_ratio: number("brim_flow_ratio", 1.0)?,
            filament_density: first("filament_density")?.filter(|density| *density > 0.0),
            nozzle_temperature: number("nozzle_temperature", defaults.nozzle_temperature)?,
            nozzle_temperature_initial_layer: number(
                "nozzle_temperature_initial_layer",
                defaults.nozzle_temperature_initial_layer,
            )?,
            bed_temperature,
            bed_temperature_initial_layer,
            wall_order,
            wall_direction,
            infill_first: settings
                .flag("is_infill_first")?
                .unwrap_or(defaults.infill_first),
            // Only with sparse fill to shelter in, as in the reference.
            reduce_infill_retraction: settings
                .flag("reduce_infill_retraction")?
                .unwrap_or(defaults.reduce_infill_retraction)
                && settings
                    .float_or_percent("sparse_infill_density")?
                    .is_none_or(|density| density.resolve(100.0) > 0.0),
            reduce_crossing_wall: settings
                .flag("reduce_crossing_wall")?
                .unwrap_or(defaults.reduce_crossing_wall),
            // Nought, as a length or a share, is no limit.
            max_travel_detour: match settings.float_or_percent("max_travel_detour_distance")? {
                Some(FloatOrPercent::Value(mm)) if mm > 0.0 => Some(Detour::Length(mm)),
                Some(FloatOrPercent::Percent(share)) if share > 0.0 => Some(Detour::Percent(share)),
                _ => defaults.max_travel_detour,
            },
            fan_min_speed: number("fan_min_speed", defaults.fan_min_speed)?,
            fan_max_speed: number("fan_max_speed", defaults.fan_max_speed)?,
            fan_cooling_layer_time: number(
                "fan_cooling_layer_time",
                defaults.fan_cooling_layer_time,
            )?,
            slow_down_layer_time: number("slow_down_layer_time", defaults.slow_down_layer_time)?,
            close_fan_the_first_x_layers: number(
                "close_fan_the_first_x_layers",
                defaults.close_fan_the_first_x_layers as f64,
            )?
            .max(0.0) as usize,
            full_fan_speed_layer: number("full_fan_speed_layer", 0.0)?.max(0.0) as usize,
            reduce_fan_stop_start_freq: flag(
                "reduce_fan_stop_start_freq",
                defaults.reduce_fan_stop_start_freq,
            )?,
            slow_down_for_layer_cooling: flag(
                "slow_down_for_layer_cooling",
                defaults.slow_down_for_layer_cooling,
            )?,
            slow_down_min_speed: number("slow_down_min_speed", defaults.slow_down_min_speed)?,
            dont_slow_down_outer_wall: flag("dont_slow_down_outer_wall", false)?,
            circle_compensation_speed: number(
                "circle_compensation_speed",
                defaults.circle_compensation_speed,
            )?
            .max(0.0),
            auxiliary_fan: flag("auxiliary_fan", false)?,
            additional_cooling_fan_speed: number("additional_cooling_fan_speed", 0.0)?,
            skirt_speed: number("skirt_speed", defaults.skirt_speed)?,
            support_speed: number("support_speed", defaults.support_speed)?,
            support_interface_speed: number(
                "support_interface_speed",
                defaults.support_interface_speed,
            )?,
            wipe_tower_max_purge_speed: number(
                "wipe_tower_max_purge_speed",
                defaults.wipe_tower_max_purge_speed,
            )?,
            default_acceleration: number("default_acceleration", 0.0)?,
            outer_wall_acceleration: number("outer_wall_acceleration", 0.0)?,
            inner_wall_acceleration: number("inner_wall_acceleration", 0.0)?,
            initial_layer_acceleration: number("initial_layer_acceleration", 0.0)?,
            top_surface_acceleration: number("top_surface_acceleration", 0.0)?,
            sparse_infill_acceleration: settings
                .float_or_percent("sparse_infill_acceleration")?
                .unwrap_or(defaults.sparse_infill_acceleration),
            internal_solid_infill_acceleration: settings
                .float_or_percent("internal_solid_infill_acceleration")?
                .unwrap_or(defaults.internal_solid_infill_acceleration),
            travel_acceleration: number("travel_acceleration", 0.0)?,
            accel_to_decel_factor: match settings.flag("accel_to_decel_enable")? {
                Some(true) => match settings.float_or_percent("accel_to_decel_factor")? {
                    Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => {
                        Some(percent / 100.0)
                    }
                    None => Some(0.5),
                },
                _ => None,
            },
            klipper: settings.text("gcode_flavor") == Some("klipper"),
            limits: match settings.get("gcode_flavor") {
                Some(_) => Some(edge_time::Limits::from_settings(settings)?),
                None => None,
            },
            pressure_advance: match settings.flag("enable_pressure_advance")? {
                Some(true) => Some(number("pressure_advance", 0.02)?),
                _ => None,
            },
            machine_start_gcode: template("machine_start_gcode"),
            machine_end_gcode: template("machine_end_gcode"),
            before_layer_change_gcode: template("before_layer_change_gcode"),
            layer_change_gcode: template("layer_change_gcode"),
            filament_start_gcode: template("filament_start_gcode"),
            filament_end_gcode: template("filament_end_gcode"),
            change_filament_gcode: settings
                .text("change_filament_gcode")
                .filter(|text| !text.trim().is_empty())
                .map(unescaped),
            retract_length_toolchange: number(
                "retract_length_toolchange",
                defaults.retract_length_toolchange,
            )?,
            standby_temperature_delta: match settings.flag("ooze_prevention")? {
                Some(true) => Some(settings.float("standby_temperature_delta")?.unwrap_or(-5.0))
                    .filter(|delta| *delta != 0.0),
                _ => None,
            },
            preheat_time: settings
                .float("preheat_time")?
                .unwrap_or(defaults.preheat_time),
            tool_change_temperature_wait: settings
                .flag("tool_change_temprature_wait")?
                .unwrap_or(defaults.tool_change_temperature_wait),
            tools: settings
                .texts("nozzle_diameter")
                .map_or(defaults.tools, |nozzles| nozzles.len().max(1)),
            filaments: settings
                .texts("filament_diameter")
                .map_or(defaults.filaments, |filaments| filaments.len().max(1)),
            variables: variables(
                settings,
                bed_temperature,
                bed_temperature_initial_layer,
                bed_key,
                bed,
            ),
            config: {
                // The settings the reference knows and would list. That
                // leaves out the print host's address and keys, which a
                // project carries but a file sent to a printer must not,
                // and what only names the preset a setting came from.
                let mut config: Vec<(String, String)> = settings
                    .iter()
                    .filter(|(key, _)| edge_config::reference_knows(key))
                    .map(|(key, value)| (key.to_owned(), edge_config::serialized(key, value)))
                    .collect();
                config.sort();
                config
            },
            produced_at: None,
            unsupported,
        })
    }
}

/// A template as the reference reads it from a preset: a backslash makes
/// `\n` a line break and `\r` a carriage return, and stands any other
/// character after it for itself (Config.cpp `unescape_string_cstyle`).
/// Vendor presets write their G-code that way; a project's is already plain.
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Every setting under its own name, plus the names templates use that are
/// not settings: the tool in use (always the first, for now) and the bed
/// temperature of the plate in use.
fn variables(
    settings: &Settings,
    bed: Option<f64>,
    bed_initial: Option<f64>,
    bed_key: Option<&str>,
    area: Option<[f64; 4]>,
) -> Variables {
    let mut variables = Variables::default();
    for (key, value) in settings.iter() {
        match value {
            Setting::One(text) => variables.set(key, Value::text(text)),
            Setting::Many(texts) => {
                variables.set_list(key, texts.iter().map(Value::text).collect())
            }
        }
    }
    // Pressure advance is an on/off setting per filament that templates
    // compare with the text "true", and one a profile may leave out: it
    // is given as the reference gives it, with the reference's defaults
    // (PrintConfig.cpp) when absent.
    let enabled: Vec<Value> =
        settings
            .texts("enable_pressure_advance")
            .map_or(vec![Value::Bool(false)], |flags| {
                flags
                    .iter()
                    .map(|flag| Value::Bool(matches!(flag.trim(), "1" | "true")))
                    .collect()
            });
    variables.set_list("enable_pressure_advance", enabled);
    if !variables.contains("pressure_advance") {
        variables.set_list("pressure_advance", vec![Value::Float(0.02)]);
    }
    for name in [
        "initial_extruder",
        "initial_tool",
        "initial_no_support_extruder",
        "current_extruder",
    ] {
        variables.set(name, Value::Int(0));
    }
    if let Some([min_x, min_y, max_x, max_y]) = area {
        let pair = |x: f64, y: f64| vec![Value::Float(x), Value::Float(y)];
        variables.set_list("print_bed_min", pair(min_x, min_y));
        variables.set_list("print_bed_max", pair(max_x, max_y));
        variables.set_list("print_bed_size", pair(max_x - min_x, max_y - min_y));
    }
    let whole = |temperature: f64| Value::Int(temperature.round() as i64);
    // The lists have an entry for each filament, as the plate's own
    // setting has, so a template can ask for any tool's; a filament with
    // none of its own takes this one's.
    let for_each = |suffix: &str, own: f64| -> Vec<Value> {
        let listed = bed_key.and_then(|key| settings.floats(&format!("{key}{suffix}")).ok()?);
        match listed {
            Some(all) if !all.is_empty() => all
                .iter()
                .map(|temperature| whole(temperature.unwrap_or(own)))
                .collect(),
            _ => vec![whole(own)],
        }
    };
    if let Some(bed) = bed_initial.or(bed) {
        variables.set("bed_temperature_initial_layer_single", whole(bed));
        variables.set_list(
            "bed_temperature_initial_layer",
            for_each("_initial_layer", bed),
        );
        variables.set("first_layer_bed_temperature", whole(bed));
    }
    if let Some(bed) = bed.or(bed_initial) {
        variables.set_list("bed_temperature", for_each("", bed));
    }
    // One filament and no tool changes, for now.
    variables.set("has_wipe_tower", Value::Bool(false));
    variables.set("total_toolchanges", Value::Int(0));
    let number = |key: &str| settings.float(key).ok().flatten();
    let first = |key: &str| {
        settings
            .floats(key)
            .ok()
            .flatten()
            .and_then(|values| values.first().copied().flatten())
    };
    if let Some(height) = number("initial_layer_print_height") {
        variables.set("first_layer_height", Value::Float(height));
    }
    if let Some(height) = number("printable_height") {
        variables.set("max_print_height", Value::Int(height as i64));
    }
    if let Some(nozzles) = settings.texts("nozzle_diameter") {
        variables.set("num_extruders", Value::Int(nozzles.len() as i64));
    }
    if let Some(lengths) = settings.texts("retraction_length") {
        variables.set_list("retract_length", lengths.iter().map(Value::text).collect());
    }
    // The flow each filament slot was sliced for, which the U1's firmware
    // reads from the end G-code: standard unless the slot says high flow.
    if let Some(filaments) = settings.texts("filament_type") {
        let kinds = settings.texts("filament_volume_type").unwrap_or(&[]);
        let list: Vec<&str> = (0..filaments.len())
            .map(|slot| match kinds.get(slot).map(String::as_str) {
                Some("high_flow") => "high_flow",
                _ => "standard",
            })
            .collect();
        variables.set("filament_volume_type_list", Value::text(list.join(",")));
    }
    // How hard the chamber is cooled, from how easily the filament in use
    // softens: 2 at a glass transition of 50 °C or under, 1 up to 70, else
    // 0 (GCode.cpp, where the start G-code's variables are set).
    let softens = first("temperature_vitrification").unwrap_or(100.0);
    let high_temperature = settings
        .texts("filament_is_high_temperature")
        .and_then(|values| values.first())
        .is_some_and(|value| value == "1");
    let cooling = match softens {
        _ if high_temperature => 0,
        at if at <= 50.0 => 2,
        at if at <= 70.0 => 1,
        _ => 0,
    };
    variables.set("chamber_cooling_mode", Value::Int(cooling));
    // The hottest chamber any filament asks for; none by default.
    let chamber = settings
        .floats("chamber_temperature")
        .ok()
        .flatten()
        .and_then(|values| values.into_iter().flatten().reduce(f64::max))
        .unwrap_or(0.0);
    variables.set(
        "overall_chamber_temperature",
        Value::Int(chamber.round() as i64),
    );
    // How much plastic the outer wall takes per second, held to what the
    // filament can melt (GCode.cpp, where the start G-code's variables are set).
    if let (Some(speed), Some(height), Some(nozzle)) = (
        number("outer_wall_speed").or(first("outer_wall_speed")),
        number("layer_height"),
        first("nozzle_diameter"),
    ) {
        let width = ["outer_wall_line_width", "line_width"]
            .into_iter()
            .filter_map(|key| settings.float_or_percent(key).ok().flatten())
            .map(|width| width.resolve(nozzle))
            .find(|width| *width > 0.0)
            .unwrap_or(nozzle);
        if let Ok(flow) = Flow::new(width, height, nozzle) {
            let wanted = speed * flow.mm3_per_mm();
            let volumetric = match first("filament_max_volumetric_speed") {
                Some(limit) if limit > 0.0 => wanted.min(limit),
                _ => wanted,
            };
            variables.set("outer_wall_volumetric_speed", Value::Float(volumetric));
        }
    }
    // Names older profiles still use for the nozzle temperatures.
    for (old, new) in [
        (
            "first_layer_temperature",
            "nozzle_temperature_initial_layer",
        ),
        ("temperature", "nozzle_temperature"),
    ] {
        if let Some(texts) = settings.texts(new) {
            variables.set_list(old, texts.iter().map(Value::text).collect());
        }
    }
    variables
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_settings_keep_reference_defaults() {
        assert_eq!(
            PrintSettings::from_settings(&Settings::default()).unwrap(),
            PrintSettings::default()
        );
    }

    #[test]
    fn reads_first_entries_and_the_plate_in_use() {
        let read = |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap());
        let settings = read(
            r#"{"outer_wall_speed": "200", "inner_wall_speed": ["300", "600"], "retraction_length": ["1.5", "1.5"],
                "filament_flow_ratio": ["0.98"], "filament_density": ["1.24"], "nozzle_temperature": ["220", "240"],
                "curr_bed_type": "Cool Plate", "cool_plate_temp": ["35"], "cool_plate_temp_initial_layer": ["40"],
                "hot_plate_temp": ["55"], "wall_sequence": "outer wall/inner wall", "is_infill_first": "1",
                "z_hop_types": ["Normal Lift"]}"#,
        )
        .unwrap();
        assert_eq!(
            (settings.outer_wall_speed, settings.inner_wall_speed),
            (200.0, 300.0)
        );
        assert_eq!(
            (settings.retraction_length, settings.filament_flow_ratio),
            (1.5, 0.98)
        );
        assert_eq!(
            (settings.filament_density, settings.nozzle_temperature),
            (Some(1.24), 220.0)
        );
        assert_eq!(
            (
                settings.bed_temperature,
                settings.bed_temperature_initial_layer
            ),
            (Some(35.0), Some(40.0))
        );
        assert_eq!(
            (settings.wall_order, settings.infill_first),
            (WallOrder::OuterInner, true)
        );
        assert!(settings.unsupported.is_empty());
    }

    #[test]
    fn reads_templates_fan_acceleration_and_pressure_advance() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let settings = read(
            r#"{"machine_start_gcode": "G28\nM140 S{bed_temperature_initial_layer_single}", "machine_end_gcode": " ",
                "filament_start_gcode": ["; first", "; second"], "curr_bed_type": "Cool Plate",
                "cool_plate_temp_initial_layer": ["40"], "nozzle_temperature": ["220"],
                "fan_min_speed": ["80"], "close_fan_the_first_x_layers": ["3"],
                "default_acceleration": "10000", "sparse_infill_acceleration": "50%", "gcode_flavor": "klipper",
                "accel_to_decel_enable": "1", "accel_to_decel_factor": "40%",
                "enable_pressure_advance": ["1"], "pressure_advance": ["0.03"]}"#,
        );
        assert_eq!(
            settings.machine_start_gcode.as_deref(),
            Some("G28\nM140 S{bed_temperature_initial_layer_single}")
        );
        // Vendor presets escape their line breaks and quotes.
        assert_eq!(
            read(r#"{"machine_start_gcode": "G28\\n{if curr_bed_type==\\\"Cool Plate\\\"}G29\\n{endif}"}"#)
                .machine_start_gcode
                .as_deref(),
            Some("G28\n{if curr_bed_type==\"Cool Plate\"}G29\n{endif}")
        );
        // Blank templates count as absent; per-filament ones take the first filament's.
        assert_eq!(
            (
                settings.machine_end_gcode,
                settings.filament_start_gcode.as_deref()
            ),
            (None, Some("; first"))
        );
        assert_eq!(
            (
                settings.fan_min_speed,
                settings.close_fan_the_first_x_layers
            ),
            (80.0, 3)
        );
        assert_eq!(
            settings.sparse_infill_acceleration,
            FloatOrPercent::Percent(50.0)
        );
        assert_eq!(
            (
                settings.klipper,
                settings.accel_to_decel_factor,
                settings.pressure_advance
            ),
            (true, Some(0.4), Some(0.03))
        );

        let expand = |template: &str| edge_template::expand(template, &settings.variables).unwrap();
        assert_eq!(
            expand("M140 S{bed_temperature_initial_layer_single} T{initial_extruder}"),
            "M140 S40 T0"
        );
        assert_eq!(
            expand("M104 S{nozzle_temperature[initial_extruder] - 90} ; {curr_bed_type}"),
            "M104 S130 ; Cool Plate"
        );
        // An older name for the nozzle temperature still works.
        assert_eq!(expand("[temperature]"), "220");

        let off = read(r#"{"enable_pressure_advance": ["0"], "accel_to_decel_enable": "0"}"#);
        assert_eq!(
            (off.pressure_advance, off.accel_to_decel_factor, off.klipper),
            (None, None, false)
        );
    }

    #[test]
    fn filaments_override_retraction_and_z_moves_respect_the_machine() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let settings = read(
            r#"{"retraction_length": ["0.8"], "filament_retraction_length": ["1", "2"],
                "retraction_speed": ["50"], "filament_retraction_speed": ["40"],
                "z_hop": ["0.3"], "filament_z_hop": ["nil", "0.4"],
                "deretraction_speed": ["30"], "filament_deretraction_speed": ["nil"],
                "travel_speed": "500", "travel_speed_z": "0", "machine_max_speed_z": ["20", "12"]}"#,
        );
        assert_eq!(
            (settings.retraction_length, settings.retraction_speed),
            (1.0, 40.0)
        );
        assert_eq!((settings.z_hop, settings.deretraction_speed), (0.3, 30.0));
        assert_eq!((settings.travel_speed, settings.z_speed), (500.0, 20.0));
        let own = read(
            r#"{"travel_speed": "200", "travel_speed_z": "8", "machine_max_speed_z": ["20"]}"#,
        );
        assert_eq!(own.z_speed, 8.0);
        assert_eq!(read(r#"{"travel_speed": "200"}"#).z_speed, 200.0);
    }

    #[test]
    fn reads_seams_wipe_lift_and_cooling() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let settings = read(
            r#"{"seam_position": "aligned_back", "seam_gap": "15%", "nozzle_diameter": ["0.4", "0.4"],
                "wipe": ["1"], "wipe_distance": ["2"], "retract_before_wipe": ["70%"], "wipe_speed": "80%",
                "travel_speed": "500", "z_hop_types": ["Auto Lift"], "travel_slope": ["3"],
                "retract_lift_below": ["269"], "retract_when_changing_layer": ["1"],
                "printable_area": ["0.5x1", "270.5x1", "270.5x271", "0.5x271"],
                "slow_down_for_layer_cooling": ["0", "1"], "slow_down_min_speed": ["40"],
                "reduce_fan_stop_start_freq": ["1"], "auxiliary_fan": "1",
                "additional_cooling_fan_speed": ["50", "25"], "resolution": "0.012"}"#,
        );
        assert_eq!(settings.seam, Seam::AlignedBack);
        assert!((settings.seam_gap - 0.06).abs() < 1e-12);
        assert_eq!(
            (
                settings.wipe,
                settings.wipe_distance,
                settings.retract_before_wipe
            ),
            (true, 2.0, 0.7)
        );
        // Wiping at the speed of the line just printed, unless told otherwise.
        assert_eq!(settings.wipe_speed, None);
        assert_eq!(
            read(r#"{"role_based_wipe_speed": "0", "wipe_speed": "80%", "travel_speed": "500"}"#)
                .wipe_speed,
            Some(400.0)
        );
        assert_eq!(
            (settings.lift, settings.retract_lift_below),
            (Lift::Auto, 269.0)
        );
        assert!(settings.retract_when_changing_layer);
        assert_eq!(settings.bed, Some([0.5, 1.0, 270.5, 271.0]));
        assert_eq!(
            (
                settings.slow_down_for_layer_cooling,
                settings.slow_down_min_speed
            ),
            (false, 40.0)
        );
        assert!(settings.reduce_fan_stop_start_freq && settings.auxiliary_fan);
        assert_eq!(settings.additional_cooling_fan_speed, 50.0);
        assert!(
            settings.unsupported.is_empty(),
            "{:?}",
            settings.unsupported
        );

        let expand = |template: &str| edge_template::expand(template, &settings.variables).unwrap();
        assert_eq!(
            expand("{print_bed_max[0]} {print_bed_size[1]}"),
            "270.5 270"
        );
        let bambu = read(
            r#"{"outer_wall_speed": "200", "layer_height": "0.2", "nozzle_diameter": ["0.4"],
                "outer_wall_line_width": "0.42", "filament_max_volumetric_speed": ["12"],
                "chamber_temperature": ["0", "45"]}"#,
        );
        let expand = |template: &str| edge_template::expand(template, &bambu.variables).unwrap();
        // 200 mm/s of a 0.42 x 0.2 line is more than the filament melts.
        assert_eq!(expand("{outer_wall_volumetric_speed}"), "12");
        assert_eq!(expand("{overall_chamber_temperature}"), "45");
        assert_eq!(expand("{chamber_cooling_mode}"), "0");
        let slots = read(
            r#"{"filament_type": ["PLA", "PLA", "PETG"], "filament_volume_type": ["standard", "high_flow"]}"#,
        );
        assert_eq!(
            edge_template::expand("{filament_volume_type_list}", &slots.variables).unwrap(),
            "standard,high_flow,standard"
        );
        let pla = read(r#"{"temperature_vitrification": ["45", "100"]}"#);
        assert_eq!(
            edge_template::expand("{chamber_cooling_mode}", &pla.variables).unwrap(),
            "2"
        );
    }

    #[test]
    fn settings_not_honoured_are_listed_not_hidden() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let settings = read(
            r#"{"wall_sequence": "outer wall last", "seam_position": "hidden", "curr_bed_type": "Glass"}"#,
        );
        assert_eq!(settings.wall_order, WallOrder::InnerOuter);
        assert_eq!(
            read(r#"{"wall_sequence": "inner-outer-inner wall"}"#).wall_order,
            WallOrder::InnerOuterInner
        );
        let seams = read(r#"{"seam_position": "random", "staggered_inner_seams": "1"}"#);
        assert_eq!(
            (seams.seam, seams.staggered_inner_seams),
            (Seam::Random, true)
        );
        assert!(seams.unsupported.is_empty());
        assert_eq!(settings.bed_temperature, None);
        assert_eq!(settings.unsupported.len(), 3);
        assert!(
            PrintSettings::from_settings(
                &Settings::from_json(r#"{"travel_speed": "fast"}"#).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn settings_are_listed_for_the_file_without_what_is_private() {
        let settings = PrintSettings::from_settings(
            &Settings::from_json(
                r#"{"filament_type": ["PLA", "PETG"], "nozzle_diameter": ["0.4", "0.4"],
                    "filament_settings_id": ["Generic PLA", "Generic PETG"],
                    "filament_notes": ["", ""], "z_hop_types": ["Auto Lift", "Auto Lift"],
                    "printable_area": ["0x0", "270x0", "270x270", "0x270"],
                    "print_settings_id": "0.20 Standard @Snapmaker U1 (0.4 nozzle)",
                    "accel_to_decel_factor": "50%", "retraction_length": ["0.8", "nil"],
                    "machine_end_gcode": "PRINT_END\nTIMELAPSE_STOP",
                    "printhost_apikey": "secret", "print_host": "192.168.1.20",
                    "printhost_password": "secret", "name": "project_settings", "from": "project"}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let listed: Vec<(&str, &str)> = settings
            .config
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("accel_to_decel_factor", "50%"),
                ("filament_notes", ";"),
                ("filament_settings_id", "\"Generic PLA\";\"Generic PETG\""),
                ("filament_type", "PLA;PETG"),
                ("machine_end_gcode", "PRINT_END\\nTIMELAPSE_STOP"),
                ("nozzle_diameter", "0.4,0.4"),
                (
                    "print_settings_id",
                    "0.20 Standard @Snapmaker U1 (0.4 nozzle)"
                ),
                ("printable_area", "0x0,270x0,270x270,0x270"),
                ("retraction_length", "0.8,nil"),
                ("z_hop_types", "Auto Lift,Auto Lift"),
            ]
        );
    }
}
