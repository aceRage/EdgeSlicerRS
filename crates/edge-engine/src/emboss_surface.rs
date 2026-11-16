//! Text that follows the surface it stands on ("Use surface"): ported in
//! intent from libslic3r/CutSurface.cpp (`cut_surface`, `cut2model`) and
//! its callers in slic3r/GUI/Jobs/EmbossJob.cpp (`cut_surface_to_its`,
//! `create_projection_for_cut`, `create_emboss_projection`, `cut_surface`,
//! `create_sources`, `create_volume_sources`, `start_update_volume`,
//! `CreateVolumesJob::process`) and slic3r/GUI/SurfaceDrag.cpp
//! (`calc_surface_offset`).
//!
//! What the reference makes. The text's shape is projected along the
//! text's own Z onto the object's other printed parts, all in the text's
//! frame; the part of their surface under the shape that faces the text
//! is cut out along the shape's outline (CGAL's corefinement of the
//! surface with the shape's outline drawn through the model,
//! `cut_from_model`); where the projection meets several layers of surface
//! one behind another, each letter takes the layer nearest the text's own
//! plane and continues on whatever layer is next to it, and a layer
//! further back is taken only where it fills a place the nearer ones leave
//! open (`choose_best_distance`, `select_patches`). That cut surface is
//! then made a solid by a copy of it moved along the text's Z, joined to
//! it along the outline (`cut2model`): a printed part from 0.015 mm under
//! the surface to the depth over it, a negative part or modifier from
//! 0.015 mm over it to the depth under it (`create_emboss_projection`).
//!
//! How it is made here, and why. CGAL is not available here, and a port
//! of the 4,000 lines of corefinement and patch bookkeeping would need its
//! exact arithmetic again. The one thing CGAL is used for, cutting the
//! model's surface along the shape's outline drawn through it, is what a
//! boolean does: the model intersected with the shape's prism (the shape
//! raised along Z past the model either way) is bounded by the model's
//! surface inside the prism, cut along the outline, and by the prism's
//! sides, which stand square to the text. So that one boolean goes
//! through `edge_mesh::boolean`, whose exact predicates make the cut, and
//! the rest is the reference's own steps on its result, in plain Rust:
//! the faces that look towards the text more than 89.9° (`max_angle`,
//! which also leaves out the prism's sides) are the candidate surface,
//! in patches connected across their edges; where several layers lie one
//! behind another, they are taken nearest the text's plane first, and a
//! patch further back only where, seen along Z, some of it is not covered
//! by those taken grown by a hundredth of the shape's size
//! (`select_patches`' `extend_delta`, which also adds a whole patch where
//! it fills some
//! of what is left); and the patches taken are made a solid as
//! `cut2model` makes it, the surface moved by the front and the back
//! amounts and walls along its outline. Several printed parts are united
//! first, so that surface inside another part is not used (the reference
//! cuts each part and throws away what lies in another, `diff_models`).
//!
//! To keep a change of the text quick (it is made again on every
//! keystroke), the model is first cut down to a box about the text's
//! origin a power of two millimetres across, kept for the next change
//! ([`Surface::near`]): the result depends on the text alone, not on the
//! edits before it.
//!
//! Left out on purpose: the reference picks layers point by point along
//! the letters' outlines, starting from the one nearest the middle and
//! carrying the depth on to the next letter (`choose_best_distance`), and
//! then fills what is left by patches sorted by their depth; here every
//! patch is ranked by how far it is from the text's plane, which gives the
//! same layers where they do not touch. The reference turns the shape
//! round when the text's frame is mirrored (`has_reflection`) to keep
//! CGAL's outlines anticlockwise; the frame's mirror is undone here with
//! the model's own corners, which needs no turning. Its "safe extension"
//! of the projection range (`safe_extension`) and the cut down to the
//! area of interest (`its_cut_AoI`) keep CGAL fast and inside its range;
//! the prism here reaches a millimetre past the model either way instead.
//! A part whose mesh does not bound a volume cannot take part in a boolean
//! and is left out of the surface (the reference cuts open surfaces too).

use std::sync::{Arc, Mutex};

use edge_3mf::{EmbossShape, PartKind, TextConfiguration, TextVolume};
use edge_geom::{ExPolygon, Point2, Polygon, scaled};
use edge_mesh::boolean::{Op, Solid};
use edge_mesh::raycast::Tree;
use edge_mesh::{Mesh, Transform};
use rustc_hash::FxHashMap;

use super::super::{Model, refused};
use super::{NewText, SAFE_SURFACE_OFFSET, TextFonts, is_text_empty, text_name, text_shape};
use crate::Failure;

type Vec3 = [f64; 3];

/// The reference's words when the cut gives nothing (EmbossJob.cpp
/// `cut_surface`).
pub const NO_SURFACE: &str = "There is no valid surface for text projection.";

/// The surface a text is projected onto: an object's printed parts other
/// than the text, united, in the text's frame (`create_volume_sources`).
/// Made once for a frame and used for every change of the text that keeps
/// the frame.
pub struct Surface {
    solid: Solid,
    /// The lowest and highest Z of the parts in the text's frame.
    low: f64,
    high: f64,
    /// The solid cut down to boxes about the text's origin, by their half
    /// sizes along X and Y, the latest last (see [`Surface::near`]).
    near: Mutex<Vec<([f64; 2], Solid)>>,
}

/// The parts a [`Surface`] is made of, each placed in the text's frame;
/// equal when they are the same meshes placed the same way.
#[derive(Clone, Debug)]
pub struct SurfaceKey {
    parts: Vec<(Arc<Mesh>, Transform)>,
}

impl PartialEq for SurfaceKey {
    fn eq(&self, other: &Self) -> bool {
        self.parts.len() == other.parts.len()
            && (self.parts.iter().zip(&other.parts))
                .all(|(a, b)| Arc::ptr_eq(&a.0, &b.0) && a.1 == b.1)
    }
}

impl SurfaceKey {
    /// The surface of these parts; none when none bounds a volume.
    pub fn surface(&self) -> Option<Surface> {
        Surface::new(
            self.parts
                .iter()
                .map(|(mesh, place)| (mesh.as_ref(), *place)),
        )
    }
}

/// How many cut-down solids a surface keeps.
const KEPT: usize = 3;

/// The smallest half size of a box a surface is cut down to, in mm.
const SMALLEST: f64 = 8.0;

impl std::fmt::Debug for Surface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surface")
            .field("low", &self.low)
            .field("high", &self.high)
            .finish()
    }
}

impl Surface {
    /// The surface of these meshes, each placed in the text's frame by its
    /// transform; none when none of them bounds a volume.
    pub fn new<'a>(parts: impl IntoIterator<Item = (&'a Mesh, Transform)>) -> Option<Surface> {
        let mut solid: Option<Solid> = None;
        for (mesh, transform) in parts {
            if mesh.triangles.is_empty() {
                continue;
            }
            let mut placed = Mesh::default();
            placed.append(mesh, &transform);
            let Ok(part) = Solid::new(&placed) else {
                continue;
            };
            solid = Some(match solid {
                None => part,
                Some(solid) => match solid.boolean(&part, Op::Union) {
                    Ok(union) => union,
                    Err(_) => solid,
                },
            });
        }
        let solid = solid?;
        let (low, high) = solid.bounds()?;
        Some(Surface {
            solid,
            low: low[2],
            high: high[2],
            near: Mutex::new(Vec::new()),
        })
    }

    /// The surface within a box about the text's origin reaching at least
    /// `reach` (mm along X and Y either way) and through everything along
    /// Z: what a text that far across is cut from, so that each change of
    /// the text works on that much of the model rather than all of it.
    /// The box's half sizes are powers of two, so the solid depends on how
    /// far the text reaches and nothing else (not on the changes before
    /// it), and is kept for the next change, which mostly fits the same
    /// box. Its sides are past the text, so the text's solid never meets
    /// them.
    fn near(&self, reach: [f64; 2]) -> Result<Solid, edge_mesh::boolean::BooleanError> {
        let half = reach.map(|r| {
            let mut half = SMALLEST;
            while half < r && half < 1e6 {
                half *= 2.0;
            }
            half
        });
        let mut kept = self.near.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = kept.iter().position(|(h, _)| *h == half) {
            let hit = kept.remove(at);
            let solid = hit.1.clone();
            kept.push(hit);
            return Ok(solid);
        }
        let (low, high) = ((self.low - 2.0) as f32, (self.high + 2.0) as f32);
        let block = Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
            [-half[0] as f32, -half[1] as f32, low],
            [half[0] as f32, half[1] as f32, high],
        ));
        let block =
            Solid::new(&block).map_err(|error| edge_mesh::boolean::BooleanError::Input {
                operand: edge_mesh::boolean::Operand::Second,
                error,
            })?;
        let solid = self.solid.boolean(&block, Op::Intersection)?.rechecked();
        if kept.len() >= KEPT {
            kept.remove(0);
        }
        kept.push((half, solid.clone()));
        Ok(solid)
    }
}

/// The text of `config` raised `depth` mm off `surface`, following it, for
/// a part of `kind`, in the text's own frame (EmbossJob.cpp `cut_surface`
/// and `cut_surface_to_its`): what the part's mesh is with "Use surface".
/// Refused as the flat text is, and with the reference's words when the
/// shape falls on no surface.
pub fn emboss_on_surface(
    fonts: TextFonts<'_>,
    config: &TextConfiguration,
    depth: f64,
    kind: PartKind,
    surface: &Surface,
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
    // `create_emboss_projection`: how far the surface is moved along the
    // text's Z for the solid's front and back.
    let (front, back) = if kind == PartKind::Normal {
        (depth, -SAFE_SURFACE_OFFSET)
    } else {
        (SAFE_SURFACE_OFFSET, -depth)
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
    let z0 = (surface.low + back - 1.0) as f32;
    let z1 = (surface.high + front + 1.0) as f32;
    let prism = edge_mesh::primitives::prism(&regions, z0, z1)
        .ok_or_else(|| refused("the text's shape could not be closed into a solid"))?;
    let prism = Solid::new(&prism).map_err(|e| refused(format!("the text's shape {e}")))?;
    let fail = |e: edge_mesh::boolean::BooleanError| refused(format!("{NO_SURFACE} ({e})"));
    // The model near the text, a millimetre past the text's box.
    let reach = regions
        .iter()
        .flat_map(|(outline, _)| outline)
        .fold([0.0f64; 2], |r, p| {
            [
                r[0].max(f64::from(p[0].abs())),
                r[1].max(f64::from(p[1].abs())),
            ]
        })
        .map(|r| r + 1.0);
    let model = surface.near(reach).map_err(fail)?;
    // The model within the prism: its surface there is cut along the
    // shape's outline, as CGAL's corefinement cuts it (`cut_from_model`).
    let within = prism
        .boolean(&model, Op::Intersection)
        .and_then(|solid| solid.mesh())
        .map_err(fail)?;
    let patches = patches(&within);
    let (low, high) = regions.iter().flat_map(|(outline, _)| outline).fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(l, h), p| {
            (
                [l[0].min(f64::from(p[0])), l[1].min(f64::from(p[1]))],
                [h[0].max(f64::from(p[0])), h[1].max(f64::from(p[1]))],
            )
        },
    );
    let delta = ((high[0] - low[0]) + (high[1] - low[1])) / (2.0 * 100.0);
    let chosen = nearest_layers(&within, patches, delta);
    let mesh = cut2model(&within, &chosen, front, back);
    if mesh.triangles.is_empty() {
        return Err(refused(NO_SURFACE));
    }
    Ok(NewText {
        name: text_name(&config.text),
        volume: TextVolume {
            text: config.clone(),
            shape: EmbossShape {
                scale,
                unhealed: !healed,
                depth,
                use_surface: true,
                ..EmbossShape::default()
            },
        },
        mesh,
    })
}

/// The cosine of the steepest a face may be to the text's Z and still be
/// cut (CutSurface.cpp `cut_surface`: `max_angle` 89.9°).
fn steepest() -> f64 {
    89.9f64.to_radians().cos()
}

/// The faces of the model's surface within the prism that look towards
/// the text (`set_skip_by_angle`), in pieces that are connected across
/// their edges (the reference's surface patches, `create_surface_patch`),
/// each a list of triangles, in the order of their first triangle. The
/// prism's own sides stand square to the text and so are never among
/// them; its ends lie past the model.
fn patches(within: &Mesh) -> Vec<Vec<usize>> {
    let limit = steepest();
    let facing: Vec<bool> = (0..within.triangles.len())
        .map(|t| {
            let [a, b, c] = within.triangle(t).map(|p| p.map(f64::from));
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            length > 0.0 && n[2] > limit * length
        })
        .collect();
    let mut edges: FxHashMap<(u32, u32), usize> = FxHashMap::default();
    for (t, tri) in within.triangles.iter().enumerate() {
        if facing[t] {
            for k in 0..3 {
                edges.insert((tri[k], tri[(k + 1) % 3]), t);
            }
        }
    }
    let mut piece = vec![usize::MAX; within.triangles.len()];
    let mut out: Vec<Vec<usize>> = Vec::new();
    for start in 0..within.triangles.len() {
        if !facing[start] || piece[start] != usize::MAX {
            continue;
        }
        let id = out.len();
        let mut list = vec![start];
        piece[start] = id;
        let mut at = 0;
        while at < list.len() {
            let tri = within.triangles[list[at]];
            at += 1;
            for k in 0..3 {
                if let Some(&next) = edges.get(&(tri[(k + 1) % 3], tri[k]))
                    && piece[next] == usize::MAX
                {
                    piece[next] = id;
                    list.push(next);
                }
            }
        }
        list.sort_unstable();
        out.push(list);
    }
    out
}

/// The patches nearest the text's plane, and those further back that
/// fill what they leave open (see the module's words): the triangles
/// kept, in order. `delta` is how far, in mm, what is taken is grown
/// before it is checked what is left (`select_patches`' `extend_delta`, a
/// hundredth of the mean of the shape's width and height), so that the
/// slivers between patches next to each other do not count as open.
fn nearest_layers(within: &Mesh, patches: Vec<Vec<usize>>, delta: f64) -> Vec<usize> {
    if patches.len() <= 1 {
        return patches.into_iter().flatten().collect();
    }
    // How far the text's plane is from each patch's reach along Z, and
    // from its middle.
    let mut order: Vec<(f64, f64, usize)> = patches
        .iter()
        .enumerate()
        .map(|(at, list)| {
            let (low, high) = list
                .iter()
                .flat_map(|&t| within.triangles[t])
                .map(|v| f64::from(within.vertices[v as usize][2]))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), z| {
                    (l.min(z), h.max(z))
                });
            let gap = if low > 0.0 {
                low
            } else if high < 0.0 {
                -high
            } else {
                0.0
            };
            (gap, ((low + high) / 2.0).abs(), at)
        })
        .collect();
    order.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    // A patch as seen along Z (`to_expoly`): its triangles united.
    let seen = |list: &[usize]| -> Vec<ExPolygon> {
        let triangles: Vec<Polygon> = list
            .iter()
            .map(|&t| Polygon {
                points: within
                    .triangle(t)
                    .iter()
                    .map(|p| Point2::from_mm(f64::from(p[0]), f64::from(p[1])))
                    .collect(),
            })
            .collect();
        edge_clip::union(&triangles).unwrap_or_default()
    };
    let grow = scaled(delta);
    let mut covered: Vec<ExPolygon> = Vec::new();
    let mut kept: Vec<usize> = Vec::new();
    for (_, _, at) in order {
        let list = &patches[at];
        let area = seen(list);
        if !kept.is_empty() {
            let open = edge_clip::difference(&area, &covered).unwrap_or_default();
            if open.iter().map(ExPolygon::area_mm2).sum::<f64>() <= 0.0 {
                continue;
            }
        }
        let grown = edge_clip::offset(&area, grow, edge_clip::Join::MITER).unwrap_or(area);
        covered.extend(grown);
        covered = edge_clip::union_ex(&covered).unwrap_or(covered);
        kept.extend_from_slice(list);
    }
    kept.sort_unstable();
    kept
}

/// The cut surface made a solid (CutSurface.cpp `cut2model`): its copy
/// moved `front` along Z, facing out, its copy moved `back`, facing in,
/// and between them a wall along every edge of the cut that no other
/// triangle of it shares (the cut's outline). A corner where two pieces
/// of the cut touch at a point only is made a corner of each, so the
/// walls meeting there do not share an edge (the reference shares it).
fn cut2model(within: &Mesh, kept: &[usize], front: f64, back: f64) -> Mesh {
    // The kept triangles' half-edges, by their corners in the cut.
    let mut edges: FxHashMap<(u32, u32), usize> = FxHashMap::default();
    for (at, &t) in kept.iter().enumerate() {
        let tri = within.triangles[t];
        for k in 0..3 {
            edges.insert((tri[k], tri[(k + 1) % 3]), at);
        }
    }
    // One node for each corner of each kept triangle; the corners at the
    // ends of an edge two kept triangles share are one corner.
    let mut parent: Vec<u32> = (0..3 * kept.len() as u32).collect();
    fn root(parent: &mut [u32], mut at: u32) -> u32 {
        while parent[at as usize] != at {
            parent[at as usize] = parent[parent[at as usize] as usize];
            at = parent[at as usize];
        }
        at
    }
    let slot = |at: usize, v: u32| -> u32 {
        let tri = within.triangles[kept[at]];
        (3 * at + tri.iter().position(|&c| c == v).unwrap_or(0)) as u32
    };
    for (at, &t) in kept.iter().enumerate() {
        let tri = within.triangles[t];
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            if let Some(&other) = edges.get(&(b, a)) {
                for v in [a, b] {
                    let (x, y) = (
                        root(&mut parent, slot(at, v)),
                        root(&mut parent, slot(other, v)),
                    );
                    // The lower root wins, so the order is fixed.
                    let (low, high) = (x.min(y), x.max(y));
                    parent[high as usize] = low;
                }
            }
        }
    }
    // A vertex for each corner, in the order corners are first met.
    let mut number: Vec<u32> = vec![u32::MAX; parent.len()];
    let mut source: Vec<u32> = Vec::new();
    let mut ids = vec![[0u32; 3]; kept.len()];
    for (at, &t) in kept.iter().enumerate() {
        let tri = within.triangles[t];
        for k in 0..3 {
            let r = root(&mut parent, (3 * at + k) as u32) as usize;
            if number[r] == u32::MAX {
                number[r] = source.len() as u32;
                source.push(tri[k]);
            }
            ids[at][k] = number[r];
        }
    }
    let count = source.len() as u32;
    let moved = |by: f64| {
        source.iter().map(move |&v| {
            let p = within.vertices[v as usize];
            [p[0], p[1], (f64::from(p[2]) + by) as f32]
        })
    };
    let mut mesh = Mesh {
        vertices: moved(front).chain(moved(back)).collect(),
        triangles: Vec::with_capacity(kept.len() * 4),
    };
    for [a, b, c] in &ids {
        mesh.triangles.push([*a, *b, *c]);
        mesh.triangles.push([a + count, c + count, b + count]);
    }
    for (at, &t) in kept.iter().enumerate() {
        let tri = within.triangles[t];
        for k in 0..3 {
            if !edges.contains_key(&(tri[(k + 1) % 3], tri[k])) {
                let (a, b) = (ids[at][k], ids[at][(k + 1) % 3]);
                mesh.triangles.push([b, a, a + count]);
                mesh.triangles.push([b, a + count, b + count]);
            }
        }
    }
    mesh
}

impl Model {
    /// The surface a text part in `object` is projected onto, in the frame
    /// `frame` (the text's, in the object): the object's printed parts
    /// other than `part` (`create_volume_sources`, `create_sources`); for
    /// a text not yet added, `part` is none. None when there is no such
    /// part, or none bounds a volume.
    pub fn text_surface(
        &self,
        object: u64,
        part: Option<u64>,
        frame: &Transform,
    ) -> Result<Option<Surface>, Failure> {
        Ok(self.text_surface_key(object, part, frame)?.surface())
    }

    /// What [`Model::text_surface`] is made from, to tell whether a
    /// surface made before can be used again: the same parts' meshes,
    /// placed the same way, under the same frame.
    pub fn text_surface_key(
        &self,
        object: u64,
        part: Option<u64>,
        frame: &Transform,
    ) -> Result<SurfaceKey, Failure> {
        let at = self.object_at(object)?;
        let shape = &self.objects[at].shape;
        let back = frame
            .inverse()
            .ok_or_else(|| refused("the text is squashed flat"))?;
        Ok(SurfaceKey {
            parts: (shape.parts.iter().zip(&shape.ids))
                .filter(|(held, id)| held.kind == PartKind::Normal && Some(**id) != part)
                .map(|(held, _)| (held.mesh.clone(), held.transform.then(&back)))
                .collect(),
        })
    }

    /// How far a text's origin is to be moved, in its own frame, to lie on
    /// the surface when "Use surface" is turned on (SurfaceDrag.cpp
    /// `calc_surface_offset`): to the nearest point the line along its Z
    /// meets on the object's other parts either way, or when it meets none
    /// the nearest point of them; none when that is nearer than the
    /// reference's `EPSILON` squared.
    pub fn text_surface_offset(
        &self,
        object: u64,
        part: u64,
        frame: &Transform,
    ) -> Result<Option<Vec3>, Failure> {
        let at = self.object_at(object)?;
        let shape = &self.objects[at].shape;
        let back = frame
            .inverse()
            .ok_or_else(|| refused("the text is squashed flat"))?;
        let mut hit: Option<(f64, Vec3)> = None;
        let mut close: Option<(f64, Vec3)> = None;
        let far = |p: Vec3| p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        for (held, id) in shape.parts.iter().zip(&shape.ids) {
            if *id == part || held.mesh.triangles.is_empty() {
                continue;
            }
            // The part in the text's frame, whose origin is the text's.
            let mut placed = Mesh::default();
            placed.append(&held.mesh, &held.transform.then(&back));
            let tree = Tree::new(&placed);
            for sign in [1.0, -1.0] {
                if let Some(h) =
                    tree.hit([0.0; 3], [0.0, 0.0, -sign], (0.0, f64::INFINITY), &|_| true)
                    && hit.is_none_or(|(d, _)| far(h.point) < d)
                {
                    hit = Some((far(h.point), h.point));
                }
            }
            if let Some((point, _, _)) = tree.closest_point([0.0; 3])
                && close.is_none_or(|(d, _)| far(point) < d)
            {
                close = Some((far(point), point));
            }
        }
        // Slic3r's `EPSILON`.
        Ok(hit.or(close).filter(|(d, _)| *d >= 1e-4).map(|(_, p)| p))
    }
}

/// The frame moved by `offset` in its own coordinates (`volume_tr *=
/// Translation(offset)`).
pub fn moved_in_frame(frame: &Transform, offset: Vec3) -> Transform {
    Transform::translation(offset).then(frame)
}
