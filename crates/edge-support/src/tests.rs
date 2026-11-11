use edge_config::Settings;
use edge_geom::{Point2, Polygon};

use super::*;

fn shape(corners: &[(f64, f64)]) -> ExPolygon {
    ExPolygon {
        contour: Polygon {
            points: corners
                .iter()
                .map(|&(x, y)| Point2::from_mm(x, y))
                .collect(),
        },
        holes: Vec::new(),
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
    shape(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn on() -> SupportSettings {
    SupportSettings {
        enabled: true,
        ..SupportSettings::default()
    }
}

fn supports(layers: &[Vec<ExPolygon>], settings: &SupportSettings) -> Vec<Vec<ExtrusionPath>> {
    let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
    generate(&slices, Point2::from_mm(0.0, 0.0), 0.2, 0.2, settings).unwrap()
}

fn points(layer: &[ExtrusionPath]) -> impl Iterator<Item = (f64, f64)> + '_ {
    layer
        .iter()
        .flat_map(|path| &path.points)
        .map(|p| (unscaled(p.x), unscaled(p.y)))
}

/// A 10 mm stem ten layers tall under a 30 mm cap of five layers.
fn mushroom() -> Vec<Vec<ExPolygon>> {
    let mut layers = vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; 10];
    layers.extend(vec![vec![rect(-10.0, -10.0, 20.0, 20.0)]; 5]);
    layers
}

#[test]
fn a_cap_is_held_up_with_a_gap_and_a_dense_top() {
    let found = supports(&mushroom(), &on());
    assert_eq!(found.len(), 15);
    // The cap starts on layer 10. One layer of air under it, then the
    // contact on layer 8; nothing in the cap itself.
    assert!(found[9..].iter().all(Vec::is_empty));
    // The contact and the two layers under it are dense; below is sparse.
    for (layer, role) in [
        (8, Role::SupportInterface),
        (7, Role::SupportInterface),
        (6, Role::SupportInterface),
        (5, Role::Support),
        (1, Role::Support),
        (0, Role::Support),
    ] {
        assert!(!found[layer].is_empty(), "layer {layer} has no support");
        assert!(
            found[layer].iter().all(|path| path.role == role),
            "layer {layer} is not all {role:?}"
        );
    }

    // Sparse lines run along X, 2.5 mm and a line apart; dense ones across them.
    let flow = on().flow(0.2, false).unwrap();
    let rows = |layer: usize, along_x: bool| -> Vec<f64> {
        let mut rows: Vec<f64> = found[layer]
            .iter()
            .flat_map(|path| path.points.windows(2))
            .filter(|pair| (pair[0].y == pair[1].y) == along_x && pair[0] != pair[1])
            .filter(|pair| pair[0].x == pair[1].x || pair[0].y == pair[1].y)
            .map(|pair| unscaled(if along_x { pair[0].y } else { pair[0].x }))
            .collect();
        rows.sort_by(f64::total_cmp);
        rows.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        rows
    };
    let sparse = rows(3, true);
    assert!(sparse.len() > 5, "{} sparse rows", sparse.len());
    assert!(
        sparse
            .windows(2)
            .all(|pair| ((pair[1] - pair[0]) - (flow.spacing + 2.5)).abs() < 1e-3),
        "{sparse:?}"
    );
    let dense = rows(7, false);
    assert!(
        dense
            .windows(2)
            .all(|pair| ((pair[1] - pair[0]) - (flow.spacing + 0.5)).abs() < 1e-3),
        "{dense:?}"
    );

    // Nothing comes nearer the stem than the distance set. The support is
    // squared off to its grid, so it stands a little proud of the cap's
    // footprint, by less than one square of 2.5 mm and a line.
    let square = flow.spacing + 2.5;
    for layer in &found[1..9] {
        for (x, y) in points(layer) {
            let clear = !(-0.34..10.34).contains(&x) || !(-0.34..10.34).contains(&y);
            assert!(clear, "support at {x} x {y} is against the stem");
            let within = -10.0 - square..20.0 + square;
            assert!(within.contains(&x) && within.contains(&y), "{x} x {y}");
        }
    }
    let reach = |layer: usize| {
        points(&found[layer])
            .map(|(x, _)| x)
            .fold(f64::MIN, f64::max)
    };
    assert!(
        reach(3) >= 19.5,
        "support stops at {}, short of the cap's edge",
        reach(3)
    );
    // On the bed the support spreads 2 mm more for a footing, and keeps
    // 0.2 mm from the part.
    let spread = reach(0) - reach(3);
    assert!(spread > 1.5 && spread < 2.3, "first layer spreads {spread}");
    for (x, y) in points(&found[0]) {
        assert!(!(-0.19..10.19).contains(&x) || !(-0.19..10.19).contains(&y));
    }

    assert!(
        supports(&mushroom(), &SupportSettings::default())
            .iter()
            .all(Vec::is_empty)
    );
}

#[test]
fn the_gap_and_the_dense_layers_follow_the_settings() {
    // Twice the air: two empty layers under the cap.
    let roomy = SupportSettings {
        top_z_distance: 0.4,
        ..on()
    };
    let found = supports(&mushroom(), &roomy);
    assert!(found[8].is_empty() && found[9].is_empty() && !found[7].is_empty());
    // No gap: the support comes right up under the cap.
    let tight = SupportSettings {
        top_z_distance: 0.0,
        ..on()
    };
    assert!(!supports(&mushroom(), &tight)[9].is_empty());
    // No dense layers: sparse all the way up.
    let plain = SupportSettings {
        interface_top_layers: 0,
        ..on()
    };
    let found = supports(&mushroom(), &plain);
    assert!(found[8].iter().all(|path| path.role == Role::Support));
    // One dense layer: only the contact.
    let one = SupportSettings {
        interface_top_layers: 1,
        ..on()
    };
    let found = supports(&mushroom(), &one);
    assert!(
        found[8]
            .iter()
            .all(|path| path.role == Role::SupportInterface)
    );
    assert!(found[7].iter().all(|path| path.role == Role::Support));
}

/// A 10 mm square that moves `step` mm along X on every one of 20 layers.
fn slope(step: f64) -> Vec<Vec<ExPolygon>> {
    (0..20)
        .map(|layer| {
            let x = f64::from(layer) * step;
            vec![rect(x, 0.0, x + 10.0, 10.0)]
        })
        .collect()
}

#[test]
fn only_what_leans_past_the_threshold_is_supported() {
    // 0.1 mm a layer rises at 63 degrees from level; 0.5 mm at 22.
    assert!(supports(&slope(0.1), &on()).iter().all(Vec::is_empty));
    let leaning = supports(&slope(0.5), &on());
    assert!(leaning.iter().filter(|layer| !layer.is_empty()).count() > 10);
    // The support stays under the lean, on the side it leans to.
    for (x, _) in points(&leaning[2]) {
        assert!(x > 10.0, "support at x {x} is under the part itself");
    }
    // A lower threshold lets it go.
    let lenient = SupportSettings {
        threshold_angle: 10.0,
        ..on()
    };
    assert!(supports(&slope(0.5), &lenient).iter().all(Vec::is_empty));
    // With no angle, half a wall resting on the layer below is enough.
    let by_overlap = SupportSettings {
        threshold_angle: 0.0,
        ..on()
    };
    assert!(supports(&slope(0.1), &by_overlap).iter().all(Vec::is_empty));
    assert!(!supports(&slope(0.5), &by_overlap).iter().all(Vec::is_empty));
}

/// A plate five layers thick, a post on it to layer 14, and a shelf from
/// the post out over the plate. The plate is wide enough that the squares
/// the support is rounded out to all stand on it.
fn shelf() -> Vec<Vec<ExPolygon>> {
    let mut layers = vec![vec![rect(-5.0, -5.0, 35.0, 15.0)]; 5];
    layers.extend(vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; 10]);
    layers.extend(vec![vec![rect(0.0, 0.0, 30.0, 10.0)]; 5]);
    layers
}

#[test]
fn a_support_standing_on_the_part_stops_short_of_it() {
    let found = supports(&shelf(), &on());
    // The shelf starts on layer 15: contact on 13, air on 14.
    assert!(found[14].is_empty() && !found[13].is_empty());
    // The plate's top is layer 4. One layer of air above it, and nothing
    // inside it.
    assert!(!found[6].is_empty(), "no support above the plate");
    assert!(
        found[..6].iter().all(Vec::is_empty),
        "support in or on the plate"
    );

    // Asked for, the foot of the support is dense for its first layers.
    assert!(found[6].iter().all(|path| path.role == Role::Support));
    let footed = SupportSettings {
        interface_bottom_layers: 2,
        ..on()
    };
    let found = supports(&shelf(), &footed);
    for layer in [6, 7] {
        assert!(
            found[layer]
                .iter()
                .any(|path| path.role == Role::SupportInterface),
            "layer {layer} has no dense foot"
        );
    }
    assert!(found[8].iter().all(|path| path.role == Role::Support));

    // Held to the build plate, there is nowhere for this support to stand.
    let plate_only = SupportSettings {
        on_build_plate_only: true,
        ..on()
    };
    assert!(supports(&shelf(), &plate_only).iter().all(Vec::is_empty));
    // The mushroom's cap is over the bed and is still held up.
    assert!(!supports(&mushroom(), &plate_only)[5].is_empty());
}

#[test]
fn a_nub_is_too_small_to_support_unless_asked() {
    // A stem with a nub 2 mm long and 0.6 mm wide on one side, from layer 10.
    let nub = shape(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 4.7),
        (12.0, 4.7),
        (12.0, 5.3),
        (10.0, 5.3),
        (10.0, 10.0),
        (0.0, 10.0),
    ]);
    let mut layers = vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; 10];
    layers.extend(vec![vec![nub]; 3]);
    assert!(supports(&layers, &on()).iter().all(Vec::is_empty));
    let every = SupportSettings {
        remove_small_overhang: false,
        ..on()
    };
    assert!(!supports(&layers, &every).iter().all(Vec::is_empty));

    // A thin arm 8 mm long is a cantilever and is held up whatever its width.
    let arm = shape(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 4.7),
        (18.0, 4.7),
        (18.0, 5.3),
        (10.0, 5.3),
        (10.0, 10.0),
        (0.0, 10.0),
    ]);
    let mut layers = vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; 10];
    layers.extend(vec![vec![arm]; 3]);
    assert!(!supports(&layers, &on()).iter().all(Vec::is_empty));
}

#[test]
fn supports_move_with_the_part() {
    let moved: Vec<Vec<ExPolygon>> = mushroom()
        .iter()
        .map(|layer| {
            layer
                .iter()
                .map(|region| ExPolygon {
                    contour: Polygon {
                        points: region
                            .contour
                            .points
                            .iter()
                            .map(|p| Point2 {
                                x: p.x + scaled(101.3),
                                y: p.y + scaled(57.7),
                            })
                            .collect(),
                    },
                    holes: Vec::new(),
                })
                .collect()
        })
        .collect();
    let here = supports(&mushroom(), &on());
    let slices: Vec<&[ExPolygon]> = moved.iter().map(Vec::as_slice).collect();
    let there = generate(&slices, Point2::from_mm(101.3, 57.7), 0.2, 0.2, &on()).unwrap();
    // The same lines, 101.3 x 57.7 mm away.
    for (a, b) in here.iter().zip(&there) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            let shifted: Vec<Point2> = a
                .points
                .iter()
                .map(|p| Point2 {
                    x: p.x + scaled(101.3),
                    y: p.y + scaled(57.7),
                })
                .collect();
            assert_eq!(shifted, b.points);
        }
    }
}

#[test]
fn the_same_part_gives_the_same_supports() {
    let once = supports(&shelf(), &on());
    let again = supports(&shelf(), &on());
    assert_eq!(once.len(), again.len());
    for (a, b) in once.iter().zip(&again) {
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b).all(|(a, b)| a.points == b.points));
    }
}

#[test]
fn settings_are_read_and_what_is_not_done_is_said() {
    let read =
        |json: &str| SupportSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
    assert_eq!(read("{}"), SupportSettings::default());
    let normal = read(
        r#"{"enable_support": "1", "support_type": "normal(auto)", "support_threshold_angle": "45",
            "support_on_build_plate_only": "1", "support_object_xy_distance": "0.45",
            "support_top_z_distance": "0.3", "support_base_pattern_spacing": "5",
            "support_interface_top_layers": "2", "support_interface_spacing": "0",
            "support_line_width": "0.42", "raft_first_layer_density": "80%",
            "nozzle_diameter": ["0.4", "0.4"], "support_interface_bottom_layers": "2",
            "support_bottom_interface_spacing": "0.3"}"#,
    );
    assert!(normal.enabled && normal.on_build_plate_only);
    assert_eq!((normal.threshold_angle, normal.xy_distance), (45.0, 0.45));
    assert_eq!((normal.top_z_distance, normal.base_spacing), (0.3, 5.0));
    assert_eq!(
        (normal.interface_top_layers, normal.interface_spacing),
        (2, 0.0)
    );
    assert_eq!(normal.first_layer_density, 0.8);
    assert_eq!(
        (
            normal.interface_bottom_layers,
            normal.bottom_interface_spacing
        ),
        (2, 0.3)
    );
    let same =
        read(r#"{"support_interface_bottom_layers": "-1", "support_interface_top_layers": "4"}"#);
    assert_eq!(same.interface_bottom_layers, 4);
    assert_eq!(normal.flow(0.2, false).unwrap().width, 0.42);
    assert!(normal.unsupported.is_empty(), "{:?}", normal.unsupported);

    assert_eq!(normal.style, Style::Normal);
    let tree = read(r#"{"enable_support": "1", "support_type": "tree(auto)", "raft_layers": "2"}"#);
    assert!(tree.enabled);
    assert_eq!((tree.style, tree.unsupported.len()), (Style::Tree, 1));
    // Painted-only supports cannot be honoured at all, and that is said louder.
    let manual = read(r#"{"enable_support": "1", "support_type": "normal(manual)"}"#);
    assert!(!manual.enabled && manual.unsupported[0].contains("NO supports"));
    // With supports off nothing is noted.
    let off = read(r#"{"enable_support": "0", "support_type": "tree(auto)", "raft_layers": "2"}"#);
    assert!(!off.enabled && off.unsupported.is_empty());
}

#[test]
fn areas_are_squared_off_to_a_grid_once_and_for_all() {
    let cell = scaled(2.0);
    // A blob from 0.5 to 3.5 reaches into two squares each way.
    let snapped = snap(&[rect(0.5, 0.5, 3.5, 3.5)], cell).unwrap();
    assert_eq!(snapped.len(), 1);
    assert_eq!(
        bounds(&snapped[0]),
        [0, 0, scaled(4.0), scaled(4.0)],
        "{snapped:?}"
    );
    assert!((snapped[0].area_mm2() - 16.0).abs() < 1e-9);
    // Squaring off what is already square changes nothing.
    let again = snap(&snapped, cell).unwrap();
    assert_eq!(bounds(&again[0]), bounds(&snapped[0]));
    assert!((again[0].area_mm2() - 16.0).abs() < 1e-9);
    // Blobs in separate squares stay separate; an L keeps its shape.
    let two = snap(&[rect(0.5, 0.5, 1.5, 1.5), rect(6.5, 0.5, 7.5, 1.5)], cell).unwrap();
    assert_eq!(two.len(), 2);
    let ell = shape(&[
        (0.5, 0.5),
        (5.5, 0.5),
        (5.5, 1.5),
        (1.5, 1.5),
        (1.5, 5.5),
        (0.5, 5.5),
    ]);
    let area: f64 = snap(&[ell], cell)
        .unwrap()
        .iter()
        .map(ExPolygon::area_mm2)
        .sum();
    assert!((area - 20.0).abs() < 1e-9, "{area}");
}

fn tree() -> SupportSettings {
    SupportSettings {
        style: Style::Tree,
        ..on()
    }
}

/// A 10 mm stem sixty layers tall under a 30 mm cap.
fn tall_mushroom() -> Vec<Vec<ExPolygon>> {
    let mut layers = vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; 60];
    layers.extend(vec![vec![rect(-10.0, -10.0, 20.0, 20.0)]; 5]);
    layers
}

fn is_loop(path: &ExtrusionPath) -> bool {
    // A branch's wall: closed on the bed, and above it a loop that
    // starts from a hook inside itself, so that its two ends are no
    // further apart than the hook is long.
    let (first, last) = (path.points[0], path.points[path.points.len() - 1]);
    let apart = ((first.x - last.x) as f64).hypot((first.y - last.y) as f64) / 1e6;
    path.points.len() > 4 && apart <= 6.5 * path.flow.spacing
}

#[test]
fn a_tree_holds_up_a_cap_on_branches_that_join_into_trunks() {
    let found = supports(&tall_mushroom(), &tree());
    // The cap starts on layer 60: air on 59, then three layers of roof.
    assert!(found[59..].iter().all(Vec::is_empty));
    for layer in [58, 57, 56] {
        assert!(!found[layer].is_empty(), "no roof on layer {layer}");
        assert!(
            found[layer]
                .iter()
                .all(|path| path.role == Role::SupportInterface && !is_loop(path)),
            "layer {layer} is not all roof"
        );
    }
    // Under the roof, hollow branches all the way to the bed.
    for (layer, paths) in found.iter().enumerate().take(56).skip(1) {
        assert!(!paths.is_empty(), "nothing on layer {layer}");
        assert!(
            paths
                .iter()
                .all(|path| path.role == Role::Support && is_loop(path)),
            "layer {layer} is not all branches"
        );
    }
    // Many tips under the roof; fewer, thicker trunks further down.
    let (tips, middle, low) = (found[55].len(), found[30].len(), found[5].len());
    assert!(tips > 40, "{tips} tips");
    assert!(
        middle < tips && low <= middle,
        "{tips} tips, {middle}, then {low}"
    );
    let girth = |layer: usize| {
        found[layer]
            .iter()
            .map(ExtrusionPath::length_mm)
            .fold(0.0, f64::max)
    };
    assert!(
        girth(5) > 2.0 * girth(55),
        "{} then {}",
        girth(55),
        girth(5)
    );

    // Nothing comes nearer the stem than the distance set.
    for layer in &found[..56] {
        for (x, y) in points(layer) {
            let clear = !(-0.34..10.34).contains(&x) || !(-0.34..10.34).contains(&y);
            assert!(clear, "support at {x} x {y} is against the stem");
        }
    }
    // On the bed the trunks are outlined and filled in.
    assert!(found[0].iter().any(is_loop) && found[0].iter().any(|path| !is_loop(path)));
    assert!(found[0].iter().all(|path| path.role == Role::Support));
}

#[test]
fn a_tree_spreads_out_on_the_bed() {
    let extent = |layer: &[ExtrusionPath]| {
        points(layer).fold(
            [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
            |[a, b, c, d], (x, y)| [a.min(x), b.min(y), c.max(x), d.max(y)],
        )
    };
    let found = supports(&tall_mushroom(), &tree());
    let (bed, above) = (extent(&found[0]), extent(&found[1]));
    // The first layer reaches about the 2 mm of `raft_first_layer_expansion`
    // further out all round than the one above it.
    for (on_bed, over, outward) in [
        (bed[0], above[0], -1.0),
        (bed[1], above[1], -1.0),
        (bed[2], above[2], 1.0),
        (bed[3], above[3], 1.0),
    ] {
        let further = (on_bed - over) * outward;
        assert!((1.7..2.4).contains(&further), "{further}");
    }
    // With none asked for it reaches no further than the flare of the feet.
    let plain = SupportSettings {
        first_layer_expansion: 0.0,
        ..tree()
    };
    let found = supports(&tall_mushroom(), &plain);
    let (bed, above) = (extent(&found[0]), extent(&found[1]));
    assert!((above[0] - bed[0]).abs() < 0.5);
}

#[test]
fn a_tree_stands_on_the_part_where_it_cannot_reach_the_bed() {
    // A wide plate, a post on it and a shelf from the post, far from the
    // plate's edge.
    let mut layers = vec![vec![rect(-30.0, -30.0, 60.0, 40.0)]; 5];
    layers.extend(vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; 40]);
    layers.extend(vec![vec![rect(0.0, 0.0, 30.0, 10.0)]; 5]);
    let found = supports(&layers, &tree());
    // Roof on 43 to 41 under the shelf, branches below it.
    assert!(found[44].is_empty() && !found[43].is_empty());
    assert!(found[30].iter().all(is_loop) && !found[30].is_empty());
    // The plate's top is layer 4: a layer of air, then the branches' feet.
    assert!(!found[6].is_empty(), "no branch stands on the plate");
    assert!(
        found[..6].iter().all(Vec::is_empty),
        "support in or on the plate"
    );
    // The branches stay under the shelf instead of making for the bed.
    for (x, y) in points(&found[10]) {
        assert!(
            (5.0..35.0).contains(&x) && (-5.0..15.0).contains(&y),
            "{x} x {y}"
        );
    }

    // Asked for, a branch's foot on the part is dense for its first layers.
    let footed = SupportSettings {
        interface_bottom_layers: 2,
        ..tree()
    };
    let found = supports(&layers, &footed);
    for layer in [6, 7] {
        assert!(
            found[layer]
                .iter()
                .all(|path| path.role == Role::SupportInterface),
            "layer {layer} is not a dense foot"
        );
    }
    assert!(found[8].iter().all(|path| path.role == Role::Support));

    // Held to the build plate, a tree has nowhere to stand here.
    let plate_only = SupportSettings {
        on_build_plate_only: true,
        ..tree()
    };
    assert!(supports(&layers, &plate_only).iter().all(Vec::is_empty));
    // The mushroom's cap is over the bed and still gets its tree.
    assert!(!supports(&tall_mushroom(), &plate_only)[20].is_empty());
}

#[test]
fn trees_are_the_same_every_time_and_move_with_the_part() {
    let once = supports(&tall_mushroom(), &tree());
    let again = supports(&tall_mushroom(), &tree());
    for (a, b) in once.iter().zip(&again) {
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b).all(|(a, b)| a.points == b.points));
    }
}

/// A wide plate, a post on it `post` layers tall and a shelf from the post.
fn low_shelf(post: usize) -> Vec<Vec<ExPolygon>> {
    let mut layers = vec![vec![rect(-30.0, -30.0, 60.0, 40.0)]; 5];
    layers.extend(vec![vec![rect(0.0, 0.0, 10.0, 10.0)]; post]);
    layers.extend(vec![vec![rect(0.0, 0.0, 30.0, 10.0)]; 5]);
    layers
}

#[test]
fn a_roof_is_never_left_on_nothing() {
    let roofed = |layer: &[ExtrusionPath]| layer.iter().any(|path| !is_loop(path));
    // Seven layers of post: roof on 10 to 8 under the shelf on 12, and room
    // for two layers of branch between it and the air over the plate.
    // Short as they are, the branches stay, because the roof stands on them.
    let found = supports(&low_shelf(7), &tree());
    assert!(roofed(&found[10]) && roofed(&found[8]));
    for layer in [7, 6] {
        assert!(
            found[layer].iter().any(is_loop),
            "no branch on layer {layer}"
        );
    }
    assert!(found[..6].iter().all(Vec::is_empty));

    // Four layers of post: the roof, on 7 and 6, lies on the plate with
    // its layer of air, and there is no room or need for branches.
    let found = supports(&low_shelf(4), &tree());
    assert!(roofed(&found[7]) && roofed(&found[6]));
    assert!(found[..6].iter().all(Vec::is_empty));
    assert!(found.iter().flatten().all(|path| !is_loop(path)));

    // Whatever the height, every roof has a roof, a branch or the part's
    // own surface, a layer of air away, beneath it.
    for post in 2..12 {
        let layers = low_shelf(post);
        let found = supports(&layers, &tree());
        for layer in 6..found.len() {
            if found[layer].is_empty() {
                continue;
            }
            assert!(
                layer == 6 || !found[layer - 1].is_empty(),
                "with a post of {post} layers, layer {layer} stands on nothing"
            );
        }
    }
}

#[test]
fn ironing_a_supports_top_is_said_not_to_be_done() {
    let read =
        |json: &str| SupportSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap();
    let ironed =
        read(r#"{"enable_support": "1", "support_type": "normal(auto)", "support_ironing": "1"}"#);
    assert_eq!(
        ironed.unsupported,
        ["support_ironing \"1\": the dense top of a support is not ironed"]
    );
    // Off, or with no supports to iron, there is nothing to say.
    let plain =
        read(r#"{"enable_support": "1", "support_type": "normal(auto)", "support_ironing": "0"}"#);
    assert!(plain.unsupported.is_empty());
    let none = read(r#"{"enable_support": "0", "support_ironing": "1"}"#);
    assert!(none.unsupported.is_empty());
}
