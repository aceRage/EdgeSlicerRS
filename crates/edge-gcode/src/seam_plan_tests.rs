//! Where seams go: the measures taken at each point of an outer wall,
//! how two points are told apart, the point picked, and where the walls
//! behind start.

// A list of one layer's place reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use edge_extrusion::{Flow, Role};
use edge_geom::{ExPolygon, Point2, Polygon, unscaled};

use super::*;

fn ring(corners: &[(f64, f64)]) -> Polygon {
    Polygon {
        points: corners
            .iter()
            .map(|&(x, y)| Point2::from_mm(x, y))
            .collect(),
    }
}

fn square(x: f64, y: f64, side: f64) -> Polygon {
    ring(&[(x, y), (x + side, y), (x + side, y + side), (x, y + side)])
}

/// An L: the corner at (5, 5) folds inward, the other five outward.
fn ell() -> Polygon {
    ring(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 5.0),
        (5.0, 5.0),
        (5.0, 10.0),
        (0.0, 10.0),
    ])
}

fn circle(radius: f64, sides: usize, turn: f64) -> Polygon {
    Polygon {
        points: (0..sides)
            .map(|step| {
                let angle = turn + step as f64 * std::f64::consts::TAU / sides as f64;
                Point2::from_mm(50.0 + radius * angle.cos(), 50.0 + radius * angle.sin())
            })
            .collect(),
    }
}

fn wall(polygon: Polygon, depth: usize, is_hole: bool) -> ExtrusionLoop {
    let width = if depth == 0 { 0.42 } else { 0.45 };
    ExtrusionLoop {
        polygon,
        role: if depth == 0 {
            Role::ExternalPerimeter
        } else {
            Role::Perimeter
        },
        is_hole,
        depth,
        flow: Flow::new(width, 0.2, 0.4).unwrap(),
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
    }
}

fn layer<'a>(z: f64, loops: &'a [ExtrusionLoop], below: &'a [ExPolygon]) -> Layer<'a> {
    Layer {
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        tower: &[],
        around: &[],
        support: &[],
        loops,
        islands: &[],
        paths: &[],
        shelter: &[],
        below,
    }
}

fn settings(seam: Seam) -> [PrintSettings; 2] {
    let one = PrintSettings {
        seam,
        ..PrintSettings::default()
    };
    [one.clone(), one]
}

/// A plan of layers each at its own height.
fn plan(layers: &[Layer<'_>], seam: Seam) -> Plan {
    let groups: Vec<Range<usize>> = (0..layers.len()).map(|at| at..at + 1).collect();
    Plan::new(layers, &groups, &settings(seam), &SeamScene::default())
}

/// The candidates of the plan's only object on a layer.
fn seams(plan: &Plan, layer: usize) -> &LayerSeams {
    &plan.objects[0].layers[layer]
}

fn at(point: Point2) -> (f64, f64) {
    (unscaled(point.x), unscaled(point.y))
}

fn start(plan: &Plan, layer: usize, wall: &ExtrusionLoop) -> (f64, f64) {
    at(plan.place(layer, wall, false, None, false).unwrap().start)
}

fn near(found: (f64, f64), wanted: (f64, f64)) -> bool {
    (found.0 - wanted.0).hypot(found.1 - wanted.1) < 2e-6
}

fn candidate(x: f32, y: f32) -> Candidate {
    Candidate {
        position: [x, y, 0.1],
        perimeter: 0,
        angle: 0.0,
        visibility: 0.0,
        overhang: 0.0,
        unsupported: 0.0,
        embedded: 0.0,
        part_joint: false,
        central_part_joint: false,
    }
}

#[test]
fn inward_corners_score_best_and_straight_runs_worst() {
    let quarter = std::f32::consts::FRAC_PI_2;
    assert!(angle_penalty(-quarter) < angle_penalty(quarter));
    assert!(angle_penalty(quarter) < angle_penalty(0.0));
    assert!((angle_penalty(0.0) - (1.0 + 1.0 / 3.0)).abs() < 1e-6);
}

#[test]
fn the_angle_at_a_point_is_taken_an_arms_length_either_side() {
    let quarter = std::f32::consts::FRAC_PI_2;
    // A square turns a quarter left at every corner.
    let corners = [
        (0, 0),
        (10_000_000, 0),
        (10_000_000, 10_000_000),
        (0, 10_000_000),
    ];
    let angles = polygon_angles(&corners, &[10.0; 4], 0.4);
    assert!(angles.iter().all(|angle| (angle - quarter).abs() < 1e-6));
    // A point on a straight run turns nothing, and a corner with a
    // point close by on each side is still read as the corner it is:
    // the arms reach past them.
    let run = [
        (0, 0),
        (5_000_000, 0),
        (9_900_000, 0),
        (10_000_000, 0),
        (10_000_000, 100_000),
        (10_000_000, 10_000_000),
        (0, 10_000_000),
    ];
    let lengths = [5.0, 4.9, 0.1, 0.1, 9.9, 10.0, 10.0];
    let angles = polygon_angles(&run, &lengths, 0.4);
    assert_eq!(angles[1], 0.0);
    assert!((angles[3] - quarter).abs() < 1e-6);
    // The points beside it see most of the turn too.
    assert!(angles[2] > 0.2 && angles[4] > 0.2);
    // A loop of no length has no angles.
    assert_eq!(polygon_angles(&[(5, 5); 3], &[0.0; 3], 0.4), [0.0; 3]);
}

#[test]
fn a_point_hangs_out_by_how_far_its_line_is_past_a_45_degree_slope() {
    // A square wall 20 mm a side over a plate that stops 0.1 mm inside
    // it on the left, 1 mm outside it on the right, and well clear above
    // and below.
    let loops = [
        wall(square(0.0, 0.0, 20.0), 0, false),
        // A wall behind another is no candidate.
        wall(square(0.5, 0.5, 19.0), 1, false),
    ];
    let plate = [ExPolygon {
        contour: ring(&[(0.1, -5.0), (21.0, -5.0), (21.0, 25.0), (0.1, 25.0)]),
        holes: Vec::new(),
    }];
    let layers = [layer(0.2, &loops, &[]), layer(0.4, &loops, &plate)];
    let plan = plan(&layers, Seam::Aligned);
    let found = seams(&plan, 1);
    assert_eq!((found.perimeters.len(), found.points.len()), (1, 4));
    assert_eq!(found.perimeters[0].flow_width, 0.42);
    // Counter-clockwise from the point after the lowest, of two as low
    // the later, at the height the layer was sliced at.
    let places: Vec<[f32; 3]> = found.points.iter().map(|point| point.position).collect();
    assert_eq!(
        places,
        [
            [20.0, 20.0, 0.3],
            [0.0, 20.0, 0.3],
            [0.0, 0.0, 0.3],
            [20.0, 0.0, 0.3]
        ]
    );
    // The left corners are 0.1 mm outside the plate: 0.1 + 0.65 of the
    // width less the layer's height hangs out, and 0.1 + 0.4 of the
    // width is unsupported. The right ones are a mm inside.
    for corner in [1, 2] {
        let point = found.points[corner];
        assert!((point.overhang - (0.1 + 0.65 * 0.42 - 0.2)).abs() < 1e-5);
        assert!((point.unsupported - (0.1 + 0.4 * 0.42)).abs() < 1e-5);
    }
    for corner in [0, 3] {
        assert_eq!(found.points[corner].overhang, 0.0);
        assert!((found.points[corner].unsupported - (-1.0 + 0.4 * 0.42)).abs() < 1e-5);
    }
    // So the seam is at the first corner that rests, and the wall
    // behind starts beside it.
    assert!(near(start(&plan, 1, &loops[0]), (20.0, 20.0)));
    assert!(near(start(&plan, 1, &loops[1]), (19.5, 19.5)));
    // The same wall is given on the layer beneath, where it has
    // candidates of its own: nothing hangs out on the first layer.
    assert!(
        seams(&plan, 0)
            .points
            .iter()
            .all(|point| point.overhang == 0.0)
    );
    assert!(near(start(&plan, 0, &loops[0]), (20.0, 20.0)));
}

#[test]
fn a_holes_candidates_run_the_other_way_round_and_its_corners_fold_inward() {
    let hole = [wall(
        ring(&[(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)]),
        0,
        true,
    )];
    let layers = [layer(0.2, &hole, &[])];
    let plan = plan(&layers, Seam::Aligned);
    let found = seams(&plan, 0);
    let places: Vec<(f32, f32)> = (found.points.iter())
        .map(|point| (point.position[0], point.position[1]))
        .collect();
    // The hole's points run clockwise, the last of its lowest the
    // first given: its candidates start at the point after that one
    // and run the other way.
    assert_eq!(places, [(0.0, 10.0), (0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
    let quarter = std::f32::consts::FRAC_PI_2;
    assert!(
        (found.points.iter()).all(|point| (point.angle + quarter).abs() < 1e-6),
        "{:?}",
        found.points
    );
}

#[test]
fn places_are_measured_from_the_middle_of_the_object_they_are_of() {
    // Two squares, each an object of its own.
    let loops = [
        wall(square(100.0, 100.0, 20.0), 0, false),
        wall(square(150.0, 100.0, 10.0), 0, false),
        wall(square(150.5, 100.5, 9.0), 1, false),
    ];
    let object = |x: f64, side: f64| SeamObject {
        min: Point2::from_mm(x, 100.0),
        max: Point2::from_mm(x + side, 100.0 + side),
        centre: Point2::from_mm(x + side / 2.0, 100.0 + side / 2.0),
        mesh: None,
        parts: &[],
        alone: false,
    };
    let objects = [object(100.0, 20.0), object(150.0, 10.0)];
    let layers = [layer(0.2, &loops, &[])];
    let plan = Plan::new(
        &layers,
        std::slice::from_ref(&(0..1)),
        &settings(Seam::Back),
        &SeamScene {
            objects: &objects,
            ..SeamScene::default()
        },
    );
    let first = &plan.objects[0].layers[0];
    assert_eq!(first.points[0].position, [10.0, 10.0, 0.1]);
    assert_eq!(first.points[2].position, [-10.0, -10.0, 0.1]);
    let second = &plan.objects[1].layers[0];
    assert_eq!(second.points[0].position, [5.0, 5.0, 0.1]);
    // Each wall starts where it is on the bed, and the wall behind the
    // second one beside that one's seam, not the first's.
    assert!(near(start(&plan, 0, &loops[0]), (120.0, 120.0)));
    assert!(near(start(&plan, 0, &loops[1]), (160.0, 110.0)));
    assert!(near(start(&plan, 0, &loops[2]), (159.5, 109.5)));
}

#[test]
fn what_hangs_out_is_weighed_first_then_what_is_hidden_then_the_score() {
    let aligned = Comparator::new(Seam::Aligned);
    let plain = candidate(0.0, 0.0);
    let sharp = Candidate {
        angle: -1.5,
        ..plain
    };
    let better = |a: &Candidate, b: &Candidate| aligned.is_first_better(a, b, [0.0; 2]);
    // An inward corner beats a straight run.
    assert!(better(&sharp, &plain) && !better(&plain, &sharp));
    // Unless it hangs out: of two points either of which does, the one
    // that hangs out less is the better, whatever else.
    let over = Candidate {
        overhang: 0.01,
        ..sharp
    };
    assert!(better(&plain, &over) && !better(&over, &plain));
    let further = Candidate {
        overhang: 0.2,
        ..sharp
    };
    assert!(better(&over, &further) && !better(&further, &over));
    // A point more than half a mm inside the print beats one that is
    // not, by any score.
    let hidden = Candidate {
        embedded: -0.6,
        ..plain
    };
    assert!(better(&hidden, &sharp) && !better(&sharp, &hidden));
    // Visibility counts whole and the angle for 0.6; for the nearest
    // corner the angle counts whole.
    let seen = Candidate {
        visibility: 0.5,
        ..sharp
    };
    assert!(better(&sharp, &seen));
    let gain = angle_penalty(0.0) - angle_penalty(-1.5);
    let hardly = Candidate {
        visibility: 0.6 * gain - 0.01,
        ..sharp
    };
    let barely = Candidate {
        visibility: 0.6 * gain + 0.01,
        ..sharp
    };
    assert!(better(&hardly, &plain) && better(&plain, &barely));
    let nearest = Comparator::new(Seam::Nearest);
    assert!(nearest.is_first_better(&barely, &plain, [0.0; 2]));
    // The nearest corner: of two alike, the one by the nozzle.
    let far = candidate(30.0, 0.0);
    assert!(nearest.is_first_better(&plain, &far, [1.0, 0.0]));
    assert!(nearest.is_first_better(&far, &plain, [29.0, 0.0]));
    assert!(!aligned.is_first_better(&far, &plain, [29.0, 0.0]));
    // At the back the point further back is the better whatever its
    // score, short of hanging out.
    let back = Comparator::new(Seam::Back);
    let behind = candidate(0.0, 5.0);
    assert!(back.is_first_better(&behind, &sharp, [0.0; 2]));
    assert!(!back.is_first_better(&sharp, &behind, [0.0; 2]));
    let behind_over = Candidate {
        overhang: 0.1,
        ..behind
    };
    assert!(back.is_first_better(&sharp, &behind_over, [0.0; 2]));
}

#[test]
fn a_point_is_no_worse_by_much_within_the_allowances() {
    let aligned = Comparator::new(Seam::Aligned);
    let plain = candidate(0.0, 0.0);
    let worse = |a: &Candidate, b: &Candidate| aligned.is_first_not_much_worse(a, b, 0.42);
    // Hanging out by a tenth of the line's width more is no worse.
    let little = Candidate {
        overhang: 0.04,
        ..plain
    };
    let more = Candidate {
        overhang: 0.05,
        ..plain
    };
    assert!(worse(&little, &plain) && worse(&plain, &little));
    assert!(!worse(&more, &plain) && worse(&plain, &more));
    // A score 0.3 worse is no worse by much.
    let seen = Candidate {
        visibility: 0.29,
        ..plain
    };
    let much = Candidate {
        visibility: 0.31,
        ..plain
    };
    assert!(worse(&seen, &plain) && !worse(&much, &plain) && worse(&plain, &much));
    // A hidden point is never worse than one that is not.
    let hidden = Candidate {
        embedded: -0.6,
        visibility: 5.0,
        ..plain
    };
    assert!(worse(&hidden, &plain) && !worse(&plain, &hidden));
    // At the back, a point up to 1.5 mm further forward is no worse.
    let back = Comparator::new(Seam::Back);
    let forward = candidate(0.0, -1.4);
    let well_forward = candidate(0.0, -1.6);
    assert!(back.is_first_not_much_worse(&forward, &plain, 0.42));
    assert!(!back.is_first_not_much_worse(&well_forward, &plain, 0.42));
    // For a seam put anywhere only what hangs out and what is hidden
    // are weighed.
    let random = Comparator::new(Seam::Random);
    assert!(random.is_first_not_much_worse(&much, &plain, 0.42));
    assert!(!random.is_first_not_much_worse(&more, &plain, 0.42));
}

#[test]
fn the_seam_goes_to_the_best_point_the_first_of_those_as_good() {
    let outer = [wall(ell(), 0, false)];
    let layers = [layer(0.2, &outer, &[])];
    // The inward corner hides a seam best, wherever the nozzle is.
    for seam in [Seam::Aligned, Seam::AlignedBack, Seam::Nearest] {
        let plan = plan(&layers, seam);
        assert!(near(start(&plan, 0, &outer[0]), (5.0, 5.0)), "{seam:?}");
    }
    // The back ignores the angle: the points furthest back, and of the
    // two the first.
    assert!(near(
        start(&plan(&layers, Seam::Back), 0, &outer[0]),
        (5.0, 10.0)
    ));

    // A square's corners are all alike: the first, or by the nozzle,
    // or the first of those furthest back.
    let outer = [wall(square(0.0, 0.0, 10.0), 0, false)];
    let layers = [layer(0.2, &outer, &[])];
    assert!(near(
        start(&plan(&layers, Seam::Aligned), 0, &outer[0]),
        (10.0, 10.0)
    ));
    assert!(near(
        start(&plan(&layers, Seam::Back), 0, &outer[0]),
        (10.0, 10.0)
    ));
    let nearest = plan(&layers, Seam::Nearest);
    let by = |x: f64, y: f64| {
        let nozzle = Some(Point2::from_mm(x, y));
        at(nearest
            .place(0, &outer[0], false, nozzle, false)
            .unwrap()
            .start)
    };
    assert!(near(by(-1.0, 11.0), (0.0, 10.0)));
    assert!(near(by(11.0, 9.0), (10.0, 10.0)));
    // The setups that lean to a side go to a corner on it: what faces
    // the other way counts as easier seen.
    let leant = |seam: Seam| start(&plan(&layers, seam), 0, &outer[0]);
    assert!(leant(Seam::AlignedBack).1 > 9.0);
    assert!(leant(Seam::AlignedFront).1 < 1.0);
    assert!(leant(Seam::Left).0 < 1.0);
    assert!(leant(Seam::Right).0 > 9.0);
}

#[test]
fn a_point_hidden_where_two_materials_meet_takes_the_seam() {
    // Two blocks side by side in two filaments; the first has a point
    // half way up the side where they meet.
    let first = [wall(
        ring(&[
            (0.21, 0.21),
            (9.79, 0.21),
            (9.79, 5.0),
            (9.79, 9.79),
            (0.21, 9.79),
        ]),
        0,
        false,
    )];
    let second = [wall(
        ring(&[(10.21, 0.21), (19.79, 0.21), (19.79, 9.79), (10.21, 9.79)]),
        0,
        false,
    )];
    let whole = [ExPolygon {
        contour: ring(&[(0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (0.0, 10.0)]),
        holes: Vec::new(),
    }];
    let parts = [
        layer(0.2, &first, &[]),
        Layer {
            filament: 1,
            ..layer(0.2, &second, &[])
        },
    ];
    let outlines: [&[ExPolygon]; 1] = [&whole];
    let scene = SeamScene {
        outlines: &outlines,
        ..SeamScene::default()
    };
    let groups = [0..2];
    let plan = Plan::new(&parts, &groups, &settings(Seam::Aligned), &scene);
    let found = seams(&plan, 0);
    // The point is 5 mm inside the layer's outline, less 0.65 of the
    // line's width; the corners are a fifth of a mm inside.
    assert!((found.points[0].embedded - (-5.0 + 0.65 * 0.42)).abs() < 1e-5);
    assert!((found.points[1].embedded - (-0.21 + 0.65 * 0.42)).abs() < 1e-5);
    assert!(near(start(&plan, 0, &first[0]), (9.79, 5.0)));
    // With one material, or with no outline given, nothing is hidden.
    let alone = Plan::new(&parts[..1], &[0..1], &settings(Seam::Aligned), &scene);
    assert!(
        seams(&alone, 0)
            .points
            .iter()
            .all(|point| point.embedded == 0.0)
    );
    let unknown = Plan::new(
        &parts,
        &groups,
        &settings(Seam::Aligned),
        &SeamScene::default(),
    );
    assert!(
        seams(&unknown, 0)
            .points
            .iter()
            .all(|point| point.embedded == 0.0)
    );
    assert!(near(start(&unknown, 0, &first[0]), (9.79, 9.79)));
}

#[test]
fn a_random_seam_is_on_the_loop_and_moves_from_layer_to_layer() {
    let outer = [wall(square(0.0, 0.0, 20.0), 0, false)];
    let layers: Vec<Layer<'_>> = (1..=40)
        .map(|index| layer(0.2 * f64::from(index), &outer, &[]))
        .collect();
    let random = plan(&layers, Seam::Random);
    let on_loop = |(x, y): (f64, f64)| {
        let edge = |v: f64| v.abs() < 1e-3 || (v - 20.0).abs() < 1e-3;
        (edge(x) && (-1e-3..=20.001).contains(&y)) || (edge(y) && (-1e-3..=20.001).contains(&x))
    };
    let picked: Vec<(f64, f64)> = (0..40).map(|at| start(&random, at, &outer[0])).collect();
    assert!(picked.iter().all(|&point| on_loop(point)));
    // The same for the same loop and height; not all in one place.
    let again = plan(&layers, Seam::Random);
    assert!((0..40).all(|at| start(&again, at, &outer[0]) == picked[at]));
    let mut distinct = picked.clone();
    distinct.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    distinct.dedup();
    assert!(distinct.len() > 30, "{}", distinct.len());
    // Spread over all four sides.
    assert!(picked.iter().any(|p| p.1 < 1e-3));
    assert!(picked.iter().any(|p| p.0 > 19.999));
    assert!(picked.iter().any(|p| p.1 > 19.999));
    assert!(picked.iter().any(|p| p.0 < 1e-3));
}

#[test]
fn a_random_seam_keeps_off_what_hangs_out() {
    let random = Comparator::new(Seam::Random);
    let corners = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
    let picked = |overhang: &dyn Fn(usize) -> f32| -> Vec<[f32; 3]> {
        (0..40)
            .map(|layer| {
                let candidates: Vec<Candidate> = (corners.iter().enumerate())
                    .map(|(corner, &(x, y))| Candidate {
                        position: [x, y, 0.1 + 0.2 * layer as f32],
                        overhang: overhang(corner),
                        ..candidate(x, y)
                    })
                    .collect();
                pick_random(&candidates, 0.42, &random).1
            })
            .collect()
    };
    // The two left corners hang out by half a mm: only the stretches
    // that set off from the right ones are picked among, the right side
    // and the top.
    let found = picked(&|corner| if corner == 0 || corner == 3 { 0.5 } else { 0.0 });
    assert!(found.iter().all(|p| p[0] > 19.999 || p[1] > 19.999));
    assert!(found.iter().any(|p| p[0] < 19.0));
    assert!(found.iter().any(|p| p[1] < 19.0));
    // A point that hangs out by less than a tenth of the line's width
    // more than another is as good as it: with all four out by about as
    // much, every side is picked from.
    let found = picked(&|corner| 0.3 + 0.01 * corner as f32);
    assert!(found.iter().any(|p| p[1] < 1e-3));
    assert!(found.iter().any(|p| p[0] < 1e-3));
    // With one corner that rests among three well out, the seam is on
    // the stretch that sets off from it.
    let found = picked(&|corner| if corner == 2 { 0.0 } else { 0.4 });
    assert!(found.iter().all(|p| p[1] > 19.999));
}

#[test]
fn a_round_wall_keeps_its_seam_in_line_with_the_one_beneath() {
    // Each layer's wall has its corners elsewhere: turned a third of a
    // side on from the one beneath.
    let walls: Vec<[ExtrusionLoop; 1]> = (0..12)
        .map(|at| [wall(circle(20.0, 90, 0.023 * f64::from(at)), 0, false)])
        .collect();
    let layers: Vec<Layer<'_>> = (walls.iter().enumerate())
        .map(|(at, loops)| layer(0.2 * (at + 1) as f64, loops, &[]))
        .collect();
    for seam in [Seam::Aligned, Seam::Back] {
        let plan = plan(&layers, seam);
        let starts: Vec<(f64, f64)> = (0..12).map(|at| start(&plan, at, &walls[at][0])).collect();
        for pair in starts.windows(2) {
            let moved = (pair[0].0 - pair[1].0).hypot(pair[0].1 - pair[1].1);
            assert!(moved < 0.75, "{seam:?} moved {moved} mm");
        }
    }
    // At the back the line sets out from the seam furthest back of
    // all, and no seam on it is more than 1.5 mm further forward than
    // its own layer's furthest back.
    let back = plan(&layers, Seam::Back);
    let ys: Vec<f64> = (0..12)
        .map(|at| start(&back, at, &walls[at][0]).1)
        .collect();
    assert!(ys.iter().any(|y| *y > 69.98), "{ys:?}");
    assert!(ys.iter().all(|y| *y > 68.5), "{ys:?}");
    // A seam picked by where the nozzle is is not kept in line.
    let nearest = plan(&layers, Seam::Nearest);
    assert!(seams(&nearest, 5).perimeters[0].finalized.is_none());
}

#[test]
fn a_wall_behind_starts_beside_the_seam_and_in_an_inward_corner_at_its_own_corner() {
    // An L with a wall behind it 0.435 mm in.
    let loops = [
        wall(ell(), 0, false),
        wall(
            ring(&[
                (0.435, 0.435),
                (9.565, 0.435),
                (9.565, 4.565),
                (4.565, 4.565),
                (4.565, 9.565),
                (0.435, 9.565),
            ]),
            1,
            false,
        ),
    ];
    let layers = [layer(0.2, &loops, &[])];
    let aligned = plan(&layers, Seam::Aligned);
    // The seam is in the inward corner, and the wall behind starts in
    // its own.
    assert!(near(start(&aligned, 0, &loops[0]), (5.0, 5.0)));
    assert!(near(start(&aligned, 0, &loops[1]), (4.565, 4.565)));
    // At the back the seam is at an outward corner, and the wall behind
    // starts at the point nearest it.
    let back = plan(&layers, Seam::Back);
    assert!(near(start(&back, 0, &loops[0]), (5.0, 10.0)));
    assert!(near(start(&back, 0, &loops[1]), (4.565, 9.565)));
    // Started on round from each other, the wall behind sets off its
    // own width on, the way it is laid: more than its depth square to
    // the wall, which is the distance to the seam brought back by the
    // corner's angle.
    let on = |turned: bool| at(back.place(0, &loops[1], turned, None, true).unwrap().start);
    assert!(near(on(false), (4.565 - 0.45, 9.565)));
    assert!(near(on(true), (4.565, 9.565 - 0.45)));
    // An outermost wall is not moved on.
    let outer = back.place(0, &loops[0], false, None, true).unwrap();
    assert!(near(at(outer.start), (5.0, 10.0)));
    // The nearest corner is picked on the outer wall by where the
    // nozzle is, for the wall behind as for the outer wall itself.
    let nearest = plan(&layers, Seam::Nearest);
    let nozzle = Some(Point2::from_mm(11.0, -1.0));
    let outer = nearest.place(0, &loops[0], false, nozzle, false).unwrap();
    let inner = nearest.place(0, &loops[1], false, nozzle, false).unwrap();
    assert!(near(at(outer.start), (10.0, 0.0)));
    assert!(near(at(inner.start), (9.565, 0.435)));
}

#[test]
fn a_wall_with_no_outer_wall_on_its_layer_has_no_place() {
    let loops = [wall(square(0.5, 0.5, 9.0), 1, false)];
    let layers = [layer(0.2, &loops, &[])];
    let plan = plan(&layers, Seam::Aligned);
    assert_eq!(plan.place(0, &loops[0], false, None, false), None);
}

#[test]
fn the_points_near_a_place_are_found_in_their_own_order() {
    let points: Vec<Candidate> = (0..100)
        .map(|at| candidate((at % 10) as f32 * 3.0, (at / 10) as f32 * 3.0))
        .collect();
    let grid = Grid::new(&points);
    assert_eq!(
        grid.nearby(&points, [9.0, 9.0, 0.1], 3.5),
        [23, 32, 33, 34, 43]
    );
    // Nearer than the reach, not as near.
    assert_eq!(grid.nearby(&points, [9.0, 9.0, 0.1], 3.0), [33]);
    assert!(grid.nearby(&points, [100.0, 9.0, 0.1], 3.0).is_empty());
    assert_eq!(grid.closest(&points, [9.4, 8.9, 0.1]), Some(33));
    // However far off, and the first of two as near.
    assert_eq!(grid.closest(&points, [-50.0, -40.0, 0.1]), Some(0));
    assert_eq!(grid.closest(&points, [200.0, 13.5, 0.1]), Some(49));
    assert_eq!(grid.closest(&points, [1.5, 0.0, 0.1]), Some(0));
    assert_eq!(Grid::new(&[]).closest(&[], [0.0; 3]), None);
}

#[test]
fn the_distance_to_the_outline_beneath_is_told_however_far() {
    let plate = [ExPolygon {
        contour: ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
        holes: vec![ring(&[(4.0, 4.0), (4.0, 6.0), (6.0, 6.0), (6.0, 4.0)])],
    }];
    let below = Below::new(&plate, 0.6).unwrap();
    let near = |found: f64, wanted: f64| (found - wanted).abs() < 1e-9;
    // Over the plate, over its hole, and off it by a little and a lot.
    assert!(near(below.distance(2.0, 5.0), -2.0));
    assert!(near(below.distance(5.0, 5.0), 1.0));
    assert!(near(below.distance(10.25, 5.0), 0.25));
    assert!(near(below.distance(40.0, 5.0), 30.0));
    assert!(near(below.distance(13.0, 14.0), 5.0));
    assert!(near(below.distance(-30.0, -40.0), 50.0));
    // Where the nearer measure tells them apart the two agree.
    assert!(near(below.signed_distance(10.25, 5.0), 0.25));
    assert!(near(below.signed_distance(2.0, 5.0), -0.6));
}

#[test]
fn seams_are_lined_up_along_a_curve_and_a_sharp_corner_keeps_its_own() {
    // A round wall whose corners are elsewhere on every layer: once
    // lined up the seams lie along a smooth line, nearer each other
    // than the corners they were picked at.
    let walls: Vec<[ExtrusionLoop; 1]> = (0..30)
        .map(|at| {
            let turn = f64::from(at % 2).to_radians();
            [wall(circle(20.0, 90, turn), 0, false)]
        })
        .collect();
    let layers: Vec<Layer<'_>> = (walls.iter().enumerate())
        .map(|(at, loops)| layer(0.2 * (at + 1) as f64, loops, &[]))
        .collect();
    let aligned = plan(&layers, Seam::Aligned);
    assert!((0..30).all(|at| seams(&aligned, at).perimeters[0].finalized.is_some()));
    let starts: Vec<(f64, f64)> = (0..30)
        .map(|at| start(&aligned, at, &walls[at][0]))
        .collect();
    // The corners of one layer are 1.4 mm apart and turned a degree,
    // 0.35 mm, from the next layer's, so the seams as picked go to and
    // fro by that; lined up, those in the middle of the line are
    // within a twentieth of a mm of each other.
    for pair in starts[8..22].windows(2) {
        let moved = (pair[0].0 - pair[1].0).hypot(pair[0].1 - pair[1].1);
        assert!(moved < 0.05, "moved {moved} mm: {starts:?}");
    }
    // And each is on its wall.
    for (at, point) in starts.iter().enumerate() {
        let off = ((point.0 - 50.0).hypot(point.1 - 50.0) - 20.0).abs();
        assert!(off < 0.03, "layer {at} is {off} mm off its wall");
    }

    // A square's seam is at a sharp corner, which holds it: lined up,
    // every layer's is still at the corner.
    let squares: Vec<[ExtrusionLoop; 1]> = (0..12)
        .map(|at| {
            [wall(
                square(0.0, 0.0, 20.0 + 0.01 * f64::from(at % 2)),
                0,
                false,
            )]
        })
        .collect();
    let layers: Vec<Layer<'_>> = (squares.iter().enumerate())
        .map(|(at, loops)| layer(0.2 * (at + 1) as f64, loops, &[]))
        .collect();
    let aligned = plan(&layers, Seam::Aligned);
    for (at, loops) in squares.iter().enumerate() {
        let side = 20.0 + 0.01 * (at % 2) as f64;
        assert!(seams(&aligned, at).perimeters[0].finalized.is_some());
        assert!(near(start(&aligned, at, &loops[0]), (side, side)));
    }
    // A line of fewer than six seams is left as it was picked.
    let short = plan(&layers[..5], Seam::Aligned);
    assert!((0..5).all(|at| seams(&short, at).perimeters[0].finalized.is_none()));
}

#[test]
fn a_line_of_seams_that_stops_short_of_the_top_takes_in_the_layer_beneath_its_first() {
    // Thirty layers of a square wall; from the 26th on the wall is
    // somewhere else, so a line of seams from below ends at the 25th.
    // The 13th layer's wall reaches a little further back, which makes
    // its seam the best at the back and the one the line sets out from.
    let walls: Vec<[ExtrusionLoop; 1]> = (0..30)
        .map(|at| {
            let side = if at == 12 { 20.05 } else { 20.0 };
            let x = if at >= 25 { 100.0 } else { 0.0 };
            [wall(square(x, 0.0, side), 0, false)]
        })
        .collect();
    let layers: Vec<Layer<'_>> = (walls.iter().enumerate())
        .map(|(at, loops)| layer(0.2 * (at + 1) as f64, loops, &[]))
        .collect();
    let back = plan(&layers, Seam::Back);
    // Every layer up to the 25th is in the line, the 12th with them.
    // (The reference, having looked upward and stopped short of the
    // top, steps down twice before it looks again, and leaves the layer
    // beneath the first seam out.)
    for at in 0..25 {
        assert!(
            seams(&back, at).perimeters[0].finalized.is_some(),
            "layer {at} is not in the line"
        );
    }
    // The five above are too few to line up.
    assert!((25..30).all(|at| seams(&back, at).perimeters[0].finalized.is_none()));
}

/// A box from `min` to `max`, as triangles facing out.
fn block(
    min: [f32; 3],
    max: [f32; 3],
    vertices: &mut Vec<[f32; 3]>,
    triangles: &mut Vec<[u32; 3]>,
) {
    let offset = vertices.len() as u32;
    vertices.extend((0..8).map(|index| -> [f32; 3] {
        std::array::from_fn(|axis| {
            if index >> axis & 1 == 1 {
                max[axis]
            } else {
                min[axis]
            }
        })
    }));
    let faces = [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    triangles.extend(faces.iter().map(|face| face.map(|vertex| vertex + offset)));
}

#[test]
fn a_seam_goes_where_the_parts_surface_is_least_seen() {
    // Two blocks 10 mm a side and 4 mm apart, one object: the faces
    // that look at each other across the gap are seen less than those
    // that look out.
    let (mut vertices, mut triangles) = (Vec::new(), Vec::new());
    block(
        [0.0, 0.0, 0.0],
        [10.0, 10.0, 10.0],
        &mut vertices,
        &mut triangles,
    );
    block(
        [14.0, 0.0, 0.0],
        [24.0, 10.0, 10.0],
        &mut vertices,
        &mut triangles,
    );
    fn object(mesh: Option<SeamMesh<'_>>) -> [SeamObject<'_>; 1] {
        [SeamObject {
            min: Point2::from_mm(0.0, 0.0),
            max: Point2::from_mm(24.0, 10.0),
            centre: Point2::from_mm(12.0, 5.0),
            mesh,
            parts: &[],
            alone: false,
        }]
    }
    let mesh = SeamMesh {
        vertices: &vertices,
        triangles: &triangles,
        negative_from: triangles.len(),
    };
    let loops = [
        wall(square(0.21, 0.21, 9.58), 0, false),
        wall(square(14.21, 0.21, 9.58), 0, false),
    ];
    let layers = [layer(5.0, &loops, &[])];
    let placed = |seam: Seam, mesh: Option<SeamMesh<'_>>| {
        let objects = object(mesh);
        let scene = SeamScene {
            objects: &objects,
            ..SeamScene::default()
        };
        let plan = Plan::new(
            &layers,
            std::slice::from_ref(&(0..1)),
            &settings(seam),
            &scene,
        );
        let nozzle = Some(Point2::from_mm(200.0, 12.0));
        at(plan
            .place(0, &loops[1], false, nozzle, false)
            .unwrap()
            .start)
    };
    // With nothing known of the surface every corner of the second
    // block's wall is as good as another: the first, or the one
    // nearest the nozzle, which is far off to the right; both are on
    // the block's far side.
    assert!(near(placed(Seam::Aligned, None), (23.79, 9.79)));
    assert!(near(placed(Seam::Nearest, None), (23.79, 9.79)));
    // With the surface known the seam is at a corner by the gap.
    assert!(placed(Seam::Aligned, Some(mesh)).0 < 14.3);
    assert!(placed(Seam::Nearest, Some(mesh)).0 < 14.3);
    // The back and anywhere do not go by what is seen.
    assert!(near(placed(Seam::Back, Some(mesh)), (23.79, 9.79)));
    // The candidates carry the measure: less by the gap than outside.
    let objects = object(Some(mesh));
    let scene = SeamScene {
        objects: &objects,
        ..SeamScene::default()
    };
    let plan = Plan::new(
        &layers,
        std::slice::from_ref(&(0..1)),
        &settings(Seam::Aligned),
        &scene,
    );
    let points = &seams(&plan, 0).points[4..8];
    let seen = |x: f32| -> f32 {
        (points.iter())
            .filter(|point| (point.position[0] - x).abs() < 0.01)
            .map(|point| point.visibility)
            .fold(0.0, f32::max)
    };
    // From the object's middle the wall runs from 2.21 to 11.79 mm.
    assert!(
        seen(2.21) < seen(11.79) - 0.1,
        "{} {}",
        seen(2.21),
        seen(11.79)
    );
}

#[test]
fn a_seam_goes_where_two_parts_meet() {
    // Two blocks of one object side by side, touching at x = 10, and the
    // one outer wall round both.
    let (mut vertices, mut triangles) = (Vec::new(), Vec::new());
    block(
        [0.0, 0.0, 0.0],
        [10.0, 10.0, 10.0],
        &mut vertices,
        &mut triangles,
    );
    let (mut second, mut second_triangles) = (Vec::new(), Vec::new());
    block(
        [10.0, 0.0, 0.0],
        [20.0, 10.0, 10.0],
        &mut second,
        &mut second_triangles,
    );
    let parts = [
        SeamPart {
            vertices: &vertices,
            triangles: &triangles,
        },
        SeamPart {
            vertices: &second,
            triangles: &second_triangles,
        },
    ];
    let loops = [wall(
        ring(&[(0.21, 0.21), (19.79, 0.21), (19.79, 9.79), (0.21, 9.79)]),
        0,
        false,
    )];
    let layers: Vec<Layer<'_>> = (0..8)
        .map(|at| layer(0.2 + 0.2 * at as f64, &loops, &[]))
        .collect();
    fn placed(
        layers: &[Layer<'_>],
        seam: Seam,
        joints: bool,
        parts: &[SeamPart<'_>],
    ) -> (Plan, Vec<(f64, f64)>) {
        let objects = [SeamObject {
            min: Point2::from_mm(0.0, 0.0),
            max: Point2::from_mm(20.0, 10.0),
            centre: Point2::from_mm(10.0, 5.0),
            mesh: None,
            parts,
            alone: true,
        }];
        let scene = SeamScene {
            objects: &objects,
            part_joints: joints,
            ..SeamScene::default()
        };
        let groups: Vec<Range<usize>> = (0..layers.len()).map(|at| at..at + 1).collect();
        let plan = Plan::new(layers, &groups, &settings(seam), &scene);
        let starts: Vec<(f64, f64)> = (0..layers.len())
            .map(|at| start(&plan, at, layers[at].loops.first().unwrap()))
            .collect();
        (plan, starts)
    }
    // At every aligned position the seam is on the joint, in the middle
    // of a run of points along one long side, which the edge across the
    // joint was given every fifth of a mm.
    for seam in [Seam::Aligned, Seam::AlignedBack, Seam::Left] {
        let (plan, starts) = placed(&layers, seam, true, &parts);
        for (x, y) in &starts {
            assert!((x - 10.0).abs() < 0.25, "{seam:?} {x} {y}");
            assert!(
                (y - 0.21).abs() < 1e-6 || (y - 9.79).abs() < 1e-6,
                "{seam:?} {y}"
            );
        }
        let layer = seams(&plan, 3);
        assert!(layer.points.len() > 4 + 2 * 90);
        let joint: Vec<&Candidate> = layer.points.iter().filter(|p| p.part_joint).collect();
        assert!(joint.iter().all(|p| p.position[0].abs() < 0.8), "{joint:?}");
        // One middle on each long side.
        let middles: Vec<&Candidate> = (layer.points.iter())
            .filter(|p| p.central_part_joint)
            .collect();
        assert_eq!(middles.len(), 2, "{middles:?}");
    }
    // Turned off, or at a position that does not go by a score, it is
    // not: the first corner as before, or the back.
    let corner = |start: &(f64, f64)| (start.0 - 10.0).abs() > 9.5;
    let (_, starts) = placed(&layers, Seam::Aligned, false, &parts);
    assert!(starts.iter().all(corner), "{starts:?}");
    let (plan, starts) = placed(&layers, Seam::Back, true, &parts);
    assert!(
        starts
            .iter()
            .all(|start| (start.1 - 9.79).abs() < 1e-6 && corner(start))
    );
    assert!(seams(&plan, 3).points.iter().all(|p| !p.part_joint));
    // With one part there is no joint.
    let (_, starts) = placed(&layers, Seam::Aligned, true, &parts[..1]);
    assert!(starts.iter().all(corner), "{starts:?}");
}

#[test]
fn the_middle_of_a_joint_is_a_hidden_point_or_an_inward_corner_or_the_middle() {
    let run = |marks: &[(bool, f32, f32, f32)]| -> Vec<usize> {
        let mut points: Vec<Candidate> = (marks.iter().enumerate())
            .map(|(at, &(joint, embedded, angle, overhang))| Candidate {
                position: [at as f32, 0.0, 0.1],
                part_joint: joint,
                embedded,
                angle,
                overhang,
                ..candidate(0.0, 0.0)
            })
            .collect();
        mark_part_joint_centres(&mut points);
        (0..points.len())
            .filter(|&at| points[at].central_part_joint)
            .collect()
    };
    let plain = (true, 0.0, 0.0, 0.0);
    let off = (false, 0.0, 0.0, 0.0);
    // The middle by length.
    assert_eq!(run(&[off, plain, plain, plain, plain, plain, off]), vec![3]);
    // A run round the end of the wall is one run.
    assert_eq!(run(&[plain, plain, off, off, off, plain, plain]), vec![0]);
    // A point that hangs out splits a run in two.
    let over = (true, 0.0, 0.0, 0.3);
    assert_eq!(
        run(&[off, plain, plain, over, plain, plain, off]),
        vec![2, 5]
    );
    // A hidden point is preferred, and then a sharp inward corner.
    let hidden = (true, -1.0, 0.0, 0.0);
    let corner = (true, 0.0, -1.2, 0.0);
    assert_eq!(
        run(&[off, plain, hidden, plain, plain, plain, off]),
        vec![2]
    );
    assert_eq!(
        run(&[off, corner, plain, plain, plain, plain, off]),
        vec![1]
    );
    // A wall on a joint all the way round has none, nor one with none.
    assert!(run(&[plain; 6]).is_empty());
    assert!(run(&[off; 6]).is_empty());
}
