//! "Cut" (the reference's planar cut, GLGizmoCut.cpp `perform_cut`,
//! CutUtils.cpp `Cut::perform_with_plane`): an object cut by a plane into
//! the part above it and the part under it, kept as two objects or as two
//! parts of one ("Cut to parts").
//!
//! What happens to each part of the object, as the reference does it:
//!
//! - a printed part is cut ([`edge_mesh::cut`]); each piece keeps its name
//!   (with `_A` above and `_B` under for "Cut to parts"), its filament and
//!   its settings, and what was painted on it where the piece is the
//!   triangle it was; a piece of a triangle that was painted takes the
//!   paint at its middle, the cap none (the fork carries paint over a cut:
//!   `KeepPaint`, CutUtils.cpp `Cut::finalize`);
//! - a modifier, a negative part, a support blocker or enforcer is not
//!   cut: it goes whole to each side it reaches into (by the box round it),
//!   and is left out of a side whose printed material it does not reach
//!   (`process_modifier_cut`, `delete_extra_modifiers`); with "Cut to
//!   parts" every one stays with the one object;
//! - the copy cut has the turn and size it stood with put into the new
//!   meshes, and stands where it stood with no turn of its own, or turned
//!   to lie on its cut face ("Place on cut") or upside down ("Flip");
//!   every copy is then stood on the bed again (`reset_instance_transformation`,
//!   `load_model_objects`);
//! - the objects made keep the object's settings and height ranges (the
//!   reference's halves are copies of it: `clone_for_cut`).
//!
//! Where this differs, on purpose:
//!
//! - the other copies of the object keep their own turn and size relative
//!   to the copy that was cut; the reference gives each its own turn about
//!   z on top of the cut copy's turn, which is baked into the halves, so a
//!   copy of a turned object comes out turned twice;
//! - a side with no printed part is not made into an object; the reference
//!   keeps an object whose volumes are only modifiers, and then deletes
//!   those, leaving an object with nothing in it;
//! - the objects made stand where the cut object stood in the list.

use std::sync::Arc;

use edge_3mf::{PaintKind, Part, PartKind};
use edge_mesh::{
    Transform,
    cut::{self, Half},
    selector::code,
};

use super::{Bed, Model, ModelCopy, ModelObject, Shape, fresh, refused};
use crate::Failure;

/// Which side a cut's thickness is taken from (the reference's
/// `CutThicknessOffset`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CutThickness {
    /// Half each side of the plane.
    #[default]
    Centred,
    /// From above the plane: the lower part's face stays on it.
    Above,
    /// From under the plane: the upper part's face stays on it.
    Below,
}

/// The greatest cut thickness (CurvedCut.hpp `CutThicknessMax`).
pub const CUT_THICKNESS_MAX: f64 = 20.0;

/// What a cut keeps and how it lays what it keeps ("After cut").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutOptions {
    pub keep_upper: bool,
    pub keep_lower: bool,
    pub place_on_cut_upper: bool,
    pub place_on_cut_lower: bool,
    pub flip_upper: bool,
    pub flip_lower: bool,
    /// "Cut to parts": one object of the two parts, each part's pieces
    /// named `_A` and `_B`.
    pub to_parts: bool,
    /// How much is cut away, in mm along the plane's normal.
    pub thickness: f64,
    pub thickness_from: CutThickness,
}

impl Default for CutOptions {
    fn default() -> Self {
        Self {
            keep_upper: true,
            keep_lower: true,
            place_on_cut_upper: false,
            place_on_cut_lower: false,
            flip_upper: false,
            flip_lower: false,
            to_parts: false,
            thickness: 0.0,
            thickness_from: CutThickness::Centred,
        }
    }
}

impl CutOptions {
    /// The heights of the lower part's and the upper part's cut faces over
    /// the plane (CurvedCut.cpp `curved_cut_thickness_faces`).
    pub fn faces(&self) -> [f64; 2] {
        let t = self.thickness.max(0.0);
        match self.thickness_from {
            CutThickness::Centred => [-t / 2.0, t / 2.0],
            CutThickness::Above => [0.0, t],
            CutThickness::Below => [-t, 0.0],
        }
    }
}

/// What a cut made.
#[derive(Clone, Debug, Default)]
pub struct CutDone {
    /// The copies of the objects made, which the reference selects.
    pub made: Vec<u64>,
    /// How many chains of cut edges were left without a cap, the object
    /// being open where the plane crossed it.
    pub open: usize,
}

/// The section of an object by a plane, for the tool to show while the
/// plane is placed.
#[derive(Clone, Debug, Default)]
pub struct CutSection {
    /// Where the plane cuts the printed parts, as pairs of points in the
    /// scene.
    pub lines: Vec<[[f64; 3]; 2]>,
    /// How far the copy's printed material reaches above the plane and
    /// under it, in mm along the normal: the reference's tooltip on the
    /// plane's grip ("Object A", "Object B").
    pub above: f64,
    pub below: f64,
    /// The box round the copy's printed material in the plane's own
    /// coordinates (x, y in the plane, z along its normal), least and
    /// greatest corners: the reference's "Build Volume" of the cut
    /// (`m_transformed_bounding_box`). None for nothing printed.
    pub reach: Option<[[f64; 3]; 2]>,
}

/// Where a line of sight meets a copy's printed parts ("Pick face"): the
/// point, the face's outward normal and its corners, in the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceHit {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub corners: [[f64; 3]; 3],
}

/// The linear part of a transform, its move left out.
fn linear(transform: &Transform) -> Transform {
    let mut out = *transform;
    out.rows[3] = [0.0; 3];
    out
}

/// A plane's frame (rows: its x, y and normal, then its middle) checked:
/// finite, its axes of unit length and square to one another, turning the
/// right way.
fn sound_plane(plane: &Transform) -> bool {
    let r = &plane.rows;
    if !r.iter().flatten().all(|value| value.is_finite()) {
        return false;
    }
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    (0..3).all(|i| (dot(r[i], r[i]) - 1.0).abs() < 1e-6)
        && (dot(r[0], r[1]).abs() < 1e-6)
        && (dot(r[1], r[2]).abs() < 1e-6)
        && (dot(r[0], r[2]).abs() < 1e-6)
        && plane.determinant() > 0.0
}

impl Model {
    /// The section of a copy's object by `plane` (its frame in the scene:
    /// rows x, y, normal, middle), and how far the object reaches each
    /// side of it.
    pub fn cut_section(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
    ) -> Result<CutSection, Failure> {
        let (at, _) = self.copy_at(copy)?;
        if !sound_plane(plane) {
            return Err(refused("that is no plane to cut by"));
        }
        // A bare model is shown where an edit would settle it (in the
        // middle of the bed): measured there too.
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.cut_section(bed, copy, plane);
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let to_plane = plane
            .inverse()
            .ok_or_else(|| refused("that is no plane to cut by"))?;
        let mut out = CutSection {
            lines: Vec::new(),
            above: 0.0,
            below: 0.0,
            reach: None,
        };
        for part in self.objects[at].shape.parts.iter() {
            if part.kind != PartKind::Normal {
                continue;
            }
            let placed = part.transform.then(&scene);
            let into = placed.then(&to_plane);
            for line in cut::section(&part.mesh, &into) {
                out.lines.push(line.map(|p| plane.apply(p)));
            }
            for vertex in &part.mesh.vertices {
                let p = into.apply(vertex.map(f64::from));
                out.above = out.above.max(p[2]);
                out.below = out.below.max(-p[2]);
                let [low, high] = out.reach.get_or_insert([p, p]);
                for axis in 0..3 {
                    low[axis] = low[axis].min(p[axis]);
                    high[axis] = high[axis].max(p[axis]);
                }
            }
        }
        Ok(out)
    }

    /// The face of a copy's printed parts a line of sight from `origin`
    /// along `direction` (both in the scene) meets first, from either side
    /// (FacetPicker.cpp `update`: the nearest hit of any model part).
    pub fn face_hit(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        origin: [f64; 3],
        direction: [f64; 3],
    ) -> Result<Option<FaceHit>, Failure> {
        let (at, _) = self.copy_at(copy)?;
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.face_hit(bed, copy, origin, direction);
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let shape = &self.objects[at].shape;
        let trees = shape.trees.get_or_init(|| {
            shape
                .parts
                .iter()
                .map(|part| {
                    (part.kind == PartKind::Normal)
                        .then(|| edge_mesh::raycast::Tree::new(&part.mesh))
                })
                .collect()
        });
        let mut best: Option<(f64, FaceHit)> = None;
        for (part, tree) in shape.parts.iter().zip(trees) {
            let Some(tree) = tree else { continue };
            let placed = part.transform.then(&scene);
            let Some(back) = placed.inverse() else {
                continue;
            };
            let local_origin = back.apply(origin);
            let ahead = back.apply([
                origin[0] + direction[0],
                origin[1] + direction[1],
                origin[2] + direction[2],
            ]);
            let local_direction: [f64; 3] =
                std::array::from_fn(|axis| ahead[axis] - local_origin[axis]);
            let Some(hit) = tree.hit(local_origin, local_direction, (0.0, f64::INFINITY), &|_| {
                true
            }) else {
                continue;
            };
            let corners = part
                .mesh
                .triangle(hit.triangle)
                .map(|p| placed.apply(p.map(f64::from)));
            let point = placed.apply(hit.point);
            let distance: f64 = (0..3)
                .map(|axis| (point[axis] - origin[axis]).powi(2))
                .sum();
            if best.as_ref().is_some_and(|(far, _)| *far <= distance) {
                continue;
            }
            let u: [f64; 3] = std::array::from_fn(|axis| corners[1][axis] - corners[0][axis]);
            let v: [f64; 3] = std::array::from_fn(|axis| corners[2][axis] - corners[0][axis]);
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if length == 0.0 {
                continue;
            }
            // A mirrored part's triangles face in: its normal is turned.
            let sign = if placed.is_mirrored() { -1.0 } else { 1.0 };
            let normal = n.map(|value| sign * value / length);
            best = Some((
                distance,
                FaceHit {
                    point,
                    normal,
                    corners,
                },
            ));
        }
        Ok(best.map(|(_, hit)| hit))
    }

    /// Cuts a copy's object by `plane` (its frame in the scene: rows x, y,
    /// the normal, which points into the upper part, then its middle), as
    /// `options` say. The object is replaced by what is kept of it, in its
    /// place in the list, on the plates its copies were on. Refused,
    /// nothing changed, when nothing would be kept.
    pub fn cut(
        &mut self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
        options: &CutOptions,
    ) -> Result<CutDone, Failure> {
        let (at, which) = self.copy_at(copy)?;
        if !sound_plane(plane) {
            return Err(refused("that is no plane to cut by"));
        }
        if !(0.0..=CUT_THICKNESS_MAX).contains(&options.thickness) {
            return Err(refused(format!(
                "a cut is from 0 to {CUT_THICKNESS_MAX} mm thick"
            )));
        }
        let to_parts = options.to_parts;
        let keep_upper = options.keep_upper || to_parts;
        let keep_lower = options.keep_lower || to_parts;
        if !keep_upper && !keep_lower {
            return Err(refused("Select at least one object to keep after cutting."));
        }
        self.settle(bed);
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        // The halves are made in the copy's coordinates with its move left
        // out: its turn and size go into their meshes, as the reference
        // cuts each volume in `instance_matrix * volume_matrix`.
        let offset = scene.rows[3];
        let turned = linear(&scene);
        let back = plane.then(&Transform::translation(offset.map(|value| -value)));
        let to_plane = back
            .inverse()
            .ok_or_else(|| refused("that is no plane to cut by"))?;
        let faces = options.faces();
        let held = self.objects[at].clone();
        let base = self.name_of(at);

        // Each part's pieces, and where the others go.
        let mut upper: Vec<Part> = Vec::new();
        let mut lower: Vec<Part> = Vec::new();
        let mut helpers: Vec<(Part, bool, bool)> = Vec::new();
        let mut open = 0;
        for part in held.shape.parts.iter() {
            let placed = part.transform.then(&turned);
            if part.kind == PartKind::Normal {
                if part.mesh.triangles.is_empty() {
                    continue;
                }
                let into = placed.then(&to_plane);
                let made = cut::cut(&part.mesh, &into, faces, &back);
                open += made.open;
                // What was painted on it fits a whole triangle of a piece
                // only when its corners keep their order.
                let flipped = into.is_mirrored() != back.is_mirrored();
                let suffix = |mark: &str| {
                    part.name
                        .as_deref()
                        .filter(|name| !name.is_empty())
                        .map(|name| format!("{name}{mark}"))
                        .or_else(|| to_parts.then(|| format!("{base}{mark}")))
                };
                for (half, side, mark) in [(made.upper, true, "_A"), (made.lower, false, "_B")] {
                    if half.is_empty() || (side && !keep_upper) || (!side && !keep_lower) {
                        continue;
                    }
                    let name = if to_parts {
                        suffix(mark)
                    } else {
                        part.name.clone()
                    };
                    let piece = carry_paint(part, &placed, half, name, flipped);
                    if side {
                        upper.push(piece);
                    } else {
                        lower.push(piece);
                    }
                }
            } else {
                // Not cut: whole, its place in the copy put into its own.
                let into = placed.then(&to_plane);
                let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
                for vertex in &part.mesh.vertices {
                    let z = into.apply(vertex.map(f64::from))[2];
                    low = low.min(z);
                    high = high.max(z);
                }
                let crossed = low <= 0.0 && high >= 0.0;
                let moved = Part {
                    transform: placed,
                    ..part.clone()
                };
                helpers.push((moved, low >= 0.0 || crossed, high <= 0.0 || crossed));
            }
        }

        // The objects made, each as its parts with the copies it will have.
        let mut made_objects: Vec<(Vec<Part>, bool)> = Vec::new();
        if to_parts {
            let mut parts = upper;
            parts.extend(lower);
            if parts.is_empty() {
                return Err(refused("Cut plane is placed out of object"));
            }
            parts.extend(helpers.into_iter().map(|(part, _, _)| part));
            made_objects.push((parts, true));
        } else {
            for (mut parts, side) in [(upper, true), (lower, false)] {
                if parts.is_empty() {
                    continue;
                }
                let reach = reach_of(&parts);
                for (helper, up, down) in &helpers {
                    let goes = if side { *up } else { *down };
                    // Left out where it does not reach the printed material
                    // (`delete_extra_modifiers`).
                    if goes && reach.is_some_and(|reach| meets(&reach, helper)) {
                        parts.push(helper.clone());
                    }
                }
                made_objects.push((parts, side));
            }
            if made_objects.is_empty() {
                return Err(refused("Select at least one object to keep after cutting."));
            }
        }

        // Where the copies stand: the copy cut as it stood, without its turn
        // and size (they are in the meshes now); the others keep theirs
        // relative to it. Then turned for "Place on cut" or "Flip".
        let undo = turned
            .inverse()
            .ok_or_else(|| refused("the object is squashed flat"))?;
        let mut objects: Vec<ModelObject> = Vec::new();
        let mut instead: Vec<(u64, Vec<u64>)> = held
            .copies
            .iter()
            .map(|copy| (copy.id, Vec::new()))
            .collect();
        for (parts, side) in made_objects {
            let shape = Shape::new(parts);
            let about = if to_parts {
                Transform::IDENTITY
            } else {
                let place = if side {
                    options.place_on_cut_upper
                } else {
                    options.place_on_cut_lower
                };
                let flip = if side {
                    options.flip_upper
                } else {
                    options.flip_lower
                };
                // The upper part's cut face looks along -normal, the lower
                // part's along +normal: laid on it, the face looks down
                // (`reset_instance_transformation`: the cut's turn undone,
                // then for the lower part half a turn about x as well).
                let upside_down = rotation_x_half();
                let undo_plane = linear(plane).inverse().unwrap_or(Transform::IDENTITY);
                let turn = match (place, flip, side) {
                    (true, _, true) => Some(undo_plane),
                    (true, _, false) => Some(undo_plane.then(&upside_down)),
                    (false, true, _) => Some(upside_down),
                    _ => None,
                };
                // About the middle of the part, in its own coordinates.
                match (turn, shape.reach(&Transform::IDENTITY)) {
                    (Some(turn), Some([low, high])) => {
                        let middle: [f64; 3] =
                            std::array::from_fn(|axis| (low[axis] + high[axis]) / 2.0);
                        Transform::translation(middle.map(|value| -value))
                            .then(&turn)
                            .then(&Transform::translation(middle))
                    }
                    _ => Transform::IDENTITY,
                }
            };
            let copies: Vec<ModelCopy> = held
                .copies
                .iter()
                .enumerate()
                .zip(&mut instead)
                .map(|((place, old), (_, made))| {
                    let id = fresh();
                    made.push(id);
                    let relative = if place == which {
                        Transform::IDENTITY
                    } else {
                        undo.then(&linear(&old.local))
                    };
                    let mut local = about.then(&relative);
                    for axis in 0..3 {
                        local.rows[3][axis] += old.local.rows[3][axis];
                    }
                    ModelCopy {
                        id,
                        reach: shape.reach(&local),
                        local,
                        world: local,
                        printable: old.printable,
                    }
                })
                .collect();
            objects.push(ModelObject {
                id: fresh(),
                name: base.clone(),
                filament: held.filament,
                shape,
                copies,
                kept: held.kept.as_deref().map(|kept| Arc::new(kept.cut_half())),
            });
        }
        self.objects.remove(at);
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
        // Each copy stood on the bed where it is (its plate's place put
        // back first).
        let new: Vec<u64> = self.objects[at..at + count]
            .iter()
            .flat_map(|object| object.copies.iter().map(|copy| copy.id))
            .collect();
        for copy in &new {
            let plate = self.plate_of(*copy);
            if let Some((object, place)) = self.find(*copy) {
                let local = self.objects[object].copies[place].local;
                self.stand((object, place), plate, local);
            }
            if let Some((scene, _)) = self.in_scene(bed, *copy) {
                self.stand_in_scene(bed, *copy, scene);
            }
        }
        Ok(CutDone { made: new, open })
    }
}

/// Half a turn about x (the reference's `rotation_transform(PI * UnitX)`).
fn rotation_x_half() -> Transform {
    Transform {
        rows: [
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0; 3],
        ],
    }
}

/// The box round the printed parts of a list, where their transforms put
/// them.
fn reach_of(parts: &[Part]) -> Option<[[f64; 3]; 2]> {
    let mut reach: Option<[[f64; 3]; 2]> = None;
    for part in parts.iter().filter(|part| part.kind == PartKind::Normal) {
        for vertex in &part.mesh.vertices {
            let point = part.transform.apply(vertex.map(f64::from));
            let [low, high] = reach.get_or_insert([point, point]);
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
    }
    reach
}

/// Whether a part's box meets a box.
fn meets(reach: &[[f64; 3]; 2], part: &Part) -> bool {
    let Some(theirs) = reach_of(&[Part {
        kind: PartKind::Normal,
        ..part.clone()
    }]) else {
        return false;
    };
    (0..3).all(|axis| theirs[0][axis] <= reach[1][axis] && theirs[1][axis] >= reach[0][axis])
}

/// A piece of a part: its mesh, named as given, with what was painted on
/// the part carried over (see the module's own words). `placed` took the
/// part's mesh to the coordinates the piece is in.
fn carry_paint(
    part: &Part,
    placed: &Transform,
    half: Half,
    name: Option<String>,
    flipped: bool,
) -> Part {
    let mesh = Arc::new(half.mesh);
    let mut piece = Part {
        name,
        mesh: mesh.clone(),
        paint: None,
        seam: None,
        transform: Transform::IDENTITY,
        own_filament: part.own_filament,
        kept: part.kept.as_deref().map(|kept| Arc::new(kept.piece())),
        ..part.clone()
    };
    let back = placed.inverse();
    for kind in PaintKind::ALL {
        let mut codes = part.paint_codes(kind);
        codes.sort_by_key(|(triangle, _)| *triangle);
        if codes.is_empty() {
            continue;
        }
        let mut carried: Vec<(u32, String)> = Vec::new();
        for (at, origin) in half.from.iter().enumerate() {
            let Some(origin) = origin else { continue };
            let Ok(found) = codes.binary_search_by_key(&origin.triangle, |(t, _)| *t) else {
                continue;
            };
            let text = &codes[found].1;
            if origin.whole && !flipped {
                carried.push((at as u32, text.clone()));
                continue;
            }
            // A piece: the state at its middle, back in the part's mesh.
            let Some(back) = back else { continue };
            let corners = mesh.triangle(at).map(|p| p.map(f64::from));
            let middle: [f64; 3] = std::array::from_fn(|axis| {
                (corners[0][axis] + corners[1][axis] + corners[2][axis]) / 3.0
            });
            let middle = back.apply(middle);
            let whole = part.mesh.triangle(origin.triangle as usize);
            let Some(leaves) = code::pieces(text, whole) else {
                continue;
            };
            let state = leaves
                .iter()
                .find(|(leaf, _)| contains(leaf, middle))
                .map(|(_, state)| *state)
                .unwrap_or(0);
            if state != 0 {
                carried.push((at as u32, code::write(&[code::Node::Leaf(state)])));
            }
        }
        piece = piece.with_paint(kind, &carried);
    }
    piece
}

/// Whether a point (in the triangle's plane, or near it) lies in a
/// triangle, its edges included.
fn contains(triangle: &[[f32; 3]; 3], p: [f64; 3]) -> bool {
    let [a, b, c] = triangle.map(|v| v.map(f64::from));
    let sub = |x: [f64; 3], y: [f64; 3]| [x[0] - y[0], x[1] - y[1], x[2] - y[2]];
    let cross = |x: [f64; 3], y: [f64; 3]| {
        [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ]
    };
    let dot = |x: [f64; 3], y: [f64; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let n = cross(sub(b, a), sub(c, a));
    let scale = dot(n, n);
    if scale == 0.0 {
        return false;
    }
    let tolerance = -1e-9 * scale;
    dot(cross(sub(b, a), sub(p, a)), n) >= tolerance
        && dot(cross(sub(c, b), sub(p, b)), n) >= tolerance
        && dot(cross(sub(a, c), sub(p, c)), n) >= tolerance
}
