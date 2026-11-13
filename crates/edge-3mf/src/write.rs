//! Writes a project in the reference's own layout, as its "Save Project"
//! does (bbs_3mf.cpp `_BBS_3MF_Exporter::_save_model_to_file` with
//! `SplitModel | ShareMesh`): `[Content_Types].xml`, `_rels/.rels`, the
//! root model `3D/3dmodel.model` naming each object's file under
//! `3D/Objects/` (`3D/_rels/3dmodel.model.rels`), `Metadata/model_settings.config`
//! (objects, parts, their names, filaments and settings, the plates and
//! the copies on each, the assembly view), `Metadata/project_settings.config`
//! (the settings, flat, as JSON), `Metadata/slice_info.config`, and what
//! the file it was read from carried ([`crate::Kept`]).
//!
//! What a project holds is written as it stands: the meshes (one written
//! once and named by every part that shares it, painted alike), each
//! object's and part's name, filament and settings, the copies and where
//! each stands, the plates with their names and copies. What it was read
//! with and the edits leave alone is written as it was read: what each
//! triangle carries (every kind of paint), the settings of each object
//! and part, each plate's own settings, layer ranges and heights, brim
//! ears, cut connectors, custom G-code at layers, a plate's pictures while
//! they still show it, and every other entry of the package.
//!
//! The same project gives the same bytes: entries come in a fixed order
//! and carry one time, `SOURCE_DATE_EPOCH` when it is set, else the zip
//! format's first day (1980-01-01). A file is written to `<path>.tmp` and
//! moved into place when it is whole.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    fs,
    io::{BufWriter, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use edge_mesh::{Mesh, Transform};
use rayon::prelude::*;
use zip::{CompressionMethod, DateTime, ZipWriter, write::SimpleFileOptions};

use crate::{
    Error, Extras, Object, Part, PartKind, Plate, Project,
    kept::{self, Carry, Entry, Kept, Left, ObjectKept, Picture},
    sliced::{self, SlicedFile, SlicedPlate},
};

/// The version of the reference this layout is that of: the pinned
/// submodule's `Snapmaker_VERSION` (src/common_func/common_func.hpp),
/// bumped with the pin. A file says it was written by "EdgeSlicer-" and
/// this, because the reference reads a file as a whole project (its
/// plates, settings and presets) only when its `Application` names
/// EdgeSlicer, Snapmaker Orca or Bambu Studio (bbs_3mf.cpp
/// `_handle_end_metadata`), and this is the reference's own layout.
pub const REFERENCE_VERSION: &str = "2.4.9.0";

const MODEL: &str = "3D/3dmodel.model";
const MODEL_RELS: &str = "3D/_rels/3dmodel.model.rels";
const MODEL_SETTINGS: &str = "Metadata/model_settings.config";
const PROJECT_SETTINGS: &str = "Metadata/project_settings.config";

const CORE_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const BAMBU_NS: &str = "http://schemas.bambulab.com/package/2021";
const PRODUCTION_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const MODEL_TYPE: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

// bbs_3mf.cpp l.298–303.
const OBJECT_UUID: &str = "-61cb-4c03-9d28-80fed5dfa1dc";
const OBJECT_UUID_SHARED: &str = "-71cb-4c03-9d28-80fed5dfa1dc";
const SUB_OBJECT_UUID: &str = "-81cb-4c03-9d28-80fed5dfa1dc";
const COMPONENT_UUID: &str = "-b206-40ff-9872-83e8017abed1";
const BUILD_UUID: &str = "2c7c17d8-22b5-4d84-8835-1976022ea369";
const BUILD_ITEM_UUID: &str = "-b1ec-4553-aec9-835e5b724bb4";

/// What to write.
pub struct Save<'a> {
    pub project: &'a Project,
    /// `Metadata/project_settings.config` as it is to be written; none
    /// for a project without settings.
    pub settings: Option<String>,
    /// Whether the presets the file carried go with it: only when the
    /// settings written are the file's own.
    pub presets: bool,
    /// How many filaments the settings have, for each plate's
    /// `filament_maps`.
    pub filaments: usize,
}

/// What was written.
#[derive(Clone, Debug, Default)]
pub struct Written {
    /// The file's size.
    pub bytes: u64,
    /// The entries of the package, in order.
    pub entries: Vec<String>,
}

/// The settings a project's file names that would reach a printer: the
/// reference takes them out of a project it saves
/// (PresetBundle.cpp `full_config_secure`) and does not read them from one
/// (Preset.cpp `load_external_preset`).
pub const SECRETS: [&str; 7] = [
    "print_host",
    "print_host_webui",
    "printhost_apikey",
    "printhost_cafile",
    "printhost_user",
    "printhost_password",
    "printhost_port",
];

/// `Metadata/project_settings.config` for settings, as the reference
/// writes it (Config.cpp `ConfigBase::save_to_json`): one JSON object,
/// keys in order, four spaces to a level, with `name`, `from` and
/// `version` among them (given here when the settings have none), and
/// without what would reach a printer.
pub fn settings_text(settings: &edge_config::Settings) -> String {
    let mut object = serde_json::Map::new();
    for (key, value) in settings.iter() {
        if SECRETS.contains(&key) {
            continue;
        }
        let value = match value {
            edge_config::Value::One(text) => serde_json::Value::from(text.as_str()),
            edge_config::Value::Many(items) => serde_json::Value::from(items.as_slice()),
        };
        object.insert(key.to_owned(), value);
    }
    for (key, value) in [
        ("name", "project_settings"),
        ("from", "project"),
        ("version", REFERENCE_VERSION),
    ] {
        object.entry(key).or_insert_with(|| value.into());
    }
    // In order of key, two spaces to a level; each line's indent then
    // doubled (a value's own line breaks are written as `\n`).
    let pretty = serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .expect("strings always serialise");
    let mut text = String::with_capacity(pretty.len() * 5 / 4);
    for line in pretty.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        text.extend(std::iter::repeat_n(' ', indent * 2));
        text.push_str(&line[indent..]);
        text.push('\n');
    }
    text
}

/// The text a file's own settings are written back with: as they were,
/// unless they name what would reach a printer, which is taken out.
pub fn own_settings_text(text: &str) -> Result<String, Error> {
    let settings = edge_config::Settings::from_json(text)
        .map_err(|reason| crate::xml::malformed(PROJECT_SETTINGS, reason))?;
    if SECRETS.iter().any(|key| settings.get(key).is_some()) {
        return Ok(settings_text(&settings));
    }
    Ok(text.to_owned())
}

/// What a project read from a file would not take with it if written as
/// it stands, as sentences: the person is asked before it is lost. Empty
/// when everything is kept.
pub fn losses(project: &Project, presets: bool) -> Vec<String> {
    let mut losses = Vec::new();
    let Some(kept) = &project.kept else {
        return losses;
    };
    if !by_place_holds(project, kept) {
        let names: Vec<&str> = kept
            .files
            .iter()
            .filter(|file| file.carry == Carry::ByObjectPlace)
            .map(|file| file.name.as_str())
            .collect();
        if !names.is_empty() {
            losses.push(format!(
                "What the file keeps by each object's place in the list ({}): an object of the file was deleted, so it would be another object's.",
                names.join(", ")
            ));
        }
    }
    for object in &project.objects {
        let Some(own) = &object.kept else { continue };
        if own.copies != object.instances.len() && own.assembly.iter().any(Option::is_some) {
            losses.push(format!(
                "Where the copies of \"{}\" stand in the assembly view: a copy of it was deleted.",
                object.name.as_deref().unwrap_or("")
            ));
        }
    }
    for object in &project.objects {
        if let Some(Err(())) = cut_of(object) {
            losses.push(format!(
                "The cut information of \"{}\" (its connectors): a part a connector is was deleted or split, so it would be another part's.",
                object.name.as_deref().unwrap_or("")
            ));
        }
    }
    if !presets && kept.has_presets() {
        let names: Vec<&str> = kept
            .files
            .iter()
            .filter(|file| file.carry == Carry::Preset)
            .map(|file| file.name.as_str())
            .collect();
        losses.push(format!(
            "The presets the file carries ({}): the project is saved with the presets chosen here.",
            names.join(", ")
        ));
    }
    for (name, why) in &kept.left {
        if *why == Left::Unsafe {
            losses.push(format!(
                "The entry \"{name}\", whose name reaches outside the file."
            ));
        }
    }
    losses
}

/// Whether every object of the file is still at its place in the list,
/// so that what goes by place still names the same objects.
fn by_place_holds(project: &Project, kept: &Kept) -> bool {
    (0..kept.objects).all(|at| {
        project
            .objects
            .get(at)
            .and_then(|object| object.kept.as_ref())
            .is_some_and(|own| own.index == at + 1)
    })
}

/// How a project's package is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// "Save Project": a model file of its own for each object, a mesh
    /// two parts share written once (`SplitModel | ShareMesh`).
    Project,
    /// "Export Generic 3MF": every mesh in the one model file, each
    /// part's its own, without the production extension (Plater.cpp
    /// `export_core_3mf`: `export_3mf` with `Silence` alone).
    Core,
}

/// How the package is written.
#[derive(Clone, Copy)]
enum Mode<'a> {
    Layout(Layout),
    /// A sliced file: the plates' G-code and what goes with it, without
    /// the model (`SplitModel | WithGcode | SkipModel`).
    Sliced(&'a SlicedFile<'a>),
}

/// Writes `save` to `path`: to `<path>.tmp`, then moved into place, so a
/// file is there whole or not at all, and one that was there is left as
/// it was when writing fails.
pub fn write(save: &Save<'_>, path: &Path) -> Result<Written, Error> {
    write_as(save, path, Layout::Project)
}

/// [`write`], laid out as `layout` says.
pub fn write_as(save: &Save<'_>, path: &Path, layout: Layout) -> Result<Written, Error> {
    whole(path, |out| package(save, out, Mode::Layout(layout)))
}

/// Writes a sliced file (`.gcode.3mf`) of `save` and `sliced` to `path`,
/// whole or not at all, as [`write`] does.
pub fn write_sliced(
    save: &Save<'_>,
    sliced: &SlicedFile<'_>,
    path: &Path,
) -> Result<Written, Error> {
    whole(path, |out| package(save, out, Mode::Sliced(sliced)))
}

/// [`write_sliced`] to `out`.
pub fn write_sliced_to<W: Write + Seek>(
    save: &Save<'_>,
    sliced: &SlicedFile<'_>,
    out: W,
) -> Result<Written, Error> {
    package(save, out, Mode::Sliced(sliced))
}

fn whole(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<fs::File>) -> Result<Written, Error>,
) -> Result<Written, Error> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = (|| -> Result<Written, Error> {
        let file = fs::File::create(&temporary)?;
        let mut out = BufWriter::new(file);
        let mut written = write(&mut out)?;
        let file = out.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        written.bytes = file.metadata()?.len();
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// The time every entry carries.
fn stamp() -> DateTime {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok());
    let Some(seconds) = seconds else {
        return DateTime::default();
    };
    // Days from 1970-01-01 to a civil date (Howard Hinnant's
    // `civil_from_days`), in UTC.
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let clamp = |value: i64, low: i64, high: i64| value.clamp(low, high);
    DateTime::from_date_and_time(
        clamp(year, 1980, 2107) as u16,
        month as u8,
        day as u8,
        (rest / 3600) as u8,
        (rest % 3600 / 60) as u8,
        (rest % 60) as u8,
    )
    .unwrap_or_default()
}

/// Escapes text for an attribute's value or an element's text.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out
}

/// Twelve numbers, the rows of the 4×3 matrix, translation last: what
/// [`crate::model`] reads. Each the shortest that reads back to the same
/// number.
fn transform_text(transform: &Transform) -> String {
    let numbers: Vec<String> = transform
        .rows
        .iter()
        .flatten()
        .map(|value| format!("{value}"))
        .collect();
    numbers.join(" ")
}

/// The part's matrix as the reference writes it in a part's `matrix`:
/// sixteen numbers, row by row, of the matrix that takes a column vector.
fn matrix_text(transform: &Transform) -> String {
    let rows = &transform.rows;
    let mut numbers = Vec::with_capacity(16);
    for r in 0..3 {
        numbers.extend(rows[..3].iter().map(|row| row[r]));
        numbers.push(rows[3][r]);
    }
    numbers.extend([0.0, 0.0, 0.0, 1.0]);
    let numbers: Vec<String> = numbers.iter().map(|v| format!("{v}")).collect();
    numbers.join(" ")
}

fn model_head(out: &mut String) {
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        out,
        r#"<model unit="millimeter" xml:lang="en-US" xmlns="{CORE_NS}" xmlns:BambuStudio="{BAMBU_NS}" xmlns:p="{PRODUCTION_NS}" requiredextensions="p">"#
    );
}

/// One object's file under `3D/Objects/`: the meshes of its parts that
/// are not another part's, each an object of its own.
fn object_file(meshes: &[(u32, u32, &Mesh, Option<&Extras>)]) -> String {
    let size: usize = meshes
        .iter()
        .map(|(_, _, mesh, _)| mesh.vertices.len() * 60 + mesh.triangles.len() * 50)
        .sum();
    let mut out = String::with_capacity(size + 512);
    model_head(&mut out);
    out.push_str(" <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n <resources>\n");
    mesh_objects(&mut out, meshes, true);
    out.push_str(" </resources>\n <build/>\n</model>\n");
    out
}

/// The meshes, each an `<object>` of its own; with the production
/// extension, each with its UUID.
fn mesh_objects(out: &mut String, meshes: &[(u32, u32, &Mesh, Option<&Extras>)], production: bool) {
    for (id, uuid, mesh, extras) in meshes {
        if production {
            let _ = writeln!(
                out,
                r#"  <object id="{id}" p:UUID="{uuid:08x}{SUB_OBJECT_UUID}" type="model">"#
            );
        } else {
            let _ = writeln!(out, r#"  <object id="{id}" type="model">"#);
        }
        out.push_str("   <mesh>\n    <vertices>\n");
        for [x, y, z] in &mesh.vertices {
            let _ = writeln!(out, r#"     <vertex x="{x}" y="{y}" z="{z}"/>"#);
        }
        out.push_str("    </vertices>\n    <triangles>\n");
        let mut carried = extras.map(|extras| extras.iter().peekable());
        for (at, [a, b, c]) in mesh.triangles.iter().enumerate() {
            let _ = write!(out, r#"     <triangle v1="{a}" v2="{b}" v3="{c}""#);
            if let Some(carried) = &mut carried {
                while let Some((index, _)) = carried.peek() {
                    if (*index as usize) < at {
                        carried.next();
                    } else {
                        break;
                    }
                }
                if let Some((index, text)) = carried.peek()
                    && *index as usize == at
                {
                    out.push_str(text);
                    carried.next();
                }
            }
            out.push_str("/>\n");
        }
        out.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    }
}

/// A name fit for a path in the package: the reference uses an object's
/// name as it is, which a `/` would cut in two.
fn path_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    if cleaned.is_empty() {
        "object".to_owned()
    } else {
        cleaned
    }
}

fn subtype(kind: PartKind) -> &'static str {
    match kind {
        PartKind::Normal => "normal_part",
        PartKind::Negative => "negative_part",
        PartKind::Modifier | PartKind::PreciseSeam => "modifier_part",
        PartKind::SupportBlocker => "support_blocker",
        PartKind::SupportEnforcer => "support_enforcer",
    }
}

/// An `extruder` as the reference reads it: 0 or none for "not chosen".
fn chosen(entries: &[Entry]) -> Option<usize> {
    Entry::value_of(entries, "extruder")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|extruder| *extruder > 0)
}

/// Sets a metadata entry in place, or adds it at the end; none takes it
/// away.
fn set_entry(entries: &mut Vec<Entry>, key: &str, value: Option<String>) {
    let at = entries.iter().position(|entry| entry.key() == Some(key));
    match (at, value) {
        (Some(at), Some(value)) => {
            entries[at] = Entry::Meta {
                key: key.to_owned(),
                value,
            }
        }
        (Some(at), None) => {
            entries.remove(at);
        }
        (None, Some(value)) => entries.push(Entry::Meta {
            key: key.to_owned(),
            value,
        }),
        (None, None) => {}
    }
}

fn write_entries(out: &mut String, entries: &[Entry], indent: &str) {
    for entry in entries {
        match entry {
            Entry::Meta { key, value } => {
                let _ = writeln!(
                    out,
                    r#"{indent}<metadata key="{}" value="{}"/>"#,
                    escape(key),
                    escape(value)
                );
            }
            Entry::Raw(raw) => {
                let _ = writeln!(out, "{indent}{raw}");
            }
        }
    }
}

/// An object's own entries and each part's, as the object and its parts
/// stand now. What the file had is kept as it was wherever the part is
/// still what it was read as; what has been edited since (a name, a kind,
/// a filament, where a part stands in its object) is written as it is now,
/// in its place among the entries, as the reference writes it
/// (bbs_3mf.cpp `_add_model_config_file_to_archive`). A part's own
/// filament is written as its `extruder`, and none for one that follows its
/// object ("default"), which the reference reads as the object's
/// (Model.cpp `ModelVolume::extruder_id`).
fn entries_of(object: &Object, name: &str) -> (Vec<Entry>, Vec<Vec<Entry>>) {
    let own = object.kept.as_deref();
    let mut entries = own.map(|own| own.entries.clone()).unwrap_or_default();
    // The name as it stands.
    match entries.iter().position(|entry| entry.key() == Some("name")) {
        Some(at) => {
            entries[at] = Entry::Meta {
                key: "name".into(),
                value: name.to_owned(),
            }
        }
        None if !name.is_empty() => entries.insert(
            0,
            Entry::Meta {
                key: "name".into(),
                value: name.to_owned(),
            },
        ),
        None => {}
    }
    // The object's own filament, where it is not what the file says.
    if own.is_none() || chosen(&entries) != object.filament {
        set_entry(
            &mut entries,
            "extruder",
            object.filament.map(|filament| filament.to_string()),
        );
    }
    let parts = object.parts.iter().map(part_entries).collect();
    (entries, parts)
}

/// A part's entries: see [`entries_of`].
fn part_entries(part: &Part) -> Vec<Entry> {
    let kept = part.kept.as_deref();
    let read = kept.and_then(|kept| kept.read.as_ref());
    let mut entries = kept.map(|kept| kept.entries.clone()).unwrap_or_default();
    // Its kind, where it has been changed: a Precise Seam part is written a
    // modifier that names its seam type, with its settings kept dormant
    // under a prefix; any other kind has neither.
    let was_seam = read.map_or_else(
        || {
            entries
                .iter()
                .any(|entry| entry.key() == Some("precise_seam_type"))
        },
        |read| read.kind == PartKind::PreciseSeam,
    );
    let is_seam = part.kind == PartKind::PreciseSeam;
    if read.is_none_or(|read| read.kind != part.kind) && was_seam != is_seam {
        for entry in &mut entries {
            if let Entry::Meta { key, .. } = entry {
                if is_seam && kept::is_setting(key) {
                    *key = format!("{}{key}", kept::PRECISE_SEAM_PREFIX);
                } else if !is_seam && let Some(bare) = key.strip_prefix(kept::PRECISE_SEAM_PREFIX) {
                    *key = bare.to_owned();
                }
            }
        }
        entries.retain(|entry| entry.key() != Some("precise_seam_type"));
        if is_seam {
            // The type the reference gives a part made one
            // (GUI_ObjectList.cpp `change_part_type`).
            entries.insert(
                0,
                Entry::Meta {
                    key: "precise_seam_type".into(),
                    value: "precise_seam_center".into(),
                },
            );
        }
    }
    // A part the file did not describe (an object that is one mesh) is
    // given its name and where it stands, as the reference writes them.
    let described = kept.is_some_and(|kept| kept.subtype.is_some());
    // Its name, where it is not what was read.
    if let Some(name) = &part.name
        && (!described || read.is_none_or(|read| read.name.as_ref() != Some(name)))
    {
        let value = Some(name.clone());
        if entries.iter().any(|entry| entry.key() == Some("name")) {
            set_entry(&mut entries, "name", value);
        } else {
            let at = usize::from(entries.first().and_then(Entry::key) == Some("precise_seam_type"));
            entries.insert(
                at,
                Entry::Meta {
                    key: "name".into(),
                    value: name.clone(),
                },
            );
        }
    }
    // Where it stands in its object, where it has moved: the reference
    // reads this only for "Reload from disk" (`source.transform`); the
    // component's transform is what places it.
    let has_matrix = entries.iter().any(|entry| entry.key() == Some("matrix"));
    if read.is_none_or(|read| read.transform != part.transform) || (!described && !has_matrix) {
        let value = matrix_text(&part.transform);
        if has_matrix {
            set_entry(&mut entries, "matrix", Some(value));
        } else {
            let at = entries
                .iter()
                .position(|entry| entry.key() == Some("name"))
                .map_or(0, |at| at + 1);
            entries.insert(
                at,
                Entry::Meta {
                    key: "matrix".into(),
                    value,
                },
            );
        }
    }
    // Its own filament, where it is not what was read.
    let as_read = read.map_or_else(|| chosen(&entries), |read| read.own_filament);
    if as_read != part.own_filament {
        let value = part.own_filament.map(|filament| filament.to_string());
        if entries.iter().any(|entry| entry.key() == Some("extruder")) || value.is_none() {
            set_entry(&mut entries, "extruder", value);
        } else if let Some(value) = value {
            // After its other metadata, before what is not metadata.
            let at = entries
                .iter()
                .rposition(|entry| matches!(entry, Entry::Meta { .. }))
                .map_or(0, |at| at + 1);
            entries.insert(
                at,
                Entry::Meta {
                    key: "extruder".into(),
                    value,
                },
            );
        }
    }
    if !entries
        .iter()
        .any(|entry| matches!(entry, Entry::Raw(raw) if raw.starts_with("<mesh_stat")))
    {
        entries.push(Entry::Raw(
            r#"<mesh_stat edges_fixed="0" degenerate_facets="0" facets_removed="0" facets_reversed="0" backwards_edges="0"/>"#
                .into(),
        ));
    }
    entries
}

/// The `subtype` a part is written with: as the file had it while its kind
/// is what was read.
fn subtype_of(part: &Part) -> String {
    let kept = part.kept.as_deref();
    let unchanged = kept
        .and_then(|kept| kept.read.as_ref())
        .is_none_or(|read| read.kind == part.kind);
    match kept.and_then(|kept| kept.subtype.clone()) {
        Some(written) if unchanged => written,
        _ => subtype(part.kind).to_owned(),
    }
}

/// Where one part's mesh is written: the object file and the id there.
#[derive(Clone)]
struct Placed {
    path: String,
    id: u32,
}

/// Whether two parts' meshes can be written once for both: the same
/// mesh, painted alike (bbs_3mf.cpp: `ShareMesh` compares every kind of
/// paint).
pub(crate) fn extras_of(part: &Part) -> Option<&Arc<Extras>> {
    let kept = part.kept.as_deref()?;
    // Painted here: of the mesh it was painted on.
    if let Some(painted) = &kept.painted_on {
        return Arc::ptr_eq(&painted.0, &part.mesh)
            .then_some(kept.extras.as_ref())
            .flatten();
    }
    // Of the mesh it was read with, by the triangle's place: not of a mesh
    // that has taken its place.
    if let Some(read) = &kept.read
        && !Arc::ptr_eq(&read.mesh, &part.mesh)
    {
        return None;
    }
    kept.extras.as_ref()
}

fn same_extras(a: Option<&Arc<Extras>>, b: Option<&Arc<Extras>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        _ => false,
    }
}

/// Writes the package to `out`.
pub fn write_to<W: Write + Seek>(save: &Save<'_>, out: W) -> Result<Written, Error> {
    package(save, out, Mode::Layout(Layout::Project))
}

/// [`write_to`], laid out as `layout` says.
pub fn write_to_as<W: Write + Seek>(
    save: &Save<'_>,
    out: W,
    layout: Layout,
) -> Result<Written, Error> {
    package(save, out, Mode::Layout(layout))
}

fn package<W: Write + Seek>(save: &Save<'_>, out: W, mode: Mode<'_>) -> Result<Written, Error> {
    let project = save.project;
    let sliced = match mode {
        Mode::Sliced(sliced) => Some(sliced),
        Mode::Layout(_) => None,
    };
    let core = matches!(mode, Mode::Layout(Layout::Core));
    let kept = project.kept.as_deref();
    let time = stamp();
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(time)
        .unix_permissions(0o644);
    let stored = deflated.compression_method(CompressionMethod::Stored);
    let mut zip = Package {
        zip: ZipWriter::new(out),
        entries: Vec::new(),
        deflated,
        stored,
    };

    // [Content_Types].xml (bbs_3mf.cpp `_add_content_types_file_to_archive`).
    zip.add("[Content_Types].xml",
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n",
            " <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n",
            " <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n",
            " <Default Extension=\"png\" ContentType=\"image/png\"/>\n",
            " <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\n",
            " <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\n",
            " <Default Extension=\"json\" ContentType=\"application/json\"/>\n",
            " <Default Extension=\"gcode\" ContentType=\"text/x.gcode\"/>\n",
            "</Types>"
        )
        .as_bytes(),
    )?;

    // Each plate's pictures, under its number as it stands, while they
    // still show it.
    let mut pictures: Vec<(u32, Picture)> = Vec::new();
    if let Some(sliced) = sliced {
        // A sliced file's are those drawn for it, in the reference's
        // order: each plate's picture and its small one, then each one's
        // without light, then each one's from above and to pick by.
        for (plate, drawn) in &sliced.pictures {
            for (picture, bytes) in [
                (Picture::Plate, &drawn.plate),
                (Picture::Small, &drawn.small),
            ] {
                if !bytes.is_empty() {
                    zip.add(&picture.file(*plate), bytes)?;
                    pictures.push((*plate, picture));
                }
            }
        }
        for (plate, drawn) in &sliced.pictures {
            if !drawn.no_light.is_empty() {
                zip.add(&Picture::NoLight.file(*plate), &drawn.no_light)?;
                pictures.push((*plate, Picture::NoLight));
            }
        }
        for (plate, drawn) in &sliced.pictures {
            for (picture, bytes) in [(Picture::Top, &drawn.top), (Picture::Pick, &drawn.pick)] {
                if !bytes.is_empty() {
                    zip.add(&picture.file(*plate), bytes)?;
                    pictures.push((*plate, picture));
                }
            }
        }
        // The first layer's boxes of each plate sliced.
        for plate in &sliced.plates {
            if let Some(boxes) = &plate.boxes {
                zip.add(
                    &format!("Metadata/plate_{}.json", plate.plate),
                    boxes.json().as_bytes(),
                )?;
            }
        }
    } else {
        for plate in &project.plates {
            let Some(own) = &plate.kept else { continue };
            for (picture, bytes) in &own.pictures {
                zip.add(&picture.file(plate.id), bytes)?;
                pictures.push((plate.id, *picture));
            }
        }
    }

    // Number the meshes, the objects and their files as the reference
    // does: each mesh written takes the next number, then its object.
    let mut counter = 1u32;
    let mut written: Vec<(Arc<Mesh>, Option<Arc<Extras>>, Placed)> = Vec::new();
    let mut objects_out: Vec<ObjectOut> = Vec::new();
    let modelled: &[Object] = if sliced.is_some() {
        &[]
    } else {
        &project.objects
    };
    for (at, object) in modelled.iter().enumerate() {
        // An object a file names nowhere is named as the reference names
        // it when it reads that file (bbs_3mf.cpp `_load_model_from_file`:
        // "Object_" and its place).
        let name = match (&object.name, &object.kept) {
            (Some(name), _) => name.clone(),
            (None, Some(own)) if Entry::value_of(&own.entries, "name").is_none() => {
                format!("Object_{}", own.index)
            }
            (None, _) => String::new(),
        };
        let path = format!("3D/Objects/{}_{}.model", path_name(&name), at + 1);
        let backup = at as u32 + 1;
        let mut components = Vec::new();
        let mut meshes = Vec::new();
        let mut shared = false;
        for (index, part) in object.parts.iter().enumerate() {
            let extras = extras_of(part);
            // Without `ShareMesh` every part has a mesh of its own.
            let found = written.iter().find(|(mesh, other, _)| {
                !core && Arc::ptr_eq(mesh, &part.mesh) && same_extras(other.as_ref(), extras)
            });
            let placed = match found {
                Some((_, _, placed)) => {
                    shared = true;
                    placed.clone()
                }
                None => {
                    let placed = Placed {
                        path: path.clone(),
                        id: counter,
                    };
                    counter += 1;
                    meshes.push((placed.id, index as u32 + (backup << 16), index));
                    written.push((part.mesh.clone(), extras.cloned(), placed.clone()));
                    placed
                }
            };
            components.push(placed);
        }
        let id = counter;
        counter += 1;
        objects_out.push(ObjectOut {
            id,
            backup,
            path,
            name,
            components,
            meshes,
            shared,
        });
    }

    // 3D/3dmodel.model.
    let mut root = String::new();
    if core {
        let _ = writeln!(root, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        let _ = writeln!(
            root,
            r#"<model unit="millimeter" xml:lang="en-US" xmlns="{CORE_NS}" xmlns:BambuStudio="{BAMBU_NS}">"#
        );
    } else {
        model_head(&mut root);
    }
    let mut header: BTreeMap<String, String> = kept
        .map(|kept| kept.header.iter().cloned().collect())
        .unwrap_or_default();
    for key in [
        "Copyright",
        "Description",
        "Designer",
        "DesignerCover",
        "License",
        "Origin",
        "Title",
    ] {
        header.entry(key.to_owned()).or_default();
    }
    // A sliced file keeps the name the reference has always given it
    // (bbs_3mf.cpp `_add_model_file_to_archive`: "a Bambu printer /
    // Bambu Studio artifact rather than a project").
    let application = if sliced.is_some() {
        "BambuStudio"
    } else {
        "EdgeSlicer"
    };
    header.insert(
        "Application".into(),
        format!("{application}-{REFERENCE_VERSION}"),
    );
    header.insert("BambuStudio:3mfVersion".into(), "1".into());
    // The reference writes neither when the file was made nor who made it
    // (bbs_3mf.cpp, "PRIVACY").
    header.insert("CreationDate".into(), String::new());
    header.insert("ModificationDate".into(), String::new());
    header.insert("DesignerUserId".into(), String::new());
    header.remove("bamboo_slicer:Version3mf");
    let first_plate = project.plates.first().map(|plate| plate.id);
    // A cover named in the header that is no longer written is not named.
    let carried = |name: &str| {
        kept.is_some_and(|kept| {
            kept.files
                .iter()
                .any(|file| file.name.eq_ignore_ascii_case(name.trim_start_matches('/')))
        })
    };
    // A sliced file carries the header as it was read, as the reference's
    // does (`model_info->metadata_items`).
    for key in ["Thumbnail_Middle", "Thumbnail_Small"] {
        if sliced.is_some() {
            break;
        }
        if let Some(target) = header.get(key).cloned() {
            let shown = Picture::ALL.iter().any(|picture| {
                first_plate.is_some_and(|plate| {
                    pictures.contains(&(plate, *picture))
                        && picture.file(plate) == target.trim_start_matches('/')
                })
            });
            if !target.is_empty() && !shown && !carried(&target) {
                header.remove(key);
            }
        }
    }
    for (name, value) in &header {
        let _ = writeln!(
            root,
            r#" <metadata name="{}">{}</metadata>"#,
            escape(name),
            escape(value)
        );
    }
    root.push_str(" <resources>\n");
    // The generic layout has every mesh here, each object's in turn, and
    // then the objects made of them (`_add_model_file_to_archive` without
    // `SplitModel`).
    if core {
        let inline: Vec<String> = objects_out
            .par_iter()
            .zip(modelled)
            .map(|(out, object)| {
                let meshes: Vec<(u32, u32, &Mesh, Option<&Extras>)> = out
                    .meshes
                    .iter()
                    .map(|(id, uuid, index)| {
                        let part = &object.parts[*index];
                        (
                            *id,
                            *uuid,
                            part.mesh.as_ref(),
                            extras_of(part).map(|e| e.as_ref()),
                        )
                    })
                    .collect();
                let mut text = String::new();
                mesh_objects(&mut text, &meshes, false);
                text
            })
            .collect();
        for text in inline {
            root.push_str(&text);
        }
        for (object, source) in objects_out.iter().zip(modelled) {
            let _ = writeln!(root, r#"  <object id="{}" type="model">"#, object.id);
            root.push_str("   <components>\n");
            for (placed, part) in object.components.iter().zip(&source.parts) {
                let _ = writeln!(
                    root,
                    r#"    <component objectid="{}" transform="{}"/>"#,
                    placed.id,
                    transform_text(&part.transform)
                );
            }
            root.push_str("   </components>\n  </object>\n");
        }
    }
    for (object, source) in objects_out.iter().zip(modelled) {
        if core {
            break;
        }
        let suffix = if object.shared {
            OBJECT_UUID_SHARED
        } else {
            OBJECT_UUID
        };
        let _ = writeln!(
            root,
            r#"  <object id="{}" p:UUID="{:08x}{suffix}" type="model">"#,
            object.id, object.backup
        );
        root.push_str("   <components>\n");
        for (index, (placed, part)) in object.components.iter().zip(&source.parts).enumerate() {
            let _ = writeln!(
                root,
                r#"    <component p:path="/{}" objectid="{}" p:UUID="{:08x}{COMPONENT_UUID}" transform="{}"/>"#,
                escape(&placed.path),
                placed.id,
                index as u32 + (object.backup << 16),
                transform_text(&part.transform)
            );
        }
        root.push_str("   </components>\n  </object>\n");
    }
    root.push_str(" </resources>\n");
    let items: Vec<(u32, &crate::Instance)> = modelled
        .iter()
        .zip(&objects_out)
        .flat_map(|(object, out)| object.instances.iter().map(move |copy| (out.id, copy)))
        .collect();
    if items.is_empty() {
        root.push_str(" <build/>\n");
    } else if core {
        root.push_str(" <build>\n");
        for (id, copy) in items {
            let _ = writeln!(
                root,
                r#"  <item objectid="{id}" transform="{}" printable="{}"/>"#,
                transform_text(&copy.transform),
                u8::from(copy.printable)
            );
        }
        root.push_str(" </build>\n");
    } else {
        let _ = writeln!(root, r#" <build p:UUID="{BUILD_UUID}">"#);
        for (id, copy) in items {
            let _ = writeln!(
                root,
                r#"  <item objectid="{id}" p:UUID="{id:08x}{BUILD_ITEM_UUID}" transform="{}" printable="{}"/>"#,
                transform_text(&copy.transform),
                u8::from(copy.printable)
            );
        }
        root.push_str(" </build>\n");
    }
    root.push_str("</model>\n");
    zip.add(MODEL, root.as_bytes())?;

    // 3D/_rels/3dmodel.model.rels: each object's file.
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
    );
    for (at, object) in objects_out.iter().enumerate() {
        let _ = writeln!(
            rels,
            r#" <Relationship Target="/{}" Id="rel-{}" Type="{MODEL_TYPE}"/>"#,
            escape(&object.path),
            at + 1
        );
    }
    rels.push_str("</Relationships>");
    if !objects_out.is_empty() && !core {
        zip.add(MODEL_RELS, rels.as_bytes())?;
    }

    // The objects' files, made side by side.
    let files: Vec<String> = objects_out
        .par_iter()
        .zip(modelled)
        .filter(|_| !core)
        .map(|(out, object)| {
            let meshes: Vec<(u32, u32, &Mesh, Option<&Extras>)> = out
                .meshes
                .iter()
                .map(|(id, uuid, index)| {
                    let part = &object.parts[*index];
                    (
                        *id,
                        *uuid,
                        part.mesh.as_ref(),
                        extras_of(part).map(|e| e.as_ref()),
                    )
                })
                .collect();
            object_file(&meshes)
        })
        .collect();
    for (object, text) in objects_out.iter().zip(&files) {
        zip.add(&object.path, text.as_bytes())?;
    }
    drop(files);

    // What goes by an object's place in the list, under its place now.
    let place_files = by_place_files(project, kept);
    for (name, text) in &place_files {
        zip.add(name, text.as_bytes())?;
    }

    // The cuts kept editable, under each object's place now, and the
    // uncut meshes they name (`_add_cut_recipe_file_to_archive`).
    let recipes = project
        .objects
        .iter()
        .enumerate()
        .filter_map(|(at, object)| {
            let own = object.kept.as_deref()?;
            let recipe = own.recipe.as_deref()?;
            Some((at + 1, recipe, own.recipe_element.as_deref()))
        });
    if let Some((text, meshes)) = crate::recipe::write(recipes) {
        zip.add(crate::recipe::RECIPES, text.as_bytes())?;
        for (name, bytes) in &meshes {
            zip.add(name, bytes)?;
        }
    }

    // Custom G-code at layers, by plate number (the engine keeps each
    // plate's with it, so it is under the plate's number as it stands).
    if let Some(text) = custom_gcode(project, kept) {
        zip.add(kept::CUSTOM_GCODE, text.as_bytes())?;
    }

    if let Some(settings) = &save.settings {
        zip.add(PROJECT_SETTINGS, settings.as_bytes())?;
    }

    // The file's own entries, as they were.
    let holds = kept.is_some_and(|kept| by_place_holds(project, kept));
    if let Some(kept) = kept {
        for file in &kept.files {
            let wanted = match file.carry {
                Carry::AsIs => true,
                Carry::Preset => save.presets,
                Carry::ByObjectPlace => holds && sliced.is_none(),
            };
            if wanted && file.name != PROJECT_SETTINGS && !zip.entries.contains(&file.name) {
                zip.add(&file.name, &file.bytes)?;
            }
        }
    }

    // Each plate's G-code's MD5, then the G-code itself, and what names
    // them (`_add_gcode_file_to_archive`, `_add_model_config_file_to_archive`).
    if let Some(sliced) = sliced {
        for plate in &sliced.plates {
            zip.add(
                &format!("Metadata/plate_{}.gcode.md5", plate.plate),
                sliced::gcode_md5(plate.gcode).as_bytes(),
            )?;
        }
        for plate in &sliced.plates {
            zip.add(
                &format!("Metadata/plate_{}.gcode", plate.plate),
                plate.gcode,
            )?;
        }
        let mut rels = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
        );
        for (at, plate) in sliced.plates.iter().enumerate() {
            let _ = writeln!(
                rels,
                r#" <Relationship Target="/Metadata/plate_{}.gcode" Id="rel-{}" Type="http://schemas.bambulab.com/package/2021/gcode"/>"#,
                plate.plate,
                at + 1
            );
        }
        rels.push_str("</Relationships>");
        if !sliced.plates.is_empty() {
            zip.add("Metadata/_rels/model_settings.config.rels", rels.as_bytes())?;
        }
    }

    // Metadata/model_settings.config.
    let settings = match sliced {
        Some(sliced) => sliced_model_settings(project, sliced, &pictures, save.filaments),
        None => model_settings(project, &objects_out, &pictures, save.filaments),
    };
    zip.add(MODEL_SETTINGS, settings.as_bytes())?;

    // Metadata/slice_info.config: a project saved unsliced has its header
    // alone (`_add_slice_info_config_file_to_archive`).
    let version: Vec<String> = REFERENCE_VERSION
        .split('.')
        .map(|part| format!("{part:0>2}"))
        .collect();
    let slice_info = format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n  <header>\n",
            "    <header_item key=\"X-BBL-Client-Type\" value=\"slicer\"/>\n",
            "    <header_item key=\"X-BBL-Client-Version\" value=\"{}\"/>\n",
            "  </header>\n</config>\n"
        ),
        version.join(".")
    );
    match sliced {
        Some(sliced) => {
            zip.add(
                kept::SLICE_INFO,
                sliced::slice_info(&sliced.plates).as_bytes(),
            )?;
            let ids: Vec<u32> = project.plates.iter().map(|plate| plate.id).collect();
            let ids = if ids.is_empty() { vec![1] } else { ids };
            zip.add(
                "Metadata/filament_sequence.json",
                sliced::filament_sequence(&ids, &sliced.plates).as_bytes(),
            )?;
        }
        None => zip.add(kept::SLICE_INFO, slice_info.as_bytes())?,
    }

    // _rels/.rels: the model, and the cover (the file's own, else the
    // first plate's picture), as `_add_relationships_file_to_archive`.
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
    );
    let _ = writeln!(
        rels,
        r#" <Relationship Target="/{MODEL}" Id="rel-1" Type="{MODEL_TYPE}"/>"#
    );
    // A sliced file of one plate shows that plate's picture; of them all,
    // the first plate's (`_add_relationships_file_to_archive`).
    let plate_one = sliced
        .and_then(|sliced| sliced.plate)
        .or(first_plate)
        .unwrap_or(1);
    let has_picture = |picture: Picture| pictures.contains(&(plate_one, picture));
    for (id, cover, picture, kind) in [
        (
            2,
            "Auxiliaries/.thumbnails/thumbnail_3mf.png",
            Picture::Plate,
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail",
        ),
        (
            4,
            "Auxiliaries/.thumbnails/thumbnail_middle.png",
            Picture::Plate,
            "http://schemas.bambulab.com/package/2021/cover-thumbnail-middle",
        ),
        (
            5,
            "Auxiliaries/.thumbnails/thumbnail_small.png",
            Picture::Small,
            "http://schemas.bambulab.com/package/2021/cover-thumbnail-small",
        ),
    ] {
        let plate_shown = sliced.is_some_and(|sliced| sliced.plate.is_some());
        let target = if !plate_shown && zip.entries.iter().any(|entry| entry == cover) {
            Some(cover.to_owned())
        } else if has_picture(picture) {
            Some(picture.file(plate_one))
        } else {
            None
        };
        if let Some(target) = target {
            let _ = writeln!(
                rels,
                r#" <Relationship Target="/{}" Id="rel-{id}" Type="{kind}"/>"#,
                escape(&target)
            );
        }
    }
    rels.push_str("</Relationships>");
    zip.add("_rels/.rels", rels.as_bytes())?;

    zip.zip
        .finish()
        .map_err(|e| Error::Archive(e.to_string()))?;
    Ok(Written {
        bytes: 0,
        entries: zip.entries,
    })
}

/// The package being written, and the names written so far.
struct Package<W: Write + Seek> {
    zip: ZipWriter<W>,
    entries: Vec<String>,
    deflated: SimpleFileOptions,
    stored: SimpleFileOptions,
}

impl<W: Write + Seek> Package<W> {
    /// An entry; pictures and what is packed already are stored, the
    /// rest deflated, as the reference does (`_add_file_to_archive`).
    fn add(&mut self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let lower = name.to_ascii_lowercase();
        let raw = [".png", ".jpg", ".jpeg", ".mp4", ".zip", ".3mf"]
            .iter()
            .any(|ext| lower.ends_with(ext));
        self.zip
            .start_file(name, if raw { self.stored } else { self.deflated })
            .map_err(|e| Error::Archive(e.to_string()))?;
        self.zip.write_all(bytes)?;
        self.entries.push(name.to_owned());
        Ok(())
    }
}

/// One object as it is written.
struct ObjectOut {
    id: u32,
    /// What the reference calls its backup id: its place in the list.
    backup: u32,
    path: String,
    name: String,
    /// For each part, where its mesh is.
    components: Vec<Placed>,
    /// The meshes written in its own file: id, UUID number, part.
    meshes: Vec<(u32, u32, usize)>,
    shared: bool,
}

/// The files that go by an object's place in the list, under each
/// object's place as it stands.
fn by_place_files(project: &Project, kept: Option<&Kept>) -> Vec<(&'static str, String)> {
    let mut files = Vec::new();
    let owns = |pick: fn(&ObjectKept) -> Option<&String>| -> Vec<(usize, &String)> {
        project
            .objects
            .iter()
            .enumerate()
            .filter_map(|(at, object)| Some((at + 1, pick(object.kept.as_deref()?)?)))
            .collect()
    };
    let lines = owns(|own| own.layer_heights.as_ref());
    if !lines.is_empty() {
        let text: String = lines
            .iter()
            .map(|(at, values)| format!("object_id={at}|{values}\n"))
            .collect();
        files.push((kept::LAYER_HEIGHTS, text));
    }
    let xml = |objects: Vec<(usize, &String)>| -> Option<String> {
        if objects.is_empty() {
            return None;
        }
        let mut text = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<objects>\n");
        for (at, inside) in objects {
            let _ = writeln!(text, " <object id=\"{at}\">{inside}</object>");
        }
        text.push_str("</objects>\n");
        Some(text)
    };
    if let Some(text) = xml(owns(|own| own.layer_ranges.as_ref())) {
        files.push((kept::LAYER_RANGES, text));
    }
    let points = owns(|own| own.brim_points.as_ref());
    if !points.is_empty() {
        let mut text = kept
            .and_then(|kept| kept.brim_version.clone())
            .unwrap_or_else(|| "brim_points_format_version=1".into());
        text.push('\n');
        for (at, values) in points {
            let _ = writeln!(text, "object_id={at}|{values}");
        }
        files.push((kept::BRIM_POINTS, text));
    }
    let cuts: Vec<(usize, String)> = project
        .objects
        .iter()
        .enumerate()
        .filter_map(|(at, object)| Some((at + 1, cut_of(object)?.ok()?)))
        .collect();
    if let Some(text) = xml(cuts.iter().map(|(at, text)| (*at, text)).collect()) {
        files.push((kept::CUT_INFORMATION, text));
    }
    files
}

/// An object's cut information (`Metadata/cut_information.xml`) with each
/// connector naming its part by the part's place now, as the reference
/// writes it (bbs_3mf.cpp `_add_cut_information_file_to_archive`, which
/// numbers the volumes in order): none when it has none; an error when a
/// part a connector names is no longer there as it was (deleted, or split
/// into pieces), as the reference would refuse the file ("Invalid cut
/// connector volume_id") or give the connector to another part.
fn cut_of(object: &Object) -> Option<Result<String, ()>> {
    let own = object.kept.as_ref()?;
    // A cut made here: written afresh, each connector by its part's place.
    if own.cut.is_none()
        && let Some(cut) = own.cut_id
    {
        let mut out = format!(
            "\n  <cut_id id=\"{}\" check_sum=\"{}\" connectors_cnt=\"{}\"/>",
            cut.id, cut.check_sum, cut.connectors
        );
        let connectors: Vec<String> = object
            .parts
            .iter()
            .enumerate()
            .filter_map(|(at, part)| {
                let info = part.kept.as_ref()?.connector.as_ref()?;
                let extra: String = info
                    .extra
                    .iter()
                    .map(|(name, value)| format!(" {name}=\"{}\"", *value as f32))
                    .collect();
                Some(format!(
                    "\n   <connector volume_id=\"{at}\" type=\"{}\" r_tolerance=\"{}\" h_tolerance=\"{}\"{extra}/>",
                    info.kind, info.radius_tolerance as f32, info.height_tolerance as f32
                ))
            })
            .collect();
        if !connectors.is_empty() {
            out.push_str("\n  <connectors>");
            out.extend(connectors);
            out.push_str("\n  </connectors>");
        }
        out.push_str("\n ");
        return Some(Ok(out));
    }
    let text = own.cut.as_ref()?;
    // Where each part read from the file stands now.
    let mut now: HashMap<usize, usize> = HashMap::new();
    for (at, part) in object.parts.iter().enumerate() {
        if let Some(read) = part.kept.as_ref().and_then(|kept| kept.read.as_ref()) {
            now.entry(read.index).or_insert(at);
        }
    }
    const KEY: &str = "volume_id=\"";
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(found) = rest.find(KEY) {
        let start = found + KEY.len();
        out.push_str(&rest[..start]);
        let Some(length) = rest[start..].find('"') else {
            return Some(Err(()));
        };
        let number = &rest[start..start + length];
        let Some(place) = number
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|was| now.get(&was))
        else {
            return Some(Err(()));
        };
        if number.trim().parse::<usize>().ok() == Some(*place) {
            out.push_str(number);
        } else {
            out.push_str(&place.to_string());
        }
        rest = &rest[start + length..];
    }
    out.push_str(rest);
    Some(Ok(out))
}

fn custom_gcode(project: &Project, kept: Option<&Kept>) -> Option<String> {
    if project.custom_gcode.is_empty() {
        return kept.and_then(|kept| kept.custom_gcode.clone());
    }
    Some(crate::custom_gcode::text(
        project.custom_gcode.iter().map(|(id, info)| (*id, info)),
    ))
}

/// Each copy's number (`identify_id`), by object and copy: the one it was
/// read with while its object has all its copies, else one no other copy
/// has. The number a printer that skips objects knows each copy by, in a
/// sliced file's `slice_info.config` and pick picture.
pub fn identify_numbers(project: &Project) -> HashMap<(usize, usize), u32> {
    let mut identify = HashMap::new();
    let mut taken = std::collections::HashSet::new();
    for (at, object) in project.objects.iter().enumerate() {
        let Some(own) = &object.kept else { continue };
        if own.copies != object.instances.len() {
            continue;
        }
        for (copy, number) in own.identify.iter().enumerate() {
            if let Some(number) = number
                && taken.insert(*number)
            {
                identify.insert((at, copy), *number);
            }
        }
    }
    let mut next = taken.iter().max().map_or(1, |most| most + 1);
    for (at, object) in project.objects.iter().enumerate() {
        for copy in 0..object.instances.len() {
            identify.entry((at, copy)).or_insert_with(|| {
                next += 1;
                next - 1
            });
        }
    }
    identify
}

/// `Metadata/model_settings.config` of a sliced file: the plates alone,
/// each with its G-code, pictures and boxes (`_add_model_config_file_to_archive`
/// with `SkipModel`).
fn sliced_model_settings(
    project: &Project,
    sliced: &SlicedFile<'_>,
    pictures: &[(u32, Picture)],
    filaments: usize,
) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    let identify = HashMap::new();
    if project.plates.is_empty() {
        // A model without plates is sliced as plate 1.
        let plate = Plate {
            id: 1,
            name: String::new(),
            first_layer_sequence: Vec::new(),
            instances: Vec::new(),
            kept: None,
        };
        plate_settings(
            &mut out,
            &plate,
            &[],
            pictures,
            filaments,
            &identify,
            Some(sliced),
        );
    }
    for plate in &project.plates {
        plate_settings(
            &mut out,
            plate,
            &[],
            pictures,
            filaments,
            &identify,
            Some(sliced),
        );
    }
    out.push_str("</config>\n");
    out
}

fn model_settings(
    project: &Project,
    objects_out: &[ObjectOut],
    pictures: &[(u32, Picture)],
    filaments: usize,
) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    for (object, written) in project.objects.iter().zip(objects_out) {
        let _ = writeln!(out, r#"  <object id="{}">"#, written.id);
        let (entries, parts) = entries_of(object, &written.name);
        write_entries(&mut out, &entries, "    ");
        for ((part, placed), part_entries) in
            object.parts.iter().zip(&written.components).zip(&parts)
        {
            let kind = subtype_of(part);
            let _ = writeln!(
                out,
                r#"    <part id="{}" subtype="{}">"#,
                placed.id,
                escape(&kind)
            );
            write_entries(&mut out, part_entries, "      ");
            out.push_str("    </part>\n");
        }
        out.push_str("  </object>\n");
    }
    let identify = identify_numbers(project);
    for plate in &project.plates {
        plate_settings(
            &mut out,
            plate,
            objects_out,
            pictures,
            filaments,
            &identify,
            None,
        );
    }
    out.push_str("  <assemble>\n");
    for (object, written) in project.objects.iter().zip(objects_out) {
        let Some(own) = &object.kept else { continue };
        // Kept while every copy is still there, each in its place.
        if own.copies != object.instances.len() {
            continue;
        }
        for (copy, placing) in own.assembly.iter().enumerate() {
            if let Some((transform, offset)) = placing {
                let _ = writeln!(
                    out,
                    r#"   <assemble_item object_id="{}" instance_id="{copy}" transform="{transform}" offset="{offset}" />"#,
                    written.id
                );
            }
        }
    }
    out.push_str("  </assemble>\n");
    if let Some(kept) = &project.kept {
        for raw in &kept.settings_extra {
            let _ = writeln!(out, "  {raw}");
        }
    }
    out.push_str("</config>\n");
    out
}

fn plate_settings(
    out: &mut String,
    plate: &Plate,
    objects_out: &[ObjectOut],
    pictures: &[(u32, Picture)],
    filaments: usize,
    identify: &HashMap<(usize, usize), u32>,
    sliced: Option<&SlicedFile<'_>>,
) {
    out.push_str("  <plate>\n");
    let _ = writeln!(
        out,
        r#"    <metadata key="plater_id" value="{}"/>"#,
        plate.id
    );
    let _ = writeln!(
        out,
        r#"    <metadata key="plater_name" value="{}"/>"#,
        escape(&plate.name)
    );
    let given = sliced
        .map(|sliced| sliced.filament_maps.as_slice())
        .filter(|maps| !maps.is_empty());
    let maps = || match given {
        Some(maps) => maps
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" "),
        None => vec!["1"; filaments.max(1)].join(" "),
    };
    match &plate.kept {
        Some(own) => {
            let mut entries = own.entries.clone();
            // One filament map entry for each filament, as the reference
            // writes it.
            if let Some(Entry::Meta { value, .. }) = entries
                .iter_mut()
                .find(|entry| entry.key() == Some("filament_maps"))
            {
                if given.is_some() {
                    *value = maps();
                } else {
                    let mut values: Vec<String> =
                        value.split_whitespace().map(str::to_owned).collect();
                    values.resize(filaments.max(1), "1".into());
                    *value = values.join(" ");
                }
            }
            write_entries(out, &entries, "    ");
        }
        None => {
            out.push_str("    <metadata key=\"locked\" value=\"false\"/>\n");
            if !plate.first_layer_sequence.is_empty() {
                let order: Vec<String> = plate
                    .first_layer_sequence
                    .iter()
                    .map(u32::to_string)
                    .collect();
                let _ = writeln!(
                    out,
                    r#"    <metadata key="first_layer_print_sequence" value="{}"/>"#,
                    order.join(" ")
                );
            }
            out.push_str("    <metadata key=\"filament_map_mode\" value=\"Auto For Flush\"/>\n");
            let _ = writeln!(
                out,
                r#"    <metadata key="filament_maps" value="{}"/>"#,
                maps()
            );
        }
    }
    let this: Option<&SlicedPlate<'_>> =
        sliced.and_then(|sliced| sliced.plates.iter().find(|one| one.plate == plate.id));
    if sliced.is_some() {
        let file = this.map_or_else(String::new, |this| {
            format!("Metadata/plate_{}.gcode", this.plate)
        });
        let _ = writeln!(out, r#"    <metadata key="gcode_file" value="{file}"/>"#);
    }
    for picture in Picture::ALL {
        if let Some(key) = picture.key()
            && pictures.contains(&(plate.id, picture))
        {
            let _ = writeln!(
                out,
                r#"    <metadata key="{key}" value="{}"/>"#,
                picture.file(plate.id)
            );
        }
    }
    if this.is_some_and(|this| this.boxes.is_some()) {
        let _ = writeln!(
            out,
            r#"    <metadata key="pattern_bbox_file" value="Metadata/plate_{}.json"/>"#,
            plate.id
        );
    }
    // A sliced file has no model: its plates name no copies.
    if sliced.is_some() {
        out.push_str("  </plate>\n");
        return;
    }
    for at in &plate.instances {
        let Some(written) = objects_out.get(at.object) else {
            continue;
        };
        out.push_str("    <model_instance>\n");
        let _ = writeln!(
            out,
            r#"      <metadata key="object_id" value="{}"/>"#,
            written.id
        );
        let _ = writeln!(
            out,
            r#"      <metadata key="instance_id" value="{}"/>"#,
            at.instance
        );
        let _ = writeln!(
            out,
            r#"      <metadata key="identify_id" value="{}"/>"#,
            identify
                .get(&(at.object, at.instance))
                .copied()
                .unwrap_or(0)
        );
        out.push_str("    </model_instance>\n");
    }
    out.push_str("  </plate>\n");
}

impl From<zip::result::ZipError> for Error {
    fn from(error: zip::result::ZipError) -> Self {
        Error::Archive(error.to_string())
    }
}
