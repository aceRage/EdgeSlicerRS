//! Travel that goes round the walls (`reduce_crossing_wall`) through the
//! whole pipeline, on a frame: that moves over the hole go round it,
//! that a limit on the way round is kept, that nothing changes with the
//! setting off, and that the file comes out the same however many
//! threads make it.

use std::sync::atomic::{AtomicUsize, Ordering};

use edge_config::Value;
use edge_engine::{Chosen, Overrides, Sliced, Unwatched};
use edge_gcode::Summary;
use edge_geom::unscaled;

/// A frame 40 mm square and 2 mm tall, of four bars 8 mm wide round a
/// 24 mm hole, sliced with the reference's defaults and `changes`.
fn sliced(changes: &[(&str, &str)]) -> Sliced {
    let mut soup = Vec::new();
    for (low, high) in [
        ([0.0, 0.0], [40.0, 8.0]),
        ([0.0, 32.0], [40.0, 40.0]),
        ([0.0, 8.0], [8.0, 32.0]),
        ([32.0, 8.0], [40.0, 32.0]),
    ] {
        soup.extend(edge_mesh::primitives::cuboid(
            [low[0], low[1], 0.0],
            [high[0], high[1], 2.0],
        ));
    }
    let mesh = edge_mesh::Mesh::from_triangle_soup(soup);
    // A file of its own for each test, which run side by side.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "edge-avoid-{}-{}.stl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let opened = edge_engine::open(&path, None);
    std::fs::remove_file(&path).unwrap();
    let loaded = opened.unwrap().place().unwrap();
    let mut job = edge_engine::prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap();
    for (key, value) in changes {
        job.settings.insert(*key, Value::One((*value).to_owned()));
    }
    edge_engine::slice(job, &Unwatched).unwrap()
}

fn gcode(sliced: &Sliced) -> (String, Summary) {
    let made = sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap();
    (String::from_utf8(made.text).unwrap(), made.summary)
}

/// The box round the frame's hole and the box round the frame, in mm:
/// min x, min y, max x, max y.
fn boxes(sliced: &Sliced) -> ([f64; 4], [f64; 4]) {
    let region = &sliced.layers[5].regions[0];
    let round = |points: &[edge_geom::Point2]| {
        let xs = || points.iter().map(|p| unscaled(p.x));
        let ys = || points.iter().map(|p| unscaled(p.y));
        [
            xs().fold(f64::MAX, f64::min),
            ys().fold(f64::MAX, f64::min),
            xs().fold(f64::MIN, f64::max),
            ys().fold(f64::MIN, f64::max),
        ]
    };
    (
        round(&region.holes[0].points),
        round(&region.contour.points),
    )
}

/// How many travels of the print, its first aside, pass over the middle
/// of `hole`, and whether every point travelled to is within `part`.
fn over_the_hole(text: &str, hole: [f64; 4], part: [f64; 4]) -> (usize, bool) {
    let word = |line: &str, word: char| -> Option<f64> {
        line.split(' ')
            .find_map(|part| part.strip_prefix(word)?.parse().ok())
    };
    // Well inside the hole, clear of its walls.
    let inside = |x: f64, y: f64| {
        x > hole[0] + 2.0 && x < hole[2] - 2.0 && y > hole[1] + 2.0 && y < hole[3] - 2.0
    };
    let (mut at, mut count, mut kept_in) = (None, 0, true);
    let mut wiping = false;
    for line in text.lines() {
        match line {
            ";WIPE_START" => wiping = true,
            ";WIPE_END" => wiping = false,
            _ => {}
        }
        if !line.starts_with("G1 ") {
            continue;
        }
        let (Some(x), Some(y)) = (word(line, 'X'), word(line, 'Y')) else {
            continue;
        };
        if let Some((from_x, from_y)) = at
            && word(line, 'E').is_none()
            && !wiping
        {
            let crossed = (0..=200).any(|step| {
                let t = f64::from(step) / 200.0;
                inside(from_x + t * (x - from_x), from_y + t * (y - from_y))
            });
            count += usize::from(crossed);
            kept_in &= x >= part[0] && x <= part[2] && y >= part[1] && y <= part[3];
        }
        at = Some((x, y));
    }
    (count, kept_in)
}

#[test]
fn travel_goes_round_the_hole_when_asked_and_not_otherwise() {
    let plain = sliced(&[]);
    let (plain_text, plain_summary) = gcode(&plain);
    let (hole, part) = boxes(&plain);
    assert!((hole[2] - hole[0] - 24.0).abs() < 0.01);
    let (straight, _) = over_the_hole(&plain_text, hole, part);
    assert!(straight > 10, "{straight}");

    // Off, and a limit set with it off: the moves are what they were.
    let off = sliced(&[
        ("reduce_crossing_wall", "0"),
        ("max_travel_detour_distance", "5"),
    ]);
    let (off_text, off_summary) = gcode(&off);
    assert_eq!(off_summary, plain_summary);
    let moves = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| line.starts_with('G') || line.starts_with(";TYPE"))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(moves(&off_text), moves(&plain_text));

    let on = sliced(&[("reduce_crossing_wall", "1")]);
    let (on_text, on_summary) = gcode(&on);
    let (round, kept_in) = over_the_hole(&on_text, hole, part);
    // The first layer's start and the top layer, which is all top
    // surface, still go straight; nothing else does.
    assert!(round * 5 < straight, "{round} of {straight}");
    assert!(kept_in, "a routed travel left the part's own box");
    assert!(on_summary.travel_mm > plain_summary.travel_mm + 100.0);
    assert_eq!(on_summary.filament_mm, plain_summary.filament_mm);
    // The setting is followed, and nothing says otherwise.
    assert!(!on.notes().any(|note| note.contains("reduce_crossing_wall")));

    // A way round may be no more than 5 mm longer than straight, or no
    // more than a twentieth: across the hole it is longer than either.
    for limit in ["5", "5%"] {
        let limited = sliced(&[
            ("reduce_crossing_wall", "1"),
            ("max_travel_detour_distance", limit),
        ]);
        let (text, summary) = gcode(&limited);
        let (over, _) = over_the_hole(&text, hole, part);
        assert!(over > round && over <= straight, "{limit}: {over}");
        assert!(summary.travel_mm < on_summary.travel_mm, "{limit}");
    }
}

#[test]
fn routed_travel_comes_out_the_same_on_one_thread_and_on_many() {
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| gcode(&sliced(&[("reduce_crossing_wall", "1")])).0)
    };
    let one = run(1);
    assert!(one == run(4) && one == run(1));
}
