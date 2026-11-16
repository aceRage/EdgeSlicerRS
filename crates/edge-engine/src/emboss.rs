//! Text made into a part ("Emboss" [T], Gizmos/GLGizmoEmboss.cpp with
//! Jobs/EmbossJob.cpp and libslic3r/Emboss.cpp): the solid of a text, where
//! it goes on a surface, and the edits that add it and make it again.
//!
//! What the reference does, and so this:
//!
//! - the text's letters are laid out ([`edge_text`], `text2vshapes`) and
//!   united, each grown by 50 shape units and shrunk back, then healed in
//!   at most ten rounds (EmbossJob.cpp `create_shape`: `union_with_delta`
//!   with `UNION_DELTA`, `UNION_MAX_ITERATIN`);
//! - the shape is raised along the text's own Z into a solid as deep as
//!   the style's depth (`polygons2model` with a `ProjectZ`), scaled from
//!   shape units by `size / units per em × 0.001`; a printed part stands
//!   from 0.015 mm under its surface outwards, a negative part or a
//!   modifier from 0.015 mm over it inwards (`try_create_mesh`:
//!   `SAFE_SURFACE_OFFSET`);
//! - a new text is placed where a line of sight meets the object
//!   (`start_create_volume_on_surface_job`): its Z along the surface's
//!   normal, its Y as near the world's Z as the face allows, or the
//!   world's Y on a face that looks up or down more than `UP_LIMIT`
//!   (`create_transformation_onto_surface`, `suggest_up`); then turned by
//!   the style's angle about its Z and moved its distance along it
//!   (`apply_transformation`);
//! - where no line of sight meets it, it goes below the object's box on
//!   the bed side, square to the object (`create_volume` without a
//!   matrix); with no object at all, a new object of the text is made in
//!   the middle of a free place on the plate (`CreateObjectJob`);
//! - the part is named after its text, new lines as spaces
//!   (`text_volume_name`), and made again in its own frame when an option
//!   changes (`UpdateJob`), from the place the file's text was saved at
//!   (`fix_3mf_tr`) when it was read from a project.
//!
//! Text that follows the surface ("Use surface", CutSurface.cpp) is made
//! in `emboss_surface.rs`.
//!
//! Not here yet, each refused in words rather than made otherwise: text
//! placed letter by letter ("Per glyph"), curved text (EmbossBend.cpp), and
//! inline shapes. A text read with one of them keeps its mesh and
//! description as the file had them, until those options are taken off.

use std::sync::Arc;

use edge_3mf::{EmbossShape, HorizontalAlign, Part, TextConfiguration, TextVolume, VerticalAlign};
use edge_geom::Coord;
use edge_mesh::{Mesh, Transform};
use edge_text::Font;

use super::{Bed, Model, PartKind, Shape, fresh, refused};
use crate::Failure;

#[path = "emboss_surface.rs"]
mod surface;
pub use surface::{NO_SURFACE, Surface, SurfaceKey, emboss_on_surface, moved_in_frame};

/// How far a text's solid reaches past its surface, in mm (EmbossJob.cpp
/// `SAFE_SURFACE_OFFSET`), so that it meets the object.
pub const SAFE_SURFACE_OFFSET: f64 = 0.015;

/// How far a face's normal may point up or down before the text's up is
/// the world's Y rather than its Z (SurfaceDrag.hpp `UP_LIMIT`).
pub const UP_LIMIT: f64 = 0.9;

/// What letters are grown by before they are united, in shape units
/// (Emboss.hpp `UNION_DELTA`).
const UNION_DELTA: Coord = 50;

/// The most rounds of healing (Emboss.hpp `UNION_MAX_ITERATIN`).
const UNION_MAX_HEAL: u32 = 10;

/// The fonts a text is made from: its own, and the one a symbol it lacks
/// is taken from (the reference's bundled `NotoSansSymbols2-Subset.ttf`).
#[derive(Clone, Copy)]
pub struct TextFonts<'a> {
    pub font: &'a Font,
    pub fallback: Option<&'a Font>,
}

/// A text made, ready to go into a model: its description, solid and name.
#[derive(Clone, Debug)]
pub struct NewText {
    pub volume: TextVolume,
    pub mesh: Mesh,
    pub name: String,
}

/// Whether a text is nothing but white space (GLGizmoEmboss.cpp
/// `is_text_empty`).
pub fn is_text_empty(text: &str) -> bool {
    text.chars().all(|c| matches!(c, ' ' | '\n' | '\t' | '\r'))
}

/// The name a part of this text goes by (InlineShapes.cpp
/// `text_volume_name`): new lines as spaces.
pub fn text_name(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// The layout options of a description, as `edge_text` takes them.
pub fn font_prop(config: &TextConfiguration) -> edge_text::FontProp {
    let mut prop = edge_text::FontProp::new(config.size_in_mm);
    prop.horizontal = match config.horizontal {
        HorizontalAlign::Left => edge_text::HorizontalAlign::Left,
        HorizontalAlign::Center => edge_text::HorizontalAlign::Center,
        HorizontalAlign::Right => edge_text::HorizontalAlign::Right,
    };
    prop.vertical = match config.vertical {
        VerticalAlign::Top => edge_text::VerticalAlign::Top,
        VerticalAlign::Center => edge_text::VerticalAlign::Center,
        VerticalAlign::Bottom => edge_text::VerticalAlign::Bottom,
    };
    prop.char_gap = config.char_gap;
    prop.line_gap = config.line_gap;
    prop.boldness = config.boldness;
    prop.skew = config.skew;
    prop
}

/// What of a description cannot be made here yet, in words; none when
/// all of it can.
pub fn not_yet(config: &TextConfiguration, shape: &EmbossShape) -> Option<&'static str> {
    if config.per_glyph {
        Some("Text placed letter by letter (\"Per glyph\") cannot be made here yet.")
    } else if shape.bend.is_some() {
        Some("Curved text cannot be made here yet.")
    } else if config.inline_shapes.is_some() {
        Some("Text with inline shapes cannot be made here yet.")
    } else {
        None
    }
}

/// The text's united shape, in shape units, how many mm one is, and
/// whether it healed clean (EmbossJob.cpp `create_shape`).
pub fn text_shape(
    fonts: TextFonts<'_>,
    config: &TextConfiguration,
) -> Result<(Vec<edge_geom::ExPolygon>, f64, bool), Failure> {
    let prop = font_prop(config);
    let mut setter = edge_text::Typesetter::new(fonts.font, fonts.fallback, prop);
    let letters = setter.letters(&config.text).map_err(|e| e.to_string())?;
    let (shape, healed) = edge_text::union_with_delta(&letters, UNION_DELTA, UNION_MAX_HEAL)
        .map_err(|e| e.to_string())?;
    Ok((shape, edge_text::shape_scale(fonts.font, &prop), healed))
}

/// The text of `config` raised `depth` mm, for a part of `kind`, in the
/// text's own frame: what the part's mesh is (EmbossJob.cpp
/// `try_create_mesh`). Refused for a text of white space alone or one
/// whose letters the font has none of.
pub fn emboss(
    fonts: TextFonts<'_>,
    config: &TextConfiguration,
    depth: f64,
    kind: PartKind,
) -> Result<NewText, Failure> {
    if is_text_empty(&config.text) {
        return Err(refused("Embossed text cannot contain only white spaces."));
    }
    if !(depth.is_finite() && depth > 0.0) {
        return Err(refused("the depth of a text has to be more than nothing"));
    }
    if !(config.size_in_mm.is_finite() && config.size_in_mm > 0.0) {
        return Err(refused("the height of a text has to be more than nothing"));
    }
    let (shape, scale, healed) = text_shape(fonts, config)?;
    if shape.is_empty() {
        return Err(refused("Font doesn't have any shape for given text."));
    }
    // `ProjectZ`: the shape at 0, its copy `depth` further on, scaled with
    // the shape and moved by the offset; each corner in single precision.
    let outside = kind == PartKind::Normal;
    let offset = if outside {
        -SAFE_SURFACE_OFFSET
    } else {
        SAFE_SURFACE_OFFSET - depth
    };
    let mm = |p: &edge_geom::Point2| [(p.x as f64 * scale) as f32, (p.y as f64 * scale) as f32];
    let regions: Vec<edge_mesh::primitives::FlatRegion> = shape
        .iter()
        .map(|region| {
            (
                region.contour.points.iter().map(mm).collect(),
                region
                    .holes
                    .iter()
                    .map(|hole| hole.points.iter().map(mm).collect())
                    .collect(),
            )
        })
        .collect();
    let depth_units = depth / scale;
    let z0 = offset as f32;
    let z1 = (depth_units * scale + offset) as f32;
    let mesh = edge_mesh::primitives::prism(&regions, z0, z1)
        .ok_or_else(|| refused("the text's shape could not be closed into a solid"))?;
    let volume = TextVolume {
        text: config.clone(),
        shape: EmbossShape {
            scale,
            unhealed: !healed,
            depth,
            ..EmbossShape::default()
        },
    };
    Ok(NewText {
        name: text_name(&config.text),
        volume,
        mesh,
    })
}

type Vec3 = [f64; 3];

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Eigen's `normalize`: a vector of no length is left as it is.
fn normalized(v: Vec3) -> Vec3 {
    let length = dot(v, v).sqrt();
    if length > 0.0 {
        v.map(|c| c / length)
    } else {
        v
    }
}

/// A turn of `angle` about `axis` (unit), as columns: Eigen's
/// `AngleAxis::toRotationMatrix`.
fn turn(angle: f64, axis: Vec3) -> [Vec3; 3] {
    let (s, c) = angle.sin_cos();
    let [x, y, z] = axis;
    let t = 1.0 - c;
    // Row r, column k of the matrix that takes a column vector.
    let m = [
        [c + t * x * x, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, c + t * y * y, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, c + t * z * z],
    ];
    std::array::from_fn(|k| [m[0][k], m[1][k], m[2][k]])
}

/// A turn given by its columns as a [`Transform`].
fn transform_of(columns: [Vec3; 3]) -> Transform {
    Transform {
        rows: [columns[0], columns[1], columns[2], [0.0; 3]],
    }
}

/// The text's up on a face whose normal is `normal` (Emboss.cpp
/// `suggest_up`): the world's Z laid on the face, or its Y where the face
/// looks up or down more than `up_limit`.
pub fn suggest_up(normal: Vec3, up_limit: f64) -> Vec3 {
    let wanted = if normal[2].abs() > up_limit {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    normalized(cross(cross(normal, wanted), normal))
}

/// Where a text stands on a surface at `position` facing `normal`
/// (Emboss.cpp `create_transformation_onto_surface`): its Z along the
/// normal, its Y the suggested up.
pub fn onto_surface(position: Vec3, normal: Vec3, up_limit: f64) -> Transform {
    let normal = normalized(normal);
    let up_dir = [0.0, 1.0, 0.0];
    let emboss_dir = [0.0, 0.0, 1.0];
    let wanted_up = suggest_up(normal, up_limit);
    let (axis_view, angle_view) = if normal == [0.0, 0.0, -1.0] {
        ([0.0, 1.0, 0.0], std::f64::consts::PI)
    } else {
        // `acos` of a hair over one is not a number in the reference; it is
        // taken as one here.
        (
            normalized(cross(emboss_dir, normal)),
            dot(emboss_dir, normal).clamp(-1.0, 1.0).acos(),
        )
    };
    let view = turn(angle_view, axis_view);
    // The inverse of a turn is its transpose.
    let back = |v: Vec3| [dot(view[0], v), dot(view[1], v), dot(view[2], v)];
    let wanted_up_rotated = normalized(back(wanted_up));
    let mut angle_up = dot(up_dir, wanted_up_rotated).clamp(-1.0, 1.0).acos();
    let text_view = cross(up_dir, wanted_up_rotated);
    let diff: Vec3 = std::array::from_fn(|i| emboss_dir[i] - text_view[i]);
    if diff.iter().any(|d| d.abs() > 1.0) {
        angle_up = -angle_up;
    }
    let up = turn(angle_up, emboss_dir);
    // translate(position) * view_rot * up_rot.
    transform_of(up)
        .then(&transform_of(view))
        .then(&Transform::translation(position))
}

/// The text's turn about its own Z and its move along it (Emboss.cpp
/// `apply_transformation`): `transformation * Rz(angle)`, then the move
/// taken in the turned frame.
pub fn apply_angle_distance(
    transformation: &Transform,
    angle: Option<f64>,
    distance: Option<f64>,
) -> Transform {
    let mut out = *transformation;
    if let Some(angle) = angle {
        out = transform_of(turn(angle, [0.0, 0.0, 1.0])).then(&out);
    }
    if let Some(distance) = distance {
        out = Transform::translation([0.0, 0.0, distance]).then(&out);
    }
    out
}

/// How far a text is turned from the up its face suggests, in radians
/// (Emboss.cpp `calc_up`); none when it is not turned.
pub fn calc_up(transformation: &Transform, up_limit: f64) -> Option<f64> {
    let rows = &transformation.rows;
    let normal = normalized(rows[2]);
    let suggested = suggest_up(normal, up_limit);
    let up = normalized(rows[1]);
    // The determinant of the matrix with rows up, suggested, normal.
    let det = dot(up, cross(suggested, normal));
    let result = -det.atan2(dot(suggested, up));
    // Eigen's `is_approx` of two doubles, against 0.
    // Slic3r's `is_approx` against 0, to its `EPSILON`.
    (result.abs() > 1e-4).then_some(result)
}

/// The frame of a text on a face at `point` facing `normal` (in the
/// scene), turned and moved by the style, taken into the object by
/// `back` (`start_create_volume_on_surface_job`).
fn frame_on(
    back: &Transform,
    point: Vec3,
    normal: Vec3,
    angle: Option<f64>,
    distance: Option<f64>,
) -> Transform {
    let surface = onto_surface(point, normal, UP_LIMIT);
    apply_angle_distance(&surface, angle, distance).then(back)
}

/// The middle of the box round a mesh.
fn centre(mesh: &Mesh) -> Vec3 {
    mesh.bounds().map_or([0.0; 3], |(low, high)| {
        std::array::from_fn(|axis| (f64::from(low[axis]) + f64::from(high[axis])) / 2.0)
    })
}

impl Model {
    /// Adds a text to an object as a part of `kind` (a printed part, a
    /// negative part or a modifier) and answers its number. `hit` is where
    /// a line of sight met copy `copy` and the face's normal there, both
    /// in the scene; without it the text goes below the copy's box. The
    /// style's angle and distance are applied (`apply_transformation`).
    #[allow(clippy::too_many_arguments)]
    pub fn add_text(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        copy: u64,
        hit: Option<(Vec3, Vec3)>,
        angle: Option<f64>,
        distance: Option<f64>,
        kind: PartKind,
        text: NewText,
    ) -> Result<u64, Failure> {
        if !matches!(
            kind,
            PartKind::Normal | PartKind::Negative | PartKind::Modifier
        ) {
            return Err(refused("text is a part, a negative part or a modifier"));
        }
        let at = self.object_at(object)?;
        if self.copy_at(copy)?.0 != at {
            return Err(refused(format!("copy {copy} is not of object {object}")));
        }
        self.settle(bed);
        let (scene, reach) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let back = scene
            .inverse()
            .ok_or_else(|| refused("the object is squashed flat"))?;
        let transform = match hit {
            Some((point, normal)) => frame_on(&back, point, normal, angle, distance),
            None => {
                // `create_volume` without a matrix: under the copy, lying on
                // the bed, in the copy's unturned frame.
                let [low, high] =
                    reach.ok_or_else(|| refused("the object has nothing printed in it"))?;
                let size = text.mesh.bounds().map_or([0.0; 3], |(lo, hi)| {
                    std::array::from_fn(|i| f64::from(hi[i] - lo[i]))
                });
                let offset = [
                    0.0,
                    -(high[1] - low[1]) / 2.0 - size[1] / 2.0,
                    size[2] / 2.0 - (high[2] - low[2]) / 2.0,
                ];
                let mut linear = scene;
                linear.rows[3] = [0.0; 3];
                let unturn = linear
                    .inverse()
                    .ok_or_else(|| refused("the object is squashed flat"))?;
                Transform::translation(offset).then(&unturn)
            }
        };
        let own = (kind == PartKind::Normal)
            .then_some(self.objects[at].filament)
            .flatten();
        let made = Part {
            kept: None,
            name: Some(text.name),
            kind,
            filament: own.unwrap_or(self.objects[at].default_filament()),
            own_filament: own,
            mesh: Arc::new(text.mesh),
            paint: None,
            seam: None,
            transform,
            decoration: true,
            text: Some(Arc::new(text.volume)),
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

    /// Where a text added to copy `copy` of `object` at `hit` (a point and
    /// the face's normal there, in the scene) stands in the object, with
    /// the style's angle and distance: the frame [`Model::add_text`] gives
    /// it, worked out ahead so the text can be cut from the surface there.
    pub fn text_frame_on(
        &mut self,
        bed: Option<&Bed>,
        copy: u64,
        hit: (Vec3, Vec3),
        angle: Option<f64>,
        distance: Option<f64>,
    ) -> Result<Transform, Failure> {
        self.settle(bed);
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let back = scene
            .inverse()
            .ok_or_else(|| refused("the object is squashed flat"))?;
        Ok(frame_on(&back, hit.0, hit.1, angle, distance))
    }

    /// Makes a new object of a text, lying on plate `plate` (a position in
    /// [`Model::plates`]) at the free place nearest `near` (the bed's
    /// middle when none), and answers its copy (`CreateObjectJob`). The
    /// style's angle turns it about the world's Z; its distance is not
    /// used.
    pub fn add_text_object(
        &mut self,
        bed: Option<&Bed>,
        plate: usize,
        near: Option<(f64, f64)>,
        angle: Option<f64>,
        text: NewText,
    ) -> Result<u64, Failure> {
        if plate > self.plates.len().saturating_sub(1) && !self.plates.is_empty() {
            return Err(refused(format!("there is no plate {}", plate + 1)));
        }
        self.settle(bed);
        let name = text.name.clone();
        let part = Part {
            kept: None,
            name: Some(text.name),
            kind: PartKind::Normal,
            filament: 1,
            own_filament: None,
            mesh: Arc::new(text.mesh),
            paint: None,
            seam: None,
            transform: Transform::IDENTITY,
            decoration: true,
            text: Some(Arc::new(text.volume)),
        };
        let shape = Shape::new(vec![part]);
        let turned = apply_angle_distance(&Transform::IDENTITY, angle, None);
        let copy = self.seat((bed, plate), &shape, turned, near, true, &mut Vec::new());
        let id = copy.id;
        self.objects.push(super::ModelObject {
            id: fresh(),
            name,
            filament: None,
            shape,
            copies: vec![copy],
            kept: None,
        });
        Ok(id)
    }

    /// The text a part is, when it is text.
    pub fn text_of(&self, object: u64, part: u64) -> Result<Option<Arc<TextVolume>>, Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        Ok(self.objects[at].shape.parts[index].text.clone())
    }

    /// Whether a part is its object's only printed part
    /// (`ModelVolume::is_the_only_one_part`): "Use surface" is not for it.
    pub fn is_only_printed_part(&self, object: u64, part: u64) -> Result<bool, Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        let parts = &self.objects[at].shape.parts;
        Ok(parts[index].kind == PartKind::Normal
            && parts
                .iter()
                .filter(|held| held.kind == PartKind::Normal)
                .count()
                == 1)
    }

    /// Where a text part's own frame is in its object: its matrix, or for
    /// a text read from a project whose reference centred its mesh, the
    /// matrix that puts the text's frame back (`update_volume` with the
    /// shape's `fix_3mf_tr`).
    pub fn text_frame(&self, object: u64, part: u64) -> Result<Transform, Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        let held = &self.objects[at].shape.parts[index];
        let fix = held.text.as_ref().and_then(|text| text.shape.fix);
        Ok(match fix.and_then(|fix| fix.inverse()) {
            Some(unfix) => unfix
                .then(&Transform::translation(centre(&held.mesh)))
                .then(&held.transform),
            None => held.transform,
        })
    }

    /// Makes a text part again from a new description (`UpdateJob`): its
    /// mesh, name and description, standing in its own frame
    /// ([`Model::text_frame`]) or at `frame` when given; of `kind` when
    /// given (GLGizmoEmboss.cpp `draw_model_type`). What was painted on
    /// it goes with its old triangles; its settings and filament stay.
    pub fn set_text(
        &mut self,
        bed: Option<&Bed>,
        object: u64,
        part: u64,
        text: NewText,
        frame: Option<Transform>,
        kind: Option<PartKind>,
    ) -> Result<(), Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        let shape = self.objects[at].shape.clone();
        if shape.parts[index].text.is_none() {
            return Err(refused("that part is not text"));
        }
        if let Some(kind) = kind
            && kind != PartKind::Normal
            && !shape
                .parts
                .iter()
                .enumerate()
                .any(|(other, part)| other != index && part.kind == PartKind::Normal)
        {
            return Err(refused(
                "You can't change a type of the last solid part of the object.",
            ));
        }
        let frame = match frame {
            Some(frame) => frame,
            None => self.text_frame(object, part)?,
        };
        self.settle(bed);
        let mut list = shape.parts.clone();
        let held = &mut list[index];
        let stat = edge_3mf::MeshStat::default();
        held.kept = held
            .kept
            .as_deref()
            .map(|kept| Arc::new(kept.with_new_mesh(stat)));
        held.mesh = Arc::new(text.mesh);
        held.paint = None;
        held.seam = None;
        held.name = Some(text.name);
        held.transform = frame;
        held.text = Some(Arc::new(text.volume));
        held.decoration = true;
        if let Some(kind) = kind {
            held.kind = kind;
        }
        self.reshape(bed, at, list, shape.ids.clone());
        Ok(())
    }
}

/// The turn that takes direction `from` to `to` by the shortest way
/// (Eigen's `Quaternion::FromTwoVectors`), as a [`Transform`].
fn from_two_vectors(from: Vec3, to: Vec3) -> Transform {
    let (a, b) = (normalized(from), normalized(to));
    let cos = dot(a, b).clamp(-1.0, 1.0);
    let axis = cross(a, b);
    if dot(axis, axis) > 1e-24 {
        return transform_of(turn(cos.acos(), normalized(axis)));
    }
    if cos > 0.0 {
        return Transform::IDENTITY;
    }
    // Opposite: half a turn about any line square to them.
    let other = if a[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    transform_of(turn(std::f64::consts::PI, normalized(cross(a, other))))
}

/// The linear part of a transform as its columns, and back.
fn columns(t: &Transform) -> [Vec3; 3] {
    [t.rows[0], t.rows[1], t.rows[2]]
}

impl Model {
    /// How far a text part's origin is from the surface of its object's
    /// other printed parts, along its own Z (SurfaceDrag.cpp
    /// `calc_distance`): a line of sight both ways along the text's Z, the
    /// nearest surface, positive when the text stands off it. None when
    /// no surface is met, when it is nearer than 0.01 mm, or further than
    /// twice the text's depth or √10 mm, whichever is more.
    pub fn text_distance(
        &self,
        bed: Option<&Bed>,
        object: u64,
        part: u64,
        copy: u64,
    ) -> Result<Option<f64>, Failure> {
        let at = self.object_at(object)?;
        let index = self.part_at(at, part)?;
        if self.copy_at(copy)?.0 != at {
            return Err(refused(format!("copy {copy} is not of object {object}")));
        }
        let Some((scene, _)) = self.in_scene(bed, copy) else {
            return Ok(None);
        };
        let shape = &self.objects[at].shape;
        let held = &shape.parts[index];
        let world = self.text_frame(object, part)?.then(&scene);
        let point = world.apply([0.0; 3]);
        let dir = normalized(world.rows[2]).map(|c| -c);
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
        let mut nearest: Option<Vec3> = None;
        for (other, (part, tree)) in shape.parts.iter().zip(trees).enumerate() {
            let Some(tree) = tree else { continue };
            if other == index {
                continue;
            }
            let placed = part.transform.then(&scene);
            let Some(back) = placed.inverse() else {
                continue;
            };
            let origin = back.apply(point);
            for sign in [1.0, -1.0] {
                let ahead = back.apply(std::array::from_fn(|i| point[i] + sign * dir[i]));
                let direction: Vec3 = std::array::from_fn(|i| ahead[i] - origin[i]);
                if let Some(hit) = tree.hit(origin, direction, (0.0, f64::INFINITY), &|_| true) {
                    let hit = placed.apply(hit.point);
                    let far = |p: Vec3| (0..3).map(|i| (p[i] - point[i]).powi(2)).sum::<f64>();
                    if nearest.is_none_or(|was| far(hit) < far(was)) {
                        nearest = Some(hit);
                    }
                }
            }
        }
        let Some(hit) = nearest else { return Ok(None) };
        let to_hit: Vec3 = std::array::from_fn(|i| hit[i] - point[i]);
        let squared = dot(to_hit, to_hit);
        if squared < 1e-4 {
            return Ok(None);
        }
        let depth = held
            .mesh
            .bounds()
            .map_or(0.0, |(low, high)| f64::from(high[2] - low[2]));
        if squared > (2.0 * depth).powi(2).max(10.0) {
            return Ok(None);
        }
        let sign = if dot(to_hit, dir) > 0.0 { 1.0 } else { -1.0 };
        Ok(Some(sign * squared.sqrt()))
    }

    /// A text part's frame turned to face the camera ("Set text to face
    /// camera": SurfaceDrag.cpp `face_selected_volume_to_camera` with
    /// `get_volume_transformation`): its Z turned towards the eye at
    /// `eye` (a perspective view) or against `forward` (a parallel one),
    /// and with `keep_up` its up put back as its face suggests and its
    /// turn about Z kept. `frame` is its frame in its object.
    #[allow(clippy::too_many_arguments)]
    pub fn text_facing(
        &self,
        bed: Option<&Bed>,
        object: u64,
        copy: u64,
        frame: &Transform,
        eye: Option<Vec3>,
        forward: Vec3,
        keep_up: bool,
    ) -> Result<Transform, Failure> {
        let at = self.object_at(object)?;
        if self.copy_at(copy)?.0 != at {
            return Err(refused(format!("copy {copy} is not of object {object}")));
        }
        let (scene, _) = self
            .in_scene(bed, copy)
            .ok_or_else(|| refused(format!("there is no copy {copy}")))?;
        let back = scene
            .inverse()
            .ok_or_else(|| refused("the object is squashed flat"))?;
        let world = frame.then(&scene);
        let current_angle = if keep_up {
            calc_up(&world, UP_LIMIT)
        } else {
            None
        };
        let position = world.apply([0.0; 3]);
        let wanted = match eye {
            Some(eye) => std::array::from_fn(|i| eye[i] - position[i]),
            None => forward.map(|c| -c),
        };
        // The text's Z made square to its X and Y, its length kept along
        // the old one.
        let [x, y, old_z] = columns(&world);
        let new_z = cross(x, y);
        let z = new_z.map(|c| c * dot(old_z, new_z) / dot(new_z, new_z));
        let mut linear = transform_of([x, y, z]).then(&from_two_vectors(z, wanted));
        if keep_up {
            let z_world = normalized(linear.rows[2]);
            let up = suggest_up(z_world, UP_LIMIT);
            linear = linear.then(&from_two_vectors(linear.rows[1], up));
        }
        // Into the object: the position and the turn taken back through
        // the copy.
        let mut unturn = back;
        unturn.rows[3] = [0.0; 3];
        let mut out = linear.then(&unturn);
        out.rows[3] = back.apply(position);
        if out.rows.iter().flatten().any(|v| !v.is_finite()) {
            return Ok(*frame);
        }
        Ok(apply_angle_distance(&out, current_angle, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn text_on_a_wall_reads_upright_and_faces_out() {
        // A wall facing -Y (the front of an object).
        let t = onto_surface([1.0, 2.0, 3.0], [0.0, -1.0, 0.0], UP_LIMIT);
        assert!(close(t.rows[2], [0.0, -1.0, 0.0]), "{:?}", t.rows);
        assert!(close(t.rows[1], [0.0, 0.0, 1.0]), "{:?}", t.rows);
        // Read from in front, left to right is +X.
        assert!(close(t.rows[0], [1.0, 0.0, 0.0]), "{:?}", t.rows);
        assert!(close(t.rows[3], [1.0, 2.0, 3.0]));
        assert_eq!(calc_up(&t, UP_LIMIT), None);
    }

    #[test]
    fn text_on_a_top_or_bottom_face_has_the_world_y_up() {
        let top = onto_surface([0.0; 3], [0.0, 0.0, 1.0], UP_LIMIT);
        assert!(close(top.rows[1], [0.0, 1.0, 0.0]), "{:?}", top.rows);
        assert!(close(top.rows[2], [0.0, 0.0, 1.0]));
        let bottom = onto_surface([0.0; 3], [0.0, 0.0, -1.0], UP_LIMIT);
        assert!(close(bottom.rows[2], [0.0, 0.0, -1.0]), "{:?}", bottom.rows);
        assert!(close(bottom.rows[1], [0.0, 1.0, 0.0]), "{:?}", bottom.rows);
        assert!(bottom.determinant() > 0.0);
    }

    #[test]
    fn the_angle_turns_about_the_texts_own_z_and_reads_back() {
        let t = onto_surface([0.0; 3], [1.0, 0.0, 0.0], UP_LIMIT);
        let turned = apply_angle_distance(&t, Some(0.5), Some(2.0));
        let up = calc_up(&turned, UP_LIMIT).unwrap();
        assert!((up - 0.5).abs() < 1e-9, "{up}");
        // Moved along the face's normal.
        assert!(
            close(turned.rows[3], [2.0, 0.0, 0.0]),
            "{:?}",
            turned.rows[3]
        );
    }
}
