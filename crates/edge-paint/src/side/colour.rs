//! Each ring of a layer's outline cut into lines of one colour each.

use super::Line;

/// A stretch of a ring in one colour, 0 for unpainted (the reference's
/// `ColoredLine`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColouredLine {
    pub line: Line,
    pub colour: u8,
}

/// The runs of one colour round a ring, as the first and last line of
/// each, going round from the first change of colour; the whole ring as
/// one when it is all one colour (`get_segments`).
pub(crate) fn get_segments(ring: &[ColouredLine]) -> Vec<(usize, usize)> {
    let count = ring.len();
    let mut end = 0;
    while end + 1 < count && ring[end].colour == ring[end + 1].colour {
        end += 1;
    }
    if end == count - 1 {
        return vec![(0, count - 1)];
    }
    let first_different = (end + 1) % count;
    let mut segments = Vec::new();
    let mut offset = 0;
    while offset < count {
        let start = (first_different + offset) % count;
        let mut stop = start;
        while offset + 1 < count
            && ring[start].colour == ring[(first_different + offset + 1) % count].colour
        {
            stop = (first_different + offset + 1) % count;
            offset += 1;
        }
        segments.push((start, stop));
        offset += 1;
    }
    segments
}

#[cfg(test)]
mod tests {
    use edge_geom::Point2;

    use super::*;

    fn run(lengths_and_colours: &[(f64, u8)]) -> Vec<ColouredLine> {
        let mut x = 0.0;
        lengths_and_colours
            .iter()
            .map(|&(length, colour)| {
                let line = Line::new(Point2::from_mm(x, 0.0), Point2::from_mm(x + length, 0.0));
                x += length;
                ColouredLine { line, colour }
            })
            .collect()
    }

    #[test]
    fn runs_round_a_ring_start_at_the_first_change() {
        let lines = run(&[(1.0, 1), (1.0, 1), (1.0, 2), (1.0, 0), (1.0, 1)]);
        assert_eq!(get_segments(&lines), [(2, 2), (3, 3), (4, 1)]);
        let one = run(&[(1.0, 3), (1.0, 3)]);
        assert_eq!(get_segments(&one), [(0, 1)]);
    }
}
