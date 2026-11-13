use edge_config::variants::{self, Flow};
use edge_config::{Reads, Settings, Value};

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
    /// What is set for one filament alone: `filaments[n]` holds filament
    /// n + 1's own values, each the one entry of a setting that has a
    /// value for every filament (`nozzle_temperature`, `fan_max_speed`).
    /// Only that entry changes; every other filament keeps what it had.
    pub filaments: Vec<Settings>,
    /// What is set for one extruder alone, likewise: `extruders[n]` holds
    /// extruder n + 1's own entry of the printer's lists with a value for
    /// each extruder (`retraction_length`, `nozzle_diameter`).
    pub extruders: Vec<Settings>,
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
    /// Whether it stands where it was put by hand ([`Loaded::placed`]).
    pub placed: bool,
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

/// Each filament's flow (`filament_volume_type`): as set by hand, else as
/// the project has it, else standard for all. A filament past the list's
/// end prints at standard flow, as in the reference (PrintConfig.cpp
/// `get_config_idx`).
fn flows(overrides: &Overrides, project: Option<&Settings>) -> Result<Vec<Flow>, Failure> {
    let given = [Some(&overrides.settings), project]
        .into_iter()
        .flatten()
        .find_map(|settings| settings.texts("filament_volume_type"));
    Ok(match given {
        Some(texts) => Flow::list(texts)?,
        None => Vec::new(),
    })
}

/// Sets entry `index` of a setting with a value for each filament or
/// extruder. A list too short to have that entry is first made long
/// enough with its last value, which is what a filament past its end reads
/// ([`edge_gcode::PrintSettings::for_filament`]), so no other entry reads
/// anything new. Nothing here is noted as read: the list is not asked for,
/// only set.
fn set_entry(settings: &mut Settings, key: &str, index: usize, value: &Value) {
    let own = match value {
        Value::One(text) => text.clone(),
        Value::Many(texts) => texts.first().cloned().unwrap_or_default(),
    };
    let mut list = match settings.remove(key) {
        Some(Value::Many(list)) => list,
        Some(Value::One(text)) => vec![text],
        None => match edge_config::reference_defaults().texts(key) {
            Some(list) => list.to_vec(),
            None => Vec::new(),
        },
    };
    if list.is_empty() {
        list.push(own.clone());
    }
    while list.len() <= index {
        let last = list.last().cloned().unwrap_or_default();
        list.push(last);
    }
    list[index] = own;
    settings.insert(key, Value::Many(list));
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

/// The project's own settings that come over the presets.
const FLUSH_KEYS: [&str; 3] = [
    "flush_volumes_matrix",
    "flush_volumes_vector",
    "flush_multiplier",
];

/// Settles what a model is sliced with: the project's own settings, then
/// the presets chosen over them, then what the person set.
pub fn prepare(mut loaded: Loaded, chosen: &Chosen, overrides: &Overrides) -> Result<Job, Failure> {
    let mut project_settings = loaded.settings.take();
    let flows = flows(overrides, project_settings.as_ref())?;
    // A project saved by the reference's application may hold its
    // filaments' flow variants side by side; one entry per filament here.
    if let Some(project) = &mut project_settings {
        variants::unpack_flow_variants(project, &flows);
        crate::paint::migrate_legacy_depth(project);
    }
    let is_project = project_settings.is_some();
    let placed = loaded.placed;
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
    if loaded.precise_seams {
        remarks.push(
            "the project has Precise Seam parts, which are not followed: their seams are placed as any other's, by seam_position"
                .to_owned(),
        );
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
        let presets: Vec<(&str, &Settings)> = (chosen.filaments.iter())
            .map(|(name, preset)| (name.as_str(), preset))
            .collect();
        let (settings, notes) = variants::for_filaments(&presets, described, &flows);
        remarks.extend(notes);
        settings
    });
    // What the project holds of its own beside the presets, as the
    // reference's `project_config`: the flushing volumes and their
    // multiplier, which its "Flushing volumes" dialog sets, come over the
    // presets (PresetBundle.cpp `full_fff_config`), where a printer's
    // profile gives a multiplier of its own.
    let own: Vec<(&str, Value)> = FLUSH_KEYS
        .iter()
        .filter_map(|key| {
            let held = project_settings.as_ref()?.get(key)?.clone();
            Some((*key, held))
        })
        .collect();
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
    for (key, held) in own {
        settings.insert(key, held);
    }
    // What was set by hand comes over all of them.
    settings.overlay(&overrides.settings);
    // A printer laid out per extruder and nozzle variant: each extruder's
    // own variant's values.
    remarks.extend(variants::for_extruders(&mut settings));
    if flows.contains(&Flow::HighFlow)
        && (settings.texts("process_flow_support")).is_some_and(|support| support.len() > 1)
    {
        remarks.push(
            "a filament prints at high flow, but the process preset's high-flow values are not followed: its standard ones are used"
                .to_owned(),
        );
    }
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
    // What the plate sets for itself comes over everything else, as the
    // reference lays a plate's config over the whole one before it slices
    // (BackgroundSlicingProcess.cpp `apply`: `new_config.apply(*plate->config())`).
    if !loaded.plate_settings.is_empty() {
        if loaded.plate_settings.get("curr_bed_type").is_some() {
            bed_type = BedType::Settled;
        }
        settings.overlay(&loaded.plate_settings);
    }
    // What a printer's profile leaves unsaid is as the reference has it
    // by default, so a model sliced with named presets comes out as the
    // same model in a project would.
    if settings.get("machine_start_gcode").is_some() {
        let mut complete = edge_config::reference_defaults().clone();
        complete.overlay(&settings);
        settings = complete;
    }
    // One filament's or one extruder's own value, over whatever the list
    // came to.
    for own in [&overrides.extruders, &overrides.filaments] {
        for (index, values) in own.iter().enumerate() {
            for (key, value) in values.iter() {
                set_entry(&mut settings, key, index, value);
            }
        }
    }
    Ok(Job {
        loaded,
        is_project,
        placed,
        settings,
        layer_height,
        first_layer_height,
        closing_radius,
        wall_loops: overrides.wall_loops,
        bed_type,
        remarks,
    })
}

/// [`prepare`], with every setting asked for by name from here to the
/// end of the slice noted in `reads`: by `prepare` itself, which looks
/// in the presets, the project and what is set over them for the layer
/// heights, and by every stage that is given the job's settings after.
/// What `reads` holds once the G-code is written is what the print
/// depended on; a setting that is not there changed nothing in it.
pub fn prepare_noted(
    mut loaded: Loaded,
    chosen: &Chosen,
    overrides: &Overrides,
    reads: &Reads,
) -> Result<Job, Failure> {
    let noted = |settings: &Settings| {
        let mut settings = settings.clone();
        settings.noted_by(reads);
        settings
    };
    let named = |preset: &(String, Settings)| (preset.0.clone(), noted(&preset.1));
    let chosen = Chosen {
        machine: chosen.machine.as_ref().map(named),
        process: chosen.process.as_ref().map(named),
        filaments: chosen.filaments.iter().map(named).collect(),
    };
    let overrides = Overrides {
        settings: noted(&overrides.settings),
        ..overrides.clone()
    };
    if let Some(settings) = &mut loaded.settings {
        settings.noted_by(reads);
    }
    let mut job = prepare(loaded, &chosen, &overrides)?;
    job.settings.noted_by(reads);
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_with_precise_seam_parts_says_they_are_not_followed() {
        let loaded = |precise_seams: bool| {
            let cube = edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
                [0.0; 3], [10.0; 3],
            ));
            let mut loaded = crate::load::lone(std::path::Path::new("cube.3mf"), cube).unwrap();
            loaded.precise_seams = precise_seams;
            loaded
        };
        let said = |loaded: Loaded| {
            let job = prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap();
            (job.remarks.iter())
                .filter(|remark| remark.contains("Precise Seam"))
                .count()
        };
        assert_eq!(said(loaded(false)), 0);
        assert_eq!(said(loaded(true)), 1);
    }

    /// Two blocks side by side, the second in filament 2, with the U1's
    /// shipped presets: PLA Matte for filament 1 and PETG-CF for filament
    /// 2, each with a standard and a high-flow value of most settings.
    fn two_filaments_on_the_u1(overrides: &Overrides) -> Result<Job, Failure> {
        let profiles = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer/resources/profiles");
        let vendor = edge_config::Vendor::open(&profiles, "Snapmaker").unwrap();
        let preset = |kind, name: &str| (name.to_owned(), vendor.resolve(kind, name).unwrap());
        use edge_config::Kind;
        let chosen = Chosen {
            machine: Some(preset(Kind::Machine, "Snapmaker U1 (0.4 nozzle)")),
            process: Some(preset(
                Kind::Process,
                "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
            )),
            filaments: vec![
                preset(Kind::Filament, "Snapmaker PLA Matte @U1"),
                preset(Kind::Filament, "Snapmaker PETG-CF @U1 0.4 nozzle"),
            ],
        };
        let block = |x: f32| {
            edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
                [x, 0.0, 0.0],
                [x + 10.0, 10.0, 1.0],
            ))
        };
        let mut loaded = crate::load::lone(std::path::Path::new("two.3mf"), block(0.0)).unwrap();
        loaded.bodies.push(edge_3mf::Body {
            solid: block(20.0),
            negative: None,
            filament: 2,
            part: None,
        });
        loaded.bounds = ([0.0; 3], [30.0, 10.0, 1.0]);
        prepare(loaded, &chosen, overrides)
    }

    #[test]
    fn each_filament_takes_its_own_presets_values_into_the_gcode() {
        let job = two_filaments_on_the_u1(&Overrides::default()).unwrap();
        let texts = |key: &str| job.settings.texts(key).unwrap().to_vec();
        // Not filament 1's high-flow 220 and 0.99.
        assert_eq!(texts("nozzle_temperature"), ["215", "255"]);
        assert_eq!(texts("filament_flow_ratio"), ["1", "0.95"]);
        assert_eq!(texts("filament_max_volumetric_speed"), ["22", "12"]);
        assert_eq!(texts("fan_max_speed"), ["100", "40"]);
        assert!(job.remarks.iter().all(|remark| !remark.contains("high")));
        let sliced = crate::slice(job, &crate::Unwatched).unwrap();
        let text =
            String::from_utf8(sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text).unwrap();
        let heats = |tool: &str| -> Vec<String> {
            (text.lines())
                .filter(|line| line.starts_with("M104 ") || line.starts_with("M109 "))
                .filter(|line| line.split_whitespace().any(|word| word == tool))
                .filter_map(|line| line.split_whitespace().find(|word| word.starts_with('S')))
                .filter(|word| *word != "S0")
                .map(str::to_owned)
                .collect()
        };
        let (t0, t1) = (heats("T0"), heats("T1"));
        assert!(t0.contains(&"S215".to_owned()), "{t0:?}");
        assert!(t1.contains(&"S255".to_owned()), "{t1:?}");
        assert!(!t1.contains(&"S220".to_owned()), "{t1:?}");
    }

    #[test]
    fn a_filament_set_to_high_flow_takes_its_presets_high_flow_values() {
        let overrides = Overrides {
            settings: Settings::from_json(r#"{"filament_volume_type": ["standard", "high_flow"]}"#)
                .unwrap(),
            ..Overrides::default()
        };
        let job = two_filaments_on_the_u1(&overrides).unwrap();
        let texts = |key: &str| job.settings.texts(key).unwrap().to_vec();
        assert_eq!(texts("nozzle_temperature"), ["215", "260"]);
        assert_eq!(texts("filament_flow_ratio"), ["1", "1.01"]);
        // The process's own high-flow speeds are not followed, and it says so.
        assert!(
            job.remarks
                .iter()
                .any(|remark| remark.contains("high-flow values"))
        );
        let wrong = Overrides {
            settings: Settings::from_json(r#"{"filament_volume_type": ["fast"]}"#).unwrap(),
            ..Overrides::default()
        };
        // A flow that is not one is refused, not read as standard.
        assert!(two_filaments_on_the_u1(&wrong).is_err());
    }

    #[test]
    fn a_filament_or_an_extruder_set_alone_changes_its_own_entry_only() {
        let cube =
            edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid([0.0; 3], [10.0; 3]));
        let loaded = crate::load::lone(std::path::Path::new("cube.3mf"), cube).unwrap();
        let settings = |json: &str| Settings::from_json(json).unwrap();
        let chosen = Chosen {
            machine: Some((
                "printer".into(),
                settings(
                    r#"{"retraction_length": ["0.8", "0.9"], "nozzle_diameter": ["0.4", "0.4"]}"#,
                ),
            )),
            process: None,
            filaments: vec![
                (
                    "one".into(),
                    settings(r#"{"nozzle_temperature": ["210"], "fan_max_speed": ["100"]}"#),
                ),
                (
                    "two".into(),
                    settings(r#"{"nozzle_temperature": ["230"], "fan_max_speed": ["90"]}"#),
                ),
            ],
        };
        let overrides = Overrides {
            filaments: vec![
                Settings::default(),
                settings(r#"{"nozzle_temperature": "245"}"#),
            ],
            extruders: vec![settings(r#"{"retraction_length": "1.5"}"#)],
            ..Overrides::default()
        };
        let job = prepare(loaded, &chosen, &overrides).unwrap();
        let texts = |key: &str| job.settings.texts(key).unwrap().to_vec();
        assert_eq!(texts("nozzle_temperature"), ["210", "245"]);
        assert_eq!(texts("fan_max_speed"), ["100", "90"]);
        assert_eq!(texts("retraction_length"), ["1.5", "0.9"]);
        // A list shorter than the entry set is made long enough with what
        // the filaments past its end read already.
        let mut short = settings(r#"{"fan_min_speed": ["20", "30"]}"#);
        set_entry(&mut short, "fan_min_speed", 3, &Value::One("50".into()));
        assert_eq!(
            short.texts("fan_min_speed").unwrap(),
            ["20", "30", "30", "50"]
        );
    }
}
