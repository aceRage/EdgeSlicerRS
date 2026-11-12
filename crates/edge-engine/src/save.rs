//! Saving a model as a project file the reference opens as its own: what
//! it holds as it stands, its settings, and everything its file carried
//! that it does not change (`edge_3mf::write`).

use std::path::Path;

use edge_3mf::{Instance, InstanceRef, Object, Plate, Project};
use edge_config::{Settings, Value};

use super::{Bed, Model};
use crate::{Failure, load::plate_origin};

/// The settings a project is saved with.
#[derive(Clone, Debug)]
pub enum Saving {
    /// The settings its file carried, as they were (what would reach a
    /// printer taken out), with the presets the file carried.
    Own,
    /// These settings, complete: [`project_settings`] for presets chosen
    /// here, [`own_settings`] for the file's own with what was set over
    /// them. `presets`: whether the presets the file carried go with it.
    Settings { settings: Settings, presets: bool },
    /// No settings: a model saved before a printer is chosen.
    None,
}

/// What was set by hand over each preset, by the reference's name for
/// each setting: the reference keeps this as `different_settings_to_system`
/// and, when it opens the file, takes every other value of a preset from
/// the preset the file names (Preset.cpp `load_external_preset`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changed {
    pub process: Vec<String>,
    /// Filament 1's first.
    pub filaments: Vec<Vec<String>>,
    pub printer: Vec<String>,
}

/// What a project was saved as.
#[derive(Clone, Debug)]
pub struct Saved {
    /// The file's size.
    pub bytes: u64,
    /// The entries of the package, in order.
    pub entries: Vec<String>,
}

/// The keys a project's settings name besides the options themselves
/// (PresetBundle.cpp `full_fff_config`).
const NAMES: [&str; 9] = [
    "print_settings_id",
    "printer_settings_id",
    "filament_settings_id",
    "filament_ids",
    "different_settings_to_system",
    "inherits_group",
    "compatible_machine_expression_group",
    "compatible_process_expression_group",
    "print_compatible_printers",
];

/// Settings settled here, as a project's file holds them: only the
/// reference's options and the names above (a preset's own `name`,
/// `inherits`, `type` and the like are the preset's, not the project's),
/// and what was set by hand noted for each preset.
pub fn project_settings(settled: &Settings, changed: &Changed) -> Settings {
    let defaults = edge_config::reference_defaults();
    let mut settings = Settings::default();
    for (key, value) in settled.iter() {
        if !(edge_config::reference_knows(key) || NAMES.contains(&key)) {
            continue;
        }
        // A value taken from the table of the reference's defaults is in
        // the form its G-code lists it in, escapes and all; a project holds
        // the text itself.
        let value = if defaults.get(key) == Some(value) {
            match value {
                Value::One(text) => Value::One(unescaped(text)),
                Value::Many(items) => {
                    Value::Many(items.iter().map(|item| unescaped(item)).collect())
                }
            }
        } else {
            value.clone()
        };
        settings.insert(key, value);
    }
    // The lists the reference writes only when one of their entries says
    // something (PresetBundle.cpp `add_if_some_non_empty`).
    for key in [
        "inherits_group",
        "compatible_machine_expression_group",
        "compatible_process_expression_group",
        "print_compatible_printers",
    ] {
        if settings
            .texts(key)
            .is_some_and(|items| items.iter().all(String::is_empty))
        {
            settings.remove(key);
        }
    }
    let filaments = settings
        .texts("filament_colour")
        .map_or(1, <[String]>::len)
        .max(changed.filaments.len());
    // As the reference reads it back (PresetBundle.cpp
    // `load_config_file_config`): the process, each filament, the printer.
    let mut groups: Vec<String> = Vec::with_capacity(filaments + 2);
    let join = |keys: &[String]| {
        let mut keys = keys.to_vec();
        keys.sort();
        keys.dedup();
        keys.join(";")
    };
    groups.push(join(&changed.process));
    for filament in 0..filaments {
        groups.push(join(
            changed.filaments.get(filament).map_or(&[], Vec::as_slice),
        ));
    }
    groups.push(join(&changed.printer));
    settings.remove("different_settings_to_system");
    if groups.iter().any(|group| !group.is_empty()) {
        settings.insert("different_settings_to_system", Value::Many(groups));
    }
    settings
}

/// A text with its C-style escapes taken out (Config.cpp
/// `unescape_string_cstyle`).
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A file's own settings with what was set over them: each setting
/// changed takes its value as `settled` has it, everything else stays as
/// the file has it, and what was changed is added to what the file says
/// was changed over its presets.
pub fn own_settings(own: &Settings, settled: &Settings, changed: &Changed) -> Settings {
    let mut settings = own.clone();
    let keys = changed
        .process
        .iter()
        .chain(changed.filaments.iter().flatten())
        .chain(&changed.printer);
    for key in keys {
        if let Some(value) = settled.get(key) {
            settings.insert(key.clone(), value.clone());
        }
    }
    let filaments = settings
        .texts("filament_colour")
        .map_or(1, <[String]>::len)
        .max(changed.filaments.len());
    let mut groups: Vec<String> = settings
        .texts("different_settings_to_system")
        .map(<[String]>::to_vec)
        .unwrap_or_default();
    groups.resize(filaments + 2, String::new());
    let mut add = |slot: usize, keys: &[String]| {
        let mut all: Vec<String> = groups[slot]
            .split(';')
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .chain(keys.iter().cloned())
            .collect();
        all.sort();
        all.dedup();
        groups[slot] = all.join(";");
    };
    add(0, &changed.process);
    for (filament, keys) in changed.filaments.iter().enumerate().take(filaments) {
        add(filament + 1, keys);
    }
    add(filaments + 1, &changed.printer);
    if groups.iter().any(|group| !group.is_empty()) {
        settings.insert("different_settings_to_system", Value::Many(groups));
    }
    settings
}

impl Saving {
    /// Whether the presets the file carried go with it.
    fn presets(&self) -> bool {
        match self {
            Saving::Own => true,
            Saving::Settings { presets, .. } => *presets,
            Saving::None => false,
        }
    }
}

impl Model {
    /// The model as a project file holds it, laid out for `settings`: each
    /// copy where the reference will read it to be, its plate where that
    /// printer's bed puts it. A bare model is first stood in the middle
    /// of `bed`, on a plate of its own, as slicing puts it.
    pub fn as_project(&self, bed: Option<&Bed>, settings: Option<&Settings>) -> Project {
        let mut model = self.clone();
        if model.bare || model.plates.is_empty() {
            model.settle(bed);
        }
        let count = model.plates.len();
        let origins: Vec<Option<(f64, f64)>> = (0..count)
            .map(|index| plate_origin(index, count, settings).or(model.plates[index].origin))
            .collect();
        let objects = model
            .objects
            .iter()
            .enumerate()
            .map(|(at, object)| Object {
                id: at as u32 + 1,
                // A model read alone (an STL) goes by its file's name, as
                // the reference names what it loads and as `import` does.
                name: Some(object.name.clone())
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        object.kept.is_none().then(|| {
                            self.path
                                .as_deref()
                                .and_then(Path::file_name)
                                .map(|name| name.to_string_lossy().into_owned())
                        })?
                    }),
                filament: object.filament,
                parts: object.shape.parts.clone(),
                instances: object
                    .copies
                    .iter()
                    .map(|copy| {
                        let plate = model.plate_of(copy.id);
                        let held = plate.and_then(|plate| model.plates[plate].origin);
                        let wanted = plate.and_then(|plate| origins[plate]);
                        // Where the plate is where it was read, the copy is
                        // written where it was read, to the bit.
                        let transform = match (held, wanted) {
                            (Some(held), Some(wanted)) if held == wanted => copy.world,
                            (_, Some((x, y))) => copy
                                .local
                                .then(&edge_mesh::Transform::translation([x, y, 0.0])),
                            _ => copy.world,
                        };
                        Instance {
                            transform,
                            printable: copy.printable,
                        }
                    })
                    .collect(),
                kept: object.kept.clone(),
            })
            .collect();
        let plates = model
            .plates
            .iter()
            .map(|plate| {
                // Its pictures go with it while it is as it was read.
                let unchanged = plate.read.as_ref() == Some(&model.stamp(Some(plate.id)));
                let kept = plate.kept.as_ref().map(|kept| {
                    if unchanged || !kept.has_pictures() {
                        kept.clone()
                    } else {
                        std::sync::Arc::new(kept.without_pictures())
                    }
                });
                Plate {
                    id: plate.id,
                    name: plate.name.clone(),
                    first_layer_sequence: plate.first_layer_sequence.clone(),
                    instances: plate
                        .order
                        .iter()
                        .filter_map(|copy| model.find(*copy))
                        .map(|(object, instance)| InstanceRef { object, instance })
                        .collect(),
                    kept,
                }
            })
            .collect();
        Project {
            objects,
            plates,
            settings: settings.cloned(),
            unread_paint: model.unread_paint.clone(),
            custom_gcode: model.custom_gcode_by_plate(),
            kept: model.kept.clone(),
        }
    }

    /// Says that it is now the project saved at `path`, which is the file
    /// it goes by from then on.
    pub fn saved_as(&mut self, path: &Path) {
        self.path = Some(path.to_owned());
    }

    /// What its file held that saving it as it stands would not keep, as
    /// sentences to ask the person about first; empty when all is kept.
    pub fn save_losses(&self, saving: &Saving) -> Vec<String> {
        let project = self.as_project(None, None);
        edge_3mf::losses(&project, saving.presets())
    }

    /// Writes it to `path` as a project file, whole or not at all. `bed`
    /// is the printer's, for a bare model that has no place yet.
    pub fn save(&self, path: &Path, saving: &Saving, bed: Option<&Bed>) -> Result<Saved, Failure> {
        let (text, settings) = match saving {
            // Its own settings laid out for filaments changed here.
            Saving::Own if self.filaments_edited => {
                let settings = self
                    .settings
                    .as_deref()
                    .ok_or("the project has no settings of its own to save")?;
                (
                    Some(edge_3mf::settings_text(settings)),
                    Some(settings.clone()),
                )
            }
            Saving::Own => {
                let text = self
                    .kept
                    .as_ref()
                    .and_then(|kept| kept.settings_text())
                    .ok_or("the project has no settings of its own to save")?;
                (
                    Some(edge_3mf::own_settings_text(text)?),
                    self.settings.as_deref().cloned(),
                )
            }
            Saving::Settings { settings, .. } => (
                Some(edge_3mf::settings_text(settings)),
                Some(settings.clone()),
            ),
            Saving::None => (None, None),
        };
        // A bare model is stood in the middle of the printer's bed.
        let own_bed = settings.as_ref().and_then(|settings| {
            edge_gcode::PrintSettings::from_settings(settings)
                .ok()?
                .bed
                .map(Bed::of)
        });
        let project = self.as_project(bed.or(own_bed.as_ref()), settings.as_ref());
        let filaments = settings
            .as_ref()
            .and_then(|settings| settings.texts("filament_colour").map(<[String]>::len))
            .unwrap_or(1);
        let save = edge_3mf::Save {
            project: &project,
            settings: text,
            presets: saving.presets(),
            filaments,
        };
        let written =
            edge_3mf::write(&save, path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Saved {
            bytes: written.bytes,
            entries: written.entries,
        })
    }
}
