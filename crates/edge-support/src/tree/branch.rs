//! Branches drawn as the reference's organic trees are
//! (TreeSupport3D.cpp `organic_draw_branches`): the branch centres are
//! smoothed and pushed clear of the part as balls
//! (`organic_smooth_branches_avoid_collisions`), each stretch of branch
//! between two forks is built as a tube of circles strung along it with a
//! round cap at either end (`extrude_branch`), and the tubes are sliced
//! at the middle of every layer. A branch standing on the part is
//! carried down to it where it rests on nothing, and leaves its foot as
//! a contact for the dense layers above.

use std::f64::consts::PI;

use edge_geom::{Coord, ExPolygon, Point2, unscaled};
use edge_mesh::Mesh;
use rayon::prelude::*;

use super::config::{EPSILON, TreeConfig};
use super::element::Element;
use super::geometry::{
    Polys, area, diff_safe, extent, intersection, remove_small, tiny_area, union,
};
use super::volumes::Volumes;
use crate::{Job, SupportError};

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalized(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    if n > 0.0 { mul(a, 1.0 / n) } else { a }
}

/// Heights of the layers, in mm: the top of each and the bottom of the
/// first (`layer_z`).
struct Heights {
    first: f64,
    layer: f64,
}

impl Heights {
    fn top(&self, layer: usize) -> f64 {
        self.first + layer as f64 * self.layer
    }
    fn bottom(&self, layer: usize) -> f64 {
        if layer == 0 { 0.0 } else { self.top(layer - 1) }
    }
    /// `layer_idx_ceil` and `layer_idx_floor`.
    fn ceil(&self, z: f64) -> usize {
        ((z - self.first) / self.layer).ceil().max(0.0) as usize
    }
    fn floor(&self, z: f64) -> usize {
        ((z - self.first) / self.layer).floor().max(0.0) as usize
    }
}

/// The outline of the part on one layer as segments, binned on a grid so
/// that the nearest segment to a point within some distance is found
/// quickly (the reference keeps a tree of them per layer).
struct Lines {
    segments: Vec<([f64; 2], [f64; 2])>,
    cell: f64,
    origin: [f64; 2],
    columns: usize,
    rows: usize,
    bins: Vec<Vec<u32>>,
}

impl Lines {
    fn new(regions: &[ExPolygon]) -> Self {
        let mut segments = Vec::new();
        for ring in regions.iter().flat_map(ExPolygon::rings) {
            let points = &ring.points;
            for index in 0..points.len() {
                let (a, b) = (points[index], points[(index + 1) % points.len()]);
                segments.push((
                    [unscaled(a.x), unscaled(a.y)],
                    [unscaled(b.x), unscaled(b.y)],
                ));
            }
        }
        let cell = 1.0;
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (a, b) in &segments {
            for p in [a, b] {
                (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
            }
        }
        if segments.is_empty() {
            return Self {
                segments,
                cell,
                origin: [0.0, 0.0],
                columns: 0,
                rows: 0,
                bins: Vec::new(),
            };
        }
        let columns = ((x1 - x0) / cell) as usize + 1;
        let rows = ((y1 - y0) / cell) as usize + 1;
        let mut bins = vec![Vec::new(); columns * rows];
        for (index, (a, b)) in segments.iter().enumerate() {
            let cx0 = ((a[0].min(b[0]) - x0) / cell) as usize;
            let cx1 = ((a[0].max(b[0]) - x0) / cell) as usize;
            let cy0 = ((a[1].min(b[1]) - y0) / cell) as usize;
            let cy1 = ((a[1].max(b[1]) - y0) / cell) as usize;
            for cy in cy0..=cy1.min(rows - 1) {
                for cx in cx0..=cx1.min(columns - 1) {
                    bins[cy * columns + cx].push(index as u32);
                }
            }
        }
        Self {
            segments,
            cell,
            origin: [x0, y0],
            columns,
            rows,
            bins,
        }
    }

    /// The segments that may come within `reach` of `p`, each once.
    fn candidates(&self, p: [f64; 2], reach: f64) -> Vec<u32> {
        if self.segments.is_empty() {
            return Vec::new();
        }
        let lo_x = ((p[0] - reach - self.origin[0]) / self.cell).floor();
        let hi_x = ((p[0] + reach - self.origin[0]) / self.cell).floor();
        let lo_y = ((p[1] - reach - self.origin[1]) / self.cell).floor();
        let hi_y = ((p[1] + reach - self.origin[1]) / self.cell).floor();
        if hi_x < 0.0 || hi_y < 0.0 || lo_x >= self.columns as f64 || lo_y >= self.rows as f64 {
            return Vec::new();
        }
        let (cx0, cx1) = (
            lo_x.max(0.0) as usize,
            (hi_x as usize).min(self.columns - 1),
        );
        let (cy0, cy1) = (lo_y.max(0.0) as usize, (hi_y as usize).min(self.rows - 1));
        let mut out = Vec::new();
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                out.extend_from_slice(&self.bins[cy * self.columns + cx]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The nearest point to `p` of some of the segments, closer than
    /// `sqrt(limit2)`, with its squared distance.
    fn nearest_among(&self, ids: &[u32], p: [f64; 2], limit2: f64) -> Option<(f64, [f64; 2])> {
        let mut best: Option<(f64, [f64; 2])> = None;
        for &index in ids {
            let (a, b) = self.segments[index as usize];
            let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
            let l2 = vx * vx + vy * vy;
            let t = if l2 > 0.0 {
                (((p[0] - a[0]) * vx + (p[1] - a[1]) * vy) / l2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let q = [a[0] + vx * t, a[1] + vy * t];
            let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
            // Lowest index on ties, as ids are in order.
            if d2 < limit2 && best.is_none_or(|(least, _)| d2 < least) {
                best = Some((d2, q));
            }
        }
        best
    }
}

/// A branch centre as a ball, for smoothing.
struct Sphere {
    layer: usize,
    index: usize,
    below: Option<usize>,
    parents: Vec<usize>,
    locked: bool,
    radius: f64,
    radius_scaled: Coord,
    position: [f64; 3],
    previous: [f64; 3],
    layer_begin: usize,
    layer_end: usize,
}

/// `organic_smooth_branches_avoid_collisions`: each unlocked centre is
/// pushed out of the part where its ball reaches into it, and drawn
/// towards the average of its neighbours above and below, a little at a
/// time, until nothing reaches into the part.
fn smooth(volumes: &Volumes, config: &TreeConfig, heights: &Heights, layers: &mut [Vec<Element>]) {
    // Every element in one list, layer by layer, with the one below it.
    let mut offsets = vec![0usize];
    for elements in layers.iter() {
        offsets.push(offsets.last().unwrap() + elements.len());
    }
    let mut below_of: Vec<Vec<Option<usize>>> =
        layers.iter().map(|l| vec![None; l.len()]).collect();
    for layer in 0..layers.len().saturating_sub(1) {
        for (index, element) in layers[layer].iter().enumerate() {
            for &parent in &element.parents {
                below_of[layer + 1][parent as usize] = Some(index);
            }
        }
    }
    let mut spheres: Vec<Sphere> = Vec::with_capacity(*offsets.last().unwrap());
    let mut min_z: Vec<f64> = Vec::new();
    for (layer, elements) in layers.iter().enumerate() {
        for (index, element) in elements.iter().enumerate() {
            let at = element.state.result.unwrap_or_default();
            let below = below_of[layer][index];
            let position = [unscaled(at.x), unscaled(at.y), heights.top(layer)];
            let radius_scaled = element.state.radius(config);
            spheres.push(Sphere {
                layer,
                index,
                below: below.map(|b| offsets[layer - 1] + b),
                parents: element
                    .parents
                    .iter()
                    .map(|p| offsets[layer + 1] + *p as usize)
                    .collect(),
                locked: element.parents.is_empty() || (below.is_none() && layer > 0),
                radius: unscaled(radius_scaled),
                radius_scaled,
                position,
                previous: position,
                layer_begin: 0,
                layer_end: 0,
            });
            min_z.push(match below {
                Some(b) => min_z[offsets[layer - 1] + b],
                None => position[2],
            });
        }
    }
    let mut max_z = vec![f64::MAX; spheres.len()];
    for id in (0..spheres.len()).rev() {
        if spheres[id].parents.is_empty() {
            max_z[id] = spheres[id].position[2];
        } else {
            for &parent in &spheres[id].parents {
                max_z[id] = max_z[id].min(max_z[parent]);
            }
        }
    }
    let count = layers.len();
    let mut collision_layers: Vec<bool> = vec![false; count];
    for (id, sphere) in spheres.iter_mut().enumerate() {
        let lo = min_z[id].max(sphere.position[2] - sphere.radius);
        let hi = max_z[id].min(sphere.position[2] + sphere.radius);
        sphere.layer_begin = sphere.layer.min(heights.ceil(lo));
        sphere.layer_end = count.min(sphere.layer.max(heights.floor(hi)) + 1);
        collision_layers[sphere.layer] = true;
    }
    let lines: Vec<Option<Lines>> = (0..count)
        .into_par_iter()
        .map(|layer| collision_layers[layer].then(|| Lines::new(volumes.collision(0, layer, true))))
        .collect();
    let slow = unscaled(config.max_move_slow);
    let layer_height = heights.layer;
    const COLLISION_EXTRA_GAP: f64 = 0.1;
    const MAX_NUDGE_COLLISION_AVOIDANCE: f64 = 0.5;
    const MAX_NUDGE_SMOOTHING: f64 = 0.2;
    // Segments near each ball, and where it was when they were gathered.
    #[derive(Default)]
    struct Cache {
        anchor: Option<[f64; 2]>,
        layers: Vec<Vec<u32>>,
    }
    const CACHE_MARGIN: f64 = 1.0;
    let mut caches: Vec<Cache> = spheres.iter().map(|_| Cache::default()).collect();
    // Moves smaller than this count as none.
    const STILL: f64 = 1e-9;
    let mut active = vec![true; spheres.len()];
    for _ in 0..100 {
        for sphere in &mut spheres {
            sphere.previous = sphere.position;
        }
        let snapshot: Vec<([f64; 3], Coord)> = spheres
            .iter()
            .map(|s| (s.previous, s.radius_scaled))
            .collect();
        let moved: Vec<(Option<[f64; 2]>, bool)> = spheres
            .par_iter()
            .zip(caches.par_iter_mut())
            .zip(active.par_iter())
            .map(|((sphere, cache), active)| {
                if sphere.locked || !active {
                    return (None, false);
                }
                // Keeps a step within reach of the neighbours above and
                // below as they were (Orca's `limit_candidate_to_linked_layers`).
                let limit = |mut candidate: [f64; 2], current: [f64; 2]| {
                    let mut constrain = |anchor: [f64; 2], allowed: f64| {
                        let delta = [candidate[0] - anchor[0], candidate[1] - anchor[1]];
                        let dist = delta[0].hypot(delta[1]);
                        let now = (current[0] - anchor[0]).hypot(current[1] - anchor[1]);
                        let allowed = allowed.max(now);
                        if dist > allowed && dist > EPSILON {
                            candidate = [
                                anchor[0] + delta[0] * allowed / dist,
                                anchor[1] + delta[1] * allowed / dist,
                            ];
                        }
                    };
                    if let Some(below) = sphere.below {
                        let (p, r) = snapshot[below];
                        constrain(
                            [p[0], p[1]],
                            unscaled((r - sphere.radius_scaled).max(0)) + slow,
                        );
                    }
                    for &parent in &sphere.parents {
                        let (p, r) = snapshot[parent];
                        constrain(
                            [p[0], p[1]],
                            unscaled((sphere.radius_scaled - r).max(0)) + slow,
                        );
                    }
                    candidate
                };
                let mut position = [sphere.position[0], sphere.position[1]];
                let mut depth = f64::MIN;
                let mut hit = [0.0, 0.0];
                // The segments near the ball, gathered again once it has
                // moved far enough that one it now reaches might be missing.
                if cache
                    .anchor
                    .is_none_or(|a| (a[0] - position[0]).hypot(a[1] - position[1]) > CACHE_MARGIN)
                {
                    cache.anchor = Some(position);
                    cache.layers = (sphere.layer_begin..sphere.layer_end)
                        .map(|layer| match lines.get(layer) {
                            Some(Some(lines)) => {
                                lines.candidates(position, sphere.radius + CACHE_MARGIN)
                            }
                            _ => Vec::new(),
                        })
                        .collect();
                }
                for layer in sphere.layer_begin..sphere.layer_end {
                    let dz = (layer as f64 - sphere.layer as f64) * layer_height;
                    let r2 = sphere.radius * sphere.radius - dz * dz;
                    if r2 <= 0.0 {
                        continue;
                    }
                    let Some(Some(lines)) = lines.get(layer) else {
                        continue;
                    };
                    let ids = &cache.layers[layer - sphere.layer_begin];
                    if ids.is_empty() {
                        continue;
                    }
                    if let Some((d2, q)) = lines.nearest_among(ids, position, r2) {
                        let collision_depth = r2.sqrt() - d2.sqrt();
                        if collision_depth > depth {
                            depth = collision_depth;
                            hit = q;
                        }
                    }
                }
                let mut colliding = false;
                if depth > 0.0 {
                    colliding = depth > EPSILON;
                    let nudge =
                        (depth + COLLISION_EXTRA_GAP).clamp(0.0, MAX_NUDGE_COLLISION_AVOIDANCE);
                    let away = [position[0] - hit[0], position[1] - hit[1]];
                    let length = away[0].hypot(away[1]);
                    if length > 0.0 {
                        // The reference multiplies by the nudge twice here
                        // (`nudge_vector * nudge_dist` of a vector already
                        // that long), moving a ball the square of the
                        // distance meant; it is moved the distance.
                        let candidate = [
                            position[0] + away[0] / length * nudge,
                            position[1] + away[1] / length * nudge,
                        ];
                        position = limit(candidate, position);
                    }
                }
                // Drawn towards its neighbours.
                let (mut avg, mut weight) = ([0.0, 0.0], 0.0);
                for &parent in &sphere.parents {
                    let w = sphere.radius;
                    avg[0] += w * snapshot[parent].0[0];
                    avg[1] += w * snapshot[parent].0[1];
                    weight += w;
                }
                if let Some(below) = sphere.below {
                    let w = weight;
                    avg[0] += w * snapshot[below].0[0];
                    avg[1] += w * snapshot[below].0[1];
                    weight += w;
                }
                if weight > 0.0 {
                    let target = [
                        0.5 * position[0] + 0.5 * avg[0] / weight,
                        0.5 * position[1] + 0.5 * avg[1] / weight,
                    ];
                    let shift = [target[0] - position[0], target[1] - position[1]];
                    let length = shift[0].hypot(shift[1]);
                    let nudge = length.min(MAX_NUDGE_SMOOTHING);
                    if nudge > 0.0 {
                        let candidate = [
                            position[0] + shift[0] * nudge / length,
                            position[1] + shift[1] * nudge / length,
                        ];
                        position = limit(candidate, position);
                    }
                }
                (Some(position), colliding)
            })
            .collect();
        let mut any = false;
        let mut stirred = vec![false; spheres.len()];
        for (id, (sphere, (position, colliding))) in spheres.iter_mut().zip(moved).enumerate() {
            if let Some(p) = position {
                let step = (p[0] - sphere.position[0]).hypot(p[1] - sphere.position[1]);
                stirred[id] = step > STILL;
                sphere.position[0] = p[0];
                sphere.position[1] = p[1];
            }
            stirred[id] |= colliding;
            any |= colliding;
        }
        // A ball goes on being worked out while it or a neighbour still
        // moves; one whose neighbourhood has come to rest would only be
        // set where it is again. The reference works every ball out every
        // round, to the same positions.
        for (id, sphere) in spheres.iter().enumerate() {
            active[id] = stirred[id]
                || sphere.below.is_some_and(|below| stirred[below])
                || sphere.parents.iter().any(|parent| stirred[*parent]);
        }
        if !any {
            break;
        }
    }
    for sphere in &spheres {
        layers[sphere.layer][sphere.index].state.result =
            Some(Point2::from_mm(sphere.position[0], sphere.position[1]));
    }
}

/// A stretch of tree from a fork to the next, or to a tip.
struct Branch {
    path: Vec<(usize, usize)>,
    has_root: bool,
    has_tip: bool,
}

/// The branches of every tree (`TreeVisitor::visit_recursive`).
fn trees(layers: &[Vec<Element>]) -> Vec<Vec<Branch>> {
    let mut marked: Vec<Vec<bool>> = layers.iter().map(|l| vec![false; l.len()]).collect();
    let mut trees = Vec::new();
    for layer in 0..layers.len().saturating_sub(1) {
        for index in 0..layers[layer].len() {
            if marked[layer][index] || layers[layer][index].parents.is_empty() {
                continue;
            }
            let mut branches: Vec<Branch> = Vec::new();
            let mut stack = vec![(layer, index)];
            while let Some((l, i)) = stack.pop() {
                marked[l][i] = true;
                let root = branches.is_empty();
                let mut forks = Vec::new();
                for &parent in &layers[l][i].parents {
                    let mut path = vec![(l, i), (l + 1, parent as usize)];
                    let (mut pl, mut pi) = (l + 1, parent as usize);
                    let mut next_fork = None;
                    loop {
                        let element = &layers[pl][pi];
                        if element.parents.len() > 1 {
                            next_fork = Some((pl, pi));
                            break;
                        }
                        marked[pl][pi] = true;
                        if element.parents.is_empty() {
                            break;
                        }
                        pi = element.parents[0] as usize;
                        pl += 1;
                        path.push((pl, pi));
                    }
                    branches.push(Branch {
                        path,
                        has_root: root,
                        has_tip: next_fork.is_none(),
                    });
                    if let Some(fork) = next_fork {
                        forks.push(fork);
                    }
                }
                // Depth first, the forks in order.
                stack.extend(forks.into_iter().rev());
            }
            trees.push(branches);
        }
    }
    trees
}

/// Circle of `radius` round `centre` in the plane square to `normal`,
/// within `eps` of round (`discretize_circle`).
fn discretize_circle(
    centre: V3,
    normal: V3,
    radius: f64,
    eps: f64,
    out: &mut Vec<[f32; 3]>,
) -> (u32, u32) {
    let step = 2.0 * (1.0 - eps / radius).clamp(-1.0, 1.0).acos();
    let steps = ((2.0 * PI / step.max(1e-3)).ceil() as usize).max(3);
    let step = 2.0 * PI / steps as f64;
    let x = normalized(cross(normal, [0.0, -1.0, 0.0]));
    let y = normalized(cross(normal, x));
    let (x, y) = (mul(x, radius), mul(y, radius));
    let begin = out.len() as u32;
    for i in 0..steps {
        let angle = i as f64 * step;
        let p = add(centre, add(mul(x, angle.cos()), mul(y, angle.sin())));
        out.push([p[0] as f32, p[1] as f32, p[2] as f32]);
    }
    (begin, out.len() as u32)
}

fn fan(mesh: &mut Mesh, centre: u32, (begin, end): (u32, u32), flip: bool) {
    let mut u = end - 1;
    for v in begin..end {
        mesh.triangles
            .push(if flip { [centre, u, v] } else { [centre, v, u] });
        u = v;
    }
}

/// `triangulate_strip`: two rings joined zig-zag, the shorter diagonal
/// first.
fn strip(mesh: &mut Mesh, (b1, e1): (u32, u32), (b2, e2): (u32, u32)) {
    let vertex = |i: u32, m: &Mesh| {
        let p = m.vertices[i as usize];
        [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
    };
    let (mut n1, mut n2) = (e1 - b1, e2 - b2);
    let p1 = vertex(b1, mesh);
    let mut start2 = b2;
    let mut least = f64::MAX;
    for i in b2..e2 {
        let d = sub(vertex(i, mesh), p1);
        let d2 = dot(d, d);
        if d2 < least {
            (least, start2) = (d2, i);
        }
    }
    let (mut u, mut v) = (b1, start2);
    while n1 > 0 || n2 > 0 {
        let next = |x: u32, b: u32, e: u32| if x + 1 == e { b } else { x + 1 };
        let take_first = if n1 == 0 {
            false
        } else if n2 == 0 {
            true
        } else {
            let (u2, v2) = (next(u, b1, e1), next(v, b2, e2));
            let l1 = sub(vertex(u2, mesh), vertex(v, mesh));
            let l2 = sub(vertex(v2, mesh), vertex(u, mesh));
            dot(l1, l1) < dot(l2, l2)
        };
        if take_first {
            let u2 = next(u, b1, e1);
            mesh.triangles.push([u, u2, v]);
            n1 -= 1;
            u = u2;
        } else {
            let v2 = next(v, b2, e2);
            mesh.triangles.push([u, v2, v]);
            n2 -= 1;
            v = v2;
        }
    }
}

/// A branch as a closed tube (`extrude_branch`): a half ball at its lower
/// end (a flat foot on the bed), a circle square to the branch at every
/// centre, and a half ball at its upper end. Returns the mesh and the
/// heights it spans.
fn extrude_branch(
    path: &[(usize, usize)],
    layers: &[Vec<Element>],
    config: &TreeConfig,
    heights: &Heights,
    has_root: bool,
) -> (Mesh, f64, f64) {
    const EPS: f64 = 0.015;
    let mut mesh = Mesh {
        vertices: Vec::new(),
        triangles: Vec::new(),
    };
    let point = |(l, i): (usize, usize)| {
        let at = layers[l][i].state.result.unwrap_or_default();
        [unscaled(at.x), unscaled(at.y), heights.top(l)]
    };
    let radius = |(l, i): (usize, usize)| unscaled(layers[l][i].state.radius(config));
    let mut previous_strip = (0, 0);
    let (mut zmin, mut zmax) = (0.0, 0.0);
    for ipath in 1..path.len() {
        let (prev, current) = (path[ipath - 1], path[ipath]);
        let (p1, p2) = (point(prev), point(current));
        let v1 = normalized(sub(p2, p1));
        if ipath == 1 {
            let r = radius(prev);
            if has_root && prev.0 == 0 {
                // A trunk on the bed gets a flat foot: a rounded one would
                // reach below the bed (Orca).
                let normal = [0.0, 0.0, 1.0];
                let bottom = [p1[0], p1[1], 0.0];
                let centre = mesh.vertices.len() as u32;
                mesh.vertices
                    .push([bottom[0] as f32, bottom[1] as f32, 0.0]);
                let ring = discretize_circle(bottom, normal, r, EPS, &mut mesh.vertices);
                fan(&mut mesh, centre, ring, false);
                previous_strip = discretize_circle(p1, normal, r, EPS, &mut mesh.vertices);
                strip(&mut mesh, ring, previous_strip);
                zmin = 0.0;
            } else {
                let step = 2.0 * (1.0 - EPS / r).clamp(-1.0, 1.0).acos();
                let steps = ((PI / (2.0 * step.max(1e-3))).ceil() as usize).max(1);
                let step = PI / (2.0 * steps as f64);
                let centre = mesh.vertices.len() as u32;
                let tip = sub(p1, mul(v1, r));
                mesh.vertices
                    .push([tip[0] as f32, tip[1] as f32, tip[2] as f32]);
                zmin = tip[2];
                let mut angle = step;
                for i in 1..steps {
                    let ring = discretize_circle(
                        sub(p1, mul(v1, r * angle.cos())),
                        v1,
                        r * angle.sin(),
                        EPS,
                        &mut mesh.vertices,
                    );
                    if i == 1 {
                        fan(&mut mesh, centre, ring, false);
                    } else {
                        strip(&mut mesh, previous_strip, ring);
                    }
                    previous_strip = ring;
                    angle += step;
                }
                if steps == 1 {
                    // Too small a ball for rings between its pole and its
                    // middle: a cone to the first ring.
                    previous_strip = (centre, centre);
                }
            }
        }
        if ipath + 1 == path.len() {
            // The upper end.
            let r = radius(current);
            let step = 2.0 * (1.0 - EPS / r).clamp(-1.0, 1.0).acos();
            let steps = ((PI / (2.0 * step.max(1e-3))).ceil() as usize).max(1);
            let step = PI / (2.0 * steps as f64);
            let mut angle = PI / 2.0;
            for _ in 0..steps {
                let ring = discretize_circle(
                    add(p2, mul(v1, r * angle.cos())),
                    v1,
                    r * angle.sin(),
                    EPS,
                    &mut mesh.vertices,
                );
                connect(&mut mesh, previous_strip, ring);
                previous_strip = ring;
                angle -= step;
            }
            let centre = mesh.vertices.len() as u32;
            let top = add(p2, mul(v1, r));
            mesh.vertices
                .push([top[0] as f32, top[1] as f32, top[2] as f32]);
            zmax = top[2];
            fan(&mut mesh, centre, previous_strip, true);
        } else {
            let p3 = point(path[ipath + 1]);
            let v2 = normalized(sub(p3, p2));
            let normal = normalized(add(v1, v2));
            let ring = discretize_circle(p2, normal, radius(current), EPS, &mut mesh.vertices);
            connect(&mut mesh, previous_strip, ring);
            previous_strip = ring;
        }
    }
    (mesh, zmin, zmax)
}

/// A strip, or a fan when the first "ring" is a single pole.
fn connect(mesh: &mut Mesh, from: (u32, u32), to: (u32, u32)) {
    if from.0 == from.1 {
        fan(mesh, from.0, to, false);
    } else {
        strip(mesh, from, to);
    }
}

/// The support's body on each layer, and the places it stands on the part.
pub(crate) struct Drawn {
    pub base: Vec<Polys>,
    pub bottom_contacts: Vec<Polys>,
}

/// Cuts a branch's closed tube at each plane (`slice_mesh`): every
/// triangle the plane crosses gives a segment from the edge it enters by
/// to the edge it leaves by, solid on the left, and segments chain into
/// loops by the edges they share. A lean version of `edge_slice::slice`
/// for these small meshes, which have no flat faces at a plane.
fn slice_tube(mesh: &Mesh, planes: &[f64]) -> Vec<Vec<edge_geom::Polygon>> {
    let vertex = |i: u32| {
        let p = mesh.vertices[i as usize];
        [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
    };
    let crossing = |a: u32, b: u32, z: f64| {
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        let (pl, ph) = (vertex(low), vertex(high));
        let t = (z - pl[2]) / (ph[2] - pl[2]);
        (
            u64::from(low) << 32 | u64::from(high),
            Point2::from_mm(pl[0] + t * (ph[0] - pl[0]), pl[1] + t * (ph[1] - pl[1])),
        )
    };
    // Triangles by the planes they cross.
    let mut by_plane: Vec<Vec<u32>> = vec![Vec::new(); planes.len()];
    for (index, ids) in mesh.triangles.iter().enumerate() {
        let zs = ids.map(|v| vertex(v)[2]);
        let (lo, hi) = (zs[0].min(zs[1]).min(zs[2]), zs[0].max(zs[1]).max(zs[2]));
        let first = planes.partition_point(|&p| p <= lo);
        let end = planes.partition_point(|&p| p <= hi);
        for plane in &mut by_plane[first..end] {
            plane.push(index as u32);
        }
    }
    let mut out = Vec::with_capacity(planes.len());
    let mut next: rustc_hash::FxHashMap<u64, usize> = Default::default();
    for (plane, triangles) in by_plane.iter().enumerate() {
        let z = planes[plane];
        next.clear();
        // From edge, to edge, point at the from edge.
        let mut segments: Vec<(u64, u64, Point2)> = Vec::with_capacity(triangles.len());
        for &triangle in triangles {
            let ids = mesh.triangles[triangle as usize];
            let above = ids.map(|v| vertex(v)[2] >= z);
            let lone = if above[0] == above[1] {
                2
            } else if above[0] == above[2] {
                1
            } else {
                0
            };
            let (after, before) = ((lone + 1) % 3, (lone + 2) % 3);
            let leaving = crossing(ids[lone], ids[after], z);
            let entering = crossing(ids[before], ids[lone], z);
            let ((from, point), (to, _)) = if above[lone] {
                (leaving, entering)
            } else {
                (entering, leaving)
            };
            next.insert(from, segments.len());
            segments.push((from, to, point));
        }
        let mut used = vec![false; segments.len()];
        let mut polygons = Vec::new();
        for start in 0..segments.len() {
            if used[start] {
                continue;
            }
            let mut points = Vec::new();
            let mut at = start;
            loop {
                used[at] = true;
                let (_, to, point) = segments[at];
                points.push(point);
                match next.get(&to) {
                    Some(&following) if !used[following] => at = following,
                    _ => break,
                }
            }
            if points.len() >= 3 {
                polygons.push(edge_geom::Polygon { points });
            }
        }
        out.push(polygons);
    }
    out
}

/// Whether a box lies wholly inside a convex outline (and so needs no
/// clipping to it); `false` for an outline that is not convex.
fn inside_convex(outline: &ExPolygon, [x0, y0, x1, y1]: [Coord; 4]) -> bool {
    let points = &outline.contour.points;
    if points.len() < 3 || !outline.holes.is_empty() {
        return false;
    }
    let cross = |a: Point2, b: Point2, p: Point2| {
        i128::from(b.x - a.x) * i128::from(p.y - a.y)
            - i128::from(b.y - a.y) * i128::from(p.x - a.x)
    };
    let n = points.len();
    let sign = (0..n)
        .map(|i| cross(points[i], points[(i + 1) % n], points[(i + 2) % n]).signum())
        .find(|s| *s != 0)
        .unwrap_or(0);
    if sign == 0 {
        return false;
    }
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| Point2 { x, y });
    (0..n).all(|i| {
        let (a, b) = (points[i], points[(i + 1) % n]);
        cross(a, b, points[(i + 2) % n]).signum() * sign >= 0
            && corners.iter().all(|c| cross(a, b, *c).signum() * sign > 0)
    })
}

/// Draws every branch (`organic_draw_branches`).
pub(crate) fn draw(
    job: &Job<'_>,
    volumes: &Volumes,
    config: &TreeConfig,
    layers: &mut [Vec<Element>],
) -> Result<Drawn, SupportError> {
    let heights = Heights {
        first: job.first_layer_height,
        layer: job.layer_height,
    };
    smooth(volumes, config, &heights, layers);
    let layers: &[Vec<Element>] = layers;
    let trees = trees(layers);
    let tiny = tiny_area();
    let bed = job.bed.as_ref();
    // Each layer's collision as islands with their extents, so that a
    // branch's slice is cut only by what is near it.
    let top_layer = trees
        .iter()
        .flatten()
        .map(|branch| branch.path[branch.path.len() - 1].0 + 1)
        .max()
        .unwrap_or(0)
        + (config.branch_radius / config.layer_height.max(1)) as usize * 4
        + 8;
    let near_index: Vec<Vec<([Coord; 4], &ExPolygon)>> = (0..top_layer)
        .into_par_iter()
        .map(|l| {
            volumes
                .collision(0, l, true)
                .iter()
                .filter_map(|region| extent(std::slice::from_ref(region)).map(|e| (e, region)))
                .collect()
        })
        .collect();
    let cut_clear = |polys: Polys, l: usize| -> Result<Polys, SupportError> {
        let Some([x0, y0, x1, y1]) = extent(&polys) else {
            return Ok(polys);
        };
        let m = 4 * super::geometry::SAFETY;
        let near: Polys = match near_index.get(l) {
            Some(index) => index
                .iter()
                .filter(|([a, b, c, d], _)| {
                    *a <= x1 + m && *c >= x0 - m && *b <= y1 + m && *d >= y0 - m
                })
                .map(|(_, region)| (*region).clone())
                .collect(),
            None => volumes.collision(0, l, true).to_vec(),
        };
        let mut polys = diff_safe(&polys, &near)?;
        if let Some(bed) = bed
            && !inside_convex(bed, [x0, y0, x1, y1])
        {
            polys = intersection(&polys, std::slice::from_ref(bed))?;
        }
        Ok(polys)
    };
    // Every branch on its own, in parallel; the reference goes tree by
    // tree, which leaves a few large trees to a few threads.
    let all: Vec<&Branch> = trees.iter().flatten().collect();
    let drawn_branches: Vec<Option<(usize, Vec<Polys>, Polys)>> = all
        .par_iter()
        .map(
            |branch| -> Result<Option<(usize, Vec<Polys>, Polys)>, SupportError> {
                {
                    let (mesh, zmin, zmax) =
                        extrude_branch(&branch.path, layers, config, &heights, branch.has_root);
                    let (front, back) = (branch.path[0].0, branch.path[branch.path.len() - 1].0);
                    let mut layer_begin = if branch.has_root {
                        front
                    } else {
                        front.min(heights.ceil(zmin))
                    };
                    let layer_end = if branch.has_tip {
                        back
                    } else {
                        back.max(heights.floor(zmax))
                    } + 1;
                    let planes: Vec<f64> = (layer_begin..layer_end)
                        .map(|l| 0.5 * (heights.bottom(l) + heights.top(l)))
                        .collect();
                    let cut = slice_tube(&mesh, &planes);
                    let mut slices: Vec<Polys> = Vec::with_capacity(cut.len());
                    for (i, layer) in cut.iter().enumerate() {
                        let polys = edge_clip::union_positive(layer)?;
                        let mut polys = cut_clear(polys, layer_begin + i)?;
                        remove_small(&mut polys, tiny);
                        slices.push(polys);
                    }
                    let empty = slices.iter().take_while(|s| s.is_empty()).count();
                    if empty >= slices.len() {
                        return Ok(None);
                    }
                    slices.drain(..empty);
                    layer_begin += empty;
                    let front_contact = slices[0].clone();
                    let mut bottom_contacts: Vec<Polys> = Vec::new();
                    if branch.has_root {
                        let root = &layers[branch.path[0].0][branch.path[0].1];
                        if root.state.to_model_gracious {
                            if config.floor_layers > 0 {
                                let mut contacts = if config.rests_on_model
                                    && config.z_distance_bottom_layers > 0
                                    && layer_begin > 0
                                {
                                    front_contact.clone()
                                } else {
                                    intersection(&front_contact, volumes.placeable(0, layer_begin))?
                                };
                                remove_small(&mut contacts, tiny);
                                if contacts.is_empty() && config.rests_on_model && layer_begin > 0 {
                                    contacts = front_contact.clone();
                                }
                                if !contacts.is_empty() {
                                    bottom_contacts.push(contacts);
                                }
                            }
                        } else if layer_begin > 0 {
                            // Resting on the part where there is no flat top:
                            // carried on down until it meets the part, or less
                            // of it is left than is worth it.
                            let bottom_radius = root.state.radius(config);
                            let propagate = (5 * bottom_radius / config.layer_height) as usize;
                            let bottommost = if root.state.verylost {
                                0
                            } else {
                                layer_begin.saturating_sub(propagate)
                            };
                            let min_area = PI * (config.branch_radius as f64).powi(2);
                            let stop =
                                (0.2 * PI * (bottom_radius as f64).powi(2)).max(0.5 * min_area);
                            let mut extra: Vec<Polys> = Vec::new();
                            let mut rest: Polys = Vec::new();
                            let mut l = layer_begin;
                            while l > bottommost {
                                l -= 1;
                                // Each layer is cut by its own layer's
                                // collision, which takes in the part on the
                                // `z_distance_bottom_layers` under it, so no
                                // layer comes nearer the part than the bottom
                                // gap; and the foot this leaves is printed
                                // (`finish` in mod.rs), so none is further
                                // either. The reference (TreeSupport3D.cpp
                                // `organic_draw_branches`, l.4178) cuts the
                                // first layer down by the collision of the
                                // layer above, so it may sit right on the
                                // part, and does not print an organic foot
                                // (SupportCommon.cpp `generate_interface_layers`:
                                // "projection seeds"): the two together keep
                                // the gap where there are dense bottom layers,
                                // and fuse the foot to the part where there
                                // are none. Cutting by the layer's own
                                // collision there, without printing the foot,
                                // doubles the gap (EdgeSlicer PR #984, item 2,
                                // reverted for it). Measured 2026-11-12 on the
                                // U1's 3DBenchy with tree supports: the
                                // reference's feet 25.2 mm of line at 0.2 mm
                                // over the part with two dense bottom layers,
                                // 15.5 mm at 0 (fused) with none; here 25.3
                                // and 30.7 mm, all at 0.2 mm.
                                let collision = volumes.collision(0, l, false);
                                rest = diff_safe(
                                    if rest.is_empty() {
                                        &front_contact
                                    } else {
                                        &rest
                                    },
                                    collision,
                                )?;
                                remove_small(&mut rest, tiny);
                                if area(&rest) < stop {
                                    break;
                                }
                                extra.push(rest.clone());
                            }
                            if config.floor_layers > 0 {
                                let source = extra.last().unwrap_or(&front_contact);
                                let mut contacts = if config.rests_on_model
                                    && config.z_distance_bottom_layers > 0
                                {
                                    match (extra.last(), bed) {
                                        (Some(last), Some(bed)) => {
                                            intersection(last, std::slice::from_ref(bed))?
                                        }
                                        _ => source.clone(),
                                    }
                                } else {
                                    intersection(source, volumes.placeable(0, layer_begin))?
                                };
                                remove_small(&mut contacts, tiny);
                                if !contacts.is_empty() {
                                    bottom_contacts.push(contacts);
                                }
                                if bottom_contacts.is_empty() && config.rests_on_model {
                                    bottom_contacts.push(front_contact.clone());
                                }
                            }
                            layer_begin -= extra.len();
                            let mut moved: Vec<Polys> = extra.into_iter().rev().collect();
                            moved.extend(slices);
                            slices = moved;
                        }
                        if config.rests_on_model
                            && branch.path[0].0 > 0
                            && config.floor_layers > 0
                            && config.z_distance_bottom_layers > 0
                            && bottom_contacts.is_empty()
                            && !front_contact.is_empty()
                        {
                            bottom_contacts.push(front_contact.clone());
                        }
                    }
                    while slices.last().is_some_and(Vec::is_empty) {
                        slices.pop();
                    }
                    if slices.is_empty() {
                        return Ok(None);
                    }
                    let foot = bottom_contacts.into_iter().next().unwrap_or_default();
                    Ok(Some((layer_begin, slices, foot)))
                }
            },
        )
        .collect::<Result<_, _>>()?;
    let count = drawn_branches
        .iter()
        .flatten()
        .map(|(first, slices, _)| first + slices.len())
        .max()
        .unwrap_or(0);
    let mut base: Vec<Polys> = vec![Vec::new(); count];
    let mut bottom_contacts: Vec<Polys> = vec![Vec::new(); count];
    for (first, slices, foot) in drawn_branches.into_iter().flatten() {
        bottom_contacts[first].extend(foot);
        for (j, polys) in slices.into_iter().enumerate() {
            base[first + j].extend(polys);
        }
    }
    let base = base
        .into_par_iter()
        .map(|polys| union(&polys))
        .collect::<Result<Vec<_>, _>>()?;
    let bottom_contacts = bottom_contacts
        .into_par_iter()
        .map(|polys| union(&polys))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Drawn {
        base,
        bottom_contacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tube_slices_into_its_circles() {
        let config_heights = Heights {
            first: 0.2,
            layer: 0.2,
        };
        // A straight upright tube of radius 1 mm from layer 5 to layer 15.
        let mut mesh = Mesh {
            vertices: Vec::new(),
            triangles: Vec::new(),
        };
        let a = discretize_circle(
            [0.0, 0.0, 1.2],
            [0.0, 0.0, 1.0],
            1.0,
            0.015,
            &mut mesh.vertices,
        );
        let b = discretize_circle(
            [0.0, 0.0, 3.2],
            [0.0, 0.0, 1.0],
            1.0,
            0.015,
            &mut mesh.vertices,
        );
        let c0 = mesh.vertices.len() as u32;
        mesh.vertices.push([0.0, 0.0, 0.2]);
        fan(&mut mesh, c0, a, false);
        strip(&mut mesh, a, b);
        let c1 = mesh.vertices.len() as u32;
        mesh.vertices.push([0.0, 0.0, 4.2]);
        fan(&mut mesh, c1, b, true);
        let planes: Vec<f64> = (8..12)
            .map(|l| 0.5 * (config_heights.bottom(l) + config_heights.top(l)))
            .collect();
        for (layer, theirs) in slice_tube(&mesh, &planes)
            .iter()
            .zip(edge_slice::slice(&mesh, &planes))
        {
            let polys = edge_clip::union_positive(layer).unwrap();
            // The same as the general slicer's.
            let general = edge_clip::union_positive(&theirs.polygons).unwrap();
            let area_general: f64 = general.iter().map(ExPolygon::area_mm2).sum();
            // A full circle, wound the right way.
            let area_mm2: f64 = polys.iter().map(ExPolygon::area_mm2).sum();
            // Within 0.015 mm of round, so a little short of pi.
            assert!((area_mm2 - PI).abs() < 0.1, "{area_mm2}");
            assert!(
                (area_mm2 - area_general).abs() < 1e-4,
                "{area_mm2} {area_general}"
            );
        }
    }
}
