//! The Voronoi diagram of a set of segments as plain numbers, for a
//! crate that needs the whole diagram and may not know which library
//! made it.

use boostvoronoi::prelude::{Builder, SourceCategory};

use crate::MedialError;

/// What a cell is the cell of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A segment, without its ends.
    Segment,
    /// The point a segment starts at.
    SegmentStart,
    /// The point a segment ends at.
    SegmentEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoronoiCell {
    /// Which of the segments given.
    pub segment: usize,
    pub source: Source,
    /// One of the edges round the cell; none for a cell with no edges.
    pub incident_edge: Option<usize>,
}

/// One side of an edge: the half that has `cell` on its left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoronoiEdge {
    pub cell: usize,
    /// Where it starts and ends; none where it runs off without end.
    pub vertex0: Option<usize>,
    pub vertex1: Option<usize>,
    /// The other side.
    pub twin: usize,
    /// The next edge round the cell, counter-clockwise.
    pub next: usize,
    /// False for an edge between a segment and its own end.
    pub primary: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoronoiDiagram {
    pub cells: Vec<VoronoiCell>,
    pub edges: Vec<VoronoiEdge>,
    pub vertices: Vec<[f64; 2]>,
}

/// The Voronoi diagram of `segments`, each given as its start and end.
/// Segments may meet only at their ends.
pub fn diagram(segments: &[[i64; 4]]) -> Result<VoronoiDiagram, MedialError> {
    let limit = i64::from(i32::MAX / 2);
    if segments.iter().flatten().any(|&c| c.abs() > limit) {
        return Err(MedialError::TooLarge);
    }
    if segments.is_empty() {
        return Ok(VoronoiDiagram::default());
    }
    let built = Builder::<i64>::default()
        .with_segments(segments.iter())?
        .build()?;

    let mut cells = Vec::with_capacity(built.cells().len());
    for cell in built.cells() {
        cells.push(VoronoiCell {
            segment: cell.source_index().usize(),
            source: match cell.source_category() {
                SourceCategory::Segment => Source::Segment,
                SourceCategory::SegmentEnd => Source::SegmentEnd,
                SourceCategory::SegmentStart | SourceCategory::SinglePoint => Source::SegmentStart,
            },
            incident_edge: cell.get_incident_edge().map(|edge| edge.usize()),
        });
    }
    let count = built.edges().len();
    let mut edges = Vec::with_capacity(count);
    for index in 0..count {
        let edge = built.edge_index_unchecked(index);
        edges.push(VoronoiEdge {
            cell: built.edge_get_cell(edge)?.usize(),
            vertex0: built.edge_get_vertex0(edge)?.map(|vertex| vertex.usize()),
            vertex1: built.edge_get_vertex1(edge)?.map(|vertex| vertex.usize()),
            twin: built.edge_get_twin(edge)?.usize(),
            next: built.edge_get_next(edge)?.usize(),
            primary: built.edge(edge)?.is_primary(),
        });
    }
    let vertices = built
        .vertices()
        .iter()
        .map(|vertex| [vertex.x(), vertex.y()])
        .collect();
    Ok(VoronoiDiagram {
        cells,
        edges,
        vertices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_has_a_cell_for_every_side_and_corner() {
        let side = 10_000_000;
        let square = [
            [0, 0, side, 0],
            [side, 0, side, side],
            [side, side, 0, side],
            [0, side, 0, 0],
        ];
        let found = diagram(&square).unwrap();
        let sides = found
            .cells
            .iter()
            .filter(|cell| cell.source == Source::Segment)
            .count();
        assert_eq!((sides, found.cells.len()), (4, 8));
        // The four sides' cells meet in the middle.
        assert!(
            found
                .vertices
                .iter()
                .any(|v| (v[0] - 5e6).abs() < 1.0 && (v[1] - 5e6).abs() < 1.0)
        );
        // Every edge is its twin's twin, and going round a cell comes back.
        for (index, edge) in found.edges.iter().enumerate() {
            assert_eq!(found.edges[edge.twin].twin, index);
            let mut at = edge.next;
            let mut steps = 0;
            while at != index {
                assert_eq!(found.edges[at].cell, edge.cell);
                at = found.edges[at].next;
                steps += 1;
                assert!(steps < 20);
            }
        }
        assert!(diagram(&[]).unwrap().cells.is_empty());
        assert!(matches!(
            diagram(&[[0, 0, i64::from(i32::MAX), 0]]),
            Err(MedialError::TooLarge)
        ));
    }
}
