//! Roughening one wall: new points along it, each moved by the noise.
//!
//! A port of `fuzzy_polyline`, `fuzzy_extrusion_line`, the ripple and the
//! overhang blend of the reference's Feature/FuzzySkin/FuzzySkin.cpp.

use std::f64::consts::PI;

use edge_arachne::Junction;
use edge_geom::{Coord, Point2, Polygon};

use crate::config::{FuzzyConfig, FuzzyMode, NoiseType};
use crate::noise::{MAX_OCTAVES, Module};
use crate::rng::CallRng;

/// libslic3r.h, `SCALING_FACTOR`: mm per unit. The reference unscales by
/// multiplying with it and scales by dividing by it (`unscale_`, `scale_`);
/// `edge_geom::unscaled` divides by a million instead, which is not the
/// same number in the last bit, and the noise is sampled at these values.
const SCALING_FACTOR: f64 = 0.000001;
/// libslic3r.h, `EPSILON`.
const EPSILON: f64 = 1e-4;

/// libslic3r.h, `unscale_`.
fn unscale(units: f64) -> f64 {
    units * SCALING_FACTOR
}

/// libslic3r.h, `scale_`, and Point.hpp's `scaled<double>`.
fn scale(mm: f64) -> f64 {
    mm / SCALING_FACTOR
}

/// A `Vec2d` of the reference, with the few Eigen operations it uses on
/// one, each done the way Eigen does it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vec2 {
    x: f64,
    y: f64,
}

impl Vec2 {
    const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    /// `(b - a).cast<double>()`.
    fn between(a: Point2, b: Point2) -> Self {
        Self {
            x: (b.x - a.x) as f64,
            y: (b.y - a.y) as f64,
        }
    }

    fn squared_norm(self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    fn norm(self) -> f64 {
        self.squared_norm().sqrt()
    }

    /// Point.hpp, `perp`: a quarter turn counter-clockwise.
    fn perp(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }

    /// Eigen's `normalized()`: divided by its length, or itself when that
    /// is zero.
    fn normalized(self) -> Self {
        let z = self.squared_norm();
        if z > 0.0 { self.over(z.sqrt()) } else { self }
    }

    fn times(self, factor: f64) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
        }
    }

    fn over(self, divisor: f64) -> Self {
        Self {
            x: self.x / divisor,
            y: self.y / divisor,
        }
    }
}

/// `p + v.cast<coord_t>()`: the vector is cut to whole units towards
/// zero, as the C++ cast does. A vector that is not a number (C++ leaves
/// the cast undefined) moves nothing or saturates instead.
fn moved(p: Point2, v: Vec2) -> Point2 {
    Point2 {
        x: p.x.saturating_add(v.x as Coord),
        y: p.y.saturating_add(v.y as Coord),
    }
}

/// clipper.cpp, `ClipperLib::PointInPolygon` (Hormann and Agathos): 0
/// outside, 1 inside, -1 on the boundary.
fn point_in_polygon(pt: Point2, path: &[Point2]) -> i32 {
    let mut result = 0;
    let cnt = path.len();
    if cnt < 3 {
        return 0;
    }
    let cross = |ip: Point2, ip_next: Point2| -> i128 {
        i128::from(ip.x - pt.x) * i128::from(ip_next.y - pt.y)
            - i128::from(ip_next.x - pt.x) * i128::from(ip.y - pt.y)
    };
    let mut ip = path[0];
    for i in 1..=cnt {
        let ip_next = if i == cnt { path[0] } else { path[i] };
        if ip_next.y == pt.y
            && (ip_next.x == pt.x || (ip.y == pt.y && ((ip_next.x > pt.x) == (ip.x < pt.x))))
        {
            return -1;
        }
        if (ip.y < pt.y) != (ip_next.y < pt.y) {
            if ip.x >= pt.x {
                if ip_next.x > pt.x {
                    result = 1 - result;
                } else {
                    let d = cross(ip, ip_next);
                    if d == 0 {
                        return -1;
                    }
                    if (d > 0) == (ip_next.y > ip.y) {
                        result = 1 - result;
                    }
                }
            } else if ip_next.x > pt.x {
                let d = cross(ip, ip_next);
                if d == 0 {
                    return -1;
                }
                if (d > 0) == (ip_next.y > ip.y) {
                    result = 1 - result;
                }
            }
        }
        ip = ip_next;
    }
    result
}

/// Polygon.cpp, `contains(const Polygons&, const Point&)`: inside an odd
/// number of the polygons, or on the boundary of one.
pub(crate) fn contains(polygons: &[Polygon], p: Point2) -> bool {
    let mut poly_count_inside = 0;
    for poly in polygons {
        let is_inside_this_poly = point_in_polygon(p, &poly.points);
        if is_inside_this_poly == -1 {
            return true;
        }
        poly_count_inside += is_inside_this_poly;
    }
    poly_count_inside % 2 == 1
}

/// What a point's displacement is read from.
enum Noise {
    /// The reference's `UniformNoise`: a fresh random number in [-1, 1)
    /// for every point, from the call's own generator.
    Uniform,
    Coherent(Module),
}

impl Noise {
    /// FuzzySkin.cpp, `get_noise_module`.
    ///
    /// libnoise throws when asked for more than 30 octaves, or fewer than
    /// one for Perlin and Billow, which the settings' limits (1 to 10)
    /// keep the reference from ever doing. Here the count is held to
    /// what libnoise takes; [`FuzzyConfig::validate`] reports it.
    fn of(cfg: &FuzzyConfig) -> Self {
        let frequency = 1.0 / cfg.scale;
        match cfg.noise {
            NoiseType::Perlin => Noise::Coherent(Module::Perlin {
                frequency,
                octaves: cfg.octaves.clamp(1, MAX_OCTAVES),
                persistence: cfg.persistence,
            }),
            NoiseType::Billow => Noise::Coherent(Module::Billow {
                frequency,
                octaves: cfg.octaves.clamp(1, MAX_OCTAVES),
                persistence: cfg.persistence,
            }),
            NoiseType::RidgedMulti => Noise::Coherent(Module::RidgedMulti {
                frequency,
                octaves: cfg.octaves.min(MAX_OCTAVES),
            }),
            NoiseType::Voronoi => Noise::Coherent(Module::Voronoi { frequency }),
            // The ripple never gets here; like the reference, anything
            // else is the classic noise.
            NoiseType::Classic | NoiseType::Ripple => Noise::Uniform,
        }
    }

    /// `noise->GetValue(x, y, z)`.
    fn value(&self, rng: &mut CallRng, x: f64, y: f64, z: f64) -> f64 {
        match self {
            Noise::Uniform => rng.value() * 2.0 - 1.0,
            Noise::Coherent(module) => module.value(x, y, z),
        }
    }
}

/// FuzzySkin.cpp, `kBlendSamples`: over how many points the roughness
/// comes back after a stretch over air.
const BLEND_SAMPLES: i32 = 2;

/// FuzzySkin.cpp, `support_blend_factors`: turns "is this point carried by
/// the layer below" into how much of its displacement each point keeps: 0
/// over air, 1 well away from it, and a ramp of `BLEND_SAMPLES` points in
/// between. `closed` says the points go round (the ends are neighbours).
pub(crate) fn support_blend_factors(supported: &[bool], closed: bool) -> Vec<f64> {
    let n = supported.len() as i32;
    let mut factors = vec![1.0; supported.len()];
    if n == 0 {
        return factors;
    }

    // Distance, in points, to the nearest unsupported one.
    let unreached = n + BLEND_SAMPLES + 1;
    let mut to_edge: Vec<i32> = supported
        .iter()
        .map(|&carried| if carried { unreached } else { 0 })
        .collect();

    // A closed loop needs the sweeps to wrap; one extra lap of the ramp's
    // width on each side is enough to settle it.
    let extra = if closed { n.min(BLEND_SAMPLES + 1) } else { 0 };
    for k in 0..n + extra {
        let i = k % n;
        if !closed && i == 0 {
            continue;
        }
        let prev = (i - 1 + n) % n;
        to_edge[i as usize] = to_edge[i as usize].min(to_edge[prev as usize] + 1);
    }
    for k in (0..n + extra).rev() {
        let i = k % n;
        if !closed && i == n - 1 {
            continue;
        }
        let next = (i + 1) % n;
        to_edge[i as usize] = to_edge[i as usize].min(to_edge[next as usize] + 1);
    }

    for (factor, (&carried, &distance)) in factors.iter_mut().zip(supported.iter().zip(&to_edge)) {
        *factor = if carried {
            f64::min(1.0, f64::from(distance) / f64::from(BLEND_SAMPLES + 1))
        } else {
            0.0
        };
    }
    factors
}

/// FuzzySkin.cpp, `skip_overhangs_active`: the region to test points
/// against, when the overhang skip is live for this call at all.
fn active_support<'a>(cfg: &FuzzyConfig, support: Option<&'a [Polygon]>) -> Option<&'a [Polygon]> {
    support.filter(|polygons| cfg.skip_overhangs && !polygons.is_empty())
}

/// FuzzySkin.cpp, `ripple_phase_shift_rad`: how far round the wave has
/// stepped by this layer.
fn ripple_phase_shift_rad(cfg: &FuzzyConfig) -> f64 {
    if cfg.ripple_offset == 0.0 || cfg.layers_between_ripple_offset <= 0 {
        return 0.0;
    }

    let effective_layer = cfg.layer_id.max(0);
    let period_index = effective_layer / cfg.layers_between_ripple_offset.max(1);
    let raw_shift = f64::from(period_index) * (cfg.ripple_offset / 100.0) * (2.0 * PI);
    raw_shift % (2.0 * PI)
}

/// FuzzySkin.cpp, `ripple_anchor_arc_mm`: how far along the outline, in
/// mm, the point lies that the wave is pinned to. That is the leftmost
/// place the outline crosses y = 0, or failing one the vertex nearest to
/// that line. The outline is always walked as closed, as the reference
/// walks it.
fn ripple_anchor_arc_mm(poly: &[Point2]) -> f64 {
    let np = poly.len();
    let mm = |p: Point2| Vec2 {
        x: unscale(p.x as f64),
        y: unscale(p.y as f64),
    };

    let mut anchor_world = Vec2 {
        x: f64::MAX,
        y: f64::MAX,
    };
    let mut found_crossing = false;
    for i in 0..np {
        let a = mm(poly[i]);
        let b = mm(poly[(i + 1) % np]);
        if (a.y <= 0.0 && b.y >= 0.0) || (a.y >= 0.0 && b.y <= 0.0) {
            let t = if (b.y - a.y).abs() < 1e-9 {
                0.0
            } else {
                a.y / (a.y - b.y)
            };
            let x_cross = a.x + t.clamp(0.0, 1.0) * (b.x - a.x);
            if !found_crossing || x_cross < anchor_world.x {
                anchor_world = Vec2 { x: x_cross, y: 0.0 };
                found_crossing = true;
            }
        }
    }
    if !found_crossing {
        let mut best_abs_y = f64::MAX;
        for &p in poly {
            let p = mm(p);
            let ay = p.y.abs();
            if ay < best_abs_y {
                best_abs_y = ay;
                anchor_world = p;
            }
        }
    }

    // The arc length of the point of the outline closest to the anchor.
    let mut anchor_arc_mm = 0.0;
    let mut best_dist_sq = f64::MAX;
    let mut accum_mm = 0.0;
    for i in 0..np {
        let pa_mm = mm(poly[i]);
        let pb_mm = mm(poly[(i + 1) % np]);
        let seg = Vec2 {
            x: pb_mm.x - pa_mm.x,
            y: pb_mm.y - pa_mm.y,
        };
        let seg_len = seg.norm();
        if seg_len > 1e-9 {
            let to_anchor = Vec2 {
                x: anchor_world.x - pa_mm.x,
                y: anchor_world.y - pa_mm.y,
            };
            let dot = to_anchor.x * seg.x + to_anchor.y * seg.y;
            let t = (dot / (seg_len * seg_len)).clamp(0.0, 1.0);
            let nearest = Vec2 {
                x: pa_mm.x + seg.x * t - anchor_world.x,
                y: pa_mm.y + seg.y * t - anchor_world.y,
            };
            let dist_sq = nearest.squared_norm();
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                anchor_arc_mm = accum_mm + t * seg_len;
            }
        }
        accum_mm += seg_len;
    }
    anchor_arc_mm
}

/// One point the ripple puts on a wall.
struct RippleSample {
    /// Where it lies on the wall as it was.
    base: Point2,
    /// Its full displacement, in units.
    offset: Vec2,
    /// The index of the point its segment runs to.
    segment_end: usize,
}

/// The sampling loop `fuzzy_polyline_ripple` and
/// `fuzzy_extrusion_line_ripple` (FuzzySkin.cpp) share word for word: a
/// point every `point_distance` along the first `nseg` segments of
/// `poly`, each with the sine's displacement across its segment. `None`
/// where the reference returns before sampling (no ripples asked for, a
/// step or a wall of no length).
///
/// The reference's comments promise a peak at the anchor; its code adds a
/// whole turn there (`M_PI * 2.0`), not a quarter, so the wave passes
/// through zero. The code is what is ported.
fn ripple_samples(
    poly: &[Point2],
    closed: bool,
    nseg: usize,
    cfg: &FuzzyConfig,
) -> Option<Vec<RippleSample>> {
    let amplitude = unscale(cfg.thickness as f64);
    let n = f64::from(cfg.ripples_per_layer);
    let fill_step_mm = unscale(cfg.point_distance as f64);

    if n <= 0.0 || fill_step_mm < 1e-6 {
        return None;
    }

    let np = poly.len();
    let mut perimeter_mm = 0.0;
    for i in 0..nseg {
        perimeter_mm += unscale(Vec2::between(poly[i], poly[(i + 1) % np]).norm());
    }

    if perimeter_mm < 1e-6 {
        return None;
    }

    let anchor_arc_mm = ripple_anchor_arc_mm(poly);
    let phase_shift_rad = ripple_phase_shift_rad(cfg);

    let phase_at_anchor = PI * 2.0 + phase_shift_rad;
    let arc_phase = |arc_mm: f64| -> f64 {
        n * (2.0 * PI) * (arc_mm - anchor_arc_mm) / perimeter_mm + phase_at_anchor
    };

    let mut out = Vec::new();
    let mut accum_mm = 0.0;
    for i in 0..nseg {
        let p0 = poly[i];
        let segment_end = (i + 1) % np;
        let seg = Vec2::between(p0, poly[segment_end]);
        let seg_len = seg.norm();
        if seg_len < EPSILON {
            continue;
        }

        let seg_len_mm = unscale(seg_len);
        let seg_unit = seg.over(seg_len);
        let seg_perp = seg_unit.perp();
        let seg_end_mm = accum_mm + seg_len_mm;
        // An open run's first point is one step in, not on its start.
        let first_s = f64::max(
            (accum_mm / fill_step_mm).ceil() * fill_step_mm,
            if closed { 0.0 } else { fill_step_mm },
        );

        let mut s = first_s;
        while s < seg_end_mm {
            let t = (s - accum_mm) / seg_len_mm;
            let disp = arc_phase(s).sin() * amplitude;
            out.push(RippleSample {
                base: moved(p0, seg.times(t)),
                offset: seg_perp.times(scale(disp)),
                segment_end,
            });
            s += fill_step_mm;
        }

        accum_mm = seg_end_mm;
    }
    Some(out)
}

/// Where the ripple's points end up: each at its base plus its
/// displacement, the displacement scaled by `support_blend_factors` when
/// the overhang skip is live (FuzzySkin.cpp, `ripple_blend_supported`).
/// Without the skip the factor is one, which changes no number.
fn ripple_positions(
    samples: &[RippleSample],
    support: Option<&[Polygon]>,
    closed: bool,
) -> Vec<Point2> {
    let factors = match support {
        Some(support) => {
            let supported: Vec<bool> = samples
                .iter()
                .map(|sample| contains(support, sample.base))
                .collect();
            support_blend_factors(&supported, closed)
        }
        None => vec![1.0; samples.len()],
    };
    samples
        .iter()
        .zip(factors)
        .map(|(sample, factor)| moved(sample.base, sample.offset.times(factor)))
        .collect()
}

/// FuzzySkin.cpp, `fuzzy_polyline_ripple`: a sine wave across a polygon,
/// or across an open run of one (no segment back from the last point to
/// the first). An open run comes back as its inner points only; the
/// caller keeps its two ends.
fn fuzzy_polyline_ripple(
    poly: &mut Vec<Point2>,
    closed: bool,
    cfg: &FuzzyConfig,
    support: Option<&[Polygon]>,
) {
    let np = poly.len();
    let nseg = if closed { np } else { np - 1 };
    let Some(samples) = ripple_samples(poly, closed, nseg, cfg) else {
        return;
    };
    let mut out = ripple_positions(&samples, active_support(cfg, support), closed);

    if !closed {
        // A run too short for a single point keeps its own inner vertices.
        if out.is_empty() {
            out.extend_from_slice(&poly[1..np - 1]);
        }
        *poly = out;
        return;
    }

    while out.len() < 3 {
        out.push(poly[np - 2]);
    }
    *poly = out;
}

/// FuzzySkin.cpp, `fuzzy_extrusion_line_ripple`: the same wave across a
/// wall of changing width; each new junction takes the width of the one
/// its segment runs to. An open line keeps its first and last junction
/// around the new ones.
fn fuzzy_extrusion_line_ripple(
    ext_lines: &mut Vec<Junction>,
    cfg: &FuzzyConfig,
    support: Option<&[Polygon]>,
) {
    let poly: Vec<Point2> = ext_lines.iter().map(|junction| junction.p).collect();
    let np = poly.len();
    // Closed when the first and last junction coincide.
    let closed = poly[np - 1] == poly[0];

    let Some(samples) = ripple_samples(&poly, closed, np - 1, cfg) else {
        return;
    };
    let positions = ripple_positions(&samples, active_support(cfg, support), closed);
    let mut out: Vec<Junction> = samples
        .iter()
        .zip(positions)
        .map(|(sample, p)| {
            let j1 = ext_lines[sample.segment_end];
            Junction {
                p,
                w: j1.w,
                perimeter_index: j1.perimeter_index,
            }
        })
        .collect();

    if !closed {
        // With no new junction the line stays as it is.
        if !out.is_empty() {
            out.insert(0, ext_lines[0]);
            out.push(ext_lines[np - 1]);
            *ext_lines = out;
        }
        return;
    }

    pad_to_three(&mut out, ext_lines, |junction| junction);
    if out.len() >= 3 {
        *ext_lines = out;
    }
}

/// The loop that ends `fuzzy_polyline`, `fuzzy_extrusion_line` and
/// `fuzzy_extrusion_line_ripple` (FuzzySkin.cpp): a result of fewer than
/// three points is filled up with the source's next-to-last point. The
/// reference means to walk back from there but starts over on every
/// turn, so it is that one point each time, and only once when it is the
/// source's first. `source` has at least two points.
fn pad_to_three<T: Copy, U>(out: &mut Vec<U>, source: &[T], convert: impl Fn(T) -> U) {
    while out.len() < 3 {
        let point_idx = source.len() - 2;
        out.push(convert(source[point_idx]));
        if point_idx == 0 {
            break;
        }
    }
}

/// Roughens a polygon (`closed`), or an open run of one cut out by
/// [`split_line`](crate::split_line): the reference's `fuzzy_polyline`
/// (FuzzySkin.cpp, from Cura).
///
/// New points are put along the outline, between 3/4 and 5/4 of
/// `point_distance` apart, each moved across it by the noise at that
/// place times `thickness`; the original points are dropped. An open run
/// comes back without its first point, and the last new point falls
/// short of its last: the caller puts both ends back, as
/// [`apply_to_polygon`](crate::apply_to_polygon) does. A result of fewer
/// than three points (a very short outline) is padded, and when even that
/// does not reach three the points are left as they were.
///
/// `support` is the region the layer below carries; with
/// `cfg.skip_overhangs` a point outside it is put on the outline unmoved
/// and the roughness ramps back in over the next points. `None`, or the
/// setting off, changes nothing.
///
/// Different from the reference on purpose:
/// - the random numbers come from a generator seeded from this call's
///   input (see `rng.rs`), so the same call gives the same result;
/// - fewer than two points, or a `point_distance` of zero or less, leave
///   the points as they are. The reference reads out of bounds for the
///   first and never returns for the second.
pub fn fuzzy_polyline(
    points: &mut Vec<Point2>,
    closed: bool,
    slice_z: f64,
    cfg: &FuzzyConfig,
    support: Option<&[Polygon]>,
) {
    if cfg.noise == NoiseType::Ripple {
        // An open run (a straight one included) needs only its two ends.
        if points.len() < if closed { 3 } else { 2 } {
            return;
        }
        fuzzy_polyline_ripple(points, closed, cfg, support);
        return;
    }
    if points.len() < 2 || cfg.point_distance <= 0 {
        return;
    }

    let noise = Noise::of(cfg);
    let mut rng = CallRng::for_call(slice_z, points[0], points.len());

    let point_distance = cfg.point_distance as f64;
    let thickness = cfg.thickness as f64;
    // The point distance may vary between 3/4 and 5/4 the supplied value.
    let min_dist_between_points = point_distance * 3.0 / 4.0;
    let range_random_point_dist = point_distance / 2.0;
    // The distance to go along the line before making the first new point.
    let mut dist_left_over = rng.value() * (min_dist_between_points / 2.0);
    let support = active_support(cfg, support);

    // Each new point on the outline, and its displacement. The reference
    // keeps these apart only when the overhang skip is on and otherwise
    // adds them at once; the sum is the same either way.
    let mut base_points: Vec<Point2> = Vec::with_capacity(points.len());
    let mut offsets: Vec<Vec2> = Vec::with_capacity(points.len());

    let mut p0 = points[points.len() - 1];
    let mut skip_first = !closed;
    for &p1 in points.iter() {
        if skip_first {
            // Skip the first point for open path.
            skip_first = false;
            p0 = p1;
            continue;
        }
        // 'a' is the (next) new point between p0 and p1.
        let p0p1 = Vec2::between(p0, p1);
        let p0p1_size = p0p1.norm();
        let mut p0pa_dist = dist_left_over;
        while p0pa_dist < p0p1_size {
            let pa = moved(p0, p0p1.times(p0pa_dist / p0p1_size));
            let r = noise.value(
                &mut rng,
                unscale(pa.x as f64),
                unscale(pa.y as f64),
                slice_z,
            ) * thickness;
            base_points.push(pa);
            offsets.push(p0p1.perp().normalized().times(r));
            p0pa_dist += min_dist_between_points + rng.value() * range_random_point_dist;
        }
        dist_left_over = p0pa_dist - p0p1_size;
        p0 = p1;
    }

    let factors = match support {
        Some(support) => {
            let supported: Vec<bool> = base_points
                .iter()
                .map(|&pa| contains(support, pa))
                .collect();
            // `closed` as the caller gave it: the reference keeps it aside
            // as `was_closed`, since its loop flips the flag.
            support_blend_factors(&supported, closed)
        }
        None => vec![1.0; base_points.len()],
    };
    let mut out: Vec<Point2> = base_points
        .iter()
        .zip(&offsets)
        .zip(factors)
        .map(|((&base, offset), factor)| moved(base, offset.times(factor)))
        .collect();

    pad_to_three(&mut out, points, |point| point);
    if out.len() >= 3 {
        *points = out;
    }
}

/// How one new junction takes the noise `r` (in units), in each mode: the
/// `switch (cfg.mode)` that `fuzzy_extrusion_line` (FuzzySkin.cpp) has
/// twice. Widths are cut to whole units, as the C++ conversion to
/// `coord_t` does.
fn apply_mode(
    junction: &mut Junction,
    dir: Vec2,
    r: f64,
    mode: FuzzyMode,
    min_extrusion_width: f64,
) {
    let w0 = junction.w as f64;
    match mode {
        FuzzyMode::Displacement => {
            junction.p = moved(junction.p, dir.times(r));
        }
        FuzzyMode::Extrusion => {
            junction.w = f64::max(w0 + r + min_extrusion_width, min_extrusion_width) as Coord;
        }
        FuzzyMode::Combined => {
            let rad = f64::max(w0 + r + min_extrusion_width, min_extrusion_width);
            junction.p = moved(junction.p, dir.times((rad - w0) / 2.0));
            junction.w = rad as Coord;
        }
    }
}

/// Roughens a wall of changing width: the reference's
/// `fuzzy_extrusion_line` (FuzzySkin.cpp, from Cura).
///
/// New junctions are put along the line as [`fuzzy_polyline`] puts
/// points, each with the width of the junction its segment runs to, and
/// the noise then moves them (`Displacement`), changes their width
/// (`Extrusion`) or both (`Combined`). A width never goes below what a
/// line of `layer_height` (mm) needs to be a line at all. The first
/// junction is kept; on a closed line (first and last junction at the
/// same place) it is moved onto the last new one, so the line stays
/// closed, and new junctions too close to that seam are dropped.
///
/// `support` works as in [`fuzzy_polyline`]: with `cfg.skip_overhangs`, a
/// junction outside it keeps its place and its width.
///
/// As ported, the reference's own oddities included: the width modes add
/// the smallest width on top of the noise (`w + r + min`), and with the
/// overhang skip live in those modes the junction that is passed through
/// unchanged gets that added as well.
///
/// Different from the reference on purpose: the random numbers (see
/// `rng.rs`), and that fewer than two junctions or a `point_distance` of
/// zero or less leave the line as it is.
pub fn fuzzy_extrusion_line(
    junctions: &mut Vec<Junction>,
    slice_z: f64,
    layer_height: f64,
    cfg: &FuzzyConfig,
    support: Option<&[Polygon]>,
) {
    if cfg.noise == NoiseType::Ripple {
        // An open line (a straight thin wall) needs only its two ends.
        let closed = match (junctions.first(), junctions.last()) {
            (Some(first), Some(last)) => first.p == last.p,
            _ => false,
        };
        if junctions.len() < if closed { 3 } else { 2 } {
            return;
        }
        fuzzy_extrusion_line_ripple(junctions, cfg, support);
        return;
    }
    if junctions.len() < 2 || cfg.point_distance <= 0 {
        return;
    }

    let noise = Noise::of(cfg);
    let mut rng = CallRng::for_call(slice_z, junctions[0].p, junctions.len());

    let point_distance = cfg.point_distance as f64;
    let thickness = cfg.thickness as f64;
    let min_dist_between_points = point_distance * 3.0 / 4.0;
    let range_random_point_dist = point_distance / 2.0;
    // A rounded-rectangle extrusion needs width > height * (1 - PI / 4);
    // keep 5% above it.
    let min_extrusion_width = scale(layer_height * (1.0 - 0.25 * PI) * 1.05);
    let mut dist_left_over = rng.value() * (min_dist_between_points / 2.0);
    let support = active_support(cfg, support);

    let mut out: Vec<Junction> = Vec::with_capacity(junctions.len());
    // Only filled when the overhang skip is live: displacement and width
    // change both wait until every junction's support is known. One entry
    // per junction of `out`, the passed-through ones included.
    let mut supported: Vec<bool> = Vec::new();
    let mut noise_r: Vec<f64> = Vec::new();
    let mut dirs: Vec<Vec2> = Vec::new();

    let mut p0 = junctions[0];
    for &p1 in junctions.iter() {
        if p0.p == p1.p {
            // Connect endpoints.
            out.push(p1);
            if support.is_some() {
                // Passed through: counted as supported so it does not
                // pull the ramp down, with nothing to displace it by.
                supported.push(true);
                noise_r.push(0.0);
                dirs.push(Vec2::ZERO);
            }
            continue;
        }

        // 'a' is the (next) new point between p0 and p1.
        let p0p1 = Vec2::between(p0.p, p1.p);
        let p0p1_size = p0p1.norm();
        let mut p0pa_dist = dist_left_over;
        while p0pa_dist < p0p1_size {
            let pa = moved(p0.p, p0p1.times(p0pa_dist / p0p1_size));
            let r = noise.value(
                &mut rng,
                unscale(pa.x as f64),
                unscale(pa.y as f64),
                slice_z,
            ) * thickness;
            let dir = p0p1.perp().normalized();
            let mut junction = Junction {
                p: pa,
                w: p1.w,
                perimeter_index: p1.perimeter_index,
            };
            match support {
                Some(support) => {
                    supported.push(contains(support, pa));
                    noise_r.push(r);
                    dirs.push(dir);
                }
                None => apply_mode(&mut junction, dir, r, cfg.mode, min_extrusion_width),
            }
            out.push(junction);
            p0pa_dist += min_dist_between_points + rng.value() * range_random_point_dist;
        }
        dist_left_over = p0pa_dist - p0p1_size;
        p0 = p1;
    }

    let closed = junctions[junctions.len() - 1].p == junctions[0].p;
    if support.is_some() && !out.is_empty() {
        let factors = support_blend_factors(&supported, closed);
        for (i, junction) in out.iter_mut().enumerate() {
            let f = factors[i];
            let r = noise_r[i] * f;
            if r == 0.0 && f == 0.0 {
                // Unsupported: leave it alone.
                continue;
            }
            apply_mode(junction, dirs[i], r, cfg.mode, min_extrusion_width);
        }
    }

    pad_to_three(&mut out, junctions, |junction| junction);

    if closed {
        // Connect endpoints. The closing segment is the only one that can
        // be shorter than the smallest point spacing (the last new point
        // may land anywhere before the seam), and a tiny segment with a
        // large step in width prints as a blob at the seam: drop trailing
        // points until the closure keeps that spacing too.
        let min_closing_dist = min_dist_between_points;
        while out.len() > 3
            && Vec2::between(out[1].p, out[out.len() - 1].p).norm() < min_closing_dist
        {
            out.pop();
        }
        let back = out[out.len() - 1];
        out[0].p = back.p;
        out[0].w = back.w;
    }

    if out.len() >= 3 {
        *junctions = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FuzzyType;
    use crate::testing::{mm, outline_arc, outline_distance, rect};
    use edge_geom::scaled;

    /// The settings the tests start from: the reference's defaults (0.2 mm
    /// thick, a point every 0.3 mm, classic noise) on the outer wall.
    fn config() -> FuzzyConfig {
        FuzzyConfig {
            kind: FuzzyType::External,
            layer_id: 3,
            ..FuzzyConfig::default()
        }
    }

    fn square() -> Vec<Point2> {
        rect(0.0, 0.0, 20.0, 20.0).points
    }

    fn fuzzed(
        points: &[Point2],
        closed: bool,
        z: f64,
        cfg: &FuzzyConfig,
        support: Option<&[Polygon]>,
    ) -> Vec<Point2> {
        let mut points = points.to_vec();
        fuzzy_polyline(&mut points, closed, z, cfg, support);
        points
    }

    fn junction(x: f64, y: f64) -> Junction {
        Junction {
            p: mm(x, y),
            w: 450_000,
            perimeter_index: 0,
        }
    }

    /// The square as a closed wall: its first junction again at the end.
    fn square_line() -> Vec<Junction> {
        vec![
            junction(0.0, 0.0),
            junction(20.0, 0.0),
            junction(20.0, 20.0),
            junction(0.0, 20.0),
            junction(0.0, 0.0),
        ]
    }

    fn fuzzed_line(
        line: &[Junction],
        cfg: &FuzzyConfig,
        support: Option<&[Polygon]>,
    ) -> Vec<Junction> {
        let mut line = line.to_vec();
        fuzzy_extrusion_line(&mut line, 0.6, 0.2, cfg, support);
        line
    }

    /// `min_extrusion_width` for a 0.2 mm layer.
    fn min_width() -> f64 {
        scale(0.2 * (1.0 - 0.25 * PI) * 1.05)
    }

    #[test]
    fn blend_factors_by_hand() {
        let t = true;
        let f = false;
        let third = 1.0 / 3.0;
        let two_thirds = 2.0 / 3.0;

        assert_eq!(support_blend_factors(&[], true), Vec::<f64>::new());
        assert_eq!(support_blend_factors(&[t, t, t, t], false), [1.0; 4]);
        assert_eq!(support_blend_factors(&[t, t, t, t], true), [1.0; 4]);
        assert_eq!(support_blend_factors(&[f, f, f], true), [0.0; 3]);

        // A single point over air in the middle of an open run: a ramp of
        // two points on either side.
        assert_eq!(
            support_blend_factors(&[t, t, t, t, f, t, t, t, t], false),
            [
                1.0, 1.0, two_thirds, third, 0.0, third, two_thirds, 1.0, 1.0
            ]
        );

        // Over air at the start. Open, the far end is clear of it; closed,
        // the end is the start's neighbour and ramps too.
        let at_start = [f, t, t, t, t, t, t];
        assert_eq!(
            support_blend_factors(&at_start, false),
            [0.0, third, two_thirds, 1.0, 1.0, 1.0, 1.0]
        );
        assert_eq!(
            support_blend_factors(&at_start, true),
            [0.0, third, two_thirds, 1.0, 1.0, two_thirds, third]
        );

        // Over air at the end.
        let at_end = [t, t, t, t, t, f];
        assert_eq!(
            support_blend_factors(&at_end, false),
            [1.0, 1.0, 1.0, two_thirds, third, 0.0]
        );
        assert_eq!(
            support_blend_factors(&at_end, true),
            [third, two_thirds, 1.0, two_thirds, third, 0.0]
        );

        // A supported run shorter than the ramp never gets to one.
        assert_eq!(support_blend_factors(&[f, t, f], false), [0.0, third, 0.0]);
        assert_eq!(
            support_blend_factors(&[f, t, t, f], true),
            [0.0, third, third, 0.0]
        );
        // One supported point alone on a loop has air on both sides.
        assert_eq!(support_blend_factors(&[t, f], true), [third, 0.0]);
    }

    #[test]
    fn contains_counts_polygons_and_takes_the_border() {
        let outer = rect(0.0, 0.0, 10.0, 10.0);
        let inner = rect(4.0, 4.0, 6.0, 6.0);
        let one = [outer.clone()];
        assert!(contains(&one, mm(5.0, 5.0)));
        assert!(!contains(&one, mm(15.0, 5.0)));
        assert!(!contains(&one, mm(-0.000001, 5.0)));
        // On an edge and on a corner.
        assert!(contains(&one, mm(10.0, 5.0)));
        assert!(contains(&one, mm(0.0, 0.0)));
        assert!(contains(&one, mm(3.0, 10.0)));
        // Inside two polygons is outside: the second is a hole, whichever
        // way it is wound.
        let two = [outer, inner];
        assert!(!contains(&two, mm(5.0, 5.0)));
        assert!(contains(&two, mm(2.0, 5.0)));
        assert!(contains(&two, mm(4.0, 5.0)));
        assert!(!contains(&[], mm(0.0, 0.0)));
        // A slanted edge, just to either side of it and on it.
        let triangle = [Polygon {
            points: vec![mm(0.0, 0.0), mm(10.0, 0.0), mm(0.0, 10.0)],
        }];
        assert!(contains(&triangle, mm(5.0, 5.0)));
        assert!(contains(&triangle, mm(4.999999, 5.0)));
        assert!(!contains(&triangle, mm(5.000001, 5.0)));
    }

    #[test]
    fn polyline_is_deterministic_and_changes_with_height() {
        let cfg = config();
        let first = fuzzed(&square(), true, 0.6, &cfg, None);
        assert_eq!(first, fuzzed(&square(), true, 0.6, &cfg, None));
        assert_ne!(first, square());
        assert_ne!(first, fuzzed(&square(), true, 0.8, &cfg, None));
        // Another outline at the same height is not the same either.
        let shifted: Vec<Point2> = square()
            .iter()
            .map(|p| Point2 { x: p.x + 7, y: p.y })
            .collect();
        let other = fuzzed(&shifted, true, 0.6, &cfg, None);
        let back: Vec<Point2> = other
            .iter()
            .map(|p| Point2 { x: p.x - 7, y: p.y })
            .collect();
        assert_ne!(first, back);
    }

    #[test]
    fn polyline_points_stay_within_the_thickness() {
        let cfg = config();
        let outline = square();
        let out = fuzzed(&outline, true, 0.6, &cfg, None);
        let limit = cfg.thickness as f64 + 2.0;
        let mut furthest: f64 = 0.0;
        for &p in &out {
            let distance = outline_distance(p, &outline);
            assert!(distance <= limit, "{p:?} is {distance} from the outline");
            furthest = furthest.max(distance);
        }
        // And the points do use the room they have.
        assert!(furthest > 0.9 * cfg.thickness as f64, "{furthest}");
        // Both sides of the outline: some inside the square, some outside.
        let inside = |p: &Point2| p.x > 0 && p.x < scaled(20.0) && p.y > 0 && p.y < scaled(20.0);
        let inside_count = out.iter().filter(|p| inside(p)).count();
        assert!(inside_count > out.len() / 4 && inside_count < out.len() * 3 / 4);
    }

    #[test]
    fn polyline_points_are_spaced_around_the_point_distance() {
        // With no thickness the points stay on the outline, where their
        // spacing can be measured; the same random numbers are drawn.
        let flat = FuzzyConfig {
            thickness: 0,
            ..config()
        };
        let outline = square();
        let base = fuzzed(&outline, true, 0.6, &flat, None);
        let perimeter = scaled(80.0) as f64;
        let distance = flat.point_distance as f64;
        assert!(base.len() as f64 >= perimeter / (1.25 * distance) - 1.0);
        assert!(base.len() as f64 <= perimeter / (0.75 * distance) + 1.0);

        // The walk starts with the segment from the last point to the
        // first, so the arc is measured from the last point.
        let from_last: Vec<Point2> = [3, 0, 1, 2].iter().map(|&i| outline[i]).collect();
        let arcs: Vec<f64> = base
            .iter()
            .map(|&p| {
                assert!(outline_distance(p, &outline) <= 1.0);
                outline_arc(p, &from_last)
            })
            .collect();
        // The first point is less than 3/8 of the distance in.
        assert!(arcs[0] < 0.375 * distance + 2.0, "{}", arcs[0]);
        let (mut shortest, mut longest) = (f64::INFINITY, 0.0_f64);
        for pair in arcs.windows(2) {
            let step = pair[1] - pair[0];
            assert!(step >= 0.75 * distance - 3.0, "{step}");
            assert!(step <= 1.25 * distance + 3.0, "{step}");
            shortest = shortest.min(step);
            longest = longest.max(step);
        }
        // The whole range is used.
        assert!(shortest < 0.8 * distance && longest > 1.2 * distance);

        // With the thickness back, each point is its place on the outline
        // moved by no more than the thickness.
        let cfg = config();
        let out = fuzzed(&outline, true, 0.6, &cfg, None);
        assert_eq!(out.len(), base.len());
        for (&moved, &on_outline) in out.iter().zip(&base) {
            let by = ((moved.x - on_outline.x) as f64).hypot((moved.y - on_outline.y) as f64);
            assert!(by <= cfg.thickness as f64 + 1.5, "{by}");
        }
    }

    #[test]
    fn open_polyline_drops_its_first_point_and_has_no_closing_segment() {
        let flat = FuzzyConfig {
            thickness: 0,
            ..config()
        };
        // Along x, then up: an L of two segments.
        let run = vec![mm(0.0, 0.0), mm(10.0, 0.0), mm(10.0, 5.0)];
        let out = fuzzed(&run, false, 0.6, &flat, None);
        let distance = flat.point_distance as f64;

        // The first point is not kept: the result starts with the first
        // new point, less than 3/8 of the distance along from it.
        assert_ne!(out[0], run[0]);
        assert_eq!(out[0].y, 0);
        assert!((out[0].x as f64) < 0.375 * distance + 1.0);
        // Nor is the last: the result ends short of it.
        let last = out[out.len() - 1];
        assert_eq!(last.x, scaled(10.0));
        assert!(last.y < scaled(5.0));
        assert!((scaled(5.0) - last.y) as f64 <= 1.25 * distance + 1.0);
        // Every point is on one of the two segments, none on the way back
        // from the end to the start.
        for &p in &out {
            assert!(
                (p.y == 0 && (0..=scaled(10.0)).contains(&p.x))
                    || (p.x == scaled(10.0) && (0..=scaled(5.0)).contains(&p.y)),
                "{p:?}"
            );
        }
        let expected = scaled(15.0) as f64 / distance;
        assert!((out.len() as f64) > expected * 0.8 && (out.len() as f64) < expected * 1.34);

        // Closed, the same three points do get the segment back.
        let closed = fuzzed(&run, true, 0.6, &flat, None);
        assert!(closed.iter().any(|p| p.y != 0 && p.x != scaled(10.0)));
    }

    #[test]
    fn short_and_degenerate_polylines() {
        let cfg = config();
        // Nothing to walk: left as they are.
        for points in [vec![], vec![mm(1.0, 1.0)]] {
            assert_eq!(fuzzed(&points, true, 0.2, &cfg, None), points);
            assert_eq!(fuzzed(&points, false, 0.2, &cfg, None), points);
        }
        // A point distance of zero would never get anywhere.
        let stuck = FuzzyConfig {
            point_distance: 0,
            ..cfg
        };
        assert_eq!(fuzzed(&square(), true, 0.2, &stuck, None), square());

        // An outline too small for three new points is filled up to three
        // with its next-to-last point.
        let tiny = vec![mm(0.0, 0.0), mm(0.05, 0.0), mm(0.0, 0.05)];
        let out = fuzzed(&tiny, true, 0.2, &cfg, None);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2], tiny[1]);

        // An open run of two points too short for a new point: one copy
        // of its first point is all the padding gives, which is not
        // three, so the run is left alone.
        let stub = vec![mm(0.0, 0.0), mm(0.0001, 0.0)];
        assert_eq!(fuzzed(&stub, false, 0.2, &cfg, None), stub);
    }

    #[test]
    fn coherent_noises_follow_the_height_and_the_place() {
        for noise in [
            NoiseType::Perlin,
            NoiseType::Billow,
            NoiseType::RidgedMulti,
            NoiseType::Voronoi,
        ] {
            let cfg = FuzzyConfig { noise, ..config() };
            let flat = FuzzyConfig {
                thickness: 0,
                ..cfg
            };
            let outline = square();
            let out = fuzzed(&outline, true, 0.6, &cfg, None);
            assert_eq!(out, fuzzed(&outline, true, 0.6, &cfg, None), "{noise:?}");
            let base = fuzzed(&outline, true, 0.6, &flat, None);
            assert_eq!(out.len(), base.len(), "{noise:?}");

            // Each point is moved by the module's value where it stands,
            // times the thickness.
            let module = match Noise::of(&cfg) {
                Noise::Coherent(module) => module,
                Noise::Uniform => panic!("{noise:?} is not the classic noise"),
            };
            let mut moved_any = false;
            for (&at, &on_outline) in out.iter().zip(&base) {
                let value = module.value(
                    unscale(on_outline.x as f64),
                    unscale(on_outline.y as f64),
                    0.6,
                );
                let by = ((at.x - on_outline.x) as f64).hypot((at.y - on_outline.y) as f64);
                let expected = (value * cfg.thickness as f64).abs();
                assert!(
                    (by - expected).abs() <= 1.5,
                    "{noise:?}: {by} vs {expected}"
                );
                moved_any |= by > 1000.0;
            }
            assert!(moved_any, "{noise:?}");
        }
    }

    #[test]
    fn octaves_are_held_to_what_libnoise_takes() {
        let with = |noise, octaves| {
            Noise::of(&FuzzyConfig {
                noise,
                octaves,
                ..config()
            })
        };
        let octaves_of = |noise: Noise| match noise {
            Noise::Coherent(Module::Perlin { octaves, .. })
            | Noise::Coherent(Module::Billow { octaves, .. })
            | Noise::Coherent(Module::RidgedMulti { octaves, .. }) => octaves,
            _ => panic!("no octaves"),
        };
        assert_eq!(octaves_of(with(NoiseType::Perlin, 4)), 4);
        assert_eq!(octaves_of(with(NoiseType::Perlin, 0)), 1);
        assert_eq!(octaves_of(with(NoiseType::Billow, 99)), 30);
        assert_eq!(octaves_of(with(NoiseType::RidgedMulti, 99)), 30);
        assert_eq!(octaves_of(with(NoiseType::RidgedMulti, 0)), 0);
        assert!(matches!(with(NoiseType::Classic, 4), Noise::Uniform));
        // The frequency is one over the scale.
        let scaled_noise = Noise::of(&FuzzyConfig {
            noise: NoiseType::Voronoi,
            scale: 4.0,
            ..config()
        });
        assert!(matches!(
            scaled_noise,
            Noise::Coherent(Module::Voronoi { frequency }) if frequency == 0.25
        ));
    }

    /// A square the y = 0 line cuts through, so that the ripple's anchor
    /// is a crossing: the leftmost, at (0, 0), 70 mm round from the first
    /// point.
    fn ripple_square() -> Vec<Point2> {
        rect(0.0, -10.0, 20.0, 10.0).points
    }

    fn ripple_config() -> FuzzyConfig {
        FuzzyConfig {
            noise: NoiseType::Ripple,
            point_distance: scaled(0.5),
            // 80 mm round: a wave every 10 mm.
            ripples_per_layer: 8,
            layer_id: 0,
            ..config()
        }
    }

    /// How far inside the ripple square a point is, in units, and how far
    /// round (mm); `None` near a corner, where "inside" has two answers.
    fn ripple_offset_at(p: Point2, outline: &[Point2]) -> Option<(f64, f64)> {
        let arc_mm = outline_arc(p, outline) / 1e6;
        let along_side = arc_mm % 20.0;
        if !(0.5..=19.5).contains(&along_side) {
            return None;
        }
        let inside = p.x > 0 && p.x < scaled(20.0) && p.y > scaled(-10.0) && p.y < scaled(10.0);
        let distance = outline_distance(p, outline);
        Some((if inside { distance } else { -distance }, arc_mm))
    }

    #[test]
    fn ripple_is_a_sine_along_the_wall() {
        let cfg = ripple_config();
        let outline = ripple_square();
        let out = fuzzed(&outline, true, 0.6, &cfg, None);
        // A point every 0.5 mm of 80 mm.
        assert!((159..=161).contains(&out.len()), "{}", out.len());
        assert_eq!(
            out,
            fuzzed(&outline, true, 99.0, &cfg, None),
            "the height plays no part"
        );

        let mut checked = 0;
        let mut peak: f64 = 0.0;
        for &p in &out {
            let Some((offset, arc_mm)) = ripple_offset_at(p, &outline) else {
                continue;
            };
            // Zero at the anchor and rising, a wave every 10 mm, towards
            // the inside (the left of a counter-clockwise outline) first.
            let expected = (2.0 * PI * (arc_mm - 70.0) / 10.0).sin() * cfg.thickness as f64;
            assert!(
                (offset - expected).abs() <= 3.0,
                "at {arc_mm} mm: {offset} vs {expected}"
            );
            // Periodic: the same 10 mm on.
            let again = (2.0 * PI * (arc_mm + 10.0 - 70.0) / 10.0).sin() * cfg.thickness as f64;
            assert!((offset - again).abs() <= 3.0);
            peak = peak.max(offset.abs());
            checked += 1;
        }
        assert!(checked > 140, "{checked}");
        assert!(peak > 0.99 * cfg.thickness as f64 && peak <= cfg.thickness as f64 + 2.0);

        // The displacement repeats from point to point 10 mm (20 points)
        // apart along a side.
        let offsets: Vec<Option<(f64, f64)>> =
            out.iter().map(|&p| ripple_offset_at(p, &outline)).collect();
        let mut pairs = 0;
        for i in 0..offsets.len() - 20 {
            if let (Some((a, arc_a)), Some((b, arc_b))) = (offsets[i], offsets[i + 20])
                && (arc_b - arc_a - 10.0).abs() < 1e-3
            {
                assert!((a - b).abs() <= 3.0, "{a} vs {b}");
                pairs += 1;
            }
        }
        assert!(pairs > 30, "{pairs}");
    }

    #[test]
    fn ripple_steps_round_from_layer_to_layer() {
        let outline = ripple_square();
        let on_layer = |layer_id: i32, layers_between: i32, ripple_offset: f64| {
            let cfg = FuzzyConfig {
                layer_id,
                layers_between_ripple_offset: layers_between,
                ripple_offset,
                ..ripple_config()
            };
            fuzzed(&outline, true, 0.6, &cfg, None)
        };
        let offsets = |points: &[Point2]| -> Vec<f64> {
            points
                .iter()
                .filter_map(|&p| ripple_offset_at(p, &outline))
                .map(|(offset, _)| offset)
                .collect()
        };
        let base = offsets(&on_layer(0, 1, 50.0));

        // Half a wave on: the mirror image.
        let half = offsets(&on_layer(1, 1, 50.0));
        assert_eq!(base.len(), half.len());
        for (a, b) in base.iter().zip(&half) {
            assert!((a + b).abs() <= 4.0, "{a} vs {b}");
        }
        // Two halves on: back where it began.
        let whole = offsets(&on_layer(2, 1, 50.0));
        for (a, b) in base.iter().zip(&whole) {
            assert!((a - b).abs() <= 4.0, "{a} vs {b}");
        }
        // Layers that share a step share the pattern.
        assert_eq!(on_layer(4, 3, 50.0), on_layer(5, 3, 50.0));
        assert_ne!(on_layer(5, 3, 50.0), on_layer(6, 3, 50.0));
        // No offset, no step.
        assert_eq!(on_layer(0, 1, 0.0), on_layer(9, 1, 0.0));

        assert_eq!(ripple_phase_shift_rad(&ripple_config()), 0.0);
        let quarter = FuzzyConfig {
            layer_id: 5,
            layers_between_ripple_offset: 2,
            ripple_offset: 25.0,
            ..ripple_config()
        };
        // Layer 5 is in the third pair of layers: two quarter waves on.
        assert!((ripple_phase_shift_rad(&quarter) - PI).abs() < 1e-12);
    }

    #[test]
    fn ripple_anchor_is_the_leftmost_crossing_or_the_nearest_vertex() {
        assert!((ripple_anchor_arc_mm(&ripple_square()) - 70.0).abs() < 1e-9);
        // No crossing: the vertex nearest to y = 0, here the first one.
        assert_eq!(
            ripple_anchor_arc_mm(&rect(0.0, 5.0, 20.0, 25.0).points),
            0.0
        );
        // Above the line and wound the other way round.
        let mut above = rect(0.0, -30.0, 20.0, -5.0).points;
        above.rotate_left(2);
        // Now starts at (20, -5); (0, -5) is as near to the line and
        // comes later, so the first found stays.
        assert_eq!(ripple_anchor_arc_mm(&above), 0.0);
    }

    #[test]
    fn open_ripple_runs_come_back_as_their_inner_points() {
        let cfg = ripple_config();
        // 10 mm along x: new points every 0.5 mm from 0.5 on, its two ends
        // left to the caller.
        let run = vec![mm(0.0, 3.0), mm(10.0, 3.0)];
        let out = fuzzed(&run, false, 0.6, &cfg, None);
        assert!((19..=20).contains(&out.len()), "{}", out.len());
        assert!((out[0].x - scaled(0.5)).abs() <= 1);
        assert!(out.iter().all(|p| p.x > 0 && p.x < scaled(10.0)));
        assert!(out.iter().any(|p| (p.y - scaled(3.0)).abs() > scaled(0.15)));

        // Too short for a single point: its inner vertices, here none.
        let stub = vec![mm(0.0, 3.0), mm(0.2, 3.0)];
        assert!(fuzzed(&stub, false, 0.6, &cfg, None).is_empty());
        let bent = vec![mm(0.0, 3.0), mm(0.1, 3.0), mm(0.2, 3.0)];
        assert_eq!(fuzzed(&bent, false, 0.6, &cfg, None), vec![mm(0.1, 3.0)]);

        // Closed needs three points, open two; fewer are left alone, and
        // so is a wall asked for no ripples.
        let two = vec![mm(0.0, 0.0), mm(10.0, 0.0)];
        assert_eq!(fuzzed(&two, true, 0.6, &cfg, None), two);
        let one = vec![mm(0.0, 0.0)];
        assert_eq!(fuzzed(&one, false, 0.6, &cfg, None), one);
        let none = FuzzyConfig {
            ripples_per_layer: 0,
            ..cfg
        };
        assert_eq!(
            fuzzed(&ripple_square(), true, 0.6, &none, None),
            ripple_square()
        );
    }

    /// For each point, how many points away (going round) the nearest one
    /// over air is.
    fn steps_to_air(supported: &[bool]) -> Vec<usize> {
        let n = supported.len();
        (0..n)
            .map(|i| {
                (0..n)
                    .find(|&d| !supported[(i + d) % n] || !supported[(i + n - d) % n])
                    .unwrap_or(n)
            })
            .collect()
    }

    #[test]
    fn overhang_skip_holds_unsupported_points_on_the_outline() {
        let cfg = FuzzyConfig {
            skip_overhangs: true,
            ..config()
        };
        let outline = square();
        // The layer below carries the left half.
        let support = [rect(-5.0, -5.0, 10.0, 25.0)];

        let free = fuzzed(&outline, true, 0.6, &cfg, None);
        let base = fuzzed(
            &outline,
            true,
            0.6,
            &FuzzyConfig {
                thickness: 0,
                ..cfg
            },
            None,
        );
        let held = fuzzed(&outline, true, 0.6, &cfg, Some(&support));
        assert_eq!(held.len(), free.len());
        assert_eq!(held.len(), base.len());
        assert_ne!(held, free);

        let supported: Vec<bool> = base.iter().map(|&p| p.x <= scaled(10.0)).collect();
        let steps = steps_to_air(&supported);
        let mut counts = [0; 4];
        for i in 0..held.len() {
            let full = (
                (free[i].x - base[i].x) as f64,
                (free[i].y - base[i].y) as f64,
            );
            let got = (
                (held[i].x - base[i].x) as f64,
                (held[i].y - base[i].y) as f64,
            );
            let factor = (steps[i] as f64 / 3.0).min(1.0);
            match steps[i] {
                // Over air: exactly on the outline.
                0 => assert_eq!(held[i], base[i]),
                // Well inside: untouched by the option.
                3.. => assert_eq!(held[i], free[i]),
                // In between: a third, then two thirds, of the way out.
                _ => {
                    assert!(
                        (got.0 - full.0 * factor).abs() <= 1.0,
                        "{got:?} vs {full:?} * {factor}"
                    );
                    assert!(
                        (got.1 - full.1 * factor).abs() <= 1.0,
                        "{got:?} vs {full:?} * {factor}"
                    );
                }
            }
            counts[steps[i].min(3)] += 1;
        }
        // About half the points are over air, and the loop has two edges
        // of the support to ramp at, on both sides of the air.
        assert!(
            counts[0] > held.len() / 3 && counts[3] > held.len() / 4,
            "{counts:?}"
        );
        assert_eq!(counts[1], 2);
        assert_eq!(counts[2], 2);

        // Off, absent or empty, the option changes nothing.
        let off = FuzzyConfig {
            skip_overhangs: false,
            ..cfg
        };
        assert_eq!(fuzzed(&outline, true, 0.6, &off, Some(&support)), free);
        assert_eq!(fuzzed(&outline, true, 0.6, &cfg, Some(&[])), free);
        // Carried everywhere, nothing is held back.
        let all = [rect(-5.0, -5.0, 25.0, 25.0)];
        assert_eq!(fuzzed(&outline, true, 0.6, &cfg, Some(&all)), free);
        // Carried nowhere, everything is.
        let nowhere = [rect(50.0, 50.0, 60.0, 60.0)];
        assert_eq!(fuzzed(&outline, true, 0.6, &cfg, Some(&nowhere)), base);
    }

    #[test]
    fn overhang_skip_on_an_open_run_does_not_wrap() {
        let cfg = FuzzyConfig {
            skip_overhangs: true,
            ..config()
        };
        // Air over the first 2 mm of a 10 mm run.
        let run = vec![mm(0.0, 0.0), mm(10.0, 0.0)];
        let support = [rect(2.0, -5.0, 15.0, 5.0)];
        let free = fuzzed(&run, false, 0.6, &cfg, None);
        let held = fuzzed(&run, false, 0.6, &cfg, Some(&support));
        assert_eq!(held.len(), free.len());
        let over_air = held.iter().filter(|p| p.x < scaled(2.0)).count();
        assert!(over_air >= 4);
        for i in 0..held.len() {
            if i < over_air {
                assert_eq!(held[i].y, 0);
            } else if i >= over_air + 2 {
                // The far end is far from the air: open, the ends are
                // not neighbours.
                assert_eq!(held[i], free[i]);
            }
        }
    }

    #[test]
    fn overhang_skip_scales_the_ripple() {
        let cfg = FuzzyConfig {
            skip_overhangs: true,
            ..ripple_config()
        };
        let outline = ripple_square();
        let support = [rect(-5.0, -15.0, 10.0, 15.0)];
        let free = fuzzed(&outline, true, 0.6, &cfg, None);
        let held = fuzzed(&outline, true, 0.6, &cfg, Some(&support));
        assert_eq!(held.len(), free.len());
        let mut on_outline = 0;
        for (&h, &f) in held.iter().zip(&free) {
            if f.x > scaled(10.5) {
                assert!(outline_distance(h, &outline) <= 1.0, "{h:?}");
                on_outline += 1;
            } else if f.x < scaled(8.0) {
                assert_eq!(h, f);
            }
        }
        assert!(on_outline > 60);
    }

    #[test]
    fn line_displacement_keeps_widths_and_closes() {
        let cfg = config();
        let line = square_line();
        let out = fuzzed_line(&line, &cfg, None);
        assert_eq!(out, fuzzed_line(&line, &cfg, None));
        assert!(out.len() > 200);
        // Closed: the first junction is the last.
        assert_eq!(out[0], out[out.len() - 1]);
        let outline = square();
        let mut furthest: f64 = 0.0;
        for junction in &out {
            assert_eq!(junction.w, 450_000);
            assert_eq!(junction.perimeter_index, 0);
            let distance = outline_distance(junction.p, &outline);
            assert!(distance <= cfg.thickness as f64 + 2.0);
            furthest = furthest.max(distance);
        }
        assert!(furthest > 0.9 * cfg.thickness as f64);
        // The seam keeps the smallest spacing: no junction crowds the end.
        let last = out[out.len() - 1].p;
        let second = out[1].p;
        let gap = ((last.x - second.x) as f64).hypot((last.y - second.y) as f64);
        assert!(gap >= 0.75 * cfg.point_distance as f64);
    }

    #[test]
    fn line_extrusion_changes_widths_only() {
        let cfg = FuzzyConfig {
            mode: FuzzyMode::Extrusion,
            ..config()
        };
        let out = fuzzed_line(&square_line(), &cfg, None);
        assert_eq!(out[0], out[out.len() - 1]);
        let outline = square();
        let min = min_width();
        let (mut narrowest, mut widest) = (Coord::MAX, 0);
        for junction in &out {
            // On the line it was on.
            assert!(outline_distance(junction.p, &outline) <= 1.0);
            narrowest = narrowest.min(junction.w);
            widest = widest.max(junction.w);
        }
        // The reference adds the smallest width to the noise: the widths
        // spread around the line's own plus that.
        let middle = 450_000.0 + min;
        assert!(narrowest as f64 >= middle - cfg.thickness as f64 - 1.0);
        assert!(widest as f64 <= middle + cfg.thickness as f64);
        assert!((narrowest as f64) < middle - 0.9 * cfg.thickness as f64);
        assert!((widest as f64) > middle + 0.9 * cfg.thickness as f64);
    }

    #[test]
    fn line_widths_floor_at_the_smallest_extrusion_width() {
        // Noise deeper than the line is wide: the width would go below
        // nothing, and stops at the floor.
        let min = min_width();
        assert!((min - 45_066.0).abs() < 1.0, "{min}");
        for mode in [FuzzyMode::Extrusion, FuzzyMode::Combined] {
            let cfg = FuzzyConfig {
                mode,
                thickness: scaled(0.8),
                ..config()
            };
            let out = fuzzed_line(&square_line(), &cfg, None);
            let narrowest = out.iter().map(|junction| junction.w).min().unwrap();
            assert_eq!(narrowest, min as Coord, "{mode:?}");
            let at_floor = out
                .iter()
                .filter(|junction| junction.w == min as Coord)
                .count();
            assert!(at_floor > out.len() / 10, "{mode:?}: {at_floor}");
            assert!(out.iter().any(|junction| junction.w > 900_000), "{mode:?}");
            assert_eq!(out[0], out[out.len() - 1], "{mode:?}");
        }
    }

    #[test]
    fn line_combined_moves_by_half_the_change_in_width() {
        let cfg = FuzzyConfig {
            mode: FuzzyMode::Combined,
            ..config()
        };
        let out = fuzzed_line(&square_line(), &cfg, None);
        assert_eq!(out[0], out[out.len() - 1]);
        let outline = square();
        let mut moved_any = false;
        // The first junction is put on the last; every one in between is
        // a new one.
        for junction in &out[1..] {
            // Near a corner the nearest side is not always the one the
            // junction was moved off.
            let near_corner = outline.iter().any(|corner| {
                ((junction.p.x - corner.x) as f64).hypot((junction.p.y - corner.y) as f64)
                    < 500_000.0
            });
            if near_corner {
                continue;
            }
            let change = (junction.w - 450_000) as f64;
            let distance = outline_distance(junction.p, &outline);
            assert!(
                (distance - change.abs() / 2.0).abs() <= 2.5,
                "{distance} vs {change}"
            );
            // Wider moves to the left of the way round, the inside.
            let inside = junction.p.x > 0
                && junction.p.x < scaled(20.0)
                && junction.p.y > 0
                && junction.p.y < scaled(20.0);
            if distance > 10.0 {
                assert_eq!(inside, change > 0.0);
                moved_any = true;
            }
        }
        assert!(moved_any);
    }

    #[test]
    fn open_line_keeps_its_first_junction_and_stays_open() {
        let cfg = config();
        let mut line = vec![junction(0.0, 0.0), junction(10.0, 0.0), junction(10.0, 6.0)];
        line[1].w = 300_000;
        line[2].w = 500_000;
        line[2].perimeter_index = 2;
        let out = fuzzed_line(&line, &cfg, None);
        assert_eq!(out[0], line[0]);
        assert_ne!(out[out.len() - 1].p, out[0].p);
        // Not the last junction either: the line ends on a new one,
        // short of it.
        assert_ne!(out[out.len() - 1].p, line[2].p);
        // A new junction has the width and wall number of the junction
        // its segment runs to.
        for junction in &out[1..] {
            let on_first = junction.p.x < scaled(10.0) - cfg.thickness - 2;
            let on_second = junction.p.y > cfg.thickness + 2;
            if on_first {
                assert_eq!((junction.w, junction.perimeter_index), (300_000, 0));
            } else if on_second {
                assert_eq!((junction.w, junction.perimeter_index), (500_000, 2));
            }
        }
        assert!(out.iter().any(|junction| junction.w == 300_000));
        assert!(out.iter().any(|junction| junction.w == 500_000));
    }

    #[test]
    fn short_and_degenerate_lines() {
        let cfg = config();
        for line in [vec![], vec![junction(1.0, 1.0)]] {
            assert_eq!(fuzzed_line(&line, &cfg, None), line);
            let ripple = FuzzyConfig {
                noise: NoiseType::Ripple,
                ..cfg
            };
            assert_eq!(fuzzed_line(&line, &ripple, None), line);
        }
        let stuck = FuzzyConfig {
            point_distance: 0,
            ..cfg
        };
        assert_eq!(fuzzed_line(&square_line(), &stuck, None), square_line());
        // Two junctions too close for three: left alone.
        let stub = vec![junction(0.0, 0.0), junction(0.0001, 0.0)];
        assert_eq!(fuzzed_line(&stub, &cfg, None), stub);
    }

    #[test]
    fn overhang_skip_on_a_line_leaves_unsupported_junctions_alone() {
        let support = [rect(-5.0, -5.0, 10.0, 25.0)];
        let outline = square();
        for mode in [
            FuzzyMode::Displacement,
            FuzzyMode::Extrusion,
            FuzzyMode::Combined,
        ] {
            let cfg = FuzzyConfig {
                mode,
                skip_overhangs: true,
                ..config()
            };
            let free = fuzzed_line(&square_line(), &cfg, None);
            let held = fuzzed_line(&square_line(), &cfg, Some(&support));
            assert_eq!(held[0], held[held.len() - 1], "{mode:?}");
            assert_ne!(held, free, "{mode:?}");
            let mut over_air = 0;
            let mut changed = 0;
            for junction in &held[1..held.len() - 1] {
                if junction.p.x > scaled(10.0) + cfg.thickness {
                    // Neither moved nor made wider or narrower.
                    assert!(outline_distance(junction.p, &outline) <= 1.0, "{mode:?}");
                    assert_eq!(junction.w, 450_000, "{mode:?}");
                    over_air += 1;
                } else if junction.p.x < scaled(9.0)
                    && (junction.w != 450_000 || outline_distance(junction.p, &outline) > 1.0)
                {
                    changed += 1;
                }
            }
            assert!(over_air > 60, "{mode:?}: {over_air}");
            assert!(changed > 60, "{mode:?}: {changed}");
        }
    }

    #[test]
    fn ripple_on_lines() {
        let cfg = ripple_config();
        // Closed: as the polygon, with widths carried along.
        let mut line: Vec<Junction> = ripple_square()
            .iter()
            .map(|&p| Junction {
                p,
                w: 450_000,
                perimeter_index: 1,
            })
            .collect();
        line.push(line[0]);
        let out = fuzzed_line(&line, &cfg, None);
        let polygon = fuzzed(&ripple_square(), true, 0.6, &cfg, None);
        let points: Vec<Point2> = out.iter().map(|junction| junction.p).collect();
        assert_eq!(points, polygon);
        assert!(
            out.iter()
                .all(|junction| junction.w == 450_000 && junction.perimeter_index == 1)
        );

        // Open: its own first and last junction around the new ones.
        let mut open = vec![junction(0.0, 3.0), junction(10.0, 3.0)];
        open[1].w = 320_000;
        let out = fuzzed_line(&open, &cfg, None);
        assert_eq!(out[0], open[0]);
        assert_eq!(out[out.len() - 1], open[1]);
        assert!((21..=22).contains(&out.len()), "{}", out.len());
        assert!(out[1..].iter().all(|junction| junction.w == 320_000));
        let inner: Vec<Point2> = out[1..out.len() - 1]
            .iter()
            .map(|junction| junction.p)
            .collect();
        assert_eq!(
            inner,
            fuzzed(&[mm(0.0, 3.0), mm(10.0, 3.0)], false, 0.6, &cfg, None)
        );

        // Too short for a new junction: as it was.
        let stub = vec![junction(0.0, 3.0), junction(0.2, 3.0)];
        assert_eq!(fuzzed_line(&stub, &cfg, None), stub);
    }
}
