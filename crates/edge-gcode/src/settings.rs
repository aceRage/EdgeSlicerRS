use edge_config::{ConfigError, Settings};

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
            filament_max_volumetric_speed: number(
                "filament_max_volumetric_speed",
                defaults.filament_max_volumetric_speed,
            )?,
            retraction_length: number("retraction_length", defaults.retraction_length)?,
            retraction_speed: number("retraction_speed", defaults.retraction_speed)?,
            deretraction_speed: number("deretraction_speed", defaults.deretraction_speed)?,
            retraction_minimum_travel: number(
                "retraction_minimum_travel",
                defaults.retraction_minimum_travel,
            )?,
            z_hop: number("z_hop", defaults.z_hop)?,
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
            unsupported,
        })
    }
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
