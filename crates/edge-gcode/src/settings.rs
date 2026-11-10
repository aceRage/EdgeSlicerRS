use edge_config::{ConfigError, FloatOrPercent, Settings, Value as Setting};
use edge_template::{Value, Variables};

/// Which wall of an island is printed first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallOrder {
    InnerOuter,
    OuterInner,
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
    pub bridge_speed: f64,
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

    pub filament_diameter: f64,
    pub filament_flow_ratio: f64,
    pub print_flow_ratio: f64,
    /// In g/cm³, when the filament profile gives one.
    pub filament_density: Option<f64>,

    pub nozzle_temperature: f64,
    pub nozzle_temperature_initial_layer: f64,
    /// Absent when the settings name no bed type this engine knows.
    pub bed_temperature: Option<f64>,
    pub bed_temperature_initial_layer: Option<f64>,

    pub wall_order: WallOrder,
    pub infill_first: bool,
    /// Skip retraction on moves between fill lines that stay inside the walls.
    pub reduce_infill_retraction: bool,
    /// Part cooling fan, as percentages: the speed on layers that take
    /// `fan_cooling_layer_time` seconds or longer, and the speed on layers
    /// as short as `slow_down_layer_time`. Between the two it is scaled.
    pub fan_min_speed: f64,
    pub fan_max_speed: f64,
    pub fan_cooling_layer_time: f64,
    pub slow_down_layer_time: f64,
    /// The fan stays off for this many layers at the start.
    pub close_fan_the_first_x_layers: usize,

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

    pub pressure_advance: Option<f64>,

    /// The printer's and filament's own G-code, as templates. Without a
    /// start template a generic preamble is written instead.
    pub machine_start_gcode: Option<String>,
    pub machine_end_gcode: Option<String>,
    pub before_layer_change_gcode: Option<String>,
    pub layer_change_gcode: Option<String>,
    pub filament_start_gcode: Option<String>,
    pub filament_end_gcode: Option<String>,
    /// Every setting, by name, for the templates to refer to.
    pub variables: Variables,

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
            bridge_speed: 25.0,
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
            filament_diameter: 1.75,
            filament_flow_ratio: 1.0,
            print_flow_ratio: 1.0,
            filament_density: None,
            nozzle_temperature: 200.0,
            nozzle_temperature_initial_layer: 200.0,
            bed_temperature: None,
            bed_temperature_initial_layer: None,
            wall_order: WallOrder::InnerOuter,
            infill_first: false,
            reduce_infill_retraction: false,
            fan_min_speed: 20.0,
            fan_max_speed: 100.0,
            fan_cooling_layer_time: 60.0,
            slow_down_layer_time: 5.0,
            close_fan_the_first_x_layers: 1,
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
            pressure_advance: None,
            machine_start_gcode: None,
            machine_end_gcode: None,
            before_layer_change_gcode: None,
            layer_change_gcode: None,
            filament_start_gcode: None,
            filament_end_gcode: None,
            variables: variables(&Settings::default(), None, None),
            unsupported: Vec::new(),
        }
    }
}

impl PrintSettings {
    /// Reads what is present; anything absent keeps its default. Lists are
    /// per extruder or filament and the first entry is used.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let first = |key: &str| -> Result<Option<f64>, ConfigError> {
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
                .and_then(|texts| texts.first())
                .filter(|text| !text.trim().is_empty())
                .cloned()
        };
        let mut unsupported = Vec::new();

        let wall_order = match settings
            .texts("wall_sequence")
            .and_then(|v| v.first())
            .map(String::as_str)
        {
            None | Some("inner wall/outer wall") => WallOrder::InnerOuter,
            Some("outer wall/inner wall") => WallOrder::OuterInner,
            Some(other) => {
                unsupported.push(format!(
                    "wall_sequence \"{other}\": inner wall first is used"
                ));
                WallOrder::InnerOuter
            }
        };
        if let Some(kind) = settings.texts("z_hop_types").and_then(|v| v.first())
            && kind != "Normal Lift"
        {
            unsupported.push(format!(
                "z_hop_types \"{kind}\": a straight vertical lift is used"
            ));
        }

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
            bridge_speed: number("bridge_speed", defaults.bridge_speed)?,
            initial_layer_speed: number("initial_layer_speed", defaults.initial_layer_speed)?,
            initial_layer_infill_speed: number(
                "initial_layer_infill_speed",
                defaults.initial_layer_infill_speed,
            )?,
            travel_speed: number("travel_speed", defaults.travel_speed)?,
            z_speed: {
                let travel = number("travel_speed", defaults.travel_speed)?;
                let wanted = first("travel_speed_z")?
                    .filter(|speed| *speed > 0.0)
                    .unwrap_or(travel);
                match first("machine_max_speed_z")?.filter(|limit| *limit > 0.0) {
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
            filament_diameter: number("filament_diameter", defaults.filament_diameter)?,
            filament_flow_ratio: number("filament_flow_ratio", defaults.filament_flow_ratio)?,
            print_flow_ratio: number("print_flow_ratio", defaults.print_flow_ratio)?,
            filament_density: first("filament_density")?.filter(|density| *density > 0.0),
            nozzle_temperature: number("nozzle_temperature", defaults.nozzle_temperature)?,
            nozzle_temperature_initial_layer: number(
                "nozzle_temperature_initial_layer",
                defaults.nozzle_temperature_initial_layer,
            )?,
            bed_temperature,
            bed_temperature_initial_layer,
            wall_order,
            infill_first: settings
                .flag("is_infill_first")?
                .unwrap_or(defaults.infill_first),
            reduce_infill_retraction: settings
                .flag("reduce_infill_retraction")?
                .unwrap_or(defaults.reduce_infill_retraction),
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
            variables: variables(settings, bed_temperature, bed_temperature_initial_layer),
            unsupported,
        })
    }
}

/// Every setting under its own name, plus the names templates use that are
/// not settings: the tool in use (always the first, for now) and the bed
/// temperature of the plate in use.
fn variables(settings: &Settings, bed: Option<f64>, bed_initial: Option<f64>) -> Variables {
    let mut variables = Variables::default();
    for (key, value) in settings.iter() {
        match value {
            Setting::One(text) => variables.set(key, Value::text(text)),
            Setting::Many(texts) => {
                variables.set_list(key, texts.iter().map(Value::text).collect())
            }
        }
    }
    for name in [
        "initial_extruder",
        "initial_tool",
        "initial_no_support_extruder",
        "current_extruder",
    ] {
        variables.set(name, Value::Int(0));
    }
    let whole = |temperature: f64| Value::Int(temperature.round() as i64);
    if let Some(bed) = bed_initial.or(bed) {
        variables.set("bed_temperature_initial_layer_single", whole(bed));
        variables.set_list("bed_temperature_initial_layer", vec![whole(bed)]);
        variables.set("first_layer_bed_temperature", whole(bed));
    }
    if let Some(bed) = bed.or(bed_initial) {
        variables.set_list("bed_temperature", vec![whole(bed)]);
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
    fn settings_not_honoured_are_listed_not_hidden() {
        let read =
            |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
        let settings = read(
            r#"{"wall_sequence": "inner-outer-inner wall", "z_hop_types": ["Auto Lift"], "curr_bed_type": "Glass"}"#,
        );
        assert_eq!(settings.wall_order, WallOrder::InnerOuter);
        assert_eq!(settings.bed_temperature, None);
        assert_eq!(settings.unsupported.len(), 3);
        assert!(
            PrintSettings::from_settings(
                &Settings::from_json(r#"{"travel_speed": "fast"}"#).unwrap()
            )
            .is_err()
        );
    }
}
