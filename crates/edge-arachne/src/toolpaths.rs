//! The walls of one region, from its outline to finished lines
//! (`WallToolPaths`, WallToolPaths.cpp): the outline is prepared, cut
//! into its skeleton and walled, and the pieces of wall that come out
//! are joined end to end, the ones too short to print dropped, and the
//! points nobody would miss taken out.

use edge_clip::ClipError;
use edge_geom::{Coord, ExPolygon, Polygon, scaled};
use edge_medial::MedialError;

use crate::skeleton::Skeleton;
use crate::{ExtrusionLine, VariableWidthLines, beading, prepare, stitch};

/// How much the plastic a line lays may change where a point is taken
/// out of it, in scaled units squared over 1000
/// (`meshfix_maximum_extrusion_area_deviation`).
const MAX_EXTRUSION_AREA_DEVIATION: i64 = 2_000_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error(transparent)]
    Diagram(#[from] MedialError),
}

/// What the walls of a region are made to (`WallToolPaths`'s arguments
/// and `WallToolPathsParams`). The lengths in mm are held in single
/// precision, as the reference holds them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The outer wall's width, and the others', in scaled units.
    pub bead_width_0: Coord,
    pub bead_width_x: Coord,
    /// How many walls, at most.
    pub inset_count: usize,
    /// How far the outer wall is moved in.
    pub wall_0_inset: Coord,
    pub layer_height: f64,
    /// The narrowest line laid, in mm (`min_bead_width` of the nozzle).
    pub min_bead_width: f32,
    /// The thinnest part that gets a line, in mm (`min_feature_size`).
    pub min_feature_size: f32,
    /// A loose line down a middle shorter than this many of its own
    /// widths is dropped (`min_length_factor`).
    pub min_length_factor: f32,
    /// The length a change in the number of lines is spread over, in mm.
    pub wall_transition_length: f32,
    /// The angle of a wedge above which the number of lines changes
    /// along it, in degrees.
    pub wall_transition_angle: f32,
    /// How much a line's width may change to save two changes in number
    /// close together, in mm.
    pub wall_transition_filter_deviation: f32,
    /// Over how many of the innermost lines a change in width is shared.
    pub wall_distribution_count: i32,
    pub is_top_or_bottom_layer: bool,
    /// Points nearer each other than this may be merged, and how far
    /// that may move the line.
    pub wall_maximum_resolution: Coord,
    pub wall_maximum_deviation: Coord,
}

/// The reference's `scaled<coord_t>(float)`: divided in single precision
/// and cut to a whole number.
fn scaled_f32(mm: f32) -> Coord {
    (mm / 1e-6f32) as Coord
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Walls {
    /// By depth, outermost first; depths with nothing are left out.
    pub toolpaths: Vec<VariableWidthLines>,
    /// What is left inside the innermost wall.
    pub inner_contour: Vec<ExPolygon>,
}

/// The walls of `outline` (`WallToolPaths::generate`).
pub fn generate(outline: &[ExPolygon], params: &Params) -> Result<Walls, Error> {
    if params.inset_count < 1 {
        return Ok(Walls {
            toolpaths: Vec::new(),
            inner_contour: outline.to_vec(),
        });
    }
    let min_bead_width = scaled_f32(params.min_bead_width);
    let min_feature_size = scaled_f32(params.min_feature_size);
    let small_area_length = params.bead_width_0 as f64 / 2.0;
    let transitioning_angle = f64::from(params.wall_transition_angle).to_radians();
    let discretization_step_size = scaled(0.8);

    let prepared = prepare::prepare(
        outline,
        params.wall_maximum_resolution,
        params.wall_maximum_deviation,
        small_area_length,
    )?;
    if prepared.is_empty() {
        return Ok(Walls::default());
    }

    // The widths the reference's flow would have for these spacings.
    let width_of = |spacing: Coord| {
        let (spacing, height) = ((spacing as f64 * 1e-6) as f32, params.layer_height as f32);
        (f64::from(spacing) + f64::from(height) * (1.0 - 0.25 * std::f64::consts::PI)) as f32
    };
    let outer_width = f64::from(width_of(params.bead_width_0));
    let inner_width = f64::from(width_of(params.bead_width_x));
    let wall_transition_length = scaled_f32(params.wall_transition_length);
    let min_bead_mm = min_bead_width as f64 * 1e-6;
    // An odd number of lines: when the middle one is made two. An even
    // number: when one is added in the middle.
    let wall_split_middle_threshold = (2.0 * min_bead_mm / outer_width - 1.0).clamp(0.01, 0.99);
    let wall_add_middle_threshold = (min_bead_mm / inner_width).clamp(0.01, 0.99);
    let max_bead_count = 2 * params.inset_count as Coord;
    let strategy = beading::make_strategy(
        params.bead_width_0,
        params.bead_width_x,
        wall_transition_length,
        transitioning_angle as f32,
        true,
        min_bead_width,
        min_feature_size,
        wall_split_middle_threshold,
        wall_add_middle_threshold,
        max_bead_count,
        params.wall_0_inset,
        params.wall_distribution_count,
        0.5,
    );
    let mut skeleton = Skeleton::new(
        &prepared,
        strategy.as_ref(),
        strategy.get_transitioning_angle(),
        discretization_step_size,
        scaled(100.0),
        scaled_f32(params.wall_transition_filter_deviation),
        wall_transition_length,
    )?;
    let mut toolpaths = skeleton.generate_toolpaths(false);

    stitch_toolpaths(&mut toolpaths, params.bead_width_x);
    remove_small_lines(&mut toolpaths, params);
    let inner_contour = separate_out_inner_contour(&mut toolpaths)?;
    for line in toolpaths.iter_mut().flatten() {
        line.simplify(
            params.wall_maximum_resolution * params.wall_maximum_resolution,
            params.wall_maximum_deviation * params.wall_maximum_deviation,
            MAX_EXTRUSION_AREA_DEVIATION,
        );
    }
    toolpaths.retain(|lines| !lines.is_empty());
    Ok(Walls {
        toolpaths,
        inner_contour,
    })
}

/// Joins the pieces of each depth's walls end to end, and closes the
/// ones that come round to where they began (`stitchToolPaths`).
fn stitch_toolpaths(toolpaths: &mut [VariableWidthLines], bead_width_x: Coord) {
    // Where a wall has no width its pieces may be a line's width apart,
    // and no more is bridged.
    let stitch_distance = bead_width_x - 1;
    for wall_lines in toolpaths.iter_mut() {
        let mut stitched = Vec::new();
        let mut closed = Vec::new();
        stitch::stitch(
            wall_lines,
            &mut stitched,
            &mut closed,
            stitch_distance,
            stitch::DEFAULT_SNAP_DISTANCE,
        );
        *wall_lines = stitched;
        for mut polygon in closed {
            let (Some(&front), Some(&back)) = (polygon.junctions.first(), polygon.junctions.last())
            else {
                continue;
            };
            // A closed wall ends on the point it began at.
            let apart = ((back.p.x - front.p.x) as f64).hypot((back.p.y - front.p.y) as f64);
            if front.p != back.p && apart < stitch_distance as f64 {
                polygon.junctions.push(front);
            }
            polygon.is_closed = true;
            wall_lines.push(polygon);
        }
    }
}

/// Whether `line`, round to its start again, is shorter than
/// `check_length` (`shorterThan`).
fn shorter_than(line: &ExtrusionLine, check_length: Coord) -> bool {
    let Some(mut p0) = line.junctions.last().map(|junction| junction.p) else {
        return true;
    };
    let mut length = 0;
    for junction in &line.junctions {
        length += crate::graph::length(p0, junction.p);
        if length >= check_length {
            return false;
        }
        p0 = junction.p;
    }
    true
}

/// Drops loose lines down a middle that are too short to be worth
/// starting (`removeSmallLines`).
fn remove_small_lines(toolpaths: &mut [VariableWidthLines], params: &Params) {
    for inset in toolpaths.iter_mut() {
        let mut index = 0;
        while index < inset.len() {
            let line = &inset[index];
            let min_width = line.get_minimal_width();
            let check_length = if params.is_top_or_bottom_layer {
                min_width / 2
            } else {
                (min_width as f32 * params.min_length_factor) as Coord
            };
            if line.is_odd && !line.is_closed && shorter_than(line, check_length) {
                // The last takes its place, and is looked at next.
                inset.swap_remove(index);
            } else {
                index += 1;
            }
        }
    }
}

/// Takes the walls of no width out of `toolpaths`: they mark where the
/// walls end, and closed ones are the outline of what is left inside
/// (`separateOutInnerContour`).
fn separate_out_inner_contour(
    toolpaths: &mut Vec<VariableWidthLines>,
) -> Result<Vec<ExPolygon>, ClipError> {
    let mut actual = Vec::new();
    let mut contour: Vec<Polygon> = Vec::new();
    for inset in toolpaths.drain(..) {
        if inset.is_empty() {
            continue;
        }
        // By the first junction of the last line that has one.
        let is_contour = inset
            .iter()
            .rev()
            .find_map(|line| line.junctions.first())
            .is_some_and(|junction| junction.w == 0);
        if is_contour {
            contour.extend(
                inset
                    .iter()
                    .filter(|line| !line.is_odd && line.is_closed)
                    .map(ExtrusionLine::to_polygon),
            );
        } else {
            actual.push(inset);
        }
    }
    *toolpaths = actual;
    // Pieces joined into walls are wound either way; what is inside one
    // and not inside another within it is the inside.
    edge_clip::union_even_odd(&contour)
}

#[cfg(test)]
mod tests {
    use edge_geom::Point2;

    use super::*;

    pub(crate) fn params(inset_count: usize) -> Params {
        Params {
            bead_width_0: 420_000,
            bead_width_x: 450_000,
            inset_count,
            wall_0_inset: 0,
            layer_height: 0.2,
            min_bead_width: 0.34,
            min_feature_size: 0.1,
            min_length_factor: 0.5,
            wall_transition_length: 0.4,
            wall_transition_angle: 10.0,
            wall_transition_filter_deviation: 0.1,
            wall_distribution_count: 1,
            is_top_or_bottom_layer: false,
            wall_maximum_resolution: 500_000,
            wall_maximum_deviation: 25_000,
        }
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn walls(shape: ExPolygon, inset_count: usize) -> Walls {
        generate(&[shape], &params(inset_count)).unwrap()
    }

    #[test]
    fn a_square_gets_its_walls_at_full_width() {
        let walls = walls(rect(0.0, 0.0, 20.0, 20.0), 2);
        assert_eq!(walls.toolpaths.len(), 2);
        // One closed loop a depth, each as wide as set, the outer one's
        // middle 0.21 mm in and the inner one's 0.42 + 0.225.
        for (depth, (width, inset)) in [(420_000, 210_000), (450_000, 645_000)]
            .into_iter()
            .enumerate()
        {
            let lines = &walls.toolpaths[depth];
            assert_eq!(lines.len(), 1);
            let line = &lines[0];
            assert!(line.is_closed && !line.is_odd && line.inset_idx == depth);
            assert_eq!(line.junctions.first(), line.junctions.last());
            assert_eq!(line.junctions.len(), 5);
            assert!(line.junctions.iter().all(|j| j.w == width));
            assert!(line.junctions.iter().all(|j| {
                [j.p.x, j.p.y]
                    .iter()
                    .all(|&c| c == inset || c == 20_000_000 - inset)
            }));
        }
        // Inside the walls: 20 - 2 * (0.42 + 0.45) mm a side.
        let inside: f64 = walls.inner_contour.iter().map(ExPolygon::area_mm2).sum();
        assert!((inside - 18.26 * 18.26).abs() < 1e-3, "{inside}");

        // No walls asked for: the outline is the inside.
        let none = walls_of_none();
        assert!(none.toolpaths.is_empty());
        assert_eq!(none.inner_contour, [rect(0.0, 0.0, 20.0, 20.0)]);
    }

    fn walls_of_none() -> Walls {
        walls(rect(0.0, 0.0, 20.0, 20.0), 0)
    }

    #[test]
    fn a_strip_too_thin_for_two_walls_gets_one_line_as_wide_as_it_is() {
        let walls = walls(rect(0.0, 0.0, 20.0, 0.6), 2);
        assert_eq!(walls.toolpaths.len(), 1);
        let line = &walls.toolpaths[0][0];
        assert!(line.is_odd && !line.is_closed);
        assert!(
            line.junctions
                .iter()
                .all(|j| j.w == 600_000 && j.p.y == 300_000)
        );
        // From half its width in from one end to the same from the other.
        assert_eq!(line.get_length(), 19_400_000);
        assert!(walls.inner_contour.is_empty());
    }

    #[test]
    fn a_strip_between_gets_a_loop_and_a_line_down_the_middle() {
        let walls = walls(rect(0.0, 0.0, 20.0, 1.5), 2);
        assert_eq!(walls.toolpaths.len(), 2);
        let outer = &walls.toolpaths[0][0];
        assert!(outer.is_closed && outer.junctions.iter().all(|j| j.w == 420_000));
        // What the two sides of the loop leave: 1.5 - 2 * 0.42.
        let middle = &walls.toolpaths[1][0];
        assert!(middle.is_odd && !middle.is_closed);
        assert!(
            middle
                .junctions
                .iter()
                .all(|j| j.w == 660_000 && j.p.y == 750_000)
        );
        assert!(walls.inner_contour.is_empty());
    }

    #[test]
    fn a_wedge_changes_from_two_lines_to_one_along_its_length() {
        // 30 mm long, 1.6 mm wide at one end and 0.5 at the other.
        let wedge = ExPolygon {
            contour: Polygon {
                points: [(0.0, 0.0), (30.0, 0.55), (30.0, 1.05), (0.0, 1.6)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        };
        let walls = walls(wedge, 2);
        let all: Vec<&ExtrusionLine> = walls.toolpaths.iter().flatten().collect();
        assert!(!all.is_empty());
        // Widths change along the lines. Where one line becomes two
        // each starts at about three quarters of its full width (the
        // narrowest allowed sets where that happens, not a floor), and
        // none is wider than two lines' worth.
        let widths: Vec<Coord> = all
            .iter()
            .flat_map(|line| line.junctions.iter().map(|j| j.w))
            .collect();
        let (least, most) = (*widths.iter().min().unwrap(), *widths.iter().max().unwrap());
        assert!(
            least >= 300_000 && most <= 900_000 && most > least,
            "{least}..{most}"
        );
        // Everything laid is inside the wedge.
        assert!(
            all.iter()
                .flat_map(|line| &line.junctions)
                .all(|j| { (0..=30_000_000).contains(&j.p.x) && (0..=1_600_000).contains(&j.p.y) })
        );
        // The same again, the same.
        let again = generate(
            &[ExPolygon {
                contour: Polygon {
                    points: [(0.0, 0.0), (30.0, 0.55), (30.0, 1.05), (0.0, 1.6)]
                        .map(|(x, y)| Point2::from_mm(x, y))
                        .to_vec(),
                },
                holes: Vec::new(),
            }],
            &params(2),
        )
        .unwrap();
        assert_eq!(again, walls);
    }

    #[test]
    fn a_ring_is_walled_inside_and_out() {
        let mut ring = rect(0.0, 0.0, 20.0, 20.0);
        ring.holes.push(Polygon {
            points: [(5.0, 5.0), (5.0, 15.0), (15.0, 15.0), (15.0, 5.0)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        });
        let walls = walls(ring, 2);
        assert_eq!(walls.toolpaths.len(), 2);
        // An outer and an inner loop at each depth.
        assert!(walls.toolpaths.iter().all(|lines| lines.len() == 2));
        assert!(walls.toolpaths.iter().flatten().all(|line| line.is_closed));
        // The walls go round the hole's corners in arcs, 0.87 mm out,
        // so the inside has that much more at each than a square corner.
        let inside: f64 = walls.inner_contour.iter().map(ExPolygon::area_mm2).sum();
        let corners = 4.0 * 0.87 * 0.87 * (1.0 - std::f64::consts::FRAC_PI_4);
        let expected = 18.26 * 18.26 - 11.74 * 11.74 + corners;
        assert!(
            (inside - expected).abs() < 0.05,
            "{inside} against {expected}"
        );
    }
}
