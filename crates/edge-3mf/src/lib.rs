//! Reads 3MF projects: core 3MF geometry, plus the Bambu/Orca additions the
//! reference slicer writes (parts split across model files, part roles,
//! plates, and the project's settings).
//!
//! Colours painted onto triangles are read ([`Project::painted`]).
//!
//! Not read yet: painted supports and seams, per-object setting overrides,
//! and everything about a plate except which instances are on it.

mod custom_gcode;
mod kept;
mod model;
mod model_settings;
mod own;
mod package;
mod paint;
mod write;
mod xml;

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, Read, Seek},
    path::Path,
    sync::Arc,
};

use edge_config::Settings;
use edge_mesh::{Mesh, Transform};
use rayon::prelude::*;

pub use custom_gcode::{CustomGcode, CustomItem, CustomKind, CustomMode};
pub use kept::{Entry, Extras, Kept, LayerRange, Left, ObjectKept, PartKept, PlateKept};
use model::{ModelFile, RawObject, Shape};
use model_settings::{ModelSettings, ObjectSettings};
use package::Package;
pub use paint::{Facet, SeamFacet, SeamPaint};
pub use write::{
    REFERENCE_VERSION, SECRETS, Save, Written, losses, own_settings_text, settings_text, write,
    write_to,
};
use xml::malformed;

const DEFAULT_MODEL_PART: &str = "3D/3dmodel.model";
const MODEL_SETTINGS_PART: &str = "Metadata/model_settings.config";
const PROJECT_SETTINGS_PART: &str = "Metadata/project_settings.config";
/// Components may nest; this bounds the recursion and catches cycles.
const MAX_COMPONENT_DEPTH: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read 3MF: {0}")]
    Io(#[from] io::Error),
    #[error("not a 3MF package: {0}")]
    Archive(String),
    #[error("3MF has no {0}")]
    Missing(String),
    #[error("3MF part {part} is malformed: {reason}")]
    Malformed { part: String, reason: String },
}

#[derive(Clone, Debug)]
pub struct Project {
    pub objects: Vec<Object>,
    /// Empty for 3MF files written by tools that have no notion of plates.
    pub plates: Vec<Plate>,
    /// The full settings the project was saved with, when it carries them.
    pub settings: Option<Settings>,
    /// What is painted on the project's models other than colour, seams
    /// and supports (fuzzy skin), in order of name. None of it is read.
    pub unread_paint: Vec<&'static str>,
    /// The G-code set at layers (pauses, custom G-code, filament
    /// changes), by plate number, as the reference keeps them
    /// (`Model::plates_custom_gcodes`): a plate may have an entry with no
    /// items, which is written as one. A project without plates has its
    /// items under plate 1, as the reference reads them.
    pub custom_gcode: std::collections::BTreeMap<u32, CustomGcode>,
    /// What the file holds beyond all this, kept to be written back; none
    /// for a project that was not read from a file.
    pub kept: Option<Arc<Kept>>,
}

#[derive(Clone, Debug)]
pub struct Object {
    /// The object's id in the 3MF, which plates and settings refer to.
    pub id: u32,
    pub name: Option<String>,
    /// The filament the object itself was given (its `extruder`), counting
    /// from 1, when it was: its parts without one of their own print in
    /// it, and in filament 1 when it has none.
    pub filament: Option<usize>,
    pub parts: Vec<Part>,
    pub instances: Vec<Instance>,
    /// What the file holds of it beyond this, kept to be written back.
    pub kept: Option<Arc<ObjectKept>>,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub name: Option<String>,
    pub kind: PartKind,
    /// Which filament the part is printed in, counting from 1: its own
    /// choice, or else its object's, or else the first.
    pub filament: usize,
    /// The filament the part was given itself (its own `extruder`), when
    /// it was; without one it follows its object (the reference's
    /// "default"), and `filament` is the object's.
    pub own_filament: Option<usize>,
    /// Shared between parts and objects that reuse the same geometry.
    pub mesh: Arc<Mesh>,
    /// The mesh's triangles as painted for filaments, cut up where the
    /// paint cuts them, when any of them is painted.
    pub paint: Option<Arc<Vec<Facet>>>,
    /// The pieces of the mesh's triangles painted to have seams or to
    /// keep them off, when any are.
    pub seam: Option<Arc<Vec<SeamFacet>>>,
    /// Places the mesh in its object's coordinates.
    pub transform: Transform,
    /// Whether the part is embossed text or an embossed picture, which
    /// decorates a surface rather than being a piece of the object.
    pub decoration: bool,
    /// What the file holds of it beyond this (its settings, what its
    /// triangles carry, what it was read as), kept to be written back:
    /// see [`Part::settings`].
    pub kept: Option<Arc<PartKept>>,
}

impl Object {
    /// The object's own settings as its file carries them
    /// ([`ObjectKept::settings`]); none for an object no file held.
    pub fn settings(&self) -> Vec<(String, String)> {
        self.kept
            .as_deref()
            .map(ObjectKept::settings)
            .unwrap_or_default()
    }

    /// The object's height ranges ([`ObjectKept::layer_ranges`]).
    pub fn layer_ranges(&self) -> Vec<LayerRange> {
        self.kept
            .as_deref()
            .map(ObjectKept::layer_ranges)
            .unwrap_or_default()
    }
}

impl Part {
    /// The part placed where `instance` puts its object, in world
    /// coordinates.
    pub fn placed(&self, instance: &Instance) -> Mesh {
        let mut mesh = Mesh::default();
        mesh.append(&self.mesh, &self.transform.then(&instance.transform));
        mesh
    }

    /// The pieces of the part's triangles painted to have supports (an
    /// enforcer) or to keep them off (a blocker), in the part's own
    /// coordinates: its triangles' `paint_supports`, as the reference
    /// keeps them in `supported_facets`, coded as seams are.
    pub fn support_paint(&self) -> Vec<SeamFacet> {
        let mut facets = Vec::new();
        let Some(extras) = self.kept.as_deref().and_then(PartKept::extras) else {
            return facets;
        };
        const KEY: &str = " paint_supports=\"";
        for (triangle, attributes) in extras.iter() {
            let Some(at) = attributes.find(KEY) else {
                continue;
            };
            let rest = &attributes[at + KEY.len()..];
            let code = &rest[..rest.find('"').unwrap_or(rest.len())];
            if (*triangle as usize) < self.mesh.triangles.len() && !code.is_empty() {
                paint::decode_seam(code, self.mesh.triangle(*triangle as usize), &mut facets);
            }
        }
        facets
    }

    /// The part's own settings as its file carries them, in the file's
    /// order ([`PartKept::settings`]); none for a part no file held.
    pub fn settings(&self) -> Vec<(String, String)> {
        self.kept
            .as_deref()
            .map(PartKept::settings)
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartKind {
    /// Printed material.
    #[default]
    Normal,
    /// Volume subtracted from the object's normal parts.
    Negative,
    /// Region where different settings apply.
    Modifier,
    SupportBlocker,
    SupportEnforcer,
    /// Where the reference's Precise Seam puts a seam, or keeps one in or
    /// out (GCode/PreciseSeam.cpp; Model.cpp `type_from_string`,
    /// `precise_seam_center` to `precise_seam_neutral`): helper geometry,
    /// not printed. Not followed here: the seams near it are placed as
    /// any other, and a project with one says so.
    PreciseSeam,
}

impl PartKind {
    /// Unknown subtypes read as normal parts, as the reference does.
    fn from_subtype(subtype: &str) -> Self {
        match subtype {
            "negative_part" => Self::Negative,
            "modifier_part" => Self::Modifier,
            "support_blocker" => Self::SupportBlocker,
            "support_enforcer" => Self::SupportEnforcer,
            _ if is_precise_seam(subtype) => Self::PreciseSeam,
            _ => Self::Normal,
        }
    }
}

/// Whether a part type, as the reference writes it, is one of Precise
/// Seam's.
pub(crate) fn is_precise_seam(kind: &str) -> bool {
    matches!(
        kind,
        "precise_seam_center"
            | "precise_seam_left"
            | "precise_seam_right"
            | "precise_seam_enforced"
            | "precise_seam_blocked"
            | "precise_seam_neutral"
    )
}

#[derive(Clone, Copy, Debug)]
pub struct Instance {
    /// Places the object in the project's world coordinates.
    pub transform: Transform,
    pub printable: bool,
}

#[derive(Clone, Debug)]
pub struct Plate {
    /// The plate's number as the reference shows it, starting at 1.
    pub id: u32,
    pub name: String,
    /// The order the plate sets for filaments on the first layer, as
    /// filament numbers from 1 (`first_layer_print_sequence`); empty when
    /// it sets none.
    pub first_layer_sequence: Vec<u32>,
    pub instances: Vec<InstanceRef>,
    /// What the file holds of it beyond this, kept to be written back.
    pub kept: Option<Arc<PlateKept>>,
}

/// Indices into `Project::objects` and that object's `instances`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceRef {
    pub object: usize,
    pub instance: usize,
}

impl Project {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::read(BufReader::new(File::open(path)?))
    }

    pub fn read(reader: impl Read + Seek) -> Result<Self, Error> {
        let mut package = Package::open(reader)?;
        let root = root_model_part(&mut package)?;
        let files = load_models(&mut package, &root)?;
        let settings_text = package.read(MODEL_SETTINGS_PART)?;
        let (model_settings, mut kept_settings) = match &settings_text {
            Some(text) => (
                model_settings::parse(text, MODEL_SETTINGS_PART)?,
                kept::read_settings(text, MODEL_SETTINGS_PART)?,
            ),
            None => (ModelSettings::default(), kept::KeptSettings::default()),
        };
        let project_text = package.read(PROJECT_SETTINGS_PART)?;
        let settings = project_text
            .as_deref()
            .map(|text| {
                Settings::from_json(text).map_err(|reason| malformed(PROJECT_SETTINGS_PART, reason))
            })
            .transpose()?;
        let header = match package.read(&root)? {
            Some(text) => kept::read_header(&text, &root)?,
            None => Vec::new(),
        };

        // Build items that place the same object are that object's instances.
        let mut objects: Vec<Object> = Vec::new();
        let mut slots: HashMap<(&str, u32), usize> = HashMap::new();
        for item in &files[&root].items {
            let file = item.path.as_deref().map_or(root.as_str(), part_key);
            let slot = match slots.get(&(file, item.object)) {
                Some(&slot) => slot,
                None => {
                    let raw = find(&files, file, item.object)?;
                    let object_settings = model_settings.objects.get(&item.object);
                    let (entries, kept_parts) = kept_settings
                        .objects
                        .remove(&item.object)
                        .unwrap_or_default();
                    objects.push(Object {
                        id: item.object,
                        name: object_settings
                            .and_then(|s| s.name.clone())
                            .or_else(|| raw.name.clone()),
                        filament: object_settings
                            .and_then(|s| s.extruder)
                            .filter(|extruder| *extruder > 0)
                            .map(|extruder| extruder as usize),
                        parts: parts(&files, file, raw, object_settings, &kept_parts)?,
                        instances: Vec::new(),
                        kept: Some(Arc::new(ObjectKept {
                            index: objects.len() + 1,
                            entries,
                            ..ObjectKept::default()
                        })),
                    });
                    slots.insert((file, item.object), objects.len() - 1);
                    objects.len() - 1
                }
            };
            objects[slot].instances.push(Instance {
                transform: item.transform,
                printable: item.printable,
            });
        }

        // The rest of the package: what goes by an object's place or a
        // plate's number, the pictures, and every other entry.
        let models: Vec<String> = files.keys().cloned().collect();
        let mut by_place = kept::ByPlace::default();
        let mut pictures: HashMap<u32, kept::Pictures> = HashMap::new();
        let mut kept_files = Vec::new();
        let mut left = Vec::new();
        let mut custom_gcode = None;
        for name in package.names() {
            if name.ends_with('/') {
                continue;
            }
            match kept::role(&name, &models) {
                kept::Role::Written => {
                    let lower = name.to_ascii_lowercase();
                    let canonical = [
                        kept::LAYER_HEIGHTS,
                        kept::LAYER_RANGES,
                        kept::BRIM_POINTS,
                        kept::CUT_INFORMATION,
                        kept::CUSTOM_GCODE,
                    ]
                    .into_iter()
                    .find(|known| known.eq_ignore_ascii_case(&lower));
                    if let Some(known) = canonical
                        && let Some(text) = package.read(&name)?
                    {
                        if known == kept::CUSTOM_GCODE {
                            custom_gcode = Some(text);
                        } else {
                            kept::ByPlace::read(known, &text, &mut by_place)?;
                        }
                    }
                }
                kept::Role::Picture(picture, plate) => {
                    if let Some(bytes) = package.bytes(&name)? {
                        pictures
                            .entry(plate)
                            .or_default()
                            .push((picture, Arc::new(bytes)));
                    }
                }
                kept::Role::Carried(carry) => {
                    if let Some(bytes) = package.bytes(&name)? {
                        kept_files.push(kept::KeptFile {
                            name,
                            bytes: Arc::new(bytes),
                            carry,
                        });
                    }
                }
                kept::Role::Left(why) => left.push((name, why)),
            }
        }
        let gcode_by_plate = match &custom_gcode {
            Some(text) => custom_gcode::read(text, kept::CUSTOM_GCODE)?,
            None => None,
        };
        for (at, object) in objects.iter_mut().enumerate() {
            let index = at + 1;
            let mut assembly = vec![None; object.instances.len()];
            for (id, copy, transform, offset) in &kept_settings.assembly {
                if *id == object.id && *copy < assembly.len() {
                    assembly[*copy] = Some((transform.clone(), offset.clone()));
                }
            }
            let mut identify = vec![None; object.instances.len()];
            for (id, copy, number) in &kept_settings.identify {
                if *id == object.id && *copy < identify.len() {
                    identify[*copy] = Some(*number);
                }
            }
            if let Some(kept) = &mut object.kept {
                let kept = Arc::make_mut(kept);
                kept.copies = object.instances.len();
                kept.assembly = assembly;
                kept.identify = identify;
                kept.layer_heights = by_place.layer_heights.remove(&index);
                kept.layer_ranges = by_place.layer_ranges.remove(&index);
                kept.brim_points = by_place.brim_points.remove(&index);
                kept.cut = by_place.cut.remove(&index);
            }
        }
        let mut kept_plates = std::mem::take(&mut kept_settings.plates).into_iter();

        let plates = model_settings
            .plates
            .into_iter()
            .map(|plate| Plate {
                kept: kept_plates.next().map(|mut kept| {
                    kept.id = plate.id;
                    let mut own = pictures.remove(&plate.id).unwrap_or_default();
                    own.sort_by_key(|(picture, _)| *picture);
                    kept.pictures = own;
                    Arc::new(kept)
                }),
                id: plate.id,
                name: plate.name,
                first_layer_sequence: plate.first_layer_sequence,
                // Entries naming objects or instances the file does not
                // contain are dropped, as the reference drops them.
                instances: plate
                    .instances
                    .into_iter()
                    .filter_map(|(id, instance)| {
                        let object = objects.iter().position(|object| object.id == id)?;
                        (instance < objects[object].instances.len())
                            .then_some(InstanceRef { object, instance })
                    })
                    .collect(),
            })
            .collect();

        let unread_paint: std::collections::BTreeSet<&'static str> = files
            .values()
            .flat_map(|file| file.unread_paint.iter().copied())
            .collect();
        let kept = Kept {
            header,
            objects: objects.len(),
            settings: project_text,
            brim_version: by_place.brim_version,
            // A file of custom G-code not laid out as the reference lays
            // it is kept whole.
            custom_gcode: custom_gcode.filter(|_| gcode_by_plate.is_none()),
            settings_extra: kept_settings.extra,
            files: kept_files,
            left,
        };
        Ok(Self {
            objects,
            plates,
            settings,
            unread_paint: unread_paint.into_iter().collect(),
            custom_gcode: gcode_by_plate.unwrap_or_default(),
            kept: Some(Arc::new(kept)),
        })
    }

    /// Position in `plates` of the plate numbered `id`.
    pub fn plate_index(&self, id: u32) -> Option<usize> {
        self.plates.iter().position(|plate| plate.id == id)
    }

    /// The printable normal parts of one plate, or of the whole project for
    /// `None`, merged into one mesh in world coordinates. Parts stay separate
    /// shells; overlapping ones are not united.
    pub fn mesh(&self, plate: Option<usize>) -> Mesh {
        let mut mesh = Mesh::default();
        for (object, instance) in self.printable(plate) {
            append_parts(&mut mesh, object, instance, PartKind::Normal, None);
        }
        mesh
    }

    /// What to slice for one plate (or the whole project for `None`), in
    /// world coordinates: one body for each filament. Objects without
    /// negative parts share a filament's body; each instance of an object
    /// that has them gets bodies of its own, so its negative parts carve it
    /// and nothing else. Bodies come in ascending order of filament.
    pub fn bodies(&self, plate: Option<usize>) -> Vec<Body> {
        self.bodies_where(plate, |_| true)
    }

    /// [`Project::bodies`] of the objects `keep` keeps alone.
    pub fn bodies_where(&self, plate: Option<usize>, keep: impl Fn(&Object) -> bool) -> Vec<Body> {
        let mut shared: std::collections::BTreeMap<usize, Mesh> = Default::default();
        let mut carved = Vec::new();
        for (object, instance) in self.printable(plate).filter(|(object, _)| keep(object)) {
            let has_negative = object
                .parts
                .iter()
                .any(|part| part.kind == PartKind::Negative);
            for filament in filaments(object) {
                if has_negative {
                    let mut body = Body {
                        solid: Mesh::default(),
                        negative: Some(Mesh::default()),
                        filament,
                        part: None,
                    };
                    append_parts(
                        &mut body.solid,
                        object,
                        instance,
                        PartKind::Normal,
                        Some(filament),
                    );
                    append_parts(
                        body.negative.as_mut().expect("set above"),
                        object,
                        instance,
                        PartKind::Negative,
                        None,
                    );
                    carved.push(body);
                } else {
                    append_parts(
                        shared.entry(filament).or_default(),
                        object,
                        instance,
                        PartKind::Normal,
                        Some(filament),
                    );
                }
            }
        }
        let mut bodies: Vec<Body> = shared
            .into_iter()
            .filter(|(_, solid)| !solid.triangles.is_empty())
            .map(|(filament, solid)| Body {
                solid,
                negative: None,
                filament,
                part: None,
            })
            .collect();
        bodies.extend(carved);
        bodies.sort_by_key(|body| body.filament);
        bodies
    }

    /// The printable instances on a plate (or in the whole project for
    /// `None`) that have colours painted on them.
    pub fn painted(&self, plate: Option<usize>) -> Vec<Painted> {
        self.printable(plate)
            .filter(|(object, _)| {
                object
                    .parts
                    .iter()
                    .any(|part| part.kind == PartKind::Normal && part.paint.is_some())
            })
            .map(|(object, instance)| {
                let mut solid = Mesh::default();
                append_parts(&mut solid, object, instance, PartKind::Normal, None);
                let mut negative = Mesh::default();
                append_parts(&mut negative, object, instance, PartKind::Negative, None);
                let mut facets = Vec::new();
                for part in object
                    .parts
                    .iter()
                    .filter(|part| part.kind == PartKind::Normal)
                {
                    let placed = part.transform.then(&instance.transform);
                    let mirrored = placed.is_mirrored();
                    let mut place = |corners: edge_mesh::Triangle, filament: u8| {
                        let [a, b, c] = corners
                            .map(|corner| placed.apply(corner.map(f64::from)).map(|c| c as f32));
                        facets.push(Facet {
                            // Mirrored, a triangle faces the other way round.
                            corners: if mirrored { [a, c, b] } else { [a, b, c] },
                            filament,
                        });
                    };
                    match &part.paint {
                        Some(paint) => {
                            for facet in paint.iter() {
                                place(facet.corners, facet.filament);
                            }
                        }
                        None => {
                            for index in 0..part.mesh.triangles.len() {
                                place(part.mesh.triangle(index), 0);
                            }
                        }
                    }
                }
                Painted {
                    solid,
                    negative: (!negative.triangles.is_empty()).then_some(negative),
                    facets,
                }
            })
            .collect()
    }

    /// Each printable instance on a plate (or in the whole project for
    /// `None`) as a mesh of its own, in world coordinates and in the order
    /// the plate lists them. Negative parts are left out. For what depends
    /// on which object is which, such as how wide a brim each one needs.
    pub fn instance_meshes(&self, plate: Option<usize>) -> Vec<Mesh> {
        self.printable(plate)
            .map(|(object, instance)| {
                let mut mesh = Mesh::default();
                append_parts(&mut mesh, object, instance, PartKind::Normal, None);
                mesh
            })
            .filter(|mesh| !mesh.triangles.is_empty())
            .collect()
    }

    /// The volumes cut out of each of those instances (its negative
    /// parts), in the same order and coordinates: an empty mesh for an
    /// instance with none. For what goes by an object's whole surface,
    /// such as how visible each place on it is.
    pub fn instance_negatives(&self, plate: Option<usize>) -> Vec<Mesh> {
        self.printable(plate)
            .filter_map(|(object, instance)| {
                let mut solid = Mesh::default();
                append_parts(&mut solid, object, instance, PartKind::Normal, None);
                let mut negative = Mesh::default();
                append_parts(&mut negative, object, instance, PartKind::Negative, None);
                (!solid.triangles.is_empty()).then_some(negative)
            })
            .collect()
    }

    /// The printed parts of each of those instances, each part a mesh of
    /// its own, in the same order and coordinates: for where parts meet,
    /// which seams are moved to. With each, whether its seams may also be
    /// moved to where other objects stand against it.
    pub fn instance_parts(&self, plate: Option<usize>) -> Vec<InstanceParts> {
        let printable: Vec<(&Object, &Instance)> = self.printable(plate).collect();
        printable
            .iter()
            // As `instance_meshes` leaves them out: those with nothing
            // printed.
            .filter(|(object, _)| {
                (object.parts.iter())
                    .any(|part| part.kind == PartKind::Normal && !part.mesh.triangles.is_empty())
            })
            .map(|&(object, instance)| {
                let parts = (object.parts.iter())
                    .filter(|part| part.kind == PartKind::Normal && !part.decoration)
                    .filter(|part| !part.mesh.triangles.is_empty())
                    .map(|part| {
                        let mut mesh = Mesh::default();
                        mesh.append(&part.mesh, &part.transform.then(&instance.transform));
                        mesh
                    })
                    .collect();
                // The reference's seams are an object's, shared by all
                // its copies, so other objects count only for an object
                // with one copy on the plate; and only for one whose
                // layers are its own, not those of an object the same in
                // every respect, which the reference slices once and
                // lays twice (Print.cpp `is_print_object_the_same`,
                // SeamPlacer.cpp `gather_part_joints`).
                let copies = (printable.iter())
                    .filter(|(other, _)| std::ptr::eq(*other, object))
                    .count();
                let twin = (printable.iter()).any(|(other, copy)| {
                    !std::ptr::eq(*other, object) && same(object, instance, other, copy)
                });
                // Seams painted on any part, whatever it is
                // (`gather_enforcers_blockers`).
                let (mut enforcers, mut blockers) = (Vec::new(), Vec::new());
                for part in &object.parts {
                    let Some(seam) = &part.seam else { continue };
                    let placed = part.transform.then(&instance.transform);
                    for facet in seam.iter() {
                        let corners = facet
                            .corners
                            .map(|corner| placed.apply(corner.map(f64::from)).map(|c| c as f32));
                        match facet.kind {
                            SeamPaint::Enforcer => enforcers.push(corners),
                            SeamPaint::Blocker => blockers.push(corners),
                        }
                    }
                }
                InstanceParts {
                    parts,
                    alone: copies == 1 && !twin,
                    enforcers,
                    blockers,
                }
            })
            .collect()
    }

    /// Whether anything on a plate (or in the whole project for `None`)
    /// has a Precise Seam part, which is not followed.
    pub fn has_precise_seams(&self, plate: Option<usize>) -> bool {
        self.has_parts_of(plate, PartKind::PreciseSeam)
    }

    /// Whether anything printable on a plate (or in the whole project for
    /// `None`) has a part of this kind.
    pub fn has_parts_of(&self, plate: Option<usize>, kind: PartKind) -> bool {
        self.printable(plate)
            .any(|(object, _)| (object.parts.iter()).any(|part| part.kind == kind))
    }

    /// The printable instances on a plate (or in the whole project for
    /// `None`), each with its object, in the plate's order.
    pub fn printed(&self, plate: Option<usize>) -> Vec<(&Object, &Instance)> {
        self.printable(plate).collect()
    }

    /// The printable instances on a plate, each with its object.
    fn printable(&self, plate: Option<usize>) -> impl Iterator<Item = (&Object, &Instance)> {
        let selected: Vec<InstanceRef> = match plate {
            Some(index) => self.plates[index].instances.clone(),
            None => self
                .objects
                .iter()
                .enumerate()
                .flat_map(|(object, o)| {
                    (0..o.instances.len()).map(move |instance| InstanceRef { object, instance })
                })
                .collect(),
        };
        selected
            .into_iter()
            .map(|InstanceRef { object, instance }| {
                (
                    &self.objects[object],
                    &self.objects[object].instances[instance],
                )
            })
            .filter(|(_, instance)| instance.printable)
    }
}

/// One instance of an object with paint on it, in world coordinates.
#[derive(Clone, Debug)]
pub struct Painted {
    /// All of its printed material, whatever the filament.
    pub solid: Mesh,
    /// What its negative parts take away, when it has any.
    pub negative: Option<Mesh>,
    /// Every triangle of its printed parts with the filament it is
    /// painted for, 0 where it is not painted.
    pub facets: Vec<Facet>,
}

/// One instance's printed parts, each by itself.
#[derive(Clone, Debug)]
pub struct InstanceParts {
    /// Each normal part but embossed text and pictures, in world
    /// coordinates.
    pub parts: Vec<Mesh>,
    /// Whether the instance is the only copy of its object on the plate
    /// and no other object is the same as it.
    pub alone: bool,
    /// The pieces of any of its parts painted to have seams, and to keep
    /// them off, in world coordinates.
    pub enforcers: Vec<edge_mesh::Triangle>,
    pub blockers: Vec<edge_mesh::Triangle>,
}

/// Whether two objects are the same in all the reference asks before it
/// slices one for both (Print.cpp `is_print_object_the_same`): placed
/// alike but for where they stand across the bed, the same parts, each
/// the same mesh, placed alike, in the same filament and painted alike.
/// The settings of each, which it also asks, are not read here.
fn same(a: &Object, a_copy: &Instance, b: &Object, b_copy: &Instance) -> bool {
    let (ma, mb) = (&a_copy.transform.rows, &b_copy.transform.rows);
    // All but the move across the bed.
    let alike = ma[..3] == mb[..3] && ma[3][2] == mb[3][2];
    alike
        && a.parts.len() == b.parts.len()
        && a.parts.iter().zip(&b.parts).all(|(a, b)| {
            a.kind == b.kind
                && Arc::ptr_eq(&a.mesh, &b.mesh)
                && a.transform == b.transform
                && a.filament == b.filament
                && match (&a.paint, &b.paint) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
                && match (&a.seam, &b.seam) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                }
        })
}

/// Printed material and, for objects that have negative parts, the volume
/// to remove from it.
#[derive(Clone, Debug)]
pub struct Body {
    pub solid: Mesh,
    pub negative: Option<Mesh>,
    /// Which filament the solid is printed in, counting from 1.
    pub filament: usize,
    /// A tag of whoever made the body from one part by itself, to find
    /// the part again by; none for every body [`Project::bodies`] makes.
    pub part: Option<(usize, usize)>,
}

/// Adds an object's parts of one kind to a mesh; with `filament`, only
/// those printed in it.
fn append_parts(
    mesh: &mut Mesh,
    object: &Object,
    instance: &Instance,
    kind: PartKind,
    filament: Option<usize>,
) {
    let wanted = object
        .parts
        .iter()
        .filter(|part| part.kind == kind && filament.is_none_or(|wanted| part.filament == wanted));
    for part in wanted {
        mesh.append(&part.mesh, &part.transform.then(&instance.transform));
    }
}

/// The filaments an object's printed parts use, in ascending order.
fn filaments(object: &Object) -> Vec<usize> {
    let mut used: Vec<usize> = object
        .parts
        .iter()
        .filter(|part| part.kind == PartKind::Normal)
        .map(|part| part.filament)
        .collect();
    used.sort_unstable();
    used.dedup();
    used
}

/// Part names are stored without the leading slash references carry.
fn part_key(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// The root model part named by the package relationships, or the
/// conventional one when the relationships part is absent or silent.
fn root_model_part<R: Read + Seek>(package: &mut Package<R>) -> Result<String, Error> {
    const RELATIONSHIPS: &str = "_rels/.rels";
    let mut root = None;
    if let Some(text) = package.read(RELATIONSHIPS)? {
        let mut reader = quick_xml::Reader::from_str(&text);
        loop {
            use quick_xml::events::Event;
            match reader
                .read_event()
                .map_err(|e| malformed(RELATIONSHIPS, e.to_string()))?
            {
                Event::Start(element) | Event::Empty(element)
                    if element.local_name().as_ref() == "Relationship" && root.is_none() =>
                {
                    let (mut target, mut is_model) = (None, false);
                    xml::attributes(&element, RELATIONSHIPS, |key, value| {
                        match key {
                            "Target" => target = Some(part_key(value).to_owned()),
                            "Type" => is_model = value.ends_with("/3dmodel"),
                            _ => {}
                        }
                        Ok(())
                    })?;
                    if is_model {
                        root = target;
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(root.unwrap_or_else(|| DEFAULT_MODEL_PART.to_owned()))
}

/// Parses the root model and every model part reachable from it.
fn load_models<R: Read + Seek>(
    package: &mut Package<R>,
    root: &str,
) -> Result<HashMap<String, ModelFile>, Error> {
    let mut files: HashMap<String, ModelFile> = HashMap::new();
    let mut wanted = vec![root.to_owned()];
    while !wanted.is_empty() {
        // The archive is read one part at a time; parsing, which is where
        // the time goes, then runs across cores.
        let texts = wanted
            .drain(..)
            .map(|part| match package.read(&part)? {
                Some(text) => Ok((part, text)),
                None => Err(Error::Missing(part)),
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let parsed = texts
            .par_iter()
            .map(|(part, text)| model::parse(text, part))
            .collect::<Result<Vec<_>, Error>>()?;
        for ((part, _), file) in texts.into_iter().zip(parsed) {
            files.insert(part, file);
        }
        for file in files.values() {
            for reference in file.referenced_parts().map(part_key) {
                if !files.contains_key(reference) && !wanted.iter().any(|w| w == reference) {
                    wanted.push(reference.to_owned());
                }
            }
        }
    }
    Ok(files)
}

fn find<'a>(
    files: &'a HashMap<String, ModelFile>,
    file: &str,
    id: u32,
) -> Result<&'a RawObject, Error> {
    files
        .get(file)
        .and_then(|model| model.objects.iter().find(|object| object.id == id))
        .ok_or_else(|| malformed(file, format!("object {id} is used but not defined")))
}

/// An object's parts: itself when it is a mesh, otherwise one per mesh found
/// under each of its components; each part keeps what it was read as, to
/// tell an edit of it when it is written back.
fn parts(
    files: &HashMap<String, ModelFile>,
    file: &str,
    object: &RawObject,
    settings: Option<&ObjectSettings>,
    kept: &HashMap<u32, PartKept>,
) -> Result<Vec<Part>, Error> {
    let mut parts = read_parts(files, file, object, settings, kept)?;
    for (index, part) in parts.iter_mut().enumerate() {
        let read = kept::PartRead {
            index,
            name: part.name.clone(),
            kind: part.kind,
            own_filament: part.own_filament,
            transform: part.transform,
            mesh: part.mesh.clone(),
        };
        let own = part.kept.get_or_insert_with(Default::default);
        Arc::make_mut(own).read = Some(read);
    }
    Ok(parts)
}

fn read_parts(
    files: &HashMap<String, ModelFile>,
    file: &str,
    object: &RawObject,
    settings: Option<&ObjectSettings>,
    kept: &HashMap<u32, PartKept>,
) -> Result<Vec<Part>, Error> {
    // Zero stands for "not chosen" at either level.
    let chosen = |extruder: Option<u32>| extruder.filter(|extruder| *extruder > 0);
    let object_filament =
        chosen(settings.and_then(|settings| settings.extruder)).unwrap_or(1) as usize;
    let components = match &object.body {
        Shape::Mesh(mesh, paint, seam, extras) => {
            return Ok(vec![Part {
                name: object.name.clone(),
                kind: PartKind::Normal,
                filament: object_filament,
                own_filament: None,
                mesh: mesh.clone(),
                paint: paint.clone(),
                seam: seam.clone(),
                transform: Transform::IDENTITY,
                decoration: false,
                kept: Some(Arc::new(PartKept {
                    extras: extras.clone(),
                    ..PartKept::default()
                })),
            }]);
        }
        Shape::Components(components) => components,
    };

    let mut parts = Vec::new();
    for component in components {
        let part_settings = settings.and_then(|settings| settings.parts.get(&component.object));
        let mut leaves = Vec::new();
        let child_file = component.path.as_deref().map_or(file, part_key);
        flatten(
            files,
            child_file,
            component.object,
            component.transform,
            1,
            &mut leaves,
        )?;
        for (name, mesh, paint, seam, extras, transform) in leaves {
            let own =
                chosen(part_settings.and_then(|s| s.extruder)).map(|extruder| extruder as usize);
            let mut part_kept = kept.get(&component.object).cloned().unwrap_or_default();
            part_kept.extras = extras;
            parts.push(Part {
                kept: Some(Arc::new(part_kept)),
                name: part_settings.and_then(|s| s.name.clone()).or(name),
                kind: part_settings.map_or(PartKind::Normal, |s| s.kind()),
                filament: own.unwrap_or(object_filament),
                own_filament: own,
                mesh,
                paint,
                seam,
                transform,
                decoration: part_settings.is_some_and(|s| s.decoration),
            });
        }
    }
    Ok(parts)
}

/// A mesh found under a component: its name, itself, its paint and where
/// it is placed.
type Leaf = (
    Option<String>,
    Arc<Mesh>,
    Option<Arc<Vec<Facet>>>,
    Option<Arc<Vec<SeamFacet>>>,
    Option<Arc<Extras>>,
    Transform,
);

fn flatten(
    files: &HashMap<String, ModelFile>,
    file: &str,
    id: u32,
    transform: Transform,
    depth: usize,
    leaves: &mut Vec<Leaf>,
) -> Result<(), Error> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(malformed(
            file,
            format!("components under object {id} nest more than {MAX_COMPONENT_DEPTH} deep"),
        ));
    }
    let object = find(files, file, id)?;
    match &object.body {
        Shape::Mesh(mesh, paint, seam, extras) => {
            leaves.push((
                object.name.clone(),
                mesh.clone(),
                paint.clone(),
                seam.clone(),
                extras.clone(),
                transform,
            ));
        }
        Shape::Components(components) => {
            for component in components {
                let child_file = component.path.as_deref().map_or(file, part_key);
                // The component is placed inside its parent, then the parent is placed.
                let placed = component.transform.then(&transform);
                flatten(
                    files,
                    child_file,
                    component.object,
                    placed,
                    depth + 1,
                    leaves,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
