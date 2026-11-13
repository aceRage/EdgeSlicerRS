//! Auto orient for a model's copies, gathered and put back as the
//! reference's orient job does (Jobs/OrientJob.cpp `prepare_selected`,
//! `prepare_partplate`, `prepare_selection`, `get_orient_mesh`,
//! `finalize`), with the orienter of [`crate::orient`].
//!
//! Where this is not the reference's, on purpose:
//!
//! - Each copy is oriented by its own mesh. The reference gives each
//!   instance its object's `mesh()`, which is every instance of the object
//!   placed and merged into one, so an object with several instances is
//!   measured as a pile of them (`get_orient_mesh`).
//! - "Auto orient objects on current plate" orients the copies on that
//!   plate. The reference marks an object as on the plate by its last
//!   instance alone (`prepare_partplate`, `obj_sel[oidx]` set for each
//!   instance in turn), orients every instance of a marked object,
//!   wherever it stands, and when no object is marked orients every object
//!   of the project.
//! - A copy is turned about the middle of the box round it, as "Lay on
//!   face" turns it here; the reference turns an instance about its own
//!   origin, which it keeps in the middle of the object.

use edge_config::Settings;
use edge_mesh::{Mesh, Transform};
use rayon::prelude::*;

use super::{Bed, Model, refused};
use crate::{
    Failure,
    lay_on_face::{Mat3, transform_of},
    orient::{self, Faces, Orientation, Params},
};

/// The reference's default support threshold angle.
const DEFAULT_ANGLE: f64 = 30.0;

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// What is oriented.
#[derive(Clone, Copy, Debug)]
pub enum OrientScope<'a> {
    /// "Auto orient all/selected objects" [Q]: every copy of each object
    /// a copy named belongs to, those on locked plates left out; with none
    /// named, every copy of every object not on a locked plate
    /// (OrientJob.cpp `prepare_selected`, `prepare_selection`).
    Selected(&'a [u64]),
    /// "Auto orient all objects on current plate" [Shift+Q], the plate's
    /// icon, "Auto Rotate": the copies on a plate (`prepare_partplate`).
    Plate(u32),
}

/// What came of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Oriented {
    /// The copies looked at, in order.
    pub examined: Vec<u64>,
    /// Those that were turned.
    pub turned: Vec<u64>,
}

impl Model {
    /// The copies an orient of `scope` works on.
    fn to_orient(&self, scope: OrientScope<'_>) -> Result<Vec<u64>, Failure> {
        match scope {
            OrientScope::Plate(id) => {
                let at = self
                    .plate_index(id)
                    .ok_or_else(|| refused(format!("there is no plate {id}")))?;
                let plate = &self.plates[at];
                if plate.order.is_empty() {
                    return Ok(Vec::new());
                }
                if plate.locked() {
                    return Err(refused(
                        "This plate is locked.\nCannot auto-orient on this plate.",
                    ));
                }
                // In the order of the objects, as the reference goes
                // through them.
                Ok(self
                    .objects
                    .iter()
                    .flat_map(|object| object.copies.iter().map(|copy| copy.id))
                    .filter(|copy| plate.order.contains(copy))
                    .collect())
            }
            OrientScope::Selected(named) => {
                let mut objects = Vec::new();
                for copy in named {
                    objects.push(self.copy_at(*copy)?.0);
                }
                let mut selected = Vec::new();
                let mut unselected = Vec::new();
                let mut selected_is_locked = false;
                for (at, object) in self.objects.iter().enumerate() {
                    let chosen = objects.contains(&at);
                    for copy in &object.copies {
                        let locked = self
                            .plate_of(copy.id)
                            .is_some_and(|plate| self.plates[plate].locked());
                        if locked {
                            selected_is_locked |= chosen;
                            continue;
                        }
                        if chosen {
                            selected.push(copy.id);
                        } else {
                            unselected.push(copy.id);
                        }
                    }
                }
                if selected.is_empty() {
                    if selected_is_locked {
                        return Err(refused(
                            "All the selected objects are on a locked plate.\nCannot auto-orient these objects.",
                        ));
                    }
                    selected = unselected;
                }
                Ok(selected)
            }
        }
    }

    /// The support threshold angle an object is oriented by: its own,
    /// else the settings' (`get_orient_mesh`).
    fn orient_angle(&self, object: usize, settings: Option<&Settings>) -> f64 {
        let parse = |text: &str| text.trim().parse::<f64>().ok();
        self.objects[object]
            .settings()
            .iter()
            .find(|(key, _)| key == "support_threshold_angle")
            .and_then(|(_, value)| parse(value))
            .or_else(|| {
                settings
                    .or(self.settings.as_deref())
                    .unwrap_or(edge_config::reference_defaults())
                    .texts("support_threshold_angle")
                    .and_then(|texts| texts.first().and_then(|text| parse(text)))
            })
            .unwrap_or(DEFAULT_ANGLE)
    }

    /// Which way up one copy prints best, as it stands in the scene:
    /// measured on its printed parts placed as it is placed.
    pub fn orientation(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        params: &Params,
    ) -> Result<Option<Orientation>, Failure> {
        let (object, _) = self.copy_at(copy)?;
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let shape = &self.objects[object].shape;
        Ok(orientation_of(shape, &[scene], params))
    }

    /// Auto orient (OrientJob.cpp): each copy of `scope` turned the way
    /// up its mesh prints best, then stood on the bed
    /// (`ModelObject::ensure_on_bed`) on the plate it then reaches over.
    /// Nothing else moves; nothing is arranged afterwards. `settings` are
    /// what the project is sliced with, whose support threshold angle an
    /// object without its own is measured by.
    pub fn orient(
        &mut self,
        bed: Option<&Bed>,
        settings: Option<&Settings>,
        scope: OrientScope<'_>,
    ) -> Result<Oriented, Failure> {
        let copies = self.to_orient(scope)?;
        let mut done = Oriented {
            examined: copies.clone(),
            turned: Vec::new(),
        };
        if copies.is_empty() {
            return Ok(done);
        }
        self.settle(bed);
        let work: Vec<(u64, Params)> = copies
            .iter()
            .map(|copy| {
                let (object, _) = self.find(*copy).expect("gathered above");
                (
                    *copy,
                    Params::application(self.orient_angle(object, settings)),
                )
            })
            .collect();
        let found: Vec<(u64, Option<Mat3>)> = work
            .par_iter()
            .map(|(copy, params)| {
                let rotation = self
                    .orientation(bed, *copy, params)
                    .ok()
                    .flatten()
                    .map(|found| found.rotation);
                (*copy, rotation)
            })
            .collect();
        for (copy, rotation) in found {
            let Some(rotation) = rotation.filter(|rotation| *rotation != IDENTITY) else {
                continue;
            };
            let (object, _) = self.copy_at(copy)?;
            let shape = self.objects[object].shape.clone();
            let (scene, reach) = self
                .in_scene(bed, copy)
                .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
            let middle = reach.map_or([0.0; 3], |[low, high]| {
                std::array::from_fn(|axis| (low[axis] + high[axis]) / 2.0)
            });
            let mut turned = scene
                .then(&Transform::translation(middle.map(|value| -value)))
                .then(&transform_of(&rotation))
                .then(&Transform::translation(middle));
            // `ensure_on_bed`: its lowest point on the bed, whether it was
            // in the air or sunk under it.
            if let Some([low, _]) = shape.reach(&turned) {
                turned = turned.then(&Transform::translation([0.0, 0.0, -low[2]]));
            }
            self.stand_in_scene(bed, copy, turned);
            done.turned.push(copy);
        }
        Ok(done)
    }
}

/// The orientation of a shape's printed parts placed by each of
/// `placings`, merged into one mesh, and of their convex hull.
pub(crate) fn orientation_of(
    shape: &super::Shape,
    placings: &[Transform],
    params: &Params,
) -> Option<Orientation> {
    // Placed as the reference places it (`ModelObject::mesh`): each part
    // by its own transform, kept in `f32`, then by the copy's, which is
    // two roundings, not one; the corners have to be the reference's to
    // the bit for its faces to fall into the same groups by normal.
    let mut mesh = Mesh::default();
    for placing in placings {
        for part in shape.printed() {
            let mut own = Mesh::default();
            own.append(&part.mesh, &part.transform);
            mesh.append(&own, placing);
        }
    }
    let hull = if placings.len() == 1 {
        // The hull of the placed mesh is its own hull placed.
        let hull = shape.hull()?;
        let placing = placings[0];
        let mirrored = placing.is_mirrored();
        Mesh {
            vertices: hull
                .points
                .iter()
                .map(|p| placing.apply(*p).map(|v| v as f32))
                .collect(),
            triangles: hull
                .triangles
                .iter()
                .map(|&[a, b, c]| if mirrored { [a, c, b] } else { [a, b, c] })
                .collect(),
        }
    } else {
        let points: Vec<[f64; 3]> = mesh.vertices.iter().map(|v| v.map(f64::from)).collect();
        let hull = edge_mesh::hull::convex_hull(&points)?;
        Mesh {
            vertices: hull.points.iter().map(|p| p.map(|v| v as f32)).collect(),
            triangles: hull.triangles,
        }
    };
    orient::orient(
        Faces {
            vertices: &mesh.vertices,
            triangles: &mesh.triangles,
        },
        Faces {
            vertices: &hull.vertices,
            triangles: &hull.triangles,
        },
        params,
    )
}

impl super::ModelObject {
    /// Which way up the object prints best as the reference's command line
    /// measures it (`--orient 1`, Orient.cpp `orient(ModelObject*)`): its
    /// printed parts placed by every copy and merged, with the command
    /// line's weights. For comparing with the reference.
    pub fn orientation_as_command_line(&self) -> Option<Orientation> {
        let placings: Vec<Transform> = self.copies.iter().map(|copy| copy.world()).collect();
        orientation_of(&self.shape, &placings, &Params::command_line())
    }
}
