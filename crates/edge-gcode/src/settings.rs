use edge_avoid::Detour;
use edge_config::{ConfigError, FloatOrPercent, Settings, Value as Setting};
use edge_extrusion::{ExtrusionPath, Flow};
use edge_geom::Point2;
use edge_template::{Value, Variables};

use crate::{fan_mover::FanTiming, scarf::Scarf, seam::Seam, small_area::SmallArea};

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
    /// The part fan while support interfaces are laid, in percent; none
    /// when they get the layer's (`support_material_interface_fan_speed`
    /// below zero).
    pub support_interface_fan_speed: Option<f64>,
    pub bridge_speed: f64,
    /// Lines that fill the gaps between walls.
    pub gap_infill_speed: f64,
    /// Whether a stretch of wall with nothing under it is printed as a
    /// bridge, in its own kind of line.
    pub detect_overhang_wall: bool,
    /// Whether a wall loop wholly over air is laid after the island's
    /// other walls, innermost first, and one that touches none of them
    /// after the island's fill (`unsupported_wall_last`).
    pub unsupported_wall_last: bool,
    /// Whether walls slow down as less of them is supported, and the
    /// speeds where a quarter, a half, three quarters and nearly all of
    /// the line hangs over; zero for the wall's own speed.
    pub enable_overhang_speed: bool,
    pub overhang_speeds: [FloatOrPercent; 4],
    /// Outer walls kept out of the printer's resonance band
    /// (`resonance_avoidance`): a wall speed from `min_resonance_avoidance_speed`
    /// up to below `max_resonance_avoidance_speed` is moved to the nearer
    /// end of it (GCode.cpp `_extrude`, "resonance-avoidance").
    pub resonance_avoidance: Option<(f64, f64)>,
    /// With this on, the reference prints a wall with nothing under it
    /// at the last of the overhang speeds, not at the bridge's
    /// (`slowdown_for_curled_perimeters`). Slowing over edges that have
    /// curled up, which the setting is named for, is not done.
    pub slowdown_for_curled_perimeters: bool,
    /// EdgeSlicer's overhang shadow (`overhang_shadow_inner_wall`): an
    /// inner wall is graded as if it were further out over the layer
    /// beneath, by `overhang_shadow_speed_ratio` of
    /// `overhang_shadow_distance` (each in percent, the distance of the
    /// line's width), so the wall an overhang is laid against is slowed
    /// and cooled as part of it. None when off.
    pub overhang_shadow: Option<(f64, f64)>,
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
    /// The same for a support line, its speed as a percentage of the
    /// support's or the interface's (`small_support_perimeter_threshold`,
    /// `small_support_perimeter_speed`).
    pub small_support_perimeter_threshold: f64,
    pub small_support_perimeter_speed: FloatOrPercent,
    /// Walls on the first layer.
    pub initial_layer_speed: f64,
    /// All fill on the first layer.
    pub initial_layer_infill_speed: f64,
    /// Over how many layers the speeds climb from the first layer's to
    /// their own (`slow_down_layers`; 1 or less for none), counted from
    /// the first layer above the raft's (`raft_layers`).
    pub slow_down_layers: usize,
    pub raft_layers: usize,
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
    /// Added to every height the nozzle is sent to: the printer's
    /// `z_offset` and this filament's `filament_z_offset`. The print
    /// takes the first printing filament's (GCode.cpp `_do_export`,
    /// "fold the first printing filament's Z offset into the job's").
    pub z_offset: f64,
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
    /// A wall loop ends with a small move in toward the part before the
    /// nozzle leaves it (`wipe_on_loops`, `wipe.rs`).
    pub wipe_on_loops: bool,
    /// An outer wall is come at from a little way inside the part, and
    /// the filament fed forward again there
    /// (`wipe_before_external_loop`).
    pub wipe_before_external_loop: bool,
    /// The nozzle is wiped after an outer wall over the walls behind it,
    /// this far in from the wall, in mm or as a percentage of the wall's
    /// width (`wipe_inward`, `wipe_inward_distance`); none where it is
    /// wiped along the wall.
    pub wipe_inward: Option<FloatOrPercent>,
    /// How many walls a part is set to have (`wall_loops`).
    pub wall_loops: usize,

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
    /// The corners of `printable_area`, and the box round
    /// `bed_exclude_area` where it has three corners or more.
    pub bed_outline: Vec<[f64; 2]>,
    pub bed_exclusions: Vec<[f64; 4]>,
    /// How tall a print the printer takes (`printable_height`); 0 for no
    /// limit.
    pub printable_height: f64,
    /// The ground the prime tower takes, brim included
    /// (`edge_tower::Tower::footprint`): min x, min y, max x, max y in mm.
    /// Set by the pipeline; none for no tower.
    pub tower_footprint: Option<[f64; 4]>,
    /// The minimum layer time a layer is paused to make up, in seconds
    /// (`min_layer_time_pause`, `pause.rs`); 0 for none. And where the head
    /// waits meanwhile.
    pub min_layer_time_pause: f64,
    pub pause_park: crate::pause::Park,
    /// Where the nozzle goes round on its way into the prime tower
    /// (`edge_tower::Tower::envelope`): min x, min y, max x, max y in mm.
    /// Set by the pipeline; none for no tower, Bambu Lab's tower, or a
    /// wall without gaps.
    pub tower_envelope: Option<[f64; 4]>,
    /// The last filament unloaded over the prime tower once the print is
    /// done (`edge_tower::FinalPurge`); set by the pipeline.
    pub final_purge: Option<FinalPurge>,

    pub filament_diameter: f64,
    pub filament_flow_ratio: f64,
    pub print_flow_ratio: f64,
    /// Further shares of the plastic for particular kinds of line
    /// (GCode.cpp `_extrude`).
    pub top_solid_infill_flow_ratio: f64,
    pub bottom_solid_infill_flow_ratio: f64,
    pub internal_bridge_flow: f64,
    pub brim_flow_ratio: f64,
    /// The curve short lines of solid fill are thinned by, where the
    /// profile asks (`small_area_infill_flow_compensation`).
    pub small_area: Option<SmallArea>,
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
    /// The filament is changed by hand (`manual_filament_change`).
    pub manual_filament_change: bool,
    /// The least the part fan is run at where it runs at all, as `M106 S`
    /// takes it (`part_cooling_fan_min_pwm`, in percent; 0 for none).
    pub part_fan_floor: u8,
    /// Whether this filament has the chamber heated to its
    /// `chamber_temperature`, in °C, before the start G-code and the heater
    /// put off at the end (GCode.cpp `_do_export`; the print takes the
    /// hottest of its filaments' once any of them asks).
    pub activate_chamber_temp_control: bool,
    pub chamber_temperature: f64,
    /// The part fan told early and kick-started (`fan_speedup_time`,
    /// `fan_speedup_overhangs`, `fan_kickstart`), where asked.
    pub fan_timing: Option<FanTiming>,
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
    /// What travel goes at on the first layer where that is set apart
    /// from the first layer's own: Bambu Studio's
    /// `initial_layer_travel_acceleration`, or failing that
    /// `first_layer_travel_acceleration`, which may be a share of
    /// `travel_acceleration` (GCode.cpp
    /// `first_layer_travel_acceleration_override`); 0 for neither.
    pub first_layer_travel_acceleration: f64,
    pub top_surface_acceleration: f64,
    /// Fill accelerations may be a percentage of the default.
    pub sparse_infill_acceleration: FloatOrPercent,
    pub internal_solid_infill_acceleration: FloatOrPercent,
    pub travel_acceleration: f64,
    /// Klipper also takes a deceleration limit, as a share of the acceleration.
    pub accel_to_decel_factor: Option<f64>,
    pub klipper: bool,
    /// How the printer is told an acceleration, by its firmware
    /// (GCodeWriter.cpp `set_acceleration_internal`).
    pub acceleration_form: AccelerationForm,
    /// What a printing and a travel acceleration are held to, 0 for no
    /// limit (GCodeWriter.cpp `apply_print_config`, `m_max_acceleration`
    /// and `m_max_travel_acceleration`; see [`acceleration_limits`]).
    pub max_print_acceleration: f64,
    pub max_travel_acceleration: f64,
    /// The jerk each kind of move is set to, where the process sets a
    /// default jerk (`default_jerk` above 0); none otherwise.
    pub jerks: Option<Jerks>,
    /// `M205 J` told on the first layer: the process's junction deviation
    /// held to the machine's, on Marlin 2 with both above 0 (GCode.cpp
    /// `process_layer`, GCodeWriter.cpp `set_junction_deviation`).
    pub junction_deviation: Option<(f64, f64)>,
    /// RepRapFirmware's or Sprinter's G-code (`gcode_flavor` `reprapfirmware`
    /// or `reprap`), which sets pressure advance with `M572`.
    pub reprap: bool,
    /// The pressure advance a filament is pushed out with when it is rammed
    /// on the prime tower, where the printer is to change it for that
    /// (`enable_change_pressure_when_wiping`, `ramming_pressure_advance_value`).
    pub ram_advance: Option<f64>,
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
    /// Written between two objects printed one after the other
    /// (`printing_by_object_gcode`, GCode.cpp `_do_export`).
    pub printing_by_object_gcode: Option<String>,
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
    /// The filament's own temperature for its tool while it waits, in
    /// place of the delta, when the profile keeps idle nozzles from oozing
    /// (`idle_temperature`; 0, the default, for none).
    pub idle_temperature: Option<f64>,
    pub preheat_time: f64,
    /// Whether the tool's temperature is waited for after a change.
    pub tool_change_temperature_wait: bool,
    /// One nozzle takes every filament in turn
    /// (`single_extruder_multi_material`): a change sets the new
    /// filament's temperature without waiting for it, and only where the
    /// reference's tower or tool change does (WipeTower2.cpp
    /// `toolchange_Unload`, GCode.cpp `set_extruder`).
    pub single_nozzle: bool,
    /// A tool that is not rammed is taken over the prime tower before it
    /// is changed (`tool_change_on_wipe_tower`); otherwise, on a printer
    /// of a nozzle for each filament, it is changed where it is, except
    /// first thing on a layer (GCode.cpp `append_tcr2`,
    /// `should_travel_to_tower`).
    pub change_on_tower: bool,
    /// An Elegoo printer, whose `M6211` changes filament and purges
    /// (`printer_model`).
    pub elegoo: bool,
    /// Whether a prime tower is printed (`enable_prime_tower`).
    pub prime_tower: bool,
    /// The printer gets the reference's Bambu Lab tower (WipeTower.cpp):
    /// a Bambu Lab printer, or `wipe_tower_type` `type1`.
    pub bambu_tower: bool,
    /// A Bambu Lab printer (`printer_model`, or failing that the preset's
    /// name, starting "Bambu Lab"), which the reference writes its file
    /// for in Bambu Studio's form (Print.hpp `is_BBL_printer`, set by the
    /// command line's `is_bbl_vendor_preset`): the tags its firmware reads
    /// (`; FEATURE:`, `; CHANGE_LAYER`), the objects labelled for skipping,
    /// the settings block at the top.
    pub bambu: bool,
    /// How the file labels each copy so the printer can cancel it
    /// (GCode.cpp `m_enable_exclude_object`): on a Bambu Lab printer
    /// always, elsewhere where `exclude_object` is on, in the commands of
    /// the printer's firmware. None where nothing is labelled.
    pub labels: Option<Labelling>,
    /// Whether each copy is named in comments where it starts and stops
    /// (`gcode_label_objects`), whether or not it is labelled for
    /// cancelling (GCode.cpp `process_layer`).
    pub label_comments: bool,
    /// Where each tool's nozzle sits off the first's, in mm
    /// (`extruder_offset`): every position written is taken back by it
    /// (GCode.cpp `point_to_gcode`). Bambu Lab's P1 and X1 have 0x2.
    pub extruder_offset: [f64; 2],
    /// What the reference works out of the whole plate for the templates.
    pub plate: Plate,
    /// The machine's limits as the file tells a Marlin or RepRapFirmware
    /// printer them before its start G-code (`emit_machine_limits_to_gcode`;
    /// GCode.cpp `print_machine_envelope`): `M201`, `M203`, `M204`, `M205`.
    pub machine_envelope: Option<String>,
    /// What turns power-loss recovery on from the second layer and off at
    /// the end (`enable_power_loss_recovery` `enable` or `disable`;
    /// GCodeWriter.cpp `enable_power_loss_recovery`): Bambu Lab's
    /// `M1003`, Marlin 2's `M413`. None leaves it to the printer.
    pub power_loss_recovery: Option<(&'static str, &'static str)>,
    /// A Bambu Lab printer scans the first layer before the second
    /// (`scan_first_layer`, `M976 S1 P1`).
    pub scan_first_layer: bool,
    /// The printer's timelapse G-code and where in a layer it goes
    /// (`time_lapse_gcode`; GCode.cpp `process_layer`).
    pub time_lapse: Option<(String, Timelapse)>,
    /// How far below its temperature the printer's change G-code leaves
    /// this filament before the tower, which that tower heats it back up
    /// from (`filament_cooling_before_tower`).
    pub cooling_before_tower: f64,
    /// How many tools the printer has.
    pub tools: usize,
    /// What is flushed going from filament i to filament j, in mm³
    /// (`flush_volumes_matrix` at `i * n + j`, n filaments), times
    /// `flush_multiplier`; empty where nothing is said of it.
    pub flush_volumes: Vec<f64>,
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
    /// Objects of their own layer heights are printed on one layer: each
    /// line's height is said where it changes, as the reference says it
    /// (GCode.cpp `_extrude`, the Height tag), not only the layer's.
    pub own_heights: bool,
    /// A calibration print's change from layer to layer (`calib.rs`);
    /// none for any other print.
    pub calibration: Option<crate::calib::Calibration>,
    /// Spiral vase mode (`spiral_mode`): how its spiral is laid; none
    /// with it off.
    pub spiral: Option<crate::spiral_vase::Spiral>,

    /// What this filament asks of the printer's exhaust fan
    /// (`activate_air_filtration`), in percent: during the print
    /// (`during_print_exhaust_fan_speed`, where
    /// `activate_air_filtration_during_print`) and once it is done
    /// (`complete_print_exhaust_fan_speed`, where
    /// `activate_air_filtration_on_completion`); none where it asks
    /// nothing ([`exhaust_fan`]).
    pub exhaust_fan: [Option<i64>; 2],

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
            support_interface_fan_speed: None,
            bridge_speed: 25.0,
            gap_infill_speed: 30.0,
            detect_overhang_wall: true,
            unsupported_wall_last: false,
            enable_overhang_speed: true,
            overhang_speeds: [FloatOrPercent::Value(0.0); 4],
            resonance_avoidance: None,
            slowdown_for_curled_perimeters: false,
            overhang_shadow: None,
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
            small_support_perimeter_threshold: 0.0,
            small_support_perimeter_speed: FloatOrPercent::Percent(50.0),
            initial_layer_speed: 30.0,
            initial_layer_infill_speed: 60.0,
            slow_down_layers: 0,
            raft_layers: 0,
            travel_speed: 120.0,
            z_speed: 120.0,
            filament_max_volumetric_speed: 0.0,
            retraction_length: 0.8,
            retraction_speed: 30.0,
            deretraction_speed: 0.0,
            retraction_minimum_travel: 2.0,
            z_hop: 0.4,
            z_offset: 0.0,
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
            wipe_on_loops: false,
            wipe_before_external_loop: false,
            wipe_inward: None,
            wall_loops: 2,
            nozzle_diameter: 0.4,
            seam: Seam::Aligned,
            staggered_inner_seams: false,
            seam_gap: 0.04,
            scarf: Scarf::default(),
            resolution: 0.01,
            arc_fitting: false,
            bed: None,
            bed_outline: Vec::new(),
            bed_exclusions: Vec::new(),
            printable_height: 0.0,
            tower_footprint: None,
            min_layer_time_pause: 0.0,
            pause_park: crate::pause::Park::Away,
            tower_envelope: None,
            final_purge: None,
            filament_diameter: 1.75,
            filament_flow_ratio: 1.0,
            print_flow_ratio: 1.0,
            top_solid_infill_flow_ratio: 1.0,
            bottom_solid_infill_flow_ratio: 1.0,
            internal_bridge_flow: 1.0,
            brim_flow_ratio: 1.0,
            small_area: None,
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
            manual_filament_change: false,
            part_fan_floor: 0,
            activate_chamber_temp_control: false,
            chamber_temperature: 0.0,
            fan_timing: None,
            additional_cooling_fan_speed: 0.0,
            skirt_speed: 50.0,
            support_speed: 80.0,
            support_interface_speed: 80.0,
            wipe_tower_max_purge_speed: 90.0,
            default_acceleration: 0.0,
            outer_wall_acceleration: 0.0,
            inner_wall_acceleration: 0.0,
            initial_layer_acceleration: 0.0,
            first_layer_travel_acceleration: 0.0,
            top_surface_acceleration: 0.0,
            sparse_infill_acceleration: FloatOrPercent::Percent(100.0),
            internal_solid_infill_acceleration: FloatOrPercent::Percent(100.0),
            travel_acceleration: 0.0,
            accel_to_decel_factor: None,
            klipper: false,
            acceleration_form: AccelerationForm::Single,
            max_print_acceleration: 0.0,
            max_travel_acceleration: 0.0,
            jerks: None,
            junction_deviation: None,
            reprap: false,
            // The reference's: on, at 0.
            ram_advance: Some(0.0),
            limits: None,
            pressure_advance: None,
            machine_start_gcode: None,
            machine_end_gcode: None,
            before_layer_change_gcode: None,
            layer_change_gcode: None,
            printing_by_object_gcode: None,
            filament_start_gcode: None,
            filament_end_gcode: None,
            change_filament_gcode: None,
            retract_length_toolchange: 10.0,
            standby_temperature_delta: None,
            idle_temperature: None,
            preheat_time: 30.0,
            tool_change_temperature_wait: true,
            single_nozzle: true,
            change_on_tower: false,
            elegoo: false,
            prime_tower: false,
            bambu_tower: false,
            bambu: false,
            labels: None,
            label_comments: true,
            extruder_offset: [0.0, 0.0],
            plate: Plate::default(),
            machine_envelope: None,
            power_loss_recovery: None,
            scan_first_layer: false,
            time_lapse: None,
            cooling_before_tower: 0.0,
            tools: 1,
            flush_volumes: Vec::new(),
            filaments: 1,
            variables: variables(&Settings::default(), None, None, None, None),
            config: Vec::new(),
            produced_at: None,
            own_heights: false,
            calibration: None,
            spiral: None,
            exhaust_fan: [None, None],
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
        unsupported.extend(crate::bambu_vars::fixed_note(settings));

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
        if settings.flag("exclude_object")?.unwrap_or(false) && labelling(settings)?.is_none() {
            unsupported.push(format!(
                "exclude_object: the firmware (gcode_flavor \"{}\") has no command for it, so objects are not labelled for cancelling",
                settings.text("gcode_flavor").unwrap_or("marlin")
            ));
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
        // The first layer's travel speed, a speed or a share of
        // `travel_speed` (GCodeWriter.cpp `active_travel_speed`), is not
        // followed yet: said where it differs from the speed used.
        let first_travel =
            listed_share("initial_layer_travel_speed", FloatOrPercent::Percent(100.0))?
                .resolve(travel_speed);
        if (first_travel - travel_speed).abs() > 1e-6 {
            unsupported.push(format!(
                "initial_layer_travel_speed: the first layer travels at travel_speed ({travel_speed} mm/s), not {first_travel} mm/s"
            ));
        }
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

        let corners = |key: &str| -> Vec<[f64; 2]> {
            (settings.texts(key).unwrap_or_default().iter())
                .flat_map(|item| item.split(','))
                .filter_map(|corner| {
                    let (x, y) = corner.split_once('x')?;
                    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
                })
                .collect()
        };
        let bed_outline = corners("printable_area");
        let excluded = corners("bed_exclude_area");
        let bed_exclusions: Vec<[f64; 4]> = (excluded.len() >= 3)
            .then(|| {
                (excluded.iter()).fold(
                    [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
                    |[a, b, c, d], [x, y]| [a.min(*x), b.min(*y), c.max(*x), d.max(*y)],
                )
            })
            .into_iter()
            .collect();
        let pause_park = match settings.text("min_layer_time_pause_park") {
            None | Some("away") => crate::pause::Park::Away,
            Some("prime_tower") => crate::pause::Park::PrimeTower,
            Some("custom") => crate::pause::Park::Custom([
                settings
                    .float("min_layer_time_pause_park_x")?
                    .unwrap_or(0.0),
                settings
                    .float("min_layer_time_pause_park_y")?
                    .unwrap_or(0.0),
            ]),
            Some(other) => {
                return Err(ConfigError::BadValue {
                    key: "min_layer_time_pause_park".into(),
                    found: other.to_owned(),
                    expected: "away, prime_tower or custom",
                });
            }
        };

        // The bed temperature lives under a key named after the plate in use
        // (PrintConfig.hpp `get_bed_temp_key`, `get_bed_temp_1st_layer_key`).
        // The reference falls back to the High Temp Plate's keys for a plate
        // without its own ("Default Plate"), to avoid a null pointer
        // (GCode.cpp `_do_export`, `get_bed_temperature`); not followed: such
        // a plate is listed below and gets no bed temperature.
        let bed_key = match settings.text("curr_bed_type") {
            Some("Cool Plate") => Some("cool_plate_temp"),
            Some("Engineering Plate") => Some("eng_plate_temp"),
            Some("High Temp Plate") => Some("hot_plate_temp"),
            Some("Textured PEI Plate") => Some("textured_plate_temp"),
            Some("Textured Cool Plate") => Some("textured_cool_plate_temp"),
            Some("Graphic Effect Plate") => Some("graphic_effect_plate_temp"),
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

        let part_fan_floor = crate::cooling::pwm(
            number("part_cooling_fan_min_pwm", 0.0)?
                .round()
                .clamp(0.0, 100.0),
        );
        let bambu = settings
            .text("printer_model")
            .filter(|model| !model.is_empty())
            .or(settings.text("printer_settings_id"))
            .is_some_and(|name| name.starts_with("Bambu Lab"));
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
            support_interface_fan_speed: first("support_material_interface_fan_speed")?
                .filter(|speed| *speed >= 0.0),
            bridge_speed: number("bridge_speed", defaults.bridge_speed)?,
            gap_infill_speed: number("gap_infill_speed", defaults.gap_infill_speed)?,
            detect_overhang_wall: flag("detect_overhang_wall", defaults.detect_overhang_wall)?,
            unsupported_wall_last: flag("unsupported_wall_last", defaults.unsupported_wall_last)?,
            enable_overhang_speed: flag("enable_overhang_speed", defaults.enable_overhang_speed)?,
            resonance_avoidance: if flag("resonance_avoidance", false)? {
                Some((
                    number("min_resonance_avoidance_speed", 70.0)?,
                    number("max_resonance_avoidance_speed", 120.0)?,
                ))
            } else {
                None
            },
            overhang_speeds: [
                listed_share("overhang_1_4_speed", defaults.overhang_speeds[0])?,
                listed_share("overhang_2_4_speed", defaults.overhang_speeds[1])?,
                listed_share("overhang_3_4_speed", defaults.overhang_speeds[2])?,
                listed_share("overhang_4_4_speed", defaults.overhang_speeds[3])?,
            ],
            slowdown_for_curled_perimeters: flag("slowdown_for_curled_perimeters", false)?,
            overhang_shadow: if flag("overhang_shadow_inner_wall", false)? {
                let percent = |key: &str, default: f64| -> Result<f64, ConfigError> {
                    Ok(match settings.float_or_percent(key)? {
                        Some(FloatOrPercent::Percent(value) | FloatOrPercent::Value(value)) => {
                            value
                        }
                        None => default,
                    })
                };
                Some((
                    percent("overhang_shadow_speed_ratio", 30.0)?,
                    percent("overhang_shadow_distance", 200.0)?,
                ))
            } else {
                None
            },
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
            small_support_perimeter_threshold: number("small_support_perimeter_threshold", 0.0)?,
            small_support_perimeter_speed: listed_share(
                "small_support_perimeter_speed",
                defaults.small_support_perimeter_speed,
            )?,
            initial_layer_speed: number("initial_layer_speed", defaults.initial_layer_speed)?,
            slow_down_layers: number("slow_down_layers", 0.0)?.max(0.0) as usize,
            raft_layers: number("raft_layers", 0.0)?.max(0.0) as usize,
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
            z_offset: number("z_offset", 0.0)? + number("filament_z_offset", 0.0)?,
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
            wipe_on_loops: flag("wipe_on_loops", defaults.wipe_on_loops)?,
            wipe_before_external_loop: flag(
                "wipe_before_external_loop",
                defaults.wipe_before_external_loop,
            )?,
            // PrintConfig.cpp: half the outer wall's width unless set.
            wipe_inward: if flag("wipe_inward", false)? {
                Some(
                    settings
                        .float_or_percent("wipe_inward_distance")?
                        .unwrap_or(FloatOrPercent::Percent(50.0)),
                )
            } else {
                None
            },
            wall_loops: settings
                .integer("wall_loops")?
                .map_or(defaults.wall_loops, |count| count.max(0) as usize),
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
            // GCode.cpp `_do_export` (`SpiralVase`, `set_max_xy_smoothing`
            // over the nozzle's diameter) and PrintConfig.cpp.
            spiral: if flag("spiral_mode", false)? {
                Some(crate::spiral_vase::Spiral {
                    smooth: flag("spiral_mode_smooth", false)?,
                    max_xy_smoothing: settings
                        .float_or_percent("spiral_mode_max_xy_smoothing")?
                        .unwrap_or(FloatOrPercent::Percent(200.0))
                        .resolve(nozzle_diameter),
                    starting_flow_ratio: number("spiral_starting_flow_ratio", 0.0)?,
                    finishing_flow_ratio: number("spiral_finishing_flow_ratio", 0.0)?,
                })
            } else {
                None
            },
            bed,
            bed_outline,
            bed_exclusions,
            printable_height: settings.float("printable_height")?.unwrap_or(0.0),
            tower_footprint: None,
            min_layer_time_pause: settings.float("min_layer_time_pause")?.unwrap_or(0.0),
            pause_park,
            tower_envelope: None,
            final_purge: None,
            filament_diameter: number("filament_diameter", defaults.filament_diameter)?,
            filament_flow_ratio: number("filament_flow_ratio", defaults.filament_flow_ratio)?,
            print_flow_ratio: number("print_flow_ratio", defaults.print_flow_ratio)?,
            top_solid_infill_flow_ratio: number("top_solid_infill_flow_ratio", 1.0)?,
            bottom_solid_infill_flow_ratio: number("bottom_solid_infill_flow_ratio", 1.0)?,
            internal_bridge_flow: number("internal_bridge_flow", 1.0)?,
            brim_flow_ratio: number("brim_flow_ratio", 1.0)?,
            small_area: SmallArea::read(settings)?,
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
            manual_filament_change: flag("manual_filament_change", false)?,
            part_fan_floor,
            activate_chamber_temp_control: flag("activate_chamber_temp_control", false)?,
            chamber_temperature: number("chamber_temperature", 0.0)?,
            additional_cooling_fan_speed: number("additional_cooling_fan_speed", 0.0)?,
            fan_timing: FanTiming::of(
                number("fan_speedup_time", 0.0)?,
                flag("fan_speedup_overhangs", true)?,
                number("fan_kickstart", 0.0)?,
            )
            .map(|timing| timing.with_floor(part_fan_floor)),
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
            first_layer_travel_acceleration: match number("initial_layer_travel_acceleration", 0.0)?
            {
                bambu if bambu > 0.0 => bambu,
                _ => match settings.float_or_percent("first_layer_travel_acceleration")? {
                    Some(FloatOrPercent::Value(value)) => value,
                    Some(FloatOrPercent::Percent(percent)) => {
                        percent / 100.0 * number("travel_acceleration", 0.0)?
                    }
                    None => 0.0,
                },
            },
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
            acceleration_form: AccelerationForm::of(settings.text("gcode_flavor")),
            max_print_acceleration: acceleration_limits(settings)?.0,
            jerks: Jerks::of(settings, &number, bambu)?,
            junction_deviation: {
                let deviation = number("default_junction_deviation", 0.0)?;
                // The machine's, as the writer keeps it for the flavours
                // that use the machine's limits (GCodeWriter.cpp
                // `apply_print_config`).
                let max = settings
                    .floats("machine_max_junction_deviation")?
                    .and_then(|values| values.first().copied().flatten())
                    .unwrap_or(0.0);
                (settings.text("gcode_flavor") == Some("marlin2") && deviation > 0.0 && max > 0.0)
                    .then_some((deviation, max))
            },
            max_travel_acceleration: acceleration_limits(settings)?.1,
            reprap: matches!(
                settings.text("gcode_flavor"),
                Some("reprapfirmware" | "reprap")
            ),
            ram_advance: match settings
                .flag("enable_change_pressure_when_wiping")?
                .unwrap_or(true)
            {
                true => Some(number("ramming_pressure_advance_value", 0.0)?),
                false => None,
            },
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
            printing_by_object_gcode: template("printing_by_object_gcode"),
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
            idle_temperature: match settings.flag("ooze_prevention")? {
                Some(true) => first("idle_temperature")?.filter(|idle| *idle > 0.0),
                _ => None,
            },
            preheat_time: settings
                .float("preheat_time")?
                .unwrap_or(defaults.preheat_time),
            tool_change_temperature_wait: settings
                .flag("tool_change_temprature_wait")?
                .unwrap_or(defaults.tool_change_temperature_wait),
            // The reference's defaults where the settings are silent.
            elegoo: settings
                .text("printer_model")
                .is_some_and(|model| model.to_ascii_lowercase().starts_with("elegoo")),
            single_nozzle: settings
                .flag("single_extruder_multi_material")?
                .unwrap_or(defaults.single_nozzle),
            change_on_tower: settings
                .flag("tool_change_on_wipe_tower")?
                .unwrap_or(defaults.change_on_tower),
            prime_tower: settings.flag("enable_prime_tower")?.unwrap_or(false),
            bambu_tower: settings
                .text("printer_model")
                .filter(|model| !model.is_empty())
                .or(settings.text("printer_settings_id"))
                .is_some_and(|name| name.starts_with("Bambu Lab"))
                || settings.text("wipe_tower_type") == Some("type1"),
            bambu,
            labels: labelling(settings)?,
            label_comments: settings.flag("gcode_label_objects")?.unwrap_or(true),
            extruder_offset: settings
                .texts("extruder_offset")
                // As `ConfigOptionVector::get_at`: past the end, the first.
                .and_then(|offsets| offsets.get(index).or(offsets.first()))
                .and_then(|offset| {
                    let (x, y) = offset.trim().split_once('x')?;
                    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
                })
                .unwrap_or([0.0, 0.0]),
            plate: Plate::default(),
            machine_envelope: machine_envelope(settings)?,
            power_loss_recovery: {
                let bambu = settings
                    .text("printer_model")
                    .filter(|model| !model.is_empty())
                    .or(settings.text("printer_settings_id"))
                    .is_some_and(|name| name.starts_with("Bambu Lab"));
                let marlin2 = settings.text("gcode_flavor") == Some("marlin2");
                let (on, off) = match (bambu, marlin2) {
                    (true, _) => (
                        "; open powerlost recovery\nM1003 S1\n",
                        "; close powerlost recovery\nM1003 S0\n",
                    ),
                    (false, true) => ("M413 S1\n", "M413 S0\n"),
                    (false, false) => ("", ""),
                };
                match settings.text("enable_power_loss_recovery") {
                    Some("enable") if !on.is_empty() => Some((on, off)),
                    // Turned off from the second layer on, and never back on.
                    Some("disable") if !on.is_empty() => Some((off, "")),
                    _ => None,
                }
            },
            scan_first_layer: settings.flag("scan_first_layer")?.unwrap_or(false),
            time_lapse: {
                let bambu = settings
                    .text("printer_model")
                    .filter(|model| !model.is_empty())
                    .or(settings.text("printer_settings_id"))
                    .is_some_and(|name| name.starts_with("Bambu Lab"));
                let i3 = settings.text("printer_structure") == Some("i3");
                let by_layer = settings.text("print_sequence") != Some("by object");
                let nozzles = settings.texts("nozzle_diameter").map_or(1, <[String]>::len);
                gcode_template(settings, "time_lapse_gcode").map(|template| {
                    let place = match (i3 && by_layer, bambu) {
                        (true, _) => Timelapse::Traditional,
                        (false, true) if nozzles >= 2 => Timelapse::TwoNozzles,
                        (false, true) => Timelapse::LayerStart { bambu: true },
                        (false, false) => Timelapse::LayerStart { bambu: false },
                    };
                    (template, place)
                })
            },
            cooling_before_tower: number("filament_cooling_before_tower", 0.0)?,
            flush_volumes: {
                // Some projects write the multiplier as a list of one;
                // what cannot be read flushes nothing, as an empty matrix.
                let multiplier = settings
                    .floats("flush_multiplier")
                    .ok()
                    .flatten()
                    .and_then(|values| values.first().copied().flatten())
                    .unwrap_or(1.0);
                settings
                    .floats("flush_volumes_matrix")
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|volume| volume.unwrap_or(0.0) * multiplier)
                    .collect()
            },
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
                    .filter(|(key, _)| {
                        edge_config::reference_knows(key)
                            || crate::bambu_vars::listed_mass_model(settings, key)
                    })
                    .map(|(key, value)| (key.to_owned(), edge_config::serialized(key, value)))
                    .collect();
                config.sort();
                config
            },
            produced_at: None,
            own_heights: false,
            calibration: None,
            exhaust_fan: exhaust_fan(settings, index)?,
            unsupported,
        })
    }
}

/// GCode.cpp `print_machine_envelope`, `write_input_shaping`: the input
/// shaper the printer is told before the print when asked
/// (`input_shaping_emit`): never for Marlin before 2 or a Bambu Lab
/// printer; for RepRapFirmware, or with "Disable", one line for both axes.
fn input_shaping_override(settings: &Settings, flavor: &str) -> Result<String, ConfigError> {
    let bambu = settings
        .text("printer_model")
        .filter(|model| !model.is_empty())
        .or(settings.text("printer_settings_id"))
        .is_some_and(|name| name.starts_with("Bambu Lab"));
    if settings.flag("input_shaping_emit")? != Some(true) || flavor == "marlin" || bambu {
        return Ok(String::new());
    }
    let firmware = match crate::calib::ShaperFirmware::of(Some(flavor)) {
        Ok(firmware) => firmware,
        // Only Marlin 2, RepRapFirmware and Klipper are told it.
        Err(_) => return Ok(String::new()),
    };
    let number =
        |key: &str| -> Result<f32, ConfigError> { Ok(settings.float(key)?.unwrap_or(0.0) as f32) };
    let shaper = settings.text("input_shaping_type").unwrap_or("Default");
    let comments = settings.flag("gcode_comments")?.unwrap_or(false);
    let refused = |reason: String| ConfigError::BadValue {
        key: "input_shaping_type".to_owned(),
        found: reason,
        expected: "input shaping values the firmware takes",
    };
    let mut out = crate::calib::input_shaping_line(
        firmware,
        'X',
        number("input_shaping_damp_x")?,
        number("input_shaping_freq_x")?,
        shaper,
        comments,
    )
    .map_err(refused)?;
    if flavor != "reprapfirmware" && shaper != "Disable" {
        out += &crate::calib::input_shaping_line(
            firmware,
            'Y',
            number("input_shaping_damp_y")?,
            number("input_shaping_freq_y")?,
            "",
            comments,
        )
        .map_err(refused)?;
    }
    Ok(out)
}

/// GCode.cpp `print_machine_envelope`: the machine's limits, for a Marlin
/// or RepRapFirmware printer that is to be told them, and the input
/// shaper where asked ([`input_shaping_override`]), which Klipper is told
/// too.
fn machine_envelope(settings: &Settings) -> Result<Option<String>, ConfigError> {
    // Only for a printer the settings describe: without one the file has
    // a generic preamble and no limits to tell (as `limits`).
    let Some(flavor) = settings.text("gcode_flavor") else {
        return Ok(None);
    };
    let emit = settings
        .flag("emit_machine_limits_to_gcode")?
        .unwrap_or(true);
    if flavor == "klipper" {
        let shaping = input_shaping_override(settings, flavor)?;
        return Ok((!shaping.is_empty()).then_some(shaping));
    }
    if !emit || !matches!(flavor, "marlin" | "marlin2" | "reprapfirmware") {
        return Ok(None);
    }
    let first = |key: &str| -> Result<f64, ConfigError> {
        Ok(settings
            .floats(key)?
            .and_then(|values| values.first().copied().flatten())
            .unwrap_or(0.0))
    };
    let whole = |value: f64| (value + 0.5) as i64;
    let rrf = flavor == "reprapfirmware";
    let factor = if rrf { 60.0 } else { 1.0 };
    let axes = |stem: &str, by: f64| -> Result<Vec<i64>, ConfigError> {
        ["x", "y", "z", "e"]
            .iter()
            .map(|axis| Ok(whole(first(&format!("{stem}_{axis}"))? * by)))
            .collect()
    };
    let accelerations = axes("machine_max_acceleration", 1.0)?;
    let speeds = axes("machine_max_speed", factor)?;
    let mut out = format!(
        "M201 X{} Y{} Z{} E{}\nM203 X{} Y{} Z{} E{}\n",
        accelerations[0],
        accelerations[1],
        accelerations[2],
        accelerations[3],
        speeds[0],
        speeds[1],
        speeds[2],
        speeds[3]
    );
    let extruding = whole(first("machine_max_acceleration_extruding")?);
    let retracting = whole(first("machine_max_acceleration_retracting")?);
    let travel = whole(first("machine_max_acceleration_travel")?);
    out += &match flavor {
        "reprapfirmware" => {
            format!("M204 P{extruding} T{travel} ; sets acceleration (P, T), mm/sec^2\n")
        }
        "marlin2" => format!(
            "M204 P{extruding} R{retracting} T{travel} ; sets acceleration (P, T) and retract acceleration (R), mm/sec^2\n"
        ),
        // Marlin before 2 travels at the printing acceleration.
        _ => format!("M204 P{extruding} R{retracting} T{extruding}\n"),
    };
    let jerk = |axis: &str| first(&format!("machine_max_jerk_{axis}")).map(|jerk| jerk * factor);
    out += &format!(
        "{} X{:.2} Y{:.2} Z{:.2} E{:.2} ; sets the jerk limits, {}\n",
        if rrf { "M566" } else { "M205" },
        jerk("x")?,
        jerk("y")?,
        jerk("z")?,
        jerk("e")?,
        if rrf { "mm/min" } else { "mm/sec" }
    );
    let deviation = first("machine_max_junction_deviation")?;
    if flavor == "marlin2" && deviation > 0.0 {
        out += &format!("M205 J{deviation:.3}\n");
    }
    out += &input_shaping_override(settings, flavor)?;
    Ok(Some(out))
}

/// What filament `index` asks of the exhaust fan, during the print and
/// once it is done, in percent (GCode.cpp `_do_export`, l.4621-4634 and
/// 4949-4959 at the pin): nothing unless `activate_air_filtration`, then
/// each of `during_print_exhaust_fan_speed` and
/// `complete_print_exhaust_fan_speed` where its own switch
/// (`activate_air_filtration_during_print`, `_on_completion`) is on. Each
/// a per-filament list, read at the filament's place or else at the first
/// (`ConfigOptionVector::get_at`); the reference's defaults where absent:
/// off, on, on, 60 and 80. The printer's `support_air_filtration` is not
/// asked, as the reference does not ask it there.
fn exhaust_fan(settings: &Settings, index: usize) -> Result<[Option<i64>; 2], ConfigError> {
    let at = |key: &str| {
        settings
            .texts(key)
            .and_then(|values| values.get(index).or(values.first()))
            .map(|text| text.trim().to_owned())
    };
    let flag = |key: &'static str, default: bool| -> Result<bool, ConfigError> {
        match at(key).as_deref() {
            None => Ok(default),
            Some("1" | "true") => Ok(true),
            Some("0" | "false") => Ok(false),
            Some(other) => Err(ConfigError::BadValue {
                key: key.into(),
                found: other.to_owned(),
                expected: "1 or 0",
            }),
        }
    };
    let speed = |key: &'static str, default: i64| -> Result<i64, ConfigError> {
        match at(key) {
            None => Ok(default),
            Some(text) => text.parse().map_err(|_| ConfigError::BadValue {
                key: key.into(),
                found: text,
                expected: "a whole number",
            }),
        }
    };
    if !flag("activate_air_filtration", false)? {
        return Ok([None, None]);
    }
    Ok([
        flag("activate_air_filtration_during_print", true)?
            .then(|| speed("during_print_exhaust_fan_speed", 60))
            .transpose()?,
        flag("activate_air_filtration_on_completion", true)?
            .then(|| speed("complete_print_exhaust_fan_speed", 80))
            .transpose()?,
    ])
}

/// GCodeWriter.cpp `set_exhaust_fan`: the exhaust fan (fan 3) at the
/// highest speed, in percent, the print's filaments ask for at `when` (0
/// during the print, 1 once done); nothing where none asks.
pub(crate) fn exhaust_fan_command(
    filaments: impl IntoIterator<Item = [Option<i64>; 2]>,
    when: usize,
) -> Option<String> {
    let speed = filaments
        .into_iter()
        .filter_map(|asked| asked[when])
        .max()?;
    // `(int)(speed / 100.0 * 255)`: towards zero.
    Some(format!(
        "M106 P3 S{}\n",
        (speed as f64 / 100.0 * 255.0) as i64
    ))
}

/// How the firmware is told an acceleration (GCodeWriter.cpp
/// `set_acceleration_internal`), by `gcode_flavor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccelerationForm {
    /// Klipper: `SET_VELOCITY_LIMIT ACCEL=`, with the deceleration share
    /// where one is set; printing and travel alike.
    Klipper,
    /// Marlin 2 and RepRapFirmware: `M204 P` for printing and `M204 T`
    /// for travel, each kept track of apart.
    PrintTravel,
    /// Repetier: `M201 X Y` for printing and `M202 X Y` for travel, each
    /// kept track of apart.
    Repetier,
    /// Every other firmware (Marlin before 2 among them): `M204 S`,
    /// printing and travel alike.
    Single,
}

impl AccelerationForm {
    pub fn of(flavor: Option<&str>) -> Self {
        match flavor {
            Some("klipper") => Self::Klipper,
            Some("marlin2" | "reprapfirmware") => Self::PrintTravel,
            Some("repetier") => Self::Repetier,
            _ => Self::Single,
        }
    }

    /// Whether a travel acceleration is said and kept track of apart from
    /// the printing one (GCodeWriter.cpp
    /// `supports_separate_travel_acceleration`).
    pub fn separate_travel(self) -> bool {
        matches!(self, Self::PrintTravel | Self::Repetier)
    }

    /// The line that sets `acceleration` (mm/s², whole) for printing or,
    /// where the firmware has one apart, for travel. Klipper's
    /// deceleration share is `accel_to_decel`, as
    /// `PrintSettings::accel_to_decel_factor`.
    pub fn command(self, travel: bool, acceleration: f64, accel_to_decel: Option<f64>) -> String {
        let travel = travel && self.separate_travel();
        match (self, accel_to_decel) {
            // The limit is said both ways (GCodeWriter.cpp
            // `append_klipper_accel_to_decel`): Klipper before 0.13 and
            // vendor forks read ACCEL_TO_DECEL, current Klipper only
            // MINIMUM_CRUISE_RATIO, which is one less the same share.
            (Self::Klipper, Some(factor)) if factor > 0.0 => {
                let factor = factor.min(1.0);
                let ratio = format!("{:.6}", 1.0 - factor);
                let ratio = ratio.trim_end_matches('0').trim_end_matches('.');
                format!(
                    "SET_VELOCITY_LIMIT ACCEL={acceleration:.0} ACCEL_TO_DECEL={:.0} MINIMUM_CRUISE_RATIO={ratio}\n",
                    acceleration * factor
                )
            }
            (Self::Klipper, _) => format!("SET_VELOCITY_LIMIT ACCEL={acceleration:.0}\n"),
            (Self::PrintTravel, _) => {
                format!("M204 {}{acceleration:.0}\n", if travel { 'T' } else { 'P' })
            }
            (Self::Repetier, _) => format!(
                "{} X{acceleration:.0} Y{acceleration:.0}\n",
                if travel { "M202" } else { "M201" }
            ),
            (Self::Single, _) => format!("M204 S{acceleration:.0}\n"),
        }
    }
}

/// The jerks of a process with a default jerk (GCode.cpp `_extrude`,
/// `travel_to`, `process_layer`), each 0 where it is not set, and how the
/// printer is told one (GCodeWriter.cpp `set_jerk_xy`,
/// `set_accel_and_jerk`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jerks {
    pub default: f64,
    pub outer_wall: f64,
    pub inner_wall: f64,
    pub infill: f64,
    pub top_surface: f64,
    pub initial_layer: f64,
    pub travel: f64,
    /// `first_layer_travel_jerk`, a share of `travel_jerk` where it is a
    /// percentage.
    pub first_layer_travel: f64,
    pub form: crate::JerkForm,
    /// The machine's x and y jerk as the writer keeps them (`std::lrint`)
    /// for the flavours that use the machine's limits, else 0.
    pub max: [f64; 2],
    /// A Bambu Lab printer's z and e jerk, told beside it.
    pub bambu: Option<[f64; 2]>,
}

impl Jerks {
    fn of(
        settings: &Settings,
        number: &dyn Fn(&str, f64) -> Result<f64, ConfigError>,
        bambu: bool,
    ) -> Result<Option<Self>, ConfigError> {
        let default = number("default_jerk", 0.0)?;
        if default <= 0.0 {
            return Ok(None);
        }
        let flavor = settings.text("gcode_flavor").unwrap_or("marlin");
        let machine = |key: &str| -> Result<f64, ConfigError> {
            Ok(settings
                .floats(key)?
                .and_then(|values| values.first().copied().flatten())
                .unwrap_or(0.0))
        };
        let limits = matches!(flavor, "marlin" | "marlin2" | "klipper" | "reprapfirmware");
        let kept = |key: &str| -> Result<f64, ConfigError> {
            Ok(if limits {
                machine(key)?.round_ties_even()
            } else {
                0.0
            })
        };
        let travel = number("travel_jerk", 0.0)?;
        Ok(Some(Self {
            default,
            outer_wall: number("outer_wall_jerk", 0.0)?,
            inner_wall: number("inner_wall_jerk", 0.0)?,
            infill: number("infill_jerk", 0.0)?,
            top_surface: number("top_surface_jerk", 0.0)?,
            initial_layer: number("initial_layer_jerk", 0.0)?,
            travel,
            first_layer_travel: match settings.float_or_percent("first_layer_travel_jerk")? {
                Some(FloatOrPercent::Value(value)) => value,
                Some(FloatOrPercent::Percent(percent)) => percent / 100.0 * travel,
                None => 0.0,
            },
            form: match flavor {
                "klipper" => crate::JerkForm::Klipper,
                "repetier" => crate::JerkForm::Repetier,
                _ => crate::JerkForm::Marlin,
            },
            max: [kept("machine_max_jerk_x")?, kept("machine_max_jerk_y")?],
            bambu: bambu
                .then(|| -> Result<[f64; 2], ConfigError> {
                    Ok([
                        machine("machine_max_jerk_z")?,
                        machine("machine_max_jerk_e")?,
                    ])
                })
                .transpose()?,
        }))
    }

    /// The jerk a line of `role` is laid with (GCode.cpp `_extrude`,
    /// "adjust X Y jerk").
    pub fn of_line(&self, role: edge_extrusion::Role, first_layer: bool) -> f64 {
        use edge_extrusion::Role::*;
        let set = |jerk: f64| (jerk > 0.0).then_some(jerk);
        let own = if first_layer {
            set(self.initial_layer)
        } else {
            None
        };
        own.or_else(|| match role {
            ExternalPerimeter => set(self.outer_wall),
            _ => None,
        })
        .or_else(|| match role {
            Perimeter => set(self.inner_wall),
            _ => None,
        })
        .or_else(|| match role {
            TopSolidInfill => set(self.top_surface),
            _ => None,
        })
        .or_else(|| match role {
            BridgeInfill | InternalBridge | SparseInfill | SolidInfill | TopSolidInfill
            | BottomSurface | Ironing => set(self.infill),
            _ => None,
        })
        .unwrap_or(self.default)
    }
}

/// What a printing and a travel acceleration are held to, 0 for no limit
/// (GCodeWriter.cpp `apply_print_config`, `set_acceleration_internal`,
/// `set_accel_and_jerk`). On Marlin, Marlin 2, RepRapFirmware and Klipper
/// printing is held to `machine_max_acceleration_extruding`, on Klipper
/// also to `machine_max_acceleration_x` and `_y`, since its one limit
/// covers every move. Travel is held to `machine_max_acceleration_travel`
/// where the firmware has a travel acceleration of its own (Marlin 2,
/// RepRapFirmware), on Klipper to the printing limit (its travel goes
/// through `set_accel_and_jerk`), elsewhere to nothing. Without a printer
/// (no `gcode_flavor`) nothing is held, as the file then tells no limits.
pub(crate) fn acceleration_limits(settings: &Settings) -> Result<(f64, f64), ConfigError> {
    let Some(flavor) = settings.text("gcode_flavor") else {
        return Ok((0.0, 0.0));
    };
    if !matches!(flavor, "marlin" | "marlin2" | "klipper" | "reprapfirmware") {
        return Ok((0.0, 0.0));
    }
    let first = |key: &str| -> Result<f64, ConfigError> {
        Ok(settings
            .floats(key)?
            .and_then(|values| values.first().copied().flatten())
            .unwrap_or(0.0)
            .max(0.0))
    };
    // `std::lrint`, in the default rounding: halves to even.
    let mut print = first("machine_max_acceleration_extruding")?.round_ties_even();
    if flavor == "klipper" {
        for axis in ["machine_max_acceleration_x", "machine_max_acceleration_y"] {
            let limit = first(axis)?.round_ties_even();
            if limit > 0.0 {
                print = if print > 0.0 { print.min(limit) } else { limit };
            }
        }
    }
    let travel = match flavor {
        // `std::round`: halves away from zero, as Rust's `round`.
        "marlin2" | "reprapfirmware" => first("machine_max_acceleration_travel")?.round(),
        "klipper" => print,
        _ => 0.0,
    };
    Ok((print, travel))
}

/// The commands a file labels each copy with so the printer can cancel
/// it (GCode.cpp `process_layer`, `set_object_info`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Labelling {
    /// A Bambu Lab printer's: `M624` with the copy's place among the
    /// plate's numbers as a bit in base 64, `M625`, and comments naming
    /// its number; the numbers listed in the file's header.
    Bambu,
    /// Klipper's `EXCLUDE_OBJECT_DEFINE`, `_START` and `_END`, by name.
    Klipper,
    /// Marlin's `M486 S<n>`, the names given with `M486 A<name>`.
    Marlin,
    /// RepRapFirmware's `M486 S<n> A"<name>"`.
    RepRap,
}

/// Whether and how a file labels its copies: on a Bambu Lab printer
/// always (GCode.cpp `_do_export` turns it on whatever `exclude_object`
/// says), elsewhere as `exclude_object` and the firmware say. A firmware
/// with no command for it gets none, as there.
fn labelling(settings: &Settings) -> Result<Option<Labelling>, ConfigError> {
    let bambu = settings
        .text("printer_model")
        .filter(|model| !model.is_empty())
        .or(settings.text("printer_settings_id"))
        .is_some_and(|name| name.starts_with("Bambu Lab"));
    if bambu {
        return Ok(Some(Labelling::Bambu));
    }
    if !settings.flag("exclude_object")?.unwrap_or(false) {
        return Ok(None);
    }
    Ok(match settings.text("gcode_flavor").unwrap_or("marlin") {
        "klipper" => Some(Labelling::Klipper),
        "marlin" | "marlin2" => Some(Labelling::Marlin),
        "reprapfirmware" => Some(Labelling::RepRap),
        _ => None,
    })
}

/// Where in a layer the printer's timelapse G-code goes (GCode.cpp
/// `process_layer`). A smooth timelapse's tower is not done (a note says
/// so where set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timelapse {
    /// As the layer starts, after the layer change. On a Bambu Lab
    /// printer the filament is pulled back first, where the printer
    /// retracts at layer changes, and the nozzle is put back to its height
    /// after (`generate_timelapse_gcode`).
    LayerStart { bambu: bool },
    /// A bed-slinger printing layer by layer takes the picture in the
    /// middle of the layer (`need_insert_timelapse_gcode_for_traditional`):
    /// before the first tool's work on the prime tower where the layer has
    /// any, else once the first island with fill has its walls, else at the
    /// layer's end; retracted first, the height put back after.
    Traditional,
    /// A Bambu Lab printer of two nozzles, which takes its picture with the
    /// nozzle that prints most; not done, and refused.
    TwoNozzles,
}

/// What the reference works out of the whole plate before it writes the
/// file, which the printer's templates are given (GCode.cpp `_do_export`).
/// Positions are on the bed, before any `extruder_offset`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plate {
    /// The box round the first layer's convex hull, min x, min y, max x,
    /// max y (`first_layer_print_min`, `_max`, `_size`): the skirt's hull
    /// out to its last loop (there whenever `skirt_height` is above 0, with
    /// no loops at `skirt_distance`), the first layer's islands, the brim's
    /// ground and the prime tower's corners (Print.cpp
    /// `finalize_first_layer_convex_hull`). None leaves it to what the
    /// first layer extrudes.
    pub first_layer_box: Option<[f64; 4]>,
    /// The middle of the box round the objects' first layers, each one's
    /// brim included, the prime tower left out
    /// (`first_layer_center_no_wipe_tower`, `PrintObject::get_first_layer_bbox`).
    pub first_layer_centre: Option<[f64; 2]>,
    /// Whether an object, or the first layer's hull, reaches into the
    /// printer's `head_wrap_detect_zone` (`in_head_wrap_detect_zone`).
    pub in_head_wrap_detect_zone: bool,
    /// Each object's box across the bed: min x, min y, max x, max y.
    pub objects: Vec<[f64; 4]>,
    /// Each copy on the plate as the file labels it for cancelling, in
    /// the order the reference numbers them (its objects in the
    /// project's order, each one's copies in theirs). Empty for a print
    /// whose copies are not known apart.
    pub labels: Vec<ObjectLabel>,
    /// Where the prime tower is set to stand (`wipe_tower_x`,
    /// `wipe_tower_y`), which the copies a tool prints on a layer with a
    /// tower are ordered from.
    pub tower_at: Option<Point2>,
    /// Printing object by object (`print_sequence` "by object"): each
    /// copy in the order it is printed, with the entries of the layers
    /// handed to the writer that are its own. Empty for a print laid
    /// layer by layer.
    pub by_object: Vec<ObjectRun>,
}

/// One copy printed whole before the next starts (GCode.cpp `_do_export`,
/// the loop over `print_object_instances_ordering`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectRun {
    /// The copy, by its place among [`Plate::labels`].
    pub copy: usize,
    /// Its entries among the layers handed to the writer, every one of
    /// them its own and nothing of any other copy's.
    pub entries: std::ops::Range<usize>,
}

/// One copy on the plate as the file labels it, so a printer can cancel
/// it (GCode.cpp `set_object_info`, `instance_name`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectLabel {
    /// Its object's name.
    pub name: String,
    /// Its object's number among the plate's (`PrintObject::get_id`), its
    /// copy's among the object's (`PrintInstance::id`) and its number
    /// among all the plate's copies (`unique_id`).
    pub object: usize,
    pub copy: usize,
    pub unique: usize,
    /// The number a Bambu Lab printer knows it by (`identify_id`).
    pub identify: u32,
    /// The middle of its box across the bed, in mm.
    pub centre: [f64; 2],
    /// Its outline across the bed: the convex hull, thinned by
    /// `exclude_object_tolerance` and rounded to 0.1 mm
    /// (`PrintInstance::get_convex_hull_2d`).
    pub hull: Vec<Point2>,
}

/// A template as the reference reads it from a preset: a backslash makes
/// `\n` a line break and `\r` a carriage return, and stands any other
/// character after it for itself (Config.cpp `unescape_string_cstyle`).
/// Vendor presets write their G-code that way; a project's is already plain.
/// One of the printer's G-code settings as the template it is, its
/// escapes taken out; none when it is empty (`machine_pause_gcode`,
/// `template_custom_gcode`).
pub fn gcode_template(settings: &Settings, key: &str) -> Option<String> {
    settings
        .texts(key)
        .and_then(|texts| texts.first())
        .filter(|text| !text.trim().is_empty())
        .map(|text| unescaped(text))
}

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
        let decimal = edge_config::float_number(key);
        match value {
            // A filament's option is a list in the reference, one entry
            // for each filament, even where only its default stands for
            // all of them: a template may ask for any filament's
            // (Prusa's MMU3 start G-code, `required_nozzle_HRC[4]`).
            Setting::One(text) if edge_config::filaments::is_per_filament(key) => {
                variables.set_list(key, vec![setting_value(text, decimal)])
            }
            Setting::One(text) => variables.set(key, setting_value(text, decimal)),
            Setting::Many(texts) => variables.set_list(
                key,
                texts
                    .iter()
                    .map(|text| setting_value(text, decimal))
                    .collect(),
            ),
        }
    }
    filament_retraction_overrides(settings, &mut variables);
    // A list of choices reads in an expression as the number of each
    // choice (PlaceholderParser.cpp, `coEnums` read with `set_i`): Bambu
    // Lab's change G-code asks `z_hop_types[current_extruder] == 0`.
    // (The older `[name]` form writes the choice's name there; here it
    // writes the number too.)
    for (key, choices) in VECTOR_ENUMS {
        let Some(values) = variables.list(key) else {
            continue;
        };
        let numbers: Option<Vec<Value>> = values
            .iter()
            .map(|value| {
                let text = value.to_string();
                let text = text.trim();
                choices
                    .iter()
                    .find(|(name, _)| *name == text)
                    .map(|(_, number)| Value::Int(*number))
                    .or_else(|| text.parse::<i64>().ok().map(Value::Int))
            })
            .collect();
        if let Some(numbers) = numbers {
            variables.set_list(*key, numbers);
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
    // Each filament's exhaust fan speed during the print as the fan takes
    // it, 0 to 255, cut towards zero (GCode.cpp, where the start
    // G-code's variables are set); the reference's 60 % where unset.
    let exhaust = settings
        .floats("during_print_exhaust_fan_speed")
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![Some(60.0)]);
    variables.set_list(
        "during_print_exhaust_fan_speed_num",
        exhaust
            .iter()
            .map(|speed| Value::Int((speed.unwrap_or(0.0) / 100.0 * 255.0) as i64))
            .collect(),
    );
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

/// A setting's value as a template reads it: as text, except a setting
/// the reference declares as a decimal, which is a decimal even where it
/// is written as a whole number ("21" of `filament_max_volumetric_speed`
/// halves to 10.5, as in the reference, not 10).
fn setting_value(text: &str, decimal: bool) -> Value {
    match decimal.then(|| text.trim().parse::<f64>()) {
        Some(Ok(number)) if number.is_finite() => Value::Float(number),
        _ => Value::text(text),
    }
}

/// The settings that are lists of choices (`coEnums` in PrintConfig.cpp),
/// each choice with its number (the `s_keys_map_*` tables there).
const VECTOR_ENUMS: &[(&str, &[(&str, i64)])] = &[
    (
        "overhang_fan_threshold",
        &[
            ("0%", 0),
            ("10%", 1),
            ("25%", 2),
            ("50%", 3),
            ("75%", 4),
            ("95%", 5),
        ],
    ),
    ("nozzle_volume_type", NOZZLE_VOLUME_TYPES),
    ("default_nozzle_volume_type", NOZZLE_VOLUME_TYPES),
    ("extruder_nozzle_volume_type", NOZZLE_VOLUME_TYPES),
    ("extruder_type", &[("Direct Drive", 0), ("Bowden", 1)]),
    ("filament_volume_type", &[("standard", 0), ("high_flow", 1)]),
    (
        "z_hop_types",
        &[
            ("Auto Lift", 0),
            ("Normal Lift", 1),
            ("Slope Lift", 2),
            ("Spiral Lift", 3),
        ],
    ),
    (
        "retract_lift_enforce",
        &[
            ("All Surfaces", 0),
            ("Top Only", 1),
            ("Bottom Only", 2),
            ("Top and Bottom", 3),
        ],
    ),
];

const NOZZLE_VOLUME_TYPES: &[(&str, i64)] = &[
    ("Standard", 0),
    ("High Flow", 1),
    ("Hybrid", 2),
    ("TPU High Flow", 3),
    ("TPU285", 4),
    ("E3D High Flow", 5),
];

/// The printer's retraction settings a filament may set for itself
/// (PrintConfig.cpp `is_extruder_retract_key`).
const EXTRUDER_RETRACT_KEYS: &[&str] = &[
    "retraction_length",
    "z_hop",
    "travel_slope",
    "retract_lift_above",
    "retract_lift_below",
    "retract_lift_enforce",
    "z_hop_types",
    "retraction_speed",
    "deretraction_speed",
    "retraction_minimum_travel",
    "retract_when_changing_layer",
    "wipe",
    "wipe_distance",
    "retract_before_wipe",
    "retract_after_wipe",
    "retract_length_toolchange",
    "retract_restart_extra",
    "retract_restart_extra_toolchange",
    "long_retractions_when_cut",
    "retraction_distances_when_cut",
];

/// Templates read a printer's retraction settings with each filament's own
/// laid over them, one entry per filament, as the reference gives them
/// (PrintApply.cpp `print_config_diffs`: `filament_<key>` applied to
/// `<key>` with `ConfigOptionVector::apply_override`, an entry of `nil`
/// taking the printer's first). The two settings of the long retraction
/// when cutting are taken from the filament only where the printer says
/// so (`enable_long_retraction_when_cut` 2, `EnableFilament`). Then a
/// filament's long retraction counts as on only with a distance above 0
/// (GCode.cpp `effective_long_retractions_when_cut`).
fn filament_retraction_overrides(settings: &Settings, variables: &mut Variables) {
    let by_filament = settings
        .text("enable_long_retraction_when_cut")
        .and_then(|level| level.trim().parse::<i64>().ok())
        == Some(2);
    for key in EXTRUDER_RETRACT_KEYS {
        if !by_filament && key.ends_with("_when_cut") {
            continue;
        }
        let Some(own) = settings.texts(&format!("filament_{key}")) else {
            continue;
        };
        let nil = |text: &String| text.trim() == "nil";
        if own.is_empty() || own.iter().all(nil) {
            continue;
        }
        let Some(printer) = settings.texts(key).and_then(|values| values.first()) else {
            continue;
        };
        let values = own
            .iter()
            .map(|text| Value::text(if nil(text) { printer } else { text }))
            .collect();
        variables.set_list(*key, values);
    }
    let (Some(on), Some(lengths)) = (
        variables.list("long_retractions_when_cut"),
        variables.list("retraction_distances_when_cut"),
    ) else {
        return;
    };
    let at = |values: &[Value], index: usize| values.get(index).or(values.first()).cloned();
    let effective = (0..on.len())
        .map(|index| {
            let on =
                at(on, index).is_some_and(|value| matches!(value.to_string().trim(), "1" | "true"));
            let length = at(lengths, index)
                .and_then(|value| value.to_string().trim().parse::<f64>().ok())
                .unwrap_or(0.0);
            Value::Int(i64::from(on && length > 0.0))
        })
        .collect();
    variables.set_list("long_retractions_when_cut", effective);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first layer's travel speed is not followed; where it differs
    /// from the travel speed, as a speed or a share of it, that is said.
    #[test]
    fn a_first_layer_travel_speed_not_followed_is_said() {
        let notes = |json: &str| {
            PrintSettings::from_settings(&Settings::from_json(json).unwrap())
                .unwrap()
                .unsupported
                .into_iter()
                .filter(|note| note.starts_with("initial_layer_travel_speed"))
                .count()
        };
        assert_eq!(notes(r#"{"travel_speed": ["300"]}"#), 0);
        assert_eq!(
            notes(r#"{"travel_speed": ["300"], "initial_layer_travel_speed": ["100%"]}"#),
            0
        );
        assert_eq!(
            notes(r#"{"travel_speed": ["300"], "initial_layer_travel_speed": ["300"]}"#),
            0
        );
        assert_eq!(
            notes(r#"{"travel_speed": ["300"], "initial_layer_travel_speed": ["50%"]}"#),
            1
        );
        assert_eq!(
            notes(r#"{"travel_speed": ["300"], "initial_layer_travel_speed": ["150"]}"#),
            1
        );
    }

    /// A filament's option that only its default gives is still a list,
    /// so a template may ask for any filament's entry.
    #[test]
    fn a_filament_option_left_at_its_default_is_a_list() {
        let settings =
            Settings::from_json(r#"{"required_nozzle_HRC": "0", "layer_height": "0.2"}"#).unwrap();
        let variables = variables(&settings, None, None, None, None);
        assert_eq!(
            variables.list("required_nozzle_HRC").map(<[Value]>::len),
            Some(1)
        );
        assert!(variables.list("layer_height").is_none());
    }

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
        // A Snapmaker U1's Graphic Effect Plate has keys of its own.
        let graphic = read(
            r#"{"curr_bed_type": "Graphic Effect Plate", "graphic_effect_plate_temp": ["65", "70"],
                "graphic_effect_plate_temp_initial_layer": ["68"], "hot_plate_temp": ["55"]}"#,
        )
        .unwrap();
        assert_eq!(
            (
                graphic.bed_temperature,
                graphic.bed_temperature_initial_layer
            ),
            (Some(65.0), Some(68.0))
        );
        assert!(graphic.unsupported.is_empty());
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
        // A decimal setting written as a whole number halves as a decimal
        // (the P2S start G-code's `filament_max_volumetric_speed[..]/2`); a
        // whole-number one as a whole number.
        assert_eq!(expand("{filament_max_volumetric_speed[0]/8}"), "1.5");
        assert_eq!(expand("[filament_max_volumetric_speed]"), "12");
        let whole = read(r#"{"nozzle_temperature": ["215"]}"#);
        assert_eq!(
            edge_template::expand("{nozzle_temperature[0]/2}", &whole.variables).unwrap(),
            "107"
        );
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
                    "printhost_password": "secret", "name": "project_settings", "from": "project",
                    "different_settings_to_system": ["brim_type;enable_support", "", "z_hop"]}"#,
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
                // An entry holding ";" in quotes (aceRage/EdgeSlicer#1035),
                // so it reads back as one.
                (
                    "different_settings_to_system",
                    "\"brim_type;enable_support\";;z_hop"
                ),
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

    #[test]
    fn a_filaments_retraction_is_what_templates_read() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        // Bambu Lab's: the long retraction when cutting is the filament's
        // where the printer says so (2), and on only with a distance.
        let settings = read(
            r#"{"enable_long_retraction_when_cut": "2", "long_retractions_when_cut": ["0"],
                "retraction_distances_when_cut": ["18"], "filament_long_retractions_when_cut": ["1", "1", "nil"],
                "filament_retraction_distances_when_cut": ["18", "0", "nil"],
                "retraction_length": ["0.8"], "filament_retraction_length": ["nil", "1.2", "nil"],
                "z_hop_types": ["Auto Lift"], "filament_z_hop_types": ["nil", "Spiral Lift", "nil"]}"#,
        );
        let list = |name: &str| -> Vec<String> {
            settings
                .variables
                .list(name)
                .unwrap()
                .iter()
                .map(Value::to_string)
                .collect()
        };
        assert_eq!(list("long_retractions_when_cut"), ["1", "0", "0"]);
        assert_eq!(list("retraction_distances_when_cut"), ["18", "0", "18"]);
        assert_eq!(list("retraction_length"), ["0.8", "1.2", "0.8"]);
        // A list of choices reads as each choice's number.
        assert_eq!(list("z_hop_types"), ["0", "3", "0"]);
        // Anywhere else the printer's own pair stands.
        let settings = read(
            r#"{"enable_long_retraction_when_cut": "0", "long_retractions_when_cut": ["1"],
                "retraction_distances_when_cut": ["18"], "filament_long_retractions_when_cut": ["0"]}"#,
        );
        let on: Vec<String> = (settings
            .variables
            .list("long_retractions_when_cut")
            .unwrap()
            .iter())
        .map(Value::to_string)
        .collect();
        assert_eq!(on, ["1"]);
    }

    #[test]
    fn a_marlin_printer_is_told_its_limits() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let limits = r#""machine_max_acceleration_x": ["20000"], "machine_max_acceleration_y": ["20000"],
            "machine_max_acceleration_z": ["500"], "machine_max_acceleration_e": ["5000"],
            "machine_max_speed_x": ["500"], "machine_max_speed_y": ["500"], "machine_max_speed_z": ["20"],
            "machine_max_speed_e": ["30"], "machine_max_acceleration_extruding": ["20000"],
            "machine_max_acceleration_retracting": ["5000"], "machine_max_acceleration_travel": ["9000"],
            "machine_max_jerk_x": ["9"], "machine_max_jerk_y": ["9"], "machine_max_jerk_z": ["3"],
            "machine_max_jerk_e": ["2.5"]"#;
        let marlin = read(&format!(r#"{{"gcode_flavor": "marlin", {limits}}}"#));
        assert_eq!(
            marlin.machine_envelope.as_deref(),
            Some(
                "M201 X20000 Y20000 Z500 E5000\nM203 X500 Y500 Z20 E30\nM204 P20000 R5000 T20000\n\
                 M205 X9.00 Y9.00 Z3.00 E2.50 ; sets the jerk limits, mm/sec\n"
            )
        );
        let klipper = read(&format!(r#"{{"gcode_flavor": "klipper", {limits}}}"#));
        assert_eq!(klipper.machine_envelope, None);
        let kept = read(&format!(
            r#"{{"gcode_flavor": "marlin", "emit_machine_limits_to_gcode": "0", {limits}}}"#
        ));
        assert_eq!(kept.machine_envelope, None);
    }

    #[test]
    fn a_bambu_lab_printers_own_settings_are_read() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let a1 = read(
            r#"{"printer_model": "Bambu Lab A1", "printer_structure": "i3", "extruder_offset": ["0x2"],
                "enable_power_loss_recovery": "enable", "scan_first_layer": "1",
                "time_lapse_gcode": "M971 S11 C11 O0"}"#,
        );
        assert!(a1.bambu && a1.scan_first_layer);
        assert_eq!(a1.extruder_offset, [0.0, 2.0]);
        assert_eq!(
            a1.power_loss_recovery.unwrap().0,
            "; open powerlost recovery\nM1003 S1\n"
        );
        assert_eq!(a1.time_lapse.unwrap().1, Timelapse::Traditional);
        let x1 = read(
            r#"{"printer_model": "Bambu Lab X1 Carbon", "printer_structure": "corexy", "time_lapse_gcode": "M971"}"#,
        );
        assert_eq!(
            x1.time_lapse.unwrap().1,
            Timelapse::LayerStart { bambu: true }
        );
        let u1 = read(
            r#"{"printer_model": "Snapmaker U1", "gcode_flavor": "marlin2", "enable_power_loss_recovery": "enable"}"#,
        );
        assert!(!u1.bambu);
        assert_eq!(u1.power_loss_recovery, Some(("M413 S1\n", "M413 S0\n")));
    }
}

/// The last filament unloaded over the reference's own prime tower once
/// the print is done (GCode.cpp `WipeTowerIntegration::finalize`).
#[derive(Clone, Debug, PartialEq)]
pub struct FinalPurge {
    /// Where the nozzle goes over the tower.
    pub start: Point2,
    /// Down onto the tower to ram the filament there, where it clears the
    /// print; none to unload where the nozzle is, at the print's height.
    pub drop: Option<PurgeDrop>,
}

/// The final purge's ram on the tower.
#[derive(Clone, Debug, PartialEq)]
pub struct PurgeDrop {
    /// The top of the tower's layer it is laid on, in mm.
    pub z: f64,
    pub lines: Vec<ExtrusionPath>,
    /// How fast the lines are laid, in mm a second.
    pub speed: f64,
    /// Where the nozzle then waits, clear of them.
    pub park: Point2,
}
