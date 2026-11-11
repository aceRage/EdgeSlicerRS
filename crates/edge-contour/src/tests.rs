use edge_extrusion::Flow;
use edge_geom::Polygon;
use edge_mesh::Mesh;

use super::*;

/// A wedge 20 mm long and 10 wide whose top rises from 1 mm at x = 0 to
/// 1 + 20 tan(4°) at x = 20.
fn wedge() -> Mesh {
    let rise = 20.0 * 4f64.to_radians().tan();
    let top = |x: f64| (1.0 + x / 20.0 * rise) as f32;
    let v = [
        [0.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [20.0, 10.0, 0.0],
        [0.0, 10.0, 0.0],
        [0.0, 0.0, top(0.0)],
        [20.0, 0.0, top(20.0)],
        [20.0, 10.0, top(20.0)],
        [0.0, 10.0, top(0.0)],
    ];
    let t: [[u32; 3]; 12] = [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    Mesh {
        vertices: v.to_vec(),
        triangles: t.to_vec(),
    }
}

fn zaa() -> ZaaSettings {
    ZaaSettings {
        min_z: 0.05,
        minimize_perimeter_height: 35.0,
        speed_scaling: true,
    }
}

/// The layer whose top is at `top`, 0.2 mm tall, sliced as contouring
/// slices it.
fn layer(top: f64) -> At {
    At {
        print_z: top,
        slice_z: top - 0.2 + 0.05,
        height: 0.2,
    }
}

#[test]
fn a_top_line_across_a_slope_follows_it_within_the_band() {
    let surface = Surface::new([&wedge()]);
    // Along the slope, at y = 5, on the layer whose top is 1.6 mm: the
    // surface is there from x = 6.0 (1.42 mm, the slicing plane) to
    // x = 10.7 (1.65).
    let line = [Point2::from_mm(6.5, 5.0), Point2::from_mm(10.0, 5.0)];
    let at = layer(1.6);
    let made = contour_line(&line, false, Kind::Top, 0.2, &at, &zaa(), &surface).unwrap();
    let slope = 4f64.to_radians().tan();
    // Where the slope runs straight the points in line with it are
    // dropped; where it rises past the band they fade, and are kept.
    assert!(made.points.len() < 20, "{}", made.points.len());
    for (point, height) in made.profile.points().iter().zip(made.profile.heights()) {
        let x = unscaled(point.x);
        let surface = 1.0 + x * slope;
        if surface < 1.6 + 0.04 {
            let wanted = (surface - 1.6).max(-0.15);
            assert!(
                (f64::from(*height) - wanted).abs() < 0.015,
                "{x}: {height} {wanted}"
            );
        }
    }
    // Every point stays between the slicing plane and min_z above the top.
    let samples = (0..=35).map(|i| Point2::from_mm(6.5 + 0.1 * i as f64, 5.0));
    for point in samples {
        let height = made.profile.at(point);
        assert!((-0.15 - 1e-6..=0.05 + 1e-6).contains(&height), "{height}");
    }
}

#[test]
fn a_flat_top_is_left_flat() {
    let block =
        Mesh::from_triangle_soup(edge_mesh::primitives::cuboid([0.0; 3], [10.0, 10.0, 1.6]));
    let surface = Surface::new([&block]);
    let line = [Point2::from_mm(1.0, 5.0), Point2::from_mm(9.0, 5.0)];
    assert_eq!(
        contour_line(&line, false, Kind::Top, 0.2, &layer(1.6), &zaa(), &surface),
        None
    );
    // And so is one under a top far above.
    assert_eq!(
        contour_line(&line, false, Kind::Top, 0.2, &layer(1.0), &zaa(), &surface),
        None
    );
}

#[test]
fn a_wall_loop_is_lowered_never_raised_and_comes_back_closed() {
    let surface = Surface::new([&wedge()]);
    let square = Polygon {
        points: [(0.25, 0.25), (19.75, 0.25), (19.75, 9.75), (0.25, 9.75)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
    };
    let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
    let mut loops = vec![ExtrusionLoop {
        polygon: square,
        role: Role::ExternalPerimeter,
        is_hole: false,
        depth: 0,
        flow,
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
    }];
    let made = contour_layer(&mut loops, &mut [], &layer(1.6), &zaa(), &surface);
    let profile = made.loops[0].as_ref().unwrap();
    assert!(profile.heights().iter().all(|height| *height <= 0.0));
    assert!(profile.heights().iter().any(|height| *height < -0.1));
    let points = &loops[0].polygon.points;
    assert!(points.len() > 4);
    assert_ne!(points.first(), points.last());
}

#[test]
fn a_layer_is_sliced_min_z_above_its_bottom_but_the_first() {
    let zaa = zaa();
    assert_eq!(zaa.slice_at(0, 0.0, 0.2), Ok(0.1));
    assert!((zaa.slice_at(3, 0.6, 0.2).unwrap() - 0.65).abs() < 1e-12);
    let high = ZaaSettings { min_z: 0.3, ..zaa };
    assert!(high.slice_at(3, 0.6, 0.2).is_err());
}

#[test]
fn settings_say_what_is_left_out() {
    let read = |json: &str| ZaaSettings::read(&Settings::from_json(json).unwrap()).unwrap();
    assert_eq!(read("{}"), Asked::default());
    let on = read(r#"{"zaa_enabled": "1"}"#);
    assert_eq!(on.zaa, Some(zaa()));
    assert!(on.notes.is_empty());
    let vase = read(r#"{"zaa_enabled": "1", "spiral_mode": "1"}"#);
    assert_eq!(vase.zaa, None);
    assert_eq!(vase.notes.len(), 1);
    let scarfed = read(
        r#"{"zaa_enabled": "1", "seam_slope_type": "all", "slow_down_for_layer_cooling": ["1"]}"#,
    );
    assert!(scarfed.zaa.is_some());
    assert_eq!(scarfed.notes.len(), 2, "{:?}", scarfed.notes);
    assert!(
        ZaaSettings::read(
            &Settings::from_json(r#"{"zaa_enabled": "1", "zaa_min_z": "-1"}"#).unwrap()
        )
        .is_err()
    );
}
