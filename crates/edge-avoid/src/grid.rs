//! The edges of some rings sorted into square cells, so that those near
//! a point or along a line are found without looking at all of them.
//!
//! It stands in for the reference's `EdgeGrid`. An edge is listed in
//! every cell it passes through and, to be on the safe side of rounding,
//! in some it only passes close to; what a query gives back is every
//! edge that could matter and some that do not, each once and always in
//! the same order, for the caller to test.

use edge_geom::{Coord, Point2, Polygon};

pub(crate) struct Grid {
    min: Point2,
    cell: Coord,
    cols: i64,
    rows: i64,
    /// Where each cell's edges start in `items`, and one past the last.
    starts: Vec<u32>,
    /// Indices into `edges`, cell by cell.
    items: Vec<u32>,
    /// (ring, index of the edge's first point).
    edges: Vec<(u32, u32)>,
}

impl Grid {
    /// No more cells than this, however small the cell asked for: larger
    /// cells find the same edges, only more slowly.
    const MOST_CELLS: i64 = 1 << 22;

    pub(crate) fn new(rings: &[Polygon], cell: Coord) -> Self {
        let all = || rings.iter().flat_map(|ring| ring.points.iter());
        let low = |axis: fn(&Point2) -> Coord| all().map(axis).min().unwrap_or(0) - 1;
        let high = |axis: fn(&Point2) -> Coord| all().map(axis).max().unwrap_or(0) + 1;
        let min = Point2 {
            x: low(|p| p.x),
            y: low(|p| p.y),
        };
        let (width, height) = (high(|p| p.x) - min.x, high(|p| p.y) - min.y);
        let mut cell = cell.max(1);
        while (width / cell + 1).saturating_mul(height / cell + 1) > Self::MOST_CELLS {
            cell = cell.saturating_mul(2);
        }
        let mut grid = Grid {
            min,
            cell,
            cols: width / cell + 1,
            rows: height / cell + 1,
            starts: Vec::new(),
            items: Vec::new(),
            edges: Vec::new(),
        };
        // (cell, edge), in the order found, then sorted by cell.
        let mut listed: Vec<(u32, u32)> = Vec::new();
        for (ring, polygon) in rings.iter().enumerate() {
            let points = &polygon.points;
            for (index, &a) in points.iter().enumerate() {
                let b = points[(index + 1) % points.len()];
                let edge = grid.edges.len() as u32;
                grid.edges.push((ring as u32, index as u32));
                grid.cells_along(a, b, |cell| listed.push((cell as u32, edge)));
            }
        }
        listed.sort_unstable();
        let mut starts = vec![0u32; (grid.cols * grid.rows) as usize + 1];
        for &(cell, _) in &listed {
            starts[cell as usize + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        grid.starts = starts;
        grid.items = listed.into_iter().map(|(_, edge)| edge).collect();
        grid
    }

    fn row(&self, y: Coord) -> i64 {
        (y - self.min.y)
            .div_euclid(self.cell)
            .clamp(0, self.rows - 1)
    }

    fn col(&self, x: Coord) -> i64 {
        (x - self.min.x)
            .div_euclid(self.cell)
            .clamp(0, self.cols - 1)
    }

    /// Every cell the segment from `a` to `b` passes through, row by row,
    /// with a unit to spare either side. Cells beyond the grid's edge are
    /// counted as the ones on it.
    fn cells_along(&self, a: Point2, b: Point2, mut visit: impl FnMut(usize)) {
        for row in self.row(a.y.min(b.y))..=self.row(a.y.max(b.y)) {
            // The stretch of the segment within this row of cells.
            let (x0, x1) = if a.y == b.y {
                (a.x as f64, b.x as f64)
            } else {
                let x_at = |y: Coord| {
                    let t = ((y - a.y) as f64 / (b.y - a.y) as f64).clamp(0.0, 1.0);
                    a.x as f64 + t * (b.x - a.x) as f64
                };
                (
                    x_at(self.min.y + row * self.cell),
                    x_at(self.min.y + (row + 1) * self.cell),
                )
            };
            let col = |x: f64| self.col(x as Coord);
            for col in col(x0.min(x1) - 2.0)..=col(x0.max(x1) + 2.0) {
                visit((row * self.cols + col) as usize);
            }
        }
    }

    fn collect(&self, cell: usize, found: &mut Vec<u32>) {
        let range = self.starts[cell] as usize..self.starts[cell + 1] as usize;
        found.extend_from_slice(&self.items[range]);
    }

    fn listed(&self, mut found: Vec<u32>) -> impl Iterator<Item = (usize, usize)> + '_ {
        found.sort_unstable();
        found.dedup();
        found.into_iter().map(|edge| {
            let (ring, index) = self.edges[edge as usize];
            (ring as usize, index as usize)
        })
    }

    /// The edges in the cells the segment from `a` to `b` passes
    /// through, as (ring, edge).
    pub(crate) fn along(&self, a: Point2, b: Point2) -> impl Iterator<Item = (usize, usize)> + '_ {
        let mut found = Vec::new();
        self.cells_along(a, b, |cell| self.collect(cell, &mut found));
        self.listed(found)
    }

    /// The edges in the cells the square of `radius` about `point` touches.
    pub(crate) fn near(
        &self,
        point: Point2,
        radius: Coord,
    ) -> impl Iterator<Item = (usize, usize)> + '_ {
        let mut found = Vec::new();
        for row in self.row(point.y - radius)..=self.row(point.y + radius) {
            for col in self.col(point.x - radius)..=self.col(point.x + radius) {
                self.collect((row * self.cols + col) as usize, &mut found);
            }
        }
        self.listed(found)
    }

    /// The edges in the cells from `point` to the grid's edge in the
    /// direction of +x: all that a ray that way could cross.
    pub(crate) fn rightward(&self, point: Point2) -> impl Iterator<Item = (usize, usize)> + '_ {
        let mut found = Vec::new();
        // A point above or below every edge is beside none of them.
        if (self.min.y..self.min.y + self.rows * self.cell).contains(&point.y) {
            let row = self.row(point.y);
            for col in self.col(point.x)..self.cols {
                self.collect((row * self.cols + col) as usize, &mut found);
            }
        }
        self.listed(found)
    }
}
