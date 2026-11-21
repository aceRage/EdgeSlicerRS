//! Where the reference has each part of a copy when it cuts it: each
//! part's mesh about the middle of its own box, in single precision; the
//! part's map to the frame of its object, about the middle of the
//! object's box; and where that middle stands on the bed, in whole
//! coordinate units ([`edge_mesh::slicing`]).

use edge_mesh::{Mesh, SlicingMesh, Transform, slicing::centred};

use crate::{Instance, Object, PartKind, new_scale};

/// One copy of an object as the reference places it for cutting.
pub struct Placement {
    /// Where the middle of the object's box stands for this copy, in
    /// coordinate units from the project's origin (`PrintInstance::shift`).
    pub shift: [i64; 2],
    /// For each of the object's parts, in its order: the part's mesh
    /// about the middle of its own box, and its map from there to the
    /// object's frame of cutting (`trafo_centered() * volume matrix`).
    pub parts: Vec<(Mesh, Transform)>,
}

impl Placement {
    /// Part `at` moved for cutting, by itself.
    pub fn cut(&self, at: usize) -> SlicingMesh {
        let mut cut = SlicingMesh::at(self.shift);
        let (local, to_object) = &self.parts[at];
        cut.append(local, to_object);
        cut
    }

    /// Every part of `kind` moved for cutting, together; none when the
    /// object has no part of that kind.
    pub fn cut_all(&self, object: &Object, kind: PartKind) -> Option<SlicingMesh> {
        let mut cut = SlicingMesh::at(self.shift);
        for (part, (local, to_object)) in object.parts.iter().zip(&self.parts) {
            if part.kind == kind {
                cut.append(local, to_object);
            }
        }
        (!cut.mesh.triangles.is_empty()).then_some(cut)
    }
}

/// A model of one part, `mesh` as its file has it, standing `offset` from
/// where the file has it, cut as the reference cuts an object of one part
/// ([`placement`]): an STL or OBJ, or a project's object of one part,
/// moved there.
pub fn lone_cut(mesh: &Mesh, offset: [f64; 3]) -> SlicingMesh {
    let object = Object {
        id: 0,
        name: None,
        filament: None,
        parts: vec![crate::Part {
            name: None,
            kind: PartKind::Normal,
            filament: 1,
            own_filament: None,
            mesh: std::sync::Arc::new(mesh.clone()),
            paint: None,
            seam: None,
            transform: Transform::IDENTITY,
            decoration: false,
            text: None,
            kept: None,
        }],
        instances: vec![Instance {
            transform: Transform::translation(offset),
            printable: true,
        }],
        kept: None,
    };
    placement(&object, &object.instances[0]).cut(0)
}

/// `instance` of `object` placed as the reference places it.
///
/// - Each part is made as `ModelObject::add_volume` makes it: its mesh
///   moved to the middle of its box ([`centred`]), its map the file's
///   (the component's) after that middle (bbs_3mf.cpp
///   `_generate_volumes_new`: `comp_transformation *
///   volume->get_transformation()`).
/// - An object of one part has the part's offset moved into each copy's
///   (bbs_3mf.cpp, "If instances contain a single volume, the volume
///   offset should be 0,0,0"), when the part does not already stand where
///   its first copy does (Eigen `isApprox`).
/// - A copy is cut without its offset across the bed
///   (PrintApply.cpp `print_objects_from_model_object`, the offset cut off
///   toward zero into the copy's shift), about the middle of the box of
///   the object's printed parts under its first copy without the offset
///   (`ModelObject::raw_bounding_box`, in double precision), turned about
///   Z by how far this copy is turned from the first; that middle, cut
///   off toward zero, is added to the shift (PrintObject.cpp
///   `PrintObject::PrintObject`, `set_instances`, `trafo_centered`).
pub fn placement(object: &Object, instance: &Instance) -> Placement {
    let mut parts: Vec<(Mesh, Transform)> = (object.parts.iter())
        .map(|part| {
            let (local, middle) = centred(&part.mesh);
            (local, Transform::translation(middle).then(&part.transform))
        })
        .collect();
    let mut first = object
        .instances
        .first()
        .map_or(instance.transform, |first| first.transform);
    let mut this = instance.transform;
    if let [(_, volume)] = parts.as_mut_slice() {
        let world_volume = volume.then(&first).rows[3];
        let world_instance = first.rows[3];
        let norm = |v: [f64; 3]| v.iter().map(|c| c * c).sum::<f64>().sqrt();
        let apart = norm(std::array::from_fn(|c| world_volume[c] - world_instance[c]));
        if apart > 1e-12 * norm(world_volume).min(norm(world_instance)) {
            first.rows[3] = world_volume;
            this.rows[3] = volume.then(&this).rows[3];
            volume.rows[3] = [0.0; 3];
        }
    }
    let offset = this.rows[3];
    let mut first_linear = first;
    first_linear.rows[3] = [0.0; 3];
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (part, (local, volume)) in object.parts.iter().zip(&parts) {
        if part.kind != PartKind::Normal {
            continue;
        }
        let placed = volume.then(&first_linear);
        for vertex in &local.vertices {
            let p = placed.apply(vertex.map(f64::from));
            for c in 0..2 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
    }
    let middle = if lo[0] > hi[0] {
        [0, 0]
    } else {
        let (mut x, mut y) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
        // How far this copy is turned about Z from the first: the turn
        // that takes the first's linear map to this one's. Copies that
        // differ in anything else are cut apart by the reference too, and
        // it reads the turn about Z alone off them as well.
        let mut linear = this;
        linear.rows[3] = [0.0; 3];
        if let Some(back) = first_linear.inverse() {
            let turn = back.then(&linear).rows;
            let angle = turn[0][1].atan2(turn[0][0]);
            // libslic3r.h `EPSILON`.
            if angle.abs() > 1e-4 {
                let (sin, cos) = angle.sin_cos();
                (x, y) = (cos * x - sin * y, sin * x + cos * y);
            }
        }
        [new_scale(x), new_scale(y)]
    };
    // The copy's map without its offset across the bed, then the middle
    // taken off (`trafo_centered`: `unscale` multiplies by the scaling
    // factor).
    let mut centring = this;
    centring.rows[3] = [
        -(middle[0] as f64 * 0.000_001),
        -(middle[1] as f64 * 0.000_001),
        offset[2],
    ];
    for (_, volume) in &mut parts {
        *volume = volume.then(&centring);
    }
    Placement {
        shift: [
            new_scale(offset[0]) + middle[0],
            new_scale(offset[1]) + middle[1],
        ],
        parts,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use edge_mesh::primitives::cuboid;

    use super::*;
    use crate::Part;

    fn part(low: [f32; 3], high: [f32; 3], filament: usize) -> Part {
        Part {
            name: None,
            kind: PartKind::Normal,
            filament,
            own_filament: Some(filament),
            mesh: Arc::new(Mesh::from_triangle_soup(cuboid(low, high))),
            paint: None,
            seam: None,
            transform: Transform::IDENTITY,
            decoration: false,
            text: None,
            kept: None,
        }
    }

    fn object(parts: Vec<Part>, at: [f64; 3]) -> Object {
        Object {
            id: 1,
            name: None,
            filament: None,
            parts,
            instances: vec![Instance {
                transform: Transform::translation(at),
                printable: true,
            }],
            kept: None,
        }
    }

    /// The extent across the bed of part `at` as cut, in coordinate units
    /// from the object's middle.
    fn across(placement: &Placement, at: usize) -> (f32, f32) {
        let cut = placement.cut(at);
        let xs = cut.mesh.vertices.iter().map(|v| v[0]);
        (
            xs.clone().fold(f32::INFINITY, f32::min),
            xs.fold(f32::NEG_INFINITY, f32::max),
        )
    }

    /// m2b_three: three blocks side by side in the file's own coordinates,
    /// 155 to 168.3, 168.3 to 181.7 and 181.7 to 195 mm (single
    /// precision). The pinned build, under lldb, measures its first
    /// layer's pieces (after 0.15 mm of elephant foot compensation on the
    /// outside) from -19849992 to -6699997, from -6699997 to 6699989 and
    /// from 6699989 to 19849992 units about the object's middle, 175 mm
    /// across: the end blocks 8 units apart in width, as each part is
    /// centred in single precision about its own middle first.
    #[test]
    fn parts_are_cut_where_the_reference_cuts_them() {
        let threetone = object(
            vec![
                part([155.0, 150.0, 0.0], [168.3, 170.0, 6.0], 1),
                part([168.3, 150.0, 0.0], [181.7, 170.0, 4.0], 2),
                part([181.7, 150.0, 0.0], [195.0, 170.0, 2.0], 3),
            ],
            [0.0; 3],
        );
        let placement = placement(&threetone, &threetone.instances[0]);
        assert_eq!(placement.shift, [175_000_000, 160_000_000]);
        assert_eq!(across(&placement, 0), (-19_999_992.0, -6_699_989.0));
        assert_eq!(across(&placement, 1), (-6_699_997.0, 6_699_997.0));
        assert_eq!(across(&placement, 2), (6_699_989.0, 19_999_992.0));
    }

    /// An object of one part has the part's place moved into the copy's
    /// offset: it is cut about its own middle, and the copy's shift is
    /// where that middle stands, the offset cut off toward zero.
    #[test]
    fn a_lone_part_moves_its_place_into_its_copy() {
        let block = object(
            vec![part([0.0, 0.0, 0.0], [10.3, 10.0, 2.0], 1)],
            [100.0, 50.0, 0.0],
        );
        let placement = placement(&block, &block.instances[0]);
        assert_eq!(placement.parts[0].1.rows[3], [0.0, 0.0, 1.0]);
        let middle = 100.0 + (0.0 + f64::from(10.3f32)) / 2.0;
        assert_eq!(placement.shift, [new_scale(middle), 55_000_000]);
        let (low, high) = across(&placement, 0);
        assert_eq!(low, -high);
    }
}
