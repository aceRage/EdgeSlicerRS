//! The same outline gives the same Voronoi diagram on every run and on
//! every thread.
//!
//! The Voronoi library keeps its beach line in a skip list. While the
//! sweep is sound, the shape of that list only decides which comparisons a
//! search makes. Where the sweep goes wrong the order in the list is no
//! longer one order, and what a search finds depends on the shape: with
//! the list's levels taken from the thread's random numbers, as the
//! published `cpp_map` takes them, the outline below gave eight different
//! diagrams in 65 builds. The copy in `vendor/cpp_map` starts every list's
//! levels the same way.

use std::thread;

use edge_medial::{Source, VoronoiDiagram, cells, diagram};

/// An outline of the Voron design cube with the notches along its sides,
/// in the plate's coordinates (nanometres), as the walls hand it over: the
/// one the library's debug check used to stop on in one test run of three.
/// Two of its edges are a nanometre out of line with each other
/// (`76173144` and `76173145`) and it lies 49 to 79 mm from the origin;
/// the sweep goes wrong on it, in Boost.Polygon's own diagram as well,
/// and the walls then build it again turned (`sound_diagram` in
/// `edge-arachne`). Moved to the origin it is swept soundly.
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

/// The outline with its lowest corner at the origin.
fn moved() -> Vec<[i64; 4]> {
    let least = |which: usize| {
        OUTLINE
            .iter()
            .map(|edge| edge[which].min(edge[which + 2]))
            .min()
            .unwrap_or(0)
    };
    let (x, y) = (least(0), least(1));
    OUTLINE
        .iter()
        .map(|edge| [edge[0] - x, edge[1] - y, edge[2] - x, edge[3] - y])
        .collect()
}

#[test]
fn an_outline_gives_the_same_diagram_on_every_thread_and_every_time() {
    let moved = moved();
    // Where the sweep goes wrong, and where it is sound.
    let as_given = numbers(&diagram(&OUTLINE).unwrap());
    let sound = numbers(&diagram(&moved).unwrap());
    let shares = cells(&OUTLINE).unwrap();
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..8 {
                    assert_eq!(numbers(&diagram(&OUTLINE).unwrap()), as_given);
                    assert_eq!(numbers(&diagram(&moved).unwrap()), sound);
                    assert_eq!(cells(&OUTLINE).unwrap(), shares);
                }
            });
        }
    });
}

/// One process agreeing with itself does not show that the next one will.
/// The sound diagram is made of whole-number and exactly rounded
/// arithmetic only, so it is the same on every machine, and is held to
/// what it was when this was written. (The other is not held: a number
/// that is not one is told apart by bits that differ between processors.)
#[test]
fn a_sound_diagram_is_the_same_in_every_process() {
    let found = diagram(&moved()).unwrap();
    assert_eq!(
        (found.cells.len(), found.edges.len(), found.vertices.len()),
        (CELLS, EDGES, VERTICES)
    );
    assert_eq!(digest(&numbers(&found)), DIGEST);
}

const CELLS: usize = 200;
const EDGES: usize = 1112;
const VERTICES: usize = 357;
const DIGEST: u64 = 0x2306_ec61_8d0c_b796;
