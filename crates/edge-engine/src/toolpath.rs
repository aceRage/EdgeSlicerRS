//! The moves of a finished G-code file, read back for looking at: every
//! line laid with what it is, how wide and how high, and every move
//! between lines, in the order the printer makes them.
//!
//! The reference builds its preview the same way: its G-code processor
//! reads the file it has just written (GCodeProcessor.cpp) and the viewer
//! draws the moves that come out (libvgcode). The comments read here are
//! the ones both writers leave for it: `;TYPE:`, `;WIDTH:`, `;HEIGHT:`,
//! `;LAYER_CHANGE`, `;Z:`, `;WIPE_START` and `;WIPE_END`.
//!
//! Nothing here plans a move or times it; `edge-time` does that.

use edge_extrusion::Role;
use edge_geom::unscaled;

use crate::Sliced;

/// What a line is part of, numbered and named as the reference's legend
/// has it (libvgcode `EGCodeExtrusionRole`, ExtrusionEntity.cpp
/// `ExtrusionEntity::role_to_string`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Feature {
    None = 0,
    InnerWall = 1,
    OuterWall = 2,
    OverhangWall = 3,
    SparseInfill = 4,
    SolidInfill = 5,
    TopSurface = 6,
    Ironing = 7,
    Bridge = 8,
    GapInfill = 9,
    Skirt = 10,
    Support = 11,
    SupportInterface = 12,
    PrimeTower = 13,
    Custom = 14,
    BottomSurface = 15,
    InternalBridge = 16,
    Brim = 17,
    SupportTransition = 18,
    Mixed = 19,
    WallOverSupport = 20,
    BottomSurfaceOverSupport = 21,
}

impl Feature {
    /// Every feature, in the order of its number.
    pub const ALL: [Feature; 22] = [
        Feature::None,
        Feature::InnerWall,
        Feature::OuterWall,
        Feature::OverhangWall,
        Feature::SparseInfill,
        Feature::SolidInfill,
        Feature::TopSurface,
        Feature::Ironing,
        Feature::Bridge,
        Feature::GapInfill,
        Feature::Skirt,
        Feature::Support,
        Feature::SupportInterface,
        Feature::PrimeTower,
        Feature::Custom,
        Feature::BottomSurface,
        Feature::InternalBridge,
        Feature::Brim,
        Feature::SupportTransition,
        Feature::Mixed,
        Feature::WallOverSupport,
        Feature::BottomSurfaceOverSupport,
    ];

    /// What the legend and a `;TYPE:` comment call it.
    pub fn name(self) -> &'static str {
        match self {
            Feature::None => "Undefined",
            Feature::InnerWall => "Inner wall",
            Feature::OuterWall => "Outer wall",
            Feature::OverhangWall => "Overhang wall",
            Feature::SparseInfill => "Sparse infill",
            Feature::SolidInfill => "Internal solid infill",
            Feature::TopSurface => "Top surface",
            Feature::Ironing => "Ironing",
            Feature::Bridge => "Bridge",
            Feature::GapInfill => "Gap infill",
            Feature::Skirt => "Skirt",
            Feature::Support => "Support",
            Feature::SupportInterface => "Support interface",
            Feature::PrimeTower => "Prime tower",
            Feature::Custom => "Custom",
            Feature::BottomSurface => "Bottom surface",
            Feature::InternalBridge => "Internal Bridge",
            Feature::Brim => "Brim",
            Feature::SupportTransition => "Support transition",
            Feature::Mixed => "Multiple",
            Feature::WallOverSupport => "Wall over support",
            Feature::BottomSurfaceOverSupport => "Bottom surface over support",
        }
    }

    pub fn from_name(name: &str) -> Option<Feature> {
        Feature::ALL
            .into_iter()
            .find(|feature| feature.name() == name)
    }
}

impl From<Role> for Feature {
    fn from(role: Role) -> Feature {
        match role {
            Role::ExternalPerimeter => Feature::OuterWall,
            Role::Perimeter => Feature::InnerWall,
            Role::OverhangPerimeter => Feature::OverhangWall,
            Role::SparseInfill => Feature::SparseInfill,
            Role::SolidInfill => Feature::SolidInfill,
            Role::TopSolidInfill => Feature::TopSurface,
            Role::Ironing => Feature::Ironing,
            Role::BottomSurface => Feature::BottomSurface,
            Role::BridgeInfill => Feature::Bridge,
            Role::InternalBridge => Feature::InternalBridge,
            Role::GapFill => Feature::GapInfill,
            Role::Skirt => Feature::Skirt,
            Role::Brim => Feature::Brim,
            Role::Support => Feature::Support,
            Role::SupportInterface => Feature::SupportInterface,
            Role::PrimeTower | Role::PrimeLine => Feature::PrimeTower,
        }
    }
}

/// What a move is. The first four go somewhere; the rest mark a place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MoveKind {
    /// Where the nozzle is, with nothing drawn to it: the first place a
    /// file names, and the place a line starts from after a mark that
    /// stands elsewhere.
    Start = 0,
    Travel = 1,
    Extrude = 2,
    /// A move with the filament drawn back, wiping the nozzle.
    Wipe = 3,
    Retract = 4,
    Unretract = 5,
    /// Where an outer wall's loop closes.
    Seam = 6,
    ToolChange = 7,
}

/// One move, or one mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub kind: MoveKind,
    pub feature: Feature,
    /// Where the nozzle is when the move ends, in mm. A line laid ends
    /// with the nozzle at the top of it.
    pub to: [f32; 3],
    /// Width and height of the line laid, in mm; 0 for a move that lays none.
    pub width: f32,
    pub height: f32,
    /// The speed asked for, in mm/s.
    pub feedrate: f32,
    /// The part cooling fan, in percent.
    pub fan: u8,
    /// The nozzle temperature last set, in °C.
    pub temperature: u16,
    /// The filament, by number from 1.
    pub filament: u8,
    /// The layer it belongs to, from 0.
    pub layer: u32,
    /// How far the path turns here, from this move into the next, in
    /// radians, anticlockwise positive; 0 where the next move is not of
    /// the same line.
    pub turn: f32,
}

/// One layer of a toolpath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolpathLayer {
    /// The first of its moves, as an index into [`Toolpath::moves`].
    pub first: usize,
    /// The top of the layer above the bed, and how thick it is, in mm.
    pub z: f32,
    pub height: f32,
}

/// A print as moves, in the order they are made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolpath {
    pub moves: Vec<Move>,
    /// Never empty when there are moves: the first layer starts at move 0
    /// and takes what the start G-code does.
    pub layers: Vec<ToolpathLayer>,
    /// For a toolpath read from G-code, the line of the file each move
    /// was read from, counting from 1: what the reference's G-code window
    /// shows for the move the slider is on (`gcode_id`). A mark is the
    /// line of the move that made it. Empty for one that was not read
    /// from a file.
    pub lines: Vec<u32>,
}

/// The value of a letter among a command's words: `X12.5` for `b'X'`.
fn word(words: &[&str], letter: u8) -> Option<f64> {
    words.iter().find_map(|word| {
        let (first, rest) = word.as_bytes().split_first()?;
        first
            .eq_ignore_ascii_case(&letter)
            .then(|| std::str::from_utf8(rest).ok()?.parse::<f64>().ok())
            .flatten()
            .filter(|value| value.is_finite())
    })
}

/// How finely an arc is cut into straight moves, in mm.
const ARC_STEP: f64 = 0.5;

struct Reader {
    out: Toolpath,
    at: [f64; 3],
    /// Whether the file has said where the nozzle is in x and y.
    placed: bool,
    absolute: bool,
    absolute_e: bool,
    e: f64,
    feedrate: f64,
    feature: Feature,
    width: f64,
    height: f64,
    wiping: bool,
    fan: u8,
    temperature: u16,
    filament: u8,
    /// Layer changes seen, and whether the last is still to start: a layer
    /// starts with its first line, as the reference counts them.
    changes: usize,
    pending: bool,
    layer_z: f64,
    layer_height: f64,
    /// Cross-section of the filament in mm².
    filament_area: f64,
    /// While an outer wall is being laid: where its loop began.
    seam_armed: bool,
    seam_first: Option<[f64; 3]>,
    /// The line being read, counting from 1.
    line: u32,
}

impl Reader {
    fn new() -> Self {
        Reader {
            out: Toolpath::default(),
            at: [0.0; 3],
            placed: false,
            absolute: true,
            absolute_e: true,
            e: 0.0,
            feedrate: 0.0,
            feature: Feature::Custom,
            width: 0.0,
            height: 0.0,
            wiping: false,
            fan: 0,
            temperature: 0,
            filament: 1,
            changes: 0,
            pending: false,
            layer_z: 0.0,
            layer_height: 0.0,
            filament_area: std::f64::consts::PI * 1.75 * 1.75 / 4.0,
            seam_armed: false,
            seam_first: None,
            line: 0,
        }
    }

    fn push(&mut self, kind: MoveKind, to: [f64; 3], width: f64, height: f64) {
        if self.out.layers.is_empty() {
            self.out.layers.push(ToolpathLayer {
                first: 0,
                z: self.layer_z as f32,
                height: self.layer_height as f32,
            });
        }
        self.out.moves.push(Move {
            kind,
            feature: match kind {
                MoveKind::Extrude => self.feature,
                _ => Feature::None,
            },
            to: to.map(|value| value as f32),
            width: width as f32,
            height: height as f32,
            feedrate: (self.feedrate / 60.0) as f32,
            fan: self.fan,
            temperature: self.temperature,
            filament: self.filament,
            layer: (self.out.layers.len() - 1) as u32,
            turn: 0.0,
        });
        self.out.lines.push(self.line);
    }

    fn comment(&mut self, text: &str) {
        if let Some(name) = text.strip_prefix("TYPE:") {
            // A name this reader does not know is the file's own: custom.
            self.feature = Feature::from_name(name.trim()).unwrap_or(Feature::Custom);
            if self.feature == Feature::OuterWall {
                self.seam_armed = true;
            }
        } else if let Some(value) = text.strip_prefix("WIDTH:") {
            self.width = value.trim().parse().unwrap_or(self.width);
        } else if let Some(value) = text.strip_prefix("HEIGHT:") {
            self.height = value.trim().parse().unwrap_or(self.height);
            if self.pending || self.changes <= 1 {
                self.layer_height = self.height;
                self.note_first_layer();
            }
        } else if let Some(value) = text.strip_prefix("Z:") {
            self.layer_z = value.trim().parse().unwrap_or(self.layer_z);
            self.note_first_layer();
        } else if text == "LAYER_CHANGE" {
            self.changes += 1;
            // The first change opens the layer the start G-code is already in.
            self.pending = self.changes > 1;
        } else if text == "WIPE_START" {
            self.wiping = true;
        } else if text == "WIPE_END" {
            self.wiping = false;
        } else if let Some(sizes) = text.trim_start().strip_prefix("filament_diameter:")
            && let Some(Ok(diameter)) = sizes
                .split(',')
                .next()
                .map(|size| size.trim().parse::<f64>())
            && diameter > 0.0
        {
            self.filament_area = std::f64::consts::PI * diameter * diameter / 4.0;
        }
    }

    /// The first layer is known by its tags as soon as they are read.
    fn note_first_layer(&mut self) {
        if self.changes <= 1
            && let Some(first) = self.out.layers.first_mut()
        {
            first.z = self.layer_z as f32;
            first.height = self.layer_height as f32;
        }
    }

    /// A straight move to `to`, feeding `e` mm of filament. `across`,
    /// the command says where in x or y it goes.
    fn line(&mut self, to: [f64; 3], e: f64, across: bool) {
        let from = self.at;
        let moved_xy = (to[0] - from[0]).abs() > 1e-9 || (to[1] - from[1]).abs() > 1e-9;
        let moved = moved_xy || (to[2] - from[2]).abs() > 1e-9;
        // The first place a file names is where the drawing starts from.
        if !self.placed {
            self.at = to;
            if across {
                self.placed = true;
                self.push(MoveKind::Start, to, 0.0, 0.0);
            }
            return;
        }
        // What the move is (GCodeProcessor.cpp `process_G1`).
        let kind = if self.wiping && moved {
            MoveKind::Wipe
        } else if e < 0.0 {
            if moved {
                MoveKind::Travel
            } else {
                MoveKind::Retract
            }
        } else if e > 0.0 {
            if moved_xy {
                MoveKind::Extrude
            } else if moved {
                MoveKind::Travel
            } else {
                MoveKind::Unretract
            }
        } else if moved {
            MoveKind::Travel
        } else {
            return;
        };

        // Where an outer wall's loop closes is its seam
        // (GCodeProcessor.cpp, the seams detector in `process_G1`).
        let walling = kind == MoveKind::Extrude
            && matches!(self.feature, Feature::OuterWall | Feature::OverhangWall);
        if kind == MoveKind::Extrude && self.feature == Feature::OuterWall {
            if self.seam_armed && self.seam_first.is_none_or(|first| from[2] > first[2]) {
                self.seam_first = Some(from);
            }
        } else if !walling && let Some(first) = self.seam_first.take() {
            self.seam_armed = false;
            let apart: f64 = (0..3).map(|axis| (from[axis] - first[axis]).powi(2)).sum();
            // A quarter of a millimetre, as the reference has it.
            if apart < 0.0625 {
                let middle = std::array::from_fn(|axis| (from[axis] + first[axis]) / 2.0);
                self.push(MoveKind::Seam, middle, self.width, self.height);
                // The next line starts from the nozzle, not from the mark.
                self.push(MoveKind::Start, from, 0.0, 0.0);
            }
        }

        let (width, height) = match kind {
            MoveKind::Extrude => {
                if self.pending {
                    self.pending = false;
                    self.out.layers.push(ToolpathLayer {
                        first: self.out.moves.len(),
                        z: self.layer_z as f32,
                        height: self.layer_height as f32,
                    });
                }
                if self.feature == Feature::Custom {
                    // Nothing says how wide a line of the file's own is:
                    // as wide as what it feeds comes to, at the height the
                    // nozzle is at (GCodeProcessor.cpp `process_G1`).
                    let length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
                    let height = if to[2] > 0.0 { to[2].min(2.0) } else { 0.2 };
                    let width = e * self.filament_area / (height * length)
                        + height * (1.0 - std::f64::consts::FRAC_PI_4);
                    (width.min(f64::max(2.0, 4.0 * height)), height)
                } else {
                    (self.width, self.height)
                }
            }
            MoveKind::Retract | MoveKind::Unretract => (self.width, self.height),
            _ => (0.0, 0.0),
        };
        self.push(kind, to, width, height);
        self.at = to;
    }

    fn command(&mut self, line: &str) {
        let mut words = line.split_ascii_whitespace();
        let Some(name) = words.next() else { return };
        let words: Vec<&str> = words.collect();
        let name = name.to_ascii_uppercase();
        match name.as_str() {
            "G0" | "G1" | "G2" | "G3" => {
                if let Some(feed) = word(&words, b'F') {
                    self.feedrate = feed;
                }
                let mut to = self.at;
                for (axis, letter) in (*b"XYZ").into_iter().enumerate() {
                    if let Some(value) = word(&words, letter) {
                        to[axis] = if self.absolute {
                            value
                        } else {
                            to[axis] + value
                        };
                    }
                }
                let e = match word(&words, b'E') {
                    Some(value) if self.absolute_e => {
                        let fed = value - self.e;
                        self.e = value;
                        fed
                    }
                    Some(value) => value,
                    None => 0.0,
                };
                let centre = match (word(&words, b'I'), word(&words, b'J')) {
                    (None, None) => None,
                    (i, j) => Some((i.unwrap_or(0.0), j.unwrap_or(0.0))),
                };
                match centre {
                    Some((i, j)) if name == "G2" || name == "G3" => {
                        self.arc(to, e, (self.at[0] + i, self.at[1] + j), name == "G2");
                    }
                    _ => {
                        let across = word(&words, b'X').is_some() || word(&words, b'Y').is_some();
                        self.line(to, e, across);
                    }
                }
            }
            "G90" => self.absolute = true,
            "G91" => self.absolute = false,
            "M82" => self.absolute_e = true,
            "M83" => self.absolute_e = false,
            "G92" => {
                if let Some(value) = word(&words, b'E') {
                    self.e = value;
                }
                for (axis, letter) in (*b"XYZ").into_iter().enumerate() {
                    if let Some(value) = word(&words, letter) {
                        self.at[axis] = value;
                    }
                }
            }
            // The part cooling fan; one named with P is another fan.
            "M106" if word(&words, b'P').is_none() => {
                let speed = word(&words, b'S').unwrap_or(255.0).clamp(0.0, 255.0);
                self.fan = (speed / 255.0 * 100.0).round() as u8;
            }
            "M107" => self.fan = 0,
            "M104" | "M109" => {
                if let Some(degrees) = word(&words, b'S') {
                    self.temperature = degrees.clamp(0.0, f64::from(u16::MAX)).round() as u16;
                }
            }
            _ => {
                // T0, T1, ...: the tool, and with it the filament.
                if let Some(Ok(tool)) = name.strip_prefix('T').map(str::parse::<u8>) {
                    let filament = tool.saturating_add(1);
                    if filament != self.filament {
                        self.filament = filament;
                        if self.placed {
                            self.push(MoveKind::ToolChange, self.at, self.width, self.height);
                        }
                    }
                }
            }
        }
    }

    /// An arc about `centre` to `to`, cut into straight moves.
    fn arc(&mut self, to: [f64; 3], e: f64, centre: (f64, f64), clockwise: bool) {
        let from = self.at;
        let radius = ((from[0] - centre.0).powi(2) + (from[1] - centre.1).powi(2)).sqrt();
        let start = (from[1] - centre.1).atan2(from[0] - centre.0);
        let end = (to[1] - centre.1).atan2(to[0] - centre.0);
        let full = std::f64::consts::TAU;
        let mut sweep = if clockwise { start - end } else { end - start };
        // An arc that ends where it starts is a whole turn.
        if sweep <= 1e-12 {
            sweep += full;
        }
        let steps = ((sweep * radius / ARC_STEP).ceil() as usize).clamp(1, 10_000);
        for step in 1..=steps {
            let part = step as f64 / steps as f64;
            let angle = if clockwise {
                start - sweep * part
            } else {
                start + sweep * part
            };
            let point = if step == steps {
                to
            } else {
                [
                    centre.0 + radius * angle.cos(),
                    centre.1 + radius * angle.sin(),
                    from[2] + (to[2] - from[2]) * part,
                ]
            };
            self.line(point, e / steps as f64, true);
        }
    }
}

/// How far each move turns into the next of the same line.
fn turns(moves: &mut [Move]) {
    for at in 1..moves.len().saturating_sub(1) {
        let (before, here, next) = (moves[at - 1], moves[at], moves[at + 1]);
        let going = matches!(
            here.kind,
            MoveKind::Travel | MoveKind::Extrude | MoveKind::Wipe
        );
        if !going || next.kind != here.kind || next.feature != here.feature {
            continue;
        }
        let into = [here.to[0] - before.to[0], here.to[1] - before.to[1]];
        let out = [next.to[0] - here.to[0], next.to[1] - here.to[1]];
        let cross = into[0] * out[1] - into[1] * out[0];
        let dot = into[0] * out[0] + into[1] * out[1];
        if cross != 0.0 || dot != 0.0 {
            moves[at].turn = cross.atan2(dot);
        }
    }
}

/// Reads a G-code file's moves. What cannot be read as a move (a macro,
/// a setting) is passed over: this is for looking at, not for planning.
pub fn read_toolpath(gcode: &[u8]) -> Toolpath {
    let mut reader = Reader::new();
    for line in gcode.split(|byte| *byte == b'\n') {
        reader.line += 1;
        let line = String::from_utf8_lossy(line);
        let line = line.trim();
        let (code, comment) = match line.split_once(';') {
            Some((code, comment)) => (code.trim(), Some(comment)),
            None => (line, None),
        };
        if code.is_empty() {
            if let Some(comment) = comment {
                reader.comment(comment);
            }
        } else {
            reader.command(code);
        }
    }
    turns(&mut reader.out.moves);
    reader.out
}

impl Sliced {
    /// The lines of the slice as a toolpath, for a print whose G-code
    /// could not be written. Only the lines are the print's: they are in
    /// the order they were worked out, not the order a printer would lay
    /// them in, and there are no moves between them, no speeds and no
    /// seams.
    pub fn toolpath_of_lines(&self) -> Toolpath {
        let mut out = Toolpath::default();
        for index in 0..self.layers.len() {
            let (z, height) = (self.top_of(index), self.height_of(index));
            out.layers.push(ToolpathLayer {
                first: out.moves.len(),
                z: z as f32,
                height: height as f32,
            });
            for stroke in self.strokes(index) {
                let Some(first) = stroke.points.first() else {
                    continue;
                };
                let mut push = |kind: MoveKind, point: &edge_geom::Point2| {
                    let laid = kind == MoveKind::Extrude;
                    out.moves.push(Move {
                        kind,
                        feature: if laid {
                            stroke.role.into()
                        } else {
                            Feature::None
                        },
                        to: [unscaled(point.x) as f32, unscaled(point.y) as f32, z as f32],
                        width: if laid { stroke.width as f32 } else { 0.0 },
                        height: if laid { height as f32 } else { 0.0 },
                        feedrate: 0.0,
                        fan: 0,
                        temperature: 0,
                        filament: stroke.filament.clamp(1, 255) as u8,
                        layer: index as u32,
                        turn: 0.0,
                    });
                };
                push(MoveKind::Start, first);
                for point in &stroke.points[1..] {
                    push(MoveKind::Extrude, point);
                }
                if stroke.closed && stroke.points.len() > 2 {
                    push(MoveKind::Extrude, first);
                }
            }
        }
        turns(&mut out.moves);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(path: &Toolpath) -> Vec<MoveKind> {
        path.moves.iter().map(|step| step.kind).collect()
    }

    #[test]
    fn features_are_numbered_and_named_as_the_reference_has_them() {
        for (number, feature) in Feature::ALL.into_iter().enumerate() {
            assert_eq!(feature as usize, number);
            assert_eq!(Feature::from_name(feature.name()), Some(feature));
        }
        assert_eq!(Feature::OuterWall as u8, 2);
        assert_eq!(Feature::Brim as u8, 17);
        assert_eq!(
            Feature::from_name("Internal Bridge"),
            Some(Feature::InternalBridge)
        );
        assert_eq!(Feature::from_name("Wall of China"), None);
        // Every role the engine lays has a feature under the same name.
        for role in [
            Role::ExternalPerimeter,
            Role::Perimeter,
            Role::OverhangPerimeter,
            Role::SparseInfill,
            Role::SolidInfill,
            Role::TopSolidInfill,
            Role::BottomSurface,
            Role::BridgeInfill,
            Role::InternalBridge,
            Role::GapFill,
            Role::Skirt,
            Role::Brim,
            Role::Support,
            Role::SupportInterface,
            Role::PrimeTower,
            Role::PrimeLine,
        ] {
            assert_eq!(Feature::from(role).name(), edge_gcode::role_name(role));
        }
    }

    #[test]
    fn moves_are_told_apart_as_the_reference_tells_them() {
        let path = read_toolpath(
            b"M83\n\
              G1 X10 Y10 Z0.2 F6000\n\
              G1 E0.8 F1800\n\
              ;TYPE:Inner wall\n\
              ;WIDTH:0.45\n\
              ;HEIGHT:0.2\n\
              G1 X20 Y10 E0.5 F1200\n\
              G1 X20 Y20 E0.5\n\
              G1 E-0.8 F1800\n\
              ;WIPE_START\n\
              G1 X19 Y19 E-0.1\n\
              ;WIPE_END\n\
              G1 X30 Y30 Z0.6 F9000\n\
              G1 Z0.2\n",
        );
        assert_eq!(
            kinds(&path),
            [
                MoveKind::Start,
                MoveKind::Unretract,
                MoveKind::Extrude,
                MoveKind::Extrude,
                MoveKind::Retract,
                MoveKind::Wipe,
                MoveKind::Travel,
                MoveKind::Travel,
            ]
        );
        let first = path.moves[2];
        assert_eq!(first.feature, Feature::InnerWall);
        assert_eq!(first.to, [20.0, 10.0, 0.2]);
        assert_eq!((first.width, first.height), (0.45, 0.2));
        assert_eq!(first.feedrate, 20.0);
        // A quarter turn to the left, into the second line; the second
        // line ends its path.
        assert!((first.turn - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(path.moves[3].turn, 0.0);
        // Only a line laid has a feature and a size.
        assert_eq!(path.moves[6].feature, Feature::None);
        assert_eq!(path.moves[6].width, 0.0);
        assert_eq!(path.moves[6].to, [30.0, 30.0, 0.6]);
        assert_eq!(path.moves[6].feedrate, 150.0);
        assert_eq!(path.layers.len(), 1);
    }

    /// Each move knows the line of the file it was read from, as the
    /// reference's G-code window finds the line of the move shown.
    #[test]
    fn each_move_knows_its_line() {
        let path = read_toolpath(
            b"M83\nG1 X0 Y0 Z0.3\n;LAYER_CHANGE\n;Z:0.3\n;HEIGHT:0.3\n\
              ;TYPE:Outer wall\n;WIDTH:0.42\nG1 X10 E1\nG1 E-1\nT1\nG1 X0 F600\n",
        );
        let kinds: Vec<MoveKind> = path.moves.iter().map(|step| step.kind).collect();
        assert_eq!(
            kinds,
            [
                MoveKind::Start,
                MoveKind::Extrude,
                MoveKind::Retract,
                MoveKind::ToolChange,
                MoveKind::Travel
            ]
        );
        assert_eq!(path.lines, [2, 8, 9, 10, 11]);
    }

    #[test]
    fn a_layer_starts_with_its_first_line() {
        let path = read_toolpath(
            b"M83\nG1 X0 Y0 Z0.3\n\
              ;TYPE:Custom\n\
              G1 X50 E5\n\
              ;LAYER_CHANGE\n;Z:0.25\n;HEIGHT:0.25\n\
              ;TYPE:Outer wall\n;WIDTH:0.42\n\
              G1 X60 E1\n\
              ;LAYER_CHANGE\n;Z:0.45\n;HEIGHT:0.2\n\
              G1 E-1\n\
              G1 X0 Y0 Z0.45\n\
              G1 E1\n\
              G1 X10 E0.3\n",
        );
        // The start G-code's line and the first layer are one layer; the
        // retraction and the move up end it, as the reference counts.
        assert_eq!(path.layers.len(), 2);
        assert_eq!(path.layers[0].first, 0);
        assert_eq!((path.layers[0].z, path.layers[0].height), (0.25, 0.25));
        let second = path.layers[1];
        assert_eq!((second.z, second.height), (0.45, 0.2));
        assert_eq!(path.moves[second.first].kind, MoveKind::Extrude);
        assert_eq!(path.moves[second.first].to, [10.0, 0.0, 0.45]);
        assert_eq!(path.moves[second.first].layer, 1);
        assert_eq!(path.moves[second.first - 1].kind, MoveKind::Unretract);
        assert_eq!(path.moves[second.first - 1].layer, 0);
        // The file's own line is as wide as what it feeds: 5 mm of 1.75 mm
        // filament over 50 mm at 0.3 mm high.
        let own = path.moves[1];
        assert_eq!(own.feature, Feature::Custom);
        let area = std::f64::consts::PI * 1.75 * 1.75 / 4.0;
        let wide = 5.0 * area / (0.3 * 50.0) + 0.3 * (1.0 - std::f64::consts::FRAC_PI_4);
        assert!((f64::from(own.width) - wide).abs() < 1e-6, "{}", own.width);
        assert!((own.height - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_seam_is_marked_where_an_outer_wall_closes() {
        let square = b"M83\nG1 X0 Y0 Z0.2\n\
              ;TYPE:Outer wall\n;WIDTH:0.4\n;HEIGHT:0.2\n\
              G1 X10 Y0 E1\nG1 X10 Y10 E1\nG1 X0 Y10 E1\nG1 X0 Y0.1 E1\n\
              ;TYPE:Sparse infill\n\
              G1 X5 Y5 E1\n";
        let path = read_toolpath(square);
        assert_eq!(
            kinds(&path),
            [
                MoveKind::Start,
                MoveKind::Extrude,
                MoveKind::Extrude,
                MoveKind::Extrude,
                MoveKind::Extrude,
                MoveKind::Seam,
                MoveKind::Start,
                MoveKind::Extrude,
            ]
        );
        // Half way between where the loop began and where it ended, and
        // the next line still starts from the nozzle.
        assert_eq!(path.moves[5].to, [0.0, 0.05, 0.2]);
        assert_eq!(path.moves[6].to, [0.0, 0.1, 0.2]);
        // A wall left open has no seam.
        let open = read_toolpath(
            b"M83\nG1 X0 Y0 Z0.2\n;TYPE:Outer wall\nG1 X10 Y0 E1\nG1 X10 Y10 E1\nG1 X20 Y20 F9000\n",
        );
        assert!(!kinds(&open).contains(&MoveKind::Seam));
    }

    #[test]
    fn tools_fans_temperatures_and_absolute_feeds_are_followed() {
        let path = read_toolpath(
            b"; filament_diameter: 2.85,2.85\n\
              M82\nG92 E0\nM104 S215\nM106 S127.5\nM106 P2 S255\n\
              G1 X0 Y0 Z0.2\n\
              ;TYPE:Skirt\n;WIDTH:0.5\n;HEIGHT:0.2\n\
              G1 X10 E1\nG1 X20 E2\n\
              T1\nM107\nM109 S230 ; wait\n\
              G1 X30 E2.5\nG1 X40 E2.5\n\
              G91\nG1 X5 E3\n",
        );
        assert_eq!(
            kinds(&path),
            [
                MoveKind::Start,
                MoveKind::Extrude,
                MoveKind::Extrude,
                MoveKind::ToolChange,
                MoveKind::Extrude,
                MoveKind::Travel,
                MoveKind::Extrude,
            ]
        );
        assert_eq!((path.moves[1].fan, path.moves[1].temperature), (50, 215));
        assert_eq!(path.moves[1].filament, 1);
        assert_eq!(path.moves[3].to, [20.0, 0.0, 0.2]);
        assert_eq!((path.moves[4].fan, path.moves[4].temperature), (0, 230));
        assert_eq!(path.moves[4].filament, 2);
        // Relative to where it was.
        assert_eq!(path.moves[6].to, [45.0, 0.0, 0.2]);
    }

    #[test]
    fn an_arc_is_cut_into_short_moves_that_end_where_it_ends() {
        let path = read_toolpath(
            b"M83\nG1 X10 Y0 Z0.2\n;TYPE:Outer wall\n;WIDTH:0.4\n;HEIGHT:0.2\nG3 X-10 Y0 I-10 J0 E3\n",
        );
        let laid: Vec<&Move> = path
            .moves
            .iter()
            .filter(|step| step.kind == MoveKind::Extrude)
            .collect();
        // Half a turn of radius 10 in steps of half a millimetre.
        assert_eq!(laid.len(), 63);
        assert_eq!(laid.last().unwrap().to, [-10.0, 0.0, 0.2]);
        for step in &laid {
            let radius = (step.to[0].powi(2) + step.to[1].powi(2)).sqrt();
            assert!((radius - 10.0).abs() < 1e-4 && step.to[1] >= -1e-4);
        }
    }

    #[test]
    fn reading_is_the_same_every_time_and_takes_any_bytes() {
        let text = b"G1 X1 Y1\n\xff\xfe;TYPE:\nG1 Xnan Ybad E\nT300\nG1 X2 Y2 E1\n";
        assert_eq!(read_toolpath(text), read_toolpath(text));
        assert_eq!(read_toolpath(b""), Toolpath::default());
    }
}
