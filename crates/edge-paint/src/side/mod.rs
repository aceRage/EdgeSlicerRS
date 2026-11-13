//! Colours from a part's sides: where the painted triangles cut each
//! layer's plane, which edges of the layer's outline those cuts colour,
//! and the share of the layer nearest each colour's edges.

pub(crate) mod colour;
pub(crate) mod graph;
pub use graph::SideError;

use edge_geom::{Coord, ExPolygon, Point2, UNITS_PER_MM};
use rustc_hash::FxHashMap;

use crate::{Facet, PaintError};

/// A segment from `a` to `b`, in scaled units (the reference's `Line`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub a: Point2,
    pub b: Point2,
}

impl Line {
    pub fn new(a: Point2, b: Point2) -> Self {
        Self { a, b }
    }
}

pub(crate) fn dist2(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    dx * dx + dy * dy
}

/// The reference's `scale_(mm)`.
pub(crate) fn scale(mm: f64) -> f64 {
    mm / 1e-6
}

/// `SCALED_EPSILON`, in scaled units.
pub(crate) const SCALED_EPSILON: i64 = 100;

/// A cut colours the stretch of an outline's edge it lies within this
/// far of, in scaled units: the outline has been closed up and
/// simplified since the mesh was cut, so the two do not lie exactly on
/// each other.
pub(crate) const NEAR: f64 = 0.15 * UNITS_PER_MM;
/// A run of one colour along an outline shorter than this is given to
/// the run before it: such scraps come of where triangles meet, not of
/// what was painted.
const SHORT_RUN: f64 = 0.2 * UNITS_PER_MM;
/// A triangle no taller than this is flat for the purpose of cutting it
/// (the reference's `is_equal`).
const FLAT: f32 = 1e-3;
/// The side of the squares cuts are sorted into to be found again.
const CELL: f64 = 2.0 * UNITS_PER_MM;

/// Where a painted triangle crosses a layer's plane.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cut {
    a: (f64, f64),
    b: (f64, f64),
    colour: u8,
}

/// Each layer's cuts through the painted triangles, in the order the
/// triangles are given.
pub(crate) fn cuts(facets: &[Facet], planes: &[f64]) -> Vec<Vec<Cut>> {
    let mut cuts: Vec<Vec<Cut>> = vec![Vec::new(); planes.len()];
    for facet in facets.iter().filter(|facet| facet.filament > 0) {
        let mut corners = facet.corners;
        corners.sort_by(|a, b| a[2].total_cmp(&b[2]));
        let [low, middle, high] = corners;
        if high[2] - low[2] <= FLAT {
            continue;
        }
        let first = planes.partition_point(|&z| z < f64::from(low[2]));
        for (layer, &z) in planes.iter().enumerate().skip(first) {
            let z = z as f32;
            if z > high[2] {
                break;
            }
            let along = |from: [f32; 3], to: [f32; 3]| {
                let t = if to[2] > from[2] {
                    ((z - from[2]) / (to[2] - from[2])).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (
                    f64::from(from[0] + t * (to[0] - from[0])) * UNITS_PER_MM,
                    f64::from(from[1] + t * (to[1] - from[1])) * UNITS_PER_MM,
                )
            };
            let a = along(low, high);
            let b = if middle[2] > z {
                along(low, middle)
            } else {
                along(middle, high)
            };
            cuts[layer].push(Cut {
                a,
                b,
                colour: facet.filament,
            });
        }
    }
    cuts
}

/// One edge of an outline, from `from` to the next one's `from`, and the
/// colour it is painted, 0 for none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Painted {
    pub from: Point2,
    pub colour: u8,
}

/// A layer's rings with their edges cut where the colour changes.
pub(crate) fn colour(outline: &[ExPolygon], cuts: &[Cut]) -> Vec<Vec<Painted>> {
    // The cuts by the squares they pass through.
    let cell = |c: f64| (c / CELL).floor() as i64;
    let mut grid: FxHashMap<(i64, i64), Vec<u32>> = FxHashMap::default();
    for (index, cut) in cuts.iter().enumerate() {
        let (x0, x1) = (cut.a.0.min(cut.b.0) - NEAR, cut.a.0.max(cut.b.0) + NEAR);
        let (y0, y1) = (cut.a.1.min(cut.b.1) - NEAR, cut.a.1.max(cut.b.1) + NEAR);
        for x in cell(x0)..=cell(x1) {
            for y in cell(y0)..=cell(y1) {
                grid.entry((x, y)).or_default().push(index as u32);
            }
        }
    }

    outline
        .iter()
        .flat_map(ExPolygon::rings)
        .map(|ring| {
            let mut painted = Vec::new();
            let points = &ring.points;
            for (at, &from) in points.iter().enumerate() {
                let to = points[(at + 1) % points.len()];
                if from == to {
                    continue;
                }
                let (ax, ay) = (from.x as f64, from.y as f64);
                let (dx, dy) = (to.x as f64 - ax, to.y as f64 - ay);
                let length = dx.hypot(dy);
                let (ux, uy) = (dx / length, dy / length);
                let mut near: Vec<u32> = Vec::new();
                for x in cell(ax.min(ax + dx))..=cell(ax.max(ax + dx)) {
                    for y in cell(ay.min(ay + dy))..=cell(ay.max(ay + dy)) {
                        if let Some(found) = grid.get(&(x, y)) {
                            near.extend(found);
                        }
                    }
                }
                near.sort_unstable();
                near.dedup();
                // The stretch of the edge each cut lies along, with how
                // far off it is: (from, to, off, colour).
                let mut along: Vec<(f64, f64, f64, u8)> = Vec::new();
                for &index in &near {
                    let cut = &cuts[index as usize];
                    let place = |p: (f64, f64)| {
                        let (px, py) = (p.0 - ax, p.1 - ay);
                        (px * ux + py * uy, px * uy - py * ux)
                    };
                    let ((mut t0, mut d0), (mut t1, mut d1)) = (place(cut.a), place(cut.b));
                    if t0 > t1 {
                        ((t0, d0), (t1, d1)) = ((t1, d1), (t0, d0));
                    }
                    // Only the part beside the edge counts.
                    let (low, high) = (t0.max(0.0), t1.min(length));
                    if high - low < 1.0 {
                        continue;
                    }
                    let off = |t: f64| {
                        if t1 > t0 {
                            d0 + (d1 - d0) * (t - t0) / (t1 - t0)
                        } else {
                            d0
                        }
                    };
                    let (off_low, off_high) = (off(low).abs(), off(high).abs());
                    if off_low.max(off_high) <= NEAR {
                        along.push((low, high, (off_low + off_high) / 2.0, cut.colour));
                    }
                }
                let mut breaks: Vec<f64> = along
                    .iter()
                    .flat_map(|&(low, high, ..)| [low, high])
                    .chain([0.0, length])
                    .collect();
                breaks.sort_by(f64::total_cmp);
                breaks.dedup_by(|a, b| (*a - *b).abs() < 1.0);
                // The colour of the stretch before, on this edge.
                let mut last = None;
                for pair in breaks.windows(2) {
                    let middle = (pair[0] + pair[1]) / 2.0;
                    // The nearest cut beside this stretch says its colour.
                    let colour = along
                        .iter()
                        .filter(|&&(low, high, ..)| low <= middle && middle <= high)
                        .min_by(|a, b| a.2.total_cmp(&b.2))
                        .map_or(0, |found| found.3);
                    if last == Some(colour) {
                        continue;
                    }
                    last = Some(colour);
                    let start = if pair[0] <= 0.0 {
                        from
                    } else {
                        Point2 {
                            x: (ax + ux * pair[0]).round() as Coord,
                            y: (ay + uy * pair[0]).round() as Coord,
                        }
                    };
                    if painted
                        .last()
                        .is_none_or(|last: &Painted| last.from != start)
                    {
                        painted.push(Painted {
                            from: start,
                            colour,
                        });
                    }
                }
            }
            if painted.len() > 1 && painted[0].from == painted[painted.len() - 1].from {
                painted.pop();
            }
            settle(&mut painted);
            painted
        })
        .collect()
}

/// Gives each run of one colour too short to mean anything to the run
/// before it, shortest first, until none is left.
fn settle(ring: &mut [Painted]) {
    let count = ring.len();
    if count < 2 {
        return;
    }
    let lengths: Vec<f64> = (0..count)
        .map(|at| {
            let (a, b) = (ring[at].from, ring[(at + 1) % count].from);
            ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
        })
        .collect();
    loop {
        // Runs as (first edge, edges, length), starting at a change of colour.
        let Some(start) =
            (0..count).find(|&at| ring[at].colour != ring[(at + count - 1) % count].colour)
        else {
            return;
        };
        let mut runs: Vec<(usize, usize, f64)> = Vec::new();
        let mut at = 0;
        while at < count {
            let first = (start + at) % count;
            let (mut edges, mut length) = (0, 0.0);
            while at < count && ring[(start + at) % count].colour == ring[first].colour {
                length += lengths[(start + at) % count];
                edges += 1;
                at += 1;
            }
            runs.push((first, edges, length));
        }
        let shortest = runs
            .iter()
            .enumerate()
            .filter(|(_, run)| run.2 < SHORT_RUN)
            .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
            .map(|(index, _)| index);
        let Some(shortest) = shortest else { return };
        let before = runs[(shortest + runs.len() - 1) % runs.len()];
        let colour = ring[before.0].colour;
        let (first, edges, _) = runs[shortest];
        for step in 0..edges {
            ring[(first + step) % count].colour = colour;
        }
    }
}

/// The share of a layer nearest each colour's stretches of its outline:
/// one list of regions for every colour, 0 being unpainted.
pub(crate) fn share(
    layer: usize,
    outline: &[ExPolygon],
    cuts: &[Cut],
    colours: usize,
) -> Result<Vec<Vec<ExPolygon>>, PaintError> {
    let mut shares = vec![Vec::new(); colours];
    if outline.is_empty() || cuts.is_empty() {
        return Ok(shares);
    }
    let rings = colour(outline, cuts);
    let mut seen = rings.iter().flatten().map(|edge| edge.colour);
    let Some(first) = seen.next() else {
        return Ok(shares);
    };
    // One colour all round needs no dividing.
    if seen.all(|colour| colour == first) {
        shares[first as usize] = outline.to_vec();
        return Ok(shares);
    }
    // The borders between colours traced through the graph of the
    // rings and their Voronoi diagram (MultiMaterialSegmentation.cpp
    // `build_graph`, `extract_colored_segments`).
    let coloured: Vec<Vec<colour::ColouredLine>> = rings
        .iter()
        .map(|ring| {
            (0..ring.len())
                .map(|at| colour::ColouredLine {
                    line: Line::new(ring[at].from, ring[(at + 1) % ring.len()].from),
                    colour: ring[at].colour,
                })
                .collect()
        })
        .collect();
    let faces = graph::segments(&coloured, colours)
        .map_err(|source| PaintError::Diagram { layer, source })?;
    for (colour, polygons) in faces.into_iter().enumerate() {
        if !polygons.is_empty() {
            shares[colour] = edge_clip::union(&polygons)?;
        }
    }
    Ok(shares)
}
