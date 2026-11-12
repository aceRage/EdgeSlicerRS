//! The prime tower the reference gives Bambu Lab printers, and any printer
//! set to `wipe_tower_type` `type1`: WipeTower.cpp, Bambu Studio's
//! (Print.cpp `wipe_tower_type`, `_make_wipe_tower`).
//!
//! Its layout, as the reference has it:
//!
//! - Every tool change takes a block across the tower, in the order the
//!   changes come, deep enough for `prime_volume` in whole rows
//!   (`plan_toolchange`): the change G-code purges the flush itself, and
//!   the tower only primes. Rows run from one inside face of the wall to
//!   the other, a line apart; on the tower's first layer they carry 15%
//!   more plastic, and a row over the open part of the layer below is
//!   laid as thick as a 0.2 mm layer's (`toolchange_Wipe`,
//!   `need_thick_bridge_flow`).
//! - A tower too shallow for its height has its rows spread apart, or
//!   with a ribbed wall its ribs lengthened; a ribbed tower is squared
//!   (`plan_tower`, `make_rib_tower_square`).
//! - The first change of a layer lays the wall before its block, starting
//!   at a corner that goes round the tower a layer at a time, and then
//!   the layer's frame and the lines that carry the next layer across the
//!   depth no change uses (`tool_change`, `finish_layer`). A layer with no
//!   change is laid whole by the nozzle in hand: frame, lines, wall.
//! - The brim is laid round the wall on the first layer, and a loop fewer
//!   each layer above it up to 3 mm wide.
//!
//! What is laid is handed to the writer move by move ([`Wipe`]), with the
//! speeds the reference's tower writer gives them; moves it writes
//! without a speed go at the speed in force ([`TowerStroke::Glide`]).
//!
//! Figures are worked in single precision, as there, so that rows, depths
//! and corners land where the reference's do.

use std::f32::consts::PI;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role, TowerStroke};
use edge_geom::{ExPolygon, Point2, scaled, unscaled};

use crate::{Change, Tower, TowerError, TowerLayer, TowerSettings, Wall, Wipe, next_feed};

/// libslic3r's `EPSILON` and the tower's own `WT_EPSILON`.
const EPSILON: f32 = 1e-4;
const WT_EPSILON: f32 = 1e-3;
/// The tower's lines are this many nozzle diameters wide.
const WIDTH_TO_NOZZLE: f32 = 1.25;
/// The furthest apart the lines across unused depth are (`m_bridging`,
/// fixed at 10 mm here whatever `wipe_tower_bridging` says).
const BRIDGING: f32 = 10.0;
/// The widest the brim goes on above the first layer (`finish_layer`).
const MAX_CHAMFER: f32 = 3.0;
/// The wall is thinned to this before it is laid, and a brim loop first
/// to the second (`WIPE_TOWER_RESOLUTION`, `WT_SIMPLIFY_TOLERANCE_SCALED`).
const RESOLUTION: f64 = 0.1;
const PRE_SIMPLIFY: f64 = 0.001;
/// How fast the nozzle goes from its block to the layer's frame, in
/// mm/min (`merge_tcr`).
const MERGE_FEED: f32 = 7200.0;
/// Rows over the open part of the layer below are as thick as this
/// layer's would be (`need_thick_bridge_flow`).
const BRIDGE_HEIGHT: f32 = 0.2;

/// What the reference's Type1 tower reads beyond what both towers read.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Type1 {
    /// `prime_tower_line_width`; nothing is the nozzle's 1.25 times.
    pub line_width: f64,
    /// `prime_tower_brim_width` as given: below nothing is "auto", by the
    /// print's height.
    pub brim_width: f64,
    /// `wipe_tower_max_purge_speed` (Bambu's `prime_tower_max_speed`),
    /// in mm/s: the wall and frame are held to it, the rows to it and
    /// 80 mm/s.
    pub max_speed: f64,
    /// Each filament's `filament_max_volumetric_speed`, in mm³/s; 0 for
    /// none.
    pub max_volumetric: Vec<f64>,
    /// Whether each filament is soluble or support: the first change of
    /// a layer to one that is neither lays the wall and the frame
    /// (`first_toolchange_to_nonsoluble_nonsupport`).
    pub soluble: Vec<bool>,
    pub support: Vec<bool>,
    /// Each filament's `wipe_distance`: how far the nozzle wipes back along
    /// a ribbed wall it has just laid.
    pub wipe_distance: Vec<f64>,
    /// Whether arcs are fitted to a ribbed wall (`enable_arc_fitting`).
    pub arc_fitting: bool,
}

/// One tool change as planned (`WipeTowerInfo::ToolChange`).
#[derive(Clone, Debug)]
struct Planned {
    new: usize,
    /// The depth its block takes.
    depth: f32,
    /// What it lays down, in mm³, and as a length of row.
    volume: f32,
    length: f32,
}

/// One layer of the tower as planned (`WipeTowerInfo`).
#[derive(Clone, Debug)]
struct Plan {
    /// Its top, and how thick it is.
    z: f32,
    height: f32,
    /// What it and the layers above need of the tower's depth.
    depth: f32,
    /// How far apart its rows are, in line widths.
    spacing: f32,
    changes: Vec<Planned>,
}

impl Plan {
    fn used(&self) -> f32 {
        self.changes.iter().map(|change| change.depth).sum()
    }
}

fn align_round(value: f32, base: f32) -> f32 {
    (value / base).round() * base
}

fn align_ceil(value: f32, base: f32) -> f32 {
    (value / base).ceil() * base
}

fn align_floor(value: f32, base: f32) -> f32 {
    (value / base).floor() * base
}

/// How long a line `width` wide on a layer `height` thick lays down
/// `volume` mm³ (`volume_to_length`).
fn volume_to_length(volume: f32, width: f32, height: f32) -> f32 {
    (volume / (height * (width - height * (1.0 - PI / 4.0)))).max(0.0)
}

/// The least depth a tower `height` mm tall should have to stand
/// (`get_limit_depth_by_height`), in single precision.
fn limit_depth(height: f32) -> f32 {
    if height <= 100.0 {
        20.0
    } else if height >= 250.0 {
        40.0
    } else {
        20.0 + (height - 100.0) / 150.0 * 20.0
    }
}

/// The brim "auto" gives a tower so tall (`get_auto_brim_by_height`).
fn auto_brim(height: f32) -> f32 {
    if height < 100.0 {
        height / 100.0 * 8.0
    } else {
        8.0
    }
}

/// A box, in the tower's own frame (`box_coordinates`).
#[derive(Clone, Copy, Debug)]
struct Box2 {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

impl Box2 {
    fn new(left: f32, bottom: f32, width: f32, height: f32) -> Self {
        Self {
            left,
            bottom,
            right: left + width,
            top: bottom + height,
        }
    }

    /// `ld`, `rd`, `ru`, `lu`.
    fn corners(&self) -> [(f32, f32); 4] {
        [
            (self.left, self.bottom),
            (self.right, self.bottom),
            (self.right, self.top),
            (self.left, self.top),
        ]
    }
}

/// The ribs' state once planned.
#[derive(Clone, Copy, Debug)]
struct Ribs {
    width: f32,
    length: f32,
    extra: f32,
    fillet: bool,
}

/// What the nozzle does on one visit to the tower, as the reference's
/// `WipeTowerWriter` writes it, in the tower's frame.
struct Pen {
    /// Where the tower's frame is on the bed for this layer: its corner,
    /// and how far up the layer's own front is (`m_y_shift`).
    origin: (f32, f32),
    shift: f32,
    start: (f32, f32),
    at: (f32, f32),
    /// The speed in force, in mm/min; nothing until one is written.
    feed: f32,
    strokes: Vec<TowerStroke>,
    /// What is laid, each run at one flow.
    lines: Vec<ExtrusionPath>,
    wipe: Vec<Point2>,
}

impl Pen {
    fn new(origin: (f32, f32), shift: f32, start: (f32, f32)) -> Self {
        Self {
            origin,
            shift,
            start,
            at: start,
            feed: 0.0,
            strokes: Vec::new(),
            lines: Vec::new(),
            wipe: Vec::new(),
        }
    }

    /// A point of the tower's frame on the bed (`WipeTowerWriter::rotate`
    /// and `post_process_wipe_tower_moves`, with no rotation).
    fn bed(&self, (x, y): (f32, f32)) -> Point2 {
        Point2::from_mm(
            f64::from(x + self.origin.0),
            f64::from(y + self.shift + self.origin.1),
        )
    }

    /// A move without laying anything, at the speed in force, or at
    /// `feed` mm/min when given (`travel`).
    fn travel(&mut self, to: (f32, f32), feed: Option<f32>) {
        if to == self.at && feed.is_none_or(|feed| feed == self.feed) {
            return;
        }
        let point = self.bed(to);
        match feed {
            Some(feed) => {
                self.feed = feed;
                self.strokes
                    .push(TowerStroke::Move(point, Some(f64::from(feed) / 60.0)));
            }
            None => self.strokes.push(TowerStroke::Glide(point)),
        }
        self.at = to;
    }

    /// A line laid at `flow`, at `feed` mm/min when given and at the
    /// speed in force otherwise (`extrude`).
    fn lay(&mut self, to: (f32, f32), feed: Option<f32>, flow: Flow) {
        if let Some(feed) = feed {
            self.feed = feed;
        }
        if to == self.at {
            return;
        }
        let (from, point) = (self.bed(self.at), self.bed(to));
        self.strokes
            .push(TowerStroke::LayAs(point, f64::from(self.feed) / 60.0, flow));
        match self.lines.last_mut() {
            Some(line) if line.flow == flow && line.points.last() == Some(&from) => {
                line.points.push(point);
            }
            _ => self.lines.push(ExtrusionPath {
                points: vec![from, point],
                role: Role::PrimeTower,
                flow,
                in_turn: false,
                part: 0,
            }),
        }
        self.at = to;
    }

    /// A closed outline, from the corner nearest the nozzle round to it
    /// again (`WipeTowerWriter::polygon`): thinned to the tower's
    /// resolution from its own first corner, as there, and laid from the
    /// corner left that is nearest; or, where arcs are asked for, handed
    /// on whole from the corner nearest of those, for the writer to fit
    /// arcs to ([`TowerStroke::Bend`]).
    fn polygon(
        &mut self,
        outline: &[(f64, f64)],
        feed: f32,
        flow: Flow,
        pre_simplify: bool,
        arcs: bool,
    ) {
        let mut path: Vec<(f64, f64)> = outline.iter().chain(outline.first()).copied().collect();
        if pre_simplify {
            path = douglas_peucker(&path, PRE_SIMPLIFY);
        }
        let thin = douglas_peucker(&path, RESOLUTION);
        if thin.len() < 2 || path.len() < 2 {
            return;
        }
        let mut nearest = thin[0];
        let mut least = f32::MAX;
        for start in &thin[..thin.len() - 1] {
            let (x, y) = (start.0 as f32, start.1 as f32);
            let distance = (x - self.at.0).powi(2) + (y - self.at.1).powi(2);
            if distance < least {
                (nearest, least) = (*start, distance);
            }
        }
        let laid = if arcs { &path } else { &thin };
        let count = laid.len() - 1;
        let first = laid[..count]
            .iter()
            .position(|p| *p == nearest)
            .unwrap_or(0);
        let corner = |index: usize| {
            let (x, y) = laid[(first + index) % count];
            (x as f32, y as f32)
        };
        self.travel(corner(0), None);
        self.feed = feed;
        for step in 1..=count {
            let to = corner(step);
            if to == self.at {
                continue;
            }
            let (from, point) = (self.bed(self.at), self.bed(to));
            let speed = f64::from(feed) / 60.0;
            self.strokes.push(if arcs {
                TowerStroke::Bend(point, speed, flow)
            } else {
                TowerStroke::LayAs(point, speed, flow)
            });
            self.extend(from, point, flow);
            self.at = to;
        }
    }

    /// Adds a line laid from `from` to `to` to what is laid.
    fn extend(&mut self, from: Point2, to: Point2, flow: Flow) {
        match self.lines.last_mut() {
            Some(line) if line.flow == flow && line.points.last() == Some(&from) => {
                line.points.push(to);
            }
            _ => self.lines.push(ExtrusionPath {
                points: vec![from, to],
                role: Role::PrimeTower,
                flow,
                in_turn: false,
                part: 0,
            }),
        }
    }

    /// A box, from its corner nearest the nozzle round to it again
    /// (`WipeTowerWriter::rectangle`): reached along its sides.
    fn rectangle(&mut self, area: &Box2, feed: f32, flow: Flow) {
        let corners = area.corners();
        let mut nearest = 0;
        if self.at.0 - area.left > area.right - self.at.0 {
            nearest = 1;
        }
        if self.at.1 - area.bottom > area.top - self.at.1 {
            nearest = if nearest == 0 { 3 } else { 2 };
        }
        self.travel((corners[nearest].0, self.at.1), None);
        self.travel((self.at.0, corners[nearest].1), None);
        for step in 1..=4 {
            self.lay(corners[(nearest + step) % 4], Some(feed), flow);
        }
    }

    fn finish(self) -> (Wipe, Vec<ExtrusionPath>, (f32, f32)) {
        let flow = self
            .lines
            .first()
            .map(|line| line.flow)
            .unwrap_or_else(|| Flow::new(0.5, 0.2, 0.4).expect("a valid flow"));
        let at = self.at;
        let start = self.bed(self.start);
        (
            Wipe {
                start,
                strokes: self.strokes,
                flow,
                wipe_path: self.wipe,
            },
            self.lines,
            at,
        )
    }
}

/// `MultiPoint::_douglas_peucker`, on a path in mm.
fn douglas_peucker(points: &[(f64, f64)], tolerance: f64) -> Vec<(f64, f64)> {
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut kept = vec![first];
    if points.len() == 1 {
        return kept;
    }
    let tolerance_sq = tolerance * tolerance;
    let mut anchor = 0;
    let mut floaters = vec![points.len() - 1];
    while let Some(&floater) = floaters.last() {
        let (mut furthest, mut furthest_sq) = (anchor, 0.0);
        for index in anchor + 1..floater {
            let distance_sq =
                distance_to_segment_sq(points[index], points[anchor], points[floater]);
            if distance_sq > furthest_sq {
                (furthest, furthest_sq) = (index, distance_sq);
            }
        }
        if furthest_sq <= tolerance_sq {
            kept.push(points[floater]);
            anchor = floater;
            floaters.pop();
        } else {
            floaters.push(furthest);
        }
    }
    kept
}

fn distance_to_segment_sq(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (ax, ay) = (p.0 - a.0, p.1 - a.1);
    let l2 = vx * vx + vy * vy;
    if l2 == 0.0 {
        return ax * ax + ay * ay;
    }
    let t = (ax * vx + ay * vy) / l2;
    if t <= 0.0 {
        ax * ax + ay * ay
    } else if t >= 1.0 {
        let (bx, by) = (p.0 - b.0, p.1 - b.1);
        bx * bx + by * by
    } else {
        let (ox, oy) = (ax - t * vx, ay - t * vy);
        ox * ox + oy * oy
    }
}

fn to_mm(region: &ExPolygon) -> Vec<(f64, f64)> {
    region
        .contour
        .points
        .iter()
        .map(|p| (unscaled(p.x), unscaled(p.y)))
        .collect()
}

/// The tower as a whole while it is laid out (`WipeTower`).
struct Generator<'a> {
    settings: &'a TowerSettings,
    plan: Vec<Plan>,
    /// The line width, the tower's width and depth (`m_perimeter_width`,
    /// `m_wipe_tower_width`, `m_wipe_tower_depth`).
    line: f32,
    width: f32,
    depth: f32,
    extra_spacing: f32,
    ribs: Option<Ribs>,
    /// The tower's height, as the ribs and the least depth see it.
    height: f32,
    brim_width: f32,
    /// Where the tower's frame is on the bed (`m_wipe_tower_pos` and
    /// `m_rib_offset`).
    origin: (f32, f32),
    shift: f32,
    left_to_right: bool,
    tool: usize,
    nozzle: f64,
    first_feed: f32,
    max_feed: f32,
}

/// The rib wall round `area`, a box in the tower's frame on the layer
/// whose top is `z` (`generate_rib_polygon`, `rib_wall_polygon`).
fn rib_wall(
    ribs: &Ribs,
    area: &Box2,
    (width, depth): (f32, f32),
    shift: f32,
    z: f32,
    height: f32,
) -> Result<Vec<(f64, f64)>, ClipError> {
    let diagonal = (width * width + depth * depth).sqrt();
    let over = ((ribs.length - diagonal).max(0.0) / 2.0) * ((height - z).abs() / height);
    let (over, half) = (f64::from(over), f64::from(ribs.width) / 2.0);
    let (w, d, shift) = (f64::from(width), f64::from(depth), f64::from(shift));
    let length = w.hypot(d);
    let mut shapes = Vec::with_capacity(3);
    for (from, to) in [((0.0, 0.0), (w, d)), ((w, 0.0), (0.0, d))] {
        let along = ((to.0 - from.0) / length, (to.1 - from.1) / length);
        let (a, b) = (
            (from.0 - along.0 * over, from.1 - along.1 * over - shift),
            (to.0 + along.0 * over, to.1 + along.1 * over - shift),
        );
        let across = (-along.1 * half, along.0 * half);
        shapes.push(crate::region(&[
            (a.0 + across.0, a.1 + across.1),
            (a.0 - across.0, a.1 - across.1),
            (b.0 - across.0, b.1 - across.1),
            (b.0 + across.0, b.1 + across.1),
        ]));
    }
    let plain = [
        (f64::from(area.left), f64::from(area.bottom)),
        (f64::from(area.right), f64::from(area.bottom)),
        (f64::from(area.right), f64::from(area.top)),
        (f64::from(area.left), f64::from(area.top)),
    ];
    shapes.push(crate::region(&plain));
    let ribbed = crate::outline_of(&shapes)?;
    if !ribs.fillet {
        return Ok(ribbed);
    }
    crate::outline_of(&[
        crate::region(&crate::rounded(&ribbed)),
        crate::region(&plain),
    ])
}

impl Generator<'_> {
    /// A tower line on a layer `height` thick, carrying `share` of its
    /// usual plastic.
    fn flow(&self, height: f32, share: f64) -> Result<Flow, TowerError> {
        let (line, height) = (f64::from(self.line), f64::from(height));
        let round = height * (1.0 - std::f64::consts::PI / 4.0);
        let width = if share == 1.0 {
            line
        } else {
            share * (line - round) + round
        };
        Ok(Flow::new(width, height, self.nozzle)?)
    }

    /// How fast the wall and frame are laid by `tool` on a layer `height`
    /// thick, in mm/min (`wall_feedrate`).
    fn wall_feed(&self, tool: usize, first: bool, height: f32) -> f32 {
        if first {
            return (self.first_feed).min(self.max_feed);
        }
        let most = self
            .settings
            .type1
            .max_volumetric
            .get(tool)
            .copied()
            .unwrap_or(0.0) as f32;
        let area = height * (self.line - height * (1.0 - PI / 4.0));
        if most <= 0.0 {
            self.max_feed
        } else {
            (60.0 * most / area).min(self.max_feed)
        }
    }

    /// `align_perimeter`: a box's front and back put on the rows.
    fn align(&self, area: Box2) -> Box2 {
        let spacing = self.extra_spacing * self.line;
        let up = (align_ceil(area.top - self.line, spacing) + self.line).min(self.depth);
        let down = (align_floor(area.bottom - self.line, spacing) + self.line).max(-self.shift);
        Box2 {
            top: up,
            bottom: down,
            ..area
        }
    }

    fn wall(&self, area: &Box2, z: f32) -> Result<Option<Vec<(f64, f64)>>, ClipError> {
        self.ribs
            .as_ref()
            .map(|ribs| {
                rib_wall(
                    ribs,
                    area,
                    (self.width, self.depth),
                    self.shift,
                    z,
                    self.height,
                )
            })
            .transpose()
    }

    /// The open part of the layer below, which a row over it bridges
    /// (`get_floating_area`).
    fn floating(&self, index: usize) -> Option<(f32, f32)> {
        if index < 2 {
            return None;
        }
        let below = &self.plan[index - 1];
        let fill = below.used() + self.line;
        if below.depth - fill <= 2.0 * self.line {
            return None;
        }
        Some((fill + self.line, below.depth - self.line))
    }

    /// Whether a line at `y` is laid as a bridge (`need_thick_bridge_flow`).
    fn bridged(&self, index: usize, y: f32) -> bool {
        let height = self.plan[index].height;
        let flow = |h: f32| h * (self.line - h * (1.0 - PI / 4.0));
        if flow(height) >= flow(BRIDGE_HEIGHT) {
            return false;
        }
        self.floating(index)
            .is_some_and(|(low, high)| y > low && y < high)
    }

    /// One tool change (`tool_change`), from where it starts: the wall
    /// first where `wall` says, then the block.
    fn change(
        &mut self,
        index: usize,
        new: usize,
        traversed: f32,
        wall: bool,
    ) -> Result<(Pen, f32), TowerError> {
        let layer = self.plan[index].clone();
        let first = index == 0;
        let planned = layer
            .changes
            .iter()
            .find(|change| change.new == new)
            .cloned()
            .expect("a planned change");
        let line = self.line;
        let cleaning = Box2::new(
            line,
            line,
            self.width - 2.0 * line,
            planned.depth + traversed - line,
        );
        let initial = (cleaning.left, cleaning.bottom + traversed);
        let mut pen = Pen::new(self.origin, self.shift, initial);
        let plain = self.flow(layer.height, 1.0)?;
        if wall {
            let area = Box2::new(0.0, 0.0, self.width, layer.depth + line);
            let corner = area.corners()[index % 4];
            pen.start = corner;
            pen.at = corner;
            let area = self.align(area);
            let feed = self.wall_feed(new, first, layer.height);
            match self.wall(&area, layer.z)? {
                Some(outline) => pen.polygon(
                    &outline,
                    feed,
                    plain,
                    false,
                    self.settings.type1.arc_fitting,
                ),
                // Laid at the wall's speed; the reference gives it none,
                // and it goes at whatever the change G-code left in force.
                None => pen.rectangle(&area, feed, plain),
            }
        }
        pen.travel((0.0, 0.0), None);
        pen.travel(initial, None);
        self.tool = new;
        self.rows(&mut pen, index, &cleaning, planned.length)?;
        pen.strokes.push(TowerStroke::Sync);
        Ok((pen, planned.depth))
    }

    /// A block's rows (`toolchange_Wipe`).
    fn rows(
        &mut self,
        pen: &mut Pen,
        index: usize,
        cleaning: &Box2,
        length: f32,
    ) -> Result<(), TowerError> {
        let layer = &self.plan[index];
        let first = index == 0;
        let fuller = self.flow(layer.height, if first { 1.15 } else { 1.0 })?;
        let plain = self.flow(layer.height, 1.0)?;
        let thick = self.flow(BRIDGE_HEIGHT, 1.0)?;
        let (xl, xr) = (cleaning.left, cleaning.right);
        let mut owed = length;
        let dy = layer.spacing * self.line;
        let target = self.settings.wipe_target(first);
        let mut feed = 0.33 * target;
        self.left_to_right = true;
        let mut bridging = false;
        let mut flow = fuller;
        let last_tool = layer.changes.last().map(|change| change.new);
        for row in 0.. {
            if row != 0 {
                feed = next_feed(feed, target);
            }
            if bridging || self.bridged(index, pen.at.1) {
                bridging = true;
                flow = thick;
            }
            let x = if self.left_to_right { xr } else { xl };
            pen.lay((x, pen.at.1), Some(feed), flow);
            if bridging {
                flow = plain;
            }
            if pen.at.1 - EPSILON > cleaning.top {
                break;
            }
            owed -= xr - xl;
            if owed < WT_EPSILON {
                break;
            }
            pen.lay((pen.at.0, pen.at.1 + dy), None, flow);
            self.left_to_right = !self.left_to_right;
        }
        let other = if self.left_to_right { 0.0 } else { self.width };
        pen.wipe = vec![pen.bed(pen.at), pen.bed((other, pen.at.1))];
        if last_tool != Some(self.tool) {
            self.left_to_right = !self.left_to_right;
        }
        Ok(())
    }

    /// The rest of a layer: its frame, the lines over the depth no change
    /// uses, the wall where `wall` says and the brim (`finish_layer`).
    fn finish(&mut self, index: usize, wall: bool) -> Result<Pen, TowerError> {
        let layer = self.plan[index].clone();
        let first = index == 0;
        let line = self.line;
        let plain = self.flow(layer.height, 1.0)?;
        let feed = self.wall_feed(self.tool, first, layer.height);
        let used = layer.used();
        let fill_y = used + line;
        let fill = Box2::new(line, fill_y, self.width - 2.0 * line, layer.depth - fill_y);
        let start = if self.left_to_right {
            (fill.right, fill.top)
        } else {
            (fill.left, fill.top)
        };
        let mut pen = Pen::new(self.origin, self.shift, start);
        // The frame round the open part, its front laid as a bridge over
        // the layer below where that is open.
        if fill.top - fill.bottom > line - WT_EPSILON {
            let bridge = if self.bridged(index, fill.bottom) {
                Some(self.flow(BRIDGE_HEIGHT, 1.0)?)
            } else {
                None
            };
            let corners = fill.corners();
            let mut nearest = 0;
            if pen.at.0 - fill.left > fill.right - pen.at.0 {
                nearest = 1;
            }
            if pen.at.1 - fill.bottom > fill.top - pen.at.1 {
                nearest = if nearest == 0 { 3 } else { 2 };
            }
            pen.travel((corners[nearest].0, pen.at.1), None);
            pen.travel((pen.at.0, corners[nearest].1), None);
            for step in 1..=4 {
                let corner = (nearest + step) % 4;
                let flow = match (corner, bridge) {
                    (1, Some(bridge)) => bridge,
                    _ => plain,
                };
                pen.lay(corners[corner], Some(feed), flow);
            }
        }
        // Lines to carry the next layer.
        let dy = fill.top - fill.bottom - line;
        let (mut left, mut right) = (fill.left + 2.0 * line, fill.right - 2.0 * line);
        // Where the lines end and the far end of the last of them.
        let mut target: Option<((f32, f32), (f32, f32))> = None;
        if dy > line {
            pen.travel((fill.left + 2.0 * line, fill.bottom), None);
            let soluble = |filament: usize| {
                self.settings
                    .type1
                    .soluble
                    .get(filament)
                    .copied()
                    .unwrap_or(false)
            };
            let solid = first
                || self.plan.get(index + 1).is_some_and(|above| {
                    above.changes.iter().any(|change| soluble(change.new))
                        || self.old_tools(index + 1).iter().any(|old| soluble(*old))
                });
            if solid {
                let mut sparse = 1.5;
                if first {
                    left -= line;
                    right += line;
                    sparse = 1.0;
                }
                let mut y = fill.bottom + line;
                let count = (dy / (line * sparse)) as i32;
                let spacing = (dy - line) / (count - 1) as f32;
                for row in 0..count {
                    pen.lay((pen.at.0, y), Some(feed), plain);
                    pen.lay((if row % 2 == 1 { left } else { right }, y), None, plain);
                    y += spacing;
                }
                pen.lay((pen.at.0, fill.top), None, plain);
            } else {
                pen.lay((fill.left + 2.0 * line, fill.top), Some(feed), plain);
                let count = 1 + ((right - left) / BRIDGING) as i32;
                let dx = (right - left) / count as f32;
                for column in 1..=count {
                    let x = left + dx * column as f32;
                    pen.travel((x, pen.at.1), None);
                    let to = if column % 2 == 1 {
                        fill.bottom
                    } else {
                        fill.top
                    };
                    pen.lay((x, to), None, plain);
                }
                target = Some((
                    pen.at,
                    (
                        left + dx * count as f32,
                        if count % 2 == 1 {
                            fill.top
                        } else {
                            fill.bottom
                        },
                    ),
                ));
            }
        }
        // The wall, and the brim round it.
        let mut area = self.align(Box2::new(0.0, 0.0, self.width, layer.depth + line));
        let mut outline = self.wall(&area, layer.z)?;
        if wall {
            match &outline {
                Some(outline) => {
                    pen.polygon(outline, feed, plain, false, self.settings.type1.arc_fitting)
                }
                None => pen.rectangle(&area, feed, plain),
            }
        }
        let spacing = line - layer.height * (1.0 - PI / 4.0);
        let mut loops = ((self.brim_width + spacing / 2.0) / spacing) as i32;
        if !first {
            loops = if layer.depth != self.plan[0].depth {
                0
            } else {
                loops.min((MAX_CHAMFER / spacing) as i32) - index as i32
            };
        }
        let mut walled = wall;
        for _ in 0..loops.max(0) {
            walled = true;
            match &mut outline {
                Some(ring) => {
                    let grown = edge_clip::offset(
                        &[crate::region(ring)],
                        scaled(f64::from(spacing)),
                        Join::MITER,
                    )?;
                    let Some(next) = grown.first() else { break };
                    *ring = to_mm(next);
                    pen.polygon(ring, feed, plain, true, self.settings.type1.arc_fitting);
                }
                None => {
                    area = Box2 {
                        left: area.left - spacing,
                        bottom: area.bottom - spacing,
                        right: area.right + spacing,
                        top: area.top + spacing,
                    };
                    pen.rectangle(&area, feed, plain);
                }
            }
        }
        // Where the nozzle wipes on its way back to the part.
        match (&outline, walled) {
            (Some(ring), true) => {
                let reach = self
                    .settings
                    .type1
                    .wipe_distance
                    .get(self.tool)
                    .copied()
                    .unwrap_or(2.0);
                pen.wipe = wipe_along(&pen, ring, reach);
            }
            _ => {
                let corners = area.corners();
                let mut next = match corners.iter().position(|corner| *corner == pen.at) {
                    Some(at) => corners[(at + 1) % 4],
                    None => corners[0],
                };
                if let Some((end, far)) = target
                    && end == pen.at
                {
                    next = far;
                }
                pen.wipe = vec![pen.bed(pen.at), pen.bed(next)];
            }
        }
        Ok(pen)
    }

    /// The tools put away on a layer's changes.
    fn old_tools(&self, index: usize) -> Vec<usize> {
        let mut tool = self.tool_before(index);
        let mut old = Vec::new();
        for change in &self.plan[index].changes {
            old.push(tool);
            tool = change.new;
        }
        old
    }

    /// The tool in hand as a layer starts.
    fn tool_before(&self, index: usize) -> usize {
        (0..index)
            .rev()
            .find_map(|below| self.plan[below].changes.last().map(|change| change.new))
            .unwrap_or(self.tool)
    }
}

/// The wipe back along a ribbed wall just laid: from its corner nearest
/// the nozzle round the way it came, `reach` mm (`add_wipe_path`).
fn wipe_along(pen: &Pen, ring: &[(f64, f64)], reach: f64) -> Vec<Point2> {
    if ring.is_empty() {
        return Vec::new();
    }
    let at = (f64::from(pen.at.0), f64::from(pen.at.1));
    let nearest = (0..ring.len())
        .min_by(|a, b| {
            let d = |i: usize| (ring[i].0 - at.0).powi(2) + (ring[i].1 - at.1).powi(2);
            d(*a).total_cmp(&d(*b))
        })
        .unwrap_or(0);
    // The ring from that corner round to it again, turned about.
    let mut path: Vec<(f64, f64)> = (0..=ring.len())
        .map(|step| ring[(nearest + step) % ring.len()])
        .collect();
    path.reverse();
    let mut left = reach;
    let mut out = Vec::new();
    for (index, point) in path.iter().enumerate() {
        if left < 1e-4 {
            break;
        }
        out.push(pen.bed((point.0 as f32, point.1 as f32)));
        if index != 0 {
            let before = path[index - 1];
            left -= (point.0 - before.0).hypot(point.1 - before.1);
        }
    }
    out
}

/// Plans the Type1 tower (WipeTower.cpp `plan_toolchange`, `plan_tower`,
/// `generate`) for a print whose layers are `heights` mm thick.
pub(crate) fn plan(
    changes: &[Vec<Change>],
    heights: &[f64],
    settings: &TowerSettings,
) -> Result<Tower, TowerError> {
    let count = changes.len();
    let mut tower = Tower {
        layers: vec![TowerLayer::default(); count],
        footprint: None,
    };
    let Some(last) = changes.iter().rposition(|changes| !changes.is_empty()) else {
        return Ok(tower);
    };
    let t1 = &settings.type1;
    let nozzle = settings.nozzle_diameter as f32;
    let line = if t1.line_width > 0.0 {
        t1.line_width as f32
    } else {
        nozzle * WIDTH_TO_NOZZLE
    };
    let mut width = settings.width as f32;
    // Each layer's top, as the reference's layers have it.
    let mut tops = Vec::with_capacity(count);
    let mut top = 0.0f64;
    for height in heights.iter().take(count) {
        top += height;
        tops.push(top as f32);
    }
    let print_height = tops.last().copied().unwrap_or(0.0);
    // `plan_toolchange`: each change a block of whole rows of
    // `prime_volume`.
    let across = width - 2.0 * line;
    let volume = settings.prime_volume as f32;
    let mut plan: Vec<Plan> = (0..=last)
        .map(|layer| {
            let height = heights[layer] as f32;
            Plan {
                z: tops[layer],
                height,
                depth: 0.0,
                spacing: 1.0,
                changes: if across <= EPSILON {
                    Vec::new()
                } else {
                    changes[layer]
                        .iter()
                        .map(|change| {
                            let length = volume_to_length(volume, line, height);
                            Planned {
                                new: change.filament,
                                depth: (length / across).ceil() * line,
                                volume,
                                length,
                            }
                        })
                        .collect()
                },
            }
        })
        .collect();
    let mut ribs = match settings.wall {
        Wall::Ribs {
            width,
            extra,
            rounded,
        } => Some(Ribs {
            width: width as f32,
            length: 0.0,
            extra: extra as f32,
            fillet: rounded,
        }),
        Wall::Box => None,
    };
    // `make_rib_tower_square`: a ribbed tower as wide as it is deep, with
    // the purge area it had at the width set.
    if ribs.is_some() {
        let most = plan.iter().map(Plan::used).fold(0.0f32, f32::max);
        if most >= EPSILON {
            let square = align_ceil(((most + line) * width).sqrt(), line);
            if square - 2.0 * line > EPSILON {
                width = square;
                let across = width - 2.0 * line;
                for layer in &mut plan {
                    for change in &mut layer.changes {
                        change.depth =
                            (volume_to_length(change.volume, line, layer.height) / across).ceil()
                                * line;
                    }
                }
            }
        }
    }
    // The ribs taper to the tower's top; a plain tower's least depth goes
    // by the whole print's height (`generate`, the constructor).
    let tower_height = if ribs.is_some() {
        plan.last().map_or(0.0, |layer| layer.z)
    } else {
        print_height
    };
    // `plan_tower`.
    let most = plan.iter().map(Plan::used).fold(0.0f32, f32::max);
    let least = limit_depth(tower_height);
    let mut extra_spacing = 1.0f32;
    if most + EPSILON < least {
        match &mut ribs {
            Some(ribs) => ribs.length = ribs.length.max(least * std::f32::consts::SQRT_2),
            None => extra_spacing = least / most,
        }
    }
    let across = width - 2.0 * line;
    for (index, layer) in plan.iter_mut().enumerate() {
        if index == 0 && extra_spacing > 1.0 + EPSILON {
            // The first layer stays solid: its blocks lay more instead.
            layer.spacing = 1.0;
            for change in &mut layer.changes {
                let length = volume_to_length(change.volume, line, layer.height);
                let spread = ((length * extra_spacing / across).floor() * across).max(length);
                let rows = ((spread - WT_EPSILON) / across).ceil();
                change.depth = rows * line;
                change.volume *= spread / length;
                change.length = spread;
            }
        } else {
            layer.spacing = extra_spacing;
            for change in &mut layer.changes {
                change.depth *= extra_spacing;
                change.length = volume_to_length(change.volume, line, layer.height);
            }
        }
    }
    // Each layer as deep as it and every layer above it need.
    let mut depth = 0.0f32;
    let mut most = 0.0f32;
    for layer in plan.iter_mut().rev() {
        most = most.max(layer.used());
        layer.depth = most;
        if most > depth - line {
            depth = most + line;
        }
    }
    if let Some(ribs) = &mut ribs {
        let diagonal = (depth * depth + width * width).sqrt();
        ribs.length = ribs.length.max(diagonal) + ribs.extra;
        ribs.length = ribs.length.max(diagonal);
        ribs.width = ribs.width.min(depth.min(width) / 2.0);
    }
    let mut brim_width = t1.brim_width as f32;
    if brim_width < 0.0 {
        // "Auto", by the whole print's height (the constructor).
        brim_width = auto_brim(print_height);
    }
    let first_tool = changes[..=last]
        .iter()
        .find_map(|layer| layer.first().and_then(|change| change.old))
        .unwrap_or(0);
    let mut generator = Generator {
        settings,
        plan,
        line,
        width,
        depth,
        extra_spacing,
        ribs,
        height: tower_height,
        brim_width,
        origin: (settings.x as f32, settings.y as f32),
        shift: 0.0,
        left_to_right: true,
        tool: first_tool,
        nozzle: settings.nozzle_diameter,
        first_feed: settings.first_layer_speed as f32 * 60.0,
        max_feed: t1.max_speed as f32 * 60.0,
    };
    // `set_rib_offset`: the tower stands where the box round its first
    // layer's wall has its corner on the place set.
    let mut offset = (0.0f32, 0.0f32);
    if let Some(first) = generator.plan.iter().position(|layer| layer.depth >= line)
        && generator.ribs.is_some()
    {
        let layer = &generator.plan[first];
        if layer.depth < depth - line {
            generator.shift = align_round((depth - layer.depth) / 2.0, extra_spacing * line);
        }
        let area = generator.align(Box2::new(0.0, 0.0, width, layer.depth + line));
        if let Some(outline) = generator.wall(&area, layer.z)? {
            let shift = f64::from(generator.shift);
            let low = outline.iter().fold((f64::MAX, f64::MAX), |(x, y), p| {
                (x.min(p.0), y.min(p.1 + shift))
            });
            offset = (-low.0 as f32, -low.1 as f32);
        }
        generator.shift = 0.0;
    }
    generator.origin = (settings.x as f32 + offset.0, settings.y as f32 + offset.1);

    for index in 0..=last {
        let layer = generator.plan[index].clone();
        if layer.depth < line {
            continue;
        }
        if layer.depth < depth - line {
            generator.shift =
                align_round((depth - layer.depth) / 2.0, generator.extra_spacing * line);
        }
        let soluble = &settings.type1.soluble;
        let support = &settings.type1.support;
        let lays_finish = layer.changes.iter().position(|change| {
            !soluble.get(change.new).copied().unwrap_or(false)
                && !support.get(change.new).copied().unwrap_or(false)
        });
        let out = &mut tower.layers[index];
        if layer.changes.is_empty() {
            let pen = generator.finish(index, true)?;
            let (wipe, lines, _) = pen.finish();
            out.finish = lines;
            out.sparse = Some(wipe);
            continue;
        }
        // No change on the layer is to a filament that lays the frame:
        // the frame and wall come before the first, from the nozzle in
        // hand, there; here they follow its block.
        let mut early = match lays_finish {
            None => Some(generator.finish(index, true)?),
            Some(_) => None,
        };
        let mut traversed = 0.0f32;
        for (at, change) in layer.changes.iter().enumerate() {
            let walled = lays_finish == Some(at);
            let (mut pen, deep) = generator.change(index, change.new, traversed, walled)?;
            traversed += deep;
            let tail = if walled {
                Some(generator.finish(index, false)?)
            } else if at == 0 {
                early.take()
            } else {
                None
            };
            if let Some(tail) = tail {
                let (finish, lines, _) = tail.finish();
                if pen.bed(pen.at) != finish.start {
                    let start = finish.start;
                    pen.strokes
                        .push(TowerStroke::Move(start, Some(f64::from(MERGE_FEED) / 60.0)));
                }
                pen.strokes.extend(finish.strokes);
                pen.lines.extend(lines);
                if !finish.wipe_path.is_empty() {
                    pen.wipe = finish.wipe_path;
                }
            }
            let (wipe, lines, _) = pen.finish();
            out.changes.push(crate::Block {
                ram: None,
                enter: Vec::new(),
                lines,
                wipe: Some(wipe),
            });
        }
    }
    // The ground it takes: its first layer, out to the edge of its lines.
    let half = f64::from(line) / 2.0;
    let first = &tower.layers[0];
    tower.footprint = first
        .finish
        .iter()
        .chain(first.changes.iter().flat_map(|block| &block.lines))
        .flat_map(|path| &path.points)
        .map(|p| (unscaled(p.x), unscaled(p.y)))
        .fold(None, |ground: Option<[f64; 4]>, (x, y)| {
            Some(match ground {
                None => [x, y, x, y],
                Some([a, b, c, d]) => [a.min(x), b.min(y), c.max(x), d.max(y)],
            })
        })
        .map(|[a, b, c, d]| [a - half, b - half, c + half, d + half]);
    Ok(tower)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(wall: Wall) -> TowerSettings {
        TowerSettings {
            enabled: true,
            bambu_tower: true,
            x: 15.0,
            y: 220.0,
            width: 35.0,
            prime_volume: 45.0,
            brim_width: 3.0,
            first_layer_speed: 50.0,
            max_purge_speed: 90.0,
            wall,
            type1: Type1 {
                brim_width: 3.0,
                max_speed: 90.0,
                max_volumetric: vec![21.0; 4],
                ..Type1::default()
            },
            ..TowerSettings::default()
        }
    }

    fn ribs() -> Wall {
        Wall::Ribs {
            width: 8.0,
            extra: 0.0,
            rounded: true,
        }
    }

    /// So many changes a layer, each to the next filament in turn.
    fn changes(counts: &[usize]) -> Vec<Vec<Change>> {
        let mut tool = 0;
        counts
            .iter()
            .map(|count| {
                (0..*count)
                    .map(|_| {
                        let old = tool;
                        tool = (tool + 1) % 3;
                        Change {
                            ram: None,
                            volume: 45.0,
                            filament: tool,
                            old: Some(old),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn lays(wipe: &Wipe) -> Vec<(f64, f64, f64, Flow)> {
        wipe.strokes
            .iter()
            .filter_map(|stroke| match stroke {
                TowerStroke::LayAs(to, speed, flow) => Some((
                    unscaled(to.x),
                    unscaled(to.y),
                    (*speed * 60.0).round(),
                    *flow,
                )),
                _ => None,
            })
            .collect()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 2e-3
    }

    /// A block's rows, up to where its change ends: each row as its far
    /// end, its speed and its flow, the wall left out.
    fn rows(wipe: &Wipe) -> Vec<(f64, f64, f64, Flow)> {
        let mut at = (unscaled(wipe.start.x), unscaled(wipe.start.y));
        let mut rows = Vec::new();
        let mut walled = true;
        for stroke in &wipe.strokes {
            match stroke {
                TowerStroke::Sync => break,
                TowerStroke::Glide(to) | TowerStroke::Move(to, _) => {
                    at = (unscaled(to.x), unscaled(to.y));
                    walled = false;
                }
                TowerStroke::LayAs(to, speed, flow) | TowerStroke::Bend(to, speed, flow) => {
                    let to = (unscaled(to.x), unscaled(to.y));
                    if !walled && (to.0 - at.0).abs() > 1.0 && near(to.1, at.1) {
                        rows.push((to.0, to.1, (*speed * 60.0).round(), *flow));
                    }
                    at = to;
                }
                _ => {}
            }
        }
        rows
    }

    #[test]
    fn a_block_lays_the_prime_volume_in_rows_at_rising_speeds() {
        // Two changes a layer, as the figurine has them, above a first
        // layer of none.
        let mut counts = vec![0];
        counts.extend([2; 30]);
        let tower = plan(&changes(&counts), &[0.2; 31], &settings(ribs())).unwrap();
        let block = &tower.layers[5].changes[0];
        // The rows: from the wall's inside on the left to its inside on
        // the right, half a millimetre apart, 22 of them for 45 mm³ on a
        // tower squared to 23.5 mm (WipeTower.cpp `make_rib_tower_square`).
        let across = rows(block.wipe.as_ref().unwrap());
        assert_eq!(across.len(), 22);
        assert!(
            near(across[0].0, 40.011) && near(across[1].0, 17.511),
            "{across:?}"
        );
        assert!(near(across[1].1 - across[0].1, 0.5));
        let speeds: Vec<f64> = across.iter().map(|row| row.2).take(6).collect();
        assert_eq!(speeds, [1584.0, 1800.0, 2198.0, 4200.0, 4250.0, 4300.0]);
        assert!(across.iter().all(|row| row.2 <= 4800.0));
        let laid = 22.5 * across.len() as f64 * across[0].3.mm3_per_mm();
        assert!((45.0..46.0).contains(&laid), "{laid}");
        // The second block starts a block, 11 mm, further in.
        let second = rows(tower.layers[5].changes[1].wipe.as_ref().unwrap());
        assert!(near(second[0].1 - across[0].1, 11.0), "{second:?}");
    }

    #[test]
    fn a_ribbed_tower_stands_with_its_first_wall_on_its_place() {
        let mut settings = settings(ribs());
        settings.type1.brim_width = 0.0;
        let mut counts = vec![0];
        counts.extend([2; 30]);
        let tower = plan(&changes(&counts), &[0.2; 31], &settings).unwrap();
        // Its wall's outline has its corner on the place set; the wall as
        // laid, thinned, comes within a few hundredths of it.
        let [x, y, ..] = tower.footprint.unwrap();
        assert!(
            (x - 14.75).abs() < 0.05 && (y - 219.75).abs() < 0.05,
            "{x} {y}"
        );
        // The wall goes round from another corner each layer: a layer's
        // first change starts where it lays it from.
        let corners = [
            (17.011, 222.065),
            (40.511, 222.065),
            (40.511, 244.565),
            (17.011, 244.565),
        ];
        for (layer, corner) in (8..12).zip(corners) {
            let start = tower.layers[layer].changes[0].wipe.as_ref().unwrap().start;
            let start = (unscaled(start.x), unscaled(start.y));
            assert!(
                near(start.0, corner.0) && near(start.1, corner.1),
                "{layer} {start:?}"
            );
        }
    }

    #[test]
    fn the_first_layer_is_solid_and_its_rows_carry_more() {
        let tower = plan(&changes(&[1, 1, 1]), &[0.2; 3], &settings(Wall::Box)).unwrap();
        let first = rows(tower.layers[0].changes[0].wipe.as_ref().unwrap());
        let plain = Flow::new(0.5, 0.2, 0.4).unwrap();
        assert!((first[0].3.mm3_per_mm() / plain.mm3_per_mm() - 1.15).abs() < 1e-6);
        // The first layer's rows go from a third of its own speed, the
        // first layer's 50 mm/s.
        assert_eq!(first[0].2, 990.0);
        // A tower this low and this shallow is spread to the 20 mm it
        // needs to stand, above its first layer: there the rows are
        // further apart, and on the first they lay more.
        let second = rows(tower.layers[1].changes[0].wipe.as_ref().unwrap());
        assert!(near(first[1].1 - first[0].1, 0.5));
        assert!(second[1].1 - second[0].1 > 0.5 + 1e-2, "{second:?}");
        assert!(first.len() > second.len());
    }

    #[test]
    fn a_row_over_the_open_part_below_is_laid_thick() {
        // Thin layers; layer 3 has a change, layer 2 none: its rows past
        // layer 2's frame are over the open part.
        let counts = [1, 1, 0, 2, 2, 2];
        let tower = plan(&changes(&counts), &[0.1; 6], &settings(Wall::Box)).unwrap();
        let thick = lays(tower.layers[4].changes[0].wipe.as_ref().unwrap())
            .iter()
            .any(|row| (row.3.height - 0.2).abs() < 1e-6);
        let below = tower.layers[2].sparse.is_some();
        assert!(below);
        let first = lays(tower.layers[3].changes[0].wipe.as_ref().unwrap())
            .iter()
            .any(|row| (row.3.height - 0.2).abs() < 1e-6);
        assert!(first || thick);
    }

    #[test]
    fn a_layer_with_no_change_is_laid_whole_by_the_nozzle_in_hand() {
        let tower = plan(&changes(&[0, 0, 1]), &[0.2; 3], &settings(Wall::Box)).unwrap();
        for layer in 0..2 {
            let layer = &tower.layers[layer];
            assert!(layer.changes.is_empty());
            let wipe = layer.sparse.as_ref().unwrap();
            assert!(!layer.finish.is_empty());
            // It ends with somewhere to wipe to.
            assert_eq!(wipe.wipe_path.len(), 2);
        }
    }
}
