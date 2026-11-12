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
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use edge_3mf::{
    CustomGcode, Instance, InstanceRef, Kept, Object, ObjectKept, PaintKind, Part, PartKind, Plate,
    PlateKept, Project,
};
use edge_config::Settings;
use edge_mesh::{Transform, hull::Hull};

#[path = "save.rs"]
mod save;
pub use save::{Changed, Saved, Saving, own_settings, project_settings};

#[path = "plates.rs"]
mod plates;

#[path = "filaments.rs"]
mod filaments;

#[path = "item_settings.rs"]
mod item_settings;

#[path = "arrange.rs"]
mod arrange;

#[path = "cut.rs"]
mod cut;
pub use arrange::{ArrangeOptions, ArrangeScope, Arranged};
pub use cut::{
    CONNECTOR_HOLE_OVERLAP, CUT_THICKNESS_MAX, Connector, ConnectorKind, ConnectorShape,
    ConnectorStyle, CutDone, CutEdit, CutOptions, CutPieces, CutSection, CutThickness, FaceHit,
    Groove, Prepared, connector_mesh,
};
pub use filaments::{FlushBasis, MAX_FILAMENTS, NEXT_COLOURS};
pub use item_settings::SettingChange;
pub use plates::{FIRST_LATER_LAYER, LAST_LAYER, LONGEST_PLATE_NAME, LayerSequence, PlateSettings};

use crate::{
    Failure, Loaded, PlateInfo,
    lay_on_face::{self, Face, Filter},
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
    /// What it looks like: shared with the shape it was made from when
    /// only what is painted on it for the slice changed (supports, seams,
    /// fuzzy skin), which is not drawn with it, so that its mesh to look
    /// at need not be made and sent again.
    look: Arc<Look>,
    parts: Vec<Part>,
    /// A number for each part that stays the part's through every edit
    /// that leaves it there, in the order of `parts`.
    ids: Vec<u64>,
    /// The convex hull of its printed parts, worked out when first asked.
    hull: OnceLock<Option<Hull>>,
    /// Whether each part's mesh is of several pieces, worked out when
    /// first asked.
    pieces: OnceLock<Vec<bool>>,
    /// A tree of boxes round each printed part's triangles, for what a
    /// line of sight meets ("Cut"'s "Pick face"), made when first asked.
    trees: OnceLock<Vec<Option<edge_mesh::raycast::Tree>>>,
}

impl Shape {
    fn new(parts: Vec<Part>) -> Arc<Self> {
        let ids = parts.iter().map(|_| fresh()).collect();
        Self::keeping(parts, ids)
    }

    /// A shape of these parts, each keeping the number it had.
    fn keeping(parts: Vec<Part>, ids: Vec<u64>) -> Arc<Self> {
        debug_assert_eq!(parts.len(), ids.len());
        Arc::new(Self {
            stamp: fresh(),
            look: Arc::new(Look { stamp: fresh() }),
            parts,
            ids,
            hull: OnceLock::new(),
            pieces: OnceLock::new(),
            trees: OnceLock::new(),
        })
    }

    /// Whether "Split to objects" can be done to it (GUI_ObjectList.cpp
    /// `is_splittable(true)`): it has several parts, of whatever kind, or
    /// its one part is of several pieces.
    pub fn splits_to_objects(&self) -> bool {
        self.parts.len() > 1 || self.splits_to_parts()
    }

    /// Whether "Split to parts" can be done to it (`is_splittable(false)`
    /// with the object selected): it has one part, of several pieces.
    pub fn splits_to_parts(&self) -> bool {
        self.parts.len() == 1 && self.part_splits(0)
    }

    /// Whether part `at` can be split ("Split" of a part:
    /// `ModelVolume::is_splittable`): its mesh is of several pieces.
    pub fn part_splits(&self, at: usize) -> bool {
        self.pieces
            .get_or_init(|| {
                self.parts
                    .iter()
                    .map(|part| edge_mesh::split::is_splittable(&part.mesh))
                    .collect()
            })
            .get(at)
            .copied()
            .unwrap_or(false)
    }

    /// The number of each part, in order: see [`Shape::parts`].
    pub fn part_ids(&self) -> &[u64] {
        &self.ids
    }

    /// Its parts, in order.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The convex hull of its printed parts in the object's own
    /// coordinates: what it can rest on. None when they do not enclose
    /// anything (flat, or nothing at all). The reference merges the hulls
    /// of the parts and takes the hull of that (LayOnFace.cpp
    /// `lay_on_face_planes`), which is the same solid.
    pub fn hull(&self) -> Option<&Hull> {
        self.hull
            .get_or_init(|| {
                let points: Vec<[f64; 3]> = self
                    .printed()
                    .flat_map(|part| {
                        part.mesh
                            .vertices
                            .iter()
                            .map(|vertex| part.transform.apply(vertex.map(f64::from)))
                    })
                    .collect();
                edge_mesh::hull::convex_hull(&points)
            })
            .as_ref()
    }

    /// A number that is this shape's alone: another shape, or this one
    /// with another filament, has another.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// What it looks like ([`Look`]): two shapes with the same look have
    /// the same mesh to look at ([`Shape::mesh`]).
    pub fn look(&self) -> &Arc<Look> {
        &self.look
    }

    /// The shape as one mesh to look at, in the object's own coordinates,
    /// with what each triangle is. This is the costly part of showing a
    /// model; it is the same for as long as [`Shape::stamp`] is.
    pub fn mesh(&self, name: &str) -> SceneObject {
        object_of(&Object {
            id: 0,
            name: Some(name.to_owned()),
            filament: None,
            parts: self.parts.clone(),
            instances: Vec::new(),
            kept: None,
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

/// What a shape looks like, shared by the shapes that look alike: see
/// [`Shape::look`].
#[derive(Debug)]
pub struct Look {
    stamp: u64,
}

impl Look {
    /// A number that is this look's alone.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }
}

/// One object of a model.
#[derive(Clone, Debug)]
pub struct ModelObject {
    id: u64,
    name: String,
    /// The filament the object itself is set to, counting from 1, when it
    /// is: its parts without one of their own print in it, else in the
    /// first (the reference's object `extruder`, 0 for none).
    filament: Option<usize>,
    shape: Arc<Shape>,
    copies: Vec<ModelCopy>,
    /// What its file held of it beyond this, to be saved with it.
    kept: Option<Arc<ObjectKept>>,
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

    /// The cut that made it, when it was kept editable ("Edit cut...").
    pub fn cut_recipe(&self) -> Option<&Arc<edge_3mf::CutRecipe>> {
        self.kept.as_deref()?.recipe()
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

    /// The filament the object itself is set to, when it is: what a part
    /// without one of its own prints in ("default"), else filament 1.
    pub fn filament(&self) -> Option<usize> {
        self.filament
    }

    /// What a part without a filament of its own prints in.
    fn default_filament(&self) -> usize {
        self.filament.unwrap_or(1)
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

    /// Takes the object's coordinates to the project's, in which each
    /// plate stands where the reference's grid of plates puts it: what an
    /// export of the model's objects places them by.
    pub fn world(&self) -> Transform {
        self.world
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
    /// What its file held of it beyond this, to be saved with it.
    kept: Option<Arc<PlateKept>>,
    /// What its slice depended on when it was read: while that is so, its
    /// pictures in the file still show it.
    read: Option<PlateStamp>,
    /// The G-code set at its layers (pauses, custom G-code, filament
    /// changes), when it has an entry: one emptied stays, as the
    /// reference keeps it. It goes with the plate, whatever its number.
    custom_gcode: Option<CustomGcode>,
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

    /// The G-code set at its layers, when it has an entry.
    pub fn custom_gcode(&self) -> Option<&CustomGcode> {
        self.custom_gcode.as_ref()
    }
}

/// What a plate's slice depends on: equal for two models exactly when the
/// plate would slice alike in both.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PlateStamp {
    /// Where the plate is among the others, and its first layer's order.
    index: usize,
    first_layer_sequence: Vec<u32>,
    /// What else it sets for itself (its bed type, print sequence, vase
    /// mode, later layers' order), as a slice reads it.
    own: Vec<(&'static str, String)>,
    origin: Option<[u64; 2]>,
    /// Each printable copy on it: its shape and where it stands.
    copies: Vec<(u64, [u64; 12])>,
    /// A bare model, which slicing puts in the middle of the bed.
    lone: bool,
}

/// A part to add to an object: see [`Model::add_part`].
#[derive(Clone, Debug)]
pub struct NewPart {
    pub name: String,
    pub kind: PartKind,
    /// In its own coordinates.
    pub mesh: edge_mesh::Mesh,
    /// Takes the mesh to the object's coordinates.
    pub transform: Transform,
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
    /// What its file held beyond this, to be saved with it.
    kept: Option<Arc<Kept>>,
    /// The G-code set at layers of a model without plates, which is
    /// plate 1's as the reference reads it; it goes to plate 1 when that
    /// is made.
    loose_custom_gcode: Option<CustomGcode>,
    /// Whether its filaments were changed here ([`Model::add_filament`]
    /// and the rest), so that its settings are no longer its file's.
    filaments_edited: bool,
    /// For a model without settings of its own, what says what its
    /// filaments are once they were changed: their colours and what is
    /// flushed between them. A slice lays the presets over it.
    filament_lists: Option<Arc<Settings>>,
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
            kept: None,
            loose_custom_gcode: None,
            filaments_edited: false,
            filament_lists: None,
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

    /// What a slice lays the presets over: its own settings, else what
    /// says what its filaments are once they were changed here.
    pub fn underlying(&self) -> Option<&Arc<Settings>> {
        self.settings.as_ref().or(self.filament_lists.as_ref())
    }

    /// What is painted on it that a slice does not follow ("fuzzy skin",
    /// "supports"), in order of name: a slice notes each.
    pub fn unread_paint(&self) -> &[&'static str] {
        &self.unread_paint
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
                    own: load::plate_config(plate.kept.as_deref()),
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
                    filament: object.filament,
                    parts: object.shape.parts.clone(),
                    instances: object
                        .copies
                        .iter()
                        .map(|copy| Instance {
                            transform: copy.world,
                            printable: copy.printable,
                        })
                        .collect(),
                    kept: object.kept.clone(),
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
                    kept: plate.kept.clone(),
                })
                .collect(),
            settings: None,
            unread_paint: self.unread_paint.clone(),
            custom_gcode: self.custom_gcode_by_plate(),
            kept: self.kept.clone(),
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
        let settings = self.underlying().map(|settings| (**settings).clone());
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
                kept: None,
                read: None,
                custom_gcode: self.loose_custom_gcode.take(),
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
    /// part of it takes it, as the reference clears the parts' own choice
    /// when the object's is set (GUI_ObjectList.cpp, `set_extruder` on an
    /// object). What is painted on it stays.
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
        let shape = self.objects[at].shape.clone();
        let mut parts = shape.parts.clone();
        for part in &mut parts {
            part.own_filament = None;
            part.filament = filament;
        }
        self.objects[at].filament = Some(filament);
        self.objects[at].shape = Shape::keeping(parts, shape.ids.clone());
        Ok(())
    }

    /// The faces a copy can be laid on, largest first, as the reference's
    /// "Lay on face" offers them for its turn and size (LayOnFace.cpp
    /// `lay_on_face_planes` with the gizmo's filter).
    pub fn faces(&self, copy: u64) -> Result<Vec<Face>, Failure> {
        let (object, at) = self.copy_at(copy)?;
        let shape = &self.objects[object].shape;
        let mut linear = self.objects[object].copies[at].local;
        linear.rows[3] = [0.0; 3];
        let size = shape.reach(&linear).map_or(0.0, |[low, high]| {
            (0..3)
                .map(|axis| (high[axis] - low[axis]).powi(2))
                .sum::<f64>()
                .sqrt()
        });
        let Some(hull) = shape.hull() else {
            return Ok(Vec::new());
        };
        Ok(lay_on_face::faces(
            hull,
            &lay_on_face::linear_of(&linear),
            &Filter::for_gizmo(size),
            size,
        ))
    }

    /// Lays a copy on the face whose outward `normal` (in the object's own
    /// coordinates) is given: turned so that face points straight down
    /// (Selection.cpp `flattening_rotate`), then stood on the bed as any
    /// turn is (GLCanvas3D.cpp `do_rotate`).
    ///
    /// The turn is about the middle of the box round the copy. The
    /// reference turns an instance about its own origin, which it keeps in
    /// the middle of the object's geometry; a model read here is not moved
    /// to stand about its origin, so its origin may be anywhere.
    pub fn lay_on_face(
        &mut self,
        bed: Option<&Bed>,
        copy: u64,
        normal: [f64; 3],
    ) -> Result<(), Failure> {
        self.copy_at(copy)?;
        if !normal.iter().all(|value| value.is_finite()) || normal == [0.0; 3] {
            return Err(refused("that is no direction to lay the object on"));
        }
        self.settle(bed);
        let (scene, reach) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let turn = lay_on_face::turn_down(&lay_on_face::linear_of(&scene), normal)
            .ok_or_else(|| refused("that would squash the object to nothing"))?;
        let middle = reach.map_or([0.0; 3], |[low, high]| {
            std::array::from_fn(|axis| (low[axis] + high[axis]) / 2.0)
        });
        let turned = scene
            .then(&Transform::translation(middle.map(|value| -value)))
            .then(&lay_on_face::transform_of(&turn))
            .then(&Transform::translation(middle));
        self.stand_in_scene(bed, copy, turned);
        Ok(())
    }

    /// Mirrors copies along an axis of the scene (0 for X, 1 for Y, 2 for
    /// Z), about the middle of the box round them all, and stands them on
    /// the bed again (Selection.cpp `mirror`, GLCanvas3D.cpp `do_mirror`).
    ///
    /// The mirror is kept as the reference keeps it: in the copy's
    /// transform, which then turns the object's coordinates inside out.
    /// The mesh is left as it is; whatever places it by such a transform
    /// winds its triangles the other way round so that it stays a solid
    /// (`Mesh::append`), which is how a mirrored copy is sliced.
    pub fn mirror(
        &mut self,
        bed: Option<&Bed>,
        copies: &[u64],
        axis: usize,
    ) -> Result<(), Failure> {
        if axis > 2 {
            return Err(refused("an axis is X, Y or Z"));
        }
        for copy in copies {
            self.copy_at(*copy)?;
        }
        if copies.is_empty() {
            return Err(refused("nothing to mirror"));
        }
        self.settle(bed);
        let placed: Vec<(u64, Transform, Option<Reach>)> = copies
            .iter()
            .filter_map(|copy| {
                let (transform, reach) = self.in_scene(bed, *copy)?;
                Some((*copy, transform, reach))
            })
            .collect();
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for [from, to] in placed.iter().filter_map(|(_, _, reach)| *reach) {
            for at in 0..3 {
                low[at] = low[at].min(from[at]);
                high[at] = high[at].max(to[at]);
            }
        }
        let middle: [f64; 3] = if low[0].is_finite() {
            std::array::from_fn(|at| (low[at] + high[at]) / 2.0)
        } else {
            [0.0; 3]
        };
        let mut flip = Transform::IDENTITY;
        flip.rows[axis][axis] = -1.0;
        let about = Transform::translation(middle.map(|value| -value))
            .then(&flip)
            .then(&Transform::translation(middle));
        for (copy, scene, _) in placed {
            self.stand_in_scene(bed, copy, scene.then(&about));
        }
        Ok(())
    }

    /// The name an object goes by: its own, else its file's (an STL's
    /// object has none of its own, and the reference names it after the
    /// file).
    fn name_of(&self, at: usize) -> String {
        let own = &self.objects[at].name;
        if !own.is_empty() {
            return own.clone();
        }
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Object".into())
    }

    /// Splits an object into objects (the reference's "Split to objects":
    /// Plater.cpp `split_object`, Model.cpp `ModelObject::split`). An
    /// object of one part becomes an object for each piece of its mesh
    /// (each a shell sharing no edge with the others, with at least three
    /// triangles), named after the object with `_1`, `_2`, …; an object of
    /// several parts becomes an object for each printed part, named after
    /// the part, its other parts (modifiers, negative parts) being left
    /// out as the reference leaves them out. Each new object has a copy
    /// wherever the object had one, on the same plates, and stands where
    /// its piece stood, brought down onto the bed when it was left in the
    /// air (the reference's `ensure_on_bed` as it loads them). Answers the
    /// new copies and whether parts were left out.
    pub fn split_to_objects(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
    ) -> Result<(Vec<u64>, bool), Failure> {
        let at = self.object_at(object)?;
        let held = self.objects[at].clone();
        let parts = &held.shape.parts;
        let base = self.name_of(at);
        let own_name = |part: &Part| {
            part.name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| base.clone())
        };
        let mut made: Vec<(String, Part)> = Vec::new();
        if parts.len() == 1 {
            let part = &parts[0];
            if part.kind == PartKind::Normal {
                let pieces: Vec<edge_mesh::Mesh> = edge_mesh::split::split(&part.mesh)
                    .into_iter()
                    .filter(|piece| piece.triangles.len() >= 3)
                    .collect();
                let count = pieces.len();
                for (number, piece) in pieces.into_iter().enumerate() {
                    let name = if count == 1 {
                        own_name(part)
                    } else {
                        format!("{base}_{}", number + 1)
                    };
                    // The paint was put on the whole mesh's triangles; the
                    // reference starts each piece without it.
                    made.push((
                        name,
                        Part {
                            mesh: Arc::new(piece),
                            paint: None,
                            seam: None,
                            ..part.clone()
                        },
                    ));
                }
            }
        } else {
            for part in parts.iter().filter(|part| part.kind == PartKind::Normal) {
                if part.mesh.triangles.len() >= 3 {
                    made.push((own_name(part), part.clone()));
                }
            }
        }
        if made.len() < 2 {
            return Err(refused("The selected object couldn't be split."));
        }
        let dropped = parts.iter().any(|part| part.kind != PartKind::Normal);
        self.settle(bed);
        let held = self.objects.remove(at);
        let mut objects: Vec<ModelObject> = Vec::new();
        // For each copy of the old object, the copies made in its place.
        let mut instead: Vec<(u64, Vec<u64>)> = held
            .copies
            .iter()
            .map(|copy| (copy.id, Vec::new()))
            .collect();
        for (name, part) in made {
            let shape = Shape::new(vec![part]);
            let copies = held
                .copies
                .iter()
                .zip(&mut instead)
                .map(|(copy, (_, made))| {
                    let id = fresh();
                    made.push(id);
                    ModelCopy {
                        id,
                        reach: shape.reach(&copy.local),
                        ..copy.clone()
                    }
                })
                .collect();
            objects.push(ModelObject {
                id: fresh(),
                name,
                filament: held.filament,
                shape,
                copies,
                // What the file held of the object goes with each piece,
                // as it goes with a clone: the reference gives each new
                // object the old one's settings (`ModelObject::split`).
                kept: held.kept.clone(),
            });
        }
        let count = objects.len();
        self.objects.splice(at..at, objects);
        for plate in &mut self.plates {
            plate.order = plate
                .order
                .iter()
                .flat_map(|copy| match instead.iter().find(|(was, _)| was == copy) {
                    Some((_, made)) => made.clone(),
                    None => vec![*copy],
                })
                .collect();
        }
        // Each piece on the bed: stood again where it is.
        let new: Vec<u64> = self.objects[at..at + count]
            .iter()
            .flat_map(|object| object.copies.iter().map(|copy| copy.id))
            .collect();
        for copy in &new {
            if let Some((scene, _)) = self.in_scene(bed, *copy) {
                self.stand_in_scene(bed, *copy, scene);
            }
        }
        Ok((new, dropped))
    }

    /// Splits an object of one part into parts, one for each piece of its
    /// mesh (the reference's "Split to parts" on an object:
    /// GUI_ObjectList.cpp `ObjectList::split`): [`Model::split_part`] on
    /// its one part. The object stays where it stands.
    pub fn split_to_parts(&mut self, bed: Option<&Bed>, object: u64) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let shape = self.objects[at].shape.clone();
        if !shape.splits_to_parts() {
            return Err(refused(
                "The target object contains only one part and can not be split.",
            ));
        }
        self.split_part(bed, object, shape.ids[0])
    }

    // ---- paint ----

    /// Paints an object's parts (the painting gizmos: GLGizmoPainterBase.cpp
    /// `update_model_object`, `ModelVolume` `supported_facets`,
    /// `seam_facets`, `mmu_segmentation_facets`, `fuzzy_skin_facets`):
    /// each part named takes `kind` as its codes say, by the triangle's
    /// place in its mesh (`edge_mesh::selector::Selector::serialize`). Its
    /// slices go, its copies stay where they stand. Paint that is kept but
    /// not followed by the slice (fuzzy skin) is noted as such.
    /// False when nothing changed.
    pub fn set_paint(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        kind: PaintKind,
        parts: &[(u64, Vec<(u32, String)>)],
    ) -> Result<bool, Failure> {
        let at = self.object_at(object)?;
        let shape = self.objects[at].shape.clone();
        let mut list = shape.parts.clone();
        let mut changed = false;
        for (id, codes) in parts {
            let index = self.part_at(at, *id)?;
            if list[index].kind != PartKind::Normal {
                return Err(refused("only a part that is printed is painted"));
            }
            if list[index].paint_codes(kind) == *codes {
                continue;
            }
            list[index] = list[index].with_paint(kind, codes);
            changed = true;
        }
        if !changed {
            return Ok(false);
        }
        // A bare model painted has its place on the bed from now on, as it
        // has after any other edit.
        self.settle(bed);
        let mut next = Shape::keeping(list, shape.ids.clone());
        if kind != PaintKind::Color {
            // Only colours are drawn with the object.
            Arc::get_mut(&mut next).expect("just made").look = shape.look.clone();
        }
        self.objects[at].shape = next;
        // What is painted and not followed is said, as it is of a project
        // read with it (`unread_paint`): fuzzy skin. Supports painted are
        // followed (`regions`).
        let name = match kind {
            PaintKind::FuzzySkin => Some("fuzzy skin"),
            PaintKind::Supports | PaintKind::Seam | PaintKind::Color => None,
        };
        if let Some(name) = name {
            let anywhere = self
                .objects
                .iter()
                .flat_map(|object| object.shape.parts.iter())
                .any(|part| part.has_paint(kind));
            self.unread_paint.retain(|other| *other != name);
            if anywhere {
                self.unread_paint.push(name);
                self.unread_paint.sort_unstable();
            }
        }
        Ok(true)
    }

    // ---- parts ----

    fn part_at(&self, at: usize, part: u64) -> Result<usize, Failure> {
        self.objects[at]
            .shape
            .ids
            .iter()
            .position(|id| *id == part)
            .ok_or_else(|| refused(format!("there is no part {part}")))
    }

    fn parts_at(&self, at: usize, parts: &[u64]) -> Result<Vec<usize>, Failure> {
        if parts.is_empty() {
            return Err(refused("no part was named"));
        }
        let mut found = parts
            .iter()
            .map(|part| self.part_at(at, *part))
            .collect::<Result<Vec<_>, _>>()?;
        found.sort_unstable();
        found.dedup();
        Ok(found)
    }

    /// Gives an object other parts, each with its number, then stands
    /// every copy of it on the bed again and settles which plate it is
    /// on: what the reference does after a part has changed
    /// (Plater.cpp `changed_object`, `ModelObject::ensure_on_bed`;
    /// GLCanvas3D.cpp `do_move`, which brings a flying instance down and
    /// notifies the plates of every instance of an object whose part
    /// moved). A copy that was sunk stays sunk; one that stood stands.
    fn reshape(&mut self, bed: Option<&Bed>, at: usize, parts: Vec<Part>, ids: Vec<u64>) {
        self.objects[at].shape = Shape::keeping(parts, ids);
        let copies: Vec<u64> = self.objects[at].copies.iter().map(|copy| copy.id).collect();
        for copy in copies {
            if let Some((scene, _)) = self.in_scene(bed, copy) {
                self.stand_in_scene(bed, copy, scene);
            }
        }
    }

    /// Sets the filament parts print in (GUI_ObjectList.cpp `set_extruder`
    /// on a part): a filament of their own, counting from 1, or none, to
    /// follow their object ("default"). For a part or a modifier, the
    /// kinds the reference gives a filament to ("Change Filament",
    /// GUI_Factories.cpp `append_menu_item_change_filament`).
    pub fn set_part_filament(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        parts: &[u64],
        filament: Option<usize>,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let found = self.parts_at(at, parts)?;
        if filament == Some(0) {
            return Err(refused("filaments are counted from 1"));
        }
        let shape = self.objects[at].shape.clone();
        if found.iter().any(|index| {
            !matches!(
                shape.parts[*index].kind,
                PartKind::Normal | PartKind::Modifier
            )
        }) {
            return Err(refused(
                "only a part or a modifier has a filament of its own",
            ));
        }
        self.settle(bed);
        let default = self.objects[at].default_filament();
        let mut list = shape.parts.clone();
        for index in found {
            list[index].own_filament = filament;
            list[index].filament = filament.unwrap_or(default);
        }
        self.objects[at].shape = Shape::keeping(list, shape.ids.clone());
        Ok(())
    }

    /// Gives a part another name (GUI_ObjectList.cpp `update_name_in_model`).
    pub fn rename_part(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        part: u64,
        name: &str,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(refused("The name cannot be empty."));
        }
        self.settle(bed);
        let shape = self.objects[at].shape.clone();
        let mut list = shape.parts.clone();
        list[index].name = Some(name.to_owned());
        self.objects[at].shape = Shape::keeping(list, shape.ids.clone());
        Ok(())
    }

    /// Takes parts out of an object (GUI_ObjectList.cpp
    /// `del_subobject_from_object`). The last of its printed parts cannot
    /// go: "Deleting the last solid part is not allowed." When one part
    /// is left, its own filament becomes the object's, as the reference
    /// gives the last part's settings to the object.
    pub fn remove_parts(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        parts: &[u64],
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let found = self.parts_at(at, parts)?;
        let shape = self.objects[at].shape.clone();
        let solid_left = shape
            .parts
            .iter()
            .enumerate()
            .any(|(index, part)| part.kind == PartKind::Normal && !found.contains(&index));
        if !solid_left {
            return Err(refused("Deleting the last solid part is not allowed."));
        }
        self.settle(bed);
        let (mut list, ids): (Vec<Part>, Vec<u64>) = shape
            .parts
            .iter()
            .cloned()
            .zip(shape.ids.iter().copied())
            .enumerate()
            .filter(|(index, _)| !found.contains(index))
            .map(|(_, held)| held)
            .unzip();
        if let [last] = list.as_mut_slice() {
            if let Some(own) = last.own_filament.take() {
                self.objects[at].filament = Some(own);
            }
            // The rest of its own settings go to the object as well, as
            // the reference applies the last part's config to its object
            // and clears the part's.
            if let (Some(part), Some(object)) = (&last.kept, &self.objects[at].kept)
                && !part.settings().is_empty()
            {
                self.objects[at].kept = Some(Arc::new(object.with_settings_of(part)));
                last.kept = Some(Arc::new(part.without_settings()));
            }
        }
        self.reshape(bed, at, list, ids);
        Ok(())
    }

    /// Sets where parts stand within their object: each part's transform,
    /// which takes its mesh to the object's coordinates. Every copy of the
    /// object has its parts alike, as the reference's instances share
    /// their volumes; each is then stood on the bed again.
    pub fn set_part_transforms(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        to: &[(u64, Transform)],
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        if to.is_empty() {
            return Err(refused("no part was named"));
        }
        let mut places = Vec::new();
        for (part, transform) in to {
            places.push((self.part_at(at, *part)?, *transform));
            let sound = transform
                .rows
                .iter()
                .flatten()
                .all(|value| value.is_finite())
                && transform.determinant().abs() > 1e-12;
            if !sound {
                return Err(refused("that would squash the part to nothing"));
            }
        }
        self.settle(bed);
        let shape = self.objects[at].shape.clone();
        let mut list = shape.parts.clone();
        for (index, transform) in places {
            list[index].transform = transform;
        }
        self.reshape(bed, at, list, shape.ids.clone());
        Ok(())
    }

    /// Mirrors parts of an object along an axis of the scene (0 for X, 1
    /// for Y, 2 for Z), as they stand in one copy of it, about the middle
    /// of the box round them there (Selection.cpp `mirror` in its volume
    /// mode). The mirror is kept in each part's transform, so every copy's
    /// parts are mirrored alike.
    pub fn mirror_parts(
        &mut self,
        bed: Option<&Bed>,
        copy: u64,
        parts: &[u64],
        axis: usize,
    ) -> Result<(), Failure> {
        if axis > 2 {
            return Err(refused("an axis is X, Y or Z"));
        }
        let (at, _) = self.copy_at(copy)?;
        let found = self.parts_at(at, parts)?;
        self.settle(bed);
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let back = scene
            .inverse()
            .ok_or_else(|| refused("the object is squashed flat"))?;
        let shape = self.objects[at].shape.clone();
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for index in &found {
            let part = &shape.parts[*index];
            let placed = part.transform.then(&scene);
            for vertex in &part.mesh.vertices {
                let point = placed.apply(vertex.map(f64::from));
                for axis in 0..3 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
        }
        let middle: [f64; 3] = if low[0].is_finite() {
            std::array::from_fn(|at| (low[at] + high[at]) / 2.0)
        } else {
            [0.0; 3]
        };
        let mut flip = Transform::IDENTITY;
        flip.rows[axis][axis] = -1.0;
        let about = Transform::translation(middle.map(|value| -value))
            .then(&flip)
            .then(&Transform::translation(middle));
        let to: Vec<(u64, Transform)> = found
            .iter()
            .map(|index| {
                let part = &shape.parts[*index];
                (
                    shape.ids[*index],
                    part.transform.then(&scene).then(&about).then(&back),
                )
            })
            .collect();
        let object = self.objects[at].id;
        self.set_part_transforms(bed, object, &to)
    }

    /// Splits a part into parts, one for each piece of its mesh, in its
    /// place among the object's parts (the reference's "Split" of a part:
    /// GUI_ObjectList.cpp `ObjectList::split`, Model.cpp
    /// `ModelVolume::split`): each of its kind, named after it with `_1`,
    /// `_2`, …, given the filament it printed in as its own (Model.cpp,
    /// "always set the extruder id the same as original"), the paint on
    /// it let go; a piece that encloses nothing (whose convex hull is
    /// flat) is dropped. Its pieces keep its transform, so they stay
    /// where they were.
    pub fn split_part(&mut self, bed: Option<&Bed>, object: u64, part: u64) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        let shape = self.objects[at].shape.clone();
        if !shape.part_splits(index) {
            return Err(refused(
                "The target object contains only one part and can not be split.",
            ));
        }
        let held = &shape.parts[index];
        let name = held
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.name_of(at));
        let pieces: Vec<Part> = edge_mesh::split::split(&held.mesh)
            .into_iter()
            .filter(|piece| !piece.triangles.is_empty())
            .enumerate()
            .filter(|(_, piece)| {
                let points: Vec<[f64; 3]> =
                    piece.vertices.iter().map(|v| v.map(f64::from)).collect();
                edge_mesh::hull::convex_hull(&points).is_some()
            })
            .map(|(number, piece)| Part {
                name: Some(format!("{name}_{}", number + 1)),
                mesh: Arc::new(piece),
                paint: None,
                seam: None,
                own_filament: Some(held.filament),
                // Its kind and settings go with each piece, not what the
                // part's triangles carried or where it came from.
                kept: held.kept.as_deref().map(|kept| Arc::new(kept.piece())),
                ..held.clone()
            })
            .collect();
        if pieces.is_empty() {
            return Err(refused(
                "The target object contains only one part and can not be split.",
            ));
        }
        self.settle(bed);
        let mut list = shape.parts.clone();
        let mut ids = shape.ids.clone();
        let count = pieces.len();
        list.splice(index..=index, pieces);
        ids.splice(index..=index, (0..count).map(|_| fresh()));
        self.reshape(bed, at, list, ids);
        Ok(())
    }

    /// Makes parts another kind (GUI_ObjectList.cpp `change_part_type`,
    /// `set_volume_type`). The last printed part of an object stays one:
    /// "The type of the last solid object part is not to be changed."
    /// Each part keeps its place in the list, its name, filament and
    /// paint.
    pub fn set_part_kind(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        parts: &[u64],
        kind: PartKind,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let found = self.parts_at(at, parts)?;
        let shape = self.objects[at].shape.clone();
        if kind != PartKind::Normal {
            let solid_left = shape
                .parts
                .iter()
                .enumerate()
                .any(|(index, part)| part.kind == PartKind::Normal && !found.contains(&index));
            if !solid_left {
                return Err(refused(
                    "The type of the last solid object part is not to be changed.",
                ));
            }
        }
        if found.iter().all(|index| shape.parts[*index].kind == kind) {
            return Err(refused("the parts are of that type already"));
        }
        self.settle(bed);
        let mut list = shape.parts.clone();
        for index in found {
            list[index].kind = kind;
        }
        self.reshape(bed, at, list, shape.ids.clone());
        Ok(())
    }

    /// Adds a part to an object and answers its number: a mesh of the
    /// kind given, named, placed in the object's coordinates by
    /// `transform` (GUI_ObjectList.cpp `load_modifier` for a file,
    /// `load_generic_subobject` for a shape). A printed part takes the
    /// object's own filament as its own, if the object has one; any other
    /// none, as the reference sets the new volume's `extruder`.
    pub fn add_part(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        part: NewPart,
    ) -> Result<u64, Failure> {
        let at = self.object_at(object)?;
        if part.mesh.triangles.is_empty() {
            return Err(refused("there is nothing in that part"));
        }
        self.settle(bed);
        let own = (part.kind == PartKind::Normal)
            .then_some(self.objects[at].filament)
            .flatten();
        let made = Part {
            kept: None,
            name: Some(part.name),
            kind: part.kind,
            filament: own.unwrap_or(self.objects[at].default_filament()),
            own_filament: own,
            mesh: Arc::new(part.mesh),
            paint: None,
            seam: None,
            transform: part.transform,
            decoration: false,
        };
        let shape = self.objects[at].shape.clone();
        let mut list = shape.parts.clone();
        let mut ids = shape.ids.clone();
        let id = fresh();
        list.push(made);
        ids.push(id);
        self.reshape(bed, at, list, ids);
        Ok(id)
    }

    /// Adds a generic shape to an object as a part of the kind given
    /// (GUI_ObjectList.cpp `load_generic_subobject`, `create_mesh`), beside
    /// the copy named (the object's first when none is): only "Cube" so
    /// far, a tenth of the bed's greater side, named "Generic-Cube".
    /// Answers the new part's number.
    pub fn add_generic(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        copy: Option<u64>,
        kind: PartKind,
        shape: &str,
    ) -> Result<u64, Failure> {
        let at = self.object_at(object)?;
        if shape != "Cube" {
            return Err(refused(format!("there is no shape named {shape}")));
        }
        let bed = bed
            .ok_or_else(|| refused("no printer is chosen: there is no bed to size the shape by"))?;
        let copy = match copy {
            Some(copy) => {
                if self.copy_at(copy)?.0 != at {
                    return Err(refused(format!("copy {copy} is not of object {object}")));
                }
                copy
            }
            None => self.objects[at]
                .copies
                .first()
                .map(|copy| copy.id)
                .ok_or_else(|| refused("the object has no copy"))?,
        };
        // Given its place first, so that the shape goes beside it where
        // it then stands.
        self.settle(Some(bed));
        let side = 0.1 * (bed.max[0] - bed.min[0]).max(bed.max[1] - bed.min[1]);
        let half = (side / 2.0) as f32;
        let transform = self.beside(Some(bed), copy, [side; 3])?;
        self.add_part(
            Some(bed),
            object,
            NewPart {
                name: format!("Generic-{shape}"),
                kind,
                mesh: edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
                    [-half; 3], [half; 3],
                )),
                transform,
            },
        )
    }

    /// Adds what another model prints to an object as one part of the
    /// kind given, named after its file (GUI_ObjectList.cpp
    /// `load_modifier`, from "Load..."): every printed part of it where
    /// its copies stand on their plates, in one mesh, taken in the
    /// object's own coordinates. Answers the new part's number.
    pub fn add_from(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        other: &Model,
        kind: PartKind,
    ) -> Result<u64, Failure> {
        let mut mesh = edge_mesh::Mesh::default();
        for held in &other.objects {
            for copy in &held.copies {
                for part in held.shape.printed() {
                    mesh.append(&part.mesh, &part.transform.then(&copy.local));
                }
            }
        }
        let name = other
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.add_part(
            bed,
            object,
            NewPart {
                name,
                kind,
                mesh,
                transform: Transform::IDENTITY,
            },
        )
    }

    /// Where a generic shape added to an object goes, in the object's
    /// coordinates (GUI_ObjectList.cpp `load_generic_subobject`): square
    /// to the bed whatever the copy's turn, its middle half its size
    /// beyond the right front corner of the box round the copy, at the
    /// height of its foot. `size` is the shape's own box.
    pub fn beside(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        size: [f64; 3],
    ) -> Result<Transform, Failure> {
        let (scene, reach) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let back = scene
            .inverse()
            .ok_or_else(|| refused("the object is squashed flat"))?;
        let [low, high] = reach.ok_or_else(|| refused("the object has nothing printed in it"))?;
        let middle = [
            high[0] + size[0] / 2.0,
            low[1] + size[1] / 2.0,
            low[2] + size[2] / 2.0,
        ];
        Ok(Transform::translation(middle).then(&back))
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
            kept: None,
            read: None,
            custom_gcode: None,
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

    // ---- G-code at layers ----

    /// The G-code set at the layers of a plate (by number; none for a
    /// model without plates, whose plate 1 it then is), when it has an
    /// entry.
    pub fn custom_gcode(&self, plate: Option<u32>) -> Option<&CustomGcode> {
        match plate.or((!self.plates.is_empty()).then_some(1)) {
            Some(id) => self
                .plate_index(id)
                .and_then(|index| self.plates[index].custom_gcode.as_ref()),
            None => self.loose_custom_gcode.as_ref(),
        }
    }

    /// Sets what is done at a plate's layers, as the reference sets it
    /// when a tick of the layers slider is added, edited or deleted
    /// (Plater.cpp, `EVT_CUSTOMEVT_TICKSCHANGED`): the plate's entry as a
    /// whole, an emptied one staying. Nothing on the plate moves.
    pub fn set_custom_gcode(
        &mut self,
        plate: Option<u32>,
        info: CustomGcode,
    ) -> Result<(), Failure> {
        match plate.or((!self.plates.is_empty()).then_some(1)) {
            Some(id) => {
                let index = self
                    .plate_index(id)
                    .ok_or_else(|| refused(format!("there is no plate {id}")))?;
                self.plates[index].custom_gcode = Some(info);
            }
            None => self.loose_custom_gcode = Some(info),
        }
        Ok(())
    }

    /// Each plate's G-code at layers by its number as it stands, as a
    /// project file holds them.
    pub(crate) fn custom_gcode_by_plate(&self) -> std::collections::BTreeMap<u32, CustomGcode> {
        let mut out: std::collections::BTreeMap<u32, CustomGcode> = self
            .plates
            .iter()
            .filter_map(|plate| Some((plate.id, plate.custom_gcode.clone()?)))
            .collect();
        if self.plates.is_empty()
            && let Some(loose) = &self.loose_custom_gcode
        {
            out.insert(1, loose.clone());
        }
        out
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
                filament: object.filament,
                shape: object.shape.clone(),
                copies,
                kept: object.kept.clone(),
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
            let (name, shape, filament, kept) = (
                self.objects[object].name.clone(),
                self.objects[object].shape.clone(),
                self.objects[object].filament,
                self.objects[object].kept.clone(),
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
                filament,
                shape,
                copies: vec![seated],
                kept: kept.clone(),
            });
        }
        Ok(added)
    }

    /// Arranges what stands on a plate (on every plate for none named)
    /// with the reference's arranger and the default options, by the
    /// model's own settings: see [`Model::arrange_with`]. Answers how many
    /// copies no plate could hold, which are put beside the plates.
    ///
    /// A locked plate is left as it is, as the reference leaves it
    /// (ArrangeJob.cpp: `preprocess_arrange_polygon` keeps what stands on
    /// it out of the arrangement); arranging it alone is refused in the
    /// reference's words (`prepare_partplate`). So are the plates in
    /// `leave`.
    pub fn arrange(&mut self, bed: Option<&Bed>, plate: Option<u32>) -> Result<usize, Failure> {
        self.arrange_leaving(bed, plate, &[])
    }

    /// [`Model::arrange`], leaving the plates in `leave` as they are too:
    /// the reference leaves a plate whose own print sequence is not the
    /// project's alone while it arranges every plate (ArrangeJob.cpp
    /// `prepare_all`, `m_uncompatible_plates`).
    pub fn arrange_leaving(
        &mut self,
        bed: Option<&Bed>,
        plate: Option<u32>,
        leave: &[u32],
    ) -> Result<usize, Failure> {
        let scope = match plate {
            Some(id) => ArrangeScope::Plate(id),
            None => ArrangeScope::All { leave },
        };
        let arranged = self.arrange_with(bed, None, &ArrangeOptions::default(), scope)?;
        Ok(arranged.unplaced.len())
    }
}

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
                    own_filament: None,
                    mesh: Arc::new(mesh),
                    paint: None,
                    seam: None,
                    transform: Transform::IDENTITY,
                    decoration: false,
                    kept: None,
                }]);
                let reach = shape.reach(&Transform::IDENTITY);
                Self {
                    path,
                    objects: vec![ModelObject {
                        id: fresh(),
                        name: String::new(),
                        filament: None,
                        shape,
                        copies: vec![ModelCopy {
                            id: fresh(),
                            world: Transform::IDENTITY,
                            local: Transform::IDENTITY,
                            reach,
                            printable: true,
                        }],
                        kept: None,
                    }],
                    plates: Vec::new(),
                    settings: None,
                    unread_paint: Vec::new(),
                    bare: true,
                    placed: false,
                    kept: None,
                    loose_custom_gcode: None,
                    filaments_edited: false,
                    filament_lists: None,
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
                            filament: object.filament,
                            shape,
                            copies,
                            kept: object.kept.clone(),
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
                        kept: plate.kept.clone(),
                        read: None,
                        custom_gcode: project.custom_gcode.get(&plate.id).cloned(),
                    })
                    .collect();
                let mut model = Self {
                    path,
                    objects,
                    plates,
                    settings: project.settings.map(Arc::new),
                    unread_paint: project.unread_paint,
                    bare: false,
                    placed: false,
                    kept: project.kept,
                    // A project without plates has its items under plate 1.
                    filaments_edited: false,
                    filament_lists: None,
                    loose_custom_gcode: if project.plates.is_empty() {
                        project.custom_gcode.get(&1).cloned()
                    } else {
                        None
                    },
                };
                // What each plate's slice depends on as read, for its
                // pictures.
                for at in 0..model.plates.len() {
                    let id = model.plates[at].id;
                    model.plates[at].read = Some(model.stamp(Some(id)));
                }
                model
            }
        }
    }
}
