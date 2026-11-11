//! `counterbore_hole_bridging` through the whole pipeline, on a plate
//! with a hole up through it that is 10 mm across for the first 2 mm and
//! 4 mm across above: the first layer of the narrow part with the option
//! off, partly bridged and with a sacrificial layer, and that every
//! other layer keeps its walls.

use std::{
    f64::consts::{PI, TAU},
    sync::atomic::{AtomicUsize, Ordering},
};

use edge_config::Value;
use edge_engine::{Chosen, Failure, Overrides, Sliced, Unwatched};
use edge_extrusion::Role;
use edge_geom::{Point2, unscaled};
use edge_infill::{Surface, SurfaceKind};
use edge_mesh::Triangle;

/// How many sides the holes are drawn with.
const SIDES: usize = 48;
/// The middle of the plate, where the hole is.
const MIDDLE: (f64, f64) = (15.0, 15.0);
/// The wide hole's radius and the narrow one's, in mm.
const WIDE: f64 = 5.0;
const NARROW: f64 = 2.0;

/// An outline pulled up from `z0` to `z1` with a hole through it: both
/// counter-clockwise, point for point at one angle.
fn prism(outline: &[(f64, f64)], hole: &[(f64, f64)], z0: f64, z1: f64) -> Vec<Triangle> {
    let at = |(x, y): (f64, f64), z: f64| [x as f32, y as f32, z as f32];
    let n = outline.len();
    let mut triangles = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b, c, d) = (outline[i], outline[j], hole[i], hole[j]);
        triangles.push([at(a, z0), at(b, z0), at(b, z1)]);
        triangles.push([at(a, z0), at(b, z1), at(a, z1)]);
        triangles.push([at(c, z0), at(d, z1), at(d, z0)]);
        triangles.push([at(c, z0), at(c, z1), at(d, z1)]);
        triangles.push([at(a, z0), at(d, z0), at(b, z0)]);
        triangles.push([at(a, z0), at(c, z0), at(d, z0)]);
        triangles.push([at(a, z1), at(b, z1), at(d, z1)]);
        triangles.push([at(a, z1), at(d, z1), at(c, z1)]);
    }
    triangles
}

fn circle(radius: f64) -> Vec<(f64, f64)> {
    (0..SIDES)
        .map(|i| {
            let angle = TAU * i as f64 / SIDES as f64;
            (
                MIDDLE.0 + radius * angle.cos(),
                MIDDLE.1 + radius * angle.sin(),
            )
        })
        .collect()
}

/// The plate, 30 mm square and 6 mm tall, sliced with the reference's
/// defaults, classic walls and `changes`.
fn slice(changes: &[(&str, &str)]) -> Result<Sliced, Failure> {
    // The plate's outline, a point for each of the hole's.
    let square: Vec<(f64, f64)> = (0..SIDES)
        .map(|i| {
            let angle = TAU * i as f64 / SIDES as f64;
            let reach = 15.0 / angle.cos().abs().max(angle.sin().abs());
            (
                MIDDLE.0 + reach * angle.cos(),
                MIDDLE.1 + reach * angle.sin(),
            )
        })
        .collect();
    let mut soup = prism(&square, &circle(WIDE), 0.0, 2.0);
    soup.extend(prism(&square, &circle(NARROW), 2.0, 6.0));
    let mesh = edge_mesh::Mesh::from_triangle_soup(soup);
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "edge-counterbore-{}-{}.stl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let opened = edge_engine::open(&path, None);
    std::fs::remove_file(&path).unwrap();
    let loaded = opened.unwrap().place().unwrap();
    let mut job = edge_engine::prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap();
    job.settings
        .insert("wall_generator", Value::One("classic".to_owned()));
    for (key, value) in changes {
        job.settings.insert(*key, Value::One((*value).to_owned()));
    }
    edge_engine::slice(job, &Unwatched)
}

fn sliced(changes: &[(&str, &str)]) -> Sliced {
    slice(changes).unwrap()
}

fn gcode(sliced: &Sliced) -> Vec<u8> {
    sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text
}

/// The first layer cut through the narrow part of the hole.
fn narrow_layer(sliced: &Sliced) -> usize {
    let layer = sliced
        .layers
        .iter()
        .position(|layer| layer.z > 2.0)
        .unwrap();
    assert!(layer > 0 && sliced.layers[layer - 1].z < 2.0);
    layer
}

/// Where the plate's middle is in the slice: a bare model is put in the
/// middle of the bed.
fn middle(sliced: &Sliced) -> (f64, f64) {
    let (low, high) = sliced.bounds;
    let own = (
        f64::from(low[0] + high[0]) / 2.0,
        f64::from(low[1] + high[1]) / 2.0,
    );
    sliced.centred.unwrap_or(own)
}

/// How far a point is from the middle of the hole, in mm.
fn out(sliced: &Sliced, point: Point2) -> f64 {
    let (x, y) = middle(sliced);
    (unscaled(point.x) - x).hypot(unscaled(point.y) - y)
}

/// The nearest any wall on a layer comes to the middle of the hole.
fn nearest_wall(sliced: &Sliced, layer: usize) -> f64 {
    sliced.walls_by[0][layer]
        .loops
        .iter()
        .flat_map(|wall| &wall.polygon.points)
        .map(|point| out(sliced, *point))
        .fold(f64::MAX, f64::min)
}

fn bridges(sliced: &Sliced, layer: usize) -> Vec<&Surface> {
    sliced.surfaces_by[0][layer]
        .iter()
        .filter(|surface| surface.kind == SurfaceKind::Bridge)
        .collect()
}

/// The nearest a bridge line on a layer comes to the middle of the hole,
/// measured along the line and not at its ends alone.
fn nearest_bridge_line(sliced: &Sliced, layer: usize) -> f64 {
    let mut nearest = f64::MAX;
    for path in &sliced.fills_by[0][layer] {
        if path.role != Role::BridgeInfill {
            continue;
        }
        for pair in path.points.windows(2) {
            for step in 0..=20 {
                let t = f64::from(step) / 20.0;
                let point = Point2 {
                    x: pair[0].x + ((pair[1].x - pair[0].x) as f64 * t) as i64,
                    y: pair[0].y + ((pair[1].y - pair[0].y) as f64 * t) as i64,
                };
                nearest = nearest.min(out(sliced, point));
            }
        }
    }
    nearest
}

#[test]
fn with_the_option_off_everything_is_as_it_was() {
    let plain = sliced(&[]);
    let off = sliced(&[
        ("counterbore_hole_bridging", "none"),
        // What goes with it is not looked at.
        ("relative_bridge_angle", "1"),
    ]);
    assert_eq!(plain.walls_by, off.walls_by);
    assert_eq!(plain.surfaces_by, off.surfaces_by);
    assert_eq!(plain.fills_by, off.fills_by);
    // The narrow hole's walls are laid over the wide hole, and what is
    // between them and the wide hole's edge is one bridge round them.
    let layer = narrow_layer(&plain);
    assert!(nearest_wall(&plain, layer) < NARROW + 0.5);
    assert!(nearest_wall(&plain, layer - 1) > WIDE);
    for sliced in [&plain, &off] {
        assert!(sliced.notes().all(|note| !note.contains("counterbore")));
    }
}

#[test]
fn partly_bridged_spans_the_wide_hole_either_side_of_the_narrow_one() {
    let off = sliced(&[]);
    let on = sliced(&[("counterbore_hole_bridging", "partiallybridge")]);
    let layer = narrow_layer(&on);
    assert!(on.notes().all(|note| !note.contains("counterbore")));
    // The walls go round the wide hole, a little out from where it was.
    let nearest = nearest_wall(&on, layer);
    assert!(nearest > WIDE && nearest < WIDE + 1.0, "{nearest}");
    // Two bridges laid the same way, with the narrow hole between them.
    let found = bridges(&on, layer);
    assert_eq!(found.len(), 2, "{found:?}");
    let (one, two) = (found[0].angle.unwrap(), found[1].angle.unwrap());
    let apart = (one - two).abs();
    assert!(apart.min(PI - apart) < 0.1, "{one} {two}");
    let near = nearest_bridge_line(&on, layer);
    assert!(near > NARROW - 0.6 && near < NARROW + 0.6, "{near}");
    // Each reaches from side to side of the wide hole and onto its edge.
    for bridge in &found {
        let reach = bridge
            .area
            .contour
            .points
            .iter()
            .map(|point| out(&on, *point))
            .fold(f64::MIN, f64::max);
        assert!(reach > WIDE + 0.2 && reach < WIDE + 0.7, "{reach}");
    }
    // Less is bridged than with the option off, and no wall is laid in
    // the air.
    let area = |sliced: &Sliced| -> f64 {
        bridges(sliced, layer)
            .iter()
            .map(|surface| surface.area.area_mm2())
            .sum()
    };
    assert!(area(&on) < area(&off), "{} {}", area(&on), area(&off));
    // Every other layer has the walls it had.
    for other in (0..on.layers.len()).filter(|other| *other != layer) {
        assert_eq!(
            on.walls_by[0][other], off.walls_by[0][other],
            "layer {other}"
        );
    }
    assert!(nearest_wall(&on, layer + 1) < NARROW + 0.5);
}

#[test]
fn a_sacrificial_layer_closes_the_hole() {
    let off = sliced(&[]);
    let on = sliced(&[("counterbore_hole_bridging", "sacrificiallayer")]);
    let layer = narrow_layer(&on);
    let nearest = nearest_wall(&on, layer);
    assert!(nearest > WIDE && nearest < WIDE + 1.0, "{nearest}");
    // One bridge, over the wide hole and the narrow one both.
    let found = bridges(&on, layer);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].area.holes.is_empty());
    let whole = found[0].area.area_mm2();
    assert!(whole > PI * WIDE * WIDE * 0.99, "{whole}");
    assert!(whole < PI * (WIDE + 0.6) * (WIDE + 0.6), "{whole}");
    assert!(nearest_bridge_line(&on, layer) < 0.3);
    // Only that layer is closed.
    for other in (0..on.layers.len()).filter(|other| *other != layer) {
        assert_eq!(
            on.walls_by[0][other], off.walls_by[0][other],
            "layer {other}"
        );
        assert!(
            nearest_bridge_line(&on, other) > NARROW,
            "a bridge over the hole on layer {other}"
        );
    }
    // One direction for every bridge is the cover's too.
    let turned = sliced(&[
        ("counterbore_hole_bridging", "sacrificiallayer"),
        ("bridge_angle", "30"),
    ]);
    let found = bridges(&turned, layer);
    assert_eq!(found.len(), 1);
    assert!((found[0].angle.unwrap() - 30f64.to_radians()).abs() < 1e-9);
}

#[test]
fn a_value_that_is_none_of_the_three_stops_the_slice() {
    let error = slice(&[("counterbore_hole_bridging", "always")])
        .err()
        .map(|error| error.to_string());
    assert!(
        error
            .as_ref()
            .is_some_and(|error| error.contains("counterbore_hole_bridging")),
        "{error:?}"
    );
}

#[test]
fn the_file_is_the_same_however_many_threads_make_it() {
    for mode in ["partiallybridge", "sacrificiallayer"] {
        let with = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| gcode(&sliced(&[("counterbore_hole_bridging", mode)])))
        };
        let one = with(1);
        assert!(one == with(4), "{mode}");
        assert!(one == with(7), "{mode}");
    }
}
