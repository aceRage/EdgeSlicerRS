//! What a project file holds beyond what [`crate::Project`] reads into its
//! objects, parts and plates, kept as the file has it so that a project
//! can be written back without losing it ([`crate::write`]): the settings
//! of each object and part, what each triangle carries (every kind of
//! paint), each plate's own settings and pictures, the files that go by
//! an object's place in the list or a plate's number, and every other
//! entry of the package.
//!
//! The reference writes all of this in `bbs_3mf.cpp`
//! (`_BBS_3MF_Exporter`); its reader is `_BBS_3MF_Importer`.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use quick_xml::{Reader, events::Event};

use crate::{
    Error,
    xml::{attributes, malformed},
};

/// What a mesh's triangles carry besides their corners, by the
/// triangle's place in the mesh: the attributes as written
/// (` paint_color="4" paint_seam="8"`).
pub type Extras = Vec<(u32, Box<str>)>;

/// One child of an object, a part or a plate in
/// `Metadata/model_settings.config`.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// `<metadata key=".." value=".."/>`, its key and value unescaped.
    Meta { key: String, value: String },
    /// Anything else (`mesh_stat`, an embossed text's description), as
    /// written.
    Raw(String),
}

impl Entry {
    pub(crate) fn key(&self) -> Option<&str> {
        match self {
            Entry::Meta { key, .. } => Some(key),
            Entry::Raw(_) => None,
        }
    }

    pub(crate) fn value_of<'a>(entries: &'a [Entry], wanted: &str) -> Option<&'a str> {
        entries.iter().find_map(|entry| match entry {
            Entry::Meta { key, value } if key == wanted => Some(value.as_str()),
            _ => None,
        })
    }
}

/// What an object of a file holds that [`crate::Object`] does not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectKept {
    /// Its place in the file's list of objects, from 1: what the files
    /// below were written under.
    pub(crate) index: usize,
    /// How many copies it had.
    pub(crate) copies: usize,
    /// Its metadata and settings (`name`, `extruder`, the settings set for
    /// it alone), in the file's order.
    pub(crate) entries: Vec<Entry>,
    /// Its line of `Metadata/layer_heights_profile.txt`, after the `|`.
    pub(crate) layer_heights: Option<String>,
    /// Inside its `<object>` of `Metadata/layer_config_ranges.xml`.
    pub(crate) layer_ranges: Option<String>,
    /// Its line of `Metadata/brim_ear_points.txt`, after the `|`.
    pub(crate) brim_points: Option<String>,
    /// Inside its `<object>` of `Metadata/cut_information.xml`.
    pub(crate) cut: Option<String>,
    /// Each copy's place in the assembly view (`<assemble_item>`'s
    /// `transform` and `offset`, as written), by the copy's place.
    pub(crate) assembly: Vec<Option<(String, String)>>,
    /// Each copy's `identify_id`, by the copy's place: the number the
    /// reference labels it with in G-code (`; model label id`).
    pub(crate) identify: Vec<Option<u32>>,
}

/// What a part of a file holds that [`crate::Part`] does not. This is the
/// one keeper of a part's metadata: its settings ([`PartKept::settings`])
/// are read from here, and edits that move them (a part split, the last
/// part left of an object) go through here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartKept {
    /// Its `subtype` as written (`normal_part`, `modifier_part`, …).
    pub(crate) subtype: Option<String>,
    /// Its metadata and settings, and its other children, in order.
    pub(crate) entries: Vec<Entry>,
    /// What its mesh's triangles carry; shared by every part that uses
    /// the same mesh of the file. Only for the mesh it was read with.
    pub(crate) extras: Option<Arc<Extras>>,
    /// What the part was read as, to tell what has been edited since:
    /// none for a part the file did not hold as it is (a piece of a part
    /// split).
    pub(crate) read: Option<PartRead>,
}

/// What a part was read as.
#[derive(Clone, Debug)]
pub(crate) struct PartRead {
    /// Its place among its object's parts in the file: what the cut
    /// connectors of `Metadata/cut_information.xml` go by.
    pub index: usize,
    pub name: Option<String>,
    pub kind: crate::PartKind,
    pub own_filament: Option<usize>,
    pub transform: edge_mesh::Transform,
    /// The mesh it was read with, which what its triangles carry is of.
    pub mesh: Arc<edge_mesh::Mesh>,
}

impl PartialEq for PartRead {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
            && self.name == other.name
            && self.kind == other.kind
            && self.own_filament == other.own_filament
            && self.transform == other.transform
            && Arc::ptr_eq(&self.mesh, &other.mesh)
    }
}

/// What a part's metadata says that is not one of its settings: read into
/// [`crate::Part`], or about where it came from (bbs_3mf.cpp
/// `_generate_volumes_new`, the keys it takes before handing the rest to the
/// part's config).
const NOT_SETTINGS: [&str; 18] = [
    "name",
    "extruder",
    "matrix",
    "mesh_shared",
    "volume_type",
    "part_type",
    "precise_seam_type",
    "source_file",
    "source_object_id",
    "source_volume_id",
    "source_offset_x",
    "source_offset_y",
    "source_offset_z",
    "source_in_inches",
    "source_in_meters",
    "cad_body_file",
    "airbrush_field_file",
    "texture_displacement",
];

/// Where a Precise Seam part's settings are kept, dormant, in its metadata
/// (bbs_3mf.cpp `PRECISE_SEAM_CONFIG_PREFIX`).
pub(crate) const PRECISE_SEAM_PREFIX: &str = "precise_seam_config:";

/// Whether a part's metadata key is one of its settings.
pub(crate) fn is_setting(key: &str) -> bool {
    !key.is_empty() && !NOT_SETTINGS.contains(&key)
}

impl PartKept {
    /// What its mesh's triangles carry, as the file has it.
    pub fn extras(&self) -> Option<&Arc<Extras>> {
        self.extras.as_ref()
    }

    /// The part's own settings as the file carries them: every key and
    /// value of its metadata but those read into the part (its name, type,
    /// filament, matrix) and those about where it came from, in the file's
    /// order. Nothing here slices by them; they are written back with the
    /// part.
    pub fn settings(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Meta { key, value } if is_setting(key) => {
                    // A Precise Seam part's, kept dormant under a prefix.
                    let key = key.strip_prefix(PRECISE_SEAM_PREFIX).unwrap_or(key);
                    Some((key.to_owned(), value.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// What one piece of the part keeps when it is split (Model.cpp
    /// `ModelVolume::split`): its kind and settings, as each piece is a copy
    /// of the part; not where it came from (`source` is reset), its text
    /// (`text_configuration` is reset), what its triangles carry (the
    /// facets are reset) or its matrix and mesh statistics, which are of
    /// the piece.
    pub fn piece(&self) -> PartKept {
        let entries = self
            .entries
            .iter()
            .filter(|entry| match entry {
                Entry::Meta { key, .. } => {
                    !(key.starts_with("source_") || key == "matrix" || key == "name")
                }
                Entry::Raw(raw) => !(raw.starts_with("<mesh_stat") || is_text(raw)),
            })
            .cloned()
            .collect();
        PartKept {
            subtype: self.subtype.clone(),
            entries,
            extras: None,
            read: None,
        }
    }

    /// The part without its settings: what is left of it once they are
    /// given to its object.
    pub fn without_settings(&self) -> PartKept {
        PartKept {
            entries: self
                .entries
                .iter()
                .filter(|entry| !matches!(entry, Entry::Meta { key, .. } if is_setting(key)))
                .cloned()
                .collect(),
            ..self.clone()
        }
    }
}

/// The settings of an object, a part or a height range that name a
/// filament, 0 being "default" (MixedFilamentConfigRemap.hpp
/// `remap_model_config_filament_ids`): `extruder` and the feature filaments.
const FILAMENT_KEYS: [&str; 6] = [
    "extruder",
    "wall_filament",
    "sparse_infill_filament",
    "solid_infill_filament",
    "support_filament",
    "support_interface_filament",
];

/// Entries with the filaments they name renumbered by `map` (old number
/// from 1 to new, 0 for none): a feature filament that is gone is taken
/// out, so the project's own takes effect, and an `extruder` that is gone
/// becomes 0, "default", as MixedFilamentConfigRemap.hpp does it.
fn renumbered(entries: &[Entry], map: &dyn Fn(usize) -> usize) -> Vec<Entry> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Meta { key, value } if FILAMENT_KEYS.contains(&key.as_str()) => {
                match value.trim().parse::<usize>() {
                    Ok(0) | Err(_) => Some(entry.clone()),
                    Ok(old) => match map(old) {
                        0 if key != "extruder" => None,
                        new => Some(Entry::Meta {
                            key: key.clone(),
                            value: new.to_string(),
                        }),
                    },
                }
            }
            _ => Some(entry.clone()),
        })
        .collect()
}

/// A height range's settings (`<option opt_key="extruder">2</option>`)
/// with the filaments they name renumbered as [`renumbered`] does.
fn renumbered_ranges(text: &str, map: &dyn Fn(usize) -> usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<option opt_key=\"") {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(close) = tail.find("</option>") else {
            out.push_str(tail);
            return out;
        };
        let element = &tail[..close + "</option>".len()];
        rest = &tail[element.len()..];
        let key = element["<option opt_key=\"".len()..]
            .split('"')
            .next()
            .unwrap_or_default();
        let value_at = element.find('>').map_or(0, |at| at + 1);
        let value = &element[value_at..close];
        match (FILAMENT_KEYS.contains(&key), value.trim().parse::<usize>()) {
            (true, Ok(old)) if old > 0 => match map(old) {
                0 if key != "extruder" => {}
                new => {
                    out.push_str(&element[..value_at]);
                    out.push_str(&new.to_string());
                    out.push_str("</option>");
                }
            },
            _ => out.push_str(element),
        }
    }
    out.push_str(rest);
    out
}

impl PartKept {
    /// The part with the filaments it names renumbered by `map` (old
    /// number from 1 to new, 0 for none): the feature filaments of its
    /// settings, and what its triangles are painted with, a colour that
    /// is gone left unpainted (Plater.cpp `on_filaments_delete`).
    pub fn with_filaments(&self, map: &dyn Fn(usize) -> usize) -> PartKept {
        let paint = |filament: u8| -> u8 {
            match filament {
                0 => 0,
                old => u8::try_from(map(usize::from(old))).unwrap_or(0),
            }
        };
        let extras = self.extras.as_ref().map(|extras| {
            Arc::new(
                extras
                    .iter()
                    .filter_map(|(triangle, attributes)| {
                        let rewritten = rewrite_paint(attributes, &paint)?;
                        Some((*triangle, rewritten))
                    })
                    .collect::<Extras>(),
            )
        });
        PartKept {
            entries: renumbered(&self.entries, map),
            extras,
            ..self.clone()
        }
    }
}

/// A triangle's attributes with its `paint_color` renumbered; none when
/// nothing is left on it.
fn rewrite_paint(attributes: &str, map: &dyn Fn(u8) -> u8) -> Option<Box<str>> {
    const KEY: &str = " paint_color=\"";
    let Some(start) = attributes.find(KEY) else {
        return Some(attributes.into());
    };
    let value_at = start + KEY.len();
    let Some(length) = attributes[value_at..].find('"') else {
        return Some(attributes.into());
    };
    let code = &attributes[value_at..value_at + length];
    let rest = &attributes[value_at + length + 1..];
    let out = match crate::paint::remap(code, map) {
        None => return Some(attributes.into()),
        Some(None) => format!("{}{rest}", &attributes[..start]),
        Some(Some(code)) => format!("{}{KEY}{code}\"{rest}", &attributes[..start]),
    };
    (!out.trim().is_empty()).then(|| out.into())
}

impl ObjectKept {
    /// The object with the filaments its settings and its height ranges
    /// name renumbered by `map`, as [`PartKept::with_filaments`] does.
    pub fn with_filaments(&self, map: &dyn Fn(usize) -> usize) -> ObjectKept {
        ObjectKept {
            entries: renumbered(&self.entries, map),
            layer_ranges: self
                .layer_ranges
                .as_deref()
                .map(|ranges| renumbered_ranges(ranges, map)),
            ..self.clone()
        }
    }
}

impl ObjectKept {
    /// The object's own settings as the file carries them: every key and
    /// value of its metadata but its name and filament, in the file's
    /// order, as [`PartKept::settings`] has a part's.
    pub fn settings(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Meta { key, value } if is_setting(key) => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    /// The object's height ranges (`Metadata/layer_config_ranges.xml`,
    /// read as bbs_3mf.cpp `_extract_layer_config_ranges_from_archive`
    /// reads them): each range's bottom and top in the object's own
    /// heights, and what it sets, in the file's order. A range whose
    /// heights cannot be read is left out, as the reference leaves it.
    pub fn layer_ranges(&self) -> Vec<LayerRange> {
        self.layer_ranges
            .as_deref()
            .map(parse_ranges)
            .unwrap_or_default()
    }
}

/// One height range of an object: a band of its height with settings of
/// its own (the reference's `t_layer_config_ranges`, "height range" in
/// its object list).
#[derive(Clone, Debug, PartialEq)]
pub struct LayerRange {
    /// Bottom and top, in mm, measured in the object's own heights.
    pub min_z: f64,
    pub max_z: f64,
    /// What it sets, key and value as written.
    pub settings: Vec<(String, String)>,
}

/// The ranges inside one `<object>` of `Metadata/layer_config_ranges.xml`.
fn parse_ranges(text: &str) -> Vec<LayerRange> {
    let attribute = |tag: &str, name: &str| -> Option<String> {
        let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
        let rest = &tag[at..];
        Some(rest[..rest.find('"')?].to_owned())
    };
    let unescape = |value: &str| -> String {
        quick_xml::escape::unescape(value)
            .map(|text| text.into_owned())
            .unwrap_or_else(|_| value.to_owned())
    };
    let mut ranges = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<range") {
        let tail = &rest[start..];
        let Some(head_end) = tail.find('>') else {
            break;
        };
        let head = &tail[..head_end];
        let (body, next) = if head.ends_with('/') {
            ("", &tail[head_end + 1..])
        } else {
            match tail.find("</range>") {
                Some(close) => (
                    &tail[head_end + 1..close],
                    &tail[close + "</range>".len()..],
                ),
                None => (&tail[head_end + 1..], ""),
            }
        };
        rest = next;
        let height =
            |name| attribute(head, name).and_then(|value| value.trim().parse::<f64>().ok());
        let (Some(min_z), Some(max_z)) = (height("min_z"), height("max_z")) else {
            continue;
        };
        let mut settings = Vec::new();
        let mut options = body;
        while let Some(at) = options.find("<option") {
            let element = &options[at..];
            let Some(open_end) = element.find('>') else {
                break;
            };
            let key = attribute(&element[..open_end], "opt_key").unwrap_or_default();
            let after = &element[open_end + 1..];
            let close = after.find("</option>").unwrap_or(after.len());
            if !key.is_empty() {
                settings.push((unescape(&key), unescape(after[..close].trim())));
            }
            options = &after[close..];
        }
        ranges.push(LayerRange {
            min_z,
            max_z,
            settings,
        });
    }
    ranges
}

/// Whether an entry of a part is its text's description (bbs_3mf.cpp
/// `TEXT_TAG`).
fn is_text(raw: &str) -> bool {
    raw.starts_with("<slic3rpe:text")
}

impl ObjectKept {
    /// The object with a part's settings given to it, each set over what
    /// the object had: what the reference does when one part is left of an
    /// object (GUI_ObjectList.cpp `del_subobject_from_object`,
    /// `object->config.apply(last_volume->config)`).
    pub fn with_settings_of(&self, part: &PartKept) -> ObjectKept {
        let mut object = self.clone();
        for (key, value) in part.settings() {
            match object
                .entries
                .iter_mut()
                .find(|entry| entry.key() == Some(key.as_str()))
            {
                Some(entry) => *entry = Entry::Meta { key, value },
                None => object.entries.push(Entry::Meta { key, value }),
            }
        }
        object
    }
}

/// The pictures of a plate: what each is and its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Picture {
    /// `plate_N.png`, `thumbnail_file`.
    Plate,
    /// `plate_N_small.png`.
    Small,
    /// `plate_no_light_N.png`, `thumbnail_no_light_file`.
    NoLight,
    /// `top_N.png`, `top_file`.
    Top,
    /// `pick_N.png`, `pick_file`.
    Pick,
}

impl Picture {
    pub(crate) const ALL: [Picture; 5] = [
        Picture::Plate,
        Picture::Small,
        Picture::NoLight,
        Picture::Top,
        Picture::Pick,
    ];

    pub(crate) fn file(self, plate: u32) -> String {
        match self {
            Picture::Plate => format!("Metadata/plate_{plate}.png"),
            Picture::Small => format!("Metadata/plate_{plate}_small.png"),
            Picture::NoLight => format!("Metadata/plate_no_light_{plate}.png"),
            Picture::Top => format!("Metadata/top_{plate}.png"),
            Picture::Pick => format!("Metadata/pick_{plate}.png"),
        }
    }

    /// The plate's metadata that names it, for those that have one.
    pub(crate) fn key(self) -> Option<&'static str> {
        match self {
            Picture::Plate => Some("thumbnail_file"),
            Picture::Small => None,
            Picture::NoLight => Some("thumbnail_no_light_file"),
            Picture::Top => Some("top_file"),
            Picture::Pick => Some("pick_file"),
        }
    }

    /// Which picture of which plate an entry's name is.
    fn of(name: &str) -> Option<(Picture, u32)> {
        let rest = name.strip_prefix("Metadata/")?.strip_suffix(".png")?;
        let number = |text: &str| text.parse::<u32>().ok();
        if let Some(n) = rest.strip_prefix("plate_no_light_") {
            return Some((Picture::NoLight, number(n)?));
        }
        if let Some(n) = rest.strip_prefix("top_") {
            return Some((Picture::Top, number(n)?));
        }
        if let Some(n) = rest.strip_prefix("pick_") {
            return Some((Picture::Pick, number(n)?));
        }
        let n = rest.strip_prefix("plate_")?;
        match n.strip_suffix("_small") {
            Some(n) => Some((Picture::Small, number(n)?)),
            None => Some((Picture::Plate, number(n)?)),
        }
    }
}

/// A plate's pictures, by what each is.
pub(crate) type Pictures = Vec<(Picture, Arc<Vec<u8>>)>;

/// What a plate of a file holds that [`crate::Plate`] does not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlateKept {
    /// The plate's number in the file.
    pub(crate) id: u32,
    /// Its own settings and metadata (`locked`, `bed_type`,
    /// `print_sequence`, …), in order, without those written afresh: its
    /// number and name, its pictures, its G-code, its copies.
    pub(crate) entries: Vec<Entry>,
    /// Its pictures, when they still show it.
    pub(crate) pictures: Pictures,
}

impl PlateKept {
    /// The same plate's own settings, without its pictures: for a plate
    /// that no longer looks as they show it.
    pub fn without_pictures(&self) -> Self {
        Self {
            pictures: Vec::new(),
            ..self.clone()
        }
    }

    /// Whether it has pictures of itself.
    pub fn has_pictures(&self) -> bool {
        !self.pictures.is_empty()
    }

    /// What a plate the file did not have holds: what the writer writes
    /// for one (`locked`, its first layer's order when it sets one, the
    /// filament map), so that keeping it changes nothing in the file. `id`
    /// is the plate's number.
    pub fn fresh(id: u32, first_layer_sequence: &[u32]) -> Self {
        let meta = |key: &str, value: String| Entry::Meta {
            key: key.to_owned(),
            value,
        };
        let mut entries = vec![meta("locked", "false".into())];
        if !first_layer_sequence.is_empty() {
            let order: Vec<String> = first_layer_sequence.iter().map(u32::to_string).collect();
            entries.push(meta("first_layer_print_sequence", order.join(" ")));
        }
        entries.push(meta("filament_map_mode", "Auto For Flush".into()));
        entries.push(meta("filament_maps", "1".into()));
        Self {
            id,
            entries,
            ..Self::default()
        }
    }

    /// One of the plate's own settings or metadata as the file has it
    /// (`locked`, `bed_type`, `print_sequence`, `spiral_mode`, …).
    pub fn value(&self, key: &str) -> Option<&str> {
        Entry::value_of(&self.entries, key)
    }

    /// The same plate with `key` set to `value`, or taken out for none. An
    /// entry it has already is changed where it stands; a new one goes
    /// where the reference writes it among the others
    /// (bbs_3mf.cpp `_add_model_config_file_to_archive`), so that the file
    /// reads as the reference's own.
    pub fn with(&self, key: &str, value: Option<&str>) -> Self {
        let mut entries = self.entries.clone();
        let at = entries.iter().position(|entry| entry.key() == Some(key));
        match (at, value) {
            (Some(at), Some(value)) => {
                entries[at] = Entry::Meta {
                    key: key.to_owned(),
                    value: value.to_owned(),
                }
            }
            (Some(at), None) => {
                entries.remove(at);
            }
            (None, Some(value)) => {
                let rank = |key: &str| PLATE_ORDER.iter().position(|known| *known == key);
                let mine = rank(key).unwrap_or(PLATE_ORDER.len());
                // After the last entry the reference writes before it.
                let place = entries
                    .iter()
                    .rposition(|entry| {
                        entry
                            .key()
                            .and_then(rank)
                            .is_some_and(|theirs| theirs < mine)
                    })
                    .map_or(0, |at| at + 1);
                entries.insert(
                    place,
                    Entry::Meta {
                        key: key.to_owned(),
                        value: value.to_owned(),
                    },
                );
            }
            (None, None) => {}
        }
        Self {
            entries,
            ..self.clone()
        }
    }
}

/// The order the reference writes a plate's own metadata in, after its
/// number and name (bbs_3mf.cpp `_add_model_config_file_to_archive`).
const PLATE_ORDER: [&str; 11] = [
    "locked",
    "bed_type",
    "print_sequence",
    "first_layer_print_sequence",
    "other_layers_print_sequence",
    "other_layers_print_sequence_nums",
    "spiral_mode",
    "filament_map_mode",
    "filament_maps",
    "dual_nozzle_confirm",
    "print_history",
];

/// What kind of entry of the package a file is, for writing it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Carry {
    /// Written as it was read.
    AsIs,
    /// A preset the project carries (`Metadata/process_settings_1.config`
    /// and the like): written with the project's own settings only.
    Preset,
    /// A file that goes by the objects' places in the list and that is
    /// not read here (the reference's re-editable cuts, height patterns,
    /// Showcase scenes): written as it was while every object of the file
    /// is where it was.
    ByObjectPlace,
}

/// A file of the package written back as it was read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KeptFile {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
    pub carry: Carry,
}

/// What a project file holds as a whole that [`crate::Project`] does not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    /// The model's header (`<metadata name="..">`), unescaped.
    pub(crate) header: Vec<(String, String)>,
    /// How many objects the file had.
    pub(crate) objects: usize,
    /// `Metadata/project_settings.config` as it was.
    pub(crate) settings: Option<String>,
    /// The first line of `Metadata/brim_ear_points.txt`.
    pub(crate) brim_version: Option<String>,
    /// `Metadata/custom_gcode_per_layer.xml` when it is not laid out as
    /// the reference lays it (which it then does not read), as it was.
    pub(crate) custom_gcode: Option<String>,
    /// Children of `model_settings.config`'s `<config>` that are not an
    /// object, a plate or the assembly, as written.
    pub(crate) settings_extra: Vec<String>,
    /// Entries written back as they were.
    pub(crate) files: Vec<KeptFile>,
    /// Entries that are not written back, and why: the slice that was
    /// saved with the project, and names that would reach outside it.
    pub(crate) left: Vec<(String, Left)>,
}

/// Why an entry of a file is not written back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Left {
    /// Part of a slice saved with the project (its G-code, the boxes of
    /// what it printed, its filament order): a saved project is not
    /// sliced, as the reference's "Save Project" writes it.
    Slice,
    /// A name that reaches outside the package (`../`, a drive, a
    /// leading slash), which no reader should follow.
    Unsafe,
}

impl Kept {
    /// The settings the file was saved with, as they were written.
    pub fn settings_text(&self) -> Option<&str> {
        self.settings.as_deref()
    }

    /// Whether the file carries presets of its own.
    pub fn has_presets(&self) -> bool {
        self.files.iter().any(|file| file.carry == Carry::Preset)
    }
}

/// The parts of `model_settings.config` read whole.
#[derive(Default)]
pub(crate) struct KeptSettings {
    pub objects: HashMap<u32, (Vec<Entry>, HashMap<u32, PartKept>)>,
    /// Each `<plate>`, in order.
    pub plates: Vec<PlateKept>,
    /// `<assemble_item>`s: object id, copy, transform, offset.
    pub assembly: Vec<(u32, usize, String, String)>,
    /// Each plate's copies' `identify_id`: object id, copy, number.
    pub identify: Vec<(u32, usize, u32)>,
    pub extra: Vec<String>,
}

/// The raw text of the element whose start tag began at `start`, once the
/// reader has passed that tag.
fn element_text<'t>(
    text: &'t str,
    reader: &mut Reader<&[u8]>,
    start: usize,
    empty: bool,
    name: &str,
    part: &str,
) -> Result<&'t str, Error> {
    if !empty {
        reader
            .read_to_end(quick_xml::name::QName(name))
            .map_err(|e| malformed(part, e.to_string()))?;
    }
    let end = reader.buffer_position() as usize;
    Ok(text[start..end].trim_start())
}

/// `<metadata key value/>` as an entry; anything else as written.
fn entry(
    text: &str,
    reader: &mut Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
    start: usize,
    empty: bool,
    part: &str,
) -> Result<Entry, Error> {
    let name = element.name().as_ref().to_owned();
    if element.local_name().as_ref() == "metadata" {
        let (mut key, mut value) = (None, String::new());
        attributes(element, part, |name, text| {
            match name {
                "key" => key = Some(text.to_owned()),
                "value" => value = text.to_owned(),
                _ => {}
            }
            Ok(())
        })?;
        if !empty {
            element_text(text, reader, start, empty, &name, part)?;
        }
        if let Some(key) = key {
            return Ok(Entry::Meta { key, value });
        }
    }
    let raw = element_text(text, reader, start, empty, &name, part)?;
    Ok(Entry::Raw(raw.to_owned()))
}

/// Plate metadata written afresh, not kept.
const PLATE_WRITTEN: [&str; 10] = [
    "plater_id",
    "plater_name",
    "thumbnail_file",
    "thumbnail_no_light_file",
    "top_file",
    "pick_file",
    "pattern_file",
    "pattern_bbox_file",
    "gcode_file",
    "index",
];

pub(crate) fn read_settings(text: &str, part: &str) -> Result<KeptSettings, Error> {
    let mut reader = Reader::from_str(text);
    let mut kept = KeptSettings::default();
    // Where the reader is: in `<config>`, in an object, in a part of it,
    // in a plate.
    let mut object: Option<u32> = None;
    let mut part_of: Option<u32> = None;
    let mut in_plate = false;
    let mut in_assembly = false;
    let mut depth = 0usize;
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?;
        let (element, empty) = match event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => {
                match depth {
                    3 => part_of = None,
                    2 => {
                        object = None;
                        in_plate = false;
                        in_assembly = false;
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let start = before + text[before..].find('<').unwrap_or(0);
        let local = element.local_name().as_ref().to_owned();
        let mut id = None;
        attributes(&element, part, |name, value| {
            if name == "id" {
                id = value.trim().parse::<u32>().ok();
            }
            Ok(())
        })?;
        match (depth, local.as_str()) {
            (0, _) => {
                // `<config>` itself.
                if !empty {
                    depth = 1;
                }
            }
            (1, "object") if id.is_some() && !empty => {
                object = id;
                kept.objects.entry(id.unwrap_or(0)).or_default();
                depth = 2;
            }
            (1, "plate") if !empty => {
                kept.plates.push(PlateKept::default());
                in_plate = true;
                depth = 2;
            }
            (1, "assemble") if !empty => {
                in_assembly = true;
                depth = 2;
            }
            (1, _) => {
                let name = element.name().as_ref().to_owned();
                let raw = element_text(text, &mut reader, start, empty, &name, part)?;
                kept.extra.push(raw.to_owned());
            }
            (2, "part") if object.is_some() && id.is_some() && !empty => {
                let mut subtype = None;
                attributes(&element, part, |name, value| {
                    if name == "subtype" {
                        subtype = Some(value.to_owned());
                    }
                    Ok(())
                })?;
                let (_, parts) = kept.objects.entry(object.unwrap_or(0)).or_default();
                parts.insert(
                    id.unwrap_or(0),
                    PartKept {
                        subtype,
                        ..PartKept::default()
                    },
                );
                part_of = id;
                depth = 3;
            }
            (2, _) if object.is_some() => {
                let entry = entry(text, &mut reader, &element, start, empty, part)?;
                kept.objects
                    .entry(object.unwrap_or(0))
                    .or_default()
                    .0
                    .push(entry);
            }
            (2, "model_instance") if in_plate => {
                // Written afresh from the plate's copies; its number is
                // kept.
                let name = element.name().as_ref().to_owned();
                let raw = element_text(text, &mut reader, start, empty, &name, part)?;
                let (mut object, mut copy, mut number) = (None, None, None);
                let mut inner = Reader::from_str(raw);
                loop {
                    match inner
                        .read_event()
                        .map_err(|e| malformed(part, e.to_string()))?
                    {
                        Event::Start(element) | Event::Empty(element)
                            if element.local_name().as_ref() == "metadata" =>
                        {
                            let (mut key, mut value) = (String::new(), String::new());
                            attributes(&element, part, |name, text| {
                                match name {
                                    "key" => key = text.to_owned(),
                                    "value" => value = text.to_owned(),
                                    _ => {}
                                }
                                Ok(())
                            })?;
                            let value = value.trim();
                            match key.as_str() {
                                "object_id" => object = value.parse::<u32>().ok(),
                                "instance_id" => copy = value.parse::<usize>().ok(),
                                "identify_id" => number = value.parse::<u32>().ok(),
                                _ => {}
                            }
                        }
                        Event::Eof => break,
                        _ => {}
                    }
                }
                if let (Some(object), Some(copy), Some(number)) = (object, copy, number) {
                    kept.identify.push((object, copy, number));
                }
            }
            (2, _) if in_plate => {
                let entry = entry(text, &mut reader, &element, start, empty, part)?;
                let plate = kept.plates.last_mut().expect("in a plate");
                match &entry {
                    Entry::Meta { key, value } if key == "plater_id" => {
                        plate.id = value.trim().parse().unwrap_or(0);
                    }
                    Entry::Meta { key, .. } if PLATE_WRITTEN.contains(&key.as_str()) => {}
                    _ => plate.entries.push(entry),
                }
            }
            (2, "assemble_item") if in_assembly => {
                let (mut object, mut copy, mut transform, mut offset) =
                    (None, None, String::new(), String::new());
                for attribute in element.attributes().with_checks(false) {
                    let attribute = attribute.map_err(|e| malformed(part, e.to_string()))?;
                    let value = attribute.value.to_string();
                    match attribute.key.as_ref() {
                        "object_id" => object = value.trim().parse::<u32>().ok(),
                        "instance_id" => copy = value.trim().parse::<usize>().ok(),
                        "transform" => transform = value,
                        "offset" => offset = value,
                        _ => {}
                    }
                }
                let name = element.name().as_ref().to_owned();
                element_text(text, &mut reader, start, empty, &name, part)?;
                if let (Some(object), Some(copy)) = (object, copy) {
                    kept.assembly.push((object, copy, transform, offset));
                }
            }
            (3, _) if object.is_some() && part_of.is_some() => {
                let entry = entry(text, &mut reader, &element, start, empty, part)?;
                let (_, parts) = kept.objects.entry(object.unwrap_or(0)).or_default();
                parts
                    .entry(part_of.unwrap_or(0))
                    .or_default()
                    .entries
                    .push(entry);
            }
            _ => {
                let name = element.name().as_ref().to_owned();
                element_text(text, &mut reader, start, empty, &name, part)?;
            }
        }
    }
    Ok(kept)
}

/// The lines of a file of `object_id=N|…` lines (layer heights, brim ear
/// points), by N; other lines are left.
fn lines_by_object(text: &str) -> BTreeMap<usize, String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("object_id=")?;
            let (number, values) = rest.split_once('|')?;
            Some((number.trim().parse().ok()?, values.to_owned()))
        })
        .collect()
}

/// The inside of each `<object id="N">` directly under the root of an XML
/// file, by N.
fn objects_by_id(text: &str, part: &str) -> Result<BTreeMap<usize, String>, Error> {
    let mut reader = Reader::from_str(text);
    let mut found = BTreeMap::new();
    let mut depth = 0usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?;
        match event {
            Event::Start(element) if depth == 1 && element.local_name().as_ref() == "object" => {
                let mut id = None;
                attributes(&element, part, |name, value| {
                    if name == "id" {
                        id = value.trim().parse::<usize>().ok();
                    }
                    Ok(())
                })?;
                let name = element.name().as_ref().to_owned();
                let span = reader
                    .read_to_end(quick_xml::name::QName(name.as_str()))
                    .map_err(|e| malformed(part, e.to_string()))?;
                if let Some(id) = id {
                    found.insert(id, text[span.start as usize..span.end as usize].to_owned());
                }
            }
            Event::Empty(element) if depth == 1 && element.local_name().as_ref() == "object" => {
                let mut id = None;
                attributes(&element, part, |name, value| {
                    if name == "id" {
                        id = value.trim().parse::<usize>().ok();
                    }
                    Ok(())
                })?;
                if let Some(id) = id {
                    found.insert(id, String::new());
                }
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(found)
}

/// The files that go by an object's place in the list, read by place.
#[derive(Default)]
pub(crate) struct ByPlace {
    pub layer_heights: BTreeMap<usize, String>,
    pub layer_ranges: BTreeMap<usize, String>,
    pub brim_points: BTreeMap<usize, String>,
    pub brim_version: Option<String>,
    pub cut: BTreeMap<usize, String>,
}

pub(crate) const LAYER_HEIGHTS: &str = "Metadata/layer_heights_profile.txt";
pub(crate) const LAYER_RANGES: &str = "Metadata/layer_config_ranges.xml";
pub(crate) const BRIM_POINTS: &str = "Metadata/brim_ear_points.txt";
pub(crate) const CUT_INFORMATION: &str = "Metadata/cut_information.xml";
pub(crate) const CUSTOM_GCODE: &str = "Metadata/custom_gcode_per_layer.xml";
pub(crate) const SLICE_INFO: &str = "Metadata/slice_info.config";

impl ByPlace {
    pub fn read(file: &str, text: &str, into: &mut ByPlace) -> Result<(), Error> {
        match file {
            LAYER_HEIGHTS => into.layer_heights = lines_by_object(text),
            BRIM_POINTS => {
                into.brim_version = text
                    .lines()
                    .next()
                    .filter(|line| line.starts_with("brim_points_format_version="))
                    .map(str::to_owned);
                into.brim_points = lines_by_object(text);
            }
            LAYER_RANGES => into.layer_ranges = objects_by_id(text, file)?,
            CUT_INFORMATION => into.cut = objects_by_id(text, file)?,
            _ => {}
        }
        Ok(())
    }
}

/// What becomes of an entry of a package that is not one of the parts
/// [`crate::write`] writes afresh.
pub(crate) enum Role {
    /// Read here and written afresh.
    Written,
    Picture(Picture, u32),
    Carried(Carry),
    Left(Left),
}

pub(crate) fn role(name: &str, models: &[String]) -> Role {
    let lower = name.to_ascii_lowercase();
    // A name that would reach outside the package where it is unpacked.
    let parts: Vec<&str> = name.split(['/', '\\']).collect();
    if name.starts_with('/')
        || name.contains('\\')
        || name.contains(':')
        || parts.iter().any(|part| *part == ".." || *part == ".")
    {
        return Role::Left(Left::Unsafe);
    }
    // Every model part and its relationships are written afresh, the
    // files of objects that are no longer there with them.
    if models.iter().any(|model| model.eq_ignore_ascii_case(name))
        || (lower.starts_with("3d/") && (lower.ends_with(".model") || lower.ends_with(".rels")))
    {
        return Role::Written;
    }
    const WRITTEN: [&str; 11] = [
        "[content_types].xml",
        "_rels/.rels",
        "3d/3dmodel.model",
        "3d/_rels/3dmodel.model.rels",
        "metadata/model_settings.config",
        "metadata/project_settings.config",
        "metadata/slice_info.config",
        "metadata/layer_heights_profile.txt",
        "metadata/layer_config_ranges.xml",
        "metadata/brim_ear_points.txt",
        "metadata/cut_information.xml",
    ];
    if WRITTEN.contains(&lower.as_str()) || lower == "metadata/custom_gcode_per_layer.xml" {
        return Role::Written;
    }
    if let Some((picture, plate)) = Picture::of(name) {
        return Role::Picture(picture, plate);
    }
    // The slice saved with a project (bbs_3mf.cpp `_add_gcode_file_to_archive`,
    // `_add_bbox_file_to_archive`, `_add_filament_sequence_file_to_archive`).
    let slice = (lower.starts_with("metadata/plate_")
        && (lower.ends_with(".gcode")
            || lower.ends_with(".gcode.md5")
            || lower.ends_with(".json")))
        || lower == "metadata/filament_sequence.json"
        || lower == "metadata/_rels/model_settings.config.rels";
    if slice {
        return Role::Left(Left::Slice);
    }
    for preset in [
        "metadata/print_setting_",
        "metadata/process_settings_",
        "metadata/filament_settings_",
        "metadata/machine_settings_",
    ] {
        if lower.starts_with(preset) && lower.ends_with(".config") {
            return Role::Carried(Carry::Preset);
        }
    }
    for by_place in [
        "metadata/cut_recipe.xml",
        "metadata/cut_recipe/",
        "metadata/edgeslicer_height_patterns.json",
        "metadata/showcase/",
    ] {
        if lower.starts_with(by_place) {
            return Role::Carried(Carry::ByObjectPlace);
        }
    }
    Role::Carried(Carry::AsIs)
}

/// The model's header: each `<metadata name="..">value</metadata>` before
/// its resources, unescaped.
pub(crate) fn read_header(text: &str, part: &str) -> Result<Vec<(String, String)>, Error> {
    let mut reader = Reader::from_str(text);
    let mut header = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| malformed(part, e.to_string()))?
        {
            Event::Start(element) if element.local_name().as_ref() == "metadata" => {
                let mut name = None;
                attributes(&element, part, |key, value| {
                    if key == "name" {
                        name = Some(value.to_owned());
                    }
                    Ok(())
                })?;
                let tag = element.name().as_ref().to_owned();
                let span = reader
                    .read_to_end(quick_xml::name::QName(tag.as_str()))
                    .map_err(|e| malformed(part, e.to_string()))?;
                let raw = &text[span.start as usize..span.end as usize];
                let value = quick_xml::escape::unescape(raw)
                    .map_err(|e| malformed(part, e.to_string()))?
                    .into_owned();
                if let Some(name) = name {
                    header.push((name, value));
                }
            }
            Event::Empty(element) if element.local_name().as_ref() == "metadata" => {
                let mut name = None;
                attributes(&element, part, |key, value| {
                    if key == "name" {
                        name = Some(value.to_owned());
                    }
                    Ok(())
                })?;
                if let Some(name) = name {
                    header.push((name, String::new()));
                }
            }
            Event::Start(element) | Event::Empty(element)
                if matches!(element.local_name().as_ref(), "resources" | "build") =>
            {
                break;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(header)
}
