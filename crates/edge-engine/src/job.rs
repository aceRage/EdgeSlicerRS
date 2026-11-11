use edge_config::{Settings, Value};

use crate::{Failure, Loaded};

/// The presets to slice with, each by the name it goes by and with
/// everything it inherits.
#[derive(Clone, Debug, Default)]
pub struct Chosen {
    pub machine: Option<(String, Settings)>,
    pub process: Option<(String, Settings)>,
    /// A preset for filament 1, filament 2 and so on. One alone serves
    /// every filament; a filament past the last named takes the last.
    pub filaments: Vec<(String, Settings)>,
}

/// What the person slicing sets over the presets and the project.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    /// Layer height in mm.
    pub layer_height: Option<f64>,
    /// First layer height in mm.
    pub first_layer_height: Option<f64>,
    /// `slice_closing_radius` in mm; 0 turns it off.
    pub closing_radius: Option<f64>,
    /// Number of walls.
    pub wall_loops: Option<usize>,
    /// Which build plate is on the printer, e.g. "Textured PEI Plate".
    pub bed_type: Option<String>,
    /// Any other setting, by its name and in the text a profile gives it
    /// in: these replace what the presets and the project say. A height
    /// or a radius given above comes before one given here.
    pub settings: Settings,
}

/// A slicing setting's value and where it came from.
pub type Sourced = (f64, &'static str);

/// How the build plate was settled when none was named.
#[derive(Clone, Debug, PartialEq)]
pub enum BedType {
    /// Named by the person, or by the project, or no printer to need one.
    Settled,
    /// The printer's own default plate, by this name.
    PrinterDefault(String),
    /// The reference's default, Cool Plate.
    Assumed,
}

/// Everything a slice needs, settled.
pub struct Job {
    pub loaded: Loaded,
    /// Whether the model came with settings of its own.
    pub is_project: bool,
    /// The project's settings under the presets', completed with the
    /// reference's defaults when there is a printer profile.
    pub settings: Settings,
    pub layer_height: Sourced,
    pub first_layer_height: Sourced,
    pub closing_radius: Sourced,
    pub wall_loops: Option<usize>,
    pub bed_type: BedType,
    /// Things settled here that the person should know of, as sentences.
    pub remarks: Vec<String>,
}

/// A filament preset describes one filament: each of its lists holds
/// that filament's one value. A project's lists hold a value for every
/// filament in turn. This makes the second of the first: filament `n` of
/// `count` takes the `n`th preset, or the last when there are fewer, and
/// what is not a list comes from the first.
fn for_filaments(presets: &[&Settings], count: usize) -> Settings {
    let first = presets[0];
    let mut merged = first.clone();
    let single: Vec<(String, String)> = first
        .iter()
        .filter_map(|(key, value)| match value {
            Value::Many(values) if values.len() == 1 => Some((key.to_owned(), values[0].clone())),
            _ => None,
        })
        .collect();
    for (key, own) in single {
        let values = (0..count)
            .map(|filament| {
                let preset = presets[filament.min(presets.len() - 1)];
                match preset.get(&key) {
                    Some(Value::Many(values)) if values.len() == 1 => values[0].clone(),
                    _ => own.clone(),
                }
            })
            .collect();
        merged.insert(&key, Value::Many(values));
    }
    merged
}

/// Layer height, first layer height and closing radius, each from the first
/// of: the person, process preset, the project's settings, default.
fn slice_settings(
    overrides: &Overrides,
    preset: Option<&Settings>,
    project: Option<&Settings>,
) -> Result<[Sourced; 3], Failure> {
    let pick = |flag: Option<f64>, key: &str| -> Result<Option<Sourced>, Failure> {
        if let Some(value) = flag {
            return Ok(Some((value, "command line")));
        }
        for (settings, source) in [
            (Some(&overrides.settings), "set by hand"),
            (preset, "process preset"),
            (project, "project settings"),
        ] {
            if let Some(value) = settings
                .map(|settings| settings.float(key))
                .transpose()?
                .flatten()
            {
                return Ok(Some((value, source)));
            }
        }
        Ok(None)
    };
    let layer = pick(overrides.layer_height, "layer_height")?.unwrap_or((0.2, "default"));
    let first = pick(overrides.first_layer_height, "initial_layer_print_height")?
        .unwrap_or((layer.0, "layer height"));
    // 0.049 is the reference's default for slice_closing_radius.
    let closing =
        pick(overrides.closing_radius, "slice_closing_radius")?.unwrap_or((0.049, "default"));
    if !(layer.0 > 0.0 && first.0 > 0.0) {
        return Err("layer heights must be positive".into());
    }
    if closing.0.is_nan() || closing.0 < 0.0 {
        return Err("closing radius cannot be negative".into());
    }
    Ok([layer, first, closing])
}

/// Settles what a model is sliced with: the project's own settings, then
/// the presets chosen over them, then what the person set.
pub fn prepare(mut loaded: Loaded, chosen: &Chosen, overrides: &Overrides) -> Result<Job, Failure> {
    let project_settings = loaded.settings.take();
    let is_project = project_settings.is_some();
    let [layer_height, first_layer_height, closing_radius] = slice_settings(
        overrides,
        chosen.process.as_ref().map(|(_, settings)| settings),
        project_settings.as_ref(),
    )?;
    // The highest-numbered filament anything is printed in.
    let filaments = loaded
        .bodies
        .iter()
        .map(|body| body.filament)
        .max()
        .unwrap_or(1);
    let mut remarks = Vec::new();
    for kind in &loaded.unread_paint {
        remarks.push(format!(
            "the project has {kind} painted on it, which is not followed: the settings alone decide"
        ));
    }
    let named = chosen.filaments.len();
    // As many filaments as the model prints in or presets were named for.
    let described = filaments.max(named);
    let filament = chosen.filaments.last().map(|(last, _)| {
        if named == 1 && filaments > 1 {
            remarks.push(format!(
                "the model prints in {filaments} filaments; the filament preset \"{last}\" is used for all of them"
            ));
        } else if named < filaments {
            remarks.push(format!(
                "the model prints in {filaments} filaments and {named} filament presets were chosen; \"{last}\" is used for the rest"
            ));
        }
        let presets: Vec<&Settings> = chosen.filaments.iter().map(|(_, preset)| preset).collect();
        for_filaments(&presets, described)
    });
    // Later sources replace earlier ones: project, then the presets named.
    let mut settings = project_settings.unwrap_or_default();
    for preset in [
        chosen.machine.as_ref().map(|(_, preset)| preset),
        filament.as_ref(),
        chosen.process.as_ref().map(|(_, preset)| preset),
    ]
    .into_iter()
    .flatten()
    {
        settings.overlay(preset);
    }
    // What was set by hand comes over all of them.
    settings.overlay(&overrides.settings);
    // The presets are named in the file, as the reference names them, for
    // whoever reads it back.
    if let Some((name, _)) = &chosen.process {
        settings.insert("print_settings_id", Value::One(name.clone()));
    }
    if let Some((name, _)) = &chosen.machine {
        settings.insert("printer_settings_id", Value::One(name.clone()));
    }
    if let Some((last, _)) = chosen.filaments.last() {
        let names = (0..described)
            .map(|filament| {
                chosen
                    .filaments
                    .get(filament)
                    .map_or(last, |(name, _)| name)
            })
            .cloned()
            .collect();
        settings.insert("filament_settings_id", Value::Many(names));
    }
    let mut bed_type = BedType::Settled;
    if let Some(bed) = &overrides.bed_type {
        settings.insert("curr_bed_type", Value::One(bed.clone()));
    } else if settings.get("curr_bed_type").is_none()
        && settings.get("machine_start_gcode").is_some()
    {
        // The printer's own default plate, which a profile gives by name
        // or by its number in the reference's list (PrintConfig.hpp
        // `BedType`); failing that, the reference's default.
        let plate = match settings.text("default_bed_type").map(str::trim) {
            Some("1") => Some("Cool Plate"),
            Some("2") => Some("Engineering Plate"),
            Some("3") => Some("High Temp Plate"),
            Some("4") => Some("Textured PEI Plate"),
            Some("5") => Some("Textured Cool Plate"),
            Some("7") => Some("Supertack Plate"),
            Some(name) if name.ends_with("Plate") => Some(name),
            _ => None,
        }
        .map(str::to_owned);
        let plate = match plate {
            Some(plate) => {
                bed_type = BedType::PrinterDefault(plate.clone());
                plate
            }
            None => {
                bed_type = BedType::Assumed;
                "Cool Plate".into()
            }
        };
        settings.insert("curr_bed_type", Value::One(plate));
    }
    // What a printer's profile leaves unsaid is as the reference has it
    // by default, so a model sliced with named presets comes out as the
    // same model in a project would.
    if settings.get("machine_start_gcode").is_some() {
        let mut complete = edge_config::reference_defaults().clone();
        complete.overlay(&settings);
        settings = complete;
    }
    Ok(Job {
        loaded,
        is_project,
        settings,
        layer_height,
        first_layer_height,
        closing_radius,
        wall_loops: overrides.wall_loops,
        bed_type,
        remarks,
    })
}
