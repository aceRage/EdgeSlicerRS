//! Running a bridge over infill out to the sparse lines that carry it, and
//! choosing which way it runs: the two helpers of the reference's
//! `PrintObject::bridge_over_infill` (PrintObject.cpp, the lambdas
//! `determine_bridging_angle` and `construct_anchored_polygon`), with the
//! parts of `AABBTreeLines.hpp` they lean on.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

/// A straight piece of a line or of an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Segment {
    pub a: Point2,
    pub b: Point2,
}

impl Segment {
    /// Its direction from `a` to `b`, from 0 up to 2π (Line.cpp
    /// `Line::orientation`).
    fn orientation(&self) -> f64 {
        let angle = ((self.b.y - self.a.y) as f64).atan2((self.b.x - self.a.x) as f64);
        if angle < 0.0 { 2.0 * PI + angle } else { angle }
    }
}

/// The pieces of open lines (`to_lines(Polylines)`).
pub(crate) fn segments_of_lines(lines: &[Vec<Point2>]) -> Vec<Segment> {
    lines
        .iter()
        .flat_map(|line| {
            line.windows(2).map(|pair| Segment {
                a: pair[0],
                b: pair[1],
            })
        })
        .collect()
}

/// The edges of closed rings, the closing edge included
/// (`to_lines(Polygons)`).
pub(crate) fn segments_of_rings<'a>(rings: impl IntoIterator<Item = &'a Polygon>) -> Vec<Segment> {
    let mut out = Vec::new();
    for ring in rings {
        let points = &ring.points;
        for (index, a) in points.iter().enumerate() {
            out.push(Segment {
                a: *a,
                b: points[(index + 1) % points.len()],
            });
        }
    }
    out
}

/// Every ring of the regions as a closed line, its first point repeated
/// at its end (`to_polylines(Polygons)`).
pub(crate) fn closed_lines(regions: &[ExPolygon]) -> Vec<Vec<Point2>> {
    regions
        .iter()
        .flat_map(ExPolygon::rings)
        .filter(|ring| !ring.points.is_empty())
        .map(|ring| {
            let mut line = ring.points.clone();
            line.push(ring.points[0]);
            line
        })
        .collect()
}

/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// Where two segments cross, if they do (Line.hpp
/// `line_alg::intersection`): parallel ones never do, and the point is
/// cut to whole units toward zero.
fn crossing(l1: &Segment, l2: &Segment) -> Option<Point2> {
    let v1 = ((l1.b.x - l1.a.x) as f64, (l1.b.y - l1.a.y) as f64);
    let v2 = ((l2.b.x - l2.a.x) as f64, (l2.b.y - l2.a.y) as f64);
    let denom = v1.0 * v2.1 - v1.1 * v2.0;
    if denom.abs() < EPSILON {
        return None;
    }
    let v12 = ((l1.a.x - l2.a.x) as f64, (l1.a.y - l2.a.y) as f64);
    let t1 = (v2.0 * v12.1 - v2.1 * v12.0) / denom;
    let t2 = (v1.0 * v12.1 - v1.1 * v12.0) / denom;
    if (0.0..=1.0).contains(&t1) && (0.0..=1.0).contains(&t2) {
        Some(Point2 {
            x: (l1.a.x as f64 + t1 * v1.0) as Coord,
            y: (l1.a.y as f64 + t1 * v1.1) as Coord,
        })
    } else {
        None
    }
}

/// How many of the segments a ray from `origin` along one axis meets on
/// either side, or none where it runs along one or starts on one
/// (AABBTreeLines.hpp `coordinate_aligned_ray_hit_count`). `along` is 0
/// for x, 1 for y.
fn hits(segments: &[Segment], origin: Point2, along: usize) -> Option<(usize, usize)> {
    let get = |p: Point2, axis: usize| if axis == 0 { p.x } else { p.y };
    let other = 1 - along;
    let (mut above, mut below) = (0, 0);
    for line in segments {
        let (oa, ob) = (get(line.a, other), get(line.b, other));
        let at = get(origin, other);
        // The second test is not sharp, so that two segments meeting on
        // the ray count once.
        if at < oa.min(ob) || at >= oa.max(ob) {
            continue;
        }
        let (ca, cb) = (get(line.a, along), get(line.b, along));
        let value = get(origin, along);
        if value > ca.max(cb) {
            above += 1;
        } else if value < ca.min(cb) {
            below += 1;
        } else {
            let t = (at - oa) as f64 / (ob - oa) as f64;
            let crossed = ca as f64 + t * (cb - ca) as f64;
            let value = value as f64;
            if value > crossed {
                above += 1;
            } else if value < crossed {
                below += 1;
            } else {
                return None;
            }
        }
    }
    Some((above, below))
}

/// Whether a point lies outside closed outlines: 1 outside, -1 inside, 0
/// on one or not to be told (AABBTreeLines.hpp
/// `point_outside_closed_contours`).
fn outside(segments: &[Segment], point: Point2) -> i32 {
    if segments.is_empty() {
        return 1;
    }
    for along in [0, 1] {
        match hits(segments, point, along) {
            None => return 0,
            Some((above, below)) if above % 2 == 1 && below % 2 == 1 => return -1,
            Some((above, below)) if above % 2 == 0 && below % 2 == 0 => return 1,
            Some(_) => {}
        }
    }
    0
}

/// The segment nearest a point; the first of equals.
fn nearest(segments: &[Segment], point: (f64, f64)) -> Option<&Segment> {
    let mut best: Option<(&Segment, f64)> = None;
    for segment in segments {
        let (ax, ay) = (segment.a.x as f64, segment.a.y as f64);
        let (dx, dy) = (segment.b.x as f64 - ax, segment.b.y as f64 - ay);
        let length = dx * dx + dy * dy;
        let t = if length > 0.0 {
            (((point.0 - ax) * dx + (point.1 - ay) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (ex, ey) = (ax + t * dx - point.0, ay + t * dy - point.1);
        let distance = ex * ex + ey * ey;
        if best.is_none_or(|(_, least)| distance < least) {
            best = Some((segment, distance));
        }
    }
    best.map(|(segment, _)| segment)
}

/// The way a bridge over infill is to run, in radians, chosen from the
/// lines round it (PrintObject.cpp `determine_bridging_angle`): every
/// 2 mm or so along the bridge's outline the nearest of `anchors` votes
/// for running square to itself, and the direction with the most votes
/// within a tenth of a half turn either side wins, averaged over those.
///
/// Different from the reference, which means to count the votes near the
/// far end of its range together with those near the near end, as one
/// direction, and does it for a window that reaches below the range but
/// not for one that reaches above it (its test for the second asks for a
/// window that starts above the range's end, which none does, so the
/// code is never run). Here both ends are taken round. Where the votes
/// lie away from those ends, which is nearly always, it makes no
/// difference.
pub(crate) fn bridging_angle(bridged: &[ExPolygon], anchors: &[Segment]) -> f64 {
    let step = edge_geom::scaled(2.0) as f64;
    // Votes by direction; every direction is from a half up to one and a
    // half turns, so its bits order as the numbers do.
    let mut counted: BTreeMap<u64, usize> = BTreeMap::new();
    if !anchors.is_empty() {
        for ring in bridged.iter().flat_map(ExPolygon::rings) {
            let points = &as_the_reference_starts(&ring.points);
            let mut travelled = 0.0;
            // Edge by edge, the edge back to the first point left out.
            for pair in points.windows(2) {
                let start = (pair[0].x as f64, pair[0].y as f64);
                let v = (pair[1].x as f64 - start.0, pair[1].y as f64 - start.1);
                let length = v.0.hypot(v.1);
                travelled += length;
                if travelled <= step {
                    continue;
                }
                travelled = 0.0;
                let unit = (v.0 / length, v.1 / length);
                let count = (length / step).ceil() as usize;
                let size = (length / count as f64) as f32 as f64;
                for i in 0..count {
                    let at = Point2 {
                        x: (start.0 + unit.0 * (i as f64 * size)) as Coord,
                        y: (start.1 + unit.1 * (i as f64 * size)) as Coord,
                    };
                    let Some(line) = nearest(anchors, (at.x as f64, at.y as f64)) else {
                        continue;
                    };
                    let mut angle = line.orientation();
                    if angle > PI {
                        angle -= PI;
                    }
                    angle += 0.5 * PI;
                    *counted.entry(angle.to_bits()).or_insert(0) += 1;
                }
            }
        }
    }
    let counted: Vec<(f64, usize)> = counted
        .into_iter()
        .map(|(bits, count)| (f64::from_bits(bits), count))
        .collect();
    let mut best = (0.0, 0usize);
    for &(direction, _) in &counted {
        let (start, end) = (direction - PI * 0.1, direction + PI * 0.1);
        let (mut sum, mut score) = (0.0, 0usize);
        for &(angle, count) in &counted {
            if angle >= start && angle <= end {
                sum += angle * count as f64;
                score += count;
            }
        }
        // The range is a half to one and a half turns: a window that
        // reaches past either end takes in the directions at the other,
        // as the same.
        if start < 0.5 * PI {
            for &(angle, count) in counted.iter().filter(|(angle, _)| *angle >= start + PI) {
                sum += (angle - PI) * count as f64;
                score += count;
            }
        }
        if end > 1.5 * PI {
            for &(angle, count) in counted.iter().filter(|(angle, _)| *angle <= end - PI) {
                sum += (angle + PI) * count as f64;
                score += count;
            }
        }
        if score > best.1 {
            best = (sum / score as f64, score);
        }
    }
    if best.0 == 0.0 { 0.001 } else { best.0 }
}

/// A ring started where the reference's clipping library (Clipper 6)
/// starts it: just after its lowest point, the rightmost of equals. The
/// vote in [`bridging_angle`] leaves out the edge back to the first point
/// and only counts an edge once 2 mm have gone by since the last it
/// counted, so where a ring starts can tip it. (Found by running that
/// library's own code on simple shapes; where it has split a shape in
/// two it may start a piece elsewhere.)
fn as_the_reference_starts(points: &[Point2]) -> Vec<Point2> {
    let Some(lowest) = (0..points.len()).min_by_key(|&i| (points[i].y, -points[i].x)) else {
        return Vec::new();
    };
    let first = (lowest + 1) % points.len();
    points[first..]
        .iter()
        .chain(&points[..first])
        .copied()
        .collect()
}

/// Turns a point about the origin, to whole units (MultiPoint.cpp
/// `MultiPoint::rotate`).
fn turn(p: Point2, cos: f64, sin: f64) -> Point2 {
    let (x, y) = (p.x as f64, p.y as f64);
    Point2 {
        x: (cos * x - sin * y).round() as Coord,
        y: (cos * y + sin * x).round() as Coord,
    }
}

/// One stretch of a scan line across the area, from `a` up to `b`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Section {
    a: Point2,
    b: Point2,
}

/// The rings an anchored bridge is traced into as the scan moves along.
struct Traced {
    lows: Vec<Point2>,
    highs: Vec<Point2>,
}

impl Traced {
    fn close(mut self) -> Polygon {
        self.lows.extend(self.highs.into_iter().rev());
        Polygon { points: self.lows }
    }
}

/// The area a bridge over infill covers once each of its lines is run on,
/// the way the bridge goes, to the first of `anchors` beyond either end
/// and a line's `width` past it (PrintObject.cpp
/// `construct_anchored_polygon`). The area is scanned along the bridge's
/// direction every `scan`; each stretch of a scan line that crosses it is
/// run out, stretches that come to overlap are made one, and the
/// stretches of neighbouring scan lines are traced into outlines, a step
/// out sideways where one ends or jumps more than six `spacing`s.
pub(crate) fn anchored(
    bridged: &[ExPolygon],
    anchors: &[Segment],
    spacing: Coord,
    width: Coord,
    angle: f64,
    scan: Coord,
) -> Result<Vec<ExPolygon>, edge_clip::ClipError> {
    // Turned so the bridge runs along +y.
    let aligning = -angle + PI * 0.5;
    let (sin, cos) = aligning.sin_cos();
    let rings: Vec<Polygon> = bridged
        .iter()
        .flat_map(ExPolygon::rings)
        .map(|ring| Polygon {
            points: ring.points.iter().map(|p| turn(*p, cos, sin)).collect(),
        })
        .collect();
    let anchors: Vec<Segment> = anchors
        .iter()
        .map(|s| Segment {
            a: turn(s.a, cos, sin),
            b: turn(s.b, cos, sin),
        })
        .collect();
    let bounds = |points: &mut dyn Iterator<Item = Point2>| {
        points.fold(None, |found: Option<(Point2, Point2)>, p| {
            Some(match found {
                None => (p, p),
                Some((low, high)) => (
                    Point2 {
                        x: low.x.min(p.x),
                        y: low.y.min(p.y),
                    },
                    Point2 {
                        x: high.x.max(p.x),
                        y: high.y.max(p.y),
                    },
                ),
            })
        })
    };
    let Some((low_x, high_x)) = bounds(&mut rings.iter().flat_map(|r| r.points.iter().copied()))
    else {
        return Ok(Vec::new());
    };
    let (low_y, high_y) = bounds(&mut anchors.iter().flat_map(|s| [s.a, s.b]))
        .unwrap_or((Point2 { x: 0, y: 0 }, Point2 { x: 0, y: 0 }));
    let scan = scan.max(1);
    let count = ((high_x.x - low_x.x + scan - 1) / scan).max(0) as usize;
    let scan_at = |i: usize| low_x.x + ((i as f64 + 0.5) * scan as f64) as Coord;
    let (y_min, y_max) = (low_y.y - spacing, high_y.y + spacing);

    // Each segment goes to the scan lines within its reach across.
    let area = segments_of_rings(&rings);
    let by_scan = |segments: &[Segment]| -> Vec<Vec<usize>> {
        let mut buckets = vec![Vec::new(); count];
        if count == 0 {
            return buckets;
        }
        for (index, s) in segments.iter().enumerate() {
            let (x0, x1) = (s.a.x.min(s.b.x), s.a.x.max(s.b.x));
            let first = (((x0 - low_x.x) as f64 / scan as f64 - 0.5).floor() as i64).max(0);
            let last =
                (((x1 - low_x.x) as f64 / scan as f64 - 0.5).ceil() as i64).min(count as i64 - 1);
            for i in first..=last {
                buckets[i as usize].push(index);
            }
        }
        buckets
    };
    let (area_by, anchors_by) = (by_scan(&area), by_scan(&anchors));
    let crossings = |segments: &[Segment], near: &[usize], line: &Segment| -> Vec<Point2> {
        let mut found: Vec<(f64, Point2)> = near
            .iter()
            .filter_map(|&index| crossing(line, &segments[index]))
            .map(|p| {
                let (dx, dy) = ((p.x - line.a.x) as f64, (p.y - line.a.y) as f64);
                (dx * dx + dy * dy, p)
            })
            .collect();
        found.sort_by(|l, r| l.0.total_cmp(&r.0));
        found.into_iter().map(|(_, p)| p).collect()
    };

    let overlap = |alow: Coord, ahigh: Coord, blow: Coord, bhigh: Coord| {
        (alow >= blow && alow <= bhigh)
            || (ahigh >= blow && ahigh <= bhigh)
            || (blow >= alow && blow <= ahigh)
            || (bhigh >= alow && bhigh <= ahigh)
    };
    let mut slices: Vec<Vec<Section>> = Vec::with_capacity(count);
    for i in 0..count {
        let x = scan_at(i);
        let line = Segment {
            a: Point2 { x, y: y_min },
            b: Point2 { x, y: y_max },
        };
        let through = crossings(&area, &area_by[i], &line);
        let mut sections: Vec<Section> = through
            .windows(2)
            .filter(|pair| {
                let middle = Point2 {
                    x: (pair[0].x + pair[1].x) / 2,
                    y: (pair[0].y + pair[1].y) / 2,
                };
                outside(&area, middle) < 0
            })
            .map(|pair| Section {
                a: pair[0],
                b: pair[1],
            })
            .collect();
        let held = crossings(&anchors, &anchors_by[i], &line);
        let reach = width as f64 * (0.5 + 0.5);
        for section in &mut sections {
            // The highest anchor below the stretch's start, and the lowest
            // above its end.
            if let Some(below) = held.iter().rev().find(|p| section.a.y > p.y) {
                section.a = *below;
                section.a.y = (section.a.y as f64 - reach) as Coord;
            }
            if let Some(above) = held.iter().find(|p| section.b.y < p.y) {
                section.b = *above;
                section.b.y = (section.b.y as f64 + reach) as Coord;
            }
        }
        for index in 0..sections.len().saturating_sub(1) {
            let (first, second) = (sections[index], sections[index + 1]);
            if overlap(first.a.y, first.b.y, second.a.y, second.b.y) {
                sections[index + 1].a = if first.a.y < second.a.y {
                    first.a
                } else {
                    second.a
                };
                sections[index + 1].b = if first.b.y < second.b.y {
                    second.b
                } else {
                    first.b
                };
                sections[index].a = first.b;
            }
        }
        sections.retain(|s| s.a != s.b);
        sections.sort_by_key(|s| (s.a.y, s.b.y));
        slices.push(sections);
    }

    // The stretches traced into outlines, scan line by scan line.
    let half = Point2 { x: scan / 2, y: 0 };
    let plus = |p: Point2| Point2 {
        x: p.x + half.x,
        y: p.y,
    };
    let minus = |p: Point2| Point2 {
        x: p.x - half.x,
        y: p.y,
    };
    let far = 36.0 * spacing as f64 * spacing as f64;
    let squared = |p: Point2, q: Point2| {
        let (dx, dy) = ((p.x - q.x) as f64, (p.y - q.y) as f64);
        dx * dx + dy * dy
    };
    let mut outlines: Vec<Polygon> = Vec::new();
    let mut current: Vec<Traced> = Vec::new();
    for slice in &slices {
        let mut used = vec![false; slice.len()];
        let mut open = Vec::with_capacity(current.len());
        for mut traced in current.drain(..) {
            let (Some(&low), Some(&high)) = (traced.lows.last(), traced.highs.last()) else {
                continue;
            };
            let begin = slice.partition_point(|s| s.b.y <= low.y);
            let end = slice.partition_point(|s| s.a.y <= high.y).max(begin);
            let mut added = false;
            for index in begin..end {
                if used[index] {
                    continue;
                }
                let candidate = slice[index];
                if squared(low, candidate.a) < far {
                    traced.lows.push(candidate.a);
                } else {
                    traced
                        .lows
                        .extend([plus(low), minus(candidate.a), candidate.a]);
                }
                if squared(high, candidate.b) < far {
                    traced.highs.push(candidate.b);
                } else {
                    traced
                        .highs
                        .extend([plus(high), minus(candidate.b), candidate.b]);
                }
                used[index] = true;
                added = true;
                break;
            }
            if added {
                open.push(traced);
            } else {
                // Nothing goes on with it: closed at its right edge.
                traced.lows.push(plus(low));
                traced.highs.push(plus(high));
                outlines.push(traced.close());
            }
        }
        current = open;
        for (index, section) in slice.iter().enumerate() {
            if !used[index] {
                current.push(Traced {
                    lows: vec![minus(section.a), section.a],
                    highs: vec![minus(section.b), section.b],
                });
            }
        }
    }
    outlines.extend(current.into_iter().map(Traced::close));
    let united = edge_clip::union(&outlines)?;
    // And turned back.
    let (sin, cos) = (-aligning).sin_cos();
    let back = |ring: &Polygon| Polygon {
        points: ring.points.iter().map(|p| turn(*p, cos, sin)).collect(),
    };
    Ok(united
        .iter()
        .map(|region| ExPolygon {
            contour: back(&region.contour),
            holes: region.holes.iter().map(back).collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use edge_geom::scaled;

    use super::*;

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point2> {
        vec![Point2::from_mm(x0, y0), Point2::from_mm(x1, y1)]
    }

    #[test]
    fn a_bridge_runs_square_to_the_lines_round_it() {
        // Lines along x every 2 mm round a small patch: the bridge runs
        // along y, a quarter turn.
        let lines: Vec<Vec<Point2>> = (0..10)
            .map(|i| line(0.0, 2.0 * i as f64, 20.0, 2.0 * i as f64))
            .collect();
        let patch = block(8.0, 7.0, 12.0, 11.0);
        let angle = bridging_angle(std::slice::from_ref(&patch), &segments_of_lines(&lines));
        assert!((angle - 0.5 * PI).abs() < 1e-9, "{angle}");
        // Lines at 45°: the bridge at 135°.
        let slanted: Vec<Vec<Point2>> = (-10..10)
            .map(|i| line(2.0 * i as f64, 0.0, 2.0 * i as f64 + 20.0, 20.0))
            .collect();
        let angle = bridging_angle(&[patch], &segments_of_lines(&slanted));
        assert!((angle - 0.75 * PI).abs() < 1e-9, "{angle}");
        // With nothing to go by, the reference's stand-in.
        assert_eq!(bridging_angle(&[block(0.0, 0.0, 1.0, 1.0)], &[]), 0.001);
    }

    #[test]
    fn a_patch_is_run_out_to_the_next_line_either_side() {
        // Lines along x at y = 0, 5, 10 and 15; a patch from y = 6 to 8,
        // bridged along y: it reaches from the line at 5 to the one at
        // 10, and a line's width past each.
        let lines: Vec<Vec<Point2>> = [0.0, 5.0, 10.0, 15.0]
            .iter()
            .map(|y| line(-5.0, *y, 25.0, *y))
            .collect();
        let patch = block(4.0, 6.0, 8.0, 8.0);
        let width = scaled(0.4);
        let found = anchored(
            &[patch],
            &segments_of_lines(&lines),
            scaled(0.4),
            width,
            0.5 * PI,
            scaled(0.4),
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        let ys: Vec<f64> = found[0]
            .contour
            .points
            .iter()
            .map(|p| edge_geom::unscaled(p.y))
            .collect();
        let (low, high) = ys
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(*y), h.max(*y)));
        assert!((low - 4.6).abs() < 1e-3, "{low}");
        assert!((high - 10.4).abs() < 1e-3, "{high}");
        // From half a step before the first scan line to the last: the
        // reference closes an outline still open at the end of the scan
        // where it stands, with no half step after it.
        let area = found[0].area_mm2();
        assert!((area - 3.8 * 5.8).abs() < 1e-3, "{area}");
    }

    #[test]
    fn rings_start_where_the_reference_s_do() {
        // As Clipper 6 hands back a 10 mm square and a triangle: from the
        // point after the lowest, the rightmost of equals.
        let square = block(0.0, 0.0, 10.0, 10.0).contour.points;
        let at = |x, y| Point2::from_mm(x, y);
        assert_eq!(
            as_the_reference_starts(&square),
            [at(10.0, 10.0), at(0.0, 10.0), at(0.0, 0.0), at(10.0, 0.0)]
        );
        let triangle = [at(0.0, 5.0), at(10.0, 0.0), at(6.0, 9.0)];
        assert_eq!(
            as_the_reference_starts(&triangle),
            [at(6.0, 9.0), at(0.0, 5.0), at(10.0, 0.0)]
        );
    }

    #[test]
    fn crossings_and_insides_are_the_reference_s() {
        let square = block(0.0, 0.0, 10.0, 10.0);
        let edges = segments_of_rings(square.rings());
        assert_eq!(outside(&edges, Point2::from_mm(5.0, 5.0)), -1);
        assert_eq!(outside(&edges, Point2::from_mm(15.0, 5.0)), 1);
        let across = Segment {
            a: Point2::from_mm(5.0, -1.0),
            b: Point2::from_mm(5.0, 11.0),
        };
        let found: Vec<Point2> = edges.iter().filter_map(|e| crossing(&across, e)).collect();
        assert_eq!(found.len(), 2);
        assert_eq!(
            closed_lines(&[square])[0].len(),
            5,
            "a ring as a line comes back to its start"
        );
    }
}
