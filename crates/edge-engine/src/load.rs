use std::path::{Path, PathBuf};

use edge_3mf::{Body, InstanceParts, Painted, Project};
use edge_config::Settings;
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
    /// The least and greatest corners of everything to slice, in mm.
    pub bounds: ([f32; 3], [f32; 3]),
    /// Whether everything stands where it is to print because it was put
    /// there by hand ([`crate::Model`]): a model without settings of its
    /// own is then not moved to the middle of the bed.
    pub placed: bool,
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

/// Reads an STL (binary or ASCII) or a 3MF project. `plate` is the number
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
        let solid = edge_mesh::stl::load(path).map_err(|e| named(&e))?;
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
        bounds,
        placed: false,
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
    let mut bodies = project.bodies(plate);
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
        for object in &mut painted {
            shift_meshes(
                std::iter::once(&mut object.solid).chain(object.negative.as_mut()),
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
        bounds,
        placed: false,
    })
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
            objects: vec![
                Object {
                    kept: None,
                    id: 1,
                    name: None,
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
        }];
        shift(&mut bodies, 10.0, -5.0);
        for mesh in [&bodies[0].solid, bodies[0].negative.as_ref().unwrap()] {
            let (min, max) = mesh.bounds().unwrap();
            assert_eq!((min, max), ([10.0, -5.0, 0.0], [11.0, -4.0, 1.0]));
        }
    }
}
