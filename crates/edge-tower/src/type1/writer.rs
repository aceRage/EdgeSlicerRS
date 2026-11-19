//! The Type 1 tower's G-code writer (WipeTower.cpp `WipeTowerWriter`):
//! moves in the tower's own frame, written as the reference writes them,
//! with the layer's y shift and the rib offset applied, before the
//! G-code generator moves them to the tower's place on the bed
//! (GCode.cpp `post_process_wipe_tower_moves`).
//!
//! Figures are single precision, as there.

use edge_gcode::arc::{Arc, Fitted, Kind, tolerance};
use edge_geom::{Point2, Polygon};

/// libslic3r's `EPSILON`.
const EPSILON: f32 = 1e-4;
/// The wall's resolution, and a brim loop's first thinning
/// (`WIPE_TOWER_RESOLUTION`, `WT_SIMPLIFY_TOLERANCE_SCALED`), in mm.
const RESOLUTION: f64 = 0.1;
const PRE_SIMPLIFY: f64 = 0.001;

pub(crate) type V2 = (f32, f32);

fn norm(v: V2) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

fn sub(a: V2, b: V2) -> V2 {
    (a.0 - b.0, a.1 - b.1)
}

/// `float_to_string_decimal_point(value, places)`: `%.*f`.
pub(crate) fn fixed(value: f64, places: usize) -> String {
    format!("{value:.places$}")
}

/// A point in mm as the clipping library holds it.
pub(crate) fn scaled(p: V2) -> Point2 {
    Point2::from_mm(f64::from(p.0), f64::from(p.1))
}

pub(crate) fn unscaled(p: Point2) -> V2 {
    (
        edge_geom::unscaled(p.x) as f32,
        edge_geom::unscaled(p.y) as f32,
    )
}

/// One stretch of a wall path: a line, or an arc (`WallSegment`).
#[derive(Clone, Copy, Debug)]
struct Segment {
    start: V2,
    end: V2,
    arc: Option<Arc>,
}

/// The segments of a path, fitted with arcs within the tower's
/// resolution, or thinned to it, or kept as they are
/// (`simplify_by_fitting_arc`, `simplify`).
fn segments_of(points: &[Point2], arcs: bool, thin: bool, out: &mut Vec<Segment>) {
    if points.len() < 2 {
        return;
    }
    let within = tolerance(RESOLUTION);
    let fitted = if arcs {
        Fitted::new(points, within)
    } else if thin {
        Fitted::straight(edge_gcode::arc::douglas_peucker(points, within))
    } else {
        Fitted::straight(points.to_vec())
    };
    for part in &fitted.parts {
        match part.kind {
            Kind::Line => {
                for j in part.start..part.end {
                    out.push(Segment {
                        start: unscaled(fitted.points[j]),
                        end: unscaled(fitted.points[j + 1]),
                        arc: None,
                    });
                }
            }
            Kind::Arc(arc) => out.push(Segment {
                start: unscaled(fitted.points[part.start]),
                end: unscaled(fitted.points[part.end]),
                arc: Some(arc),
            }),
        }
    }
}

/// What the tower lays in one visit, as the reference's writer writes it.
#[derive(Clone, Debug)]
pub(crate) struct Writer {
    pub gcode: String,
    pos: V2,
    start: V2,
    feed: f32,
    layer_height: f32,
    flow: f32,
    y_shift: f32,
    width: f32,
    depth: f32,
    origin: V2,
    arc_fitting: bool,
    tool: usize,
    /// Each filament's fastest feed of filament, in mm/s (`max_e_speed`).
    max_e_speed: Vec<f32>,
    /// What a ramming holds the feed of filament to instead, in mm/s; 0
    /// for the filament's own (`m_e_speed_limit`).
    e_speed_limit: f32,
    pub wipe_path: Vec<V2>,
}

impl Writer {
    /// `WipeTowerWriter(layer_height, line_width, ...)`, then the setters
    /// every visit makes (`set_extrusion_flow`, `set_initial_tool`,
    /// `set_origin_offset`, `set_arc_fitting`, `set_y_shift`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        layer_height: f32,
        line_width: f32,
        flow: f32,
        tool: usize,
        origin: V2,
        arc_fitting: bool,
        y_shift: f32,
        max_e_speed: Vec<f32>,
    ) -> Self {
        let mut writer = Self {
            gcode: String::new(),
            pos: (f32::MAX, f32::MAX),
            start: (0.0, 0.0),
            feed: 0.0,
            layer_height,
            flow,
            y_shift: 0.0,
            width: 0.0,
            depth: 0.0,
            origin,
            arc_fitting,
            tool,
            max_e_speed,
            e_speed_limit: 0.0,
            wipe_path: Vec::new(),
        };
        writer.gcode += &format!("; LAYER_HEIGHT: {}\n", fixed(f64::from(layer_height), 6));
        writer.gcode += "; FEATURE: Prime tower\n";
        writer.change_line_width(line_width);
        writer.set_y_shift(y_shift);
        writer
    }

    pub fn change_line_width(&mut self, width: f32) -> &mut Self {
        self.gcode += &format!("; LINE_WIDTH: {}\n", fixed(f64::from(width), 6));
        self
    }

    pub fn height_tag(&mut self, height: f32) -> &mut Self {
        self.gcode += &format!("; LAYER_HEIGHT: {}\n", fixed(f64::from(height), 6));
        self
    }

    fn set_y_shift(&mut self, shift: f32) {
        self.pos.1 -= shift - self.y_shift;
        self.y_shift = shift;
    }

    pub fn set_flow(&mut self, flow: f32) -> &mut Self {
        self.flow = flow;
        self
    }

    /// `set_e_speed_limit`.
    pub fn set_e_speed_limit(&mut self, limit: f32) -> &mut Self {
        self.e_speed_limit = limit;
        self
    }

    pub fn set_tool(&mut self, tool: usize) -> &mut Self {
        self.tool = tool;
        self
    }

    pub fn x(&self) -> f32 {
        self.pos.0
    }

    pub fn y(&self) -> f32 {
        self.pos.1
    }

    pub fn pos(&self) -> V2 {
        self.pos
    }

    /// `set_initial_position`, with the tower square to the bed.
    pub fn set_initial_position(&mut self, pos: V2, width: f32, depth: f32) -> &mut Self {
        self.width = width;
        self.depth = depth;
        self.start = self.rotate(pos);
        self.pos = pos;
        self
    }

    pub fn start_rotated(&self) -> V2 {
        self.start
    }

    pub fn pos_rotated(&self) -> V2 {
        self.rotate(self.pos)
    }

    /// `rotate` at an angle of 0: through the tower's middle and back, as
    /// there, then the layer's y shift and the rib offset.
    fn rotate(&self, p: V2) -> V2 {
        let x = p.0 - self.width / 2.0;
        let y = p.1 + (self.y_shift - self.depth / 2.0);
        let mut out = (
            (f64::from(x) * 1.0 - f64::from(y) * 0.0) as f32 + self.width / 2.0,
            (f64::from(x) * 0.0 + f64::from(y) * 1.0) as f32 + self.depth / 2.0,
        );
        if self.origin != (0.0, 0.0) {
            out = (out.0 + self.origin.0, out.1 + self.origin.1);
        }
        out
    }

    pub fn append(&mut self, text: &str) -> &mut Self {
        self.gcode += text;
        self
    }

    pub fn comment_with_value(&mut self, comment: &str, value: usize) -> &mut Self {
        self.gcode += &format!(";{comment}{value}\n");
        self
    }

    fn format_f(&mut self, f: f32) -> String {
        self.feed = f;
        format!(" F{}", (f + 0.5).floor() as i32)
    }

    pub fn feedrate(&mut self, f: f32) -> &mut Self {
        if f != self.feed {
            let word = self.format_f(f);
            self.gcode += &format!("G1{word}\n");
        }
        self
    }

    /// `extrude_explicit`.
    pub fn extrude_explicit(&mut self, to: V2, e: f32, f: f32, limit: bool) -> &mut Self {
        if to == self.pos && e == 0.0 && (f == 0.0 || f == self.feed) {
            return self;
        }
        let (dx, dy) = (to.0 - self.pos.0, to.1 - self.pos.1);
        let len = (dx * dx + dy * dy).sqrt();
        let from = self.pos_rotated();
        let rot = self.rotate(to);
        let mut line = String::from("G1");
        if (rot.0 - from.0).abs() > EPSILON {
            line += &format!(" X{}", fixed(f64::from(rot.0), 3));
        }
        if (rot.1 - from.1).abs() > EPSILON {
            line += &format!(" Y{}", fixed(f64::from(rot.1), 3));
        }
        if e != 0.0 {
            line += &format!(" E{}", fixed(f64::from(e), 4));
        }
        if f != 0.0 && f != self.feed {
            let mut f = f;
            if limit {
                let along = if len == 0.0 { e.abs() } else { len };
                let e_speed = e / (along / f * 60.0);
                let most = if self.e_speed_limit > 0.0 {
                    self.e_speed_limit
                } else {
                    self.max_e_speed.get(self.tool).copied().unwrap_or(f32::MAX)
                };
                f /= 1f32.max(e_speed / most);
            }
            line += &self.format_f(f);
        }
        self.pos = to;
        self.gcode += &line;
        self.gcode += "\n";
        self
    }

    pub fn extrude(&mut self, to: V2, f: f32) -> &mut Self {
        let (dx, dy) = (to.0 - self.pos.0, to.1 - self.pos.1);
        let e = (dx * dx + dy * dy).sqrt() * self.flow;
        self.extrude_explicit(to, e, f, true)
    }

    pub fn travel(&mut self, to: V2, f: f32) -> &mut Self {
        self.extrude_explicit(to, 0.0, f, true)
    }

    /// `extrude_arc`: one arc at the writer's flow.
    fn extrude_arc(&mut self, arc: &Arc, f: f32) -> &mut Self {
        let to = unscaled(arc.end);
        let len = (arc.length * 1e-6) as f32;
        let e = len * self.flow;
        if len < EPSILON && e == 0.0 && (f == 0.0 || f == self.feed) {
            return self;
        }
        let from = self.pos_rotated();
        let rot = self.rotate(to);
        let centre = self.rotate(unscaled(arc.center));
        let mut line = String::from(if arc.ccw { "G3" } else { "G2" });
        line += &format!(
            " X{} Y{} I{} J{}",
            fixed(f64::from(rot.0), 3),
            fixed(f64::from(rot.1), 3),
            fixed(f64::from(centre.0 - from.0), 3),
            fixed(f64::from(centre.1 - from.1), 3)
        );
        if e != 0.0 {
            line += &format!(" E{}", fixed(f64::from(e), 4));
        }
        if f != 0.0 && f != self.feed {
            let along = if len == 0.0 { e.abs() } else { len };
            let e_speed = e / (along / f * 60.0);
            let most = self.max_e_speed.get(self.tool).copied().unwrap_or(f32::MAX);
            let f = f / 1f32.max(e_speed / most);
            line += &self.format_f(f);
        }
        self.pos = to;
        self.gcode += &line;
        self.gcode += "\n";
        self
    }

    fn lay_segment(&mut self, segment: &Segment, f: f32) {
        match &segment.arc {
            Some(arc) => {
                self.extrude_arc(arc, f);
            }
            None => {
                self.extrude(segment.end, f);
            }
        }
    }

    /// `polygon`: a closed wall once round, from the segment that starts
    /// nearest the nozzle.
    pub fn polygon(&mut self, wall: &Polygon, f: f32, pre_simplify: bool) -> &mut Self {
        let mut points: Vec<Point2> = wall.points.clone();
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
        if pre_simplify {
            points = edge_gcode::arc::douglas_peucker(&points, tolerance(PRE_SIMPLIFY));
        }
        let mut segments = Vec::new();
        segments_of(&points, self.arc_fitting, true, &mut segments);
        if segments.is_empty() {
            return self;
        }
        let mut closest = 0;
        let mut least = f32::MAX;
        for (i, segment) in segments.iter().enumerate() {
            let d = sub(segment.start, self.pos);
            let distance = d.0 * d.0 + d.1 * d.1;
            if distance < least {
                least = distance;
                closest = i;
            }
        }
        self.travel(segments[closest].start, 0.0);
        let mut i = closest;
        loop {
            let segment = segments[i];
            self.lay_segment(&segment, f);
            i = (i + 1) % segments.len();
            if i == closest {
                break;
            }
        }
        self
    }

    /// `rectangle`: from the corner nearest the nozzle, once round.
    pub fn rectangle(&mut self, ld: V2, width: f32, height: f32, f: f32) -> &mut Self {
        let corners = [
            ld,
            (ld.0 + width, ld.1),
            (ld.0 + width, ld.1 + height),
            (ld.0, ld.1 + height),
        ];
        let mut closest = 0;
        if self.x() - ld.0 > ld.0 + width - self.x() {
            closest = 1;
        }
        if self.y() - ld.1 > ld.1 + height - self.y() {
            closest = if closest == 0 { 3 } else { 2 };
        }
        self.travel((corners[closest].0, self.y()), 0.0);
        self.travel((self.x(), corners[closest].1), 0.0);
        let mut i = closest;
        loop {
            i = (i + 1) % 4;
            self.extrude(corners[i], f);
            if i == closest {
                break;
            }
        }
        self
    }

    /// `rectangle_fill_box`: a rectangle whose right side, over the open
    /// part of the layer below, may be laid as thick as a 0.2 mm layer's
    /// (`bridge` gives that flow).
    pub fn rectangle_fill_box(
        &mut self,
        ld: V2,
        width: f32,
        height: f32,
        f: f32,
        bridge: Option<f32>,
    ) -> &mut Self {
        let own = self.flow;
        let corners = [
            ld,
            (ld.0 + width, ld.1),
            (ld.0 + width, ld.1 + height),
            (ld.0, ld.1 + height),
        ];
        let mut closest = 0;
        if self.x() - ld.0 > ld.0 + width - self.x() {
            closest = 1;
        }
        if self.y() - ld.1 > ld.1 + height - self.y() {
            closest = if closest == 0 { 3 } else { 2 };
        }
        self.travel((corners[closest].0, self.y()), 0.0);
        self.travel((self.x(), corners[closest].1), 0.0);
        let mut i = closest;
        let mut changed = false;
        loop {
            i = (i + 1) % 4;
            if let Some(bridge) = bridge {
                if i == 1 {
                    self.flow = bridge;
                    self.height_tag(0.2);
                    changed = true;
                } else if i == 2 && changed {
                    self.flow = own;
                    let height = self.layer_height;
                    self.height_tag(height);
                }
            }
            self.extrude(corners[i], f);
            if i == closest {
                break;
            }
        }
        self
    }

    /// `spiral_flat_ironing`: a square spiral of travels round `center`.
    pub fn spiral_flat_ironing(&mut self, center: V2, area: f32, step: f32, f: f32) -> &mut Self {
        if step <= 0.0 {
            return self;
        }
        let edge = area.sqrt();
        let mut high = (center.0 + step, center.1 + step);
        let mut low = (center.0 - step, center.1 - step);
        let mut n = (edge / step / 2.0).ceil() as i32;
        while n > 0 {
            n -= 1;
            self.travel((high.0, self.pos.1), f);
            self.travel((self.pos.0, high.1), f);
            self.travel((low.0, self.pos.1), f);
            self.travel((self.pos.0, low.1), f);
            high = (high.0 + step, high.1 + step);
            low = (low.0 - step, low.1 - step);
        }
        self
    }

    /// `gap_wall`: a wall cut open at its gaps, from the piece nearest the
    /// nozzle, pulling back over each gap. `arcs`: a ribbed wall, fitted
    /// or thinned; a box keeps its corners.
    pub fn gap_wall(
        &mut self,
        pieces: &[Vec<Point2>],
        f: f32,
        retract_length: f32,
        retract_speed: f32,
        arcs: bool,
    ) -> &mut Self {
        let mut segments = Vec::new();
        for piece in pieces {
            if piece.len() < 2 {
                continue;
            }
            segments_of(piece, self.arc_fitting && arcs, arcs, &mut segments);
        }
        if segments.is_empty() {
            return self;
        }
        let mut closest = 0;
        let mut least = f32::MAX;
        for (i, segment) in segments.iter().enumerate() {
            let d = sub(segment.start, self.pos);
            let distance = d.0 * d.0 + d.1 * d.1;
            if distance < least {
                least = distance;
                closest = i;
            }
        }
        self.travel(segments[closest].start, 0.0);
        let first = segments[closest];
        self.lay_segment(&first, f);
        let mut i = (closest + 1) % segments.len();
        while i != closest {
            let segment = segments[i];
            if norm(sub(segment.start, self.pos)) > EPSILON {
                if retract_length > 0.0 {
                    self.retract(retract_length, retract_speed);
                }
                self.travel(segment.start, 600.0);
                if retract_length > 0.0 {
                    self.retract(-retract_length, retract_speed);
                }
            }
            self.lay_segment(&segment, f);
            i = (i + 1) % segments.len();
        }
        self
    }

    /// `load`, and `retract` as its negative.
    pub fn load(&mut self, e: f32, f: f32) -> &mut Self {
        if e == 0.0 && (f == 0.0 || f == self.feed) {
            return self;
        }
        let mut line = String::from("G1");
        if e != 0.0 {
            line += &format!(" E{}", fixed(f64::from(e), 4));
        }
        if f != 0.0 && f != self.feed {
            line += &self.format_f(f);
        }
        self.gcode += &line;
        self.gcode += "\n";
        self
    }

    pub fn retract(&mut self, e: f32, f: f32) -> &mut Self {
        self.load(-e, f)
    }

    pub fn add_wipe_point(&mut self, p: V2) -> &mut Self {
        let rotated = self.rotate(p);
        self.wipe_path.push(rotated);
        self
    }

    /// `add_wipe_path`: back along the wall from its point nearest the
    /// nozzle, `distance` mm.
    pub fn add_wipe_path(&mut self, wall: &Polygon, distance: f64) -> &mut Self {
        if wall.points.is_empty() {
            return self;
        }
        let here = scaled(self.pos);
        let closest = (0..wall.points.len())
            .min_by(|a, b| {
                let d = |i: &usize| {
                    let p = wall.points[*i];
                    let (dx, dy) = ((p.x - here.x) as f64, (p.y - here.y) as f64);
                    dx * dx + dy * dy
                };
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0);
        // `split_at_index` then `reverse`: from the point back round.
        let count = wall.points.len();
        let mut path: Vec<Point2> = (0..=count)
            .map(|k| wall.points[(closest + k) % count])
            .collect();
        path.reverse();
        let mut left = distance;
        for i in 0..path.len() {
            if left < f64::from(EPSILON) {
                break;
            }
            let p = unscaled(path[i]);
            self.add_wipe_point(p);
            if i != 0 {
                let (a, b) = (path[i], path[i - 1]);
                left -= edge_geom::unscaled(a.x - b.x).hypot(edge_geom::unscaled(a.y - b.y));
            }
        }
        self
    }
}
