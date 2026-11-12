//! The painting tools of Prepare: "Support Painting", "Seam painting",
//! "Paint-on fuzzy skin" and "Color Painting" (Gizmos/GLGizmoPainterBase.cpp
//! and the four gizmos built on it).
//!
//! The paint is held here while a tool is open, one selector
//! (`edge_engine::selector`, the reference's `TriangleSelector`) for each
//! printed part of the object painted, with a tree of boxes round the
//! part's triangles to find what the pointer is on. The page sends the
//! lines of sight under the pointer (`paint_event`); what they meet is
//! found and painted here, and the pieces of the triangles whose paint
//! changed go back in bytes, a block of triangles at a time, for the page
//! to draw over the object. When the button is let go the paint goes into
//! the model as one step of the undo history, as the reference takes a
//! snapshot when the mouse is released (`GLGizmoPainterBase::gizmo_event`,
//! `LeftUp`). Why the work is here and not in the page is in
//! `UI_PARITY.md`, section 11, with what was measured.
//!
//! Calls:
//!
//! - `paint_open { project, copy, kind }`: opens a tool on the object of
//!   a copy; `kind` is `supports`, `seam`, `color` or `fuzzy_skin`.
//! - `paint_close`.
//! - `paint_mesh { project }` (bytes): the painted parts' triangles,
//!   unpainted, as one mesh in the object's coordinates (`scene_mesh`'s
//!   layout), each triangle's mark its part's filament.
//! - `paint_event { … }` (bytes): see [`Session::paint_event`].

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Weak},
    time::Instant,
};

use edge_engine::{
    Mesh, PaintKind, PartKind, Shape,
    raycast::Tree,
    selector::{Affine, Clip, Cursor, Selector, Shape as Brush, Topology},
};
use serde_json::{Value, json};

use super::{File, Session, edit::push_step, lock, needed, whole};

/// The calls answered here as JSON.
pub(super) const JSON: [&str; 2] = ["paint_open", "paint_close"];
/// The calls answered here in bytes.
pub(super) const BYTES: [&str; 2] = ["paint_mesh", "paint_event"];

/// How many triangles of a part's mesh go back to the page together.
const CHUNK: usize = 1024;

/// The mark of a piece a fill has marked and not yet painted: this more
/// than its state (the page draws it lighter, as the reference does:
/// `TriangleSelectorGUI::get_seed_fill_color`).
pub const MARKED: u8 = 128;

/// A mesh's topology and tree of boxes, while the mesh is held.
type Tools = (Weak<Mesh>, Arc<Topology>, Arc<Tree>);

/// What the selectors and trees made for a mesh, kept for as long as the
/// mesh is held, so that opening a tool again on the same object is quick.
#[derive(Default)]
pub(super) struct Cache {
    meshes: std::sync::Mutex<HashMap<usize, Tools>>,
    painter: std::sync::Mutex<Option<Painter>>,
}

/// One printed part being painted.
struct Painted {
    id: u64,
    /// Its own coordinates to the object's.
    transform: Affine,
    mirrored: bool,
    filament: usize,
    mesh: Arc<Mesh>,
    tree: Arc<Tree>,
    selector: Selector,
}

/// A tool open on an object.
pub(super) struct Painter {
    project: u32,
    object: u64,
    kind: PaintKind,
    /// The highest state the tool keeps: what is painted above it is read
    /// as unpainted (`deserialize` with `max_ebt`).
    max_state: u8,
    /// The shape the selectors were loaded from or written to.
    shape: Arc<Shape>,
    parts: Vec<Painted>,
    /// The part a fill last marked (`m_seed_fill_last_mesh_id`).
    fill_part: Option<usize>,
    /// The gap fill's area while that tool is chosen, whose preview colours
    /// the pieces it would fill.
    gap: Option<f32>,
}

fn kind_of(name: &str) -> Result<PaintKind, String> {
    Ok(match name {
        "supports" => PaintKind::Supports,
        "seam" => PaintKind::Seam,
        "color" => PaintKind::Color,
        "fuzzy_skin" => PaintKind::FuzzySkin,
        other => return Err(format!("kind cannot be {other}")),
    })
}

fn numbers<const N: usize>(value: Option<&Value>, what: &str) -> Result<[f64; N], String> {
    let list = value
        .and_then(Value::as_array)
        .filter(|list| list.len() == N)
        .ok_or_else(|| format!("{what} is not {N} numbers"))?;
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(list) {
        *slot = item
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("{what} is not {N} numbers"))?;
    }
    Ok(out)
}

/// A matrix of the page's kind (sixteen numbers, column by column) as an
/// affine map.
fn affine_of(value: Option<&Value>, what: &str) -> Result<Affine, String> {
    let m = numbers::<16>(value, what)?;
    Ok(Affine {
        m: std::array::from_fn(|i| std::array::from_fn(|j| m[4 * j + i])),
        t: [m[12], m[13], m[14]],
    })
}

fn to32(p: [f64; 3]) -> [f32; 3] {
    p.map(|c| c as f32)
}

/// The bytes of a mesh's triangles for the page's mesh program: each
/// triangle three vertices of 16 bytes (`renderer.js` `mesh`): `f32` x, y,
/// z, the face's normal as three `i8`, and a mark.
fn push_triangle(out: &mut Vec<u8>, corners: [[f32; 3]; 3], mark: u8) {
    let [a, b, c] = corners;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length > 0.0 {
        n = n.map(|value| value / length);
    } else {
        n = [0.0, 0.0, 1.0];
    }
    for corner in [a, b, c] {
        for value in corner {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in n {
            out.push((value * 127.0).round() as i8 as u8);
        }
        out.push(mark);
    }
}

impl Painted {
    /// A piece in the object's coordinates, wound to face out.
    fn placed(&self, corners: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
        let [a, b, c] = corners.map(|p| self.transform.apply(p));
        if self.mirrored { [a, c, b] } else { [a, b, c] }
    }

    /// The pieces of one block of triangles that are painted or marked.
    /// `zero` is the state an unpainted piece a fill has marked is drawn
    /// as: the part's filament for colours, which the unpainted part is
    /// drawn in.
    fn chunk_bytes(
        &self,
        chunk: usize,
        gaps: &HashMap<usize, u8>,
        zero: u8,
        out: &mut Vec<u8>,
    ) -> u32 {
        let mut pieces = Vec::new();
        let mut ids = Vec::new();
        let mut count = 0;
        let end = ((chunk + 1) * CHUNK).min(self.selector.triangle_count());
        for at in chunk * CHUNK..end {
            pieces.clear();
            self.selector.leaves(at, &mut pieces);
            if !gaps.is_empty() {
                ids.clear();
                self.selector.leaf_ids(at, &mut ids);
            }
            for (k, (corners, state, marked)) in pieces.iter().enumerate() {
                let state = if gaps.is_empty() {
                    *state
                } else {
                    gaps.get(&ids[k]).copied().unwrap_or(*state)
                };
                let mark = if *marked {
                    MARKED.saturating_add((if state == 0 { zero } else { state }).min(MARKED - 1))
                } else if state == 0 {
                    continue;
                } else {
                    state.min(MARKED - 1)
                };
                push_triangle(out, self.placed(*corners), mark);
                count += 1;
            }
        }
        count
    }

    fn chunks(&self) -> usize {
        self.selector.triangle_count().div_ceil(CHUNK)
    }
}

/// What a line of sight met.
#[derive(Clone, Copy, Debug)]
struct Met {
    part: usize,
    facet: usize,
    /// In the part's own coordinates.
    point: [f64; 3],
    t: f64,
}

impl Painter {
    /// The world's coordinates of each part (`matrix` takes the object's to
    /// the world's).
    fn worlds(&self, matrix: &Affine) -> Vec<Affine> {
        self.parts
            .iter()
            .map(|part| matrix.after(&part.transform))
            .collect()
    }

    /// What a line of sight (in the world) meets first, what a section
    /// view cuts away passed through (`update_raycast_cache`).
    fn cast(
        &self,
        worlds: &[Affine],
        origin: [f64; 3],
        direction: [f64; 3],
        clip: Option<Clip>,
    ) -> Option<Met> {
        let mut best: Option<Met> = None;
        for (index, (part, world)) in self.parts.iter().zip(worlds).enumerate() {
            let Some(back) = world.inverse() else {
                continue;
            };
            let o = back.apply64(origin);
            let d: [f64; 3] = std::array::from_fn(|i| {
                back.m[i][0] * direction[0]
                    + back.m[i][1] * direction[1]
                    + back.m[i][2] * direction[2]
            });
            let keep =
                |point: [f64; 3]| clip.is_none_or(|clip| !clip.clipped(to32(world.apply64(point))));
            if let Some(hit) = part.tree.hit(o, d, (0.0, f64::INFINITY), &keep)
                && best.is_none_or(|met| hit.t < met.t)
            {
                best = Some(Met {
                    part: index,
                    facet: hit.triangle,
                    point: hit.point,
                    t: hit.t,
                });
            }
        }
        best
    }
}

/// The clipping plane in a part's coordinates
/// (`get_clipping_plane_in_volume_coordinates`).
fn clip_in(clip: Option<Clip>, world: &Affine) -> Option<Clip> {
    let clip = clip?;
    let back = world.inverse()?;
    let n = clip.normal.map(f64::from);
    let on = back.apply64(n.map(|c| c * f64::from(clip.offset)));
    // The normal by the transpose of the linear part.
    let normal: [f64; 3] = std::array::from_fn(|j| (0..3).map(|i| world.m[i][j] * n[i]).sum());
    let offset = (0..3).map(|i| on[i] * normal[i]).sum::<f64>();
    Some(Clip {
        normal: to32(normal),
        offset: offset as f32,
    })
}

/// The points a dragged stroke goes through, as the reference makes them
/// (`get_projected_mouse_positions`): the hits in order, in runs on one
/// part; a run on one triangle reduced to its ends, a longer one to the
/// points that keep its shape to a tenth of the brush's radius.
///
/// The reference projects a run onto the plane through three of its
/// points, simplifies it there and finds the triangle nearest each point
/// it keeps; here the points kept are the hits themselves, which are on
/// the surface already.
fn runs(hits: &[Met], tolerance: f64) -> Vec<Vec<Met>> {
    let mut out: Vec<Vec<Met>> = Vec::new();
    for hit in hits {
        match out.last_mut() {
            Some(run) if run[0].part == hit.part => run.push(*hit),
            _ => out.push(vec![*hit]),
        }
    }
    for run in &mut out {
        if run.len() <= 2 {
            continue;
        }
        if run.iter().all(|met| met.facet == run[0].facet) {
            *run = vec![run[0], run[run.len() - 1]];
            continue;
        }
        let mut keep = vec![false; run.len()];
        keep[0] = true;
        keep[run.len() - 1] = true;
        douglas_peucker(run, 0, run.len() - 1, tolerance, &mut keep);
        *run = run
            .iter()
            .zip(&keep)
            .filter(|(_, kept)| **kept)
            .map(|(met, _)| *met)
            .collect();
    }
    out
}

fn douglas_peucker(run: &[Met], from: usize, to: usize, tolerance: f64, keep: &mut [bool]) {
    if to <= from + 1 {
        return;
    }
    let (a, b) = (run[from].point, run[to].point);
    let ab: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let length2: f64 = ab.iter().map(|c| c * c).sum();
    let mut far = (0.0, from);
    for (at, met) in run.iter().enumerate().take(to).skip(from + 1) {
        let ap: [f64; 3] = std::array::from_fn(|i| met.point[i] - a[i]);
        let t = if length2 > 0.0 {
            ((0..3).map(|i| ap[i] * ab[i]).sum::<f64>() / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d: f64 = (0..3)
            .map(|i| (ap[i] - t * ab[i]).powi(2))
            .sum::<f64>()
            .sqrt();
        if d > far.0 {
            far = (d, at);
        }
    }
    if far.0 > tolerance {
        keep[far.1] = true;
        douglas_peucker(run, from, far.1, tolerance, keep);
        douglas_peucker(run, far.1, to, tolerance, keep);
    }
}

impl Session {
    pub(super) fn paint_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "paint_open" => self.paint_open(args),
            "paint_close" => {
                *lock(&self.paint.painter) = None;
                Ok(json!({}))
            }
            other => Err(format!("there is no call named {other}")),
        }
    }

    pub(super) fn paint_bytes(&self, name: &str, args: &Value) -> Result<Vec<u8>, String> {
        match name {
            "paint_mesh" => self.paint_mesh(),
            "paint_event" => self.paint_event(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// The topology and tree of a mesh, made once while it is held.
    fn paint_tools(&self, mesh: &Arc<Mesh>) -> (Arc<Topology>, Arc<Tree>) {
        let key = Arc::as_ptr(mesh) as usize;
        let mut meshes = lock(&self.paint.meshes);
        if let Some((held, topology, tree)) = meshes.get(&key)
            && held.upgrade().is_some_and(|held| Arc::ptr_eq(&held, mesh))
        {
            return (topology.clone(), tree.clone());
        }
        meshes.retain(|_, (held, _, _)| held.strong_count() > 0);
        let made = (Topology::of(mesh), Arc::new(Tree::new(mesh)));
        meshes.insert(key, (Arc::downgrade(mesh), made.0.clone(), made.1.clone()));
        made
    }

    /// Selectors for the printed parts of a shape, loaded with its paint.
    fn painted_parts(&self, shape: &Shape, kind: PaintKind, max_state: u8) -> Vec<Painted> {
        shape
            .parts()
            .iter()
            .zip(shape.part_ids())
            .filter(|(part, _)| part.kind == PartKind::Normal)
            .map(|(part, id)| {
                let (topology, tree) = self.paint_tools(&part.mesh);
                let mut selector = Selector::new(&part.mesh, topology);
                let codes = part.paint_codes(kind);
                selector.load_all(
                    codes.iter().map(|(at, code)| (*at, code.as_str())),
                    max_state,
                );
                selector.take_changed();
                Painted {
                    id: *id,
                    transform: Affine::from_transform(&part.transform),
                    mirrored: part.transform.is_mirrored(),
                    filament: part.filament,
                    mesh: part.mesh.clone(),
                    tree,
                    selector,
                }
            })
            .collect()
    }

    fn paint_open(&self, args: &Value) -> Result<Value, String> {
        let started = Instant::now();
        let stage = self.stage_of(args)?;
        let kind = kind_of(needed(args, "kind")?)?;
        let copy = whole(args, "copy")?;
        let file = stage
            .file
            .as_ref()
            .ok_or("the project is empty: it has no objects")?;
        let object = file
            .model
            .objects()
            .iter()
            .find(|object| object.copies().iter().any(|held| held.id() == copy))
            .ok_or_else(|| format!("there is no copy {copy}"))?;
        // The states each tool knows (the `max_ebt` each gizmo's
        // `update_from_model_object` reads with): supports also keep the
        // stabilizer points (3), colours as many filaments as there are.
        let filaments = args
            .get("filaments")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .clamp(1, 127) as u8;
        let max_state = match kind {
            PaintKind::Supports => 3,
            PaintKind::Seam => 2,
            PaintKind::FuzzySkin => 1,
            PaintKind::Color => filaments,
        };
        let parts = self.painted_parts(object.shape(), kind, max_state);
        if parts.is_empty() {
            return Err("the object has no printed part to paint".into());
        }
        let listed: Vec<Value> = parts
            .iter()
            .map(|part| json!({ "id": part.id, "triangles": part.selector.triangle_count(), "filament": part.filament }))
            .collect();
        let painter = Painter {
            project: stage.id,
            object: object.id(),
            kind,
            max_state,
            shape: object.shape().clone(),
            parts,
            fill_part: None,
            gap: None,
        };
        *lock(&self.paint.painter) = Some(painter);
        Ok(json!({
            "project": stage.id,
            "object": object.id(),
            "parts": listed,
            "max_state": max_state,
            "ms": started.elapsed().as_secs_f64() * 1e3,
        }))
    }

    fn paint_mesh(&self) -> Result<Vec<u8>, String> {
        let held = lock(&self.paint.painter);
        let painter = held.as_ref().ok_or("no painting tool is open")?;
        let count: usize = painter
            .parts
            .iter()
            .map(|part| part.mesh.triangles.len())
            .sum();
        let mut out = Vec::with_capacity(8 + count * 48);
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&(painter.parts.len() as u32).to_le_bytes());
        for part in &painter.parts {
            let mark = u8::try_from(part.filament.min(250)).unwrap_or(1);
            for at in 0..part.mesh.triangles.len() {
                push_triangle(&mut out, part.placed(part.mesh.triangle(at)), mark);
            }
        }
        Ok(out)
    }

    /// One thing the pointer or the panel did, and what came of it, in
    /// bytes: a `u32` length and that many bytes of JSON (padded to four),
    /// then blocks of triangles to draw over the object: for each a `u32`
    /// part, a `u32` block (of 1024 of the part's triangles) and a `u32`
    /// count, then that many triangles of three 16-byte vertices
    /// (`renderer.js` `mesh`), the mark the state (or [`MARKED`] more for
    /// what a fill has marked). A block sent replaces what was sent for it.
    ///
    /// Takes `project`, `matrix` (the copy's, object to world), `eye`
    /// (the camera's place in the world), `clip` (null, or a `normal` and
    /// an `offset`: what has `normal · p > offset` is cut away), and `op`:
    ///
    /// - `hover`, `ray` ([origin, direction] in the world): where it meets
    ///   the object; with `tool` `fill` or `bucket`, what a fill from there
    ///   would take is marked (`SLAGizmoEventType::Moving`).
    /// - `paint`, `rays` (the pointer's way since the last, the newest
    ///   first), `state`, `tool` (`circle`, `sphere`, `fill`, `bucket`,
    ///   `triangle`, `height`), `radius` or `height` (mm), `angle` (smart
    ///   fill's, degrees), `highlight` (degrees; 0 for none): paints
    ///   (`LeftDown`, `RightDown`, `Dragging`).
    /// - `commit`, `name`: puts the paint into the model as one step of the
    ///   undo history named `name` (`LeftUp`, `RightUp`); with `all`,
    ///   every block is sent again.
    /// - `erase_all`, `name`: "Erase all painting" (the step "Reset
    ///   selection", as every gizmo names it).
    /// - `gap`, `area` (null to stop): marks what "Gap fill" would fill;
    ///   `gap_fill`, `area`, `name`: "Perform" (the colour gizmo's step is
    ///   "Gap fill", the support gizmo's "Reset selection").
    /// - `unmark`: takes away what a fill marked.
    /// - `refresh`: every block again, after the model changed (an undo).
    pub(super) fn paint_event(&self, args: &Value) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let op = needed(args, "op")?;
        let stage = self.stage_of(args)?;
        let mut held = lock(&self.paint.painter);
        let painter = held.as_mut().ok_or("no painting tool is open")?;
        if painter.project != stage.id {
            return Err("the painting tool is open on another project".into());
        }
        // The object as the model has it: after an undo or a redo the paint
        // is read from it again (`GLGizmoPainterBase::on_load`,
        // `update_from_model_object`).
        let file = stage
            .file
            .as_ref()
            .ok_or("the project is empty: it has no objects")?;
        let object = file
            .model
            .objects()
            .iter()
            .find(|object| object.id() == painter.object)
            .ok_or("the object painted is gone")?;
        let mut all = op == "refresh" || args.get("all").and_then(Value::as_bool) == Some(true);
        if !Arc::ptr_eq(object.shape(), &painter.shape) {
            painter.parts = self.painted_parts(object.shape(), painter.kind, painter.max_state);
            painter.shape = object.shape().clone();
            painter.fill_part = None;
            all = true;
        }
        let matrix = affine_of(args.get("matrix"), "matrix")?;
        let worlds = painter.worlds(&matrix);
        let eye = numbers::<3>(args.get("eye"), "eye").unwrap_or([0.0, 0.0, 1e4]);
        let clip = match args.get("clip") {
            None | Some(Value::Null) => None,
            Some(clip) => Some(Clip {
                normal: to32(numbers::<3>(clip.get("normal"), "clip normal")?),
                offset: clip
                    .get("offset")
                    .and_then(Value::as_f64)
                    .ok_or("clip offset was not given")? as f32,
            }),
        };
        let ray = |value: &Value| -> Result<([f64; 3], [f64; 3]), String> {
            let r = numbers::<6>(Some(value), "a ray")?;
            Ok(([r[0], r[1], r[2]], [r[3], r[4], r[5]]))
        };
        let number =
            |key: &str, default: f64| args.get(key).and_then(Value::as_f64).unwrap_or(default);
        let tool = args.get("tool").and_then(Value::as_str).unwrap_or("circle");
        let angle = number("angle", 30.0) as f32;
        let highlight = number("highlight", 0.0) as f32;
        let mut answer = json!({});
        let mut hit_json = Value::Null;
        let mut gaps_changed = false;
        let mut committed: Option<String> = None;

        // The world's place of a hit, its normal, and the part.
        let hit_value = |painter: &Painter, met: &Met| -> Value {
            let world = &worlds[met.part];
            let part = &painter.parts[met.part];
            let [a, b, c] = part
                .mesh
                .triangle(met.facet)
                .map(|p| world.apply64(p.map(f64::from)));
            let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let mut n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-30);
            // A map that turns the part inside out turns its faces in.
            let m = &world.m;
            let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            let length = if det < 0.0 { -length } else { length };
            n = n.map(|c| c / length);
            json!({ "part": met.part, "facet": met.facet, "point": world.apply64(met.point), "normal": n })
        };

        match op {
            "hover" => {
                let (origin, direction) = ray(args.get("ray").ok_or("ray was not given")?)?;
                let met = painter.cast(&worlds, origin, direction, clip);
                if let Some(met) = &met {
                    hit_json = hit_value(painter, met);
                }
                if matches!(tool, "fill" | "bucket" | "triangle") {
                    match met {
                        None => {
                            for part in &mut painter.parts {
                                part.selector.unmark_fill();
                            }
                            painter.fill_part = None;
                        }
                        Some(met) => {
                            if painter.fill_part != Some(met.part) {
                                for part in &mut painter.parts {
                                    part.selector.unmark_fill();
                                }
                            }
                            let world = worlds[met.part];
                            let part = &mut painter.parts[met.part];
                            let point = to32(met.point);
                            let clip = clip_in(clip, &world);
                            // Marked anew, though the pointer is on what is
                            // marked, after the angle changed (Ctrl and the
                            // wheel: `force_reselection`).
                            let force = args.get("force").and_then(Value::as_bool) == Some(true);
                            match tool {
                                "fill" => part.selector.seed_fill(
                                    point, met.facet, &world, clip, angle, highlight, force,
                                ),
                                "bucket" => part
                                    .selector
                                    .bucket_fill(point, met.facet, clip, angle, true, force),
                                _ => part
                                    .selector
                                    .bucket_fill(point, met.facet, clip, -1.0, false, force),
                            }
                            painter.fill_part = Some(met.part);
                        }
                    }
                }
                if tool == "height"
                    && let Some(met) = &met
                {
                    // The band's two outlines on the object, at the bottom
                    // and the top of the band (`render_cursor_height_range`).
                    let z = worlds[met.part].apply64(met.point)[2];
                    let height = number("height", 0.2);
                    answer["band"] = json!(painter.band(&worlds, z, height));
                }
            }
            "paint" => {
                let state = u8::try_from(whole(args, "state")?).map_err(|_| "state is too high")?;
                if state > painter.max_state {
                    return Err(format!("state {state} is not one this tool paints"));
                }
                let rays = args
                    .get("rays")
                    .and_then(Value::as_array)
                    .filter(|rays| !rays.is_empty())
                    .ok_or("rays was not given")?;
                let radius = number("radius", 1.0) as f32;
                let splitting = args.get("split").and_then(Value::as_bool).unwrap_or(true);
                let mut hits = Vec::new();
                for value in rays {
                    let (origin, direction) = ray(value)?;
                    match painter.cast(&worlds, origin, direction, clip) {
                        Some(met) => hits.push(met),
                        // Only the last may miss (the reference stops there).
                        None => break,
                    }
                }
                if let Some(first) = hits.first() {
                    hit_json = hit_value(painter, first);
                }
                match tool {
                    "fill" | "bucket" | "triangle" => {
                        for met in &hits {
                            let world = worlds[met.part];
                            let part = &mut painter.parts[met.part];
                            let point = to32(met.point);
                            let clip = clip_in(clip, &world);
                            part.selector.apply_fill(state);
                            match tool {
                                "fill" => part.selector.seed_fill(
                                    point, met.facet, &world, clip, angle, highlight, true,
                                ),
                                "bucket" => part
                                    .selector
                                    .bucket_fill(point, met.facet, clip, angle, true, true),
                                _ => part
                                    .selector
                                    .bucket_fill(point, met.facet, clip, -1.0, false, true),
                            }
                            painter.fill_part = None;
                        }
                    }
                    "height" => {
                        // The band from where the pointer is, on every part
                        // it reaches (`get_projected_height_range`).
                        if let Some(met) = hits.first() {
                            let z = worlds[met.part].apply64(met.point)[2] as f32;
                            let height = number("height", 0.2) as f32;
                            for (index, part) in painter.parts.iter_mut().enumerate() {
                                let world = worlds[index];
                                let source = world
                                    .inverse()
                                    .map_or([0.0; 3], |back| to32(back.apply64(eye)));
                                let cursor = Cursor::new(
                                    Brush::HeightRange { z, height },
                                    [0.0; 3],
                                    [0.0; 3],
                                    source,
                                    1.0,
                                    world,
                                    clip_in(clip, &world),
                                );
                                part.selector.select_patch(
                                    0, cursor, state, &world, splitting, highlight, false,
                                );
                            }
                        }
                    }
                    _ => {
                        let brush = tool == "sphere";
                        for run in runs(&hits, f64::from(radius) / 10.0) {
                            let index = run[0].part;
                            let world = worlds[index];
                            let part = &mut painter.parts[index];
                            let source = world
                                .inverse()
                                .map_or([0.0; 3], |back| to32(back.apply64(eye)));
                            let clip = clip_in(clip, &world);
                            if run.len() == 1 {
                                let shape = if brush { Brush::Sphere } else { Brush::Circle };
                                let centre = to32(run[0].point);
                                let cursor =
                                    Cursor::new(shape, centre, centre, source, radius, world, clip);
                                part.selector.select_patch(
                                    run[0].facet,
                                    cursor,
                                    state,
                                    &world,
                                    splitting,
                                    highlight,
                                    false,
                                );
                            } else {
                                let shape = if brush {
                                    Brush::Capsule3D
                                } else {
                                    Brush::Capsule2D
                                };
                                for pair in run.windows(2) {
                                    let (a, b) = (to32(pair[0].point), to32(pair[1].point));
                                    if a == b {
                                        continue;
                                    }
                                    let cursor =
                                        Cursor::new(shape, a, b, source, radius, world, clip);
                                    part.selector.select_patch(
                                        pair[0].facet,
                                        cursor,
                                        state,
                                        &world,
                                        splitting,
                                        highlight,
                                        false,
                                    );
                                }
                            }
                        }
                    }
                }
                if painter.gap.is_some() {
                    gaps_changed = true;
                }
            }
            "unmark" => {
                for part in &mut painter.parts {
                    part.selector.unmark_fill();
                }
                painter.fill_part = None;
            }
            "erase_all" => {
                for part in &mut painter.parts {
                    part.selector.reset();
                }
                committed = Some(
                    args.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Reset selection")
                        .to_owned(),
                );
            }
            "gap" => {
                painter.gap = args
                    .get("area")
                    .and_then(Value::as_f64)
                    .map(|area| area as f32);
                all = true;
            }
            "gap_fill" => {
                let area = number("area", 0.0) as f32;
                for part in &mut painter.parts {
                    part.selector.fill_gaps(area);
                }
                gaps_changed = painter.gap.is_some();
                committed = Some(
                    args.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Gap fill")
                        .to_owned(),
                );
            }
            "commit" => {
                let name = needed(args, "name")?;
                // "Triangle" dragged: the triangle the pointer was let go
                // on is painted too. The reference paints what was marked
                // only at the next position, so the last triangle of a drag
                // is left marked and unpainted (GLGizmoPainterBase.cpp
                // `gizmo_event`, `LeftUp` takes the snapshot without
                // `seed_fill_apply_on_triangles`).
                if tool == "triangle"
                    && let Some(state) = args.get("state").and_then(Value::as_u64)
                    && let Ok(state) = u8::try_from(state)
                    && state <= painter.max_state
                {
                    for part in &mut painter.parts {
                        part.selector.apply_fill(state);
                    }
                }
                committed = Some(name.to_owned());
            }
            "refresh" => {}
            other => return Err(format!("there is no painting event named {other}")),
        }

        if let Some(name) = committed {
            let codes: Vec<(u64, Vec<(u32, String)>)> = painter
                .parts
                .iter()
                .map(|part| (part.id, part.selector.serialize()))
                .collect();
            let mut model = file.model.clone();
            let bed = stage.bed();
            let changed = model
                .set_paint(bed.as_ref(), painter.object, painter.kind, &codes)
                .map_err(|e| e.to_string())?;
            if changed {
                let next = Arc::new(File::of(model));
                let mut state = lock(&self.state);
                let at = state
                    .index_of(stage.id)
                    .ok_or_else(|| format!("there is no project {}", stage.id))?;
                let project = &mut state.projects[at];
                let still = project
                    .file
                    .as_ref()
                    .is_some_and(|now| Arc::ptr_eq(now, file));
                if !still {
                    return Err("the project changed meanwhile; nothing was done".into());
                }
                let before = project.file.replace(next.clone());
                push_step(project, &name, before, None);
                drop(state);
                let object = next
                    .model
                    .objects()
                    .iter()
                    .find(|object| object.id() == painter.object)
                    .ok_or("the object painted is gone")?;
                painter.shape = object.shape().clone();
                answer["done"] = json!(name);
                answer["edited"] = self.edited(stage.id, json!({ "done": name }))?;
            } else {
                answer["done"] = Value::Null;
            }
        }

        // What goes back: every block, or those whose paint changed.
        let gaps: HashMap<usize, u8> = match painter.gap {
            Some(area) if all || gaps_changed => painter
                .parts
                .iter()
                .enumerate()
                .flat_map(|(index, part)| {
                    part.selector
                        .gaps(area)
                        .into_iter()
                        .map(move |(piece, state)| ((index << 40) | piece, state))
                })
                .collect(),
            _ => HashMap::new(),
        };
        if gaps_changed {
            all = true;
        }
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for (index, part) in painter.parts.iter_mut().enumerate() {
            let changed = part.selector.take_changed();
            if all {
                blocks.extend((0..part.chunks()).map(|chunk| (index, chunk)));
            } else {
                let set: BTreeSet<usize> = changed.iter().map(|&at| at as usize / CHUNK).collect();
                blocks.extend(set.into_iter().map(|chunk| (index, chunk)));
            }
        }
        answer["hit"] = hit_json;
        answer["all"] = json!(all);
        answer["states"] = json!(
            painter
                .parts
                .iter()
                .flat_map(|part| part.selector.states())
                .collect::<BTreeSet<u8>>()
        );
        answer["marked"] = json!(painter.parts.iter().any(|part| part.selector.has_fill()));
        let mut body = Vec::new();
        for (index, chunk) in &blocks {
            let part = &painter.parts[*index];
            let own: HashMap<usize, u8> = gaps
                .iter()
                .filter(|(key, _)| *key >> 40 == *index)
                .map(|(key, state)| (key & ((1 << 40) - 1), *state))
                .collect();
            let start = body.len();
            body.extend_from_slice(&(*index as u32).to_le_bytes());
            body.extend_from_slice(&(*chunk as u32).to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            let zero = if painter.kind == PaintKind::Color {
                u8::try_from(part.filament.min(127)).unwrap_or(1)
            } else {
                0
            };
            let count = part.chunk_bytes(*chunk, &own, zero, &mut body);
            body[start + 8..start + 12].copy_from_slice(&count.to_le_bytes());
        }
        // What the engine says when it slices paint it does not follow
        // (`edge_engine::prepare`'s note for each kind in the model's
        // `unread_paint`, in its words), for the kind painted here.
        let model_now = lock(&self.state)
            .projects
            .iter()
            .find(|project| project.id == stage.id)
            .and_then(|project| project.file.clone());
        let unread = match painter.kind {
            PaintKind::FuzzySkin => Some("fuzzy skin"),
            _ => None,
        };
        answer["note"] = match (unread, model_now) {
            (Some(kind), Some(file)) if file.model.unread_paint().contains(&kind) => {
                json!(format!(
                    "the project has {kind} painted on it, which is not followed: the settings alone decide"
                ))
            }
            _ => Value::Null,
        };
        answer["blocks"] = json!(blocks.len());
        answer["ms"] = json!(started.elapsed().as_secs_f64() * 1e3);
        let text = answer.to_string().into_bytes();
        let mut out = Vec::with_capacity(4 + text.len() + 3 + body.len());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(&text);
        while out.len() % 4 != 0 {
            out.push(b' ');
        }
        out.extend_from_slice(&body);
        Ok(out)
    }
}

impl Painter {
    /// Where the planes `z` and `z + height` of the world cut the parts, as
    /// pairs of points in the world.
    fn band(&self, worlds: &[Affine], z: f64, height: f64) -> Vec<[[f64; 3]; 2]> {
        let mut out = Vec::new();
        for (part, world) in self.parts.iter().zip(worlds) {
            for plane in [z, z + height] {
                for at in 0..part.mesh.triangles.len() {
                    let p = part
                        .mesh
                        .triangle(at)
                        .map(|q| world.apply64(q.map(f64::from)));
                    let mut ends = Vec::with_capacity(2);
                    for k in 0..3 {
                        let (a, b) = (p[k], p[(k + 1) % 3]);
                        if (a[2] - plane) * (b[2] - plane) < 0.0 {
                            let t = (plane - a[2]) / (b[2] - a[2]);
                            ends.push(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])));
                        }
                    }
                    if ends.len() == 2 {
                        out.push([ends[0], ends[1]]);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
