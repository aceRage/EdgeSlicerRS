use edge_geom::unscaled;

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

fn area(regions: &[ExPolygon]) -> f64 {
    regions.iter().map(ExPolygon::area_mm2).sum()
}

/// The least and greatest corners of everything in `regions`, in mm.
fn bounds(regions: &[ExPolygon]) -> (f64, f64, f64, f64) {
    let points = || {
        regions
            .iter()
            .flat_map(|region| &region.contour.points)
            .map(|point| (unscaled(point.x), unscaled(point.y)))
    };
    (
        points().map(|p| p.0).fold(f64::INFINITY, f64::min),
        points().map(|p| p.1).fold(f64::INFINITY, f64::min),
        points().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max),
        points().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
    )
}

fn read(json: &str) -> Result<IroningSettings, ConfigError> {
    IroningSettings::from_settings(&Settings::from_json(json).unwrap())
}

/// Ironing of one kind with the U1's numbers: lines 0.15 mm apart, a
/// tenth of the flow, a 0.4 mm nozzle and 0.2 mm layers.
fn ironing(kind: &str) -> Ironing {
    read(&format!(
        r#"{{"ironing_type": "{kind}", "ironing_spacing": "0.15", "sparse_infill_density": "15%"}}"#
    ))
    .unwrap()
    .for_filament(1, 0.2)
}

fn surface(kind: SurfaceKind, area: ExPolygon) -> Surface {
    Surface {
        kind,
        area,
        angle: None,
    }
}

/// A layer whose share is a 20 mm square, with these faces and fill.
struct Square {
    faces: Faces,
    surfaces: Vec<Surface>,
    on_bed: bool,
    last: bool,
}

impl Square {
    fn areas(&self, ironing: &Ironing) -> Vec<ExPolygon> {
        let whole = [block(0.0, 0.0, 20.0, 20.0)];
        let infill = [block(1.0, 1.0, 19.0, 19.0)];
        ironing_areas(
            &IronedLayer {
                slices: &whole,
                faces: &self.faces,
                on_bed: self.on_bed,
                surfaces: &self.surfaces,
                infill: &infill,
                outline: &whole,
                ends_here: &|_| self.last,
            },
            ironing,
        )
        .unwrap()
    }
}

/// A layer in the middle of a part, sparse inside.
fn middle() -> Square {
    Square {
        faces: (Vec::new(), Vec::new()),
        surfaces: vec![surface(SurfaceKind::Sparse, block(1.0, 1.0, 19.0, 19.0))],
        on_bed: false,
        last: false,
    }
}

#[test]
fn settings_default_to_the_references_and_read_what_is_set() {
    let unset = read("{}").unwrap();
    assert_eq!(unset, IroningSettings::default());
    assert!(!unset.enabled());
    assert_eq!(
        (unset.kind, unset.pattern, unset.flow, unset.spacing),
        (IroningType::None, IroningPattern::Rectilinear, 10.0, 0.1)
    );
    assert_eq!(
        (unset.inset, unset.angle, unset.angle_fixed),
        (0.0, 0.0, false)
    );

    let set = read(
        r#"{"ironing_type": "topmost", "ironing_pattern": "concentric", "ironing_flow": "15%",
            "ironing_spacing": "0.2", "ironing_inset": "0.3", "ironing_angle": "30",
            "ironing_angle_fixed": "1", "solid_infill_direction": "20", "top_layer_direction": "60",
            "top_shell_layers": "5", "bottom_shell_layers": "2", "sparse_infill_density": "15%",
            "nozzle_diameter": ["0.4", "0.6"],
            "filament_ironing_flow": ["nil", "20%"], "filament_ironing_spacing": ["0.25", "nil"],
            "filament_ironing_inset": ["nil", "0.5"]}"#,
    )
    .unwrap();
    assert!(set.enabled());
    assert_eq!(
        (set.kind, set.pattern),
        (IroningType::Topmost, IroningPattern::Concentric)
    );
    assert_eq!((set.flow, set.spacing, set.inset), (15.0, 0.2, 0.3));
    assert_eq!((set.angle, set.angle_fixed), (30.0, true));
    assert_eq!(
        (set.solid_infill_direction, set.top_layer_direction),
        (20.0, Some(60.0))
    );
    assert_eq!(
        (
            set.top_shell_layers,
            set.bottom_shell_layers,
            set.sparse_density
        ),
        (5, 2, 15.0)
    );
    assert_eq!(set.nozzle_diameters, [0.4, 0.6]);
    assert_eq!(set.filament_flow, [None, Some(20.0)]);
    assert_eq!(set.filament_spacing, [Some(0.25), None]);
    assert_eq!(set.filament_inset, [None, Some(0.5)]);

    for (kind, expected) in [
        ("no ironing", IroningType::None),
        ("top", IroningType::Top),
        ("topmost", IroningType::Topmost),
        ("solid", IroningType::Solid),
    ] {
        let found = read(&format!(r#"{{"ironing_type": "{kind}"}}"#)).unwrap();
        assert_eq!(found.kind, expected);
    }
    // As older files and other slicers' have them.
    assert_eq!(read(r#"{"ironing_angle": "-1"}"#).unwrap().angle, 0.0);
    assert_eq!(
        read(r#"{"ironing_pattern": "zig-zag"}"#).unwrap().pattern,
        IroningPattern::Rectilinear
    );
    // A top surface direction below zero is none.
    assert_eq!(
        read(r#"{"top_layer_direction": "-1"}"#)
            .unwrap()
            .top_layer_direction,
        None
    );
}

#[test]
fn bad_settings_are_errors() {
    for json in [
        r#"{"ironing_type": "sideways"}"#,
        r#"{"ironing_pattern": "gyroid"}"#,
        r#"{"ironing_flow": "150%"}"#,
        r#"{"ironing_flow": "-5%"}"#,
        r#"{"ironing_flow": "plenty"}"#,
        r#"{"ironing_spacing": "-0.1"}"#,
        r#"{"ironing_spacing": "2"}"#,
        r#"{"ironing_spacing": "close"}"#,
        r#"{"ironing_inset": "-1"}"#,
        r#"{"ironing_inset": "500"}"#,
        r#"{"ironing_angle": "askew"}"#,
        r#"{"ironing_angle_fixed": "maybe"}"#,
        r#"{"filament_ironing_flow": ["200%"]}"#,
        r#"{"filament_ironing_spacing": ["3"]}"#,
        r#"{"filament_ironing_inset": ["-2"]}"#,
        r#"{"filament_ironing_spacing": ["wide"]}"#,
    ] {
        assert!(
            matches!(read(json), Err(ConfigError::BadValue { .. })),
            "{json}"
        );
    }
    // The error names the setting.
    let Err(ConfigError::BadValue { key, .. }) = read(r#"{"ironing_type": "sideways"}"#) else {
        panic!("an unknown kind of ironing was read");
    };
    assert_eq!(key, "ironing_type");
}

#[test]
fn what_ironing_does_not_follow_is_said_only_when_something_is_ironed() {
    let asked =
        r#""solid_infill_rotate_template": "0,45,90", "align_infill_direction_to_model": "1""#;
    let on = read(&format!(r#"{{"ironing_type": "top", {asked}}}"#)).unwrap();
    assert_eq!(on.unsupported.len(), 2, "{:?}", on.unsupported);
    assert!(on.unsupported[0].starts_with("solid_infill_rotate_template"));
    assert!(on.unsupported[1].starts_with("align_infill_direction_to_model"));
    let off = read(&format!(r#"{{"ironing_type": "no ironing", {asked}}}"#)).unwrap();
    assert!(off.unsupported.is_empty());
    assert!(
        read(r#"{"ironing_type": "top"}"#)
            .unwrap()
            .unsupported
            .is_empty()
    );
}

#[test]
fn the_line_is_a_share_of_the_layer_spread_over_the_spacing() {
    // 10% of a 0.2 mm layer, lines 0.15 mm apart from a 0.4 mm nozzle:
    // 0.02 * 0.15 / 0.4 = 0.0075 mm tall, as the reference's file says
    // (";HEIGHT:0.0075", ";WIDTH:0.40161").
    let own = ironing("top");
    assert!((own.flow.height - 0.0075).abs() < 1e-12);
    assert!((own.flow.width - 0.401_609_513).abs() < 1e-8);
    assert!((own.flow.mm3_per_mm() - 0.4 * 0.0075).abs() < 1e-12);
    assert_eq!(own.spacing, 0.15);
    // No inset set: half the nozzle.
    assert_eq!(own.edge, 0.2);
    // The layer height is the print's, whatever layer the line is on.
    let thick = read(r#"{"ironing_type": "top"}"#)
        .unwrap()
        .for_filament(1, 0.3);
    assert!((thick.flow.height - 0.3 * 0.1 * 0.1 / 0.4).abs() < 1e-12);

    // Lines are never closer than 0.05 mm, and a flow of nothing is a
    // line with no plastic in it.
    let tight = read(r#"{"ironing_type": "top", "ironing_spacing": "0.01", "ironing_flow": "0%"}"#)
        .unwrap()
        .for_filament(1, 0.2);
    assert_eq!(tight.spacing, 0.05);
    assert_eq!(tight.flow.mm3_per_mm(), 0.0);
    assert_eq!(tight.flow.width, 0.4);
}

#[test]
fn a_filament_irons_with_its_own_numbers_where_it_sets_them() {
    let settings = read(
        r#"{"ironing_type": "top", "ironing_flow": "10%", "ironing_spacing": "0.1",
            "ironing_inset": "0.3", "nozzle_diameter": ["0.4", "0.6"],
            "filament_ironing_flow": ["nil", "20%"], "filament_ironing_spacing": ["nil", "0.2"],
            "filament_ironing_inset": ["nil", "0"]}"#,
    )
    .unwrap();
    let first = settings.for_filament(1, 0.2);
    assert_eq!((first.spacing, first.edge), (0.1, 0.3));
    assert!((first.flow.height - 0.2 * 0.1 * 0.1 / 0.4).abs() < 1e-12);
    // The second sets all three, and has the wider nozzle: an inset of
    // nothing is half of that.
    let second = settings.for_filament(2, 0.2);
    assert_eq!((second.spacing, second.edge), (0.2, 0.3));
    assert!((second.flow.height - 0.2 * 0.2 * 0.2 / 0.6).abs() < 1e-12);
    assert_eq!(second.flow.nozzle_diameter, 0.6);
    // A filament the lists say nothing of keeps the process's numbers,
    // and does not borrow another filament's.
    let third = settings.for_filament(3, 0.2);
    assert_eq!((third.spacing, third.edge), (0.1, 0.3));
    assert_eq!(third.flow.nozzle_diameter, 0.4);
}

#[test]
fn lines_follow_the_top_surface_unless_the_angle_is_fixed() {
    let quarter = FRAC_PI_2;
    let close = |found: f64, degrees: f64| (found - degrees.to_radians()).abs() < 1e-12;
    // Nothing set: along the solid fill, a quarter turn on odd layers.
    let plain = read(r#"{"ironing_type": "top"}"#)
        .unwrap()
        .for_filament(1, 0.2);
    assert!(close(plain.direction(0), 45.0) && close(plain.direction(1), 135.0));
    assert!(close(plain.direction(2), 45.0));
    // An angle is added to that.
    let turned =
        read(r#"{"ironing_type": "top", "ironing_angle": "30", "solid_infill_direction": "10"}"#)
            .unwrap()
            .for_filament(1, 0.2);
    assert!(close(turned.direction(0), 40.0));
    assert!((turned.direction(1) - turned.direction(0) - quarter).abs() < 1e-12);
    // Fixed, the angle is the direction itself, on every layer.
    let fixed = read(
        r#"{"ironing_type": "top", "ironing_angle": "30", "ironing_angle_fixed": "1",
            "solid_infill_direction": "10", "top_layer_direction": "70"}"#,
    )
    .unwrap()
    .for_filament(1, 0.2);
    assert!(close(fixed.direction(0), 30.0) && close(fixed.direction(1), 30.0));
    // A direction of the top surface's own is started from instead, and
    // is not turned from layer to layer.
    let top = read(r#"{"ironing_type": "top", "ironing_angle": "5", "top_layer_direction": "70"}"#)
        .unwrap()
        .for_filament(1, 0.2);
    assert!(close(top.direction(0), 75.0) && close(top.direction(1), 75.0));
}

#[test]
fn faces_are_what_has_nothing_over_it_and_nothing_under_it() {
    // A 20 mm base two layers tall carrying a 10 mm tower two layers tall.
    let layers = [
        vec![block(0.0, 0.0, 20.0, 20.0)],
        vec![block(0.0, 0.0, 20.0, 20.0)],
        vec![block(0.0, 0.0, 10.0, 20.0)],
        vec![block(0.0, 0.0, 10.0, 20.0)],
    ];
    let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
    let shells = Shells {
        top_layers: 1,
        bottom_layers: 1,
        top_thickness: 0.0,
        bottom_thickness: 0.0,
        sparse_density: 0.0,
        minimum_sparse_area: 0.0,
        opening: scaled(0.045),
        ensure: None,
        outer_spacing: 0,
        outer_width: 0,
        second_bridge: None,
        interface: false,
    };
    let faces = exposed(&slices, &slices, &shells).unwrap();
    let sizes: Vec<(f64, f64)> = faces
        .iter()
        .map(|(top, bottom)| (area(top).round(), area(bottom).round()))
        .collect();
    // The bed under the first layer; the ledge beside the tower on the
    // second; the whole of the last.
    assert_eq!(
        sizes,
        [(0.0, 400.0), (200.0, 0.0), (0.0, 0.0), (200.0, 0.0)]
    );
    assert_eq!(bounds(&faces[1].0), (10.0, 0.0, 20.0, 20.0));
}

#[test]
fn top_irons_what_faces_up_and_keeps_back_from_the_layers_edge_alone() {
    // The left half faces up; the right has the part over it.
    let ledge = Square {
        faces: (vec![block(0.0, 0.0, 10.0, 20.0)], Vec::new()),
        ..middle()
    };
    let found = ledge.areas(&ironing("top"));
    // Half a nozzle from the outline on three sides, and right up to
    // what stands on the surface on the fourth.
    let (left, bottom, right, top) = bounds(&found);
    assert!((left - 0.2).abs() < 1e-6 && (bottom - 0.2).abs() < 1e-6);
    assert!((right - 10.0).abs() < 1e-6 && (top - 19.8).abs() < 1e-6);
    assert!((area(&found) - 9.8 * 19.6).abs() < 1e-6);
    // Nothing facing up, nothing ironed; and with no top shell a face
    // up is not a top surface.
    assert!(middle().areas(&ironing("top")).is_empty());
    let mut shell_less = ironing("top");
    shell_less.top_shell_layers = 0;
    assert!(ledge.areas(&shell_less).is_empty());
    // Not ironed at all.
    let mut off = ironing("top");
    off.kind = IroningType::None;
    assert!(ledge.areas(&off).is_empty());
}

#[test]
fn the_inset_is_the_one_set() {
    let top = Square {
        faces: (vec![block(0.0, 0.0, 20.0, 20.0)], Vec::new()),
        ..middle()
    };
    let wide = read(r#"{"ironing_type": "top", "ironing_inset": "1.5"}"#)
        .unwrap()
        .for_filament(1, 0.2);
    assert!((area(&top.areas(&wide)) - 17.0 * 17.0).abs() < 1e-6);
    // The filament's own wins.
    let own = read(
        r#"{"ironing_type": "top", "ironing_inset": "1.5", "filament_ironing_inset": ["2.5"]}"#,
    )
    .unwrap()
    .for_filament(1, 0.2);
    assert!((area(&top.areas(&own)) - 15.0 * 15.0).abs() < 1e-6);
}

#[test]
fn topmost_irons_only_an_objects_last_layer() {
    let top = |last: bool| Square {
        faces: (vec![block(0.0, 0.0, 20.0, 20.0)], Vec::new()),
        last,
        ..middle()
    };
    // A top surface lower down is left alone.
    assert!(top(false).areas(&ironing("topmost")).is_empty());
    assert!((area(&top(true).areas(&ironing("topmost"))) - 19.6 * 19.6).abs() < 1e-6);
    // `top` takes both.
    assert!((area(&top(false).areas(&ironing("top"))) - 19.6 * 19.6).abs() < 1e-6);
}

#[test]
fn solid_irons_a_solid_layer_whole_and_the_solid_part_of_any_other() {
    let solid = ironing("solid");
    let whole = 19.6 * 19.6;
    // Solid right through: everything, walls and all, wherever it is.
    let through = |kind: SurfaceKind| Square {
        surfaces: vec![surface(kind, block(1.0, 1.0, 19.0, 19.0))],
        ..middle()
    };
    for kind in [
        SurfaceKind::InternalSolid,
        SurfaceKind::Top,
        SurfaceKind::Bottom,
    ] {
        assert!((area(&through(kind).areas(&solid)) - whole).abs() < 1e-6);
    }
    // Sparse inside, or bridged over sparse fill: not whole.
    assert!(through(SurfaceKind::Sparse).areas(&solid).is_empty());
    assert!(
        through(SurfaceKind::InternalBridge)
            .areas(&solid)
            .is_empty()
    );
    // Unless the sparse fill is dense enough to count as solid.
    let mut dense = solid;
    dense.sparse_density = 96.0;
    assert!((area(&through(SurfaceKind::Sparse).areas(&dense)) - whole).abs() < 1e-6);
    // With no sparse fill at all the inside is left empty, and shows
    // as fill area no surface covers.
    let mut hollow = solid;
    hollow.sparse_density = 0.0;
    let shelled = Square {
        surfaces: vec![surface(
            SurfaceKind::InternalSolid,
            block(1.0, 1.0, 19.0, 3.0),
        )],
        ..middle()
    };
    let found = shelled.areas(&hollow);
    assert!((area(&found) - 18.0 * 2.0).abs() < 0.01, "{}", area(&found));

    // A layer that is part sparse: what faces up and what of its fill
    // is solid, the two as one area; the walls beside the solid fill
    // are not ironed.
    let mixed = Square {
        faces: (vec![block(0.0, 0.0, 6.0, 20.0)], Vec::new()),
        surfaces: vec![
            surface(SurfaceKind::Top, block(1.0, 1.0, 6.0, 19.0)),
            surface(SurfaceKind::InternalSolid, block(6.0, 1.0, 10.0, 19.0)),
            surface(SurfaceKind::Sparse, block(10.0, 1.0, 19.0, 19.0)),
        ],
        ..middle()
    };
    let found = mixed.areas(&solid);
    assert_eq!(found.len(), 1);
    let expected = 5.8 * 19.6 + 4.0 * 18.0;
    assert!((area(&found) - expected).abs() < 0.01, "{}", area(&found));
    // `top` on the same layer takes the face alone.
    assert!((area(&mixed.areas(&ironing("top"))) - 5.8 * 19.6).abs() < 1e-6);
}

#[test]
fn solid_irons_what_lies_on_the_bed_but_not_a_bridge() {
    let solid = ironing("solid");
    // Part of the layer faces down; the rest is sparse.
    let facing_down = |on_bed: bool| Square {
        faces: (Vec::new(), vec![block(0.0, 0.0, 8.0, 20.0)]),
        on_bed,
        ..middle()
    };
    assert!((area(&facing_down(true).areas(&solid)) - 7.8 * 19.6).abs() < 1e-6);
    assert!(facing_down(false).areas(&solid).is_empty());
    // With no bottom shell there is no bottom surface.
    let mut shell_less = solid;
    shell_less.bottom_shell_layers = 0;
    assert!(facing_down(true).areas(&shell_less).is_empty());
    // Neither `top` nor `topmost` irons what faces down.
    assert!(facing_down(true).areas(&ironing("top")).is_empty());
}

#[test]
fn each_filament_irons_its_own_share_up_to_where_they_meet() {
    // Two halves of a 20 mm square, one in each filament, both facing up.
    let whole = [block(0.0, 0.0, 20.0, 20.0)];
    let shares = [block(0.0, 0.0, 10.0, 20.0), block(10.0, 0.0, 20.0, 20.0)];
    let own = ironing("top");
    let found: Vec<Vec<ExPolygon>> = shares
        .iter()
        .map(|share| {
            let share = std::slice::from_ref(share);
            let faces = (share.to_vec(), Vec::new());
            ironing_areas(
                &IronedLayer {
                    slices: share,
                    faces: &faces,
                    on_bed: false,
                    surfaces: &[],
                    infill: &[],
                    outline: &whole,
                    ends_here: &|_| true,
                },
                &own,
            )
            .unwrap()
        })
        .collect();
    // Each keeps back from the outside of the layer and from nothing
    // else: the two meet in the middle, and neither irons the other's.
    assert_eq!(bounds(&found[0]), (0.2, 0.2, 10.0, 19.8));
    assert_eq!(bounds(&found[1]), (10.0, 0.2, 19.8, 19.8));
    assert!(
        edge_clip::intersection(&found[0], &found[1])
            .unwrap()
            .iter()
            .all(|sliver| sliver.area_mm2() < 1e-6)
    );
    // An empty share irons nothing.
    let nothing = ironing_areas(
        &IronedLayer {
            slices: &[],
            faces: &(Vec::new(), Vec::new()),
            on_bed: false,
            surfaces: &[],
            infill: &[],
            outline: &whole,
            ends_here: &|_| true,
        },
        &own,
    )
    .unwrap();
    assert!(nothing.is_empty());
}

/// The direction of a path's longest stretch, in degrees from 0 to 180.
fn heading(path: &ExtrusionPath) -> f64 {
    let longest = path
        .points
        .windows(2)
        .max_by(|a, b| {
            let length = |pair: &[Point2]| {
                ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64)
            };
            length(a).total_cmp(&length(b))
        })
        .unwrap();
    let degrees = ((longest[1].y - longest[0].y) as f64)
        .atan2((longest[1].x - longest[0].x) as f64)
        .to_degrees();
    degrees.rem_euclid(180.0)
}

fn length(paths: &[ExtrusionPath]) -> f64 {
    paths.iter().map(ExtrusionPath::length_mm).sum()
}

#[test]
fn straight_lines_cover_the_area_at_the_spacing_and_in_one_sweep() {
    let own = ironing("top");
    let areas = [block(0.0, 0.0, 10.0, 10.0)];
    let origin = |_: &ExPolygon| Point2::from_mm(5.0, 5.0);
    let even = iron(&areas, 0, &own, &origin, 7).unwrap();
    assert!(!even.is_empty());
    // Ironing lines, the line worked out for them, one area's as one
    // part, each after the one before.
    assert!(even.iter().all(|path| path.role == Role::Ironing));
    assert!(
        even.iter()
            .all(|path| path.flow == own.flow && path.part == 7)
    );
    assert!(!even[0].in_turn && even[1..].iter().all(|path| path.in_turn));
    // Along the solid fill on an even layer, across it on an odd one.
    assert!((heading(&even[0]) - 45.0).abs() < 1e-6);
    let odd = iron(&areas, 1, &own, &origin, 1).unwrap();
    assert!((heading(&odd[0]) - 135.0).abs() < 1e-6);
    // Lines 0.15 mm apart over 100 mm²: about 667 mm of them, the
    // joins along the edge included.
    for paths in [&even, &odd] {
        let found = length(paths);
        assert!((found - 100.0 / 0.15).abs() < 0.02 * found, "{found}");
    }
    // Nothing leaves the area.
    let reach = |paths: &[ExtrusionPath]| {
        paths
            .iter()
            .flat_map(|path| &path.points)
            .map(|point| {
                unscaled(point.x)
                    .max(unscaled(point.y))
                    .max(-unscaled(point.x).min(unscaled(point.y)))
            })
            .fold(f64::NEG_INFINITY, f64::max)
    };
    assert!(reach(&even) <= 10.0 && reach(&odd) <= 10.0);
    // One sweep: along a line across the lines' direction, each long
    // stretch lies further on than the one before.
    let across: Vec<f64> = even
        .iter()
        .flat_map(|path| path.points.windows(2))
        .filter(|pair| {
            ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64)
                > scaled(1.0) as f64
        })
        .map(|pair| unscaled(pair[0].y) - unscaled(pair[0].x))
        .collect();
    assert!(across.len() > 50);
    assert!(
        across.windows(2).all(|pair| pair[1] > pair[0])
            || across.windows(2).all(|pair| pair[1] < pair[0])
    );
    // And 0.15 mm apart: 0.15 * sqrt(2) measured along x - y.
    let step = (across[1] - across[0]).abs();
    assert!(
        (step - 0.15 * std::f64::consts::SQRT_2).abs() < 1e-3,
        "{step}"
    );

    // A fixed angle is the same on every layer.
    let fixed =
        read(r#"{"ironing_type": "top", "ironing_angle": "90", "ironing_angle_fixed": "1"}"#)
            .unwrap()
            .for_filament(1, 0.2);
    for layer in [0, 1] {
        let lines = iron(&areas, layer, &fixed, &origin, 1).unwrap();
        assert!((heading(&lines[0]) - 90.0).abs() < 1e-6);
    }
}

#[test]
fn lines_lie_on_a_grid_through_the_objects_middle() {
    let own = read(r#"{"ironing_type": "top", "ironing_angle": "90", "ironing_angle_fixed": "1", "ironing_spacing": "0.2"}"#)
        .unwrap()
        .for_filament(1, 0.2);
    let areas = [block(0.0, 0.0, 10.0, 10.0)];
    // Upright lines: where the long stretches stand along x.
    let columns = |middle: f64| -> Vec<f64> {
        iron(&areas, 0, &own, &|_| Point2::from_mm(middle, 5.0), 1)
            .unwrap()
            .iter()
            .flat_map(|path| path.points.windows(2))
            .filter(|pair| (pair[1].y - pair[0].y).abs() > scaled(1.0))
            .map(|pair| unscaled(pair[0].x))
            .collect()
    };
    // Half a spacing off the middle, and every spacing from there.
    let centred = columns(5.0);
    assert!(centred.iter().all(|x| {
        let off = (x - 5.1).rem_euclid(0.2);
        off < 1e-3 || off > 0.2 - 1e-3
    }));
    // The same area in an object whose middle is elsewhere has its lines
    // elsewhere.
    let moved = columns(5.05);
    assert!(moved.iter().all(|x| {
        let off = (x - 5.15).rem_euclid(0.2);
        off < 1e-3 || off > 0.2 - 1e-3
    }));
}

#[test]
fn each_area_is_a_part_of_its_own() {
    let own = ironing("top");
    let areas = [block(0.0, 0.0, 5.0, 5.0), block(10.0, 0.0, 15.0, 5.0)];
    let lines = iron(&areas, 0, &own, &|_| Point2::from_mm(0.0, 0.0), 4).unwrap();
    let parts: Vec<u32> = lines.iter().map(|path| path.part).collect();
    assert!(parts.contains(&4) && parts.contains(&5));
    // Each begins a run of its own.
    let starts: Vec<u32> = lines
        .iter()
        .filter(|path| !path.in_turn)
        .map(|path| path.part)
        .collect();
    assert_eq!(starts, [4, 5]);
    // An area too slight for a line gets none.
    let speck = [block(0.0, 0.0, 0.1, 0.1)];
    assert!(
        iron(&speck, 0, &own, &|_| Point2::from_mm(0.0, 0.0), 1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn concentric_loops_follow_the_edge_from_the_outside_in() {
    let own = read(
        r#"{"ironing_type": "top", "ironing_pattern": "concentric", "ironing_spacing": "0.5"}"#,
    )
    .unwrap()
    .for_filament(1, 0.2);
    let areas = [block(0.0, 0.0, 10.0, 10.0)];
    let loops = iron(&areas, 0, &own, &|_| Point2::from_mm(5.0, 5.0), 1).unwrap();
    // The first a quarter of a millimetre in and one every half
    // millimetre from there, down to the last that leaves room for a
    // line inside it: nine, the innermost 1.5 mm across.
    assert_eq!(loops.len(), 9);
    let size = |path: &ExtrusionPath| {
        Polygon {
            points: path.points[..path.points.len() - 1].to_vec(),
        }
        .signed_area_mm2()
    };
    assert!((size(&loops[0]) - 9.5 * 9.5).abs() < 1e-6);
    assert!((size(&loops[8]) - 1.5 * 1.5).abs() < 1e-6);
    for (index, path) in loops.iter().enumerate() {
        // Closed, counter-clockwise, laid after the one outside it.
        assert_eq!(path.points.first(), path.points.last());
        assert!(size(path) > 0.0);
        assert_eq!(path.in_turn, index > 0);
        assert_eq!(path.role, Role::Ironing);
        if index > 0 {
            assert!(size(path) < size(&loops[index - 1]));
        }
    }
    // The same on every layer: loops have no direction to turn.
    assert_eq!(
        loops,
        iron(&areas, 1, &own, &|_| Point2::from_mm(5.0, 5.0), 1).unwrap()
    );

    // Round a hole, loops go round the hole too, and none crosses it.
    let mut hole = block(4.0, 4.0, 6.0, 6.0).contour;
    hole.points.reverse();
    let ring = [ExPolygon {
        contour: block(0.0, 0.0, 10.0, 10.0).contour,
        holes: vec![hole],
    }];
    let round = iron(&ring, 0, &own, &|_| Point2::from_mm(5.0, 5.0), 1).unwrap();
    assert!(!round.is_empty());
    assert!(
        round
            .iter()
            .all(|path| path.points.first() == path.points.last())
    );
    let inside_hole = |point: &Point2| {
        let (x, y) = (unscaled(point.x), unscaled(point.y));
        x > 4.0 && x < 6.0 && y > 4.0 && y < 6.0
    };
    assert!(!round.iter().flat_map(|path| &path.points).any(inside_hole));
    // Every loop is counter-clockwise, the ones that came from the hole
    // as well.
    assert!(round.iter().all(|path| size(path) > 0.0));
}
