//! The Voronoi diagram of a set of segments as plain numbers, for a
//! crate that needs the whole diagram and may not know which library
//! made it.
//!
//! The segments are swept in coordinates of their own, measured from the
//! middle of the box round them, and the diagram is handed back in those
//! with the place of that middle (`VoronoiDiagram::origin`). Where the
//! segments lie on the plate then makes no difference to the diagram,
//! which it did: the sweep goes wrong on some outlines at one place and
//! not at another (the Voron design cube's 49 to 79 mm from the origin,
//! in Boost.Polygon's library as in this one, and not about its own
//! middle in either). The reference's walls are in each object's own
//! coordinates, measured from the middle of the object's box
//! (`PrintObject::m_center_offset`, PrintObject.cpp), and are swept as
//! they are (`SkeletalTrapezoidation::constructFromPolygons`), so its
//! diagrams are of coordinates about a middle as well.

use boostvoronoi::prelude::{Builder, SourceCategory};
use edge_geom::Point2;

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
    /// Whether it is a piece of a parabola: between a point and a segment.
    pub curved: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoronoiDiagram {
    pub cells: Vec<VoronoiCell>,
    pub edges: Vec<VoronoiEdge>,
    /// Where each vertex is, measured from `origin`.
    pub vertices: Vec<[f64; 2]>,
    /// The point the vertices are measured from, in the coordinates the
    /// segments were given in. It is whole, so a vertex is put back
    /// where the segments are without its fraction being touched.
    pub origin: [i64; 2],
}

impl VoronoiDiagram {
    /// A vertex on the grid, in the coordinates the segments were given
    /// in: rounded where the diagram was made and then moved, as whole
    /// numbers (`VoronoiUtils::to_point`, which rounds the reference's
    /// in its object's coordinates). A vertex that is not finite has no
    /// place, and what comes back for one means nothing.
    pub fn point(&self, vertex: usize) -> Point2 {
        let [x, y] = self.vertices[vertex];
        Point2 {
            x: (x.round() as i64).saturating_add(self.origin[0]),
            y: (y.round() as i64).saturating_add(self.origin[1]),
        }
    }
}

/// The point `diagram` measures from for `segments`: the middle of the
/// box round them, on the grid. None when there are no segments or the
/// box is wider than a coordinate can say.
pub fn diagram_origin(segments: &[[i64; 4]]) -> Option<[i64; 2]> {
    let middle = |which: usize| {
        let all = || {
            segments
                .iter()
                .flat_map(move |edge| [edge[which], edge[which + 2]])
        };
        let (least, most) = (all().min()?, all().max()?);
        Some(least + most.checked_sub(least)? / 2)
    };
    Some([middle(0)?, middle(1)?])
}

/// The Voronoi diagram of `segments`, each given as its start and end.
/// Segments may meet only at their ends. They may lie anywhere; it is
/// how far they reach from their own middle that is limited.
pub fn diagram(segments: &[[i64; 4]]) -> Result<VoronoiDiagram, MedialError> {
    if segments.is_empty() {
        return Ok(VoronoiDiagram::default());
    }
    let origin = diagram_origin(segments).ok_or(MedialError::TooLarge)?;
    let limit = i64::from(i32::MAX / 2);
    // No further from the middle than half the box, so none of these
    // can overflow.
    let local: Vec<[i64; 4]> = segments
        .iter()
        .map(|edge| {
            [
                edge[0] - origin[0],
                edge[1] - origin[1],
                edge[2] - origin[0],
                edge[3] - origin[1],
            ]
        })
        .collect();
    if local.iter().flatten().any(|&c| c.abs() > limit) {
        return Err(MedialError::TooLarge);
    }
    let built = Builder::<i64>::default()
        .with_segments(local.iter())?
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
            curved: built.edge(edge)?.is_curved(),
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
        origin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: i64, y: i64, side: i64) -> [[i64; 4]; 4] {
        [
            [x, y, x + side, y],
            [x + side, y, x + side, y + side],
            [x + side, y + side, x, y + side],
            [x, y + side, x, y],
        ]
    }

    #[test]
    fn a_square_has_a_cell_for_every_side_and_corner() {
        let side = 10_000_000;
        let found = diagram(&square(0, 0, side)).unwrap();
        let sides = found
            .cells
            .iter()
            .filter(|cell| cell.source == Source::Segment)
            .count();
        assert_eq!((sides, found.cells.len()), (4, 8));
        // The four sides' cells meet in the middle, which is where the
        // vertices are measured from.
        assert_eq!(found.origin, [side / 2, side / 2]);
        let middle = Point2 {
            x: side / 2,
            y: side / 2,
        };
        assert!((0..found.vertices.len()).any(|vertex| found.point(vertex) == middle));
        assert!(found.vertices.contains(&[0.0, 0.0]));
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
    }

    #[test]
    fn the_limit_is_on_how_far_the_segments_reach_not_on_where_they_lie() {
        // Three metres from the origin, where no coordinate fits the
        // library: the same diagram as at the origin, measured from the
        // square's own middle.
        let side = 10_000_000;
        let far = 3_000_000_000;
        let (here, there) = (
            diagram(&square(0, 0, side)).unwrap(),
            diagram(&square(far, -far, side)).unwrap(),
        );
        assert_eq!(there.origin, [far + side / 2, -far + side / 2]);
        assert_eq!(
            (&there.cells, &there.edges, &there.vertices),
            (&here.cells, &here.edges, &here.vertices)
        );
        for vertex in 0..here.vertices.len() {
            let (a, b) = (here.point(vertex), there.point(vertex));
            assert_eq!((b.x - a.x, b.y - a.y), (far, -far));
        }
        // Too wide wherever it lies, and too wide for a coordinate to say.
        let wide = i64::from(i32::MAX) * 2;
        for segment in [[0, 0, wide, 0], [-wide / 2, 0, wide / 2, 0]] {
            assert!(matches!(diagram(&[segment]), Err(MedialError::TooLarge)));
        }
        assert!(matches!(
            diagram(&[[i64::MIN, 0, i64::MAX, 0]]),
            Err(MedialError::TooLarge)
        ));
        assert_eq!(diagram_origin(&[[i64::MIN, 0, i64::MAX, 0]]), None);
        assert_eq!(diagram_origin(&[]), None);
    }

    #[test]
    fn a_vertex_halfway_between_grid_points_goes_away_from_the_middle() {
        // Three upright lines, each an odd number from the next, their
        // ends level: the vertices between them are half a unit off the
        // grid. They are rounded as they lie about the middle of the
        // box, away from it, as the reference's are away from the middle
        // of its object; on the plate both would have gone up.
        let lines = [
            [100, 100, 100, 200],
            [111, 100, 111, 200],
            [140, 100, 140, 200],
        ];
        let found = diagram(&lines).unwrap();
        assert_eq!(found.origin, [120, 150]);
        let at = |x: f64| -> Vec<usize> {
            (0..found.vertices.len())
                .filter(|&vertex| found.vertices[vertex][0] == x)
                .collect()
        };
        let (left, right) = (at(-14.5), at(5.5));
        assert!(!left.is_empty() && !right.is_empty());
        assert!(left.iter().all(|&vertex| found.point(vertex).x == 105));
        assert!(right.iter().all(|&vertex| found.point(vertex).x == 126));
    }
}
