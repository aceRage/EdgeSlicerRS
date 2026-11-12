//! A model's objects as meshes to write out: "Export all objects as one
//! STL..." and "Export all objects as STLs..." (Plater.cpp
//! `Plater::export_stl` with `mesh_to_export_fff_no_boolean`, the
//! reference's choice for a printer of filament).
//!
//! As the reference makes them: every object, every copy of it (printable
//! or not), on every plate; an object is its printed parts ("Part"s, not
//! modifiers, support blockers or enforcers, seam parts), each placed by
//! its own transform and then by the copy's, in the project's coordinates
//! (where each plate stands in the reference's grid of plates), in
//! millimetres. The parts are put side by side, not welded or joined, as
//! `csgmesh_merge_positive_parts` puts them.
//!
//! An object with negative parts is where this differs: the reference cuts
//! them out of it (CGAL's boolean difference) and, when that fails, keeps
//! the printed parts alone and says "Unable to perform boolean operation on
//! model meshes. Only positive parts will be exported.". There is no solid
//! boolean here, so it is always the second: the printed parts, and the
//! object named in [`Exported::uncut`] for the caller to say so.

pub use edge_mesh::Mesh;
/// A mesh as a binary STL, as the reference writes one (an empty header,
/// the triangles' normals from their corners).
pub use edge_mesh::stl::to_binary as stl_binary;

use crate::{Model, ModelObject, PartKind};

/// What an export holds.
#[derive(Debug, Default)]
pub struct Exported {
    pub mesh: Mesh,
    /// The objects whose negative parts were not cut out of them, by name.
    pub uncut: Vec<String>,
}

/// One object, every copy of it, as one mesh.
pub fn object_mesh(object: &ModelObject) -> Exported {
    let mut mesh = Mesh::default();
    for copy in object.copies() {
        for part in object.shape().parts() {
            if part.kind == PartKind::Normal {
                mesh.append(&part.mesh, &part.transform.then(&copy.world()));
            }
        }
    }
    let negative = object
        .shape()
        .parts()
        .iter()
        .any(|part| part.kind == PartKind::Negative);
    Exported {
        mesh,
        uncut: if negative {
            vec![object.name().to_owned()]
        } else {
            Vec::new()
        },
    }
}

/// Every object of the model as one mesh, in the model's order.
pub fn model_mesh(model: &Model) -> Exported {
    let mut all = Exported::default();
    for object in model.objects() {
        let one = object_mesh(object);
        all.mesh.append(&one.mesh, &edge_mesh::Transform::IDENTITY);
        all.uncut.extend(one.uncut);
    }
    all
}

/// A file name from an object's name: what no file system the app runs
/// on takes is put as `_`, the ends trimmed, "object" for one with no
/// name. The reference names the file by the object's name as it is
/// (`get_save_file`), so a name with a `/` in it is not written.
pub fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let stem = stem.trim().trim_end_matches('.').trim();
    if stem.is_empty() {
        "object".to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_keep_what_a_file_system_takes() {
        assert_eq!(file_stem("bracket / v2"), "bracket _ v2");
        assert_eq!(file_stem("  "), "object");
        assert_eq!(file_stem("a:b*c?.stl"), "a_b_c_.stl");
        assert_eq!(file_stem("end."), "end");
    }
}
