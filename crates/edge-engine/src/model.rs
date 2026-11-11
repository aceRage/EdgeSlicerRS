//! A model held in memory so that what stands on its plates can be
//! changed before it is sliced: its plates, its objects, each object's
//! copies and where they stand. The reference's `Model`, `ModelObject` and
//! `ModelInstance` with its `PartPlateList` are the same thing.
//!
//! [`Model::open`] reads a file, the edits below change it and
//! [`Model::place`] gives what [`crate::prepare`] takes. A model that has
//! not been changed gives exactly what [`crate::open`] and
//! [`crate::Opened::place`] give for its file, so it slices to the same
//! bytes.
//!
//! A copy belongs to a plate, or to none, and is kept in that plate's own
//! coordinates, so it goes with its plate when the plates are laid out
//! anew: for another bed, or for a plate more or fewer. What is on no
//! plate is not sliced. The edits take where things are to stand in the
//! scene's coordinates, in which the first plate's origin is (0, 0) and
//! the others stand in the reference's grid for a [`Bed`].
//!
//! Nothing here knows of a window; a `Model` is cheap to copy (the meshes
//! are shared), which is what makes undoing an edit cheap for whoever
//! keeps the copies.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use edge_3mf::{Instance, InstanceRef, Object, Part, PartKind, Plate, Project};
use edge_config::Settings;
use edge_mesh::Transform;

use crate::{
    Failure, Loaded, PlateInfo,
    load::{self, Held, Opened, plate_origin},
    scene::{SceneObject, object_of},
};

/// The most plates a model may have (PartPlate.hpp `MAX_PLATES_COUNT`).
pub const MAX_PLATES: usize = 36;

/// How far under the bed a copy's lowest point may be and still count as
/// standing on it (Model.hpp `SINKING_Z_THRESHOLD`).
const SINKING: f64 = -0.001;

/// The room left between objects that are put somewhere for the person:
/// what is added, cloned or arranged.
const APART: f64 = 6.0;

/// Numbers that tell objects, copies and shapes apart for whoever holds a
/// model. They are never part of what is sliced.
static NEXT: AtomicU64 = AtomicU64::new(1);

fn fresh() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The least and greatest corners of something, in mm.
pub type Reach = [[f64; 3]; 2];

/// The bed the plates are laid out for: the box round the printer's
/// printable area, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bed {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

impl Bed {
    /// From the least and greatest x and y: `[min_x, min_y, max_x, max_y]`.
    pub fn of(extent: [f64; 4]) -> Self {
        Self {
            min: [extent[0], extent[1]],
            max: [extent[2], extent[3]],
        }
    }

    /// Whole millimetres, as the reference takes a bed's size.
    fn size(&self) -> (f64, f64) {
        (
            (self.max[0] - self.min[0]).trunc(),
            (self.max[1] - self.min[1]).trunc(),
        )
    }

    /// Where plate `index` of `count` stands in the scene.
    pub fn origin(&self, index: usize, count: usize) -> (f64, f64) {
        let (width, depth) = self.size();
        crate::plate_position(index, count.max(1), width, depth)
    }

    /// Where what stood on a plate that is taken away is put: beside the
    /// plates that are left (PartPlate.cpp `compute_origin_for_unprintable`).
    fn aside(&self, count: usize) -> (f64, f64) {
        let columns = ((count as f64).sqrt().ceil() as usize).max(1);
        let (index, columns) = if count == columns * columns {
            (count + columns - 1, columns + 1)
        } else {
            (count, columns)
        };
        let (width, depth) = self.size();
        let (row, column) = (index / columns, index % columns);
        (column as f64 * width * 1.2, -(row as f64) * depth * 1.2)
    }

    /// The bed of the plate that stands at `origin`, as a box on the ground.
    fn at(&self, origin: (f64, f64)) -> [f64; 4] {
        [
            self.min[0] + origin.0,
            self.min[1] + origin.1,
            self.max[0] + origin.0,
            self.max[1] + origin.1,
        ]
    }
}

/// Whether a box and a bed's box share any ground.
fn overlaps(reach: &Reach, bed: [f64; 4]) -> bool {
    reach[0][0] <= bed[2] && reach[1][0] >= bed[0] && reach[0][1] <= bed[3] && reach[1][1] >= bed[1]
}

/// Whether a box lies within a bed's box.
fn within(reach: &Reach, bed: [f64; 4]) -> bool {
    reach[0][0] >= bed[0] && reach[1][0] <= bed[2] && reach[0][1] >= bed[1] && reach[1][1] <= bed[3]
}

/// An object's geometry: its parts, each with what it is and the filament
/// it prints in. Shared by every copy of the model that has not changed it.
#[derive(Debug)]
pub struct Shape {
    stamp: u64,
    parts: Vec<Part>,
}

impl Shape {
    fn new(parts: Vec<Part>) -> Arc<Self> {
        Arc::new(Self {
            stamp: fresh(),
            parts,
        })
    }

    /// A number that is this shape's alone: another shape, or this one
    /// with another filament, has another.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// The shape as one mesh to look at, in the object's own coordinates,
    /// with what each triangle is. This is the costly part of showing a
    /// model; it is the same for as long as [`Shape::stamp`] is.
    pub fn mesh(&self, name: &str) -> SceneObject {
        object_of(&Object {
            id: 0,
            name: Some(name.to_owned()),
            parts: self.parts.clone(),
            instances: Vec::new(),
        })
    }

    /// The filaments its printed parts are set to, in ascending order.
    /// Colours painted onto it are not counted.
    pub fn filaments(&self) -> Vec<usize> {
        let mut used: Vec<usize> = self.printed().map(|part| part.filament).collect();
        used.sort_unstable();
        used.dedup();
        used
    }

    fn printed(&self) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(|part| part.kind == PartKind::Normal)
    }

    /// The corners of its printed material where `transform` puts it.
    fn reach(&self, transform: &Transform) -> Option<Reach> {
        let mut reach: Option<Reach> = None;
        for part in self.printed() {
            let placed = part.transform.then(transform);
            for vertex in &part.mesh.vertices {
                let point = placed.apply(vertex.map(f64::from));
                let [low, high] = reach.get_or_insert([point, point]);
                for axis in 0..3 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
        }
        reach
    }
}

/// One object of a model.
#[derive(Clone, Debug)]
pub struct ModelObject {
    id: u64,
    name: String,
    shape: Arc<Shape>,
    copies: Vec<ModelCopy>,
}

impl ModelObject {
    /// A number that stays this object's through every edit.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The name it goes by; empty when it has none.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn shape(&self) -> &Arc<Shape> {
        &self.shape
    }

    pub fn copies(&self) -> &[ModelCopy] {
        &self.copies
    }

    /// How many parts it is made of.
    pub fn parts(&self) -> usize {
        self.shape.parts.len()
    }
}

/// One copy of an object (the reference's instance): where it stands.
#[derive(Clone, Debug)]
pub struct ModelCopy {
    id: u64,
    /// Takes the object's coordinates to the project's, in which its
    /// plate's origin is [`ModelPlate::origin`]. This is what is sliced.
    world: Transform,
    /// The same measured from its plate's origin; for a copy on no plate,
    /// from the first plate's.
    local: Transform,
    /// The corners of its printed material under `local`.
    reach: Option<Reach>,
    printable: bool,
}

impl ModelCopy {
    /// A number that stays this copy's through every edit.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn printable(&self) -> bool {
        self.printable
    }

    /// Takes the object's coordinates to those of the copy's plate, whose
    /// origin is the bed's (0, 0); for a copy on no plate, to the scene's.
    pub fn transform(&self) -> Transform {
        self.local
    }

    /// The corners of its printed material in the coordinates of
    /// [`ModelCopy::transform`]; none when it has no printed material.
    pub fn reach(&self) -> Option<Reach> {
        self.reach
    }
}

/// One plate of a model.
#[derive(Clone, Debug)]
pub struct ModelPlate {
    id: u32,
    name: String,
    first_layer_sequence: Vec<u32>,
    /// Where the plate's origin is in the coordinates its copies are kept
    /// in: where the file has it for a plate that was read, which is what
    /// lets an untouched plate slice to the bytes its file does. None when
    /// the file does not say how large its bed is.
    origin: Option<(f64, f64)>,
    /// The copies on it, in the order the plate lists them.
    order: Vec<u64>,
}

impl ModelPlate {
    /// The plate's number, from 1.
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The copies on it, by [`ModelCopy::id`], in the plate's own order.
    pub fn copies(&self) -> &[u64] {
        &self.order
    }
}

/// What a plate's slice depends on: equal for two models exactly when the
/// plate would slice alike in both.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PlateStamp {
    /// Where the plate is among the others, and its first layer's order.
    index: usize,
    first_layer_sequence: Vec<u32>,
    origin: Option<[u64; 2]>,
    /// Each printable copy on it: its shape and where it stands.
    copies: Vec<(u64, [u64; 12])>,
    /// A bare model, which slicing puts in the middle of the bed.
    lone: bool,
}

/// Why an edit was not made.
fn refused(reason: impl Into<String>) -> Failure {
    reason.into().into()
}

/// A model in memory: see the module's own words.
#[derive(Clone, Debug)]
pub struct Model {
    /// What it was read from; none for one that was put together here.
    path: Option<PathBuf>,
    objects: Vec<ModelObject>,
    plates: Vec<ModelPlate>,
    settings: Option<Arc<Settings>>,
    unread_paint: Vec<&'static str>,
    /// A bare model as it was read: one mesh with no place on a bed.
    /// Slicing puts it in the middle, standing on the bed.
    bare: bool,
    /// Whether anything in it was put where it stands by hand.
    placed: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    /// A model with nothing in it, to add to.
    pub fn new() -> Self {
        Self {
            path: None,
            objects: Vec::new(),
            plates: Vec::new(),
            settings: None,
            unread_paint: Vec::new(),
            bare: false,
            placed: true,
        }
    }

    /// Reads an STL (binary or ASCII) or a 3MF project.
    pub fn open(path: &Path) -> Result<Self, Failure> {
        Ok(Self::from(crate::open(path, None)?))
    }

    fn named(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| "the project".into())
    }

    // ---- what is in it ----

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn objects(&self) -> &[ModelObject] {
        &self.objects
    }

    pub fn plates(&self) -> &[ModelPlate] {
        &self.plates
    }

    /// The settings a project was saved with.
    pub fn settings(&self) -> Option<&Arc<Settings>> {
        self.settings.as_ref()
    }

    /// A bare model that nothing has been done to: it has no place on a
    /// bed of its own, and slicing puts it in the middle.
    pub fn bare(&self) -> bool {
        self.bare
    }

    /// Whether anything in it stands where it was put by hand, so that
    /// slicing leaves all of it where it is.
    pub fn placed(&self) -> bool {
        self.placed
    }

    /// Where a copy is: its object's and its own position in the lists.
    fn find(&self, copy: u64) -> Option<(usize, usize)> {
        self.objects.iter().enumerate().find_map(|(at, object)| {
            let copy = object.copies.iter().position(|held| held.id == copy)?;
            Some((at, copy))
        })
    }

    fn object_at(&self, id: u64) -> Result<usize, Failure> {
        self.objects
            .iter()
            .position(|object| object.id == id)
            .ok_or_else(|| refused(format!("there is no object {id}")))
    }

    fn copy_at(&self, id: u64) -> Result<(usize, usize), Failure> {
        self.find(id)
            .ok_or_else(|| refused(format!("there is no copy {id}")))
    }

    /// The plate a copy is on, as a position in [`Model::plates`].
    pub fn plate_of(&self, copy: u64) -> Option<usize> {
        self.plates
            .iter()
            .position(|plate| plate.order.contains(&copy))
    }

    fn plate_index(&self, id: u32) -> Option<usize> {
        self.plates.iter().position(|plate| plate.id == id)
    }

    /// Every plate in order with what is on it, as [`Opened::plate_list`]
    /// lists a file's.
    pub fn plate_list(&self) -> Vec<PlateInfo> {
        self.plates
            .iter()
            .map(|plate| {
                let on: Vec<(&ModelObject, &ModelCopy)> = plate
                    .order
                    .iter()
                    .filter_map(|copy| self.find(*copy))
                    .map(|(object, copy)| {
                        (&self.objects[object], &self.objects[object].copies[copy])
                    })
                    .filter(|(_, copy)| copy.printable)
                    .collect();
                PlateInfo {
                    id: plate.id,
                    name: plate.name.clone(),
                    filaments: filaments_of(on.iter().map(|(object, _)| &*object.shape)),
                    objects: on.iter().map(|(object, _)| object.name.clone()).collect(),
                }
            })
            .collect()
    }

    /// The filaments everything printable in it prints in, by number from
    /// 1, in order: on its plates when it has plates, else all of it.
    pub fn filaments(&self) -> Vec<usize> {
        if self.bare {
            return vec![1];
        }
        let on_a_plate = |copy: &ModelCopy| {
            self.plates.is_empty()
                || self
                    .plates
                    .iter()
                    .any(|plate| plate.order.contains(&copy.id))
        };
        filaments_of(
            self.objects
                .iter()
                .filter(|object| {
                    object
                        .copies
                        .iter()
                        .any(|copy| copy.printable && on_a_plate(copy))
                })
                .map(|object| &*object.shape),
        )
    }

    /// What the slice of a plate depends on; of the whole model for one
    /// without plates.
    pub fn stamp(&self, plate: Option<u32>) -> PlateStamp {
        let bits = |transform: &Transform| -> [u64; 12] {
            std::array::from_fn(|at| transform.rows[at / 3][at % 3].to_bits())
        };
        let entry = |(object, copy): (usize, usize)| {
            let (object, copy) = (&self.objects[object], &self.objects[object].copies[copy]);
            copy.printable
                .then(|| (object.shape.stamp, bits(&copy.world)))
        };
        match plate.and_then(|id| self.plate_index(id)) {
            Some(index) => {
                let plate = &self.plates[index];
                PlateStamp {
                    index,
                    first_layer_sequence: plate.first_layer_sequence.clone(),
                    origin: plate.origin.map(|(x, y)| [x.to_bits(), y.to_bits()]),
                    copies: plate
                        .order
                        .iter()
                        .filter_map(|copy| self.find(*copy))
                        .filter_map(entry)
                        .collect(),
                    lone: false,
                }
            }
            None => PlateStamp {
                copies: self
                    .objects
                    .iter()
                    .enumerate()
                    .flat_map(|(at, object)| (0..object.copies.len()).map(move |copy| (at, copy)))
                    .filter_map(entry)
                    .collect(),
                lone: self.bare,
                ..PlateStamp::default()
            },
        }
    }

    // ---- slicing ----

    /// The model as a project of the file's kind, each copy where it is
    /// kept, for the code that gathers what a plate prints.
    fn project(&self) -> Project {
        Project {
            objects: self
                .objects
                .iter()
                .enumerate()
                .map(|(at, object)| Object {
                    id: at as u32 + 1,
                    name: Some(object.name.clone()).filter(|name| !name.is_empty()),
                    parts: object.shape.parts.clone(),
                    instances: object
                        .copies
                        .iter()
                        .map(|copy| Instance {
                            transform: copy.world,
                            printable: copy.printable,
                        })
                        .collect(),
                })
                .collect(),
            plates: self
                .plates
                .iter()
                .map(|plate| Plate {
                    id: plate.id,
                    name: plate.name.clone(),
                    first_layer_sequence: plate.first_layer_sequence.clone(),
                    instances: plate
                        .order
                        .iter()
                        .filter_map(|copy| self.find(*copy))
                        .map(|(object, instance)| InstanceRef { object, instance })
                        .collect(),
                })
                .collect(),
            settings: None,
            unread_paint: self.unread_paint.clone(),
        }
    }

    /// Everything on a plate, where it sits on the bed: what
    /// [`crate::prepare`] takes. `plate` is the plate's number; left out,
    /// it is plate 1 when the model has plates, as [`crate::open`] has it.
    pub fn place(&self, plate: Option<u32>) -> Result<Loaded, Failure> {
        let path = self.named();
        if self.bare {
            let mesh = self.objects[0].shape.parts[0].mesh.as_ref().clone();
            return load::lone(&path, mesh);
        }
        let index = match plate {
            Some(id) => Some(self.plate_index(id).ok_or_else(|| {
                let ids: Vec<String> = self
                    .plates
                    .iter()
                    .map(|plate| plate.id.to_string())
                    .collect();
                match ids.as_slice() {
                    [] => format!("{}: no plate {id}; it has no plates", path.display()),
                    _ => format!(
                        "{}: no plate {id}; it has plates {}",
                        path.display(),
                        ids.join(", ")
                    ),
                }
            })?),
            None => self.plate_index(1),
        };
        let origin = index
            .map(|index| {
                let plate = &self.plates[index];
                plate.origin.ok_or_else(|| load::unplaced(&path, plate.id))
            })
            .transpose()?;
        let settings = self.settings.as_deref().cloned();
        let mut loaded = load::placed(&path, &self.project(), index, origin, settings)?;
        loaded.placed = self.placed;
        Ok(loaded)
    }

    // ---- edits ----

    /// Sets where a copy stands, in its plate's own coordinates.
    fn stand(&mut self, at: (usize, usize), plate: Option<usize>, local: Transform) {
        let origin = plate
            .and_then(|plate| self.plates[plate].origin)
            .unwrap_or((0.0, 0.0));
        let object = &mut self.objects[at.0];
        let reach = object.shape.reach(&local);
        let copy = &mut object.copies[at.1];
        copy.local = local;
        copy.world = local.then(&Transform::translation([origin.0, origin.1, 0.0]));
        copy.reach = reach;
    }

    /// Puts a copy on a plate, or on none, leaving it where it stands in
    /// its coordinates.
    fn assign(&mut self, copy: u64, plate: Option<usize>) {
        for held in &mut self.plates {
            held.order.retain(|id| *id != copy);
        }
        if let Some(plate) = plate {
            self.plates[plate].order.push(copy);
        }
    }

    /// Where a plate stands in the scene; (0, 0) with no bed to lay the
    /// plates out for.
    fn standing(&self, bed: Option<&Bed>, plate: Option<usize>) -> (f64, f64) {
        match (bed, plate) {
            (Some(bed), Some(plate)) => bed.origin(plate, self.plates.len()),
            _ => (0.0, 0.0),
        }
    }

    /// Where a copy stands in the scene: its transform there and the
    /// corners of its printed material.
    pub fn in_scene(&self, bed: Option<&Bed>, copy: u64) -> Option<(Transform, Option<Reach>)> {
        let (object, at) = self.find(copy)?;
        let held = &self.objects[object].copies[at];
        let (x, y) = self.standing(bed, self.plate_of(copy));
        let moved = held.reach.map(|[low, high]| {
            [
                [low[0] + x, low[1] + y, low[2]],
                [high[0] + x, high[1] + y, high[2]],
            ]
        });
        Some((held.local.then(&Transform::translation([x, y, 0.0])), moved))
    }

    /// Whether a copy reaches over the edge of the plate it is on: such a
    /// plate cannot be printed until the copy is moved wholly on or off it.
    pub fn overhangs(&self, bed: Option<&Bed>, copy: u64) -> bool {
        let (Some(bed), Some(_), Some((object, at))) = (bed, self.plate_of(copy), self.find(copy))
        else {
            return false;
        };
        self.objects[object].copies[at]
            .reach
            .is_some_and(|reach| !within(&reach, bed.at((0.0, 0.0))))
    }

    /// Makes of a model as it was read one that can be edited: a bare
    /// model is put where slicing would put it, in the middle of the bed
    /// and standing on it, and a model without plates is given one that
    /// holds all of it. Every edit starts with this.
    fn settle(&mut self, bed: Option<&Bed>) {
        if self.bare {
            self.bare = false;
            let at = (0, 0);
            let reach = self.objects[0].shape.reach(&Transform::IDENTITY);
            let offset = match (reach, bed) {
                (Some([low, high]), Some(bed)) => [
                    (bed.min[0] + bed.max[0]) / 2.0 - (low[0] + high[0]) / 2.0,
                    (bed.min[1] + bed.max[1]) / 2.0 - (low[1] + high[1]) / 2.0,
                    -low[2],
                ],
                (Some([low, _]), None) => [0.0, 0.0, -low[2]],
                _ => [0.0; 3],
            };
            self.stand(at, None, Transform::translation(offset));
        }
        self.placed = true;
        if self.plates.is_empty() {
            self.plates.push(ModelPlate {
                id: 1,
                name: String::new(),
                first_layer_sequence: Vec::new(),
                origin: Some((0.0, 0.0)),
                order: self
                    .objects
                    .iter()
                    .flat_map(|object| object.copies.iter().map(|copy| copy.id))
                    .collect(),
            });
        }
    }

    /// Stands a copy at `scene`, which takes its object's coordinates to
    /// the scene's, and settles which plate it is then on, as the
    /// reference does when an instance has moved (PartPlate.cpp
    /// `notify_instance_update`): it stays on its plate while it reaches
    /// over any of it, else it goes to the first plate it reaches over,
    /// else to none.
    ///
    /// It is then stood on the bed, as the reference stands what was
    /// turned or sized (GLCanvas3D.cpp `do_rotate`, `do_scale`): a copy
    /// that stood on the bed before stands on it after, and one left in
    /// the air is brought down. Only a copy that reached under the bed
    /// before and still does is left sunk, as it was read.
    fn stand_in_scene(&mut self, bed: Option<&Bed>, copy: u64, scene: Transform) {
        let Some(at) = self.find(copy) else { return };
        let shape = self.objects[at.0].shape.clone();
        let stood = self.objects[at.0].copies[at.1]
            .reach
            .is_none_or(|[low, _]| low[2] >= SINKING);
        let mut scene = scene;
        let mut reach = shape.reach(&scene);
        if let Some([low, _]) = reach
            && (stood || low[2] > SINKING)
            && low[2] != 0.0
        {
            scene = scene.then(&Transform::translation([0.0, 0.0, -low[2]]));
            reach = shape.reach(&scene);
        }
        let was = self.plate_of(copy);
        let count = self.plates.len();
        let on = |plate: usize| match (bed, &reach) {
            (Some(bed), Some(reach)) => overlaps(reach, bed.at(bed.origin(plate, count))),
            // With no bed to measure by, or nothing to measure, a copy
            // stays where it is.
            _ => Some(plate) == was,
        };
        let plate = was
            .filter(|plate| on(*plate))
            .or_else(|| (0..count).find(|plate| on(*plate)));
        let (x, y) = self.standing(bed, plate);
        if plate != was {
            self.assign(copy, plate);
        }
        self.stand(
            at,
            plate,
            scene.then(&Transform::translation([-x, -y, 0.0])),
        );
    }

    /// Stands each copy named where its transform puts it in the scene.
    pub fn set_transforms(
        &mut self,
        bed: Option<&Bed>,
        to: &[(u64, Transform)],
    ) -> Result<(), Failure> {
        for (copy, transform) in to {
            self.copy_at(*copy)?;
            let sound = transform
                .rows
                .iter()
                .flatten()
                .all(|value| value.is_finite())
                && transform.determinant().abs() > 1e-12;
            if !sound {
                return Err(refused("that would squash the object to nothing"));
            }
        }
        self.settle(bed);
        for (copy, transform) in to {
            self.stand_in_scene(bed, *copy, *transform);
        }
        Ok(())
    }

    /// Takes copies away. An object whose last copy goes, goes with it.
    pub fn remove(&mut self, bed: Option<&Bed>, copies: &[u64]) -> Result<(), Failure> {
        for copy in copies {
            self.copy_at(*copy)?;
        }
        self.settle(bed);
        for plate in &mut self.plates {
            plate.order.retain(|copy| !copies.contains(copy));
        }
        for object in &mut self.objects {
            object.copies.retain(|copy| !copies.contains(&copy.id));
        }
        self.objects.retain(|object| !object.copies.is_empty());
        Ok(())
    }

    /// Gives an object another name.
    pub fn rename(&mut self, bed: Option<&Bed>, object: u64, name: &str) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(refused("The name cannot be empty."));
        }
        self.settle(bed);
        self.objects[at].name = name.to_owned();
        Ok(())
    }

    /// Says whether copies are to be printed.
    pub fn set_printable(
        &mut self,
        bed: Option<&Bed>,
        copies: &[u64],
        printable: bool,
    ) -> Result<(), Failure> {
        let found = copies
            .iter()
            .map(|copy| self.copy_at(*copy))
            .collect::<Result<Vec<_>, _>>()?;
        self.settle(bed);
        for (object, copy) in found {
            self.objects[object].copies[copy].printable = printable;
        }
        Ok(())
    }

    /// Sets the filament an object prints in, counting from 1: every
    /// printed part of it takes it, as the reference clears the parts' own
    /// choice when the object's is set. What is painted on it stays.
    pub fn set_filament(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        filament: usize,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        if filament == 0 {
            return Err(refused("filaments are counted from 1"));
        }
        self.settle(bed);
        let mut parts = self.objects[at].shape.parts.clone();
        for part in &mut parts {
            if part.kind == PartKind::Normal {
                part.filament = filament;
            }
        }
        self.objects[at].shape = Shape::new(parts);
        Ok(())
    }

    // ---- plates ----

    /// Puts each copy that is on no plate onto the first plate it reaches
    /// over, as the reference gathers what a new plate comes to stand
    /// under (PartPlate.cpp `construct_objects_list_for_new_plate`).
    fn gather(&mut self, bed: Option<&Bed>) {
        let Some(bed) = bed else { return };
        let count = self.plates.len();
        let loose: Vec<(u64, (usize, usize))> = self
            .objects
            .iter()
            .enumerate()
            .flat_map(|(at, object)| {
                object
                    .copies
                    .iter()
                    .enumerate()
                    .map(move |(copy, held)| (held.id, (at, copy)))
            })
            .filter(|(id, _)| self.plate_of(*id).is_none())
            .collect();
        for (id, at) in loose {
            let held = &self.objects[at.0].copies[at.1];
            let Some(reach) = held.reach else { continue };
            let Some(plate) =
                (0..count).find(|plate| overlaps(&reach, bed.at(bed.origin(*plate, count))))
            else {
                continue;
            };
            let (x, y) = bed.origin(plate, count);
            let local = held.local.then(&Transform::translation([-x, -y, 0.0]));
            self.assign(id, Some(plate));
            self.stand(at, Some(plate), local);
        }
    }

    /// Adds an empty plate after the last and answers its number.
    pub fn add_plate(&mut self, bed: Option<&Bed>) -> Result<u32, Failure> {
        if self.plates.len().max(1) >= MAX_PLATES {
            return Err(refused(format!(
                "a project has at most {MAX_PLATES} plates"
            )));
        }
        self.settle(bed);
        let id = self.plates.iter().map(|plate| plate.id).max().unwrap_or(0) + 1;
        self.plates.push(ModelPlate {
            id,
            name: String::new(),
            first_layer_sequence: Vec::new(),
            origin: Some((0.0, 0.0)),
            order: Vec::new(),
        });
        self.gather(bed);
        Ok(id)
    }

    /// Takes a plate away, unless it is the only one. What stood on it is
    /// not deleted: it is put beside the plates that are left, on no
    /// plate, and the plates after it are numbered anew (PartPlate.cpp
    /// `PartPlateList::delete_plate`).
    pub fn remove_plate(&mut self, bed: Option<&Bed>, plate: u32) -> Result<(), Failure> {
        self.settle(bed);
        let index = self
            .plate_index(plate)
            .ok_or_else(|| refused(format!("there is no plate {plate}")))?;
        if self.plates.len() <= 1 {
            return Err(refused("the last plate cannot be removed"));
        }
        let gone = self.plates.remove(index);
        let (x, y) = bed.map_or((0.0, 0.0), |bed| bed.aside(self.plates.len()));
        for copy in gone.order {
            if let Some(at) = self.find(copy) {
                let local = self.objects[at.0].copies[at.1].local;
                self.stand(at, None, local.then(&Transform::translation([x, y, 0.0])));
            }
        }
        for (at, plate) in self.plates.iter_mut().enumerate() {
            plate.id = at as u32 + 1;
        }
        self.gather(bed);
        Ok(())
    }

    // ---- putting things somewhere ----

    /// The ground each copy on a plate takes, in the plate's coordinates.
    fn taken(&self, plate: usize) -> Vec<[f64; 4]> {
        self.plates[plate]
            .order
            .iter()
            .filter_map(|copy| self.find(*copy))
            .filter_map(|(object, copy)| self.objects[object].copies[copy].reach)
            .map(|[low, high]| [low[0], low[1], high[0], high[1]])
            .collect()
    }

    /// Stands a new copy of a shape on a plate, turned and sized by the
    /// rows of `turned`, at the free place nearest `near`; beside the
    /// plates when the plate has no room for it. `seated` is the ground
    /// taken by copies that are not in the model yet, to which this one's
    /// is added. Answers the copy, for its object to hold.
    fn seat(
        &mut self,
        (bed, plate): (Option<&Bed>, usize),
        shape: &Arc<Shape>,
        turned: Transform,
        near: Option<(f64, f64)>,
        printable: bool,
        seated: &mut Vec<[f64; 4]>,
    ) -> ModelCopy {
        let mut turned = turned;
        turned.rows[3] = [0.0; 3];
        let reach = shape.reach(&turned);
        let [low, high] = reach.unwrap_or([[0.0; 3]; 2]);
        let size = (high[0] - low[0], high[1] - low[1]);
        let middle = bed.map_or((0.0, 0.0), |bed| {
            (
                (bed.min[0] + bed.max[0]) / 2.0,
                (bed.min[1] + bed.max[1]) / 2.0,
            )
        });
        let mut taken = self.taken(plate);
        taken.extend(seated.iter().copied());
        let spot = bed.and_then(|bed| free_place(bed, &taken, size, near.unwrap_or(middle)));
        let (on, centre) = match (spot, bed) {
            (Some(spot), _) => (Some(plate), spot),
            // No room: beside the plates, on none of them.
            (None, Some(bed)) => {
                let (x, y) = bed.aside(self.plates.len());
                (None, (x + middle.0, y + middle.1))
            }
            (None, None) => (Some(plate), middle),
        };
        let offset = [
            centre.0 - (low[0] + high[0]) / 2.0,
            centre.1 - (low[1] + high[1]) / 2.0,
            -low[2],
        ];
        let local = turned.then(&Transform::translation(offset));
        let origin = on
            .and_then(|plate| self.plates[plate].origin)
            .unwrap_or((0.0, 0.0));
        let copy = ModelCopy {
            id: fresh(),
            world: local.then(&Transform::translation([origin.0, origin.1, 0.0])),
            local,
            reach: shape.reach(&local),
            printable,
        };
        if let Some(plate) = on {
            self.plates[plate].order.push(copy.id);
            if let Some([low, high]) = copy.reach {
                seated.push([low[0], low[1], high[0], high[1]]);
            }
        }
        copy
    }

    /// Adds everything in `other` to a plate (the first when none is
    /// named): its objects with their copies, each turned and sized as it
    /// was, standing on the bed at the free place nearest the middle. This
    /// is the reference's "Import geometry only": the other model's plates
    /// and settings are left behind. Answers the copies added.
    ///
    /// The reference runs its arranger over what it adds; this takes the
    /// nearest free place, by the boxes round the copies.
    pub fn import(
        &mut self,
        bed: Option<&Bed>,
        other: &Model,
        plate: Option<u32>,
    ) -> Result<Vec<u64>, Failure> {
        let plate = match plate {
            Some(id) => Some(
                self.plate_index(id)
                    .ok_or_else(|| refused(format!("there is no plate {id}")))?,
            ),
            None => None,
        };
        if other.objects.is_empty() {
            return Err(refused("there is nothing in that file to add"));
        }
        self.settle(bed);
        let plate = plate.unwrap_or(0);
        let file = other
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut added = Vec::new();
        // The ground the copies added so far take, for the next to keep off.
        let mut seated = Vec::new();
        for object in &other.objects {
            let copies: Vec<ModelCopy> = object
                .copies
                .iter()
                .map(|copy| {
                    self.seat(
                        (bed, plate),
                        &object.shape,
                        copy.local,
                        None,
                        copy.printable,
                        &mut seated,
                    )
                })
                .collect();
            added.extend(copies.iter().map(|copy| copy.id));
            self.objects.push(ModelObject {
                id: fresh(),
                name: if object.name.is_empty() {
                    file.clone()
                } else {
                    object.name.clone()
                },
                shape: object.shape.clone(),
                copies,
            });
        }
        self.unread_paint = {
            let mut kinds = self.unread_paint.clone();
            kinds.extend(other.unread_paint.iter().copied());
            kinds.sort_unstable();
            kinds.dedup();
            kinds
        };
        Ok(added)
    }

    /// Makes a new object of each copy named, alike in shape, name, turn
    /// and size, standing at the free place nearest the copy it was made
    /// of, on `plate` or else on that copy's own plate (the reference's
    /// "Clone", and its copy and paste, which paste objects). Answers the
    /// new copies.
    pub fn clone_copies(
        &mut self,
        bed: Option<&Bed>,
        copies: &[u64],
        plate: Option<u32>,
    ) -> Result<Vec<u64>, Failure> {
        for copy in copies {
            self.copy_at(*copy)?;
        }
        let onto = match plate {
            Some(id) => Some(
                self.plate_index(id)
                    .ok_or_else(|| refused(format!("there is no plate {id}")))?,
            ),
            None => None,
        };
        self.settle(bed);
        let mut added = Vec::new();
        for copy in copies {
            let (object, at) = self.copy_at(*copy)?;
            let (name, shape) = (
                self.objects[object].name.clone(),
                self.objects[object].shape.clone(),
            );
            let held = self.objects[object].copies[at].clone();
            let own = self.plate_of(*copy);
            let plate = onto.or(own).unwrap_or(0);
            // Beside the copy it is made of, when it goes on that copy's plate.
            let near = held
                .reach
                .filter(|_| own == Some(plate))
                .map(|[low, high]| ((low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0));
            let seated = self.seat(
                (bed, plate),
                &shape,
                held.local,
                near,
                held.printable,
                &mut Vec::new(),
            );
            added.push(seated.id);
            self.objects.push(ModelObject {
                id: fresh(),
                name,
                shape,
                copies: vec![seated],
            });
        }
        Ok(added)
    }

    /// Lays out what stands on a plate (on every plate for none named) so
    /// that nothing overlaps and everything is on the bed: in rows, the
    /// deepest copies first, [`APART`] mm apart, the whole set in the
    /// middle of the bed. Nothing is turned. What a plate has no room for
    /// is put beside the plates, on none of them. Answers how many copies
    /// that was.
    ///
    /// This is not the reference's arrangement (libslic3r/Arrange, which
    /// nests the outlines of the objects and may turn them); it is a plain
    /// one by the boxes round the copies.
    pub fn arrange(&mut self, bed: Option<&Bed>, plate: Option<u32>) -> Result<usize, Failure> {
        let bed =
            bed.ok_or_else(|| refused("no printer is chosen: there is no bed to arrange on"))?;
        let only = match plate {
            Some(id) => Some(
                self.plate_index(id)
                    .ok_or_else(|| refused(format!("there is no plate {id}")))?,
            ),
            None => None,
        };
        self.settle(Some(bed));
        let (width, depth) = (bed.max[0] - bed.min[0], bed.max[1] - bed.min[1]);
        let mut left_out = 0;
        for plate in 0..self.plates.len() {
            if only.is_some_and(|only| only != plate) {
                continue;
            }
            // The copies with the ground each takes, the deepest first; as
            // the plate lists them among equals.
            let mut boxes: Vec<(u64, f64, f64)> = self.plates[plate]
                .order
                .iter()
                .filter_map(|copy| {
                    let (object, at) = self.find(*copy)?;
                    let [low, high] = self.objects[object].copies[at].reach?;
                    Some((*copy, high[0] - low[0], high[1] - low[1]))
                })
                .collect();
            boxes.sort_by(|a, b| b.2.total_cmp(&a.2));
            // Rows from the front of the bed to the back.
            let mut rows: Vec<Row> = Vec::new();
            let mut over: Vec<u64> = Vec::new();
            let mut used = 0.0;
            for (copy, wide, deep) in boxes {
                if wide > width || deep > depth {
                    over.push(copy);
                    continue;
                }
                let fits = rows
                    .last()
                    .is_some_and(|(_, along, _)| along + APART + wide <= width);
                if fits {
                    let (row, along, _) = rows.last_mut().expect("there is a row");
                    row.push((copy, wide, deep));
                    *along += APART + wide;
                } else if used + if rows.is_empty() { 0.0 } else { APART } + deep <= depth {
                    used += if rows.is_empty() { 0.0 } else { APART } + deep;
                    rows.push((vec![(copy, wide, deep)], wide, deep));
                } else {
                    over.push(copy);
                }
            }
            let widest = rows.iter().map(|(_, along, _)| *along).fold(0.0, f64::max);
            let mut y = bed.min[1] + (depth - used) / 2.0;
            for (row, _, deep) in rows {
                let mut x = bed.min[0] + (width - widest) / 2.0;
                for (copy, wide, own) in row {
                    self.slide(copy, Some(plate), (x + wide / 2.0, y + own / 2.0));
                    x += wide + APART;
                }
                y += deep + APART;
            }
            left_out += over.len();
            let (ax, ay) = bed.aside(self.plates.len());
            for copy in over {
                self.assign(copy, None);
                self.slide(
                    copy,
                    None,
                    (
                        ax + (bed.min[0] + bed.max[0]) / 2.0,
                        ay + (bed.min[1] + bed.max[1]) / 2.0,
                    ),
                );
            }
        }
        Ok(left_out)
    }

    /// Moves a copy across the ground so that the middle of the ground it
    /// takes is at `centre`, in the coordinates of `plate`.
    fn slide(&mut self, copy: u64, plate: Option<usize>, centre: (f64, f64)) {
        let Some(at) = self.find(copy) else { return };
        let held = &self.objects[at.0].copies[at.1];
        let Some([low, high]) = held.reach else {
            return;
        };
        let by = [
            centre.0 - (low[0] + high[0]) / 2.0,
            centre.1 - (low[1] + high[1]) / 2.0,
            0.0,
        ];
        // Where it is already, it is left to the bit.
        if by[0].abs() < 1e-9 && by[1].abs() < 1e-9 && self.plate_of(copy) == plate {
            return;
        }
        let local = held.local.then(&Transform::translation(by));
        self.stand(at, plate, local);
    }
}

/// A row of an arrangement: the copies in it with the ground each takes
/// (how wide, how deep), how far along the row is filled, and how deep it is.
type Row = (Vec<(u64, f64, f64)>, f64, f64);

/// The middle of the free place on a bed nearest `near` for something
/// `size` mm wide and deep, [`APART`] mm from everything in `taken` (each
/// `[min_x, min_y, max_x, max_y]`); none when the bed has no such place.
/// Places are tried on a grid of 2 mm from the bed's corner, the nearest
/// first, so the answer is the same every time.
fn free_place(
    bed: &Bed,
    taken: &[[f64; 4]],
    size: (f64, f64),
    near: (f64, f64),
) -> Option<(f64, f64)> {
    const STEP: f64 = 2.0;
    let (half_x, half_y) = (size.0 / 2.0, size.1 / 2.0);
    let free = |x: f64, y: f64| {
        x - half_x >= bed.min[0]
            && x + half_x <= bed.max[0]
            && y - half_y >= bed.min[1]
            && y + half_y <= bed.max[1]
            && taken.iter().all(|held| {
                x + half_x + APART <= held[0]
                    || x - half_x - APART >= held[2]
                    || y + half_y + APART <= held[1]
                    || y - half_y - APART >= held[3]
            })
    };
    // Where it was asked for, if that is free.
    if free(near.0, near.1) {
        return Some(near);
    }
    let across = |low: f64, high: f64, half: f64| -> Vec<f64> {
        let mut places = Vec::new();
        let mut at = low + half;
        while at + half <= high {
            places.push(at);
            at += STEP;
        }
        places
    };
    let mut best: Option<(f64, f64, f64)> = None;
    for y in across(bed.min[1], bed.max[1], half_y) {
        for x in across(bed.min[0], bed.max[0], half_x) {
            let far = (x - near.0).powi(2) + (y - near.1).powi(2);
            if best.is_none_or(|(least, ..)| far < least) && free(x, y) {
                best = Some((far, x, y));
            }
        }
    }
    best.map(|(_, x, y)| (x, y))
}

/// The filaments some shapes print in, counting the colours painted on
/// them, by number from 1, in order.
fn filaments_of<'a>(shapes: impl Iterator<Item = &'a Shape>) -> Vec<usize> {
    let mut used: Vec<usize> = Vec::new();
    for shape in shapes {
        for part in shape.printed() {
            used.push(part.filament);
            if let Some(paint) = &part.paint {
                used.extend(
                    paint
                        .iter()
                        .map(|facet| usize::from(facet.filament))
                        .filter(|filament| *filament > 0),
                );
            }
        }
    }
    used.sort_unstable();
    used.dedup();
    used
}

impl From<Opened> for Model {
    /// A file that was read, as a model to change. Which plate the file
    /// was opened for does not count: a model holds all of them.
    fn from(opened: Opened) -> Self {
        let path = Some(opened.path().to_owned());
        match opened.held {
            Held::Model(mesh) => {
                let shape = Shape::new(vec![Part {
                    name: None,
                    kind: PartKind::Normal,
                    filament: 1,
                    mesh: Arc::new(mesh),
                    paint: None,
                    seam: None,
                    transform: Transform::IDENTITY,
                    decoration: false,
                }]);
                let reach = shape.reach(&Transform::IDENTITY);
                Self {
                    path,
                    objects: vec![ModelObject {
                        id: fresh(),
                        name: String::new(),
                        shape,
                        copies: vec![ModelCopy {
                            id: fresh(),
                            world: Transform::IDENTITY,
                            local: Transform::IDENTITY,
                            reach,
                            printable: true,
                        }],
                    }],
                    plates: Vec::new(),
                    settings: None,
                    unread_paint: Vec::new(),
                    bare: true,
                    placed: false,
                }
            }
            Held::Project { project, .. } => {
                let project = *project;
                let count = project.plates.len();
                let origins: Vec<Option<(f64, f64)>> = (0..count)
                    .map(|index| plate_origin(index, count, project.settings.as_ref()))
                    .collect();
                let on = |object: usize, instance: usize| {
                    project.plates.iter().position(|plate| {
                        plate
                            .instances
                            .iter()
                            .any(|at| at.object == object && at.instance == instance)
                    })
                };
                let objects: Vec<ModelObject> = project
                    .objects
                    .iter()
                    .enumerate()
                    .map(|(at, object)| {
                        let shape = Shape::new(object.parts.clone());
                        let copies = object
                            .instances
                            .iter()
                            .enumerate()
                            .map(|(instance, held)| {
                                // Measured from its plate's origin, as
                                // slicing measures it. A plate that cannot
                                // be placed leaves the copy where the file
                                // has it.
                                let (x, y) = on(at, instance)
                                    .and_then(|plate| origins[plate])
                                    .unwrap_or((0.0, 0.0));
                                let local =
                                    held.transform.then(&Transform::translation([-x, -y, 0.0]));
                                ModelCopy {
                                    id: fresh(),
                                    world: held.transform,
                                    local,
                                    reach: shape.reach(&local),
                                    printable: held.printable,
                                }
                            })
                            .collect();
                        ModelObject {
                            id: fresh(),
                            name: object.name.clone().unwrap_or_default(),
                            shape,
                            copies,
                        }
                    })
                    .collect();
                let plates = project
                    .plates
                    .iter()
                    .zip(origins)
                    .map(|(plate, origin)| ModelPlate {
                        id: plate.id,
                        name: plate.name.clone(),
                        first_layer_sequence: plate.first_layer_sequence.clone(),
                        origin,
                        order: plate
                            .instances
                            .iter()
                            .map(|at| objects[at.object].copies[at.instance].id)
                            .collect(),
                    })
                    .collect();
                Self {
                    path,
                    objects,
                    plates,
                    settings: project.settings.map(Arc::new),
                    unread_paint: project.unread_paint,
                    bare: false,
                    placed: false,
                }
            }
        }
    }
}
