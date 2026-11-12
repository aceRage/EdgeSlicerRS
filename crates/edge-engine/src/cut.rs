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

use edge_3mf::{
    CutConnectorInfo, CutId, CutRecipe, ObjectKept, PaintKind, Part, PartKept, PartKind,
    RecipeConnector, RecipeGroove, RecipeKind,
};
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
#[derive(Clone, Debug, PartialEq)]
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
    /// Which side each piece of the cut goes to, upper (true) or lower,
    /// in the order of [`Model::cut_pieces`]: the reference's part
    /// selection, a piece given to the other side by the right button
    /// (GLGizmoCut.cpp `PartSelection`, CutUtils.cpp `perform_by_contour`).
    /// None: each piece on its own side.
    pub sides: Option<Vec<bool>>,
    /// "Dovetail": a tongue on the upper part and its groove in the lower
    /// (the reference's tongue-and-groove cut, `Cut::perform_with_groove`).
    pub groove: Option<Groove>,
    /// The values the dovetail's reset buttons go back to (depth, width,
    /// flap angle, groove angle), kept with the cut.
    pub groove_init: Option<[f64; 4]>,
    /// "Keep cut editable": the halves carry how they were cut and the
    /// object as it was, so the cut can be edited again ([`Model::cut_edit`]).
    pub editable: bool,
    /// The connectors placed on the cut ("Add connectors"). With any, both
    /// parts are kept as objects of their own, as the reference keeps them;
    /// a dovetail takes none.
    pub connectors: Vec<Connector>,
    /// A snap's slit and bulge, of its radius (`m_snap_space_proportion`,
    /// `m_snap_bulge_proportion`): the tool's, not each connector's.
    pub snap: [f64; 2],
    /// The nozzle's diameter, which a pin's slot is measured against.
    pub nozzle: f64,
    /// "Curved": the surface the cut follows, over the plane
    /// ([`crate::curved`]). A flat one is the plane cut itself.
    pub sheet: Option<crate::curved::Sheet>,
    /// "Draw": the line drawn on the part and how the surface is swept
    /// from it ([`crate::drawn`]). Wins over a curved sheet.
    pub drawn: Option<DrawnCut>,
}

/// A drawn cut: the line as the tool keeps it (the chain's samples, in
/// the plane's own coordinates) and the sweep. The cut thickness is the
/// cut's own ([`CutOptions::thickness`]).
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnCut {
    pub stroke: crate::drawn::RecipeStroke,
    pub params: crate::drawn::DrawCutParams,
}

impl DrawnCut {
    /// The line finished as the reference finishes it for the cut
    /// (`DrawCutChain::finish`, `DefaultSpacing`), or why it cannot be cut.
    pub fn finished(&self) -> Result<crate::drawn::DrawCutStroke, &'static str> {
        let chain = crate::drawn::recipe_stroke_to_chain(&self.stroke);
        let (stroke, error) = chain.finish(crate::drawn::DEFAULT_SPACING, self.stroke.smoothing);
        if error == crate::drawn::DrawCutError::None {
            Ok(stroke)
        } else {
            Err(crate::drawn::draw_cut_error_words(error))
        }
    }
}

/// What a connector is (Model.hpp `CutConnectorType`): a plug on the
/// lower part with its hole in the upper, a dowel of its own with a hole
/// in each, or a snap, a split plug that clicks into a round hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorKind {
    Plug,
    Dowel,
    Snap,
    /// The fork's push-fit pin: printed on its own, pushed into a hole in
    /// each half ([`crate::pushfit`]).
    Pin,
}

/// Its sides: straight, or narrowing (a plug's to half its radius at the
/// top, a dowel's to a point at each end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorStyle {
    Prism,
    Frustum,
}

/// Its outline across the plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorShape {
    Triangle,
    Square,
    Hexagon,
    Circle,
}

impl ConnectorKind {
    /// The reference's number for it.
    pub fn code(self) -> u32 {
        match self {
            Self::Plug => 0,
            Self::Dowel => 1,
            Self::Snap => 2,
            // 3 is BambuStudio's Thread, 4 the fork's Flexi.
            Self::Pin => 5,
        }
    }

    pub fn of(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Plug),
            1 => Some(Self::Dowel),
            2 => Some(Self::Snap),
            5 => Some(Self::Pin),
            _ => None,
        }
    }
}

impl ConnectorStyle {
    pub fn code(self) -> u32 {
        match self {
            Self::Prism => 0,
            Self::Frustum => 1,
        }
    }

    pub fn of(code: u32) -> Option<Self> {
        [Self::Prism, Self::Frustum].get(code as usize).copied()
    }
}

impl ConnectorShape {
    pub fn code(self) -> u32 {
        match self {
            Self::Triangle => 0,
            Self::Square => 1,
            Self::Hexagon => 2,
            Self::Circle => 3,
        }
    }

    pub fn of(code: u32) -> Option<Self> {
        [Self::Triangle, Self::Square, Self::Hexagon, Self::Circle]
            .get(code as usize)
            .copied()
    }

    /// How many corners its outline has.
    pub fn sides(self) -> usize {
        match self {
            Self::Triangle => 3,
            Self::Square => 4,
            Self::Hexagon => 6,
            Self::Circle => edge_mesh::connector::CIRCLE,
        }
    }
}

/// A connector placed on the cut (Model.hpp `CutConnector`): where it
/// stands on the plane, in the scene; its radius and height (how deep it
/// reaches into the part over the plane, or each way for a dowel), in mm;
/// what is added to the hole's radius and depth; its turn about the
/// plane's normal, in radians; and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Connector {
    pub at: [f64; 3],
    pub radius: f64,
    pub height: f64,
    pub radius_tolerance: f64,
    pub height_tolerance: f64,
    pub z_angle: f64,
    pub kind: ConnectorKind,
    pub style: ConnectorStyle,
    pub shape: ConnectorShape,
    /// A pin's shape; its radius and height above are then its own (half
    /// its diameter, its two lengths).
    pub pin: crate::pushfit::Pin,
}

/// How far a connector's hole reaches through the cut face, so that the
/// face is left open (CutUtils.hpp `CONNECTOR_HOLE_OVERLAP`).
pub const CONNECTOR_HOLE_OVERLAP: f64 = 0.05;

/// A dovetail's shape (the reference's `Cut::Groove`, with its count and
/// gap): in mm and radians, in the plane's own axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Groove {
    /// How deep the tongue is, along the normal.
    pub depth: f64,
    /// How wide its neck is, along the plane's x.
    pub width: f64,
    /// The angle of its sides to the plane.
    pub flaps_angle: f64,
    /// How much its sides lean towards each other along the plane's y.
    pub angle: f64,
    /// Taken off the tongue's depth and width, so that it fits its groove.
    pub depth_tolerance: f64,
    pub width_tolerance: f64,
    /// How many side by side, and how far apart.
    pub count: usize,
    pub gap: f64,
    /// The radius the dovetails are spaced by (`groove_radius`).
    pub plane_radius: f64,
}

impl Groove {
    /// The width a dovetail takes along the plane's x, its sides' lean
    /// included (`Cut::calculate_groove_width`).
    pub fn outer_width(&self) -> f64 {
        let flap = if self.flaps_angle.abs() < 1e-12 {
            self.depth
        } else {
            self.depth / self.flaps_angle.sin()
        };
        let flaps = 2.0 * flap * self.flaps_angle.cos();
        let neck = 0.5 * self.width;
        let mouth = 0.5 * (self.width + flaps);
        let half_height = 0.5 * (1.5 * (1.5 * self.plane_radius));
        let taper = half_height * self.angle.tan();
        2.0 * (mouth + taper).max(neck + taper)
    }

    /// Whether the shape can be cut (`has_valid_groove`, its first test):
    /// the flaps do not meet before the neck.
    pub fn sound(&self) -> bool {
        let flaps = -2.0 * self.depth / self.flaps_angle.tan();
        self.depth > 0.0 && self.width > 0.0 && self.count >= 1 && flaps <= self.width
    }
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
            sides: None,
            groove: None,
            groove_init: None,
            editable: true,
            connectors: Vec::new(),
            snap: [0.3, 0.15],
            nozzle: 0.4,
            sheet: None,
            drawn: None,
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
    /// A curved cut's sides with nothing in them (upper, lower): the cut
    /// still makes what it can (`perform_cut`'s note).
    pub empty: Option<[bool; 2]>,
    /// The pins made, how many print lying, and how many were made
    /// without the slit asked for (the boolean failed).
    pub pins: usize,
    pub pins_lying: usize,
    pub pin_slot_failures: usize,
    /// How many chains of cut edges were left without a cap, the object
    /// being open where the plane crossed it.
    pub open: usize,
}

/// A copy's printed parts made ready for a curved cut ([`Model::cut_prepared`]).
#[derive(Clone)]
pub struct Prepared {
    /// Each part's, by its place; none for one that is not printed.
    solids: Vec<Option<edge_mesh::boolean::Solid>>,
    /// Where the copy stood when they were made.
    scene: Transform,
}

/// A cut put back to be made again ([`Model::cut_edit`]).
#[derive(Clone, Debug)]
pub struct CutEdit {
    /// The copy of the object as it was before the cut, which stands in
    /// for its halves.
    pub copy: u64,
    /// The plane the cut was made by, in the scene.
    pub plane: Transform,
    /// How it was made.
    pub recipe: Arc<CutRecipe>,
    /// Whether fewer halves were left than the cut made: cutting again
    /// makes them all.
    pub missing: bool,
}

/// The pieces a cut makes: each half's parts split into the shells that
/// share no edge (`PartSelection`'s `volumes[id]->split(1)`), each on the
/// side it was cut on, in the order the parts are and upper before lower.
#[derive(Clone, Debug, Default)]
pub struct CutPieces {
    /// Every piece, one mesh, in the copy's coordinates with its move left
    /// out (what [`CutPieces::offset`] takes to the scene).
    pub mesh: edge_mesh::Mesh,
    /// The piece each triangle of `mesh` is of.
    pub piece_of: Vec<u32>,
    /// The side each piece was cut on: upper (true) or lower.
    pub upper: Vec<bool>,
    /// The copy's move in the scene.
    pub offset: [f64; 3],
}

/// What cutting each part makes, before it is made into objects.
struct Made {
    upper: Vec<Part>,
    lower: Vec<Part>,
    /// A curved cut's sides that came out empty: upper, lower.
    empty: [bool; 2],
    helpers: Vec<(Part, bool, bool)>,
    open: usize,
    turned: Transform,
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

    /// The cut face a plane makes of a copy's printed parts, as triangles
    /// in the scene: what the connectors are placed on (the reference
    /// draws it, `render_clipper_cut`, while they are edited).
    pub fn cut_cap(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
    ) -> Result<Vec<[[f64; 3]; 3]>, Failure> {
        let (at, _) = self.copy_at(copy)?;
        if !sound_plane(plane) {
            return Err(refused("that is no plane to cut by"));
        }
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.cut_cap(bed, copy, plane);
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let to_plane = plane
            .inverse()
            .ok_or_else(|| refused("that is no plane to cut by"))?;
        let mut out = Vec::new();
        for part in self.objects[at].shape.parts.iter() {
            if part.kind != PartKind::Normal || is_connector(part) {
                continue;
            }
            let into = part.transform.then(&scene).then(&to_plane);
            let lower = cut::cut(&part.mesh, &into, [0.0, 0.0], plane).lower;
            for (triangle, from) in lower.mesh.triangles.iter().zip(&lower.from) {
                if from.is_none() {
                    out.push(triangle.map(|v| lower.mesh.vertices[v as usize].map(f64::from)));
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

    /// Cuts each part of object `at`, whose copy stands at `scene`: the
    /// printed parts' halves (with what was painted on them), the other
    /// parts whole with the sides they reach into.
    fn cut_parts(
        &self,
        at: usize,
        scene: &Transform,
        plane: &Transform,
        options: &CutOptions,
        prepared: Option<&Prepared>,
        cancel: Option<&edge_mesh::boolean::Cancel>,
    ) -> Result<Made, Failure> {
        let drawn = options.drawn.as_ref().filter(|_| options.groove.is_none());
        let sheet = options
            .sheet
            .as_ref()
            .filter(|sheet| !sheet.is_flat() && options.groove.is_none() && drawn.is_none());
        let mut empty = [true, true];
        // The drawn line, finished once for every part.
        let drawn = match drawn {
            Some(cut) => {
                let stroke = cut.finished().map_err(refused)?;
                let mut params = cut.params.clone();
                params.thickness = options.thickness;
                params.thickness_offset = options.thickness_from;
                Some((stroke, params))
            }
            None => None,
        };
        let to_parts = options.to_parts && options.sides.is_none() && options.groove.is_none();
        let keep_upper = options.keep_upper || to_parts || options.sides.is_some();
        let keep_lower = options.keep_lower || to_parts || options.sides.is_some();
        let base = self.name_of(at);
        // The halves are made in the copy's coordinates with its move left
        // out: its turn and size go into their meshes, as the reference
        // cuts each volume in `instance_matrix * volume_matrix`.
        let offset = scene.rows[3];
        let turned = linear(scene);
        let back = plane.then(&Transform::translation(offset.map(|value| -value)));
        let to_plane = back
            .inverse()
            .ok_or_else(|| refused("that is no plane to cut by"))?;
        let faces = options.faces();
        let held = &self.objects[at];
        let mut upper: Vec<Part> = Vec::new();
        let mut lower: Vec<Part> = Vec::new();
        let mut helpers: Vec<(Part, bool, bool)> = Vec::new();
        let mut open = 0;
        for (number, part) in held.shape.parts.iter().enumerate() {
            let placed = part.transform.then(&turned);
            // A drawn surface: the part cut with the solid the line sweeps,
            // in the plane's own coordinates (DrawCut.cpp `draw_cut_split`).
            if part.kind == PartKind::Normal
                && !part.mesh.triangles.is_empty()
                && let Some((stroke, params)) = &drawn
            {
                let mut flat = edge_mesh::Mesh::default();
                flat.append(&part.mesh, &placed.then(&to_plane));
                let solid = edge_mesh::boolean::Solid::new(&flat).map_err(|error| {
                    refused(format!("The cut could not be made. The part {error}."))
                })?;
                let halves =
                    match crate::drawn::draw_cut_split(&solid, &flat, stroke, params, cancel) {
                        Ok(halves) => halves,
                        // A part the line leaves whole goes whole to its side.
                        Err(crate::drawn::DrawSplitError::Draw(
                            crate::drawn::DrawCutError::EmptySide,
                        )) => {
                            let [upper_empty, _] =
                                crate::drawn::draw_cut_empty_sides(&flat, stroke, params);
                            if upper_empty {
                                (edge_mesh::Mesh::default(), flat.clone())
                            } else {
                                (flat.clone(), edge_mesh::Mesh::default())
                            }
                        }
                        Err(crate::drawn::DrawSplitError::Draw(error)) => {
                            return Err(refused(crate::drawn::draw_cut_error_words(error)));
                        }
                        Err(error) => {
                            return Err(refused(format!("The cut could not be made. {error}")));
                        }
                    };
                for (plane_mesh, side) in [(halves.0, true), (halves.1, false)] {
                    if plane_mesh.triangles.is_empty() {
                        continue;
                    }
                    empty[usize::from(!side)] = false;
                    if (side && !keep_upper) || (!side && !keep_lower) {
                        continue;
                    }
                    let mut mesh = edge_mesh::Mesh::default();
                    mesh.append(&plane_mesh, &back);
                    let made = Part {
                        mesh: Arc::new(crate::curved::tidy(mesh)),
                        paint: None,
                        seam: None,
                        transform: Transform::IDENTITY,
                        kept: part.kept.as_deref().map(|kept| Arc::new(kept.piece())),
                        ..part.clone()
                    };
                    if side {
                        upper.push(made);
                    } else {
                        lower.push(made);
                    }
                }
                continue;
            }
            // A curved surface: the part cut with the solid under it, in the
            // halves' coordinates (CurvedCut.cpp `curved_cut_split`).
            if part.kind == PartKind::Normal
                && !part.mesh.triangles.is_empty()
                && let Some(sheet) = sheet
            {
                let mut baked = edge_mesh::Mesh::default();
                baked.append(&part.mesh, &placed);
                let made_here;
                let solid = match prepared
                    .and_then(|prepared| prepared.solids.get(number))
                    .and_then(Option::as_ref)
                {
                    Some(solid) => solid,
                    None => {
                        made_here = edge_mesh::boolean::Solid::new(&baked).map_err(|error| {
                            refused(format!("The cut could not be made. The part {error}."))
                        })?;
                        &made_here
                    }
                };
                let (low, high) = crate::curved::slabs(sheet, &back, &baked.vertices, faces);
                let (up, down) = crate::curved::cut_with_solid(solid, &low, high.as_ref(), cancel)
                    .map_err(|error| refused(format!("The cut could not be made. {error}")))?;
                for (mesh, side) in [(up, true), (down, false)] {
                    if mesh.triangles.is_empty() {
                        continue;
                    }
                    empty[usize::from(!side)] = false;
                    if (side && !keep_upper) || (!side && !keep_lower) {
                        continue;
                    }
                    // Cut through by a boolean: what was painted is not
                    // carried.
                    let made = Part {
                        mesh: Arc::new(mesh),
                        paint: None,
                        seam: None,
                        transform: Transform::IDENTITY,
                        kept: part.kept.as_deref().map(|kept| Arc::new(kept.piece())),
                        ..part.clone()
                    };
                    if side {
                        upper.push(made);
                    } else {
                        lower.push(made);
                    }
                }
                continue;
            }
            if part.kind == PartKind::Normal
                && !part.mesh.triangles.is_empty()
                && let Some(groove) = &options.groove
            {
                let mut baked = edge_mesh::Mesh::default();
                baked.append(&part.mesh, &placed);
                let (up, down) = groove_cut(&baked, &back, groove);
                for (meshes, side) in [(up, true), (down, false)] {
                    if meshes.is_empty() || (side && !keep_upper) || (!side && !keep_lower) {
                        continue;
                    }
                    let mut mesh = edge_mesh::Mesh::default();
                    for piece in &meshes {
                        mesh.append(piece, &Transform::IDENTITY);
                    }
                    // The pieces are cut many times over: what was
                    // painted is not carried (see the module's words).
                    let made = Part {
                        mesh: Arc::new(mesh),
                        paint: None,
                        seam: None,
                        transform: Transform::IDENTITY,
                        kept: part.kept.as_deref().map(|kept| Arc::new(kept.piece())),
                        ..part.clone()
                    };
                    if side {
                        upper.push(made);
                    } else {
                        lower.push(made);
                    }
                }
                continue;
            }
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
                // A dovetail's helpers go to each part they meet
                // (`distribute_modifiers_from_object`).
                let crossed = (low <= 0.0 && high >= 0.0) || options.groove.is_some();
                let moved = Part {
                    transform: placed,
                    ..part.clone()
                };
                helpers.push((moved, low >= 0.0 || crossed, high <= 0.0 || crossed));
            }
        }

        if sheet.is_none() && drawn.is_none() {
            empty = [upper.is_empty(), lower.is_empty()];
        }
        Ok(Made {
            upper,
            lower,
            empty,
            helpers,
            open,
            turned,
        })
    }

    /// The pieces a cut by `plane` makes of a copy's object, for the tool
    /// to show and for a piece to be given to the other side
    /// ([`CutOptions::sides`]).
    pub fn cut_pieces(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
        options: &CutOptions,
    ) -> Result<CutPieces, Failure> {
        let (at, _) = self.copy_at(copy)?;
        if !sound_plane(plane) {
            return Err(refused("that is no plane to cut by"));
        }
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.cut_pieces(bed, copy, plane, options);
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let options = CutOptions {
            to_parts: false,
            sides: Some(Vec::new()),
            ..options.clone()
        };
        let made = self.cut_parts(at, &scene, plane, &options, None, None)?;
        let mut out = CutPieces {
            offset: scene.rows[3],
            ..CutPieces::default()
        };
        for (part, side) in made
            .upper
            .iter()
            .map(|part| (part, true))
            .chain(made.lower.iter().map(|part| (part, false)))
        {
            let pieces = if options.groove.is_some() {
                vec![part.clone()]
            } else {
                split_part(part)
            };
            for piece in pieces {
                let number = out.upper.len() as u32;
                out.upper.push(side);
                let first = out.mesh.vertices.len() as u32;
                out.mesh
                    .vertices
                    .extend(piece.mesh.vertices.iter().copied());
                out.mesh
                    .triangles
                    .extend(piece.mesh.triangles.iter().map(|t| t.map(|v| v + first)));
                out.piece_of
                    .extend(std::iter::repeat_n(number, piece.mesh.triangles.len()));
            }
        }
        Ok(out)
    }

    /// Whether every side of each dovetail meets the copy's printed parts
    /// (`has_valid_groove`: each of `m_groove_vertices`' lines has to
    /// cross the object), as well as its shape being sound.
    pub fn groove_meets(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
        groove: &Groove,
    ) -> Result<bool, Failure> {
        if !groove.sound() {
            return Ok(false);
        }
        let g = groove;
        let neck = 0.5 * g.width;
        let flap = if g.flaps_angle.abs() < 1e-12 {
            g.depth
        } else {
            g.depth / g.flaps_angle.sin()
        };
        let mouth = 0.5 * (g.width + 2.0 * flap * g.flaps_angle.cos());
        let half_height = 0.5 * (1.5 * (1.5 * g.plane_radius));
        let taper = half_height * g.angle.tan();
        let (mouth_outer, neck_outer) = (neck + taper, mouth + taper);
        let mouth_inner = mouth - taper;
        let (front, back) = (0.5 * g.depth, -0.5 * g.depth);
        let width = g.outer_width();
        let count = g.count.max(1);
        for i in 0..count {
            let x = (i as f64 - 0.5 * (count as f64 - 1.0)) * (g.gap + width);
            let lines = [
                (
                    [-neck_outer + x, -half_height, front],
                    [-mouth_inner + x, half_height, front],
                ),
                (
                    [-mouth_outer + x, -half_height, back],
                    [-neck_outer + x, half_height, back],
                ),
                (
                    [neck_outer + x, -half_height, front],
                    [mouth_inner + x, half_height, front],
                ),
                (
                    [mouth_outer + x, -half_height, back],
                    [neck_outer + x, half_height, back],
                ),
            ];
            for (from, to) in lines {
                let (a, b) = (plane.apply(from), plane.apply(to));
                let direction: [f64; 3] = std::array::from_fn(|axis| b[axis] - a[axis]);
                let hit = self.face_hit(bed, copy, a, direction)?;
                let crosses = hit.is_some_and(|hit| {
                    let along: f64 = (0..3)
                        .map(|axis| (hit.point[axis] - a[axis]) * direction[axis])
                        .sum();
                    let length: f64 = direction.iter().map(|d| d * d).sum();
                    along <= length
                });
                if !crosses {
                    return Ok(false);
                }
            }
        }
        Ok(true)
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
        // With connectors both parts are kept, each an object of its own
        // (GLGizmoCut.cpp `perform_cut`: KeepUpper, KeepLower, not
        // KeepAsParts while there are connectors); a dovetail takes none.
        if options.sheet.as_ref().is_some_and(|sheet| !sheet.sound()) {
            return Err(refused("that is no surface to cut by"));
        }
        if !options.connectors.is_empty()
            && (options.drawn.is_some()
                || options.sheet.as_ref().is_some_and(|sheet| !sheet.is_flat()))
        {
            return Err(refused(
                "connectors are not placed on a curved cut here yet",
            ));
        }
        let connected = !options.connectors.is_empty() && options.groove.is_none();
        // A pin that cannot be made stops the cut (`can_perform_cut`,
        // `pushfit_validate`).
        if connected {
            for c in &options.connectors {
                if c.kind == ConnectorKind::Pin
                    && let Some(why) = c.pin.invalid(options.nozzle)
                {
                    return Err(refused(why));
                }
            }
        }
        let forced;
        let options = if connected {
            forced = CutOptions {
                keep_upper: true,
                keep_lower: true,
                to_parts: false,
                ..options.clone()
            };
            &forced
        } else {
            options
        };
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
        let held = self.objects[at].clone();
        let base = self.name_of(at);
        let made = self.cut_parts(at, &scene, plane, options, None, None)?;
        let drawn = options.drawn.is_some() && options.groove.is_none();
        let curved = !drawn
            && options.sheet.as_ref().is_some_and(|sheet| !sheet.is_flat())
            && options.groove.is_none();
        let Made {
            mut upper,
            mut lower,
            empty,
            helpers,
            open,
            turned,
        } = made;
        // The plane in the halves' coordinates: the scene's with the copy's
        // move taken off.
        let offset = scene.rows[3];
        let back = plane.then(&Transform::translation(offset.map(|value| -value)));
        // "Keep cut editable": the object as it was, in the coordinates of
        // its halves, and the plane in them (GLGizmoCut.cpp
        // `build_recipe_from_gizmo`, `object_part_mesh`).
        let recipe = options.editable.then(|| {
            let mut mesh = edge_mesh::Mesh::default();
            // Not the connectors an earlier cut made: they are made again from
            // what the recipe keeps of them.
            for part in held.shape.parts.iter() {
                if part.kind == PartKind::Normal && !is_connector(part) {
                    mesh.append(&part.mesh, &part.transform.then(&turned));
                }
            }
            let kind = if options.groove.is_some() {
                RecipeKind::Groove
            } else if drawn {
                RecipeKind::Drawn
            } else if curved {
                RecipeKind::Curved
            } else {
                RecipeKind::Plane
            };
            let mut recipe = CutRecipe::new(kind, back, Arc::new(mesh));
            if drawn && let Some(cut) = &options.drawn {
                let p = &cut.params;
                recipe.drawn = Some(edge_3mf::RecipeDrawn {
                    samples: cut
                        .stroke
                        .samples
                        .iter()
                        .map(|s| (s.pos, s.normal, s.facet as u64))
                        .collect(),
                    closed: cut.stroke.closed,
                    smoothing: cut.stroke.smoothing,
                    strokes: cut.stroke.stroke_bounds.clone(),
                    finished_open: cut.stroke.finished_open,
                    direction: p.direction.index(),
                    view: p.view_dir,
                    extension: p.extension,
                    angle: p.angle_deg,
                    extension_angle: p.extension_angle_deg,
                    through_all: p.through_all,
                    depth: p.depth,
                });
            }
            if curved && let Some(sheet) = &options.sheet {
                recipe.sheet = Some(edge_3mf::RecipeSheet {
                    nx: sheet.nx as u32,
                    ny: sheet.ny as u32,
                    half_u: sheet.half_u,
                    half_v: sheet.half_v,
                    values: sheet.z.clone(),
                });
            }
            if connected {
                let frame = linear(&back);
                recipe.connectors = options
                    .connectors
                    .iter()
                    .map(|c| RecipeConnector {
                        pos: std::array::from_fn(|axis| c.at[axis] - offset[axis]),
                        rotation: frame,
                        radius: c.radius,
                        height: c.height,
                        radius_tolerance: c.radius_tolerance,
                        height_tolerance: c.height_tolerance,
                        z_angle: c.z_angle,
                        kind: c.kind.code(),
                        style: c.style.code(),
                        shape: c.shape.code(),
                        extra: if c.kind == ConnectorKind::Pin {
                            c.pin.attributes()
                        } else {
                            Vec::new()
                        },
                    })
                    .collect();
            }
            recipe.thickness = if options.groove.is_some() {
                0.0
            } else {
                options.thickness
            };
            recipe.thickness_offset = match options.thickness_from {
                CutThickness::Centred => 0,
                CutThickness::Above => 1,
                CutThickness::Below => 2,
            };
            recipe.keep_upper = options.keep_upper;
            recipe.keep_lower = options.keep_lower;
            recipe.keep_as_parts = options.to_parts;
            recipe.place_on_cut_upper = options.place_on_cut_upper;
            recipe.place_on_cut_lower = options.place_on_cut_lower;
            recipe.flip_upper = options.flip_upper;
            recipe.flip_lower = options.flip_lower;
            recipe.groove = options.groove.map(|g| {
                let init =
                    options
                        .groove_init
                        .unwrap_or([g.depth, g.width, g.flaps_angle, g.angle]);
                RecipeGroove {
                    depth: g.depth,
                    width: g.width,
                    flaps_angle: g.flaps_angle,
                    angle: g.angle,
                    depth_init: init[0],
                    width_init: init[1],
                    flaps_angle_init: init[2],
                    angle_init: init[3],
                    depth_tolerance: g.depth_tolerance,
                    width_tolerance: g.width_tolerance,
                    count: g.count as u32,
                    gap: g.gap,
                    plane_radius: g.plane_radius,
                }
            });
            Arc::new(recipe)
        });
        // A piece given to the other side: the pieces of each side made
        // into one part, the objects named `_A` and `_B`, as
        // `perform_by_contour` and `merge_solid_parts_inside_object` make
        // them.
        let to_parts = to_parts && options.sides.is_none() && options.groove.is_none();
        let mut renamed: Option<(String, String)> = None;
        if let Some(groove) = &options.groove {
            if !groove.sound() {
                return Err(refused("Cut plane with groove is invalid"));
            }
            renamed = Some((format!("{base}_A"), format!("{base}_B")));
        }
        if let Some(sides) = &options.sides {
            let pieces: Vec<(Part, bool)> = upper
                .iter()
                .map(|part| (part, true))
                .chain(lower.iter().map(|part| (part, false)))
                .flat_map(|(part, side)| {
                    split_part(part).into_iter().map(move |piece| (piece, side))
                })
                .collect();
            if sides.len() != pieces.len() {
                return Err(refused(
                    "the pieces of the cut have changed; place the plane again",
                ));
            }
            if sides.iter().all(|side| *side == sides[0]) {
                return Err(refused(
                    "every piece is on one side: there is nothing to cut",
                ));
            }
            let side_of = |want: bool| -> Vec<Part> {
                let chosen: Vec<&Part> = pieces
                    .iter()
                    .zip(sides)
                    .filter(|(_, side)| **side == want)
                    .map(|((piece, _), _)| piece)
                    .collect();
                if chosen.is_empty() {
                    Vec::new()
                } else {
                    vec![merge_parts(&chosen, Some(base.clone()))]
                }
            };
            upper = if keep_upper {
                side_of(true)
            } else {
                Vec::new()
            };
            lower = if keep_lower {
                side_of(false)
            } else {
                Vec::new()
            };
            renamed = Some((format!("{base}_A"), format!("{base}_B")));
        }
        // The objects made, each as its parts with the copies it will have.
        // The connectors (GLGizmoCut.cpp `apply_connectors_in_model`,
        // CutUtils.cpp `process_connector_cut`): a plug or a snap a part of
        // the lower object with its hole in the upper, a dowel an object of
        // its own with a hole in each.
        let template = held
            .shape
            .parts
            .iter()
            .find(|part| part.kind == PartKind::Normal && !is_connector(part))
            .cloned();
        let mut dowels: Vec<Part> = Vec::new();
        let mut pins: Vec<(Part, bool, bool)> = Vec::new();
        if connected
            && !upper.is_empty()
            && !lower.is_empty()
            && let Some(template) = &template
        {
            let made = connector_parts(options, &back, offset, template);
            upper.extend(made.upper);
            lower.extend(made.lower);
            dowels = made.dowels;
            pins = made.pins;
        }
        let mut made_objects: Vec<(Vec<Part>, Option<bool>)> = Vec::new();
        if to_parts {
            let mut parts = upper;
            parts.extend(lower);
            if parts.is_empty() {
                return Err(refused("Cut plane is placed out of object"));
            }
            parts.extend(helpers.into_iter().map(|(part, _, _)| part));
            made_objects.push((parts, Some(true)));
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
                made_objects.push((parts, Some(side)));
            }
            if made_objects.is_empty() {
                return Err(refused("Select at least one object to keep after cutting."));
            }
        }
        let dowel_count = dowels.len();
        made_objects.extend(dowels.into_iter().map(|dowel| (vec![dowel], None)));
        // The results of a cut with connectors or a dovetail, both parts
        // kept, are one cut's (`update_object_cut_id`); a plain cut's are
        // not (`InvalidateCutInfo`).
        let halves = made_objects
            .iter()
            .filter(|(_, side)| side.is_some())
            .count();
        let cut_id = ((connected || options.groove.is_some()) && halves == 2).then(|| CutId {
            id: fresh(),
            check_sum: 2 + (dowel_count + pins.len()) as u64,
            connectors: if connected {
                options.connectors.len() as u64
            } else {
                0
            },
        });

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
            let dowel_name = side.is_none().then(|| {
                format!(
                    "{base}-Dowel-{}",
                    parts[0].name.as_deref().unwrap_or_default()
                )
            });
            let shape = Shape::new(parts);
            // A dowel stands upright where it was (`add_separate_parts`).
            let about = if let (false, Some(side)) = (to_parts, side) {
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
            } else {
                Transform::IDENTITY
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
            let name = match (&renamed, side, dowel_name) {
                (_, None, Some(name)) => name,
                (Some((a, _)), Some(true), _) => a.clone(),
                (Some((_, b)), Some(false), _) => b.clone(),
                _ => base.clone(),
            };
            // Dowels carry no recipe: the cut is edited from its halves.
            let recipe = recipe.as_ref().filter(|_| side.is_some());
            objects.push(ModelObject {
                id: fresh(),
                name,
                filament: held.filament,
                shape,
                copies,
                kept: match (recipe, held.kept.as_deref(), cut_id) {
                    (None, kept, None) => kept.map(|kept| Arc::new(kept.cut_half())),
                    (recipe, kept, cut_id) => Some(Arc::new(
                        kept.map(ObjectKept::cut_half)
                            .unwrap_or_default()
                            .with_recipe(recipe.cloned())
                            .with_cut_id(cut_id),
                    )),
                },
            });
        }
        // The pins: each an object in its print pose, in a row along +x five
        // millimetres past its copy, eight to a row (`add_separate_parts`).
        let (pins_made, pins_lying, slot_failures) = (
            pins.len(),
            pins.iter().filter(|(_, lying, _)| *lying).count(),
            pins.iter().filter(|(_, _, failed)| *failed).count(),
        );
        for (number, (pin, _, _)) in pins.into_iter().enumerate() {
            let (low, high) = pin.mesh.bounds().unwrap_or(([0.0; 3], [5.0; 3]));
            let size = [f64::from(high[0] - low[0]), f64::from(high[1] - low[1])];
            let (column, row) = ((number % 8) as f64, (number / 8) as f64);
            let shape = Shape::new(vec![pin]);
            let copies: Vec<ModelCopy> = held
                .copies
                .iter()
                .zip(&mut instead)
                .map(|(old, (_, made))| {
                    let id = fresh();
                    made.push(id);
                    let [from, _] = old.reach.unwrap_or([[0.0; 3]; 2]);
                    let to = old.reach.map_or([0.0; 3], |[_, to]| to);
                    let x = to[0] + 5.0 + column * (size[0] + 3.0) + 0.5 * size[0];
                    let y = from[1] + row * (size[1] + 3.0) + 0.5 * size[1];
                    let local = Transform::translation([x, y, 0.0]);
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
                name: format!("{base}-Pin-{}", number + 1),
                filament: held.filament,
                shape,
                copies,
                kept: Some(Arc::new(
                    held.kept
                        .as_deref()
                        .map(ObjectKept::cut_half)
                        .unwrap_or_default()
                        .with_cut_id(cut_id),
                )),
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
        Ok(CutDone {
            made: new,
            empty: (curved || drawn).then_some(empty),
            open,
            pins: pins_made,
            pins_lying,
            pin_slot_failures: slot_failures,
        })
    }

    /// A copy's printed parts made ready for a curved cut's booleans, in
    /// the coordinates the halves are made in: kept between the previews
    /// of one copy so that each does not prepare them again.
    pub fn cut_prepared(&self, bed: Option<&Bed>, copy: u64) -> Result<Prepared, Failure> {
        let (at, _) = self.copy_at(copy)?;
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.cut_prepared(bed, copy);
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let turned = linear(&scene);
        let solids = self.objects[at]
            .shape
            .parts
            .iter()
            .map(|part| {
                if part.kind != PartKind::Normal || part.mesh.triangles.is_empty() {
                    return Ok(None);
                }
                let mut baked = edge_mesh::Mesh::default();
                baked.append(&part.mesh, &part.transform.then(&turned));
                edge_mesh::boolean::Solid::new(&baked)
                    .map(Some)
                    .map_err(|error| {
                        refused(format!("The cut could not be made. The part {error}."))
                    })
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        Ok(Prepared { solids, scene })
    }

    /// How far a curved cut's control point at `(x, y, z)` in the plane's
    /// own coordinates must go along the plane's normal to meet the copy's
    /// printed parts ("Right-click-drag a handle to snap it onto the model
    /// surface"); None where the line misses them.
    pub fn cut_snap(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
        point: [f64; 3],
    ) -> Result<Option<f64>, Failure> {
        let (at, _) = self.copy_at(copy)?;
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.cut_snap(bed, copy, plane, point);
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let to_plane = plane
            .inverse()
            .ok_or_else(|| refused("that is no plane to cut by"))?;
        let mut mesh = edge_mesh::Mesh::default();
        for part in self.objects[at].shape.parts.iter() {
            if part.kind == PartKind::Normal && !is_connector(part) {
                mesh.append(&part.mesh, &part.transform.then(&scene).then(&to_plane));
            }
        }
        Ok(crate::curved::snap_distance(&mesh, point))
    }

    /// What a curved cut would make of a copy's printed parts, for the
    /// tool to show while the surface is bent (the reference's cut face
    /// preview): the upper half and the lower, each one mesh, in the
    /// scene. `prepared` is [`Model::cut_prepared`]'s for the copy as it
    /// stands; `cancel` stops it.
    pub fn cut_preview(
        &self,
        bed: Option<&Bed>,
        copy: u64,
        plane: &Transform,
        options: &CutOptions,
        prepared: &Prepared,
        cancel: Option<&edge_mesh::boolean::Cancel>,
    ) -> Result<[edge_mesh::Mesh; 2], Failure> {
        let (at, _) = self.copy_at(copy)?;
        if !sound_plane(plane) {
            return Err(refused("that is no plane to cut by"));
        }
        if self.bare {
            let mut settled = self.clone();
            settled.settle(bed);
            return settled.cut_preview(bed, copy, plane, options, prepared, cancel);
        }
        let options = CutOptions {
            keep_upper: true,
            keep_lower: true,
            to_parts: false,
            sides: None,
            ..options.clone()
        };
        let scene = prepared.scene;
        // A drawn cut works in the plane's own frame: the parts kept ready
        // in the halves' are not its.
        let ready = if options.drawn.is_some() {
            None
        } else {
            Some(prepared)
        };
        let made = self.cut_parts(at, &scene, plane, &options, ready, cancel)?;
        let to_scene = Transform::translation(scene.rows[3]);
        let one = |parts: &[Part]| {
            let mut mesh = edge_mesh::Mesh::default();
            for part in parts.iter().filter(|part| part.kind == PartKind::Normal) {
                mesh.append(&part.mesh, &part.transform.then(&to_scene));
            }
            mesh
        };
        Ok([one(&made.upper), one(&made.lower)])
    }

    /// How the object of a copy was cut, when the cut was kept editable.
    pub fn cut_recipe(&self, copy: u64) -> Option<Arc<CutRecipe>> {
        let (at, _) = self.find(copy)?;
        self.objects[at].kept.as_deref()?.recipe().cloned()
    }

    /// The cut of `source`'s object set up on `target` ("Copy cut to..."):
    /// its plane taken into the target's coordinates as it is, moved where
    /// the target stands (GUI_ObjectList.cpp `copy_cut_to`, "object-local
    /// as-is"), with how it was made.
    pub fn cut_copied(
        &self,
        bed: Option<&Bed>,
        source: u64,
        target: u64,
    ) -> Result<(Arc<CutRecipe>, Transform), Failure> {
        let recipe = self.cut_recipe(source).ok_or_else(|| {
            refused("This cut cannot be copied: the surface it describes is incomplete.")
        })?;
        cuttable(
            &recipe,
            "This cut cannot be copied: the surface it describes is incomplete.",
        )?;
        let (scene, _) = self
            .in_scene(bed, target)
            .ok_or_else(|| refused(format!("there is no copy {target}")))?;
        let plane = recipe.plane.then(&Transform::translation(scene.rows[3]));
        Ok((recipe, plane))
    }

    /// "Edit cut...": the halves of the cut that made a copy's object put
    /// back as the object was before it, one object standing in for them
    /// where the copy is, for the cut to be made again
    /// (GUI_ObjectList.cpp `edit_cut`, GLGizmoCut.cpp `begin_reedit`).
    ///
    /// The halves are those that carry the same cut and are on the
    /// copy's plate. The reference finds them by the cut's id, which a cut
    /// without connectors or a dovetail takes away from them; so for a
    /// plain cut it put back the half chosen alone, left the other on the
    /// bed and said that one was missing.
    pub fn cut_edit(&mut self, bed: Option<&Bed>, copy: u64) -> Result<CutEdit, Failure> {
        const CANNOT: &str = "This cut cannot be edited: the project does not contain the original, uncut shape. It was cut with \"Keep cut editable\" turned off, or the project was written by a different version.";
        let (at, which) = self.copy_at(copy)?;
        let recipe = self.objects[at]
            .kept
            .as_deref()
            .and_then(ObjectKept::recipe)
            .cloned()
            .ok_or_else(|| refused(CANNOT))?;
        cuttable(&recipe, CANNOT)?;
        self.settle(bed);
        let plate = self.plate_of(copy);
        let on_plate = |object: &ModelObject| {
            object
                .copies
                .iter()
                .any(|held| held.id == copy || self.plate_of(held.id) == plate)
        };
        let halves: Vec<usize> = self
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| {
                object
                    .kept
                    .as_deref()
                    .and_then(ObjectKept::recipe)
                    .is_some_and(|held| Arc::ptr_eq(held, &recipe))
                    && on_plate(object)
            })
            .map(|(index, _)| index)
            .collect();
        let made = if recipe.keep_as_parts {
            1
        } else {
            usize::from(recipe.keep_upper) + usize::from(recipe.keep_lower)
        };
        let missing = halves.len() < made;
        let first = &self.objects[at];
        // Its name: the halves' own, less the `_A` and `_B` a dovetail or
        // a piece given to the other side puts on them.
        let names: Vec<&str> = halves
            .iter()
            .map(|&index| self.objects[index].name.as_str())
            .collect();
        let base = names
            .first()
            .and_then(|name| name.strip_suffix("_A").or_else(|| name.strip_suffix("_B")))
            .filter(|base| {
                names.iter().all(|name| {
                    name.strip_suffix("_A").or_else(|| name.strip_suffix("_B")) == Some(*base)
                })
            })
            .map(str::to_owned)
            .unwrap_or_else(|| first.name.clone());
        // The object as it was: its printed parts as one, and the halves'
        // other parts (each once), which are in the same coordinates.
        let printed = first
            .shape
            .parts
            .iter()
            .find(|part| part.kind == PartKind::Normal && !is_connector(part))
            .cloned();
        let mut parts = vec![Part {
            name: None,
            mesh: recipe.mesh.clone(),
            paint: None,
            seam: None,
            transform: Transform::IDENTITY,
            kept: printed
                .as_ref()
                .and_then(|part| part.kept.as_deref())
                .map(|kept| Arc::new(kept.piece())),
            ..printed.ok_or_else(|| refused(CANNOT))?
        }];
        for &index in &halves {
            for part in self.objects[index].shape.parts.iter() {
                let seen = parts.iter().any(|held| {
                    Arc::ptr_eq(&held.mesh, &part.mesh) && held.transform == part.transform
                });
                if part.kind != PartKind::Normal && !seen && !is_connector(part) {
                    parts.push(part.clone());
                }
            }
        }
        let shape = Shape::new(parts);
        let held = &first.copies[which];
        // Where the half stands, its turn left out, standing on the bed.
        let mut local = Transform::translation(held.local.rows[3]);
        if let Some([low, _]) = shape.reach(&local) {
            local.rows[3][2] -= low[2];
        }
        let id = fresh();
        let object = ModelObject {
            id: fresh(),
            name: base,
            filament: first.filament,
            copies: vec![ModelCopy {
                id,
                reach: shape.reach(&local),
                local,
                world: local,
                printable: held.printable,
            }],
            shape,
            kept: Some(Arc::new(
                first
                    .kept
                    .as_deref()
                    .map(ObjectKept::cut_half)
                    .unwrap_or_default()
                    .with_recipe(Some(recipe.clone())),
            )),
        };
        // In the place of the first half in the list and on its plate.
        // The dowels the cut made go with its halves (`edit_cut` takes every
        // object of the cut's id).
        let ids: Vec<u64> = halves
            .iter()
            .filter_map(|&index| self.objects[index].kept.as_deref()?.cut_id())
            .map(|cut| cut.id)
            .collect();
        let mut halves = halves;
        for (index, object) in self.objects.iter().enumerate() {
            let ours = object
                .kept
                .as_deref()
                .and_then(ObjectKept::cut_id)
                .is_some_and(|cut| ids.contains(&cut.id));
            if ours && on_plate(object) && !halves.contains(&index) {
                halves.push(index);
            }
        }
        halves.sort_unstable();
        let gone: Vec<u64> = halves
            .iter()
            .flat_map(|&index| self.objects[index].copies.iter().map(|copy| copy.id))
            .collect();
        let place = halves.iter().copied().min().unwrap_or(at);
        for &index in halves.iter().rev() {
            self.objects.remove(index);
        }
        self.objects.insert(place, object);
        let mut put = false;
        for held in &mut self.plates {
            let mut order = Vec::with_capacity(held.order.len());
            for one in &held.order {
                if *one == copy && !put {
                    order.push(id);
                    put = true;
                } else if !gone.contains(one) {
                    order.push(*one);
                }
            }
            held.order = order;
        }
        self.stand((place, 0), plate, local);
        if let Some((scene, _)) = self.in_scene(bed, id) {
            self.stand_in_scene(bed, id, scene);
        }
        let (scene, _) = self.in_scene(bed, id).ok_or_else(|| refused(CANNOT))?;
        let plane = recipe.plane.then(&Transform::translation(scene.rows[3]));
        Ok(CutEdit {
            copy: id,
            plane,
            recipe,
            missing,
        })
    }
}

/// A connector's shape at unit size, as the cut makes it: radius 1, its
/// height centred on `z = 0` (from -0.5 to 0.5; a dowel's frustum from -1
/// to 1). `snap` is a snap's slit and bulge.
pub fn connector_mesh(
    kind: ConnectorKind,
    style: ConnectorStyle,
    shape: ConnectorShape,
    snap: [f64; 2],
) -> edge_mesh::Mesh {
    use edge_mesh::connector;
    let centred = |mut mesh: edge_mesh::Mesh| {
        for vertex in &mut mesh.vertices {
            vertex[2] -= 0.5;
        }
        mesh
    };
    let sides = shape.sides();
    match (kind, style) {
        (ConnectorKind::Snap, _) => centred(connector::snap(snap[0], snap[1])),
        // A pin is made from its own shape ([`crate::pushfit::Pin::mesh`]).
        (ConnectorKind::Pin, _) => centred(connector::prism(sides, 1.0)),
        (_, ConnectorStyle::Prism) => centred(connector::prism(sides, 1.0)),
        (ConnectorKind::Plug, ConnectorStyle::Frustum) => centred(connector::prism(sides, 0.5)),
        (ConnectorKind::Dowel, ConnectorStyle::Frustum) => connector::bicone(sides),
    }
}

/// Whether a part is a connector a cut made.
fn is_connector(part: &Part) -> bool {
    part.kept.as_deref().and_then(PartKept::connector).is_some()
}

/// What the connectors of a cut make.
struct ConnectorParts {
    upper: Vec<Part>,
    lower: Vec<Part>,
    dowels: Vec<Part>,
    /// Each pin, whether it prints lying, whether its slit failed.
    pins: Vec<(Part, bool, bool)>,
}

/// The parts the connectors make, in the halves' coordinates, `back`
/// being the plane in them and `offset` the copy's move in the scene
/// (GLGizmoCut.cpp `apply_connectors_in_model`,
/// `apply_cut_connectors`, `get_connector_mesh`; CutUtils.cpp
/// `process_connector_cut`, `apply_tolerance`). `template` is a printed
/// part of the object, whose filament they print in.
///
/// Differs on purpose: the reference centres each connector's mesh on its
/// box when it adds it (`add_volume`), which puts a triangle's middle a
/// quarter of its radius off the point it was placed on, where the tool
/// draws it; here only its height is centred, so it is cut where it is
/// shown.
fn connector_parts(
    options: &CutOptions,
    back: &Transform,
    offset: [f64; 3],
    template: &Part,
) -> ConnectorParts {
    let mut out = ConnectorParts {
        upper: Vec::new(),
        lower: Vec::new(),
        dowels: Vec::new(),
        pins: Vec::new(),
    };
    let frame = linear(back);
    let normal = back.rows[2];
    let [lo, hi] = options.faces();
    let kerf = hi - lo;
    let part = |name: String,
                kind: PartKind,
                mesh: Arc<edge_mesh::Mesh>,
                transform: Transform,
                info: CutConnectorInfo| Part {
        name: Some(name),
        kind,
        mesh,
        paint: None,
        seam: None,
        transform,
        decoration: false,
        kept: Some(Arc::new(PartKept::of_connector(info))),
        ..template.clone()
    };
    let centred = |mesh: edge_mesh::Mesh| Arc::new(mesh);
    for (number, c) in options.connectors.iter().enumerate() {
        let name = format!("Connector-{}", number + 1);
        let info = CutConnectorInfo {
            kind: c.kind.code(),
            radius_tolerance: c.radius_tolerance,
            height_tolerance: c.height_tolerance,
            // A pin's shape is written with it, and whether it was cut
            // (bbs_3mf.cpp `_add_cut_information_file_to_archive`).
            extra: if c.kind == ConnectorKind::Pin {
                let mut extra = vec![("processed".to_owned(), 1.0)];
                extra.extend(c.pin.attributes());
                extra
            } else {
                Vec::new()
            },
        };
        let mesh = Arc::new(connector_mesh(c.kind, c.style, c.shape, options.snap));
        // On the plane, in the halves' coordinates.
        let at: [f64; 3] = std::array::from_fn(|axis| c.at[axis] - offset[axis]);
        let placed = |radius: f64, height: f64, middle: [f64; 3]| {
            Transform {
                rows: [
                    [radius, 0.0, 0.0],
                    [0.0, radius, 0.0],
                    [0.0, 0.0, height],
                    [0.0; 3],
                ],
            }
            .then(&rotation_xyz(0.0, 0.0, -c.z_angle))
            .then(&frame)
            .then(&Transform::translation(middle))
        };
        let along = |from: [f64; 3], by: f64| -> [f64; 3] {
            std::array::from_fn(|axis| from[axis] + normal[axis] * by)
        };
        match c.kind {
            ConnectorKind::Plug | ConnectorKind::Snap => {
                // Over the plane, from the lower part's face, as long again
                // as the cut is thick.
                let height = c.height + kerf;
                let middle = along(at, lo + 0.5 * height);
                out.lower.push(part(
                    name.clone(),
                    PartKind::Normal,
                    mesh.clone(),
                    placed(c.radius, height, middle),
                    info.clone(),
                ));
                // Its hole: wider and deeper by the tolerances, through the
                // face; a snap's a plain round one.
                let hole = if c.kind == ConnectorKind::Snap {
                    centred(connector_mesh(
                        ConnectorKind::Plug,
                        ConnectorStyle::Prism,
                        ConnectorShape::Circle,
                        options.snap,
                    ))
                } else {
                    mesh
                };
                let deeper = height + c.height_tolerance + CONNECTOR_HOLE_OVERLAP;
                let middle = along(middle, 0.5 * (c.height_tolerance - CONNECTOR_HOLE_OVERLAP));
                out.upper.push(part(
                    name,
                    PartKind::Negative,
                    hole,
                    placed(c.radius + c.radius_tolerance, deeper, middle),
                    info,
                ));
            }
            ConnectorKind::Dowel => {
                // Each way from the plane; a prism's height is how far it
                // reaches each way.
                let height = if c.style == ConnectorStyle::Prism {
                    2.0 * c.height
                } else {
                    c.height
                } + kerf;
                // The dowel: upright, where it stood across the plane
                // (`add_separate_parts`: its turn and height taken off).
                let upright = Transform {
                    rows: [
                        [c.radius, 0.0, 0.0],
                        [0.0, c.radius, 0.0],
                        [0.0, 0.0, height],
                        [at[0], at[1], 0.0],
                    ],
                };
                out.dowels.push(part(
                    name.clone(),
                    PartKind::Normal,
                    mesh.clone(),
                    upright,
                    info.clone(),
                ));
                // Its holes: the dowel with the tolerances, cut just past
                // the plane each way so that each breaks through its face.
                let hole = placed(
                    c.radius + c.radius_tolerance,
                    height + c.height_tolerance,
                    at,
                );
                let mut baked = edge_mesh::Mesh::default();
                baked.append(&mesh, &hole);
                let Some(to_plane) = back.inverse() else {
                    continue;
                };
                let up = edge_mesh::cut::cut(&baked, &to_plane, [-CONNECTOR_HOLE_OVERLAP; 2], back)
                    .upper;
                let down =
                    edge_mesh::cut::cut(&baked, &to_plane, [CONNECTOR_HOLE_OVERLAP; 2], back).lower;
                for (half, suffix, into) in
                    [(up, "_A", &mut out.upper), (down, "_B", &mut out.lower)]
                {
                    if !half.is_empty() {
                        into.push(part(
                            format!("{name}{suffix}"),
                            PartKind::Negative,
                            Arc::new(half.mesh),
                            Transform::IDENTITY,
                            info.clone(),
                        ));
                    }
                }
            }
            ConnectorKind::Pin => {
                // The pin straddles the cut, each end across a cut's
                // thickness, and is an object of its own; a hole each side
                // from its face (CutUtils.cpp `process_pushfit_cut`).
                let (kerf_lo, kerf_hi) = ((-lo).max(0.0), hi.max(0.0));
                let made = c.pin.mesh(kerf_lo, kerf_hi, true);
                let failed = made.slot_requested && !made.slot_made;
                out.pins.push((
                    part(
                        name.clone(),
                        PartKind::Normal,
                        Arc::new(made.mesh),
                        Transform::IDENTITY,
                        info.clone(),
                    ),
                    made.lying,
                    failed,
                ));
                let there = rotation_xyz(0.0, 0.0, -c.z_angle)
                    .then(&frame)
                    .then(&Transform::translation(at));
                for (side, suffix, face) in [
                    (crate::pushfit::PinSide::Upper, "_A", kerf_hi),
                    (crate::pushfit::PinSide::Lower, "_B", kerf_lo),
                ] {
                    let hole = part(
                        format!("{name}{suffix}"),
                        PartKind::Negative,
                        Arc::new(c.pin.hole_mesh(side, face)),
                        there,
                        info.clone(),
                    );
                    if side == crate::pushfit::PinSide::Upper {
                        out.upper.push(hole);
                    } else {
                        out.lower.push(hole);
                    }
                }
            }
        }
    }
    out
}

/// Whether a recipe can be made again here: one of a schema this knows,
/// by a plane or a dovetail. A curved or drawn surface needs a mesh
/// boolean this does not have, and connectors are not placed here yet.
fn cuttable(recipe: &CutRecipe, why: &str) -> Result<(), Failure> {
    if !recipe.known() || recipe.mesh.triangles.is_empty() {
        return Err(refused(why));
    }
    if recipe.kind == RecipeKind::Curved && recipe.sheet.is_none() {
        return Err(refused(why));
    }
    if recipe.kind == RecipeKind::Drawn && recipe.drawn.is_none() {
        return Err(refused(
            "This cut was made with a curved or drawn surface, which cannot be made here yet.",
        ));
    }
    if recipe
        .connectors
        .iter()
        .any(|c| ConnectorKind::of(c.kind).is_none())
    {
        return Err(refused(
            "This cut has connectors of a kind that cannot be placed here yet.",
        ));
    }
    Ok(())
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

/// A turn about z, then y, then x by these angles, as a transform of row
/// vectors (Geometry.cpp `rotation_transform`: `Rz * Ry * Rx` applied to
/// column vectors).
fn rotation_xyz(x: f64, y: f64, z: f64) -> Transform {
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    // Rz * Ry * Rx, as columns; its transpose as rows.
    let m = [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ];
    Transform {
        rows: [
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
            [0.0; 3],
        ],
    }
}

/// Meshes cut by a plane given in the cut's own coordinates (`local`,
/// then `base` to the meshes'): what is above it and what is under it,
/// each a closed mesh for each one cut (empty ones left out).
fn halve(
    meshes: &[edge_mesh::Mesh],
    local: &Transform,
    base: &Transform,
) -> (Vec<edge_mesh::Mesh>, Vec<edge_mesh::Mesh>) {
    let frame = local.then(base);
    let Some(to) = frame.inverse() else {
        return (Vec::new(), meshes.to_vec());
    };
    let (mut above, mut below) = (Vec::new(), Vec::new());
    for mesh in meshes {
        let made = cut::cut(mesh, &to, [0.0, 0.0], &frame);
        if !made.upper.is_empty() {
            above.push(made.upper.mesh);
        }
        if !made.lower.is_empty() {
            below.push(made.lower.mesh);
        }
    }
    (above, below)
}

/// A dovetail cut of a mesh (CutUtils.cpp `Cut::perform_with_groove`, by
/// the same planes in the same order): the slab above the groove and each
/// tongue for the upper part, the slab under it and what is beside each
/// tongue for the lower; what each tolerance takes off the tongue goes.
/// `base` takes the cut's coordinates to the mesh's.
fn groove_cut(
    mesh: &edge_mesh::Mesh,
    base: &Transform,
    g: &Groove,
) -> (Vec<edge_mesh::Mesh>, Vec<edge_mesh::Mesh>) {
    let mut upper = Vec::new();
    let mut lower = Vec::new();
    let half = 0.5 * g.depth;
    let at = |x: f64, z: f64| Transform::translation([x, 0.0, z]);
    // The slab between the planes at -depth/2 and +depth/2.
    let (above, slab) = halve(std::slice::from_ref(mesh), &at(0.0, half), base);
    upper.extend(above);
    let (slab, below) = halve(&slab, &at(0.0, -half), base);
    lower.extend(below);
    let width = g.outer_width();
    let count = g.count.max(1);
    let shift = 0.5 * (g.width + g.depth / g.flaps_angle.tan());
    let across = rotation_xyz(0.0, std::f64::consts::FRAC_PI_2, 0.0);
    for i in 0..count {
        let offset = (i as f64 - 0.5 * (count as f64 - 1.0)) * (g.gap + width);
        let mut piece = slab.clone();
        if i > 0 {
            let left = -0.5 * g.gap - 0.5 * width + offset;
            piece = halve(&piece, &across.then(&at(left, 0.0)), base).0;
        }
        if i + 1 < count {
            let right = 0.5 * g.gap + 0.5 * width + offset;
            piece = halve(&piece, &across.then(&at(right, 0.0)), base).1;
        }
        let flap = |sign: f64, by: f64| {
            rotation_xyz(0.0, sign * g.flaps_angle, sign * g.angle)
                .then(&at(offset + sign * by, 0.0))
        };
        let (beside, inside) = halve(&piece, &flap(-1.0, shift), base);
        lower.extend(beside);
        let (beside, inside) = halve(&inside, &flap(1.0, shift), base);
        lower.extend(beside);
        // The tolerances: off the tongue's foot and its sides.
        let (inside, _) = halve(&inside, &at(offset, -(half - g.depth_tolerance)), base);
        let by = shift - 0.5 * g.width_tolerance;
        let (_, inside) = halve(&inside, &flap(-1.0, by), base);
        let (_, inside) = halve(&inside, &flap(1.0, by), base);
        upper.extend(inside);
    }
    (upper, lower)
}

/// The triangles of a mesh by the shells they are of (two triangles are of
/// one when they share an edge run the other way round, as
/// `edge_mesh::split` pairs them), each shell's in order, the shells in the
/// order of their first triangle.
fn shells(mesh: &edge_mesh::Mesh) -> Vec<Vec<u32>> {
    let next = edge_mesh::split::neighbours(mesh);
    let mut seen = vec![false; mesh.triangles.len()];
    let mut out = Vec::new();
    for start in 0..mesh.triangles.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut shell = Vec::new();
        let mut stack = vec![start as u32];
        while let Some(at) = stack.pop() {
            shell.push(at);
            for other in next[at as usize].into_iter().flatten() {
                if !seen[other as usize] {
                    seen[other as usize] = true;
                    stack.push(other);
                }
            }
        }
        shell.sort_unstable();
        out.push(shell);
    }
    out
}

/// A part's mesh as a part for each of its shells, each with what was
/// painted on its triangles.
fn split_part(part: &Part) -> Vec<Part> {
    let shells = shells(&part.mesh);
    if shells.len() < 2 {
        return vec![part.clone()];
    }
    let codes: Vec<(PaintKind, Vec<(u32, String)>)> = PaintKind::ALL
        .into_iter()
        .map(|kind| {
            let mut codes = part.paint_codes(kind);
            codes.sort_by_key(|(triangle, _)| *triangle);
            (kind, codes)
        })
        .filter(|(_, codes)| !codes.is_empty())
        .collect();
    shells
        .into_iter()
        .map(|shell| {
            let mut index = vec![u32::MAX; part.mesh.vertices.len()];
            let mut mesh = edge_mesh::Mesh::default();
            for &t in &shell {
                let tri = part.mesh.triangles[t as usize].map(|v| {
                    if index[v as usize] == u32::MAX {
                        index[v as usize] = mesh.vertices.len() as u32;
                        mesh.vertices.push(part.mesh.vertices[v as usize]);
                    }
                    index[v as usize]
                });
                mesh.triangles.push(tri);
            }
            let mut piece = Part {
                mesh: Arc::new(mesh),
                paint: None,
                seam: None,
                kept: part.kept.as_deref().map(|kept| Arc::new(kept.piece())),
                ..part.clone()
            };
            for (kind, codes) in &codes {
                let mapped: Vec<(u32, String)> = shell
                    .iter()
                    .enumerate()
                    .filter_map(|(at, t)| {
                        codes
                            .binary_search_by_key(t, |(triangle, _)| *triangle)
                            .ok()
                            .map(|found| (at as u32, codes[found].1.clone()))
                    })
                    .collect();
                piece = piece.with_paint(*kind, &mapped);
            }
            piece
        })
        .collect()
}

/// Parts of one object made one part (CutUtils.cpp
/// `merge_solid_parts_inside_object`: their meshes side by side, the part
/// named after the object), each part's paint going with its triangles.
fn merge_parts(parts: &[&Part], name: Option<String>) -> Part {
    if let [one] = parts {
        return Part {
            name,
            ..(*one).clone()
        };
    }
    let mut mesh = edge_mesh::Mesh::default();
    let mut codes: Vec<Vec<(u32, String)>> = vec![Vec::new(); PaintKind::ALL.len()];
    for part in parts {
        let first = mesh.triangles.len() as u32;
        mesh.append(&part.mesh, &part.transform);
        // A mirrored part's triangles are turned round: its codes no
        // longer fit them.
        if part.transform.is_mirrored() {
            continue;
        }
        for (slot, kind) in PaintKind::ALL.into_iter().enumerate() {
            codes[slot].extend(
                part.paint_codes(kind)
                    .into_iter()
                    .map(|(t, code)| (t + first, code)),
            );
        }
    }
    let first = parts[0];
    let mut merged = Part {
        name,
        mesh: Arc::new(mesh),
        paint: None,
        seam: None,
        transform: Transform::IDENTITY,
        kept: first.kept.as_deref().map(|kept| Arc::new(kept.piece())),
        ..first.clone()
    };
    for (slot, kind) in PaintKind::ALL.into_iter().enumerate() {
        if !codes[slot].is_empty() {
            codes[slot].sort_by_key(|(t, _)| *t);
            merged = merged.with_paint(kind, &codes[slot]);
        }
    }
    merged
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
