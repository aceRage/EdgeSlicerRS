//! Where every wall's seam goes, worked out for the whole print before
//! any of it is written: every point of every outermost wall as a place
//! its seam might go, with what the reference measures there, the best
//! of them for each wall, and where the walls behind start beside it.
//!
//! This is the reference's SeamPlacer.cpp. Its `init` does the same
//! before it exports: `gather_seam_candidates` (a `SeamCandidate` for
//! each point of each outer wall, with the angle the wall turns through
//! there), `calculate_overhangs_and_layer_embedding` (how far each hangs
//! out over the layer beneath, and how deep it lies inside the layer's
//! own outline where two materials meet), `pick_seam_point` and
//! `pick_random_seam_point` by its `SeamComparator`; and as each loop is
//! laid, `place_seam`.
//!
//! Not as the reference:
//! - It works object by object, in each object's own coordinates (from
//!   the middle of its bounds), and so does this, in single precision as
//!   it does; but a wall is measured against everything printed on the
//!   layer beneath and against the whole layer's outline, where the
//!   reference takes the object's own. The two differ only where objects
//!   overlap or one overhangs another.
//! - An extrusion loop of the reference's repeats its first point at its
//!   end, and a loop cut into stretches over air and stretches that rest
//!   repeats each point it is cut at, so its candidates do; a wall here
//!   is one polygon and each of its points is one candidate. (The
//!   reference counts the closing edge of such a loop, which has no
//!   length, as 0.1 mm long when it measures angles, so that a loop of
//!   no length at all cannot hang it; here a loop of no length has no
//!   angles.)
//! - A layer of an object with no walls at all is given one candidate
//!   at the object's middle there, which a line of seams may then be
//!   drawn to; here it has none.
//! - A loop opened at a seam that is not one of its points is opened at
//!   the nearest point of its stretches that rest, where it has any
//!   (`ExtrusionLoop::split_at`, `prefer_non_overhang`); here at the
//!   nearest point of the loop.
//! - With nothing known of an object's surface (no mesh given), nothing
//!   is known of how visible any place on it is, and every place counts
//!   as hidden as every other; the positions that lean to one side of
//!   the bed, which the reference leans by adding to that measure, are
//!   then leant by the way the wall itself faces.
//! - Seams are not yet lined up along a curve fitted through the layers
//!   (`align_seam_points`): for the seam positions the reference aligns,
//!   a seam follows the one beneath it where a point near enough is no
//!   worse by much, which is how the reference gathers the seams it
//!   then fits its curve to.

use std::{
    collections::HashMap,
    ops::{Not, Range},
};

use edge_extrusion::ExtrusionLoop;
use edge_geom::{ExPolygon, Point2};
use edge_visibility::{Triangles, Visibility};
use rayon::prelude::*;

use crate::{Layer, PrintSettings, Seam, address, overhang::Below, seam};

/// What seams are placed by that the layers' own walls do not say.
#[derive(Clone, Copy, Debug, Default)]
pub struct SeamScene<'a> {
    /// Each physical layer's outline, every filament's share together
    /// (the reference's `lslices`): one for each height, or none at all,
    /// and then no point is taken to be hidden inside the print.
    pub outlines: &'a [&'a [ExPolygon]],
    /// The objects on the plate; with none, everything is one object
    /// measured from the bed's corner.
    pub objects: &'a [SeamObject<'a>],
    /// The height of the bed in the meshes' coordinates, in mm.
    pub floor: f64,
}

/// One object on the plate.
#[derive(Clone, Copy, Debug)]
pub struct SeamObject<'a> {
    /// The least and greatest corners of its bounds across the bed.
    pub min: Point2,
    pub max: Point2,
    /// Where its seams are reckoned from: the middle of its bounds (the
    /// reference's `PrintObject::center_offset` and instance shift).
    pub centre: Point2,
    /// Its surface, for how visible each place on it is; without one,
    /// nothing is known of that.
    pub mesh: Option<SeamMesh<'a>>,
}

/// An object's surface: its parts' triangles, where it stands on the bed.
#[derive(Clone, Copy, Debug)]
pub struct SeamMesh<'a> {
    /// In mm.
    pub vertices: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
    /// The triangles from here on are of volumes cut out of the object
    /// (negative parts); the number of triangles where there are none.
    pub negative_from: usize,
}

/// An object's bounds and middle.
#[derive(Clone, Copy, Debug)]
struct Frame {
    min: Point2,
    max: Point2,
    centre: Point2,
}

/// One place an outer wall's seam might go (SeamPlacer.hpp
/// `SeamCandidate`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Where it is from its object's middle, in mm, and the height its
    /// layer was sliced at.
    pub(crate) position: [f32; 3],
    /// Which of its layer's perimeters it is on.
    pub(crate) perimeter: u32,
    /// How far the wall turns here, in radians: negative where it folds
    /// into the part (`local_ccw_angle`).
    pub(crate) angle: f32,
    /// How much of the part's outside can be seen from here; less is
    /// better hidden.
    pub(crate) visibility: f32,
    /// How far the wall's line hangs out over the layer beneath here, in
    /// mm, beyond what a 45 degree slope would; 0 where it rests.
    pub(crate) overhang: f32,
    /// How far the middle of the line is from the outline beneath, with
    /// two fifths of the line's width: what a conditional scarf seam goes
    /// by (`seam_overhang` in GCode.cpp `extrude_loop`).
    pub(crate) unsupported: f32,
    /// How far the point is from the layer's own outline, with 0.65 of
    /// the line's width, where the layer is printed in more than one
    /// material; negative inside (`embedded_distance`).
    pub(crate) embedded: f32,
}

/// One outer wall's candidates (SeamPlacer.hpp `Perimeter`): they are
/// `points[start..end]` of its layer, in the order the wall runs
/// counter-clockwise, from the wall's first point.
#[derive(Clone, Debug)]
pub(crate) struct Perimeter {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// The candidate chosen.
    pub(crate) seam: usize,
    /// Width of the wall's line, in mm.
    pub(crate) flow_width: f32,
    /// Where the seam was settled once seams were lined up, which need
    /// not be a point of the wall.
    pub(crate) finalized: Option<[f32; 3]>,
}

/// The candidates of one object on one layer.
#[derive(Clone, Debug, Default)]
pub(crate) struct LayerSeams {
    pub(crate) perimeters: Vec<Perimeter>,
    pub(crate) points: Vec<Candidate>,
    grid: Grid,
}

/// One object's candidates.
struct ObjectSeams {
    centre: Point2,
    /// By physical layer: every filament's walls at one height together.
    layers: Vec<LayerSeams>,
}

/// Every wall's seam.
pub(crate) struct Plan {
    seam: Seam,
    frames: Vec<Frame>,
    objects: Vec<ObjectSeams>,
    /// Which object each outer wall is of and which of its layer's
    /// perimeters it is, by the layer and the wall's address.
    walls: HashMap<(usize, usize), (u32, u32)>,
}

/// Where a wall starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    /// The corner the edge it starts on leaves from, and the place.
    pub(crate) edge: usize,
    pub(crate) start: Point2,
    /// How far the outer wall's line is unsupported at the seam the wall
    /// starts at or beside (see [`Candidate::unsupported`]); without
    /// end where nothing is known of the layer beneath.
    pub(crate) unsupported: f32,
}

/// The slope a wall may hang out by and still be a good place for a
/// seam, from the vertical (SeamPlacer.hpp `overhang_angle_threshold`).
const OVERHANG_ANGLE: f32 = 45.0 * std::f32::consts::PI / 180.0;
/// How far a seam is looked for from the one on the next layer, in line
/// widths (`seam_align_tolerable_dist_factor`).
const ALIGN_REACH: f32 = 4.0;
/// The root of two as the reference writes it where it reckons how deep
/// a wall lies behind a seam (`place_seam`).
#[allow(clippy::approx_constant)]
const ROOT_TWO: f64 = 1.4142;
/// How much worse a point may score and still be taken to keep seams in
/// line (`seam_align_score_tolerance`).
const ALIGN_TOLERANCE: f32 = 0.3;

/// The reference's bell curve (SeamPlacer.cpp `gauss`): `peak` at `mean`,
/// falling off faster the larger `falloff` is.
fn gauss(value: f32, mean: f32, peak: f32, falloff: f32) -> f32 {
    let shifted = value - mean;
    let exponent = 1.0 / (falloff * shifted * shifted + 1.0);
    peak * (exponent.exp() - 1.0) / (1.0f32.exp() - 1.0)
}

/// How bad a corner is as a seam, by the angle the wall turns through
/// there: least for a corner that folds inward, where a seam hides best,
/// most on a straight run, where it shows most (`compute_angle_penalty`).
pub(crate) fn angle_penalty(angle: f32) -> f32 {
    gauss(angle, 0.0, 1.0, 3.0) + 1.0 / (2.0 + (-angle).exp())
}

/// How two candidates are told apart (SeamPlacer.cpp `SeamComparator`).
/// Paint and the places where parts meet, which the reference weighs
/// first, are not looked at.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Comparator {
    setup: Seam,
    /// How much the angle counts beside visibility: all of it for the
    /// nearest corner, 0.6 otherwise (SeamPlacer.hpp
    /// `angle_importance_nearest`, `angle_importance_aligned`).
    angle_importance: f32,
}

impl Comparator {
    pub(crate) fn new(setup: Seam) -> Self {
        Self {
            setup,
            angle_importance: if setup == Seam::Nearest { 1.0 } else { 0.6 },
        }
    }

    /// Whether the seam is pulled one way by a rule and not by a score:
    /// to the back (`directional_seam_axis`).
    fn directional(&self) -> bool {
        self.setup == Seam::Back
    }

    fn penalty(&self, point: &Candidate) -> f32 {
        point.overhang + point.visibility + self.angle_importance * angle_penalty(point.angle)
    }

    /// Whether `a` is the better place (`is_first_better`). `preferred`
    /// is where the nozzle is, for the nearest corner.
    pub(crate) fn is_first_better(
        &self,
        a: &Candidate,
        b: &Candidate,
        preferred: [f32; 2],
    ) -> bool {
        // Avoid overhangs.
        if a.overhang > 0.0 || b.overhang > 0.0 {
            return a.overhang < b.overhang;
        }
        // Prefer hidden points (more than 0.5 mm inside).
        if a.embedded < -0.5 && b.embedded > -0.5 {
            return true;
        }
        if b.embedded < -0.5 && a.embedded > -0.5 {
            return false;
        }
        if self.directional() && a.position[1] != b.position[1] {
            return a.position[1] > b.position[1];
        }
        let (mut near_a, mut near_b) = (0.0, 0.0);
        if self.setup == Seam::Nearest {
            let away = |point: &Candidate| {
                let (dx, dy) = (
                    point.position[0] - preferred[0],
                    point.position[1] - preferred[1],
                );
                (dx * dx + dy * dy).sqrt()
            };
            near_a = 1.0 - gauss(away(a), 0.0, 1.0, 0.005);
            near_b = 1.0 - gauss(away(b), 0.0, 1.0, 0.005);
        }
        self.penalty(a) + near_a < self.penalty(b) + near_b
    }

    /// Whether `a` is no worse than `b` by much: what a seam may be
    /// moved to to keep seams in line, and what a seam put anywhere is
    /// picked among (`is_first_not_much_worse`). `flow_width` is the
    /// width of the line `a` is on.
    pub(crate) fn is_first_not_much_worse(
        &self,
        a: &Candidate,
        b: &Candidate,
        flow_width: f32,
    ) -> bool {
        if (a.overhang > 0.0 || b.overhang > 0.0)
            && (a.overhang - b.overhang).abs() > 0.1 * flow_width
        {
            return a.overhang < b.overhang;
        }
        if a.embedded < -0.5 && b.embedded > -0.5 {
            return true;
        }
        if b.embedded < -0.5 && a.embedded > -0.5 {
            return false;
        }
        if self.setup == Seam::Random {
            return true;
        }
        if self.directional() {
            return a.position[1] + ALIGN_TOLERANCE * 5.0 > b.position[1];
        }
        let (penalty_a, penalty_b) = (self.penalty(a), self.penalty(b));
        penalty_a <= penalty_b || penalty_a - penalty_b < ALIGN_TOLERANCE
    }
}

/// The setups whose seams the reference lines up from layer to layer:
/// the aligned ones, with those leant to one side of the bed, and the
/// back (`is_aligned_setup` and `spRear`, in `init_object`).
fn lined_up(setup: Seam) -> bool {
    !matches!(setup, Seam::Nearest | Seam::Random)
}

/// The way a setup leans its seams: toward this side of the bed
/// (`aligned_penalty_direction`).
fn lean(setup: Seam) -> Option<[f32; 2]> {
    match setup {
        Seam::AlignedBack => Some([0.0, 1.0]),
        Seam::AlignedFront => Some([0.0, -1.0]),
        Seam::Left => Some([-1.0, 0.0]),
        Seam::Right => Some([1.0, 0.0]),
        Seam::Nearest | Seam::Aligned | Seam::Back | Seam::Random => None,
    }
}

/// The candidates of a layer by where they are, for finding those near a
/// place (the reference keeps a k-d tree of each layer's).
#[derive(Clone, Debug, Default)]
struct Grid {
    min: [f32; 2],
    columns: usize,
    rows: usize,
    /// Where each cell's candidates start in `items`; one more than
    /// there are cells.
    starts: Vec<u32>,
    items: Vec<u32>,
}

/// The size of a grid cell, in mm.
const CELL: f32 = 2.0;

impl Grid {
    fn new(points: &[Candidate]) -> Self {
        if points.is_empty() {
            return Self::default();
        }
        let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
        for point in points {
            for axis in 0..2 {
                min[axis] = min[axis].min(point.position[axis]);
                max[axis] = max[axis].max(point.position[axis]);
            }
        }
        let columns = ((max[0] - min[0]) / CELL) as usize + 1;
        let rows = ((max[1] - min[1]) / CELL) as usize + 1;
        let mut grid = Self {
            min,
            columns,
            rows,
            starts: vec![0; columns * rows + 1],
            items: vec![0; points.len()],
        };
        for point in points {
            let cell = grid.cell(point.position);
            grid.starts[cell + 1] += 1;
        }
        for cell in 0..columns * rows {
            grid.starts[cell + 1] += grid.starts[cell];
        }
        let mut next = grid.starts.clone();
        for (index, point) in points.iter().enumerate() {
            let cell = grid.cell(point.position);
            grid.items[next[cell] as usize] = index as u32;
            next[cell] += 1;
        }
        grid
    }

    fn column(&self, x: f32) -> i64 {
        ((x - self.min[0]) / CELL).floor() as i64
    }

    fn row(&self, y: f32) -> i64 {
        ((y - self.min[1]) / CELL).floor() as i64
    }

    fn cell(&self, position: [f32; 3]) -> usize {
        let column = (self.column(position[0]).max(0) as usize).min(self.columns - 1);
        let row = (self.row(position[1]).max(0) as usize).min(self.rows - 1);
        row * self.columns + column
    }

    fn in_cell(&self, column: i64, row: i64) -> &[u32] {
        if column < 0 || row < 0 || column >= self.columns as i64 || row >= self.rows as i64 {
            return &[];
        }
        let cell = row as usize * self.columns + column as usize;
        &self.items[self.starts[cell] as usize..self.starts[cell + 1] as usize]
    }

    /// The candidates nearer `centre` than `reach`, in their own order
    /// (KDTreeIndirect.hpp `find_nearby_points`).
    fn nearby(&self, points: &[Candidate], centre: [f32; 3], reach: f32) -> Vec<usize> {
        let mut found = Vec::new();
        if points.is_empty() {
            return found;
        }
        let (left, right) = (
            self.column(centre[0] - reach),
            self.column(centre[0] + reach),
        );
        let (low, high) = (self.row(centre[1] - reach), self.row(centre[1] + reach));
        for row in low.max(0)..=high.min(self.rows as i64 - 1) {
            for column in left.max(0)..=right.min(self.columns as i64 - 1) {
                for &index in self.in_cell(column, row) {
                    if squared_distance(points[index as usize].position, centre) < reach * reach {
                        found.push(index as usize);
                    }
                }
            }
        }
        found.sort_unstable();
        found
    }

    /// The candidate nearest `at`, the first of those as near
    /// (`find_closest_point`).
    fn closest(&self, points: &[Candidate], at: [f32; 3]) -> Option<usize> {
        if points.is_empty() {
            return None;
        }
        let (column, row) = (self.column(at[0]), self.row(at[1]));
        let furthest = (column.abs().max((column - self.columns as i64).abs()))
            .max(row.abs().max((row - self.rows as i64).abs()))
            + 1;
        let mut best: Option<(f32, usize)> = None;
        for ring in 0..=furthest {
            if let Some((nearest, _)) = best {
                let clear = (ring - 1) as f32 * CELL;
                if clear > 0.0 && clear * clear > nearest {
                    break;
                }
            }
            for at_row in row - ring..=row + ring {
                let edge_row = (at_row - row).abs() == ring;
                let step = if edge_row { 1 } else { (2 * ring).max(1) };
                let mut at_column = column - ring;
                while at_column <= column + ring {
                    for &index in self.in_cell(at_column, at_row) {
                        let away = squared_distance(points[index as usize].position, at);
                        if best.is_none_or(|(nearest, first)| {
                            away < nearest || (away == nearest && (index as usize) < first)
                        }) {
                            best = Some((away, index as usize));
                        }
                    }
                    at_column += step;
                }
            }
        }
        best.map(|(_, index)| index)
    }
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}

/// The angle a closed counter-clockwise polygon turns through at each of
/// its points, between the points at least `arm` mm either side along
/// it, so that a curve cut into many short edges still reads as a curve.
/// `lengths` holds the length in mm of the edge on from each point. A
/// port of SeamPlacer.cpp `calculate_polygon_angles_at_vertices`.
fn polygon_angles(points: &[(i64, i64)], lengths: &[f32], arm: f32) -> Vec<f32> {
    let count = points.len();
    let mut result = vec![0.0; count];
    // A loop of no length has no angles.
    if count < 2 || lengths.iter().sum::<f32>() <= 0.0 {
        return result;
    }
    let next = |index: usize| (index + 1) % count;
    let previous = |index: usize| (index + count - 1) % count;
    let (mut before, mut after) = (0, 0);
    let (mut to_before, mut to_after) = (0.0f32, 0.0f32);
    // Push `before` far enough back to start with.
    while to_before < arm {
        before = previous(before);
        to_before += lengths[before];
    }
    for here in 0..count {
        // Pull `before` in as far as the arm allows.
        while to_before - lengths[before] > arm {
            to_before -= lengths[before];
            before = next(before);
        }
        // Push `after` on as far as the arm needs.
        while to_after < arm {
            to_after += lengths[after];
            after = next(after);
        }
        let (p0, p1, p2) = (points[before], points[here], points[after]);
        let (ax, ay) = ((p1.0 - p0.0) as f64, (p1.1 - p0.1) as f64);
        let (bx, by) = ((p2.0 - p1.0) as f64, (p2.1 - p1.1) as f64);
        result[here] = (ax * by - ay * bx).atan2(ax * bx + ay * by) as f32;
        to_before += lengths[here];
        to_after -= lengths[here];
    }
    result
}

impl Plan {
    /// Settles the seams of `layers`, of which `groups` says which are
    /// at one height. `settings` has an entry for each filament.
    pub(crate) fn new(
        layers: &[Layer<'_>],
        groups: &[Range<usize>],
        settings: &[PrintSettings],
        scene: &SeamScene<'_>,
    ) -> Self {
        let setup = layers
            .first()
            .and_then(|layer| settings.get(layer.filament))
            .map_or(Seam::Aligned, |settings| settings.seam);
        let mut frames: Vec<Frame> = (scene.objects.iter())
            .map(|object| Frame {
                min: object.min,
                max: object.max,
                centre: object.centre,
            })
            .collect();
        if frames.is_empty() {
            let corner = Point2 { x: 0, y: 0 };
            frames.push(Frame {
                min: corner,
                max: corner,
                centre: corner,
            });
        }
        let outlines = if scene.outlines.len() == groups.len() {
            scene.outlines
        } else {
            &[]
        };
        // How visible each object's surface is, for the setups that go
        // by it: the reference measures it for the aligned ones and for
        // the nearest corner (`init_object`).
        let seen: Vec<Option<Visibility>> = (0..frames.len())
            .map(|object| {
                let mesh = scene.objects.get(object)?.mesh?;
                matches!(setup, Seam::Back | Seam::Random)
                    .not()
                    .then(|| visibility(&mesh, frames[object].centre, scene.floor, setup))
            })
            .collect();
        let objects = &frames;
        // By layer, then by object.
        let gathered: Vec<Vec<(LayerSeams, Vec<usize>)>> = groups
            .par_iter()
            .enumerate()
            .map(|(index, group)| {
                gather(
                    &layers[group.clone()],
                    index == 0,
                    settings,
                    objects,
                    outlines.get(index).copied(),
                    setup,
                    &seen,
                )
            })
            .collect();
        let mut plan = Self {
            seam: setup,
            frames: frames.clone(),
            objects: objects
                .iter()
                .map(|object| ObjectSeams {
                    centre: object.centre,
                    layers: Vec::with_capacity(groups.len()),
                })
                .collect(),
            walls: HashMap::new(),
        };
        for (index, layer) in gathered.into_iter().enumerate() {
            for (object, (seams, addresses)) in layer.into_iter().enumerate() {
                for (perimeter, address) in addresses.into_iter().enumerate() {
                    plan.walls
                        .insert((index, address), (object as u32, perimeter as u32));
                }
                plan.objects[object].layers.push(seams);
            }
        }
        // For the nearest corner the seam is picked as the wall is
        // laid, by where the nozzle then is.
        if setup != Seam::Nearest {
            let comparator = Comparator::new(setup);
            for object in &mut plan.objects {
                object.layers.par_iter_mut().for_each(|layer| {
                    for at in 0..layer.perimeters.len() {
                        let (start, end) = (layer.perimeters[at].start, layer.perimeters[at].end);
                        if setup == Seam::Random {
                            let width = layer.perimeters[at].flow_width;
                            let (seam, position) =
                                pick_random(&layer.points[start..end], width, &comparator);
                            layer.perimeters[at].seam = start + seam;
                            layer.perimeters[at].finalized = Some(position);
                        } else {
                            layer.perimeters[at].seam =
                                start + pick(&layer.points[start..end], &comparator, [0.0; 2]);
                        }
                    }
                });
                if lined_up(setup) {
                    follow(&mut object.layers, &comparator);
                }
            }
        }
        plan
    }

    /// Where a wall of physical layer `layer` starts: an outermost wall
    /// at its seam, a wall behind one beside the seam of the outer wall
    /// nearest it (SeamPlacer.cpp `place_seam`). `turned` says the wall
    /// is laid the other way round from its points, `nozzle` where the
    /// nozzle is, and `staggered` that a wall behind another starts on
    /// round from it (`staggered_inner_seams`). None where the layer has
    /// no outer wall to go by.
    pub(crate) fn place(
        &self,
        layer: usize,
        wall: &ExtrusionLoop,
        turned: bool,
        nozzle: Option<Point2>,
        staggered: bool,
    ) -> Option<Placed> {
        let points = &wall.polygon.points;
        let count = points.len();
        let (object, perimeter) = match self.walls.get(&(layer, address(wall))) {
            Some(&(object, perimeter)) => (object as usize, perimeter as usize),
            None => {
                // The perimeter nearest the loop: the search is repeated
                // until two points of the loop running find the same one,
                // "because with arachne, T-Junctions may exist and
                // sometimes the wrong perimeter was chosen".
                let object = object_of(&self.frames, *points.first()?);
                let seams = self.objects[object].layers.get(layer)?;
                let centre = self.objects[object].centre;
                let z = seams.points.first()?.position[2];
                let mut found: Option<usize> = None;
                for step in 0..count {
                    let corner = if turned { (count - step) % count } else { step };
                    let [x, y] = centred(points[corner], centre);
                    let nearest = seams.grid.closest(&seams.points, [x, y, z])?;
                    let perimeter = seams.points[nearest].perimeter as usize;
                    if found == Some(perimeter) {
                        break;
                    }
                    found = Some(perimeter);
                }
                (object, found?)
            }
        };
        let centre = self.objects[object].centre;
        let seams = &self.objects[object].layers[layer];
        let perimeter = &seams.perimeters[perimeter];
        let (seam_index, position) = match perimeter.finalized {
            Some(position) => (perimeter.seam, position),
            None => {
                let index = if self.seam == Seam::Nearest {
                    // Before anything is laid the nozzle is taken to be
                    // at the bed's corner, as the reference takes it
                    // (GCode.cpp `set_origin` moves a last position of
                    // nought with the object's place).
                    let at = centred(nozzle.unwrap_or(Point2 { x: 0, y: 0 }), centre);
                    let candidates = &seams.points[perimeter.start..perimeter.end];
                    perimeter.start + pick(candidates, &Comparator::new(Seam::Nearest), at)
                } else {
                    perimeter.seam
                };
                (index, seams.points[index].position)
            }
        };
        let candidate = &seams.points[seam_index];
        let unsupported = candidate.unsupported;
        let on_bed = |x: f32, y: f32| Point2 {
            x: (f64::from(x) / 1e-6) as i64 + centre.x,
            y: (f64::from(y) / 1e-6) as i64 + centre.y,
        };
        let seam_point = on_bed(position[0], position[1]);
        // The loop is opened at one of its own points where the place
        // is within 1.5 microns of it (`ExtrusionLoop::split_at_vertex`:
        // "Because the G-code export has 1um resolution, don't generate
        // segments shorter than 1.5 microns").
        let placed = |(edge, start): (usize, Point2)| {
            let apart =
                |corner: Point2| ((corner.x - start.x) as f64).hypot((corner.y - start.y) as f64);
            let ends = [points[edge], points[(edge + 1) % count]];
            let nearer = if apart(ends[0]) <= apart(ends[1]) {
                0
            } else {
                1
            };
            let start = if apart(ends[nearer]) < 1500.0 {
                ends[nearer]
            } else {
                start
            };
            Some(Placed {
                edge,
                start,
                unsupported,
            })
        };
        if wall.depth == 0 {
            return placed(seam::project(points, seam_point));
        }

        // A wall behind another: beside the seam, at the point nearest
        // it, unless the outer wall folds inward there.
        let (mut edge, mut start) = seam::project(points, seam_point);
        let mm = |a: Point2, b: Point2| {
            (((a.x - b.x) as f64 * 1e-6).hypot((a.y - b.y) as f64 * 1e-6)) as f32
        };
        let mut depth = mm(seam_point, start);
        let beta = (candidate.angle / 2.0).cos();
        // "seam is on perimeter point": the reference sets a distance
        // squared against the depth, mm² against mm, and so does this.
        let on_point = squared_distance(position, candidate.position) < depth;
        if on_point && candidate.angle < -1e-4 {
            // The outer wall's seam is in an inward corner: the wall
            // behind starts in its own corner, not at the foot of a
            // perpendicular on the nearer edge. The depth is overshot;
            // in an inward corner the place then snaps to the corner.
            let size = perimeter.end - perimeter.start;
            let at = seam_index - perimeter.start;
            let before = seams.points[perimeter.start + (at + size - 1) % size].position;
            let after = seams.points[perimeter.start + (at + 1) % size].position;
            let unit = |from: [f32; 3]| {
                let (dx, dy) = (
                    candidate.position[0] - from[0],
                    candidate.position[1] - from[1],
                );
                let length = (dx * dx + dy * dy).sqrt();
                if length > 0.0 {
                    [dx / length, dy / length]
                } else {
                    [dx, dy]
                }
            };
            let (from_before, from_after) = (unit(before), unit(after));
            let middle = [
                (from_before[0] + from_after[0]) * 0.5,
                (from_before[1] + from_after[1]) * 0.5,
            ];
            depth = (ROOT_TWO * f64::from(depth) / f64::from(beta)) as f32;
            let target = on_bed(
                candidate.position[0] + depth * middle[0],
                candidate.position[1] + depth * middle[1],
            );
            (edge, start) = seam::project(points, target);
        } else {
            // Elsewhere the nearest point is the place; but for the
            // walls to start on round from each other the depth wanted
            // is the one square to the wall, which at an outward corner
            // is less than the distance to the nearest point.
            depth = (f64::from(depth * beta) / ROOT_TWO) as f32;
        }
        if staggered && wall.length_mm() > 0.0 {
            // "fix depth, it is sometimes strongly underestimated"
            let depth = (wall.flow.width as f32).max(depth);
            (edge, start) = seam::advance(points, edge, start, f64::from(depth) / 1e-6, turned);
        }
        placed((edge, start))
    }
}

/// A point of the bed from an object's middle, in mm, as the reference
/// holds a candidate's place.
fn centred(point: Point2, centre: Point2) -> [f32; 2] {
    [
        ((point.x - centre.x) as f64 * 1e-6) as f32,
        ((point.y - centre.y) as f64 * 1e-6) as f32,
    ]
}

/// The object a point is of: of those whose bounds hold it the one with
/// the smallest bounds, so an object stood inside another's outline is
/// told from it; else the one whose bounds are nearest.
fn object_of(objects: &[Frame], point: Point2) -> usize {
    let off = |object: &Frame| {
        let dx = (object.min.x - point.x).max(point.x - object.max.x).max(0);
        let dy = (object.min.y - point.y).max(point.y - object.max.y).max(0);
        (dx as f64).hypot(dy as f64)
    };
    let area = |object: &Frame| {
        (object.max.x - object.min.x) as f64 * (object.max.y - object.min.y) as f64
    };
    (0..objects.len())
        .min_by(|&a, &b| {
            (off(&objects[a]), area(&objects[a]))
                .partial_cmp(&(off(&objects[b]), area(&objects[b])))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        })
        .unwrap_or(0)
}

/// Which of a wall's points the reference's loop starts at: the one
/// after its lowest point, going round the way the wall is held (of
/// several as low in a row, after the last). Its walls come out of its
/// clipping library (Clipper, `Clipper::BuildResult`), which sweeps a
/// polygon from the top down and lists it from the right-hand side of
/// where it closes at the bottom, so that the lowest point comes last;
/// the library this engine clips with starts a polygon elsewhere. The
/// first point matters wherever points are as good as each other, of
/// which the first is taken: on a box every corner is.
fn first_point(points: &[Point2]) -> usize {
    let count = points.len();
    let lowest = points.iter().map(|point| point.y).min().unwrap_or(0);
    let last_of_run = (0..count)
        .filter(|&at| points[at].y == lowest && points[(at + 1) % count].y != lowest)
        // Of several such runs, the one furthest right.
        .max_by_key(|&at| (points[at].x, std::cmp::Reverse(at)));
    last_of_run.map_or(0, |at| (at + 1) % count)
}

/// The best of a wall's candidates, the first of those as good
/// (`pick_seam_point`, `pick_nearest_seam_point_index`).
fn pick(candidates: &[Candidate], comparator: &Comparator, preferred: [f32; 2]) -> usize {
    let mut seam = 0;
    for index in 0..candidates.len() {
        if comparator.is_first_better(&candidates[index], &candidates[seam], preferred) {
            seam = index;
        }
    }
    seam
}

/// A point somewhere along an outer wall, by a figure made from the
/// wall's first point: the same for the same wall, and scattered from
/// layer to layer. Only the stretches that set off from points about as
/// good as the best are picked among, which leaves out those over a
/// steep overhang. Gives the candidate the stretch sets off from and the
/// place. A port of `pick_random_seam_point`, in single precision as it
/// is there.
fn pick_random(
    candidates: &[Candidate],
    flow_width: f32,
    comparator: &Comparator,
) -> (usize, [f32; 3]) {
    let [x, y, z] = candidates[0].position;
    // The three products are summed as the reference's vector library
    // sums them (Eigen's `dot`: the first to the sum of the other two).
    let mut rand = ((x * 12.9898 + (y * 78.233 + z * 133.3333)).sin() * 43_758.547).abs();
    rand -= rand.trunc();
    let count = candidates.len();
    // The points as good as the best met so far, each with the stretch
    // on to the next point: its length and which way it goes.
    let mut example = 0;
    let mut viable: Vec<(usize, f32, [f32; 2])> = Vec::new();
    for index in 0..count {
        let (here, best) = (&candidates[index], &candidates[example]);
        let worse = comparator.is_first_not_much_worse(best, here, flow_width);
        let similar = worse && comparator.is_first_not_much_worse(here, best, flow_width);
        if !similar && worse {
            continue;
        }
        if !similar {
            example = index;
            viable.clear();
        }
        let next = candidates[(index + 1) % count].position;
        let (dx, dy) = (next[0] - here.position[0], next[1] - here.position[1]);
        viable.push((index, (dx * dx + dy * dy).sqrt(), [dx, dy]));
    }
    let mut picked = viable.iter().map(|edge| edge.1).sum::<f32>() * rand;
    let mut at = 0;
    while at + 1 < viable.len() && picked - viable[at].1 > 0.0 {
        picked -= viable[at].1;
        at += 1;
    }
    let (index, length, [dx, dy]) = viable[at];
    let from = candidates[index].position;
    if length <= 0.0 {
        return (index, from);
    }
    (
        index,
        [
            from[0] + dx / length * picked,
            from[1] + dy / length * picked,
            from[2],
        ],
    )
}

/// Whether there is a good seam on a layer near `projected`, the place
/// of a seam on the layer next to it: the candidate nearest it, or the
/// best of those within `reach`, if that is no worse by much than the
/// seam of the wall it is on. Gives the candidate. A port of
/// `SeamPlacer::find_next_seam_in_layer`.
fn find_next_seam_in_layer(
    layer: &LayerSeams,
    projected: [f32; 3],
    reach: f32,
    comparator: &Comparator,
) -> Option<usize> {
    let nearby = layer.grid.nearby(&layer.points, projected, reach);
    let (mut best, mut nearest) = (*nearby.first()?, *nearby.first()?);
    let settled = |index: usize| {
        layer.perimeters[layer.points[index].perimeter as usize]
            .finalized
            .is_some()
    };
    let preferred = [projected[0], projected[1]];
    for &index in &nearby {
        let point = &layer.points[index];
        // Skip the walls whose seams are settled; try to find one that
        // is not.
        if settled(index) {
            continue;
        }
        if comparator.is_first_better(point, &layer.points[best], preferred) || settled(best) {
            best = index;
        }
        if squared_distance(point.position, projected)
            < squared_distance(layer.points[nearest].position, projected)
            || settled(nearest)
        {
            nearest = index;
        }
    }
    if settled(nearest) {
        return None;
    }
    let perimeter = &layer.perimeters[layer.points[nearest].perimeter as usize];
    let own = &layer.points[perimeter.seam];
    // First the nearest, then the best nearby.
    if comparator.is_first_not_much_worse(&layer.points[nearest], own, perimeter.flow_width) {
        return Some(nearest);
    }
    let width = layer.perimeters[layer.points[best].perimeter as usize].flow_width;
    comparator
        .is_first_not_much_worse(&layer.points[best], own, width)
        .then_some(best)
}

/// Keeps seams in line from the bottom up: a wall's seam goes to the
/// point nearest a seam of the layer beneath, or the best near it, where
/// that is no worse by much. This stands in for the reference's
/// `align_seam_points`, which gathers its strings of seams by the same
/// step (`find_seam_string`) both ways from the best seams and then
/// fits a curve through each.
fn follow(layers: &mut [LayerSeams], comparator: &Comparator) {
    for index in 1..layers.len() {
        let (beneath, above) = layers.split_at_mut(index);
        let (beneath, layer) = (&beneath[index - 1], &mut above[0]);
        let Some(z) = layer.points.first().map(|point| point.position[2]) else {
            continue;
        };
        for perimeter in &beneath.perimeters {
            let from = beneath.points[perimeter.seam].position;
            let reach = ALIGN_REACH * perimeter.flow_width;
            let projected = [from[0], from[1], z];
            if let Some(found) = find_next_seam_in_layer(layer, projected, reach, comparator) {
                let position = layer.points[found].position;
                let moved = &mut layer.perimeters[layer.points[found].perimeter as usize];
                moved.seam = found;
                moved.finalized = Some(position);
            }
        }
    }
}

/// How visible an object's surface is: its mesh brought to the object's
/// own coordinates, as the reference holds it, and measured
/// (`compute_global_occlusion`).
fn visibility(mesh: &SeamMesh<'_>, centre: Point2, floor: f64, setup: Seam) -> Visibility {
    let middle = [centre.x as f64 * 1e-6, centre.y as f64 * 1e-6, floor];
    let triangles = Triangles {
        vertices: (mesh.vertices.iter())
            .map(|vertex| {
                std::array::from_fn(|axis| (f64::from(vertex[axis]) - middle[axis]) as f32)
            })
            .collect(),
        indices: mesh.triangles.to_vec(),
    };
    let negative_from = mesh.negative_from.min(triangles.indices.len());
    let toward = lean(setup).map(|[x, y]| [x, y, 0.0]);
    Visibility::new(&triangles, negative_from, toward)
}

/// The candidates of one physical layer, object by object, each with
/// the addresses of the walls its perimeters are.
fn gather(
    parts: &[Layer<'_>],
    first: bool,
    settings: &[PrintSettings],
    objects: &[Frame],
    outline: Option<&[ExPolygon]>,
    setup: Seam,
    seen: &[Option<Visibility>],
) -> Vec<(LayerSeams, Vec<usize>)> {
    let mut found: Vec<(LayerSeams, Vec<usize>)> = objects
        .iter()
        .map(|_| (LayerSeams::default(), Vec::new()))
        .collect();
    let Some(layer) = parts.first() else {
        return found;
    };
    let outer = |wall: &&ExtrusionLoop| wall.depth == 0 && wall.polygon.points.len() >= 3;
    // Which object each outer wall is of, and in how many materials
    // each object is walled on this layer.
    let owners: Vec<Vec<usize>> = parts
        .iter()
        .map(|part| {
            (part.loops.iter().filter(outer))
                .map(|wall| object_of(objects, wall.polygon.points[0]))
                .collect()
        })
        .collect();
    let mut materials = vec![0; objects.len()];
    for owned in &owners {
        for (object, count) in materials.iter_mut().enumerate() {
            *count += usize::from(owned.contains(&object));
        }
    }
    // Any distance will do for the grids: the distance asked for is exact.
    let below = (!first).then(|| Below::new(layer.below, 1.0)).flatten();
    let own = (materials.iter().any(|count| *count > 1))
        .then(|| outline.and_then(|outline| Below::new(outline, 1.0)))
        .flatten();
    // The height the layer was sliced at.
    let z = (layer.z - layer.height / 2.0) as f32;
    for (part, owned) in parts.iter().zip(&owners) {
        // The reference takes a loop for an outer wall by its kind or
        // by its being the outermost (`extract_perimeter_polygons`).
        for (wall, &object) in part.loops.iter().filter(outer).zip(owned) {
            let points = &wall.polygon.points;
            let centre = objects[object].centre;
            let (seams, addresses) = &mut found[object];
            let flow_width = wall.flow.width as f32;
            let start = seams.points.len();
            let count = points.len();
            // Counter-clockwise from the loop's first point
            // (`process_perimeter_polygon`: `make_counter_clockwise`,
            // which turns a hole's points about).
            let first = first_point(points);
            let order: Vec<usize> = (0..count)
                .map(|step| {
                    if wall.is_hole {
                        (first + count - step) % count
                    } else {
                        (first + step) % count
                    }
                })
                .collect();
            let ring: Vec<(i64, i64)> = (order.iter())
                .map(|&corner| (points[corner].x - centre.x, points[corner].y - centre.y))
                .collect();
            let lengths: Vec<f32> = (0..count)
                .map(|at| {
                    let (a, b) = (ring[at], ring[(at + 1) % count]);
                    (((a.0 - b.0) as f64 * 1e-6).hypot((a.1 - b.1) as f64 * 1e-6)) as f32
                })
                .collect();
            let arm = settings
                .get(part.filament)
                .map_or(0.5, |settings| settings.nozzle_diameter as f32);
            let angles = polygon_angles(&ring, &lengths, arm);
            for (at, &corner) in order.iter().enumerate() {
                let [x, y] = centred(points[corner], centre);
                let on_bed = (
                    f64::from(x) + centre.x as f64 * 1e-6,
                    f64::from(y) + centre.y as f64 * 1e-6,
                );
                // With nothing known of the layer beneath (the first
                // layer, or a layer given no outline to measure by)
                // nothing hangs out, and nothing is known to rest.
                let (mut overhang, mut unsupported, mut embedded) = (0.0, f32::INFINITY, 0.0);
                if let Some(below) = &below {
                    // `calculate_overhangs_and_layer_embedding`, in the
                    // precision it is worked out in there.
                    let distance = below.distance(on_bed.0, on_bed.1);
                    overhang = ((distance + f64::from(0.65 * flow_width)
                        - f64::from(OVERHANG_ANGLE.tan()) * part.height)
                        as f32)
                        .max(0.0);
                    unsupported = (distance + f64::from(0.4 * flow_width)) as f32;
                }
                if let Some(own) = own.as_ref().filter(|_| materials[object] > 1) {
                    embedded =
                        (own.distance(on_bed.0, on_bed.1) + f64::from(0.65 * flow_width)) as f32;
                }
                // How visible the part's surface is here. With nothing
                // known of the part's surface, a setup that leans to one
                // side of the bed is leant by which way the wall faces:
                // the reference adds as much to the visibility of each
                // sample of the surface, by the way the surface faces
                // (`raycast_visibility`).
                let visibility = match &seen[object] {
                    Some(seen) => seen.at([x, y, z]),
                    None => lean(setup).map_or(0.0, |toward| {
                        let (before, after) =
                            (ring[(at + count - 1) % count], ring[(at + 1) % count]);
                        let (dx, dy) = ((after.0 - before.0) as f32, (after.1 - before.1) as f32);
                        let length = (dx * dx + dy * dy).sqrt();
                        // To the right of the way round is out of the
                        // material, round a hole to the left.
                        let side = if wall.is_hole { -1.0 } else { 1.0 };
                        let normal = if length > 0.0 {
                            [side * dy / length, -side * dx / length]
                        } else {
                            [0.0, 0.0]
                        };
                        let facing = -(normal[0] * toward[0] + normal[1] * toward[1]);
                        ((facing + 1.2) * 0.5).clamp(0.0, 1.0)
                    }),
                };
                seams.points.push(Candidate {
                    position: [x, y, z],
                    perimeter: seams.perimeters.len() as u32,
                    // A hole was turned about to be measured.
                    angle: if wall.is_hole {
                        -angles[at]
                    } else {
                        angles[at]
                    },
                    visibility,
                    overhang,
                    unsupported,
                    embedded,
                });
            }
            seams.perimeters.push(Perimeter {
                start,
                end: seams.points.len(),
                seam: start,
                flow_width,
                finalized: None,
            });
            addresses.push(address(wall));
        }
    }
    for (seams, _) in &mut found {
        seams.grid = Grid::new(&seams.points);
    }
    found
}

#[cfg(test)]
#[path = "seam_plan_tests.rs"]
mod tests;
