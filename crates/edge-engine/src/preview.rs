use edge_extrusion::Role;
use edge_geom::Point2;

use crate::Sliced;

/// One line of a layer as it will be laid, for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub role: Role,
    /// Line width in mm.
    pub width: f64,
    /// The filament it is printed in, by number from 1.
    pub filament: usize,
    /// Whether the last point joins back to the first.
    pub closed: bool,
    pub points: Vec<Point2>,
}

impl Sliced {
    /// Everything printed on layer `index`, tool by tool: prime tower,
    /// skirt and brim, supports, walls, fill. Moves between lines are not
    /// part of it, nor is the order of lines within each of those.
    pub fn strokes(&self, index: usize) -> Vec<Stroke> {
        let from = self.entries.partition_point(|entry| entry.layer < index);
        let turns = self.entries[from..]
            .iter()
            .take_while(|entry| entry.layer == index);
        let mut strokes = Vec::new();
        for (turn, entry) in turns.enumerate() {
            let (filament, tower) = (&entry.filament, &entry.lines);
            let number = self.slots[*filament];
            // A ram is laid by the tool that had the turn before.
            if let (Some(ram), Some(before)) = (
                &entry.ram,
                (from + turn).checked_sub(1).map(|at| &self.entries[at]),
            ) {
                strokes.extend(ram.lines.iter().map(|line| Stroke {
                    role: line.role,
                    width: line.flow.width,
                    filament: self.slots[before.filament],
                    closed: false,
                    points: line.points.clone(),
                }));
            }
            let own = &self.walls_by[*filament][index];
            // Skirt, brim and supports go to whichever tool starts the layer.
            let (around, supports) = if turn == 0 {
                (
                    self.around[index].as_slice(),
                    self.supports[index].as_slice(),
                )
            } else {
                (&[][..], &[][..])
            };
            let open = |line: &edge_extrusion::ExtrusionPath| Stroke {
                role: line.role,
                width: line.flow.width,
                filament: number,
                closed: false,
                points: line.points.clone(),
            };
            strokes.extend(tower.iter().map(open));
            strokes.extend(around.iter().chain(&own.loops).map(|wall| Stroke {
                role: wall.role,
                width: wall.flow.width,
                filament: number,
                closed: true,
                points: wall.polygon.points.clone(),
            }));
            strokes.extend(
                supports
                    .iter()
                    .chain(&self.fills_by[*filament][index])
                    .map(open),
            );
        }
        strokes
    }
}
