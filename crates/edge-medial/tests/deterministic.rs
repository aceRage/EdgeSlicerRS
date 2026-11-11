//! The same outline gives the same Voronoi diagram on every run and on
//! every thread, and wherever it lies.
//!
//! The Voronoi library keeps its beach line in a skip list. While the
//! sweep is sound, the shape of that list only decides which comparisons a
//! search makes. Where the sweep goes wrong the order in the list is no
//! longer one order, and what a search finds depends on the shape: with
//! the list's levels taken from the thread's random numbers, as the
//! published `cpp_map` takes them, the outline below gave eight different
//! diagrams in 65 builds where it lay. The copy in `vendor/cpp_map` starts
//! every list's levels the same way.
//!
//! Where the sweep goes wrong depends on where the outline lies, and
//! `diagram` now sweeps every outline about the middle of its own box,
//! where this one is swept soundly.

use std::thread;

use edge_medial::{Source, VoronoiDiagram, cells, diagram};

/// An outline of the Voron design cube with the notches along its sides,
/// in the plate's coordinates (nanometres), as the walls hand it over: the
/// one the library's debug check used to stop on in one test run of three.
/// Two of its edges are a nanometre out of line with each other
/// (`76173144` and `76173145`) and it lies 49 to 79 mm from the origin;
/// swept there the sweep goes wrong, in Boost.Polygon's own diagram as
/// well, and the walls had to build it again turned (`sound_diagram` in
/// `edge-arachne`). Swept about its own middle it is sound, in both.
const OUTLINE: [[i64; 4]; 100] = [
    [60186098, 55673323, 63132079, 55673322],
    [63132079, 55673322, 63132079, 49021460],
    [63132079, 49021460, 65535743, 49021460],
    [65535743, 49021460, 65535739, 55673323],
    [65535739, 55673323, 68482148, 55673323],
    [68482148, 55673323, 68482148, 49021460],
    [68482148, 49021460, 78978540, 49021460],
    [78978540, 49021460, 78978540, 54474779],
    [78978540, 54474779, 76978540, 54474779],
    [76978540, 54474779, 76978540, 60211277],
    [76978540, 60211277, 78978540, 60211277],
    [78978540, 60211277, 78978540, 63071416],
    [78978540, 63071416, 76978540, 63071416],
    [76978540, 63071416, 76978540, 65560936],
    [76978540, 65560936, 78978540, 65560935],
    [78978540, 65560935, 78978540, 68421487],
    [78978540, 68421487, 76978540, 68421489],
    [76978540, 68421489, 76978540, 73766928],
    [76978540, 73766928, 78978540, 73766928],
    [78978540, 73766928, 78978540, 78978540],
    [78978540, 78978540, 67813901, 78978540],
    [67813901, 78978540, 67813901, 72326676],
    [67813901, 72326676, 64867921, 72326678],
    [64867921, 72326678, 64867921, 78978540],
    [64867921, 78978540, 62464257, 78978540],
    [62464257, 78978540, 62464258, 72326677],
    [62464258, 72326677, 59517853, 72326677],
    [59517853, 72326677, 59517853, 78978540],
    [59517853, 78978540, 49021460, 78978540],
    [49021460, 78978540, 49021460, 73525221],
    [49021460, 73525221, 51021460, 73525221],
    [51021460, 73525221, 51021460, 67788721],
    [51021460, 67788721, 49021460, 67788720],
    [49021460, 67788720, 49021460, 64928582],
    [49021460, 64928582, 51021460, 64928585],
    [51021460, 64928585, 51021460, 62439064],
    [51021460, 62439064, 49021460, 62439063],
    [49021460, 62439063, 49021460, 59578511],
    [49021460, 59578511, 51021460, 59578511],
    [51021460, 59578511, 51021460, 54233072],
    [51021460, 54233072, 49021460, 54233072],
    [49021460, 54233072, 49021460, 49021460],
    [49021460, 49021460, 60186098, 49021460],
    [60186098, 49021460, 60186098, 55673323],
    [63185132, 57428894, 62479353, 57555507],
    [62479353, 57555507, 61791068, 57757827],
    [61791068, 57757827, 61128455, 58033576],
    [61128455, 58033576, 60168549, 58599637],
    [60168549, 58599637, 59587813, 59062746],
    [59587813, 59062746, 59062746, 59587813],
    [59062746, 59587813, 58599781, 60168355],
    [58599781, 60168355, 58204739, 60797062],
    [58204739, 60797062, 57882558, 61466082],
    [57882558, 61466082, 57637320, 62166923],
    [57637320, 62166923, 57472087, 62890864],
    [57472087, 62890864, 57388950, 63628744],
    [57388950, 63628744, 57388950, 64371254],
    [57388950, 64371254, 57472089, 65109156],
    [57472089, 65109156, 57637318, 65833073],
    [57637318, 65833073, 57882558, 66533916],
    [57882558, 66533916, 58204739, 67202936],
    [58204739, 67202936, 58599781, 67831643],
    [58599781, 67831643, 59062754, 68412194],
    [59062754, 68412194, 59587805, 68937240],
    [59587805, 68937240, 60168352, 69400219],
    [60168352, 69400219, 60797058, 69795257],
    [60797058, 69795257, 61466082, 70117440],
    [61466082, 70117440, 62166923, 70362678],
    [62166923, 70362678, 62890866, 70527907],
    [62890866, 70527907, 63628931, 70611064],
    [63628931, 70611064, 64250586, 70616521],
    [64250586, 70616521, 64814920, 70571093],
    [64814920, 70571093, 65520658, 70444485],
    [65520658, 70444485, 66208934, 70242166],
    [66208934, 70242166, 66871512, 69966435],
    [66871512, 69966435, 67522827, 69606560],
    [67522827, 69606560, 68128432, 69176863],
    [68128432, 69176863, 68682081, 68682081],
    [68682081, 68682081, 69176863, 68128432],
    [69176863, 68128432, 69606561, 67522827],
    [69606561, 67522827, 69965734, 66872952],
    [69965734, 66872952, 70249892, 66186930],
    [70249892, 66186930, 70455451, 65473417],
    [70455451, 65473417, 70579831, 64741391],
    [70579831, 64741391, 70621465, 63999999],
    [70621465, 63999999, 70579831, 63258609],
    [70579831, 63258609, 70455451, 62526578],
    [70455451, 62526578, 70249894, 61813073],
    [70249894, 61813073, 69965734, 61127046],
    [69965734, 61127046, 69606561, 60477171],
    [69606561, 60477171, 69176871, 59871581],
    [69176871, 59871581, 68682082, 59317914],
    [68682082, 59317914, 68128424, 58823130],
    [68128424, 58823130, 67522827, 58393437],
    [67522827, 58393437, 66872952, 58034264],
    [66872952, 58034264, 66186930, 57750106],
    [66186930, 57750106, 65473400, 57544543],
    [65473400, 57544543, 64461962, 57394906],
    [64461962, 57394906, 63810719, 57381756],
    [63810719, 57381756, 63185132, 57428894],
];

/// Everything in a diagram as whole numbers, so that two can be compared
/// even where a coordinate is not a number.
fn numbers(diagram: &VoronoiDiagram) -> Vec<u64> {
    let index = |at: Option<usize>| at.map_or(u64::MAX, |at| at as u64);
    let mut all = Vec::new();
    for cell in &diagram.cells {
        let source = match cell.source {
            Source::Segment => 0,
            Source::SegmentStart => 1,
            Source::SegmentEnd => 2,
        };
        all.extend([cell.segment as u64, source, index(cell.incident_edge)]);
    }
    for edge in &diagram.edges {
        all.extend([
            edge.cell as u64,
            index(edge.vertex0),
            index(edge.vertex1),
            edge.twin as u64,
            edge.next as u64,
            u64::from(edge.primary),
            u64::from(edge.curved),
        ]);
    }
    for vertex in &diagram.vertices {
        all.extend([vertex[0].to_bits(), vertex[1].to_bits()]);
    }
    all.extend(diagram.origin.map(|c| c as u64));
    all
}

/// FNV-1a over `numbers`.
fn digest(numbers: &[u64]) -> u64 {
    numbers
        .iter()
        .flat_map(|number| number.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash: u64, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

/// The outline moved by `(x, y)`.
fn moved(x: i64, y: i64) -> Vec<[i64; 4]> {
    OUTLINE
        .iter()
        .map(|edge| [edge[0] + x, edge[1] + y, edge[2] + x, edge[3] + y])
        .collect()
}

/// The outline and the same outline through the origin. The middle of
/// the box round the two is the origin, so they are swept where they
/// lie, which is where the sweep of the outline goes wrong.
fn doubled() -> Vec<[i64; 4]> {
    OUTLINE
        .iter()
        .copied()
        .chain(OUTLINE.iter().map(|edge| edge.map(|c| -c)))
        .collect()
}

/// The furthest any vertex is from being where it should: as near to
/// the edge or end of an edge of each cell that meets at it as to the
/// nearest edge of all.
fn worst(edges: &[[i64; 4]], diagram: &VoronoiDiagram) -> f64 {
    let [ox, oy] = diagram.origin.map(|c| c as f64);
    let to_point = |(x, y): (f64, f64), (px, py): (f64, f64)| (x - px).hypot(y - py);
    let to_edge = |(x, y): (f64, f64), edge: &[i64; 4]| {
        let [ax, ay, bx, by] = edge.map(|c| c as f64);
        let (dx, dy) = (bx - ax, by - ay);
        let along = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        to_point((x, y), (ax + along * dx, ay + along * dy))
    };
    let mut worst = 0.0f64;
    for edge in &diagram.edges {
        let Some(vertex) = edge.vertex0 else { continue };
        let [x, y] = diagram.vertices[vertex];
        let at = (x + ox, y + oy);
        let nearest = edges
            .iter()
            .map(|edge| to_edge(at, edge))
            .fold(f64::INFINITY, f64::min);
        let cell = &diagram.cells[edge.cell];
        let [ax, ay, bx, by] = edges[cell.segment].map(|c| c as f64);
        let own = match cell.source {
            Source::Segment => to_edge(at, &edges[cell.segment]),
            Source::SegmentStart => to_point(at, (ax, ay)),
            Source::SegmentEnd => to_point(at, (bx, by)),
        };
        worst = worst.max((own - nearest).abs());
    }
    worst
}

#[test]
fn an_outline_gives_the_same_diagram_on_every_thread_and_every_time() {
    let doubled = doubled();
    // Where the sweep is sound, and where it goes wrong.
    let sound = numbers(&diagram(&OUTLINE).unwrap());
    let wrong = numbers(&diagram(&doubled).unwrap());
    let shares = cells(&OUTLINE).unwrap();
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..8 {
                    assert_eq!(numbers(&diagram(&OUTLINE).unwrap()), sound);
                    assert_eq!(numbers(&diagram(&doubled).unwrap()), wrong);
                    assert_eq!(cells(&OUTLINE).unwrap(), shares);
                }
            });
        }
    });
}

/// What the two tests above and below stand on: the outline is swept
/// soundly now, every vertex within a nanometre of where it should be,
/// and the sweep still goes wrong where the outline lay, by millimetres.
#[test]
fn the_outline_is_swept_soundly_about_its_middle_and_not_where_it_lay() {
    let found = diagram(&OUTLINE).unwrap();
    assert_eq!(found.origin, [64_000_000, 64_000_000]);
    assert!(worst(&OUTLINE, &found) < 1.5);
    let doubled = doubled();
    let found = diagram(&doubled).unwrap();
    assert_eq!(found.origin, [0, 0]);
    assert!(worst(&doubled, &found) > 1e6);
}

/// Where the outline lies makes no difference: the diagram is the same
/// to the last bit, measured from a point moved as the outline was.
#[test]
fn an_outline_gives_the_same_diagram_wherever_it_lies() {
    let here = diagram(&OUTLINE).unwrap();
    for (x, y) in [
        // Its lowest corner at the origin, and its middle.
        (-49_021_460, -49_021_460),
        (-64_000_000, -64_000_000),
        (1, 0),
        (71_500_000, 72_000_000),
        (-250_000_000, 123_456_789),
        // Three metres off, where no coordinate fits the library.
        (3_000_000_000, -3_000_000_000),
    ] {
        let there = diagram(&moved(x, y)).unwrap();
        assert_eq!(there.origin, [here.origin[0] + x, here.origin[1] + y]);
        let mut back = there;
        back.origin = here.origin;
        assert_eq!(numbers(&back), numbers(&here), "moved by ({x}, {y})");
    }
}

/// One process agreeing with itself does not show that the next one will.
/// The sound diagram is made of whole-number and exactly rounded
/// arithmetic only, so it is the same on every machine, and is held to
/// what it was when this was written. (The other is not held: a number
/// that is not one is told apart by bits that differ between processors.)
#[test]
fn a_sound_diagram_is_the_same_in_every_process() {
    let found = diagram(&OUTLINE).unwrap();
    assert_eq!(
        (found.cells.len(), found.edges.len(), found.vertices.len()),
        (CELLS, EDGES, VERTICES)
    );
    assert_eq!(digest(&numbers(&found)), DIGEST);
}

const CELLS: usize = 200;
const EDGES: usize = 1112;
const VERTICES: usize = 357;
const DIGEST: u64 = 0x14ab_0e87_ffae_7b16;
