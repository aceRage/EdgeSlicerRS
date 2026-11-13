use std::path::{Path, PathBuf};

use edge_3mf::{Body, InstanceParts, Painted, PartKind, PlateKept, Project};
use edge_config::{Settings, Value};
use edge_gcode::PrintSettings;
use edge_mesh::Mesh;

use crate::Failure;

/// A model file read, before anything is placed on a bed.
pub struct Opened {
    path: PathBuf,
    pub(crate) held: Held,
}

pub(crate) enum Held {
    Model(Mesh),
    Project {
        project: Box<Project>,
        plate: Option<usize>,
    },
}

/// What a model file holds, placed where it prints.
pub struct Loaded {
    /// What to slice.
    pub bodies: Vec<Body>,
    /// The objects with colours painted on them, each by itself.
    pub painted: Vec<Painted>,
    /// For a project, the settings it was saved with.
    pub settings: Option<Settings>,
    /// For a project, what is painted on its models that is not read.
    pub unread_paint: Vec<&'static str>,
    /// Whether anything to slice has a Precise Seam part, which is not
    /// followed.
    pub precise_seams: bool,
    /// For a project, each object on the plate by itself, for what depends
    /// on which object is which. Empty for a bare model, which is one object.
    pub objects: Vec<Mesh>,
    /// What is cut out of each of those objects (its negative parts): an
    /// empty mesh for an object with none.
    pub negatives: Vec<Mesh>,
    /// The printed parts of each of those objects, each by itself, for
    /// where they meet; empty for a bare model, which is one part.
    pub parts: Vec<InstanceParts>,
    /// Which of the project's plates this is, counting from 0.
    pub plate: usize,
    /// The order the plate sets for filaments on the first layer, as
    /// filament numbers from 1; empty when it sets none.
    pub first_layer_sequence: Vec<u32>,
    /// What the plate sets for itself over the project's settings ("Customize
    /// current plate"): its bed type, print sequence, vase mode and later
    /// layers' filament order, under the settings' own names. Empty for a
    /// plate that sets none of them.
    pub plate_settings: Settings,
    /// The least and greatest corners of everything to slice, in mm.
    pub bounds: ([f32; 3], [f32; 3]),
    /// Whether everything stands where it is to print because it was put
    /// there by hand ([`crate::Model`]): a model without settings of its
    /// own is then not moved to the middle of the bed.
    pub placed: bool,
    /// The G-code set at the plate's layers, when it has an entry.
    pub custom_gcode: Option<edge_3mf::CustomGcode>,
    /// The objects on the plate that set something for themselves: their
    /// own settings, a part's, a modifier, a height range, a support
    /// blocker or enforcer. Their printed parts are among `bodies`, one
    /// body a part, each tagged with its place here.
    pub own: Vec<OwnObject>,
}

/// An object on the plate that sets something for itself.
#[derive(Clone, Debug)]
pub struct OwnObject {
    /// Its place among [`Loaded::objects`].
    pub object: usize,
    /// Its name, for what is said of it.
    pub name: String,
    /// What it sets, as its file has it.
    pub settings: Vec<(String, String)>,
    /// Its printed parts, modifiers, support blockers and enforcers, in
    /// its own order, in world coordinates.
    pub volumes: Vec<OwnVolume>,
    /// Its height ranges, with their heights measured from the bed.
    pub ranges: Vec<edge_3mf::LayerRange>,
    /// Its variable layer height profile, in its own heights (from its
    /// bottom), when its file has one.
    pub layer_height_profile: Option<Vec<f64>>,
    /// Its bottom as it stands, in mm: what its ranges and profile are
    /// measured from.
    pub bottom: f64,
    /// Its printed parts as one mesh, in world coordinates: what its
    /// height is measured by.
    pub solid: Mesh,
    /// The pieces of its printed parts' triangles painted to have
    /// supports or to keep them off, in world coordinates, each facing
    /// out of the part.
    pub support_paint: Vec<(edge_mesh::Triangle, edge_3mf::SeamPaint)>,
}

/// One part of an [`OwnObject`].
#[derive(Clone, Debug)]
pub struct OwnVolume {
    /// A printed part, a modifier, a support blocker or enforcer.
    pub kind: PartKind,
    /// Its name, for what is said of it.
    pub name: String,
    /// The filament it prints in, from 1: for a printed part its own or
    /// its object's; for a modifier its own, when it sets one.
    pub filament: Option<usize>,
    /// What it sets, as its file has it.
    pub settings: Vec<(String, String)>,
    /// Its mesh, in world coordinates. Empty for a printed part, which is
    /// among the bodies.
    pub mesh: Mesh,
}

/// Whether an object sets anything for itself that slicing has to keep
/// apart: settings of its own or of a part, a modifier, a height range, a
/// support blocker or enforcer.
fn sets_own(object: &edge_3mf::Object) -> bool {
    !object.settings().is_empty()
        || (object.parts.iter()).any(|part| match part.kind {
            PartKind::Normal => !part.settings().is_empty() || !part.support_paint().is_empty(),
            PartKind::Modifier | PartKind::SupportBlocker | PartKind::SupportEnforcer => true,
            PartKind::Negative | PartKind::PreciseSeam => false,
        })
        || !object.layer_ranges().is_empty()
        || object.layer_height_profile().is_some()
}

/// The objects on a plate that set something for themselves, and a body
/// for each of their printed parts.
fn own_objects(project: &Project, plate: Option<usize>) -> (Vec<OwnObject>, Vec<Body>) {
    let (mut own, mut bodies) = (Vec::new(), Vec::new());
    let mut place = 0;
    for (object, instance) in project.printed(plate) {
        let printed = (object.parts.iter())
            .any(|part| part.kind == PartKind::Normal && !part.mesh.triangles.is_empty());
        if !printed {
            continue;
        }
        place += 1;
        if !sets_own(object) {
            continue;
        }
        let mut negative = Mesh::default();
        for part in object
            .parts
            .iter()
            .filter(|part| part.kind == PartKind::Negative)
        {
            negative.append(&part.mesh, &part.transform.then(&instance.transform));
        }
        let at = own.len();
        let mut volumes = Vec::new();
        let mut support_paint = Vec::new();
        for part in object
            .parts
            .iter()
            .filter(|part| part.kind == PartKind::Normal)
        {
            let placed = part.transform.then(&instance.transform);
            let mirrored = placed.is_mirrored();
            for facet in part.support_paint() {
                let [a, b, c] = facet
                    .corners
                    .map(|corner| placed.apply(corner.map(f64::from)).map(|c| c as f32));
                // Mirrored, a triangle faces the other way round.
                support_paint.push((if mirrored { [a, c, b] } else { [a, b, c] }, facet.kind));
            }
        }
        for part in &object.parts {
            let (filament, mesh) = match part.kind {
                PartKind::Normal => {
                    if part.mesh.triangles.is_empty() {
                        continue;
                    }
                    bodies.push(Body {
                        solid: part.placed(instance),
                        negative: (!negative.triangles.is_empty()).then(|| negative.clone()),
                        filament: part.filament,
                        part: Some((at, volumes.len())),
                    });
                    (Some(part.filament), Mesh::default())
                }
                PartKind::Modifier | PartKind::SupportBlocker | PartKind::SupportEnforcer => {
                    (part.own_filament, part.placed(instance))
                }
                PartKind::Negative | PartKind::PreciseSeam => continue,
            };
            volumes.push(OwnVolume {
                kind: part.kind,
                name: part.name.clone().unwrap_or_default(),
                filament,
                settings: part.settings(),
                mesh,
            });
        }
        // A range's heights are the object's own, measured from its
        // bottom as it stands on the bed (Model.cpp: the ranges are in
        // the object's coordinates, its bottom at 0).
        let mut solid = Mesh::default();
        for part in object
            .parts
            .iter()
            .filter(|part| part.kind == PartKind::Normal)
        {
            solid.append(&part.mesh, &part.transform.then(&instance.transform));
        }
        let bottom = solid.bounds().map_or(0.0, |(low, _)| f64::from(low[2]));
        own.push(OwnObject {
            object: place - 1,
            name: object.name.clone().unwrap_or_default(),
            settings: object.settings(),
            volumes,
            support_paint,
            layer_height_profile: object.layer_height_profile(),
            bottom,
            solid,
            ranges: (object.layer_ranges().into_iter())
                .map(|range| edge_3mf::LayerRange {
                    min_z: range.min_z + bottom,
                    max_z: range.max_z + bottom,
                    ..range
                })
                .collect(),
        });
    }
    (own, bodies)
}

/// One plate of a project, as it is listed before anything is placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlateInfo {
    /// The plate's number, from 1.
    pub id: u32,
    /// The name the project gives it; empty when it has none.
    pub name: String,
    /// The filaments what is on it prints in, by number from 1, in order;
    /// empty when nothing printable is on it.
    pub filaments: Vec<usize>,
    /// The name of each printable object on it, in the plate's own order;
    /// one for each copy, empty where the object has no name.
    pub objects: Vec<String>,
}

/// The filaments what is on a plate (or in the whole project for `None`)
/// prints in, by number from 1, in order.
fn filaments_on(project: &Project, plate: Option<usize>) -> Vec<usize> {
    let mut used: Vec<usize> = project
        .bodies(plate)
        .iter()
        .map(|body| body.filament)
        .chain(
            project
                .painted(plate)
                .iter()
                .flat_map(|object| &object.facets)
                .map(|facet| usize::from(facet.filament))
                .filter(|filament| *filament > 0),
        )
        .collect();
    used.sort_unstable();
    used.dedup();
    used
}

/// Reads an STL (binary or ASCII), an OBJ (its geometry) or a 3MF project. `plate` is the number
/// of the plate to take from a project; left out, it is plate 1 when the
/// project has plates.
pub fn open(path: &Path, plate: Option<u32>) -> Result<Opened, Failure> {
    let named = |e: &dyn std::error::Error| format!("{}: {e}", path.display());
    let is_3mf = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"));
    if !is_3mf {
        if plate.is_some() {
            return Err("--plate only applies to 3MF files".into());
        }
        let is_obj = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"));
        let solid = if is_obj {
            edge_mesh::obj::load(path).map_err(|e| named(&e))?
        } else {
            edge_mesh::stl::load(path).map_err(|e| named(&e))?
        };
        return Ok(Opened {
            path: path.to_owned(),
            held: Held::Model(solid),
        });
    }

    let project = Project::open(path).map_err(|e| named(&e))?;
    let plate = match plate {
        Some(id) => Some(project.plate_index(id).ok_or_else(|| {
            let ids: Vec<String> = project
                .plates
                .iter()
                .map(|plate| plate.id.to_string())
                .collect();
            match ids.as_slice() {
                [] => format!(
                    "{}: no plate {id}; the file defines no plates",
                    path.display()
                ),
                _ => format!(
                    "{}: no plate {id}; it has plates {}",
                    path.display(),
                    ids.join(", ")
                ),
            }
        })?),
        None => project.plate_index(1),
    };
    Ok(Opened {
        path: path.to_owned(),
        held: Held::Project {
            project: Box::new(project),
            plate,
        },
    })
}

impl Opened {
    /// The file it was read from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The number of the plate taken and how many the project has; none
    /// for a bare model or a project without plates.
    pub fn plate(&self) -> Option<(u32, usize)> {
        match &self.held {
            Held::Project {
                project,
                plate: Some(index),
            } => Some((project.plates[*index].id, project.plates.len())),
            _ => None,
        }
    }

    /// The numbers of the project's plates, in order; empty for a bare model.
    pub fn plates(&self) -> Vec<u32> {
        match &self.held {
            Held::Project { project, .. } => project.plates.iter().map(|plate| plate.id).collect(),
            Held::Model(_) => Vec::new(),
        }
    }

    /// The filaments what is to be sliced prints in, by number from 1, in
    /// order.
    pub fn filaments(&self) -> Vec<usize> {
        match &self.held {
            Held::Model(_) => vec![1],
            Held::Project { project, plate } => filaments_on(project, *plate),
        }
    }

    /// Every plate of a project, in order, with what is on it: for
    /// listing the plates before one is taken. Empty for a bare model and
    /// for a project that defines no plates.
    pub fn plate_list(&self) -> Vec<PlateInfo> {
        let Held::Project { project, .. } = &self.held else {
            return Vec::new();
        };
        project
            .plates
            .iter()
            .enumerate()
            .map(|(index, plate)| PlateInfo {
                id: plate.id,
                name: plate.name.clone(),
                filaments: filaments_on(project, Some(index)),
                objects: plate
                    .instances
                    .iter()
                    .map(|at| (&project.objects[at.object], at.instance))
                    .filter(|(object, instance)| object.instances[*instance].printable)
                    .map(|(object, _)| object.name.clone().unwrap_or_default())
                    .collect(),
            })
            .collect()
    }

    /// The settings a project was saved with.
    pub fn settings(&self) -> Option<&Settings> {
        match &self.held {
            Held::Project { project, .. } => project.settings.as_ref(),
            Held::Model(_) => None,
        }
    }

    /// Everything on the plate, moved to where it sits on the bed.
    pub fn place(self) -> Result<Loaded, Failure> {
        let path = self.path;
        match self.held {
            Held::Model(solid) => lone(&path, solid),
            Held::Project { project, plate } => {
                let origin = plate
                    .map(|index| {
                        plate_origin(index, project.plates.len(), project.settings.as_ref())
                            .ok_or_else(|| unplaced(&path, project.plates[index].id))
                    })
                    .transpose()?;
                let settings = project.settings.clone();
                placed(&path, &project, plate, origin, settings)
            }
        }
    }
}

/// What is said of a plate whose place among the others cannot be found.
pub(crate) fn unplaced(path: &Path, id: u32) -> Failure {
    format!(
        "{}: plate {id} cannot be placed: the project does not give the bed's size",
        path.display(),
    )
    .into()
}

/// A bare model as what to slice: one body, in the first filament, where
/// the file has it. Slicing puts it in the middle of the bed.
pub(crate) fn lone(path: &Path, solid: Mesh) -> Result<Loaded, Failure> {
    let bodies = vec![Body {
        solid,
        negative: None,
        filament: 1,
        part: None,
    }];
    let bounds = bounds_of(path, &bodies)?;
    Ok(Loaded {
        bodies,
        painted: Vec::new(),
        settings: None,
        unread_paint: Vec::new(),
        precise_seams: false,
        objects: Vec::new(),
        negatives: Vec::new(),
        parts: Vec::new(),
        plate: 0,
        first_layer_sequence: Vec::new(),
        plate_settings: Settings::default(),
        bounds,
        placed: false,
        custom_gcode: None,
        own: Vec::new(),
    })
}

/// What stands on a project's plate (or all of the project for no plate)
/// as what to slice, measured from `origin`, which is where the plate
/// stands in the project's own coordinates.
pub(crate) fn placed(
    path: &Path,
    project: &Project,
    plate: Option<usize>,
    origin: Option<(f64, f64)>,
    settings: Option<Settings>,
) -> Result<Loaded, Failure> {
    let first_layer_sequence = plate
        .map(|index| project.plates[index].first_layer_sequence.clone())
        .unwrap_or_default();
    // The objects that set something for themselves are sliced part by
    // part; the rest as they always were.
    let mut bodies = project.bodies_where(plate, |object| !sets_own(object));
    let (mut own, own_bodies) = own_objects(project, plate);
    bodies.extend(own_bodies);
    let mut painted = project.painted(plate);
    let mut objects = project.instance_meshes(plate);
    let mut negatives = project.instance_negatives(plate);
    let mut parts = project.instance_parts(plate);
    if let Some((x, y)) = origin {
        shift(&mut bodies, -x, -y);
        shift_meshes(objects.iter_mut(), -x, -y);
        shift_meshes(negatives.iter_mut(), -x, -y);
        shift_meshes(
            parts.iter_mut().flat_map(|object| &mut object.parts),
            -x,
            -y,
        );
        shift_meshes(
            own.iter_mut()
                .flat_map(|object| &mut object.volumes)
                .map(|volume| &mut volume.mesh),
            -x,
            -y,
        );
        shift_meshes(own.iter_mut().map(|object| &mut object.solid), -x, -y);
        for object in &mut painted {
            shift_meshes(
                object.parts.iter_mut().chain(object.negative.as_mut()),
                -x,
                -y,
            );
            for corner in object
                .facets
                .iter_mut()
                .flat_map(|facet| &mut facet.corners)
            {
                corner[0] = (f64::from(corner[0]) - x) as f32;
                corner[1] = (f64::from(corner[1]) - y) as f32;
            }
        }
    }
    let bounds = bounds_of(path, &bodies)?;
    Ok(Loaded {
        bodies,
        painted,
        settings,
        unread_paint: project.unread_paint.clone(),
        precise_seams: project.has_precise_seams(plate),
        objects,
        negatives,
        parts,
        plate: plate.unwrap_or(0),
        first_layer_sequence,
        plate_settings: plate
            .map(|index| plate_settings(project.plates[index].kept.as_deref()))
            .unwrap_or_default(),
        bounds,
        placed: false,
        // Plate 1's for a project without plates, as the reference reads it.
        custom_gcode: project
            .custom_gcode
            .get(&plate.map_or(1, |index| project.plates[index].id))
            .cloned(),
        own,
    })
}

/// The bed types a plate may name, as the reference keys them (PrintConfig.cpp
/// `s_keys_map_BedType`), but "Default Plate", which is the project's.
pub(crate) const BED_TYPES: [&str; 7] = [
    "Cool Plate",
    "Engineering Plate",
    "High Temp Plate",
    "Textured PEI Plate",
    "Textured Cool Plate",
    "Graphic Effect Plate",
    "Supertack Plate",
];

/// What a plate sets for itself over the project's settings, as its file
/// keeps it (`Metadata/model_settings.config`, read as bbs_3mf.cpp
/// `_BBS_3MF_Importer` reads a plate's metadata into its config), under the
/// settings' own names: `curr_bed_type`, `print_sequence`, `spiral_mode`,
/// `other_layers_print_sequence` with `other_layers_print_sequence_nums`.
/// The first layer's order is apart ([`Loaded::first_layer_sequence`]). A
/// value the reference would not read is left out.
pub(crate) fn plate_config(kept: Option<&PlateKept>) -> Vec<(&'static str, String)> {
    let Some(kept) = kept else {
        return Vec::new();
    };
    let mut config = Vec::new();
    if let Some(bed) = kept.value("bed_type").filter(|bed| BED_TYPES.contains(bed)) {
        config.push(("curr_bed_type", bed.to_owned()));
    }
    if let Some(sequence) = kept
        .value("print_sequence")
        .filter(|sequence| matches!(*sequence, "by layer" | "by object"))
    {
        config.push(("print_sequence", sequence.to_owned()));
    }
    match kept.value("spiral_mode") {
        Some("true" | "1") => config.push(("spiral_mode", "1".into())),
        Some("false" | "0") => config.push(("spiral_mode", "0".into())),
        _ => {}
    }
    // Both or neither, as the reference writes them.
    if let (Some(sequence), Some(count)) = (
        kept.value("other_layers_print_sequence"),
        kept.value("other_layers_print_sequence_nums"),
    ) {
        config.push(("other_layers_print_sequence", sequence.trim().to_owned()));
        config.push(("other_layers_print_sequence_nums", count.trim().to_owned()));
    }
    config
}

/// [`plate_config`] as settings to lay over a slice's.
pub(crate) fn plate_settings(kept: Option<&PlateKept>) -> Settings {
    let mut settings = Settings::default();
    for (key, value) in plate_config(kept) {
        let value = if key == "other_layers_print_sequence" {
            Value::Many(value.split_whitespace().map(str::to_owned).collect())
        } else {
            Value::One(value)
        };
        settings.insert(key, value);
    }
    settings
}

/// The least and greatest corners of everything to slice.
fn bounds_of(path: &Path, bodies: &[Body]) -> Result<([f32; 3], [f32; 3]), Failure> {
    bodies
        .iter()
        .filter_map(|body| body.solid.bounds())
        .reduce(|(lo, hi), (min, max)| {
            (
                std::array::from_fn(|i| lo[i].min(min[i])),
                std::array::from_fn(|i| hi[i].max(max[i])),
            )
        })
        .ok_or_else(|| format!("{}: nothing printable to slice", path.display()).into())
}

/// Moves bodies across the bed by `(x, y)` mm.
pub(crate) fn shift(bodies: &mut [Body], x: f64, y: f64) {
    let meshes = bodies
        .iter_mut()
        .flat_map(|body| std::iter::once(&mut body.solid).chain(body.negative.as_mut()));
    shift_meshes(meshes, x, y);
}

fn shift_meshes<'a>(meshes: impl Iterator<Item = &'a mut Mesh>, x: f64, y: f64) {
    for mesh in meshes {
        for vertex in &mut mesh.vertices {
            vertex[0] = (f64::from(vertex[0]) + x) as f32;
            vertex[1] = (f64::from(vertex[1]) + y) as f32;
        }
    }
}

/// Where plate `index` of `count` sits in a project's own coordinates. The
/// plates are laid out in a grid a fifth of a bed apart, rows running
/// towards -Y (PartPlate.cpp `compute_origin`); a part's position on its
/// plate, which is what the printer needs, is measured from here. The bed
/// size is the project's own `printable_area`, whole millimetres as the
/// reference takes it.
pub(crate) fn plate_origin(
    index: usize,
    count: usize,
    settings: Option<&Settings>,
) -> Option<(f64, f64)> {
    if index == 0 {
        return Some((0.0, 0.0));
    }
    let [min_x, min_y, max_x, max_y] = PrintSettings::from_settings(settings?).ok()?.bed?;
    let (width, depth) = ((max_x - min_x).trunc(), (max_y - min_y).trunc());
    Some(crate::scene::plate_position(index, count, width, depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plates_are_laid_out_in_a_grid_running_back_along_y() {
        let settings = Settings::from_json(
            r#"{"printable_area": ["0.5x1", "270.5x1", "270.5x271", "0.5x271"]}"#,
        )
        .unwrap();
        let origin = |index, count| plate_origin(index, count, Some(&settings));
        // A 270 x 270 mm bed, plates a fifth of a bed apart.
        assert_eq!(origin(0, 3), Some((0.0, 0.0)));
        assert_eq!(origin(1, 3), Some((324.0, 0.0)));
        // Three plates make two columns, so the third starts a new row.
        assert_eq!(origin(2, 3), Some((0.0, -324.0)));
        assert_eq!(origin(4, 9), Some((324.0, -324.0)));
        // The first plate needs no bed size; any other does.
        assert_eq!(plate_origin(0, 2, None), Some((0.0, 0.0)));
        assert_eq!(plate_origin(1, 2, None), None);
        assert_eq!(plate_origin(1, 2, Some(&Settings::default())), None);
    }

    #[test]
    fn what_is_cut_out_of_each_object_stands_where_the_object_does() {
        use edge_3mf::{Instance, InstanceRef, Object, Part, PartKind, Plate};
        use edge_mesh::Transform;
        use std::sync::Arc;
        let cube = |min: [f32; 3], max: [f32; 3]| {
            Arc::new(Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
                min, max,
            )))
        };
        let part = |kind, mesh| Part {
            kept: None,
            name: None,
            kind,
            filament: 1,
            own_filament: None,
            mesh,
            paint: None,
            seam: None,
            transform: Transform::IDENTITY,
            decoration: false,
        };
        let at = |x, y| Instance {
            transform: Transform::translation([x, y, 0.0]),
            printable: true,
        };
        // A box with a hole cut in it, and a plain one.
        let project = Project {
            kept: None,
            custom_gcode: Default::default(),
            objects: vec![
                Object {
                    kept: None,
                    id: 1,
                    name: None,
                    filament: None,
                    parts: vec![
                        part(PartKind::Normal, cube([0.0; 3], [10.0; 3])),
                        part(PartKind::Negative, cube([2.0; 3], [4.0; 3])),
                    ],
                    instances: vec![at(100.0, 50.0)],
                },
                Object {
                    kept: None,
                    id: 2,
                    name: None,
                    filament: None,
                    parts: vec![part(PartKind::Normal, cube([0.0; 3], [5.0; 3]))],
                    instances: vec![at(130.0, 50.0)],
                },
            ],
            plates: vec![Plate {
                kept: None,
                id: 1,
                name: String::new(),
                first_layer_sequence: Vec::new(),
                instances: vec![
                    InstanceRef {
                        object: 0,
                        instance: 0,
                    },
                    InstanceRef {
                        object: 1,
                        instance: 0,
                    },
                ],
            }],
            settings: None,
            unread_paint: Vec::new(),
        };
        let loaded = placed(
            Path::new("cut.3mf"),
            &project,
            Some(0),
            Some((80.0, 40.0)),
            None,
        )
        .unwrap();
        assert_eq!(loaded.objects.len(), 2);
        assert_eq!(loaded.negatives.len(), 2);
        assert_eq!(
            loaded.objects[0].bounds(),
            Some(([20.0, 10.0, 0.0], [30.0, 20.0, 10.0]))
        );
        assert_eq!(
            loaded.negatives[0].bounds(),
            Some(([22.0, 12.0, 2.0], [24.0, 14.0, 4.0]))
        );
        assert!(loaded.negatives[1].triangles.is_empty());
    }

    #[test]
    fn shifting_moves_solid_and_negative_parts_together() {
        let cube = edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ));
        let mut bodies = [Body {
            solid: cube.clone(),
            negative: Some(cube),
            filament: 1,
            part: None,
        }];
        shift(&mut bodies, 10.0, -5.0);
        for mesh in [&bodies[0].solid, bodies[0].negative.as_ref().unwrap()] {
            let (min, max) = mesh.bounds().unwrap();
            assert_eq!((min, max), ([10.0, -5.0, 0.0], [11.0, -4.0, 1.0]));
        }
    }
}
