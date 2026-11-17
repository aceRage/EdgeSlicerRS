//! Sparse lines in rows, made the way the reference makes them, to the
//! unit: a port of FillRectilinear.cpp `make_fill_lines` with
//! `slice_region_by_vertical_lines` and the parts of
//! `ExPolygonWithOffset` and `fill_surface_by_multilines` they rely on.
//!
//! The lines are the same as [`crate::lines`] lays them to within a unit
//! or two. That is not enough here: [`crate::connect`] joins the lines
//! along the outline nearest first, and on a square part many of those
//! arcs are the same length but for a unit, so a unit decides which ends
//! are joined. The reference turns the area to the lines' direction in
//! single precision (`float angle`), rounds every turned point to whole
//! units, and works out where a row crosses an edge as an exact fraction;
//! so is it done here.

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

/// libslic3r's `SCALED_EPSILON`, in scaled units.
const SCALED_EPSILON: Coord = 100;

/// `MultiPoint::rotate(cos, sin)` / `Point::rotate(cos, sin)`.
fn rotate(p: Point2, cos: f64, sin: f64) -> Point2 {
    let (x, y) = (p.x as f64, p.y as f64);
    Point2 {
        x: (cos * x - sin * y).round() as Coord,
        y: (cos * y + sin * x).round() as Coord,
    }
}

/// `align_to_grid(coord, spacing)`: rounded down to a multiple, also
/// below zero.
fn align(coord: Coord, spacing: Coord) -> Coord {
    if coord < 0 {
        ((coord - spacing + 1) / spacing) * spacing
    } else {
        (coord / spacing) * spacing
    }
}

/// A row crossing an edge (`SegmentIntersection`): where along the row,
/// as `p / q`, and which ring and edge.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    contour: usize,
    segment: usize,
    p: i64,
    q: i64,
    low: bool,
}

impl Crossing {
    /// `SegmentIntersection::pos`: the fraction rounded half away from zero.
    fn pos(&self) -> Coord {
        let half = self.q >> 1;
        let p = if self.p < 0 {
            self.p - half
        } else {
            self.p + half
        };
        p / self.q
    }

    /// `SegmentIntersection::operator<`, exactly.
    fn less(&self, other: &Self) -> bool {
        (self.p as i128) * (other.q as i128) < (other.p as i128) * (self.q as i128)
    }
}

/// One set of rows (`SweepParams`): the direction of the turned frame in
/// single precision, as the reference adds it up, and how far the rows
/// are moved sideways in scaled units (`pattern_shift`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sweep {
    pub angle: f32,
    pub shift: Coord,
}

/// The lines of every sweep across `outer` (the area's outline offset in
/// for the lines, `polygons_outer`), in the reference's order: one sweep
/// after the other, the rows in order across, along each row from its
/// low end in the turned frame. `src` is the area itself, whose extent
/// keeps the rows a line's width off its edge; `refpt` the point the rows
/// are laid through. `None` where the reference gives up on the area
/// (`slice_region_by_vertical_lines` finds the crossings of a row do not
/// pair up and throws `InfillFailedException`).
pub(crate) fn fill_lines(
    src: &ExPolygon,
    outer: &[&Polygon],
    refpt: Point2,
    line_spacing: Coord,
    line_width: Coord,
    sweeps: &[Sweep],
) -> Option<Vec<Vec<Point2>>> {
    let mut lines = Vec::new();
    if line_spacing <= 0 {
        return Some(lines);
    }
    // The reference slices an object about its own middle and moves it
    // to its place on the plate only when the G-code is written (Print.hpp
    // `PrintObject::bounding_box` is centred on zero, `PrintInstance::shift`
    // holds the place), so its turned points are rounded about the middle.
    // The same here: everything is moved so that `refpt` is the origin,
    // and the lines moved back.
    let local = |p: Point2| Point2 {
        x: p.x - refpt.x,
        y: p.y - refpt.y,
    };
    let src_local = ExPolygon {
        contour: Polygon {
            points: src.contour.points.iter().map(|&p| local(p)).collect(),
        },
        holes: Vec::new(),
    };
    let outer_local: Vec<Polygon> = outer
        .iter()
        .map(|ring| Polygon {
            points: ring.points.iter().map(|&p| local(p)).collect(),
        })
        .collect();
    let (src, outer, origin) = (&src_local, &outer_local, refpt);
    let refpt = Point2 { x: 0, y: 0 };
    for sweep in sweeps {
        let angle = f64::from(sweep.angle);
        // The area turned by -angle (`ExPolygonWithOffset(rhs, -angle)`).
        let (cos_in, sin_in) = ((-angle).cos(), (-angle).sin());
        let contours: Vec<Vec<Point2>> = outer
            .iter()
            .map(|ring| {
                ring.points
                    .iter()
                    .map(|&p| rotate(p, cos_in, sin_in))
                    .collect()
            })
            .collect();
        let src_turned: Vec<Point2> = src
            .contour
            .points
            .iter()
            .map(|&p| rotate(p, cos_in, sin_in))
            .collect();
        let Some(first) = src_turned.first() else {
            continue;
        };
        let (mut min, mut max) = (*first, *first);
        for p in &src_turned {
            (min.x, min.y) = (min.x.min(p.x), min.y.min(p.y));
            (max.x, max.y) = (max.x.max(p.x), max.y.max(p.y));
        }
        let x_margin = line_width + SCALED_EPSILON;
        let (x_min, x_max) = (min.x + x_margin, max.x - x_margin);
        let mut refpt = rotate(refpt, cos_in, sin_in);
        let shift = sweep.shift % line_spacing;
        refpt.x -= if shift >= 0 {
            shift
        } else {
            line_spacing + shift
        };
        let aligned = Point2 {
            x: refpt.x + align(min.x - refpt.x, line_spacing),
            y: refpt.y + align(min.y - refpt.y, line_spacing),
        };
        min.x = min.x.min(aligned.x);
        min.y = min.y.min(aligned.y);
        let n_vlines = ((max.x - min.x + line_spacing - 1) / line_spacing).max(0) as usize;
        let x0 = min.x;
        let rows = crossings(&contours, n_vlines, x0, line_spacing)?;
        let (cos_out, sin_out) = (angle.cos(), angle.sin());
        for (i, row) in rows.iter().enumerate() {
            let pos = x0 + i as Coord * line_spacing;
            if pos < x_min {
                continue;
            }
            if pos > x_max {
                break;
            }
            let mut it = 0;
            while it < row.len() {
                let low = &row[it];
                it += 1;
                if !low.low {
                    continue;
                }
                if let Some(high) = row.get(it).filter(|high| !high.low) {
                    let back = |p: Point2| Point2 {
                        x: p.x + origin.x,
                        y: p.y + origin.y,
                    };
                    lines.push(vec![
                        back(rotate(
                            Point2 {
                                x: pos,
                                y: low.pos(),
                            },
                            cos_out,
                            sin_out,
                        )),
                        back(rotate(
                            Point2 {
                                x: pos,
                                y: high.pos(),
                            },
                            cos_out,
                            sin_out,
                        )),
                    ]);
                    it += 1;
                }
            }
        }
    }
    Some(lines)
}

/// `slice_region_by_vertical_lines` for an area with no inner offset:
/// each row's crossings with the rings, in order up the row, with those
/// that only touch a row at a corner taken out. `None` when a row's
/// crossings do not pair up low and high.
fn crossings(
    contours: &[Vec<Point2>],
    n_vlines: usize,
    x0: Coord,
    line_spacing: Coord,
) -> Option<Vec<Vec<Crossing>>> {
    let mut rows: Vec<Vec<Crossing>> = vec![Vec::new(); n_vlines];
    for (contour_index, contour) in contours.iter().enumerate() {
        let n = contour.len();
        if n < 2 {
            continue;
        }
        for segment in 0..n {
            let prev = if segment == 0 { n - 1 } else { segment - 1 };
            let (p1, p2) = (contour[prev], contour[segment]);
            let (l, r) = (p1.x.min(p2.x), p1.x.max(p2.x));
            let mut il = (l - x0) / line_spacing;
            while il * line_spacing + x0 < l {
                il += 1;
            }
            il = il.max(0);
            let mut ir = (r - x0 + line_spacing) / line_spacing;
            while ir * line_spacing + x0 > r {
                ir -= 1;
            }
            ir = ir.min(n_vlines as Coord - 1);
            if il > ir {
                continue;
            }
            for i in il..=ir {
                let this_x = x0 + i * line_spacing;
                let (p, q);
                if p1.x == this_x {
                    if p2.x == this_x {
                        // A strictly vertical edge.
                        continue;
                    }
                    let p0 = contour[if prev == 0 { n - 1 } else { prev - 1 }];
                    if (p0.x - p1.x) * (p2.x - p1.x) > 0 {
                        // A corner touching the row from one side.
                        continue;
                    }
                    (p, q) = (p1.y, 1);
                } else if p2.x == this_x {
                    let p3 = contour[(segment + 1) % n];
                    if (p3.x - p2.x) * (p1.x - p2.x) > 0 {
                        continue;
                    }
                    (p, q) = (p2.y, 1);
                } else {
                    let (tp, tq) = if p2.x > p1.x {
                        (this_x - p1.x, p2.x - p1.x)
                    } else {
                        (p1.x - this_x, p1.x - p2.x)
                    };
                    (p, q) = (tp * (p2.y - p1.y) + p1.y * tq, tq);
                }
                rows[i as usize].push(Crossing {
                    contour: contour_index,
                    segment,
                    p,
                    q,
                    low: false,
                });
            }
        }
    }
    for row in &mut rows {
        crate::cxx_sort::sort_by(row, |a, b| a.less(b));
        let mut j = 0;
        for i in 0..row.len() {
            let is = row[i];
            let contour = &contours[is.contour];
            let n = contour.len();
            let prev = if is.segment == 0 {
                n - 1
            } else {
                is.segment - 1
            };
            let low = contour[is.segment].x - contour[prev].x > 0;
            row[i].low = low;
            let is = row[i];
            let mut take = true;
            if j > 0 {
                let is2 = row[j - 1];
                if is.contour == is2.contour && is.q == 1 && is2.q == 1 {
                    if is.p == is2.p {
                        // Two edges of one direction meet on the row.
                        take = false;
                    } else if is.low == is2.low {
                        // A Z whose middle runs along the row: keep the
                        // crossing that makes the row the longer.
                        if !low {
                            row[j - 1] = is;
                        }
                        take = false;
                    }
                }
            }
            if take {
                row[j] = is;
                j += 1;
            }
        }
        row.truncate(j);
        // The crossings must pair up, low then high.
        if row.len() % 2 == 1 || row.chunks(2).any(|pair| !pair[0].low || pair[1].low) {
            return None;
        }
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: Coord, y0: Coord, x1: Coord, y1: Coord) -> Polygon {
        Polygon {
            points: vec![
                Point2 { x: x0, y: y0 },
                Point2 { x: x1, y: y0 },
                Point2 { x: x1, y: y1 },
                Point2 { x: x0, y: y1 },
            ],
        }
    }

    #[test]
    fn rows_run_up_the_turned_frame_one_sweep_after_the_other() {
        // A 10 mm square, lines 2 mm apart, the frame turned a quarter
        // (lines along -x) and then a half (lines along -y).
        let src = ExPolygon {
            contour: square(0, 0, 10_000_000, 10_000_000),
            holes: Vec::new(),
        };
        let inner = square(500_000, 500_000, 9_500_000, 9_500_000);
        let quarter = std::f32::consts::FRAC_PI_2;
        let lines = fill_lines(
            &src,
            &[&inner],
            Point2 {
                x: 5_000_000,
                y: 5_000_000,
            },
            2_000_000,
            500_000,
            &[
                Sweep {
                    angle: quarter,
                    shift: 0,
                },
                Sweep {
                    angle: quarter + quarter,
                    shift: 0,
                },
            ],
        )
        .expect("rows pair up");
        // Rows every 2 mm through the middle, a line's width and a bit off
        // the area's edge: at 1, 3, 5, 7 and 9 mm, in each sweep.
        assert_eq!(lines.len(), 10);
        // First sweep: the turned frame's y runs along -x, so its rows are
        // horizontal lines run from right to left, in order up the plate
        // (the turned frame's x runs along +y).
        for line in &lines[..5] {
            assert_eq!(line[0].y, line[1].y);
            assert!(line[0].x > line[1].x);
        }
        assert!(lines[..5].windows(2).all(|w| w[0][0].y < w[1][0].y));
        assert_eq!(lines[0][0].y, 1_000_000);
        for line in &lines[5..] {
            assert_eq!(line[0].x, line[1].x);
            assert!(line[0].y > line[1].y);
        }
        // Each ends on the inner square.
        assert!(lines.iter().flatten().all(|p| [500_000, 9_500_000].contains(&p.x)
            || [500_000, 9_500_000].contains(&p.y)));
    }

    #[test]
    fn a_ring_whose_crossings_do_not_pair_up_is_given_up() {
        // A ring the wrong way round: each row meets its high side first.
        let mut bad = square(0, 0, 10_000_000, 2_000_000);
        bad.points.reverse();
        let src = ExPolygon {
            contour: square(0, 0, 10_000_000, 2_000_000),
            holes: Vec::new(),
        };
        let made = fill_lines(
            &src,
            &[&bad],
            Point2 { x: 0, y: 0 },
            1_000_000,
            100_000,
            &[Sweep {
                angle: 0.0,
                shift: 0,
            }],
        );
        assert!(made.is_none());
    }
}
