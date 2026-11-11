//! Reads 3MF projects: core 3MF geometry, plus the Bambu/Orca additions the
//! reference slicer writes (parts split across model files, part roles,
//! plates, and the project's settings).
//!
//! Colours painted onto triangles are read ([`Project::painted`]).
//!
//! Not read yet: painted supports and seams, per-object setting overrides,
//! and everything about a plate except which instances are on it.

mod model;
mod model_settings;
mod package;
mod paint;
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

use model::{ModelFile, RawObject, Shape};
use model_settings::{ModelSettings, ObjectSettings};
use package::Package;
pub use paint::Facet;
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
}

#[derive(Clone, Debug)]
pub struct Object {
    /// The object's id in the 3MF, which plates and settings refer to.
    pub id: u32,
    pub name: Option<String>,
    pub parts: Vec<Part>,
    pub instances: Vec<Instance>,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub name: Option<String>,
    pub kind: PartKind,
    /// Which filament the part is printed in, counting from 1: its own
    /// choice, or else its object's, or else the first.
    pub filament: usize,
    /// Shared between parts and objects that reuse the same geometry.
    pub mesh: Arc<Mesh>,
    /// The mesh's triangles as painted for filaments, cut up where the
    /// paint cuts them, when any of them is painted.
    pub paint: Option<Arc<Vec<Facet>>>,
    /// Places the mesh in its object's coordinates.
    pub transform: Transform,
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
}

impl PartKind {
    /// Unknown subtypes read as normal parts, as the reference does.
    fn from_subtype(subtype: &str) -> Self {
        match subtype {
            "negative_part" => Self::Negative,
            "modifier_part" => Self::Modifier,
            "support_blocker" => Self::SupportBlocker,
            "support_enforcer" => Self::SupportEnforcer,
            _ => Self::Normal,
        }
    }
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
        let model_settings = match package.read(MODEL_SETTINGS_PART)? {
            Some(text) => model_settings::parse(&text, MODEL_SETTINGS_PART)?,
            None => ModelSettings::default(),
        };
        let settings = package
            .read(PROJECT_SETTINGS_PART)?
            .map(|text| {
                Settings::from_json(&text)
                    .map_err(|reason| malformed(PROJECT_SETTINGS_PART, reason))
            })
            .transpose()?;

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
                    objects.push(Object {
                        id: item.object,
                        name: object_settings
                            .and_then(|s| s.name.clone())
                            .or_else(|| raw.name.clone()),
                        parts: parts(&files, file, raw, object_settings)?,
                        instances: Vec::new(),
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

        let plates = model_settings
            .plates
            .into_iter()
            .map(|plate| Plate {
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

        Ok(Self {
            objects,
            plates,
            settings,
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
        let mut shared: std::collections::BTreeMap<usize, Mesh> = Default::default();
        let mut carved = Vec::new();
        for (object, instance) in self.printable(plate) {
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

/// Printed material and, for objects that have negative parts, the volume
/// to remove from it.
#[derive(Clone, Debug)]
pub struct Body {
    pub solid: Mesh,
    pub negative: Option<Mesh>,
    /// Which filament the solid is printed in, counting from 1.
    pub filament: usize,
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
/// under each of its components.
fn parts(
    files: &HashMap<String, ModelFile>,
    file: &str,
    object: &RawObject,
    settings: Option<&ObjectSettings>,
) -> Result<Vec<Part>, Error> {
    // Zero stands for "not chosen" at either level.
    let chosen = |extruder: Option<u32>| extruder.filter(|extruder| *extruder > 0);
    let object_filament =
        chosen(settings.and_then(|settings| settings.extruder)).unwrap_or(1) as usize;
    let components = match &object.body {
        Shape::Mesh(mesh, paint) => {
            return Ok(vec![Part {
                name: object.name.clone(),
                kind: PartKind::Normal,
                filament: object_filament,
                mesh: mesh.clone(),
                paint: paint.clone(),
                transform: Transform::IDENTITY,
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
        for (name, mesh, paint, transform) in leaves {
            parts.push(Part {
                name: part_settings.and_then(|s| s.name.clone()).or(name),
                kind: part_settings.map_or(PartKind::Normal, |s| s.kind),
                filament: chosen(part_settings.and_then(|s| s.extruder))
                    .map_or(object_filament, |extruder| extruder as usize),
                mesh,
                paint,
                transform,
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
        Shape::Mesh(mesh, paint) => {
            leaves.push((object.name.clone(), mesh.clone(), paint.clone(), transform));
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
