//! Paths thinned to `resolution` before they are written: the points of
//! a wall, a fill line or a support line that lie within `resolution` of
//! the line between those kept are left out, so a curve cut finer than
//! the printer can show is not written move by move.
//!
//! Ported from the reference, where it is a stage of its own between
//! making the toolpaths and writing them (PrintObject.cpp
//! `simplify_extrusion_path`): every path of a layer's walls and fill
//! (LayerRegion.cpp `simplify_path`, `simplify_multi_path`,
//! `simplify_loop`) and of its supports (Layer.cpp
//! `simplify_support_path` and the two beside it) goes through
//! ExtrusionEntity.cpp `ExtrusionPath::simplify`, which is Polyline.cpp
//! `Polyline::simplify`, which is MultiPoint.cpp `_douglas_peucker`.
//! That is the branch taken when arcs are not fitted (or in spiral vase
//! mode); with arcs the same stage fits them and thins what is left,
//! which is [`crate::arc::Fitted::new`]. Skirt, brim and prime tower
//! are not part of the stage and are written as they were made.
//!
//! As there:
//!
//! - Everything is held to `resolution`. Sparse infill and supports,
//!   which are held less closely when arcs are fitted, are not here.
//! - A wall loop is one closed path, thinned from its own first point
//!   round to that point again the way it is laid, so its first point
//!   always stays, in the middle of a straight side or not. The seam is
//!   chosen afterwards, among the points left, and the loop opened there
//!   (GCode.cpp `extrude_loop`); nothing is thinned again after that.
//!   So this runs ahead of the writer, which then sees only thinned
//!   loops: its seams, scarf joints, overhang measuring and wipes all
//!   go by them.
//! - A wall whose width changes is, in the reference, already cut into
//!   paths of one width each when the stage runs, and each is thinned by
//!   itself. Here it is cut as it is laid, so its pieces are thinned
//!   then ([`piece`]).
//! - A `resolution` of nothing still takes out a point that lies
//!   exactly on the line between its neighbours.
//!
//! Where this differs from the reference:
//!
//! - There a wall that hangs clear of the layer beneath for part of its
//!   way is cut into the stretches that do and do not before the stage
//!   runs (PerimeterGenerator.cpp), and each is thinned between its
//!   ends. Here the loop is thinned whole and measured against the
//!   layer beneath as it is laid, so where a stretch begins and ends is
//!   found on the thinned loop: within `resolution` of the same place.
//! - A loop thinned to fewer than three points, which is one no wider
//!   than `resolution`, is left as it was made. The reference lays the
//!   two points left as a line out and back.

use std::borrow::Cow;

use edge_extrusion::{ExtrusionLoop, ExtrusionPath};
use edge_geom::Point2;
use rayon::prelude::*;

use crate::{Layer, PrintSettings, arc};

/// An open path with the points left out that are within `tolerance`
/// (see [`arc::tolerance`]) of the line between those kept; its two ends
/// always stay.
fn open(points: &[Point2], tolerance: f64) -> Option<Vec<Point2>> {
    if points.len() < 3 {
        return None;
    }
    let kept = arc::douglas_peucker(points, tolerance);
    (kept.len() < points.len()).then_some(kept)
}

/// A loop thinned once round from its first point, the other way round
/// from how `points` run if `turned`; the points left are given the way
/// `points` run. None when nothing is left out, or fewer than three
/// points would be left.
fn ring(points: &[Point2], turned: bool, tolerance: f64) -> Option<Vec<Point2>> {
    if points.len() < 3 {
        return None;
    }
    let mut closed = Vec::with_capacity(points.len() + 1);
    closed.extend_from_slice(points);
    closed.push(points[0]);
    if turned {
        closed.reverse();
    }
    let mut kept = arc::douglas_peucker(&closed, tolerance);
    if kept.len() == closed.len() {
        return None;
    }
    kept.pop();
    if turned {
        kept[1..].reverse();
    }
    (kept.len() >= 3).then_some(kept)
}

/// One piece of a wall whose width changes, of one width, as it is to
/// be laid: thinned to `resolution` unless arcs are fitted, which thins
/// it in fitting them.
pub(crate) fn piece<'a>(s: &PrintSettings, points: &'a [Point2]) -> Cow<'a, [Point2]> {
    if s.arc_fitting {
        return Cow::Borrowed(points);
    }
    match open(points, arc::tolerance(s.resolution)) {
        Some(kept) => Cow::Owned(kept),
        None => Cow::Borrowed(points),
    }
}

/// What thinning changed in one layer: each list whole, with the paths
/// that lost nothing as they were, or none where no path of it lost
/// anything.
#[derive(Default)]
struct Kept {
    loops: Option<Vec<ExtrusionLoop>>,
    paths: Option<Vec<ExtrusionPath>>,
    support: Option<Vec<ExtrusionPath>>,
}

/// A stretch of layers with their walls, fill and supports thinned.
#[derive(Default)]
pub(crate) struct Thinned {
    layers: Vec<Kept>,
}

impl Thinned {
    /// Thins the layers given, all at once on as many threads as there
    /// are: what is left of a path does not depend on when its turn
    /// comes. Layers whose arcs are fitted are left alone.
    pub(crate) fn new(layers: &[Layer<'_>], settings: &[PrintSettings]) -> Self {
        if settings.iter().all(|s| s.arc_fitting) {
            return Self::default();
        }
        let layers = layers
            .par_iter()
            .map(|layer| {
                let s = &settings[layer.filament];
                if s.arc_fitting {
                    return Kept::default();
                }
                let tolerance = arc::tolerance(s.resolution);
                let lines = |paths: &[ExtrusionPath]| {
                    replaced(
                        paths,
                        |path| open(&path.points, tolerance),
                        |path, points| ExtrusionPath {
                            points,
                            role: path.role,
                            flow: path.flow,
                            in_turn: path.in_turn,
                            part: path.part,
                        },
                    )
                };
                Kept {
                    loops: replaced(
                        layer.loops,
                        |wall| {
                            // Cut into pieces of one width as it is
                            // laid, and thinned then.
                            if !wall.widths.is_empty() {
                                return None;
                            }
                            let turned = crate::wall_turned(s, wall);
                            ring(&wall.polygon.points, turned, tolerance)
                        },
                        |wall, points| {
                            let mut wall = wall.clone();
                            wall.polygon.points = points;
                            wall
                        },
                    ),
                    paths: lines(layer.paths),
                    support: lines(layer.support),
                }
            })
            .collect();
        Self { layers }
    }

    /// Some of the layers this was made from, `from` being how far into
    /// them the first given is, with what was thinned in place of what
    /// was made. A layer nothing was left out of is given as it came.
    pub(crate) fn layers<'a>(&'a self, layers: &[Layer<'a>], from: usize) -> Vec<Layer<'a>> {
        layers
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                let mut layer = *layer;
                if let Some(kept) = self.layers.get(from + index) {
                    if let Some(loops) = &kept.loops {
                        layer.loops = loops;
                    }
                    if let Some(paths) = &kept.paths {
                        layer.paths = paths;
                    }
                    if let Some(support) = &kept.support {
                        layer.support = support;
                    }
                }
                layer
            })
            .collect()
    }
}

/// `items` with each that `thin` leaves fewer points of made again from
/// those points, or none when `thin` changes nothing in any.
fn replaced<T: Clone>(
    items: &[T],
    thin: impl Fn(&T) -> Option<Vec<Point2>>,
    with: impl Fn(&T, Vec<Point2>) -> T,
) -> Option<Vec<T>> {
    let first = items.iter().position(|item| thin(item).is_some())?;
    let mut out = Vec::with_capacity(items.len());
    out.extend_from_slice(&items[..first]);
    out.extend(items[first..].iter().map(|item| match thin(item) {
        Some(points) => with(item, points),
        None => item.clone(),
    }));
    Some(out)
}

#[cfg(test)]
mod tests {
    // A list holding one island reads as `&[0..1]`, which is what is meant.
    #![allow(clippy::single_range_in_vec_init)]

    use std::ops::Range;

    use edge_extrusion::{Flow, Role};
    use edge_geom::Polygon;

    use super::*;
    use crate::{Lift, Seam, write};

    fn flow() -> Flow {
        Flow::new(0.45, 0.2, 0.4).unwrap()
    }

    /// `steps` sides of a circle about (`cx`, `cy`), counter-clockwise
    /// from `from` degrees to `to`.
    fn on_circle(cx: f64, cy: f64, r: f64, from: f64, to: f64, steps: usize) -> Vec<Point2> {
        (0..=steps)
            .map(|step| {
                let angle = (from + (to - from) * step as f64 / steps as f64).to_radians();
                Point2::from_mm(cx + r * angle.cos(), cy + r * angle.sin())
            })
            .collect()
    }

    /// A circle in 128 sides, its first corner due east. At 9.79 mm,
    /// which is where the outer wall of a 20 mm post lies, a corner is
    /// 11.8 µm from the line between its neighbours.
    fn round(cx: f64, cy: f64, r: f64) -> Vec<Point2> {
        let mut points = on_circle(cx, cy, r, 0.0, 360.0, 128);
        points.pop();
        points
    }

    fn wall(points: Vec<Point2>) -> ExtrusionLoop {
        ExtrusionLoop {
            polygon: Polygon { points },
            role: Role::ExternalPerimeter,
            is_hole: false,
            depth: 0,
            flow: flow(),
            widths: Vec::new(),
            clockwise: false,
            circle_compensation: false,
        }
    }

    fn line(points: Vec<Point2>, role: Role) -> ExtrusionPath {
        ExtrusionPath {
            points,
            role,
            flow: flow(),
            in_turn: false,
            part: 0,
        }
    }

    fn mm(points: &[(f64, f64)]) -> Vec<Point2> {
        points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect()
    }

    fn layer<'a>(
        loops: &'a [ExtrusionLoop],
        islands: &'a [Range<usize>],
        paths: &'a [ExtrusionPath],
    ) -> Layer<'a> {
        Layer {
            z: 0.2,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            tower: &[],
            around: &[],
            support: &[],
            loops,
            islands,
            paths,
            shelter: &[],
            below: &[],
        }
    }

    /// Plain settings with no arcs: walls start at the nearest corner
    /// and close fully, lifts go straight up, and layers that take no
    /// time at all are not slowed down.
    fn plain(resolution: f64) -> PrintSettings {
        PrintSettings {
            arc_fitting: false,
            resolution,
            seam: Seam::Nearest,
            seam_gap: 0.0,
            lift: Lift::Normal,
            slow_down_for_layer_cooling: false,
            ..PrintSettings::default()
        }
    }

    fn print(layers: &[Layer<'_>], settings: &PrintSettings) -> String {
        let mut out = Vec::new();
        write(&mut out, layers, settings).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// Where each extruding move of a feature ends, in mm.
    fn laid(text: &str, feature: &str) -> Vec<(f64, f64)> {
        let word = |line: &str, word: char| -> Option<f64> {
            line.split(' ')
                .find_map(|part| part.strip_prefix(word)?.parse().ok())
        };
        let mut out = Vec::new();
        let mut within = false;
        for line in text.lines() {
            if let Some(name) = line.strip_prefix(";TYPE:") {
                within = name == feature;
            }
            if within
                && line.starts_with("G1 ")
                && let (Some(x), Some(y), Some(fed)) =
                    (word(line, 'X'), word(line, 'Y'), word(line, 'E'))
                && fed > 0.0
            {
                out.push((x, y));
            }
        }
        out
    }

    #[test]
    fn a_round_loop_keeps_every_other_corner_and_its_first() {
        let points = round(50.0, 50.0, 9.79);
        for turned in [false, true] {
            let kept = ring(&points, turned, arc::tolerance(0.012)).unwrap();
            let every_other: Vec<Point2> = points.iter().copied().step_by(2).collect();
            // In the order the loop is held in, whichever way it is laid.
            assert_eq!(kept, every_other, "turned: {turned}");
        }
        // A coarser print keeps one corner in four; at 10 mm, where a
        // corner is 12.05 µm out, none goes.
        assert_eq!(
            ring(&points, false, arc::tolerance(0.05)).unwrap().len(),
            32
        );
        assert_eq!(
            ring(&round(50.0, 50.0, 10.0), false, arc::tolerance(0.012)),
            None
        );
    }

    #[test]
    fn a_loops_first_point_stays_though_it_is_on_a_straight_side() {
        // A square with a point in the middle of each side, the first
        // of them the loop's first point: as the reference thins a loop
        // as a path from its first point back to it, that one stays and
        // the other three go.
        let points = mm(&[
            (5.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0),
            (10.0, 10.0),
            (5.0, 10.0),
            (0.0, 10.0),
            (0.0, 5.0),
            (0.0, 0.0),
        ]);
        let corners = mm(&[
            (5.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ]);
        for tolerance in [0.0, 0.012] {
            for turned in [false, true] {
                assert_eq!(
                    ring(&points, turned, arc::tolerance(tolerance)).unwrap(),
                    corners
                );
            }
        }
        // Begun at a corner it is the square.
        let mut from_corner = points.clone();
        from_corner.rotate_left(1);
        assert_eq!(
            ring(&from_corner, false, arc::tolerance(0.012))
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn nothing_to_spare_means_nothing_made_again() {
        let square = mm(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        assert_eq!(ring(&square, false, arc::tolerance(0.012)), None);
        assert_eq!(ring(&square, true, arc::tolerance(5.0)), None);
        let zigzag = mm(&[(0.0, 0.0), (5.0, 1.0), (10.0, 0.0)]);
        assert_eq!(open(&zigzag, arc::tolerance(0.012)), None);
        assert_eq!(open(&zigzag[..2], arc::tolerance(5.0)), None);
        // A layer of them is handed on as it came, not copied.
        let loops = [wall(square)];
        let paths = [line(zigzag.clone(), Role::SolidInfill)];
        let support = [line(zigzag, Role::Support)];
        let mut one = layer(&loops, &[0..1], &paths);
        one.support = &support;
        let settings = [plain(0.012)];
        let thinned = Thinned::new(&[one], &settings);
        let seen = thinned.layers(&[one], 0);
        assert!(std::ptr::eq(seen[0].loops, one.loops));
        assert!(std::ptr::eq(seen[0].paths, one.paths));
        assert!(std::ptr::eq(seen[0].support, one.support));
    }

    #[test]
    fn a_loop_no_wider_than_the_resolution_is_left_as_made() {
        // 10 µm across: thinned, it would be its first point and the
        // one opposite.
        let speck = round(50.0, 50.0, 0.005);
        assert_eq!(ring(&speck, false, arc::tolerance(0.012)), None);
        // One a little wider is thinned.
        let small = round(50.0, 50.0, 0.5);
        let kept = ring(&small, false, arc::tolerance(0.012)).unwrap();
        assert!(kept.len() >= 3 && kept.len() < small.len());
    }

    #[test]
    fn an_open_line_keeps_its_ends_and_what_stands_out() {
        // 20 mm with a point every mm, 5 µm to one side and the other
        // in turn, and one 0.1 mm out.
        let mut points: Vec<Point2> = (0..=20)
            .map(|step| {
                let off = if step % 2 == 0 { 0.005 } else { -0.005 };
                Point2::from_mm(step as f64, if step == 7 { 0.1 } else { off })
            })
            .collect();
        let kept = open(&points, arc::tolerance(0.012)).unwrap();
        assert_eq!(kept.first(), points.first());
        assert_eq!(kept.last(), points.last());
        assert!(kept.contains(&points[7]));
        // Its neighbours stay with it, being far from the lines that
        // now run to it; the rest of the way nothing does.
        assert!(kept.len() <= 8, "{}", kept.len());
        // With no tolerance only a point exactly on the line between
        // its neighbours goes.
        assert_eq!(open(&points, 0.0), None);
        points = mm(&[(0.0, 0.0), (4.0, 0.0), (4.0, 0.0), (10.0, 0.0), (10.0, 3.0)]);
        assert_eq!(
            open(&points, 0.0).unwrap(),
            mm(&[(0.0, 0.0), (10.0, 0.0), (10.0, 3.0)])
        );
    }

    #[test]
    fn the_same_points_are_left_every_time_and_on_any_number_of_threads() {
        let loops = [
            wall(round(50.0, 50.0, 9.79)),
            wall(round(50.0, 50.0, 9.34)),
            wall(round(80.0, 50.0, 4.0)),
        ];
        let paths = [line(
            on_circle(50.0, 50.0, 8.0, 0.0, 300.0, 200),
            Role::SolidInfill,
        )];
        let layers: Vec<Layer<'_>> = (1..=200)
            .map(|index| Layer {
                z: 0.2 * index as f64,
                ..layer(&loops, &[0..2, 2..3], &paths)
            })
            .collect();
        let on = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| print(&layers, &plain(0.012)))
        };
        let one = on(1);
        assert_eq!(one, on(1));
        assert_eq!(one, on(8));
    }

    #[test]
    fn a_round_wall_is_written_with_the_points_left_and_opened_at_one_of_them() {
        let points = round(50.0, 50.0, 9.79);
        let loops = [wall(points.clone())];
        let text = print(&[layer(&loops, &[0..1], &[])], &plain(0.012));
        let moves = laid(&text, "Outer wall");
        // Once round the 64 corners left: the seam is one of them, so
        // no move ends anywhere else, and nothing is thinned again
        // once the loop is opened there.
        assert_eq!(moves.len(), 64);
        let kept = ring(&points, false, arc::tolerance(0.012)).unwrap();
        for (x, y) in &moves {
            assert!(
                kept.iter().any(|p| {
                    (edge_geom::unscaled(p.x) - x).abs() < 0.0006
                        && (edge_geom::unscaled(p.y) - y).abs() < 0.0006
                }),
                "{x} {y} is not a point left"
            );
        }
        // With arcs fitted this stage does nothing: the fitter thins.
        let arcs = PrintSettings {
            arc_fitting: true,
            ..plain(0.012)
        };
        let one = layer(&loops, &[0..1], &[]);
        let settings = [arcs];
        let thinned = Thinned::new(&[one], &settings);
        let seen = thinned.layers(&[one], 0);
        assert!(std::ptr::eq(seen[0].loops, one.loops));
    }

    #[test]
    fn a_hole_is_thinned_the_way_it_is_laid_and_held_the_way_it_was() {
        let mut points = round(50.0, 50.0, 4.0);
        points.reverse();
        let hole = ExtrusionLoop {
            is_hole: true,
            ..wall(points.clone())
        };
        let s = plain(0.012);
        assert!(crate::wall_turned(&s, &hole));
        let loops = [hole];
        let one = layer(&loops, &[0..1], &[]);
        let settings = [s];
        let thinned = Thinned::new(&[one], &settings);
        let seen = thinned.layers(&[one], 0);
        let kept = &seen[0].loops[0].polygon.points;
        assert!(kept.len() < points.len());
        assert_eq!(kept[0], points[0]);
        // Still clockwise: each point left comes later in the loop
        // than the one before.
        let places: Vec<usize> = kept
            .iter()
            .map(|p| points.iter().position(|q| q == p).unwrap())
            .collect();
        assert!(places.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(seen[0].loops[0].is_hole);
    }

    #[test]
    fn fill_and_supports_are_held_to_the_resolution_like_the_rest() {
        // A quarter turn of 12 mm radius in 32 sides: a corner is 14 µm
        // from the line between its neighbours. With arcs fitted sparse
        // infill (40 µm) and supports (37.5 µm) would lose every other
        // one; without, everything is held to `resolution`.
        let curve = on_circle(50.0, 50.0, 12.0, 0.0, 90.0, 32);
        let moves = |role: Role, as_support: bool, resolution: f64| {
            let paths = [line(curve.clone(), role)];
            let mut one = layer(&[], &[], &[]);
            if as_support {
                one.support = &paths;
            } else {
                one.paths = &paths;
            }
            let text = print(&[one], &plain(resolution));
            laid(&text, crate::role_name(role)).len()
        };
        for (role, as_support) in [
            (Role::SparseInfill, false),
            (Role::SolidInfill, false),
            (Role::TopSolidInfill, false),
            (Role::GapFill, false),
            (Role::Ironing, false),
            (Role::Support, true),
            (Role::SupportInterface, true),
        ] {
            assert_eq!(moves(role, as_support, 0.012), 32, "{role:?}");
            assert_eq!(moves(role, as_support, 0.03), 16, "{role:?}");
        }
    }

    #[test]
    fn skirt_brim_and_prime_tower_are_written_as_made() {
        let around = [
            ExtrusionLoop {
                role: Role::Skirt,
                ..wall(round(50.0, 50.0, 13.79))
            },
            ExtrusionLoop {
                role: Role::Brim,
                ..wall(round(50.0, 50.0, 10.79))
            },
        ];
        let tower = [line(
            on_circle(90.0, 50.0, 5.0, 0.0, 180.0, 64),
            Role::PrimeTower,
        )];
        let loops = [wall(round(50.0, 50.0, 9.79))];
        let mut one = layer(&loops, &[0..1], &[]);
        one.around = &around;
        one.tower = &tower;
        let text = print(&[one], &plain(0.05));
        assert_eq!(laid(&text, "Skirt").len(), 128);
        assert_eq!(laid(&text, "Brim").len(), 128);
        assert_eq!(laid(&text, "Prime tower").len(), 64);
        assert_eq!(laid(&text, "Outer wall").len(), 32);
    }

    #[test]
    fn a_wall_whose_width_changes_is_thinned_piece_by_piece_as_it_is_laid() {
        // Round, and for a quarter of the way round a fifth wider.
        let points = round(50.0, 50.0, 9.79);
        let widths: Vec<_> = (0..points.len())
            .map(|index| edge_geom::scaled(if index < 32 { 0.54 } else { 0.45 }))
            .collect();
        let varying = ExtrusionLoop {
            widths,
            ..wall(points.clone())
        };
        // Ahead of the writer it is left alone.
        let loops = [varying];
        let one = layer(&loops, &[0..1], &[]);
        let settings = [plain(0.012)];
        let thinned = Thinned::new(&[one], &settings);
        assert!(std::ptr::eq(thinned.layers(&[one], 0)[0].loops, one.loops));
        // Laid, it has two widths and has lost corners from each
        // piece: not as many as the loop whole would, since a piece is
        // halved again and again from its own ends, and only a number
        // of sides that is a power of two comes out even.
        let text = print(&[one], &settings[0]);
        let moves = laid(&text, "Outer wall").len();
        assert!((64..100).contains(&moves), "{moves}");
        let widths: std::collections::BTreeSet<&str> = text
            .lines()
            .filter_map(|line| line.strip_prefix(";WIDTH:"))
            .collect();
        assert!(widths.len() >= 2, "{widths:?}");
        // A piece by itself.
        let quarter = &points[..33];
        assert_eq!(piece(&settings[0], quarter).len(), 17);
        let arcs = PrintSettings {
            arc_fitting: true,
            ..plain(0.012)
        };
        assert_eq!(piece(&arcs, quarter).len(), 33);
    }
}
