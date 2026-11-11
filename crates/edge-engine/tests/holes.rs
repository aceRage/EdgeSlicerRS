//! The hole options through the whole pipeline, on a plate with a round
//! hole through it and a round boss standing on it: polyholes, the auto
//! circle compensation, their order among the other compensations, the
//! walls told apart for the writer, and that nothing moves with both off.

use std::{
    f64::consts::{PI, TAU},
    sync::atomic::{AtomicUsize, Ordering},
};

use edge_config::Value;
use edge_engine::{Chosen, Failure, Overrides, Sliced, Unwatched};
use edge_geom::{Point2, Polygon};
use edge_mesh::Triangle;

/// How many sides the hole and the boss are drawn with.
const SIDES: usize = 48;

/// An outline pulled up from `z0` to `z1`, with a hole through it where
/// one is given: both counter-clockwise, point for point at one angle.
fn prism(outline: &[(f64, f64)], hole: Option<&[(f64, f64)]>, z0: f64, z1: f64) -> Vec<Triangle> {
    let at = |(x, y): (f64, f64), z: f64| [x as f32, y as f32, z as f32];
    let n = outline.len();
    let mut triangles = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (outline[i], outline[j]);
        triangles.push([at(a, z0), at(b, z0), at(b, z1)]);
        triangles.push([at(a, z0), at(b, z1), at(a, z1)]);
        match hole {
            Some(hole) => {
                let (c, d) = (hole[i], hole[j]);
                triangles.push([at(c, z0), at(d, z1), at(d, z0)]);
                triangles.push([at(c, z0), at(c, z1), at(d, z1)]);
                triangles.push([at(a, z0), at(d, z0), at(b, z0)]);
                triangles.push([at(a, z0), at(c, z0), at(d, z0)]);
                triangles.push([at(a, z1), at(b, z1), at(d, z1)]);
                triangles.push([at(a, z1), at(d, z1), at(c, z1)]);
            }
            None => {
                let m = (
                    outline.iter().map(|p| p.0).sum::<f64>() / n as f64,
                    outline.iter().map(|p| p.1).sum::<f64>() / n as f64,
                );
                triangles.push([at(m, z0), at(b, z0), at(a, z0)]);
                triangles.push([at(m, z1), at(a, z1), at(b, z1)]);
            }
        }
    }
    triangles
}

fn circle(middle: (f64, f64), radius: f64) -> Vec<(f64, f64)> {
    (0..SIDES)
        .map(|i| {
            let angle = TAU * i as f64 / SIDES as f64;
            (
                middle.0 + radius * angle.cos(),
                middle.1 + radius * angle.sin(),
            )
        })
        .collect()
}

/// A plate 30 mm square and 3 mm tall with a 5 mm hole through its
/// middle, and on one corner of it a round boss 8 mm across and 2 mm
/// tall, sliced with the reference's defaults and `changes`.
fn slice(changes: &[(&str, &str)]) -> Result<Sliced, Failure> {
    // The plate's outline, a point for each of the hole's.
    let square: Vec<(f64, f64)> = (0..SIDES)
        .map(|i| {
            let angle = TAU * i as f64 / SIDES as f64;
            let reach = 15.0 / angle.cos().abs().max(angle.sin().abs());
            (15.0 + reach * angle.cos(), 15.0 + reach * angle.sin())
        })
        .collect();
    let mut soup = prism(&square, Some(&circle((15.0, 15.0), 2.5)), 0.0, 3.0);
    soup.extend(prism(&circle((6.0, 6.0), 4.0), None, 3.0, 5.0));
    let mesh = edge_mesh::Mesh::from_triangle_soup(soup);
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "edge-holes-{}-{}.stl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let opened = edge_engine::open(&path, None);
    std::fs::remove_file(&path).unwrap();
    let loaded = opened.unwrap().place().unwrap();
    let mut job = edge_engine::prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap();
    // Walls of one width, whose loops keep the shape they go round.
    job.settings
        .insert("wall_generator", Value::One("classic".to_owned()));
    for (key, value) in changes {
        let value = match value.strip_prefix('[') {
            Some(list) => Value::Many(list.split(',').map(str::to_owned).collect()),
            None => Value::One((*value).to_owned()),
        };
        job.settings.insert(*key, value);
    }
    edge_engine::slice(job, &Unwatched)
}

fn sliced(changes: &[(&str, &str)]) -> Sliced {
    slice(changes).unwrap()
}

/// The plate's hole on a layer.
fn hole(sliced: &Sliced, layer: usize) -> &Polygon {
    let regions = &sliced.layers[layer].regions;
    assert_eq!(regions.len(), 1, "layer {layer}");
    assert_eq!(regions[0].holes.len(), 1, "layer {layer}");
    &regions[0].holes[0]
}

/// How far a ring's corners are from its middle, least and most, and
/// the middles of its sides, least, in mm.
fn reach(ring: &Polygon) -> (f64, f64, f64) {
    let middle = edge_slice::centroid(&ring.points);
    let from_middle = |x: f64, y: f64| (x - middle.x as f64).hypot(y - middle.y as f64) / 1e6;
    let points = &ring.points;
    let corners: Vec<f64> = points
        .iter()
        .map(|p| from_middle(p.x as f64, p.y as f64))
        .collect();
    let flats = (0..points.len())
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            from_middle((a.x + b.x) as f64 / 2.0, (a.y + b.y) as f64 / 2.0)
        })
        .fold(f64::MAX, f64::min);
    (
        corners.iter().copied().fold(f64::MAX, f64::min),
        corners.iter().copied().fold(f64::MIN, f64::max),
        flats,
    )
}

/// Which way a ring's first point lies from its middle, in degrees.
fn heading(ring: &Polygon) -> f64 {
    let middle = edge_slice::centroid(&ring.points);
    let first: Point2 = ring.points[0];
    ((first.y - middle.y) as f64)
        .atan2((first.x - middle.x) as f64)
        .to_degrees()
}

/// How far the corners of a ring of [`SIDES`] sides move when its sides
/// are moved out by `amount`.
fn moved(amount: f64) -> f64 {
    amount / (PI / SIDES as f64).cos()
}

fn close(found: f64, wanted: f64) -> bool {
    (found - wanted).abs() < 0.002
}

fn gcode(sliced: &Sliced) -> Vec<u8> {
    sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text
}

#[test]
fn nothing_moves_with_both_options_off() {
    let plain = sliced(&[]);
    let off = sliced(&[
        ("hole_to_polyhole", "0"),
        ("enable_circle_compensation", "0"),
        // What goes with them is not looked at.
        ("hole_to_polyhole_threshold", "5%"),
        ("hole_to_polyhole_twisted", "0"),
        ("circle_compensation_manual_offset", "0.3"),
    ]);
    assert_eq!(plain.walls_by, off.walls_by);
    assert_eq!(plain.fills_by, off.fills_by);
    assert!(
        plain
            .walls_by
            .iter()
            .flatten()
            .flat_map(|layer| &layer.loops)
            .all(|wall| !wall.circle_compensation)
    );
    assert!(off.notes().all(|note| !note.contains("circle")));
}

#[test]
fn a_round_hole_is_made_a_polygon_with_its_flats_on_the_circle() {
    let made = sliced(&[("hole_to_polyhole", "1")]);
    // A 5 mm hole on a 0.4 mm nozzle: ten sides, on every layer of the
    // plate, the middles of the sides where the circle was.
    for layer in 0..15 {
        let ring = hole(&made, layer);
        assert_eq!(ring.points.len(), 10, "layer {layer}");
        assert!(!ring.is_ccw());
        let (least, most, flats) = reach(ring);
        assert!(close(flats, 2.5), "layer {layer}: {flats}");
        assert!(close(least, 2.5 / (PI / 10.0).cos()) && close(most, least));
    }
    // Turned by two fifths of a side from one layer to the next, and
    // back where it was after five.
    let turned =
        |a: usize, b: usize| (heading(hole(&made, b)) - heading(hole(&made, a))).rem_euclid(36.0);
    assert!((turned(0, 1) - 14.4).abs() < 0.01, "{}", turned(0, 1));
    assert!((turned(1, 2) - 14.4).abs() < 0.01);
    assert_eq!(hole(&made, 1), hole(&made, 6));
    // Not twisted: one polygon all the way up.
    let flat = sliced(&[("hole_to_polyhole", "1"), ("hole_to_polyhole_twisted", "0")]);
    assert!((1..15).all(|layer| hole(&flat, layer) == hole(&flat, 0)));
    // No more sides than are allowed.
    let capped = sliced(&[
        ("hole_to_polyhole", "1"),
        ("hole_to_polyhole_max_edges", "6"),
    ]);
    assert_eq!(hole(&capped, 3).points.len(), 6);
    assert!(close(reach(hole(&capped, 3)).2, 2.5));
    // Nothing is said of a setting that is followed.
    assert!(made.notes().all(|note| !note.contains("polyhole")));
    // The walls round it are a polygon's too.
    let walls = &made.walls_by[0][3].loops;
    let round_hole: Vec<_> = walls.iter().filter(|wall| wall.is_hole).collect();
    assert!(!round_hole.is_empty());
    assert!(
        round_hole
            .iter()
            .all(|wall| wall.polygon.points.len() == 10)
    );
}

#[test]
fn a_polyhole_is_drawn_round_the_hole_as_the_compensations_left_it() {
    // The XY hole compensation first: 0.1 mm larger all the way up.
    let grown = sliced(&[("hole_to_polyhole", "1"), ("xy_hole_compensation", "0.1")]);
    assert!(close(reach(hole(&grown, 4)).2, 2.6));
    assert_eq!(hole(&grown, 4).points.len(), 10);
    // Then elephant foot compensation, which opens the first layer's
    // hole: a run of its own, of eleven sides for 2.65 mm, where the
    // layers above have ten for 2.5.
    let footed = sliced(&[
        ("hole_to_polyhole", "1"),
        ("elefant_foot_compensation", "0.15"),
        ("elefant_foot_compensation_layers", "1"),
    ]);
    assert_eq!(hole(&footed, 0).points.len(), 11);
    assert!(close(reach(hole(&footed, 0)).2, 2.65));
    assert_eq!(hole(&footed, 1).points.len(), 10);
    assert!(close(reach(hole(&footed, 1)).2, 2.5));
    // The first layer's outline as modelled keeps the hole as drawn.
    let modelled = &footed.outlines()[0][0].holes[0];
    assert!(modelled.points.len() >= SIDES);
    // Spread over three layers, the second and third are each a run of
    // one layer that is not the first, and stay round, as in the
    // reference.
    let spread = sliced(&[
        ("hole_to_polyhole", "1"),
        ("elefant_foot_compensation", "0.15"),
        ("elefant_foot_compensation_layers", "3"),
    ]);
    assert_eq!(hole(&spread, 0).points.len(), 11);
    assert!(hole(&spread, 1).points.len() >= SIDES);
    assert!(hole(&spread, 2).points.len() >= SIDES);
    assert_eq!(hole(&spread, 3).points.len(), 10);
    // And the circle compensation before polyholes are looked for: the
    // hole is 0.155 mm larger in radius, which makes it eleven sides.
    let both = sliced(&[
        ("hole_to_polyhole", "1"),
        ("enable_circle_compensation", "1"),
    ]);
    assert_eq!(hole(&both, 4).points.len(), 11);
    assert!(close(reach(hole(&both, 4)).2, 2.655));
}

#[test]
fn round_holes_and_outlines_are_resized_and_their_walls_told_apart() {
    let made = sliced(&[("enable_circle_compensation", "1")]);
    // The hole: 0.28 - 0.025 * 5 = 0.155 mm larger in radius.
    let ring = hole(&made, 4);
    assert!(ring.points.len() >= SIDES);
    let (_, most, _) = reach(ring);
    assert!(close(most, 2.5 + moved(0.155)), "{most}");
    // The boss, 8 mm across: 0.025 * 8 - 0.11 = 0.09, held to 0.05.
    let boss = &made.layers[20].regions;
    assert_eq!(boss.len(), 1);
    assert!(close(reach(&boss[0].contour).1, 4.0 + moved(0.05)));
    // The manual offset takes half of itself off either.
    let looser = sliced(&[
        ("enable_circle_compensation", "1"),
        ("circle_compensation_manual_offset", "0.1"),
    ]);
    assert!(close(reach(hole(&looser, 4)).1, 2.5 + moved(0.205)));
    assert!(close(reach(&looser.layers[20].regions[0].contour).1, 4.0));

    // On a layer of the plate the walls round the hole are marked and
    // those round the plate are not; on a layer of the boss every wall
    // is the boss's, and marked.
    let plate = &made.walls_by[0][4].loops;
    assert!(plate.iter().any(|wall| wall.is_hole));
    assert!(
        plate
            .iter()
            .all(|wall| wall.circle_compensation == wall.is_hole)
    );
    let boss = &made.walls_by[0][20].loops;
    assert!(!boss.is_empty());
    assert!(
        boss.iter()
            .all(|wall| wall.circle_compensation && !wall.is_hole)
    );
    // So with walls of changing width.
    let arachne = sliced(&[
        ("enable_circle_compensation", "1"),
        ("wall_generator", "arachne"),
    ]);
    let plate = &arachne.walls_by[0][4].loops;
    assert!(plate.iter().any(|wall| wall.is_hole));
    assert!(
        plate
            .iter()
            .all(|wall| wall.circle_compensation == wall.is_hole)
    );
    assert!(
        arachne.walls_by[0][20]
            .loops
            .iter()
            .all(|wall| wall.circle_compensation)
    );

    // A circle at the diameter limit or over is resized and not marked.
    let limited = sliced(&[
        ("enable_circle_compensation", "1"),
        ("diameter_limit", "[6"),
    ]);
    assert!(close(
        reach(&limited.layers[20].regions[0].contour).1,
        4.0 + moved(0.05)
    ));
    assert!(
        limited.walls_by[0][20]
            .loops
            .iter()
            .all(|wall| !wall.circle_compensation)
    );
    assert!(
        limited.walls_by[0][4]
            .loops
            .iter()
            .any(|wall| wall.circle_compensation)
    );

    // The filament's own figures, as Bambu PLA Basic has them on an
    // H2S: 0.12 - 0.0028 * 5 = 0.106.
    let bambu = sliced(&[
        ("enable_circle_compensation", "1"),
        ("hole_coef_2", "[-0.0028"),
        ("hole_coef_3", "[0.12"),
        ("hole_limit_min", "[0.05"),
        ("hole_limit_max", "[0.12"),
    ]);
    assert!(close(reach(hole(&bambu, 4)).1, 2.5 + moved(0.106)));
}

#[test]
fn compensated_walls_go_at_the_compensation_speed_and_the_seam_is_said() {
    // Walls at 30 and 40 mm/s, and nothing else to hold a wall back.
    let slow = [
        ("outer_wall_speed", "[30"),
        ("inner_wall_speed", "[40"),
        ("filament_max_volumetric_speed", "[0"),
        ("slow_down_for_layer_cooling", "[0"),
        ("enable_overhang_speed", "[0"),
        ("small_perimeter_threshold", "[0"),
        ("circle_compensation_speed", "[70"),
    ];
    let feeds = |sliced: &Sliced| {
        let text = String::from_utf8(gcode(sliced)).unwrap();
        // The feedrates walls are laid at above the first layer.
        let mut feeds = std::collections::BTreeSet::new();
        let (mut layer, mut wall) = (0, false);
        for line in text.lines() {
            if line.starts_with(";LAYER_CHANGE") {
                layer += 1;
            } else if let Some(kind) = line.strip_prefix(";TYPE:") {
                wall = kind == "Outer wall" || kind == "Inner wall";
            } else if let (true, true, Some((_, feed))) = (wall, layer > 1, line.split_once(" F"))
                && line.starts_with("G1 X")
                && line.contains(" E")
            {
                feeds.insert(feed.parse::<f64>().unwrap() as u32 / 60);
            }
        }
        feeds
    };
    let plain = sliced(&slow);
    assert_eq!(feeds(&plain).into_iter().collect::<Vec<_>>(), [30, 40]);
    let mut on = slow.to_vec();
    on.push(("enable_circle_compensation", "1"));
    let made = sliced(&on);
    assert_eq!(feeds(&made).into_iter().collect::<Vec<_>>(), [30, 40, 70]);
    // The scarf seam such walls are asked to have is not made, and
    // that is said; with it turned off there is nothing to say.
    let said: Vec<&String> = made
        .notes()
        .filter(|note| note.contains("circle"))
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("apply_scarf_seam_on_circles"));
    on.push(("apply_scarf_seam_on_circles", "0"));
    assert!(sliced(&on).notes().all(|note| !note.contains("circle")));
}

#[test]
fn bad_values_stop_the_slice() {
    for (key, value) in [
        ("hole_to_polyhole_max_edges", "2"),
        ("hole_to_polyhole_max_edges", "many"),
        ("hole_to_polyhole_threshold", "wide"),
        ("hole_to_polyhole", "yes"),
        ("enable_circle_compensation", "maybe"),
        ("circle_compensation_manual_offset", "a little"),
        ("hole_coef_2", "[steep"),
        ("circle_compensation_speed", "[-5"),
    ] {
        let error = slice(&[(key, value)]).err().map(|error| error.to_string());
        assert!(
            error.as_ref().is_some_and(|error| error.contains(key)),
            "{key} = {value}: {error:?}"
        );
    }
}

#[test]
fn the_file_is_the_same_however_many_threads_make_it() {
    let on = [
        ("hole_to_polyhole", "1"),
        ("enable_circle_compensation", "1"),
        ("elefant_foot_compensation", "0.15"),
    ];
    let with = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| gcode(&sliced(&on)))
    };
    let one = with(1);
    assert!(one == with(4));
    assert!(one == with(7));
}
