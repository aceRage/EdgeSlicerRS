//! "Mesh Boolean" (Gizmos/GLGizmoMeshBoolean.cpp): two parts of one
//! object made one by a union, a difference or an intersection.
//!
//! What the reference does, and so this:
//!
//! - both parts are taken into the object's own coordinates, each by its
//!   own matrix (`run_boolean`: `mesh.transform(get_matrix(), true)`), a
//!   mirrored part's triangles turned round so it stays a solid; the
//!   copies' matrices play no part, as every copy shares the parts;
//! - the boolean is done on those two meshes ([`edge_mesh::boolean`], where
//!   the reference has Manifold with mcut behind it);
//! - the result takes the first part's place in the list
//!   (`ModelObject::replace_volume_with_object_mesh`): its mesh is stored
//!   about the middle of its box, its matrix only the move back there (the
//!   first part's turn, size and mirror are in the mesh now); it is named
//!   "<first part's name> - union" ("difference", "intersection"), keeps
//!   the first part's kind, filament and settings, and drops what was
//!   painted on it;
//! - the second part is deleted for a union, and for the others when
//!   "Delete input" is ticked;
//! - all of it is one edit ("Mesh Boolean" in the undo history).
//!
//! Where this differs, on purpose:
//!
//! - an empty result (two parts that do not meet, intersected; a part the
//!   other covers whole, taken away) is said as such ([`BooleanFailure::Empty`])
//!   and changes nothing; the reference counts it as a failure, "Unable to
//!   perform boolean operation on selected parts" (fixed in
//!   aceRage/EdgeSlicer#1005, whose words are used);
//! - only printed parts can be taken. The reference lets any volume be
//!   picked, a modifier or a negative part too, and makes the result of
//!   the first one's kind: a modifier's region added to a part as solid,
//!   or a part's material cut out of a negative part, which is not what a
//!   boolean of solids is for;
//! - the airbrush paint the reference carries onto the result
//!   (`airbrush_carry_field`) is not, as this engine has no airbrush.

use std::sync::Arc;

use edge_3mf::{Part, PartKind};
use edge_mesh::{
    Mesh, Transform,
    boolean::{BooleanError, InputError, Op, Operand},
};

use super::{Bed, Model, fresh, refused};
use crate::Failure;

/// Why a boolean of two parts made no change.
#[derive(Debug)]
pub enum BooleanFailure {
    /// The parts named cannot be taken: not of the object, the same part
    /// twice, or not printed parts.
    Refused(Failure),
    /// The boolean was done and leaves nothing.
    Empty,
    /// The boolean could not be done. `part`: the name of a part that does
    /// not bound a volume, when that is why.
    Failed {
        part: Option<String>,
        reason: BooleanError,
    },
}

impl std::fmt::Display for BooleanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BooleanFailure::Refused(why) => write!(f, "{why}"),
            BooleanFailure::Empty => f.write_str("the boolean leaves nothing of the parts"),
            BooleanFailure::Failed { reason, .. } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for BooleanFailure {}

/// What the result of a boolean is called after its first part's name
/// (GLGizmoMeshBoolean.cpp `generate_new_volume`).
fn suffix(op: Op) -> &'static str {
    match op {
        Op::Union => "union",
        Op::Difference => "difference",
        Op::Intersection => "intersection",
    }
}

/// A part's mesh in its object's coordinates.
fn in_object(part: &Part) -> Mesh {
    let mut mesh = Mesh::default();
    // `append` turns a mirrored part's triangles round.
    mesh.append(&part.mesh, &part.transform);
    mesh
}

impl Model {
    /// "Mesh Boolean": parts `source` and `tool` of `object` made one by
    /// `op`, the result in `source`'s place; `tool` deleted when
    /// `delete_tool` (always for a union, as the reference's gizmo does).
    /// Answers the result's part number. On any failure the model is as
    /// it was. See the module's words.
    pub fn mesh_boolean(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        source: u64,
        tool: u64,
        op: Op,
        delete_tool: bool,
    ) -> Result<u64, BooleanFailure> {
        let at = self.object_at(object).map_err(BooleanFailure::Refused)?;
        let first = self.part_at(at, source).map_err(BooleanFailure::Refused)?;
        let second = self.part_at(at, tool).map_err(BooleanFailure::Refused)?;
        if first == second {
            return Err(BooleanFailure::Refused(refused(
                "a part cannot be booleaned with itself",
            )));
        }
        let shape = self.objects[at].shape.clone();
        let (a, b) = (&shape.parts[first], &shape.parts[second]);
        if a.kind != PartKind::Normal || b.kind != PartKind::Normal {
            return Err(BooleanFailure::Refused(refused(
                "only printed parts can be booleaned",
            )));
        }
        let object_name = self.name_of(at);
        let name_of = |part: &Part| {
            part.name
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| object_name.clone())
        };
        let result =
            edge_mesh::boolean::boolean(&in_object(a), &in_object(b), op).map_err(|reason| {
                let part = match &reason {
                    BooleanError::Input {
                        operand,
                        error: InputError::NotClosed | InputError::Empty,
                    } => Some(name_of(if *operand == Operand::First { a } else { b })),
                    _ => None,
                };
                BooleanFailure::Failed { part, reason }
            })?;
        let Some((low, high)) = result.bounds() else {
            return Err(BooleanFailure::Empty);
        };
        // Stored about the middle of its box, as `add_volume` centres a
        // new volume's mesh (`center_geometry_after_creation`).
        let middle: [f64; 3] =
            std::array::from_fn(|axis| (f64::from(low[axis]) + f64::from(high[axis])) / 2.0);
        let mut mesh = result;
        for vertex in &mut mesh.vertices {
            for axis in 0..3 {
                vertex[axis] = (f64::from(vertex[axis]) - middle[axis]) as f32;
            }
        }
        let made = Part {
            name: Some(format!("{} - {}", name_of(a), suffix(op))),
            mesh: Arc::new(mesh),
            paint: None,
            seam: None,
            transform: Transform::translation(middle),
            decoration: false,
            // Its settings go with it, not what its triangles carried.
            kept: a.kept.as_deref().map(|kept| Arc::new(kept.piece())),
            ..a.clone()
        };
        self.settle(bed);
        let id = fresh();
        let mut list = shape.parts.clone();
        let mut ids = shape.ids.clone();
        list[first] = made;
        ids[first] = id;
        if delete_tool || op == Op::Union {
            list.remove(second);
            ids.remove(second);
            self.last_part_to_object(at, &mut list);
        }
        self.reshape(bed, at, list, ids);
        Ok(id)
    }
}
