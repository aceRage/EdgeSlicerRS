//! "Export Bambu 3MF" (Plater.cpp `export_bambu_3mf`, bbs_3mf.cpp with
//! `StoreParams::bambu_compat`): the project written as "Save Project"
//! writes it, but in the form Bambu Studio reads as a project of its own.
//!
//! - `Application` is "BambuStudio-<the line the settings were converted
//!   for>" (`BambuExport::application_tag`), and so is `slice_info.config`'s
//!   `X-BBL-Client-Version`;
//! - the project's settings, each preset kept inside it and each object's,
//!   part's and height range's own settings are converted
//!   (`edge_config::bambu`), with a report of what was left out, renamed
//!   and reshaped;
//! - a plate's bed type Bambu has not is left out, so the plate follows
//!   the project's there;
//! - what is this app's alone is left out: the pricing in the header, the
//!   Layerling design of an object, a part's exact CAD body, airbrush
//!   field and texture displacement, a plate's print history and dual
//!   nozzle confirmation, and the entries that carry them
//!   (`Metadata/cad_bodies/`, `airbrush/`, `texture_displacement/`,
//!   `showcase/`, `edgeslicer_height_patterns.json`).
//!
//! Everything else (geometry, plates, paint, modifiers, cut information,
//! custom G-code) is written as for a project save.

use std::{path::Path, sync::Arc};

use edge_config::{
    Settings,
    bambu::{self, Context, Report, Scope},
};

use crate::{
    Error, Project, Save,
    inside::InsidePreset,
    kept::{self, Entry, Kept},
    write::{Layout, Written},
};

/// What the writer is given for the Bambu form.
pub(crate) struct Form {
    pub(crate) application: String,
    /// `X-BBL-Client-Version`, written as it is.
    pub(crate) client_version: String,
    /// The presets kept inside the project, converted: written in place of
    /// those the project would carry.
    pub(crate) presets: Vec<(String, Vec<u8>)>,
}

/// Entries of a project file that are this app's (and the reference's)
/// own, left out of a file for Bambu Studio (bbs_3mf.cpp: each written
/// only `!m_bambu_compat`).
const OWN_ENTRIES: [&str; 5] = [
    "Metadata/cad_bodies/",
    "Metadata/airbrush/",
    "Metadata/texture_displacement/",
    "Metadata/showcase/",
    "Metadata/edgeslicer_height_patterns.json",
];

/// Metadata of the reference's own on objects, parts, plates and in the
/// header, left out the same way.
const OWN_KEYS: [&str; 7] = [
    "layerling_design",
    "cad_body_file",
    "airbrush_field_file",
    "texture_displacement",
    "edgeslicer_dual_nozzle_confirm",
    "edgeslicer_print_history",
    "edgeslicer_pricing",
];

/// Whether an entry of the file is left out of the Bambu form.
pub(crate) fn left_out(name: &str) -> bool {
    OWN_ENTRIES.iter().any(|own| {
        if own.ends_with('/') {
            name.starts_with(own)
        } else {
            name == *own
        }
    })
}

/// A run of entries with each setting converted: the settings (all but
/// what is not one, `kept::is_setting`) taken out and their Bambu form put
/// where the first of them was, in Bambu's order; a Precise Seam part's
/// dormant ones kept dormant under their prefix.
fn converted_entries(
    entries: &[Entry],
    ctx: &Context,
    report: &mut Report,
    at: &str,
) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
    let mut first: Option<usize> = None;
    let mut found: Vec<(String, String)> = Vec::new();
    let mut seam = false;
    for entry in entries {
        match entry {
            Entry::Meta { key, .. } if OWN_KEYS.contains(&key.as_str()) => {}
            // An object's module name is metadata, not a setting.
            Entry::Meta { key, value } if kept::is_setting(key) && key != "module" => {
                first.get_or_insert(out.len());
                let bare = match key.strip_prefix(kept::PRECISE_SEAM_PREFIX) {
                    Some(bare) => {
                        seam = true;
                        bare
                    }
                    None => key,
                };
                found.push((bare.to_owned(), value.clone()));
            }
            other => out.push(other.clone()),
        }
    }
    if found.is_empty() {
        return out;
    }
    let settings = bambu::object_settings(
        found
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    let converted = bambu::convert(&settings, ctx, Scope::Object, report, at);
    let prefix = if seam { kept::PRECISE_SEAM_PREFIX } else { "" };
    let made: Vec<Entry> = converted
        .iter()
        .map(|(key, value)| Entry::Meta {
            key: format!("{prefix}{key}"),
            value: bambu::serialize(value),
        })
        .collect();
    let at = first.unwrap_or(out.len()).min(out.len());
    out.splice(at..at, made);
    out
}

/// The project as Bambu Studio is to read it: each object's, part's and
/// height range's own settings converted, a plate's bed type Bambu has
/// not left out, this app's own metadata and entries left out.
fn converted_project(project: &Project, ctx: &Context, report: &mut Report) -> Project {
    let mut project = project.clone();
    for object in &mut project.objects {
        let name = object.name.clone().unwrap_or_default();
        if let Some(own) = object.kept.as_deref() {
            let mut own = own.clone();
            own.entries = converted_entries(&own.entries, ctx, report, &format!("object {name}"));
            let ranges = own.layer_ranges();
            if !ranges.is_empty() {
                let ranges: Vec<_> = ranges
                    .into_iter()
                    .map(|mut range| {
                        let settings = bambu::object_settings(
                            range
                                .settings
                                .iter()
                                .map(|(key, value)| (key.as_str(), value.as_str())),
                        );
                        let converted = bambu::convert(
                            &settings,
                            ctx,
                            Scope::Object,
                            report,
                            &format!("height range of {name}"),
                        );
                        range.settings = converted
                            .iter()
                            .map(|(key, value)| (key.clone(), bambu::serialize(value)))
                            .collect();
                        range
                    })
                    .collect();
                own = own.with_height_ranges(&ranges);
            }
            object.kept = Some(Arc::new(own));
        }
        for part in &mut object.parts {
            if let Some(own) = part.kept.as_deref() {
                let mut own = own.clone();
                let part_name = part.name.clone().unwrap_or_default();
                own.entries =
                    converted_entries(&own.entries, ctx, report, &format!("part {part_name}"));
                part.kept = Some(Arc::new(own));
            }
        }
    }
    for plate in &mut project.plates {
        if let Some(own) = plate.kept.as_deref() {
            let mut own = own.clone();
            own.entries.retain_mut(|entry| match entry {
                Entry::Meta { key, .. } if OWN_KEYS.contains(&key.as_str()) => false,
                // bbs_3mf.cpp l.9590: a plate type Bambu Studio does not have is
                // left out, so the plate follows the project's bed type there.
                Entry::Meta { key, value } if key == "bed_type" => {
                    *value = bambu::translate_enum_value("curr_bed_type", value);
                    !value.is_empty()
                }
                _ => true,
            });
            plate.kept = Some(Arc::new(own));
        }
    }
    if let Some(own) = project.kept.as_deref() {
        let mut own: Kept = own.clone();
        own.header
            .retain(|(key, _)| !OWN_KEYS.contains(&key.as_str()));
        own.files.retain(|file| !left_out(&file.name));
        project.kept = Some(Arc::new(own));
    }
    project
}

/// One preset kept inside the project, converted: its settings, its
/// parent and its name, as the reference converts the preset's config.
fn converted_preset(preset: &InsidePreset, ctx: &Context, report: &mut Report) -> Vec<u8> {
    let mut settings = preset.own.clone();
    settings.insert(
        "inherits",
        edge_config::Value::One(preset.inherits.clone().unwrap_or_default()),
    );
    let id = match preset.kind {
        edge_config::Kind::Filament => edge_config::Value::Many(vec![preset.name.clone()]),
        _ => edge_config::Value::One(preset.name.clone()),
    };
    settings.insert(edge_config::user::id_key(preset.kind), id);
    let scope = match preset.kind {
        edge_config::Kind::Process => Scope::Print,
        edge_config::Kind::Filament => Scope::Filament,
        _ => Scope::Printer,
    };
    let converted = bambu::convert(
        &settings,
        ctx,
        scope,
        report,
        &format!("preset {}", preset.name),
    );
    bambu::to_json(
        &converted,
        &preset.name,
        "project",
        &bambu::export_version(),
    )
    .into_bytes()
}

/// Writes `save` to `path` in the Bambu form, whole or not at all, and
/// what the conversion left out, renamed and reshaped.
pub fn write_bambu(
    save: &Save<'_>,
    path: &Path,
    layout: Layout,
) -> Result<(Written, Report), Error> {
    let mut report = Report::default();
    // The project's settings (`convert_project`), which also lays out the
    // variants the rest is converted for.
    // The file's header (`name`, `from`, `version`) is not a setting: the
    // reference converts the settings themselves and writes its own header.
    let settings = match &save.settings {
        Some(text) => {
            let mut settings = Settings::from_json(text)
                .map_err(|reason| crate::xml::malformed(crate::PROJECT_SETTINGS_PART, reason))?;
            for key in ["name", "from", "version"] {
                settings.remove(key);
            }
            Some(settings)
        }
        None => None,
    };
    let mut ctx = settings.as_ref().map(Context::of).unwrap_or_default();
    let text = settings.as_ref().map(|settings| {
        let mut config = bambu::convert_project(settings, &mut ctx, &mut report);
        // Bambu Connect hangs on a file without it; an empty list it takes
        // (bbs_3mf.cpp `_add_project_config_file_to_archive`).
        config
            .entry("print_compatible_printers".into())
            .or_insert(bambu::BambuValue {
                vector: true,
                strings: true,
                values: Vec::new(),
            });
        bambu::to_json(
            &config,
            "project_settings",
            "project",
            &bambu::export_version(),
        )
    });
    if settings.is_none() {
        ctx = Context::of(&Settings::default());
    }
    // The presets kept inside: those given, else the file's own when they go
    // with it (one that could not be read as a preset goes as it was).
    let kept = save.project.kept.as_deref();
    let carried: Vec<InsidePreset> = match save.inside {
        Some(inside) => inside.to_vec(),
        None if save.presets => kept.map(|kept| kept.inside_presets().0).unwrap_or_default(),
        None => Vec::new(),
    };
    let presets: Vec<(String, Vec<u8>)> = crate::inside::inside_entries(&carried)
        .into_iter()
        .zip(sorted_like_entries(&carried))
        .map(|((entry, _), preset)| (entry, converted_preset(preset, &ctx, &mut report)))
        .collect();
    let project = converted_project(save.project, &ctx, &mut report);
    let bambu_save = Save {
        project: &project,
        settings: text,
        presets: false,
        inside: None,
        filaments: save.filaments,
    };
    let form = Form {
        application: bambu::application_tag(),
        client_version: bambu::export_version(),
        presets,
    };
    let written = crate::write::write_form(&bambu_save, path, layout, &form)?;
    Ok((written, report))
}

/// The presets in the order [`crate::inside::inside_entries`] writes them.
fn sorted_like_entries(presets: &[InsidePreset]) -> Vec<&InsidePreset> {
    let mut out = Vec::new();
    for kind in [
        edge_config::Kind::Process,
        edge_config::Kind::Filament,
        edge_config::Kind::Machine,
    ] {
        let mut of_kind: Vec<&InsidePreset> = presets
            .iter()
            .filter(|preset| preset.kind == kind)
            .collect();
        of_kind.sort_by(|a, b| a.name.cmp(&b.name));
        out.extend(of_kind);
    }
    out
}

#[cfg(test)]
mod tests;
