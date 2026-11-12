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
//! An object with negative parts has them cut out of it, as the reference
//! does (`csg::perform_csgmesh_booleans`): its printed and negative parts
//! are combined in their order, each printed part united with what is
//! there, each negative part taken away from it ([`edge_mesh::boolean::csg`]),
//! once in the object's own coordinates, and the solid that makes is placed
//! by each copy. When the boolean refuses (a part does not bound a volume,
//! say), the printed parts are kept alone, side by side, and the object is
//! named in [`Exported::uncut`] for the caller to say the reference's
//! "Unable to perform boolean operation on model meshes. Only positive
//! parts will be exported.".
//!
//! Where this differs from the reference on purpose: it refuses an object
//! with a part that crosses itself (CGAL's `does_self_intersect` in
//! `check_csgmesh_booleans`), which real meshes often do in a few tiny
//! places (the 3DBenchy does), and exports it uncut; here such a part is
//! taken as it is and the boolean checked to add up (see
//! [`edge_mesh::boolean`]), and it is refused only when it does not.

pub use edge_mesh::Mesh;
/// A mesh as a binary STL, as the reference writes one (an empty header,
/// the triangles' normals from their corners).
pub use edge_mesh::stl::to_binary as stl_binary;

use edge_mesh::boolean::{self, Op};

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
    let parts = object.shape().parts();
    let negative = parts.iter().any(|part| part.kind == PartKind::Negative);
    if negative && let Ok(solid) = cut(object) {
        let mut mesh = Mesh::default();
        for copy in object.copies() {
            mesh.append(&solid, &copy.world());
        }
        return Exported {
            mesh,
            uncut: Vec::new(),
        };
    }
    let mut mesh = Mesh::default();
    for copy in object.copies() {
        for part in parts {
            if part.kind == PartKind::Normal {
                mesh.append(&part.mesh, &part.transform.then(&copy.world()));
            }
        }
    }
    Exported {
        mesh,
        uncut: if negative {
            vec![object.name().to_owned()]
        } else {
            Vec::new()
        },
    }
}

/// An object's printed parts less its negative parts, in its own
/// coordinates, combined in the parts' order (Plater.cpp
/// `mesh_to_export_fff_no_boolean`, ModelToCSGMesh.hpp
/// `model_to_csgmesh`): a negative part takes away from the parts before
/// it, not from those after.
pub fn cut(object: &ModelObject) -> Result<Mesh, boolean::CsgError> {
    let placed: Vec<(Op, Mesh)> = (object.shape().parts().iter())
        .filter_map(|part| {
            let op = match part.kind {
                PartKind::Normal => Op::Union,
                PartKind::Negative => Op::Difference,
                _ => return None,
            };
            let mut mesh = Mesh::default();
            mesh.append(&part.mesh, &part.transform);
            Some((op, mesh))
        })
        .collect();
    boolean::csg(
        placed.iter().map(|(op, mesh)| (*op, mesh)),
        &boolean::Options::default(),
    )
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
    use crate::model::NewPart;
    use edge_mesh::Transform;
    use edge_mesh::boolean::check;

    fn cube(min: [f32; 3], max: [f32; 3]) -> Mesh {
        Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(min, max))
    }

    /// A model of one object, a 20 × 20 × 10 block, from an STL.
    fn block() -> (tempfile::TempDir, Model) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("block.stl");
        let mesh = cube([0.0; 3], [20.0, 20.0, 10.0]);
        std::fs::write(&path, stl_binary(&mesh)).unwrap();
        let model = Model::open(&path).unwrap();
        (dir, model)
    }

    fn add(model: &mut Model, kind: PartKind, mesh: Mesh) {
        let object = model.objects()[0].id();
        let part = NewPart {
            name: "part".into(),
            kind,
            mesh,
            transform: Transform::IDENTITY,
        };
        model.add_part(None, object, part).unwrap();
    }

    #[test]
    fn negative_parts_are_cut_out() {
        let (_dir, mut model) = block();
        let before = model_mesh(&model);
        assert!(before.uncut.is_empty());
        // Corners of the object's own frame, wherever the copy stands.
        let (low, _) = model.objects()[0].shape().parts()[0].mesh.bounds().unwrap();
        let at = |p: [f32; 3]| std::array::from_fn(|i| low[i] + p[i]);
        // A hole through the block, and a pocket open at the top.
        let hole = cube(at([5.0, 5.0, -1.0]), at([8.0, 8.0, 11.0]));
        add(&mut model, PartKind::Negative, hole);
        let pocket = cube(at([12.0, 12.0, 5.0]), at([16.0, 16.0, 12.0]));
        add(&mut model, PartKind::Negative, pocket);
        let exported = model_mesh(&model);
        assert!(exported.uncut.is_empty());
        assert!(check::defects(&exported.mesh).is_clean());
        let volume = exported.mesh.signed_volume();
        assert!((volume - (4000.0 - 90.0 - 80.0)).abs() < 1e-3, "{volume}");
        let one = object_mesh(&model.objects()[0]);
        assert_eq!(one.mesh.triangles.len(), exported.mesh.triangles.len());
    }

    #[test]
    fn a_negative_part_that_bounds_nothing_leaves_the_object_uncut() {
        let (_dir, mut model) = block();
        let (low, _) = model.objects()[0].shape().parts()[0].mesh.bounds().unwrap();
        let at = |p: [f32; 3]| std::array::from_fn(|i| low[i] + p[i]);
        let mut open = cube(at([5.0, 5.0, -1.0]), at([8.0, 8.0, 11.0]));
        open.triangles.truncate(10);
        add(&mut model, PartKind::Negative, open);
        let exported = model_mesh(&model);
        assert_eq!(exported.uncut, vec![model.objects()[0].name().to_owned()]);
        // The printed part alone.
        assert!((exported.mesh.signed_volume() - 4000.0).abs() < 1e-3);
    }

    #[test]
    fn file_names_keep_what_a_file_system_takes() {
        assert_eq!(file_stem("bracket / v2"), "bracket _ v2");
        assert_eq!(file_stem("  "), "object");
        assert_eq!(file_stem("a:b*c?.stl"), "a_b_c_.stl");
        assert_eq!(file_stem("end."), "end");
    }
}
