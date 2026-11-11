//! Elephant foot compensation: the first layer is printed a little smaller
//! than modelled, to make up for the way it squashes outward on the bed.
//!
//! A port of the reference's `elephant_foot_compensation`
//! (ElephantFootCompensation.cpp) and of what it calls there:
//! `resample_polygon`, `contour_distance2` and `smooth_compensation_banded`.
//! The offset itself, with a distance of its own for every vertex, is
//! `edge_clip::variable_offset_inner`.
//!
//! In outline: a region's rings are given a point at least every 0.5 mm;
//! at each point the distance across the part to the nearest outline
//! facing it is measured; a point with plenty of material behind it gives
//! up the whole compensation, one on a part no wider than an external wall
//! can spare gives up nothing, and between the two a point gives up half
//! of what the part is wider than that; the amounts are smoothed along
//! the outline, and every point is moved in by its own.
//!
//! Numbers are kept the way the reference keeps them (distances in `f32`,
//! the smoothing in `f32`, new points cut to whole units) so the two agree
//! to the last unit wherever the clipper does. The reference finds the
//! outline near a point through its `EdgeGrid`; the grid here is built
//! differently but hands over a superset of the same segments, and the
//! measure does not depend on which further ones it is shown or in what
//! order.

use std::f64::consts::PI;

use edge_clip::ClipError;
use edge_geom::{Coord, ExPolygon, Point2, Polygon};
use rayon::prelude::*;

/// The reference's `scale_`: millimetres to coordinate units, unrounded.
const fn scale(mm: f64) -> f64 {
    mm / 0.000_001
}

/// `SCALED_EPSILON`.
const SCALED_EPSILON: f64 = scale(1e-4);
/// The furthest apart two neighbouring points of a resampled ring may be.
const RESAMPLE_INTERVAL: f64 = scale(0.5);
/// Corners pushed outward are mitred no further than twice the offset.
const MITER_LIMIT: f64 = 2.0;

/// Shrinks a first layer's regions by up to `compensation` to offset the
/// way the first layer squashes outward on the bed. How much a stretch of
/// outline gives up depends on how wide the part is there: all of it
/// where the part is wider than `min_contour_width` plus twice the
/// compensation, nothing where it is narrower than `min_contour_width`,
/// and in between half of what the part has to spare, so thin features
/// keep the width a wall needs and the offset runs smoothly from one to
/// the other. `min_contour_width` is the external wall's width plus its
/// spacing.
///
/// A port of the reference's `elephant_foot_compensation`
/// (ElephantFootCompensation.cpp), with its thresholds and constants: a
/// region whose bounds are narrower than `min_contour_width` plus twice
/// the compensation, or whose area is under five times that squared, is
/// left alone; so is a region the shrinking would part in two. Each
/// region is worked out by itself, regions side by side.
///
/// Both lengths arrive in whole units here, where the reference carries
/// fractions of one (the sum of two `f32` widths; the compensation of a
/// later layer in `f32`): at most half a nanometre's difference.
pub fn compensate_elephant_foot(
    regions: &[ExPolygon],
    min_contour_width: Coord,
    compensation: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    if compensation <= 0 {
        return Ok(regions.to_vec());
    }
    let (min_contour_width, compensation) = (min_contour_width as f64, compensation as f64);
    let each: Vec<Vec<ExPolygon>> = regions
        .par_iter()
        .map(|region| compensate_region(region, min_contour_width, compensation))
        .collect::<Result<_, _>>()?;
    Ok(each.into_iter().flatten().collect())
}

/// One region, compensated. More than one comes back only where
/// simplifying the region parted it.
fn compensate_region(
    input: &ExPolygon,
    min_contour_width: f64,
    compensation: f64,
) -> Result<Vec<ExPolygon>, ClipError> {
    let min_contour_width_compensated = min_contour_width + 2.0 * compensation;
    if is_tiny(input, min_contour_width_compensated) {
        // The contour is tiny. Don't correct it.
        return Ok(vec![input.clone()]);
    }
    let mut simplified = edge_clip::simplify(std::slice::from_ref(input), SCALED_EPSILON as Coord)?;
    match simplified.len() {
        // Nothing left to shrink: a region thinner than the tolerance.
        0 => Ok(vec![input.clone()]),
        1 => {
            let shrunk = shrink(&simplified[0], min_contour_width, compensation)?;
            Ok(vec![shrunk.unwrap_or_else(|| input.clone())])
        }
        // Not as the reference, on purpose: it takes the first of the
        // pieces (`simplify(SCALED_EPSILON).front()`) and returns that one
        // shrunk as the whole region, which it does because it assumes
        // simplifying by a tenth of a micrometre cannot part a region;
        // where it does, the rest of the region is lost from the layer.
        // Here every piece is treated as the region it has become.
        _ => {
            for piece in &mut simplified {
                if !is_tiny(piece, min_contour_width_compensated)
                    && let Some(shrunk) = shrink(piece, min_contour_width, compensation)?
                {
                    *piece = shrunk;
                }
            }
            Ok(simplified)
        }
    }
}

/// Whether a region is too small to give anything up: the reference's
/// test on the bounds and the area of the region.
fn is_tiny(region: &ExPolygon, min_contour_width_compensated: f64) -> bool {
    let points = &region.contour.points;
    let span = |axis: fn(&Point2) -> Coord| {
        let low = points.iter().map(axis).min().unwrap_or(0);
        let high = points.iter().map(axis).max().unwrap_or(0);
        (high - low) as f64
    };
    let area = region.area_mm2() * scale(1.0) * scale(1.0);
    span(|p| p.x) < min_contour_width_compensated + SCALED_EPSILON
        || span(|p| p.y) < min_contour_width_compensated + SCALED_EPSILON
        || area < min_contour_width_compensated * min_contour_width_compensated * 5.0
}

/// Shrinks a simplified region by the varying offset, or `None` when the
/// result is not one region: the reference's "something went wrong, don't
/// compensate". It is kept as a guard: a part cut in two on the bed is
/// worse than one a little fat.
fn shrink(
    simplified: &ExPolygon,
    min_contour_width: f64,
    compensation: f64,
) -> Result<Option<ExPolygon>, ClipError> {
    let min_contour_width_compensated = min_contour_width + 2.0 * compensation;
    // "Make the search radius a bit larger for the averaging in
    // contour_distance over a fan of rays to work": that measure is gone
    // from the reference, the radius it left behind is still this.
    let search_radius = min_contour_width_compensated + min_contour_width * 0.5;

    let rings: Vec<&Polygon> = simplified.rings().collect();
    let grid = SegmentGrid::new(&rings, (0.7 * search_radius) as Coord);
    let mut seen = Seen::new(&grid);
    let mut resampled = ExPolygon::default();
    let mut deltas: Vec<Vec<f32>> = Vec::with_capacity(rings.len());
    for (index, ring) in rings.iter().enumerate() {
        let (points, along) = resample_polygon(&ring.points, RESAMPLE_INTERVAL);
        let mut distances = contour_distance(
            &grid,
            &mut seen,
            &rings,
            index,
            &points,
            &along,
            compensation,
            search_radius,
        );
        for d in &mut distances {
            // Convert contour width to available compensation distance.
            *d = if f64::from(*d) < min_contour_width {
                0.0
            } else if f64::from(*d) > min_contour_width_compensated {
                -(compensation as f32)
            } else {
                -(*d - min_contour_width as f32) / 2.0
            };
        }
        smooth_compensation_banded(
            &points,
            (0.8 * RESAMPLE_INTERVAL) as f32,
            &mut distances,
            0.3,
            3,
        );
        deltas.push(distances);
        if index == 0 {
            resampled.contour.points = points;
        } else {
            resampled.holes.push(Polygon { points });
        }
    }
    let mut out = edge_clip::variable_offset_inner(&resampled, &deltas, MITER_LIMIT)?;
    Ok(match out.len() {
        1 => out.pop(),
        _ => None,
    })
}

/// How far along a resampled ring its points lie.
struct Along {
    /// For each resampled point, the length of outline from the ring's
    /// last original point round to it (so the last original point itself
    /// has the whole length).
    points: Vec<f64>,
    /// The same for each of the ring's original points.
    sources: Vec<f64>,
}

impl Along {
    /// The length of the whole ring.
    fn end(&self) -> f64 {
        self.points.last().copied().unwrap_or(0.0)
    }
}

/// Adds points along a ring's edges so that no two neighbours are further
/// apart than `dist`, each edge divided evenly. A new point is cut to
/// whole units, as the reference's cast to `coord_t` cuts it.
fn resample_polygon(contour: &[Point2], dist: f64) -> (Vec<Point2>, Along) {
    let mut out = Vec::with_capacity(contour.len());
    let mut along = Along {
        points: Vec::with_capacity(contour.len()),
        sources: Vec::with_capacity(contour.len()),
    };
    if contour.len() <= 2 {
        return (out, along);
    }
    let as_f64 = |p: Point2| (p.x as f64, p.y as f64);
    let mut pt_prev = as_f64(contour[contour.len() - 1]);
    let mut travelled = 0.0;
    for &pt in contour {
        let pt_this = as_f64(pt);
        let v = (pt_this.0 - pt_prev.0, pt_this.1 - pt_prev.1);
        let l = (v.0 * v.0 + v.1 * v.1).sqrt();
        let n = (l / dist).ceil() as usize;
        // Not as the reference, on purpose: it divides by `n` whatever it
        // is, so an edge of no length (two points in one place) makes the
        // step 0 / 0 and every length measured along the ring after it
        // not a number. Such an edge has no step to add.
        let l_step = if n == 0 { 0.0 } else { l / n as f64 };
        for i in 1..n {
            let interpolation_parameter = i as f64 / n as f64;
            out.push(Point2 {
                x: (pt_prev.0 + v.0 * interpolation_parameter) as Coord,
                y: (pt_prev.1 + v.1 * interpolation_parameter) as Coord,
            });
            travelled += l_step;
            along.points.push(travelled);
        }
        out.push(pt);
        travelled += l_step;
        along.points.push(travelled);
        along.sources.push(travelled);
        pt_prev = pt_this;
    }
    (out, along)
}

/// The width of the part at every point of a resampled ring: the distance
/// to the nearest stretch of outline on the inner side of the point, up
/// to `search_radius`. Stretches of the point's own ring count only when
/// they lie a fair way round it: further along than a quarter circle of
/// the compensation, and, the outline between being judged by its length
/// against the gap, not just the far side of a bulge.
///
/// A port of the reference's `contour_distance2`. `rings` are the
/// simplified rings the grid was built from, `contour` is ring
/// `idx_contour` resampled and `along` its lengths.
#[allow(clippy::too_many_arguments)]
fn contour_distance(
    grid: &SegmentGrid,
    seen: &mut Seen,
    rings: &[&Polygon],
    idx_contour: usize,
    contour: &[Point2],
    along: &Along,
    compensation: f64,
    search_radius: f64,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(contour.len());
    if contour.len() <= 2 {
        return out;
    }
    let dist_same_contour_accept = 0.5 * compensation * PI;
    let dist_same_contour_reject = search_radius;
    let param_end = along.end();
    // The reference's `Point(search_radius, search_radius)`.
    let radius = search_radius as Coord;
    let own = &rings[idx_contour].points;
    let cross2 = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
    let between = |from: Point2, to: Point2| ((to.x - from.x) as f64, (to.y - from.y) as f64);
    // Whether `point` lies on the inner side of the corner at the start of
    // segment `i` of the point's own ring, and of segment `i` itself.
    let inside_corner = |i: usize, point: Point2| {
        let pt_prev = own[(i + own.len() - 1) % own.len()];
        let pt_this = own[i];
        let pt_next = own[(i + 1) % own.len()];
        let v1 = between(pt_prev, pt_this);
        let v2 = between(pt_this, pt_next);
        let left_of_v1 = cross2(v1, between(pt_prev, point)) > 0.0;
        let left_of_v2 = cross2(v2, between(pt_this, point)) > 0.0;
        if cross2(v1, v2) > 0.0 {
            left_of_v1 && left_of_v2
        } else {
            left_of_v1 || left_of_v2
        }
    };
    let left_of_segment = |i: usize, point: Point2| {
        let pt_this = own[i];
        let pt_next = own[(i + 1) % own.len()];
        cross2(between(pt_this, pt_next), between(pt_this, point)) > 0.0
    };

    for (idx_point, &point) in contour.iter().enumerate() {
        // Direction into the part at the point, not normalized.
        let dir_inside = {
            let prev = contour[(idx_point + contour.len() - 1) % contour.len()];
            let next = contour[(idx_point + 1) % contour.len()];
            let v1 = between(prev, point);
            let v2 = between(point, next);
            (-v1.1 - v2.1, v1.0 + v2.0)
        };
        let mut found = false;
        let mut distance = f64::MAX;
        seen.next_query();
        grid.visit(point, radius, seen, |ring, ipt| {
            let points = &rings[ring].points;
            let first = points[ipt];
            let second = points[(ipt + 1) % points.len()];
            let v = between(first, second);
            let va = between(first, point);
            let l2 = v.0 * v.0 + v.1 * v.1;
            let t = if l2 == 0.0 {
                0.0
            } else {
                ((va.0 * v.0 + va.1 * v.1) / l2).clamp(0.0, 1.0)
            };
            // Closest point of the segment to the point, and the way to it.
            let foot = (first.x as f64 + t * v.0, first.y as f64 + t * v.1);
            let bisector = (foot.0 - point.x as f64, foot.1 - point.y as f64);
            let dist = (bisector.0 * bisector.0 + bisector.1 * bisector.1).sqrt();
            if !((!found || dist < distance)
                && dir_inside.0 * bisector.0 + dir_inside.1 * bisector.1 > 0.0)
            {
                return;
            }
            let mut accept = true;
            if ring == idx_contour {
                // The closest segment is of the point's own ring. Reject
                // it unless the way round the ring to it is long compared
                // with the way straight across.
                let mut param_lo = along.points[idx_point];
                let mut param_hi = t * l2.sqrt();
                if ipt + 1 < points.len() {
                    param_hi += along.sources[ipt];
                }
                if param_lo > param_hi {
                    std::mem::swap(&mut param_lo, &mut param_hi);
                }
                let dist_along_contour = (param_hi - param_lo).min(param_lo + param_end - param_hi);
                if dist_along_contour < dist_same_contour_accept {
                    accept = false;
                } else if dist < dist_same_contour_reject + SCALED_EPSILON {
                    // The point is close to the foot. It is accepted only
                    // if the outline between them does not just bulge
                    // away from the straight line: the bulge is taken as
                    // 0.6 of a circle drawn on that line.
                    let inside = if t == 0.0 {
                        inside_corner(ipt, point)
                    } else if t == 1.0 {
                        inside_corner((ipt + 1) % points.len(), point)
                    } else {
                        left_of_segment(ipt, point)
                    };
                    accept = inside && dist_along_contour > 0.6 * PI * dist;
                }
            }
            if accept {
                distance = dist;
                found = true;
            }
        });
        out.push(if found {
            distance.min(search_radius)
        } else {
            search_radius
        } as f32);
    }
    out
}

/// Evens the compensations out along a ring: each moves `strength` of the
/// way toward the mean of the values `band` away along the outline on
/// either side, but only ever toward less compensation. All in `f32`, as
/// the reference's `smooth_compensation_banded`.
fn smooth_compensation_banded(
    contour: &[Point2],
    band: f32,
    compensation: &mut Vec<f32>,
    strength: f32,
    num_iterations: usize,
) {
    let n = contour.len();
    if n <= 2 || compensation.len() != n {
        return;
    }
    let at = |i: usize| (contour[i].x as f32, contour[i].y as f32);
    let squared = |a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (a.0 - b.0, a.1 - b.1);
        dx * dx + dy * dy
    };
    let lerp = |a: f32, b: f32, t: f32| (1.0 - t) * a + t * b;
    let dist_min2 = band * band;
    let mut out = compensation.clone();
    for _ in 0..num_iterations {
        for i in 0..n {
            let pthis = at(i);
            // The value `band` away, walking the ring one way.
            let reach = |step: &dyn Fn(usize) -> usize| {
                let mut j = step(i);
                let mut pprev = at(j);
                let mut value = compensation[j];
                let l2 = squared(pthis, pprev);
                if l2 < dist_min2 {
                    let mut l = l2.sqrt();
                    let mut jprev = j;
                    j = step(j);
                    while j != i {
                        let pp = at(j);
                        let lthis = squared(pp, pprev).sqrt();
                        let lnext = l + lthis;
                        if lnext > band {
                            // Interpolate the compensation value.
                            value = lerp(compensation[jprev], compensation[j], (band - l) / lthis);
                            break;
                        }
                        value = compensation[j];
                        pprev = pp;
                        l = lnext;
                        jprev = j;
                        j = step(j);
                    }
                }
                value
            };
            let prev = reach(&|j| (j + n - 1) % n);
            let next = reach(&|j| (j + 1) % n);
            let laplacian = compensation[i] * (1.0 - strength) + 0.5 * strength * (prev + next);
            // Compensations are negative. Only apply the laplacian if it
            // leads to lower compensation.
            out[i] = if laplacian < compensation[i] {
                compensation[i]
            } else {
                laplacian
            };
        }
        std::mem::swap(&mut out, compensation);
    }
}

/// The segments of a region's rings, sorted into square cells so those
/// near a point are found without looking at all of them.
///
/// It stands in for the reference's `EdgeGrid`. A segment is listed in
/// every cell it passes through and, to be on the safe side of rounding,
/// in some it only passes close to.
struct SegmentGrid {
    min: Point2,
    cell: Coord,
    cols: i64,
    rows: i64,
    /// Where each cell's segments start in `items`, and one past the last.
    starts: Vec<u32>,
    /// Indices into `segments`, cell by cell.
    items: Vec<u32>,
    /// (ring, index of the segment's first point).
    segments: Vec<(u32, u32)>,
}

/// Which segments a query has been shown already: one lies in several cells.
struct Seen {
    stamps: Vec<u32>,
    query: u32,
}

impl Seen {
    fn new(grid: &SegmentGrid) -> Self {
        Seen {
            stamps: vec![0; grid.segments.len()],
            query: 0,
        }
    }

    fn next_query(&mut self) {
        self.query = self.query.wrapping_add(1);
        if self.query == 0 {
            self.stamps.fill(0);
            self.query = 1;
        }
    }
}

impl SegmentGrid {
    /// No more cells than this, however small the cell asked for: larger
    /// cells find the same segments, only more slowly.
    const MOST_CELLS: i64 = 1 << 22;

    fn new(rings: &[&Polygon], cell: Coord) -> Self {
        let all = || rings.iter().flat_map(|ring| ring.points.iter());
        let low = |axis: fn(&Point2) -> Coord| all().map(axis).min().unwrap_or(0) - 1;
        let high = |axis: fn(&Point2) -> Coord| all().map(axis).max().unwrap_or(0) + 1;
        let min = Point2 {
            x: low(|p| p.x),
            y: low(|p| p.y),
        };
        let (width, height) = (high(|p| p.x) - min.x, high(|p| p.y) - min.y);
        let mut cell = cell.max(1);
        while (width / cell + 1).saturating_mul(height / cell + 1) > Self::MOST_CELLS {
            cell = cell.saturating_mul(2);
        }
        let (cols, rows) = (width / cell + 1, height / cell + 1);

        let mut segments = Vec::new();
        // (cell, segment), in the order found, then sorted by cell.
        let mut listed: Vec<(u32, u32)> = Vec::new();
        for (ring, polygon) in rings.iter().enumerate() {
            let points = &polygon.points;
            for (index, &a) in points.iter().enumerate() {
                let b = points[(index + 1) % points.len()];
                let segment = segments.len() as u32;
                segments.push((ring as u32, index as u32));
                let row_of = |y: Coord| ((y - min.y) / cell).clamp(0, rows - 1);
                let col_of =
                    |x: f64| (((x - min.x as f64) / cell as f64).floor() as i64).clamp(0, cols - 1);
                for row in row_of(a.y.min(b.y))..=row_of(a.y.max(b.y)) {
                    // The stretch of the segment within this row of cells.
                    let (x0, x1) = if a.y == b.y {
                        (a.x as f64, b.x as f64)
                    } else {
                        let x_at = |y: Coord| {
                            let t = ((y - a.y) as f64 / (b.y - a.y) as f64).clamp(0.0, 1.0);
                            a.x as f64 + t * (b.x - a.x) as f64
                        };
                        (x_at(min.y + row * cell), x_at(min.y + (row + 1) * cell))
                    };
                    for col in col_of(x0.min(x1) - 1.0)..=col_of(x0.max(x1) + 1.0) {
                        listed.push(((row * cols + col) as u32, segment));
                    }
                }
            }
        }
        listed.sort_unstable();
        let mut starts = vec![0u32; (cols * rows) as usize + 1];
        for &(cell, _) in &listed {
            starts[cell as usize + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        SegmentGrid {
            min,
            cell,
            cols,
            rows,
            starts,
            items: listed.into_iter().map(|(_, segment)| segment).collect(),
            segments,
        }
    }

    /// Shows `visitor` every segment in the cells the square of `radius`
    /// about `point` touches, each once, as (ring, segment).
    fn visit(
        &self,
        point: Point2,
        radius: Coord,
        seen: &mut Seen,
        mut visitor: impl FnMut(usize, usize),
    ) {
        let col = |x: Coord| {
            (x - self.min.x)
                .div_euclid(self.cell)
                .clamp(0, self.cols - 1)
        };
        let row = |y: Coord| {
            (y - self.min.y)
                .div_euclid(self.cell)
                .clamp(0, self.rows - 1)
        };
        for row in row(point.y - radius)..=row(point.y + radius) {
            for col in col(point.x - radius)..=col(point.x + radius) {
                let cell = (row * self.cols + col) as usize;
                let range = self.starts[cell] as usize..self.starts[cell + 1] as usize;
                for &segment in &self.items[range] {
                    if std::mem::replace(&mut seen.stamps[segment as usize], seen.query)
                        != seen.query
                    {
                        let (ring, index) = self.segments[segment as usize];
                        visitor(ring as usize, index as usize);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use edge_geom::{scaled, unscaled};

    use super::*;

    /// The external wall of a 0.4 mm nozzle's first layer: width plus spacing.
    const MIN_WIDTH: f64 = 0.857;
    const COMPENSATION: f64 = 0.15;

    fn compensate(regions: &[ExPolygon]) -> Vec<ExPolygon> {
        compensate_elephant_foot(regions, scaled(MIN_WIDTH), scaled(COMPENSATION)).unwrap()
    }

    fn ring(points: &[(f64, f64)]) -> Polygon {
        Polygon {
            points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
        }
    }

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: ring(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]),
            holes: Vec::new(),
        }
    }

    /// Lowest and highest x and y of the regions' contours, in mm.
    fn bounds(regions: &[ExPolygon]) -> (f64, f64, f64, f64) {
        let points = || regions.iter().flat_map(|region| &region.contour.points);
        let low = |axis: fn(&Point2) -> Coord| unscaled(points().map(axis).min().unwrap());
        let high = |axis: fn(&Point2) -> Coord| unscaled(points().map(axis).max().unwrap());
        (low(|p| p.x), low(|p| p.y), high(|p| p.x), high(|p| p.y))
    }

    /// What of the regions lies on the line x = `x`, as (low y, high y)
    /// stretches in mm, lowest first.
    fn across(regions: &[ExPolygon], x: f64) -> Vec<(f64, f64)> {
        let slab = [block(x - 0.0005, -1000.0, x + 0.0005, 1000.0)];
        let mut cut: Vec<(f64, f64)> = edge_clip::intersection(regions, &slab)
            .unwrap()
            .iter()
            .map(|piece| {
                let (_, low, _, high) = bounds(std::slice::from_ref(piece));
                (low, high)
            })
            .collect();
        cut.sort_by(|a, b| a.0.total_cmp(&b.0));
        cut
    }

    /// The width of the one stretch of the regions on the line x = `x`.
    fn width_at(regions: &[ExPolygon], x: f64) -> f64 {
        let cut = across(regions, x);
        assert_eq!(cut.len(), 1, "one stretch at x = {x}: {cut:?}");
        cut[0].1 - cut[0].0
    }

    #[test]
    fn a_wide_region_shrinks_by_the_whole_compensation() {
        let shrunk = compensate(&[block(0.0, 0.0, 20.0, 10.0)]);
        assert_eq!(shrunk.len(), 1);
        assert_eq!(bounds(&shrunk), (0.15, 0.15, 19.85, 9.85));
        // The points added along the sides are gone again: four corners.
        assert_eq!(shrunk[0].contour.points.len(), 4);
        // Nothing to do, and regions too small to spare any are left alone.
        let same = [block(0.0, 0.0, 20.0, 10.0)];
        assert_eq!(
            compensate_elephant_foot(&same, scaled(MIN_WIDTH), 0).unwrap(),
            same
        );
        let small = [block(0.0, 0.0, 2.0, 2.0)];
        assert_eq!(compensate(&small), small);
    }

    #[test]
    fn a_thin_strip_keeps_its_width() {
        // Narrower than a wall and its spacing: nothing to spare.
        let strip = [block(0.0, 0.0, 30.0, 0.8)];
        assert_eq!(compensate(&strip), strip);

        // A 20 x 10 body with a fin 0.6 mm wide and 8 mm long on its right side.
        let body = edge_clip::union_ex(&[block(0.0, 0.0, 20.0, 10.0), block(20.0, 4.7, 28.0, 5.3)])
            .unwrap();
        let shaped = compensate(&body);
        assert_eq!(shaped.len(), 1, "the fin stays attached");
        // The body gives up 0.15 mm on its three free sides.
        let (left, bottom, right, top) = bounds(&shaped);
        assert_eq!((left, bottom, top), (0.15, 0.15, 9.85));
        assert_eq!(across(&shaped, 10.0), [(0.15, 9.85)]);
        // The fin is as wide as modelled from its root to half a
        // millimetre short of its end.
        for x in [20.1, 21.0, 24.0, 27.0, 27.5] {
            assert_eq!(across(&shaped, x), [(4.7, 5.3)], "at x = {x}");
        }
        // Its end has nothing but the bed in front of it and is drawn
        // back, by less than the whole compensation: the amount is evened
        // out with the sides', which give up nothing.
        assert!(
            right < 28.0 - 0.05 && right > 28.0 - COMPENSATION,
            "{right}"
        );
        // The body's side runs out to meet the fin where it is rooted,
        // with no step between the two.
        let root = width_at(&shaped, 19.95);
        assert!(root > 0.6 && root < 2.0, "{root}");
    }

    #[test]
    fn a_narrowing_part_goes_smoothly_from_full_to_none() {
        // 3 mm wide at the left, 0.5 mm at the right, 40 mm long.
        let wedge = [ExPolygon {
            contour: ring(&[(0.0, -1.5), (40.0, -0.25), (40.0, 0.25), (0.0, 1.5)]),
            holes: Vec::new(),
        }];
        let shaped = compensate(&wedge);
        assert_eq!(shaped.len(), 1);
        let mut before = (f64::MAX, 2.0 * COMPENSATION);
        for step in 1..79 {
            let x = step as f64 * 0.5;
            let (modelled, printed) = (width_at(&wedge, x), width_at(&shaped, x));
            let given_up = modelled - printed;
            // All of it while the part is wider than a wall can spare,
            // none once it is no wider than a wall, and between the two
            // what leaves the part exactly a wall wide.
            let expected = (modelled - MIN_WIDTH).clamp(0.0, 2.0 * COMPENSATION);
            assert!(
                (given_up - expected).abs() < 0.02,
                "at x = {x} the part gives up {given_up}, not {expected}"
            );
            // Smoothing only ever lessens what is given up.
            assert!(given_up < expected + 0.005, "at x = {x}: {given_up}");
            // No notch: the printed width never grows along the taper, and
            // what is given up never changes by more than the taper does.
            assert!(printed < before.0 + 1e-6, "wider at x = {x}");
            assert!(
                (given_up - before.1).abs() < 0.035,
                "a step at x = {x}: {} to {given_up}",
                before.1
            );
            before = (printed, given_up);
        }
        assert!(before.1.abs() < 1e-6, "the thin end is as modelled");
    }

    #[test]
    fn holes_grow_except_where_the_wall_beside_them_is_thin() {
        // A 20 mm plate with a hole whose left side is 0.7 mm from the
        // plate's edge and whose other sides have plenty round them.
        let plate = [ExPolygon {
            contour: ring(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)]),
            holes: vec![ring(&[(0.7, 5.0), (0.7, 15.0), (8.0, 15.0), (8.0, 5.0)])],
        }];
        let shaped = compensate(&plate);
        assert_eq!((shaped.len(), shaped[0].holes.len()), (1, 1));
        assert!(shaped[0].contour.is_ccw() && !shaped[0].holes[0].is_ccw());
        // Through the middle of the hole: the thin wall on the left is
        // as modelled, the hole's right side has moved out and the
        // plate's right edge in.
        let hole = &shaped[0].holes[0].points;
        let reach = |axis: fn(&Point2) -> Coord| {
            (
                unscaled(hole.iter().map(axis).min().unwrap()),
                unscaled(hole.iter().map(axis).max().unwrap()),
            )
        };
        assert_eq!(reach(|p| p.x), (0.7, 8.15));
        assert_eq!(reach(|p| p.y), (4.85, 15.15));
        let (left, bottom, right, top) = bounds(&shaped);
        assert_eq!((left, bottom, right, top), (0.0, 0.15, 19.85, 19.85));
        // Along the plate at the middle of the hole's height: wall, hole, plate.
        let slab = [block(-1.0, 9.9995, 21.0, 10.0005)];
        let mut cut: Vec<(f64, f64)> = edge_clip::intersection(&shaped, &slab)
            .unwrap()
            .iter()
            .map(|piece| {
                let (low, _, high, _) = bounds(std::slice::from_ref(piece));
                (low, high)
            })
            .collect();
        cut.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(cut, [(0.0, 0.7), (8.15, 19.85)]);
        // The plate's left edge stays where it was only beside the hole,
        // and comes in by the whole amount within a millimetre or so of
        // its ends.
        let kept = across(&shaped, 0.1);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(kept[0].0 > 3.5 && kept[0].0 < 5.0, "{kept:?}");
        assert!(kept[0].1 > 15.0 && kept[0].1 < 16.5, "{kept:?}");
    }

    #[test]
    fn each_region_is_shrunk_by_itself_and_stays_in_its_place() {
        let regions = [
            block(0.0, 0.0, 20.0, 10.0),
            block(30.0, 0.0, 32.0, 2.0),
            block(40.0, 0.0, 70.0, 0.8),
            block(0.0, 20.0, 5.0, 25.0),
        ];
        let shaped = compensate(&regions);
        assert_eq!(shaped.len(), 4);
        assert_eq!(bounds(&shaped[..1]), (0.15, 0.15, 19.85, 9.85));
        // Too small, and too thin, to give anything up.
        assert_eq!(shaped[1..3], regions[1..3]);
        assert_eq!(bounds(&shaped[3..]), (0.15, 20.15, 4.85, 24.85));
        // A region shrinks as it would alone, whatever stands beside it.
        assert_eq!(shaped[3..], compensate(&regions[3..]));
    }

    /// A ring with lobes, a thin spoke and a slot: wide parts, thin
    /// parts and everything between.
    fn cog() -> ExPolygon {
        let mut points = Vec::new();
        for step in 0..360 {
            let angle = f64::from(step).to_radians();
            let radius = 12.0 + 3.0 * (5.0 * angle).sin() + 0.8 * (17.0 * angle).cos();
            points.push((radius * angle.cos(), radius * angle.sin()));
        }
        let lobed = ExPolygon {
            contour: ring(&points),
            holes: vec![ring(&[(-4.0, -4.0), (-4.0, 4.0), (4.0, 4.0), (4.0, -4.0)])],
        };
        let spoke = block(10.0, -0.35, 24.0, 0.35);
        let slot = block(-20.0, -0.3, -6.0, 0.3);
        let whole = edge_clip::union_ex(&[lobed, spoke]).unwrap();
        let mut cut = edge_clip::difference(&whole, &[slot]).unwrap();
        assert_eq!(cut.len(), 1);
        cut.remove(0)
    }

    #[test]
    fn output_does_not_depend_on_thread_count() {
        let mut regions = vec![cog(), block(40.0, 0.0, 60.0, 10.0)];
        for copy in 0..6 {
            let mut moved = cog();
            for point in moved
                .contour
                .points
                .iter_mut()
                .chain(moved.holes.iter_mut().flat_map(|hole| &mut hole.points))
            {
                point.y += scaled(40.0 * f64::from(copy + 1));
            }
            regions.push(moved);
        }
        let run = |threads| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| compensate(&regions))
        };
        let once = run(1);
        assert_eq!(once, run(8));
        assert_eq!(once, run(1));
        // The cog gave something up, and came out as one region.
        assert_eq!(once.len(), regions.len());
        assert!(once[0].area_mm2() < regions[0].area_mm2() - 5.0);
        let outline: f64 = regions[0].rings().map(Polygon::perimeter_mm).sum();
        assert!(once[0].area_mm2() > regions[0].area_mm2() - 1.1 * COMPENSATION * outline);
    }

    #[test]
    fn the_width_measured_does_not_depend_on_the_grid() {
        // The same distances whether the segments are sorted into small
        // cells or all lie in one.
        let cog = cog();
        let rings: Vec<&Polygon> = cog.rings().collect();
        let (compensation, search_radius) = (150_000.0, 1_585_500.0);
        let fine = SegmentGrid::new(&rings, (0.7 * search_radius) as Coord);
        let coarse = SegmentGrid::new(&rings, scaled(1000.0));
        assert!(fine.cols * fine.rows > 100);
        assert_eq!(coarse.cols * coarse.rows, 1);
        let mut thin = 0;
        for (index, ring) in rings.iter().enumerate() {
            let (points, along) = resample_polygon(&ring.points, RESAMPLE_INTERVAL);
            let measure = |grid: &SegmentGrid| {
                contour_distance(
                    grid,
                    &mut Seen::new(grid),
                    &rings,
                    index,
                    &points,
                    &along,
                    compensation,
                    search_radius,
                )
            };
            let distances = measure(&fine);
            assert_eq!(distances, measure(&coarse));
            assert_eq!(distances.len(), points.len());
            thin += distances.iter().filter(|d| **d < 857_000.0).count();
        }
        assert!(thin > 30, "the spoke is thin: {thin}");
    }

    #[test]
    fn resampling_adds_evenly_spaced_points_cut_to_whole_units() {
        // A 1.2 x 0.3 mm rectangle: the long sides are cut in three, the
        // short ones left whole.
        let contour = ring(&[(0.0, 0.0), (1.2, 0.0), (1.2, 0.3), (0.0, 0.3)]).points;
        let (points, along) = resample_polygon(&contour, RESAMPLE_INTERVAL);
        let mm: Vec<(f64, f64)> = points
            .iter()
            .map(|p| (unscaled(p.x), unscaled(p.y)))
            .collect();
        // The edge that closes the ring comes first, as in the reference.
        assert_eq!(
            mm,
            [
                (0.0, 0.0),
                (0.4, 0.0),
                (0.8, 0.0),
                (1.2, 0.0),
                (1.2, 0.3),
                (0.8, 0.3),
                (0.4, 0.3),
                (0.0, 0.3),
            ]
        );
        // Lengths run from the ring's last point round to each point.
        let lengths: Vec<f64> = along
            .points
            .iter()
            .map(|l| (l / 1e3).round() / 1e3)
            .collect();
        assert_eq!(lengths, [0.3, 0.7, 1.1, 1.5, 1.8, 2.2, 2.6, 3.0]);
        let sources: Vec<f64> = along
            .sources
            .iter()
            .map(|l| (l / 1e3).round() / 1e3)
            .collect();
        assert_eq!(sources, [0.3, 1.5, 1.8, 3.0]);
        // A new point is cut, not rounded: a third of 1 000 001 units.
        let odd = [
            Point2 { x: 0, y: 0 },
            Point2 { x: 1_000_001, y: 0 },
            Point2 { x: 0, y: 400_000 },
        ];
        let (points, _) = resample_polygon(&odd, RESAMPLE_INTERVAL);
        assert!(points.contains(&Point2 { x: 333_333, y: 0 }));
        assert!(points.contains(&Point2 { x: 666_667, y: 0 }));
        // Two points in one place add no length and no points.
        let doubled = [odd[0], odd[0], odd[1], odd[2]];
        let (points, along) = resample_polygon(&doubled, RESAMPLE_INTERVAL);
        assert!(along.points.iter().all(|l| l.is_finite()));
        assert_eq!(points.iter().filter(|p| **p == odd[0]).count(), 2);
        // Too few points to be a ring.
        assert!(resample_polygon(&odd[..2], RESAMPLE_INTERVAL).0.is_empty());
    }

    #[test]
    fn smoothing_only_ever_lessens_the_compensation() {
        // Points 0.25 mm apart round a 5 mm square; one stretch gives up
        // nothing, the rest everything.
        let (contour, _) = resample_polygon(&block(0.0, 0.0, 5.0, 5.0).contour.points, 250_000.0);
        let sharp: Vec<f32> = (0..contour.len())
            .map(|i| {
                if (20..30).contains(&i) {
                    0.0
                } else {
                    -150_000.0
                }
            })
            .collect();
        let mut smooth = sharp.clone();
        smooth_compensation_banded(&contour, 400_000.0, &mut smooth, 0.3, 3);
        assert!(
            smooth
                .iter()
                .zip(&sharp)
                .all(|(after, before)| after >= before)
        );
        assert!(
            smooth
                .iter()
                .all(|value| (-150_000.0..=0.0).contains(value))
        );
        // The step is spread over the points either side of it, and
        // nothing far from it has moved.
        assert!(smooth[19] > -150_000.0 && smooth[19] < 0.0);
        assert!(smooth[18] > -150_000.0 && smooth[18] < smooth[19]);
        assert_eq!(
            (smooth[25], smooth[5], smooth[40]),
            (0.0, -150_000.0, -150_000.0)
        );
    }
}
