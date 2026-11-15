//! A part's mesh given another in its place: what "Simplify" applies
//! (Gizmos/GLGizmoSimplify.cpp `apply_simplify`) and "Repair" puts back
//! (GUI_ObjectList.cpp `repair_by_remesh`).
//!
//! What the reference does, and so this:
//!
//! - every part of the object loses what is painted on it, supports,
//!   seams, colours and fuzzy skin, the part changed and the others alike
//!   (Plater.cpp `clear_before_change_mesh`, which says so: "Custom
//!   supports and color painting were removed before repairing.");
//! - the part keeps its place in the object, its matrix, name, kind,
//!   filament and settings; only its mesh changes (`ModelVolume::set_mesh`),
//!   and with it what the mesh's statistics say was repaired;
//! - every copy of the object is stood on the bed again
//!   (`ModelObject::ensure_on_bed`).
//!
//! The statistics shown in the object list ([`MeshErrors`]) are worked out
//! here from the mesh as the reference's `fill_initial_stats` does
//! (`its_num_open_edges`, `its_number_of_patches`), with what the part's
//! file says was repaired.

use std::sync::Arc;

use edge_3mf::{MeshStat, PaintKind, Part};
use edge_mesh::Mesh;

use super::{Bed, Model, Shape, refused};
use crate::Failure;

/// What the object list's warning says of a part or an object
/// (GUI_ObjectList.cpp `get_mesh_errors_info`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshErrors {
    /// Edges with a triangle on one side only (`open_edges`).
    pub open_edges: u64,
    /// What was repaired (`repaired_errors`).
    pub repaired: MeshStat,
}

impl MeshErrors {
    /// `TriangleMeshStats::manifold`.
    pub fn manifold(&self) -> bool {
        self.open_edges == 0
    }

    /// Whether the warning shows: errors repaired or edges left open
    /// (`get_warning_icon_name`).
    pub fn shown(&self) -> bool {
        !self.manifold() || self.repaired.repaired()
    }
}

/// `its_num_open_edges(its_face_neighbors(its))`: the edges of a mesh's
/// triangles that no other triangle shares the other way round.
pub fn open_edges(mesh: &Mesh) -> u64 {
    edge_mesh::split::neighbours(mesh)
        .iter()
        .flatten()
        .filter(|n| n.is_none())
        .count() as u64
}

impl Model {
    /// Which part of `object` a mesh edit is for: `part`, or with none named
    /// the object's one part (the reference's `get_model_volume` takes one
    /// selected volume, and an object of one part selected whole is that
    /// volume). Answers its place and its mesh.
    pub fn mesh_of_part(
        &self,
        object: u64,
        part: Option<u64>,
    ) -> Result<(u64, Arc<Mesh>), Failure> {
        let at = self.object_at(object)?;
        let shape = &self.objects[at].shape;
        let index = match part {
            Some(part) => self.part_at(at, part)?,
            None if shape.parts.len() == 1 => 0,
            None => {
                return Err(refused(
                    "Simplification is currently only allowed when a single part is selected",
                ));
            }
        };
        Ok((shape.ids[index], shape.parts[index].mesh.clone()))
    }

    /// The name a part goes by: its own, else its object's.
    pub fn part_name(&self, object: u64, part: u64) -> Result<String, Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        Ok(self.objects[at].shape.parts[index]
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.name_of(at)))
    }

    /// Part `part` of `object` given `mesh` in place of its own, with `stat`
    /// as what was repaired in it; see the module's words. Answers whether
    /// any paint was taken off the object's parts (the reference then says
    /// "Custom supports and color painting were removed before repairing.").
    pub fn set_part_mesh(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        part: u64,
        mesh: Mesh,
        stat: MeshStat,
    ) -> Result<bool, Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        if mesh.triangles.is_empty() {
            return Err(refused("the new mesh has no triangles"));
        }
        let shape = self.objects[at].shape.clone();
        let mut painted = false;
        let mut list: Vec<Part> = shape
            .parts
            .iter()
            .map(|part| {
                let kinds: Vec<PaintKind> = PaintKind::ALL
                    .into_iter()
                    .filter(|kind| part.has_paint(*kind))
                    .collect();
                painted |= !kinds.is_empty();
                kinds
                    .into_iter()
                    .fold(part.clone(), |part, kind| part.with_paint(kind, &[]))
            })
            .collect();
        let old = &list[index];
        let kept = old
            .kept
            .as_deref()
            .cloned()
            .unwrap_or_default()
            .with_new_mesh(stat);
        list[index] = Part {
            mesh: Arc::new(mesh),
            paint: None,
            seam: None,
            kept: Some(Arc::new(kept)),
            ..old.clone()
        };
        self.settle(bed);
        let ids = shape.ids.clone();
        self.reshape(bed, at, list, ids);
        Ok(painted)
    }

    /// What the object list's warning says of a part (`part`) or of the
    /// whole object (`ModelObject::get_object_stl_stats`: every part's open
    /// edges and repairs summed).
    pub fn mesh_errors(&self, object: u64, part: Option<u64>) -> Result<MeshErrors, Failure> {
        let at = self.object_at(object)?;
        let shape = &self.objects[at].shape;
        match part {
            Some(part) => Ok(shape.part_errors()[self.part_at(at, part)?]),
            None => Ok(shape.errors()),
        }
    }
}

impl Shape {
    /// What each part's statistics say, in order (`TriangleMesh::stats`):
    /// its open edges, and what its file says was repaired.
    pub fn part_errors(&self) -> &[MeshErrors] {
        self.errors.get_or_init(|| {
            self.parts
                .iter()
                .map(|part| MeshErrors {
                    open_edges: open_edges(&part.mesh),
                    repaired: part
                        .kept
                        .as_deref()
                        .map(|kept| kept.mesh_stat())
                        .unwrap_or_default(),
                })
                .collect()
        })
    }

    /// The whole object's: every part's summed.
    pub fn errors(&self) -> MeshErrors {
        self.part_errors()
            .iter()
            .fold(MeshErrors::default(), |mut sum, one| {
                sum.open_edges += one.open_edges;
                sum.repaired.merge(&one.repaired);
                sum
            })
    }
}
