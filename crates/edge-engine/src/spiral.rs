//! Spiral vase mode (`spiral_mode`) before the G-code: what it forces on
//! the settings, what it refuses, how the bottom is told from the vase,
//! and which layers are laid as a spiral. The spiral itself is laid by
//! `edge-gcode` (`spiral_vase.rs`).

use edge_config::{ConfigError, Settings, Value};

use crate::Failure;

/// Whether `settings` ask for vase mode.
pub(crate) fn on(settings: &Settings) -> Result<bool, ConfigError> {
    Ok(settings.flag("spiral_mode")? == Some(true))
}

/// PrintConfig.cpp `DynamicPrintConfig::normalize_fdm` (and
/// `normalize_fdm_1`, which PrintApply.cpp applies to the whole config
/// before every slice): with vase mode on, no retraction at a layer
/// change, the printer's and every filament's own, one wall, no extra
/// wall on alternate layers, no top layers and no sparse fill. An
/// object's, a part's or a height range's own settings are left as they
/// are, as the reference leaves them ([`object_notes`]).
pub(crate) fn normalize(settings: &mut Settings) -> Result<(), ConfigError> {
    if !on(settings)? {
        return Ok(());
    }
    for key in [
        "retract_when_changing_layer",
        "filament_retract_when_changing_layer",
    ] {
        let count = settings.texts(key).map_or(0, <[String]>::len);
        if count > 0 {
            settings.insert(key, Value::Many(vec!["0".to_owned(); count]));
        } else if key == "retract_when_changing_layer" {
            settings.insert(key, Value::Many(vec!["0".to_owned()]));
        }
    }
    for (key, value) in [
        ("wall_loops", "1"),
        ("alternate_extra_wall", "0"),
        ("top_shell_layers", "0"),
        ("sparse_infill_density", "0%"),
    ] {
        settings.insert(key, Value::One(value.to_owned()));
    }
    Ok(())
}

/// What the reference refuses in vase mode, with its own message: before
/// slicing, the settings as its command line checks them (PrintConfig.cpp
/// `validate`, `under_cli`: support, which the GUI's dialog turns off),
/// and the plate as `Print::validate` checks it. `copies` is how many
/// objects and copies are on the plate, `materials` how many filaments
/// the object prints in, colours painted on it among them.
pub(crate) fn refuse(settings: &Settings, copies: usize, materials: usize) -> Result<(), Failure> {
    if !on(settings)? {
        return Ok(());
    }
    let invalid = |key: &str, value: &str| -> Failure {
        format!("{key}: Invalid value when spiral vase mode is enabled: {value}").into()
    };
    if settings.flag("enable_support")? == Some(true) {
        return Err(invalid("enable_support", "1"));
    }
    if let Some(layers) = settings.integer("enforce_support_layers")?
        && layers > 0
    {
        return Err(invalid("enforce_support_layers", &layers.to_string()));
    }
    // Print.cpp `validate`, "#4043".
    let by_object = settings.text("print_sequence") == Some("by object");
    if copies > 1 && !by_object {
        return Err(
            "Please select \"By object\" print sequence to print multiple objects in spiral vase mode."
                .into(),
        );
    }
    // Printed one after another, each is a vase of its own; this engine
    // prints every object together, a layer at a time, which would be
    // no spiral at all.
    if copies > 1 {
        return Err(
            "several objects in spiral vase mode are printed one after another \
             (print_sequence \"by object\"), which is not done yet; no G-code was written"
                .into(),
        );
    }
    if !matches!(
        settings.text("stabilizer_supports"),
        None | Some("off" | "0")
    ) {
        return Err("Spiral vase mode can't be used with side stabilizers. Turn the stabilizers off or spiral vase mode off.".into());
    }
    // "Orca #17143": refused for the filaments the object prints in.
    if materials > 1 {
        return Err(
            "The spiral vase mode does not work when an object contains more than one materials."
                .into(),
        );
    }
    Ok(())
}

/// Print.cpp `validate`, "Orca #17240": an object whose own settings, a
/// part's or a height range's, ask for sparse fill, more than one wall
/// or top layers is said to print without a spiral where they apply.
/// `owned` is each object's name and the settings it and its parts set.
pub(crate) fn object_notes<'a>(
    settings: &Settings,
    owned: impl Iterator<Item = (&'a str, Vec<&'a (String, String)>)>,
) -> Result<Vec<String>, ConfigError> {
    if !on(settings)? {
        return Ok(Vec::new());
    }
    let mut notes = Vec::new();
    for (name, own) in owned {
        let other = own.iter().any(|(key, value)| {
            let number = value
                .trim_end_matches('%')
                .trim()
                .parse::<f64>()
                .unwrap_or(0.0);
            match key.as_str() {
                "sparse_infill_density" => number > 0.0,
                "wall_loops" => number > 1.0,
                "top_shell_layers" => number > 0.0,
                _ => false,
            }
        });
        if other {
            notes.push(format!(
                "Spiral vase mode needs no sparse infill, one wall loop and no top shell layers, \
                 but the object {name} uses other values, so it will not print as a continuous spiral."
            ));
        }
    }
    Ok(notes)
}

/// The vase's solid bottom: `bottom_shell_layers` layers, and more while
/// the layers sliced are below `bottom_shell_thickness`; `slices` are the
/// heights the layers are sliced at, from the object's bottom
/// (PrintObjectSlice.cpp `slice_volumes`, `slicing_mode_normal_below_layer`).
/// Above it each layer is sliced to its largest outline alone, its holes
/// closed (`PositiveLargestContour`).
pub(crate) fn sliced_from(settings: &Settings, slices: &[f64]) -> Result<usize, ConfigError> {
    let (layers, thickness) = bottom(settings)?;
    let mut from = layers;
    while from < slices.len() && slices[from] < thickness - 1e-4 {
        from += 1;
    }
    Ok(from)
}

/// `bottom_shell_layers` and `bottom_shell_thickness`.
pub(crate) fn bottom(settings: &Settings) -> Result<(usize, f64), ConfigError> {
    let layers = settings.integer("bottom_shell_layers")?.unwrap_or(3).max(0) as usize;
    let thickness = settings.float("bottom_shell_thickness")?.unwrap_or(0.0);
    Ok((layers, thickness))
}

/// Whether layer `index` of an object, whose top is `print_z` above its
/// bottom, has its walls made as a vase's (LayerRegion.cpp
/// `make_perimeters`, `spiral_mode`): above the bottom layers and
/// `bottom_shell_thickness`.
pub(crate) fn walls_spiral(
    settings: &Settings,
    index: usize,
    print_z: f64,
) -> Result<bool, ConfigError> {
    let (layers, thickness) = bottom(settings)?;
    Ok(index >= layers && print_z >= thickness - 1e-4)
}

/// What decides which layers are laid as a spiral, beside each layer's
/// own lines (GCode.cpp `process_layer`, "Check whether it is possible to
/// apply the spiral vase logic for this layer").
#[derive(Clone, Copy, Debug)]
pub(crate) struct Qualify {
    /// `skirt_height`.
    pub skirt_height: usize,
    /// `Print::has_infinite_skirt`: a draft shield of skirt loops.
    pub infinite_skirt: bool,
    /// `Print::has_brim`.
    pub brim: bool,
}

impl Qualify {
    pub(crate) fn read(settings: &Settings, brim: bool) -> Result<Self, ConfigError> {
        let loops = settings.integer("skirt_loops")?.unwrap_or(0);
        Ok(Self {
            skirt_height: settings.integer("skirt_height")?.unwrap_or(1).max(0) as usize,
            infinite_skirt: settings.text("draft_shield") == Some("enabled") && loops > 0,
            brim,
        })
    }

    /// Whether layer `index` is laid as a spiral: the bottom layers of
    /// its region done, past the skirt and the brim, no support, and
    /// nothing on it but one loop (`perimeters.items_count() > 1u ||
    /// fills.items_count() > 0` is not one).
    pub(crate) fn layer(
        &self,
        index: usize,
        bottom_layers: usize,
        loops: usize,
        lines: usize,
        support: bool,
    ) -> bool {
        !support
            && (index > 0 || !self.brim)
            && index >= self.skirt_height
            && !self.infinite_skirt
            && bottom_layers <= index
            && loops <= 1
            && lines == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vase_mode_forces_one_wall_no_top_no_fill_and_no_retraction_at_layer_changes() {
        let mut settings = Settings::from_json(
            r#"{"spiral_mode": "1", "wall_loops": "3", "top_shell_layers": "4",
                "sparse_infill_density": "15%", "alternate_extra_wall": "1",
                "retract_when_changing_layer": ["1", "1"],
                "filament_retract_when_changing_layer": ["1", "nil"]}"#,
        )
        .unwrap();
        normalize(&mut settings).unwrap();
        assert_eq!(settings.text("wall_loops"), Some("1"));
        assert_eq!(settings.text("top_shell_layers"), Some("0"));
        assert_eq!(settings.text("sparse_infill_density"), Some("0%"));
        assert_eq!(settings.text("alternate_extra_wall"), Some("0"));
        assert_eq!(
            settings.texts("retract_when_changing_layer").unwrap(),
            ["0", "0"]
        );
        assert_eq!(
            settings
                .texts("filament_retract_when_changing_layer")
                .unwrap(),
            ["0", "0"]
        );
        let mut off = Settings::from_json(r#"{"spiral_mode": "0", "wall_loops": "3"}"#).unwrap();
        normalize(&mut off).unwrap();
        assert_eq!(off.text("wall_loops"), Some("3"));
    }

    #[test]
    fn what_the_reference_refuses_is_refused_with_its_words() {
        let said = |json: &str, copies: usize, materials: usize| {
            let settings = Settings::from_json(json).unwrap();
            refuse(&settings, copies, materials)
                .err()
                .map(|e| e.to_string())
        };
        assert_eq!(
            said(r#"{"spiral_mode": "0", "enable_support": "1"}"#, 2, 2),
            None
        );
        assert_eq!(said(r#"{"spiral_mode": "1"}"#, 1, 1), None);
        assert_eq!(
            said(r#"{"spiral_mode": "1", "enable_support": "1"}"#, 1, 1).unwrap(),
            "enable_support: Invalid value when spiral vase mode is enabled: 1"
        );
        assert_eq!(
            said(
                r#"{"spiral_mode": "1", "print_sequence": "by layer"}"#,
                2,
                1
            )
            .unwrap(),
            "Please select \"By object\" print sequence to print multiple objects in spiral vase mode."
        );
        assert!(
            said(
                r#"{"spiral_mode": "1", "print_sequence": "by object"}"#,
                2,
                1
            )
            .unwrap()
            .contains("not done yet")
        );
        assert_eq!(
            said(r#"{"spiral_mode": "1"}"#, 1, 2).unwrap(),
            "The spiral vase mode does not work when an object contains more than one materials."
        );
        assert!(
            said(
                r#"{"spiral_mode": "1", "stabilizer_supports": "auto"}"#,
                1,
                1
            )
            .unwrap()
            .starts_with("Spiral vase mode can't be used with side stabilizers.")
        );
    }

    #[test]
    fn an_object_of_its_own_walls_is_said_to_print_without_a_spiral() {
        let settings = Settings::from_json(r#"{"spiral_mode": "1"}"#).unwrap();
        let walls = ("wall_loops".to_owned(), "2".to_owned());
        let speed = ("outer_wall_speed".to_owned(), "50".to_owned());
        let notes = object_notes(
            &settings,
            [("Vase", vec![&walls]), ("Plain", vec![&speed])].into_iter(),
        )
        .unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("the object Vase uses other values"));
    }

    #[test]
    fn the_bottom_is_its_layers_and_its_thickness() {
        let settings =
            Settings::from_json(r#"{"bottom_shell_layers": "2", "bottom_shell_thickness": "0.9"}"#)
                .unwrap();
        // Sliced at the middle of 0.2 mm layers over a first of 0.25.
        let slices = [0.125, 0.35, 0.55, 0.75, 0.95, 1.15];
        assert_eq!(sliced_from(&settings, &slices).unwrap(), 4);
        assert!(!walls_spiral(&settings, 3, 0.85).unwrap());
        assert!(walls_spiral(&settings, 4, 1.05).unwrap());
    }

    #[test]
    fn a_layer_is_a_spiral_with_one_loop_and_nothing_else() {
        let qualify = Qualify {
            skirt_height: 1,
            infinite_skirt: false,
            brim: false,
        };
        assert!(qualify.layer(3, 3, 1, 0, false));
        assert!(!qualify.layer(2, 3, 1, 0, false));
        assert!(!qualify.layer(3, 3, 2, 0, false));
        assert!(!qualify.layer(3, 3, 1, 1, false));
        assert!(!qualify.layer(3, 3, 1, 0, true));
        assert!(!qualify.layer(0, 0, 1, 0, false));
    }
}
