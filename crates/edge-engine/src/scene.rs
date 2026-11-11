//! A model file as a scene to look at before it is sliced: each object's
//! mesh once, in the object's own coordinates, and where every copy of it
//! stands on its plate. Nothing here is used for slicing; [`Opened::place`]
//! is what a slice starts from.

use edge_3mf::{PartKind, Project};
use edge_mesh::{Mesh, Transform};

use crate::load::{Held, Opened, plate_origin};

/// What a triangle of an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Printed material, in this filament, counting from 1: the filament
    /// painted onto the triangle, else its part's.
    Filament(u8),
    /// Volume taken away from the object's printed parts.
    Negative,
    /// A region where other settings apply.
    Modifier,
    SupportBlocker,
    SupportEnforcer,
}

/// One object: every part of it in one mesh, in the object's own
/// coordinates, with what each triangle is.
#[derive(Clone, Debug, Default)]
pub struct SceneObject {
    /// The name the file gives it; empty when it has none.
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// One for each triangle.
    pub marks: Vec<Mark>,
    /// The least and greatest corners of its printed material, in its own
    /// coordinates; none when it has none.
    pub bounds: Option<([f32; 3], [f32; 3])>,
}

/// One copy of an object, where it stands.
#[derive(Clone, Copy, Debug)]
pub struct Placing {
    /// Index into [`Scene::objects`].
    pub object: usize,
    /// The plate it is on, as an index into [`Scene::plates`]; none for a
    /// copy that is on no plate.
    pub plate: Option<usize>,
    pub printable: bool,
    /// Takes the object's coordinates to those of its plate, whose origin
    /// is the bed's (0, 0); for a copy on no plate, to those of the first.
    pub transform: Transform,
}

/// A plate of the scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenePlate {
    /// The plate's number, from 1.
    pub id: u32,
    pub name: String,
}

/// Everything in a model file, to look at.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub objects: Vec<SceneObject>,
    pub placings: Vec<Placing>,
    /// Empty for a bare model and for a project without plates.
    pub plates: Vec<ScenePlate>,
    /// A bare model, which has no place on a bed of its own: slicing puts
    /// it in the middle, with its lowest point on the bed.
    pub bare: bool,
}

/// Where plate `index` of `count` stands among a project's plates, on a
/// bed `width` by `depth` mm: in a grid a fifth of a bed apart, rows
/// running towards -Y (PartPlate.cpp `compute_shape_position`, with
/// `LOGICAL_PART_PLATE_GAP` and `compute_colum_count`).
pub fn plate_position(index: usize, count: usize, width: f64, depth: f64) -> (f64, f64) {
    let columns = ((count as f64).sqrt().ceil() as usize).max(1);
    let (row, column) = (index / columns, index % columns);
    (column as f64 * width * 1.2, -(row as f64) * depth * 1.2)
}

fn mark_of(kind: PartKind, filament: usize) -> Mark {
    match kind {
        // A filament past the last a triangle can carry is shown as the last.
        PartKind::Normal => Mark::Filament(filament.clamp(1, 255) as u8),
        PartKind::Negative => Mark::Negative,
        PartKind::Modifier => Mark::Modifier,
        PartKind::SupportBlocker => Mark::SupportBlocker,
        PartKind::SupportEnforcer => Mark::SupportEnforcer,
    }
}

fn widen(bounds: &mut Option<([f32; 3], [f32; 3])>, point: [f32; 3]) {
    let (low, high) = bounds.get_or_insert((point, point));
    for axis in 0..3 {
        low[axis] = low[axis].min(point[axis]);
        high[axis] = high[axis].max(point[axis]);
    }
}

fn object_of(object: &edge_3mf::Object) -> SceneObject {
    let mut scene = SceneObject {
        name: object.name.clone().unwrap_or_default(),
        ..SceneObject::default()
    };
    for part in &object.parts {
        // Mirrored, a triangle faces the other way round.
        let mirrored = part.transform.is_mirrored();
        let place = |corner: [f32; 3]| -> [f32; 3] {
            part.transform
                .apply(corner.map(f64::from))
                .map(|value| value as f32)
        };
        let printed = part.kind == PartKind::Normal;
        match (&part.paint, printed) {
            // Painted: the triangles as the paint cuts them, each with the
            // filament painted on it.
            (Some(paint), true) => {
                for facet in paint.iter() {
                    let from = scene.vertices.len() as u32;
                    for corner in facet.corners {
                        let placed = place(corner);
                        widen(&mut scene.bounds, placed);
                        scene.vertices.push(placed);
                    }
                    scene.triangles.push(if mirrored {
                        [from, from + 2, from + 1]
                    } else {
                        [from, from + 1, from + 2]
                    });
                    let filament = match facet.filament {
                        0 => part.filament,
                        painted => usize::from(painted),
                    };
                    scene.marks.push(mark_of(PartKind::Normal, filament));
                }
            }
            _ => {
                let from = scene.vertices.len() as u32;
                for vertex in &part.mesh.vertices {
                    let placed = place(*vertex);
                    if printed {
                        widen(&mut scene.bounds, placed);
                    }
                    scene.vertices.push(placed);
                }
                let mark = mark_of(part.kind, part.filament);
                for [a, b, c] in &part.mesh.triangles {
                    scene.triangles.push(if mirrored {
                        [from + a, from + c, from + b]
                    } else {
                        [from + a, from + b, from + c]
                    });
                    scene.marks.push(mark);
                }
            }
        }
    }
    scene
}

fn project_scene(project: &Project) -> Scene {
    let count = project.plates.len();
    let mut placings = Vec::new();
    for (index, object) in project.objects.iter().enumerate() {
        for (copy, instance) in object.instances.iter().enumerate() {
            let plate = project.plates.iter().position(|plate| {
                plate
                    .instances
                    .iter()
                    .any(|at| at.object == index && at.instance == copy)
            });
            // Measured from its plate's origin, as slicing measures it. A
            // plate that cannot be placed (the project does not give the
            // bed's size) leaves the copy where the file has it.
            let (x, y) = plate
                .and_then(|plate| plate_origin(plate, count, project.settings.as_ref()))
                .unwrap_or((0.0, 0.0));
            placings.push(Placing {
                object: index,
                plate,
                printable: instance.printable,
                transform: instance
                    .transform
                    .then(&Transform::translation([-x, -y, 0.0])),
            });
        }
    }
    Scene {
        objects: project.objects.iter().map(object_of).collect(),
        placings,
        plates: project
            .plates
            .iter()
            .map(|plate| ScenePlate {
                id: plate.id,
                name: plate.name.clone(),
            })
            .collect(),
        bare: false,
    }
}

fn model_scene(mesh: &Mesh) -> Scene {
    Scene {
        objects: vec![SceneObject {
            name: String::new(),
            vertices: mesh.vertices.clone(),
            triangles: mesh.triangles.clone(),
            marks: vec![Mark::Filament(1); mesh.triangles.len()],
            bounds: mesh.bounds(),
        }],
        placings: vec![Placing {
            object: 0,
            plate: None,
            printable: true,
            transform: Transform::IDENTITY,
        }],
        plates: Vec::new(),
        bare: true,
    }
}

impl Opened {
    /// Everything in the file, whichever plate was asked for: every
    /// object's mesh once and where each copy of it stands.
    pub fn scene(&self) -> Scene {
        match &self.held {
            Held::Model(mesh) => model_scene(mesh),
            Held::Project { project, .. } => project_scene(project),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use edge_3mf::{Facet, Instance, InstanceRef, Object, Part, Plate};
    use edge_config::Settings;

    use super::*;

    fn cube(low: [f32; 3], high: [f32; 3]) -> Arc<Mesh> {
        Arc::new(Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
            low, high,
        )))
    }

    fn part(kind: PartKind, filament: usize, mesh: Arc<Mesh>) -> Part {
        Part {
            name: None,
            kind,
            filament,
            mesh,
            paint: None,
            transform: Transform::IDENTITY,
        }
    }

    #[test]
    fn plates_stand_in_a_grid_a_fifth_of_a_bed_apart() {
        assert_eq!(plate_position(0, 1, 270.0, 270.0), (0.0, 0.0));
        assert_eq!(plate_position(1, 3, 270.0, 270.0), (324.0, 0.0));
        // Three plates make two columns: the third starts a row towards -Y.
        assert_eq!(plate_position(2, 3, 270.0, 270.0), (0.0, -324.0));
        assert_eq!(plate_position(4, 9, 200.0, 100.0), (240.0, -120.0));
        // Nine plates, three to a row.
        assert_eq!(plate_position(8, 9, 200.0, 100.0), (480.0, -240.0));
    }

    #[test]
    fn an_object_is_one_mesh_with_what_each_triangle_is() {
        let mut second = part(
            PartKind::Normal,
            3,
            cube([10.0, 0.0, 0.0], [12.0, 2.0, 2.0]),
        );
        second.transform = Transform::translation([0.0, 5.0, 0.0]);
        let object = Object {
            id: 1,
            name: Some("two parts".into()),
            parts: vec![
                part(PartKind::Normal, 1, cube([0.0; 3], [4.0, 4.0, 4.0])),
                second,
                part(PartKind::Negative, 1, cube([1.0; 3], [2.0, 2.0, 9.0])),
            ],
            instances: Vec::new(),
        };
        let scene = object_of(&object);
        assert_eq!(scene.name, "two parts");
        assert_eq!(scene.triangles.len(), 36);
        assert_eq!(scene.marks.len(), 36);
        assert_eq!(scene.marks[0], Mark::Filament(1));
        assert_eq!(scene.marks[12], Mark::Filament(3));
        assert_eq!(scene.marks[24], Mark::Negative);
        // The second part's triangles point at its own vertices, moved by
        // its transform.
        let first_of_second = scene.triangles[12][0] as usize;
        assert!(first_of_second >= 8);
        assert!(scene.vertices[first_of_second][1] >= 5.0);
        // The bounds are of the printed material: the negative part,
        // which reaches higher, does not count.
        assert_eq!(scene.bounds, Some(([0.0, 0.0, 0.0], [12.0, 7.0, 4.0])));
    }

    #[test]
    fn paint_cuts_a_part_into_triangles_of_their_own_filament() {
        let mut painted = part(PartKind::Normal, 2, cube([0.0; 3], [1.0, 1.0, 1.0]));
        painted.paint = Some(Arc::new(vec![
            Facet {
                corners: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                filament: 0,
            },
            Facet {
                corners: [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                filament: 5,
            },
        ]));
        let scene = object_of(&Object {
            id: 1,
            name: None,
            parts: vec![painted],
            instances: Vec::new(),
        });
        // Left unpainted, a triangle is its part's filament.
        assert_eq!(scene.marks, [Mark::Filament(2), Mark::Filament(5)]);
        assert_eq!(scene.triangles, [[0, 1, 2], [3, 4, 5]]);
        assert_eq!(scene.vertices.len(), 6);
    }

    #[test]
    fn a_mirrored_part_keeps_facing_outwards() {
        let mut mirrored = part(PartKind::Normal, 1, cube([0.0; 3], [1.0, 1.0, 1.0]));
        mirrored.transform = Transform {
            rows: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]],
        };
        let scene = object_of(&Object {
            id: 1,
            name: None,
            parts: vec![mirrored],
            instances: Vec::new(),
        });
        let mesh = Mesh {
            vertices: scene.vertices,
            triangles: scene.triangles,
        };
        assert!(mesh.signed_volume() > 0.99);
    }

    #[test]
    fn each_copy_is_measured_from_its_own_plate() {
        let settings =
            Settings::from_json(r#"{"printable_area": ["0x0", "200x0", "200x100", "0x100"]}"#)
                .unwrap();
        let at = |x: f64, y: f64, printable: bool| Instance {
            transform: Transform::translation([x, y, 0.0]),
            printable,
        };
        let project = Project {
            objects: vec![Object {
                id: 7,
                name: Some("block".into()),
                parts: vec![part(PartKind::Normal, 1, cube([0.0; 3], [1.0, 1.0, 1.0]))],
                // On plate 1, on plate 2 (which starts at x 240) and on none.
                instances: vec![
                    at(50.0, 60.0, true),
                    at(290.0, 60.0, false),
                    at(-80.0, 0.0, true),
                ],
            }],
            plates: vec![
                Plate {
                    id: 1,
                    name: String::new(),
                    first_layer_sequence: Vec::new(),
                    instances: vec![InstanceRef {
                        object: 0,
                        instance: 0,
                    }],
                },
                Plate {
                    id: 2,
                    name: "second".into(),
                    first_layer_sequence: Vec::new(),
                    instances: vec![InstanceRef {
                        object: 0,
                        instance: 1,
                    }],
                },
            ],
            settings: Some(settings),
            unread_paint: Vec::new(),
        };
        let scene = project_scene(&project);
        assert_eq!(scene.objects.len(), 1);
        assert!(!scene.bare);
        assert_eq!(
            scene.plates,
            [
                ScenePlate {
                    id: 1,
                    name: String::new()
                },
                ScenePlate {
                    id: 2,
                    name: "second".into()
                }
            ]
        );
        let placed: Vec<_> = scene
            .placings
            .iter()
            .map(|placing| {
                (
                    placing.plate,
                    placing.printable,
                    placing.transform.apply([0.0; 3]),
                )
            })
            .collect();
        assert_eq!(
            placed,
            [
                (Some(0), true, [50.0, 60.0, 0.0]),
                (Some(1), false, [50.0, 60.0, 0.0]),
                (None, true, [-80.0, 0.0, 0.0]),
            ]
        );
    }

    #[test]
    fn a_bare_model_is_one_object_with_no_plate() {
        let mesh = cube([0.0; 3], [2.0, 3.0, 4.0]);
        let scene = model_scene(&mesh);
        assert!(scene.bare);
        assert!(scene.plates.is_empty());
        assert_eq!(scene.objects[0].triangles.len(), 12);
        assert_eq!(scene.objects[0].marks, vec![Mark::Filament(1); 12]);
        assert_eq!(scene.objects[0].bounds, Some(([0.0; 3], [2.0, 3.0, 4.0])));
        assert_eq!(scene.placings.len(), 1);
        assert_eq!(scene.placings[0].plate, None);
    }
}
