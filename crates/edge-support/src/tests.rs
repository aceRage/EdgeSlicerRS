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

    // Tree supports are swapped for these, and that is said.
    let tree = read(r#"{"enable_support": "1", "support_type": "tree(auto)", "raft_layers": "2"}"#);
    assert!(tree.enabled);
    assert_eq!(tree.unsupported.len(), 2);
    assert!(tree.unsupported[0].contains("tree"));
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
