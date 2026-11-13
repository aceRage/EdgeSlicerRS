//! Reads G-code back into what was printed: per layer and per feature, how
//! far the nozzle moved while extruding and how much filament it fed, and
//! per feature how it was printed: how fast, how wide, at what
//! acceleration and with how much fan. It understands what both this
//! engine and the reference slicer write.

use std::collections::BTreeMap;

/// Extrusion of one feature on one layer, or summed over many.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stat {
    /// Distance moved while extruding, in mm.
    pub length_mm: f64,
    /// Filament fed over that distance, in mm.
    pub filament_mm: f64,
    /// How many of its extruding moves are arcs (`G2`, `G3`), and how
    /// much of the distance they cover, in mm.
    pub arcs: usize,
    pub arc_mm: f64,
}

impl Stat {
    fn add(&mut self, other: Stat) {
        self.length_mm += other.length_mm;
        self.filament_mm += other.filament_mm;
        self.arcs += other.arcs;
        self.arc_mm += other.arc_mm;
    }
}

/// How one feature is printed, over all the layers it is on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct How {
    pub length_mm: f64,
    pub filament_mm: f64,
    /// Time its moves take at the feedrates written, in seconds.
    pub seconds: f64,
    /// The slowest and fastest of those feedrates, in mm/s.
    pub slowest: f64,
    pub fastest: f64,
    /// Width, acceleration and fan in force along it, each multiplied by
    /// the length it was in force for.
    width_by_length: f64,
    acceleration_by_length: f64,
    fan_by_length: f64,
}

impl How {
    /// Mean speed in mm/s: the length over the time it takes.
    pub fn speed(&self) -> f64 {
        self.length_mm / self.seconds.max(1e-12)
    }

    /// Mean line width in mm, as the file's `;WIDTH:` comments give it.
    pub fn width(&self) -> f64 {
        self.width_by_length / self.length_mm.max(1e-12)
    }

    /// Mean acceleration in mm/s².
    pub fn acceleration(&self) -> f64 {
        self.acceleration_by_length / self.length_mm.max(1e-12)
    }

    /// Mean part fan speed, in percent.
    pub fn fan(&self) -> f64 {
        self.fan_by_length / self.length_mm.max(1e-12) / 255.0 * 100.0
    }

    /// Plastic laid per mm of line, in mm³, for 1.75 mm filament.
    pub fn mm3_per_mm(&self) -> f64 {
        self.filament_mm * std::f64::consts::PI * 0.875 * 0.875 / self.length_mm.max(1e-12)
    }
}

/// A straight extruding move, for drawing. An arc is drawn as the
/// chords it is cut into.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub feature: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    /// Height of the layer's top above the bed, from its `;Z:` comment.
    pub z: f64,
    /// By feature name, as written in `;TYPE:` comments.
    pub features: BTreeMap<String, Stat>,
    /// Corners of the box around everything extruded: min x, min y, max x, max y.
    pub bounds: Option<[f64; 4]>,
    /// Every extruding move, kept only for the layer asked for.
    pub segments: Vec<Segment>,
    /// The lowest and highest the nozzle stood while it laid the prime
    /// tower on this layer; none where it laid none. Below the layer's
    /// own `z` where the tower leaves out the layers with no tool change.
    pub tower: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Print {
    pub layers: Vec<Layer>,
    /// Extruded before the first layer: purge lines and the like.
    pub before_layers: Stat,
    pub retractions: usize,
    /// Distance moved without extruding, in mm, once layers have begun.
    pub travel_mm: f64,
    /// The machine's start G-code as written: every line from the first
    /// `;TYPE:Custom` to the first layer, without trailing spaces, blank
    /// lines or progress reports.
    pub start_block: Vec<String>,
    /// How each feature is printed on the first layer, and on the rest.
    pub how_first: BTreeMap<String, How>,
    pub how: BTreeMap<String, How>,
    /// Time spent moving without extruding once layers have begun, at
    /// the feedrates written, in seconds.
    pub travel_seconds: f64,
    /// Every nozzle and bed temperature set once layers have begun, with
    /// the layer it is set on, counting from 0.
    pub temperatures: Vec<(usize, String)>,
}

impl Print {
    /// Totals by feature over all layers.
    pub fn features(&self) -> BTreeMap<String, Stat> {
        let mut totals: BTreeMap<String, Stat> = BTreeMap::new();
        for layer in &self.layers {
            for (feature, stat) in &layer.features {
                totals.entry(feature.clone()).or_default().add(*stat);
            }
        }
        totals
    }

    pub fn total(&self) -> Stat {
        let mut total = Stat::default();
        self.features().values().for_each(|stat| total.add(*stat));
        total
    }
}

/// Parses a whole file. `keep_segments` names one layer (counting from 0)
/// whose individual moves are kept.
pub fn parse(text: &str, keep_segments: Option<usize>) -> Print {
    let mut print = Print::default();
    let mut position = [0.0f64; 2];
    // Extrusion is absolute until M83 says otherwise.
    let (mut relative_e, mut last_e) = (false, 0.0f64);
    let mut feature = String::from("Unknown");
    // What is in force as the file is read: feedrate in mm/min, line
    // width in mm, acceleration in mm/s² and the part fan from 0 to 255.
    let (mut feedrate, mut width, mut acceleration, mut fan) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);

    // The nozzle's height, and whether moves are given relative to where
    // it is (`G91`, as a change G-code may lift the nozzle).
    let (mut z, mut relative_xyz) = (0.0f64, false);
    let mut in_start_block = false;
    for line in text.lines() {
        let line = line.trim();
        // A Bambu Lab printer's file has Bambu Studio's tags, which say the
        // same (GCodeProcessor.cpp `Reserved_Tags`).
        let renamed;
        let line = match line.strip_prefix("; ") {
            Some("CHANGE_LAYER") => ";LAYER_CHANGE",
            Some(tag) => match [
                ("FEATURE: ", ";TYPE:"),
                ("LINE_WIDTH: ", ";WIDTH:"),
                ("Z_HEIGHT: ", ";Z:"),
            ]
            .iter()
            .find_map(|(from, to)| Some((tag.strip_prefix(from)?, to)))
            {
                Some((value, to)) => {
                    renamed = format!("{to}{value}");
                    renamed.as_str()
                }
                None => line,
            },
            None => line,
        };
        if print.layers.is_empty() {
            if line == ";TYPE:Custom" {
                in_start_block = true;
            } else if in_start_block
                && !line.is_empty()
                && !line.starts_with("M73")
                && !line.starts_with(";LAYER_CHANGE")
            {
                print.start_block.push(line.to_owned());
            }
        }
        if let Some(comment) = line.strip_prefix(';') {
            if comment.starts_with("LAYER_CHANGE") {
                print.layers.push(Layer::default());
            } else if let Some(name) = comment.strip_prefix("TYPE:") {
                feature = name.trim().to_owned();
            } else if let Some(value) = comment.strip_prefix("WIDTH:") {
                width = value.trim().parse().unwrap_or(width);
            } else if let (Some(z), Some(layer)) =
                (comment.strip_prefix("Z:"), print.layers.last_mut())
            {
                layer.z = z.trim().parse().unwrap_or(layer.z);
            }
            continue;
        }
        // Drop a trailing comment, then split into words like `X12.5`.
        let code = line.split(';').next().unwrap_or("").trim();
        let mut words = code.split_ascii_whitespace();
        let Some(command) = words.next() else {
            continue;
        };
        let word = |letter: char| -> Option<f64> {
            code.split_ascii_whitespace()
                .skip(1)
                .find_map(|w| w.strip_prefix(letter)?.parse().ok())
        };
        // A value written as `NAME=12`, as Klipper's own commands take them.
        let named = |name: &str| -> Option<f64> {
            code.split_ascii_whitespace()
                .skip(1)
                .find_map(|w| w.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
        };
        match command {
            "SET_VELOCITY_LIMIT" => acceleration = named("ACCEL").unwrap_or(acceleration),
            "M204" => acceleration = word('S').or(word('P')).unwrap_or(acceleration),
            // The part fan is the one with no number, or number 1.
            "M106" if word('P').is_none_or(|fan| fan == 1.0) => {
                fan = word('S').unwrap_or(255.0);
            }
            "M107" if word('P').is_none_or(|fan| fan == 1.0) => fan = 0.0,
            "M104" | "M109" | "M140" | "M190" if !print.layers.is_empty() => {
                if let Some(target) = word('S') {
                    let tool = word('T').map_or(String::new(), |tool| format!(" T{tool:.0}"));
                    print.temperatures.push((
                        print.layers.len() - 1,
                        format!("{command} S{target:.0}{tool}"),
                    ));
                }
            }
            "M82" => relative_e = false,
            "M83" => relative_e = true,
            "G90" => relative_xyz = false,
            "G91" => relative_xyz = true,
            "G92" => {
                if let Some(e) = word('E') {
                    last_e = e;
                }
            }
            "G0" | "G1" | "G2" | "G3" => {
                if let Some(to) = word('Z') {
                    z = if relative_xyz { z + to } else { to };
                }
                let target = [
                    word('X').unwrap_or(position[0]),
                    word('Y').unwrap_or(position[1]),
                ];
                feedrate = word('F').unwrap_or(feedrate);
                let fed = match word('E') {
                    Some(e) if relative_e => e,
                    Some(e) => {
                        let fed = e - last_e;
                        last_e = e;
                        fed
                    }
                    None => 0.0,
                };
                let turn = matches!(command, "G2" | "G3").then(|| {
                    (
                        [word('I').unwrap_or(0.0), word('J').unwrap_or(0.0)],
                        command == "G2",
                    )
                });
                let length = match turn {
                    Some((offset, clockwise)) => arc_length(position, target, offset, clockwise),
                    None => (target[0] - position[0]).hypot(target[1] - position[1]),
                };
                if length > 0.0 && fed > 0.0 {
                    let stat = Stat {
                        length_mm: length,
                        filament_mm: fed,
                        arcs: usize::from(turn.is_some()),
                        arc_mm: if turn.is_some() { length } else { 0.0 },
                    };
                    let index = print.layers.len();
                    if index > 0 && feedrate > 0.0 {
                        let how = if index == 1 {
                            &mut print.how_first
                        } else {
                            &mut print.how
                        };
                        let how = how.entry(feature.clone()).or_default();
                        let speed = feedrate / 60.0;
                        how.slowest = if how.length_mm == 0.0 {
                            speed
                        } else {
                            how.slowest.min(speed)
                        };
                        how.fastest = how.fastest.max(speed);
                        how.length_mm += length;
                        how.filament_mm += fed;
                        how.seconds += length / speed;
                        how.width_by_length += width * length;
                        how.acceleration_by_length += acceleration * length;
                        how.fan_by_length += fan * length;
                    }
                    match print.layers.last_mut() {
                        None => print.before_layers.add(stat),
                        Some(layer) => {
                            layer.features.entry(feature.clone()).or_default().add(stat);
                            if feature == "Prime tower" {
                                let [low, high] = layer.tower.get_or_insert([z, z]);
                                *low = low.min(z);
                                *high = high.max(z);
                            }
                            let b = layer.bounds.get_or_insert([
                                f64::MAX,
                                f64::MAX,
                                f64::MIN,
                                f64::MIN,
                            ]);
                            // Where the move goes: straight there, or by
                            // way of the points along an arc, so that the
                            // box counts its bulge.
                            let way = match turn {
                                Some((offset, clockwise)) => {
                                    arc_points(position, target, offset, clockwise)
                                }
                                None => vec![target],
                            };
                            let mut from = position;
                            for point in std::iter::once(position).chain(way.iter().copied()) {
                                *b = [
                                    b[0].min(point[0]),
                                    b[1].min(point[1]),
                                    b[2].max(point[0]),
                                    b[3].max(point[1]),
                                ];
                            }
                            if keep_segments == Some(index - 1) {
                                for to in way {
                                    layer.segments.push(Segment {
                                        from,
                                        to,
                                        feature: feature.clone(),
                                    });
                                    from = to;
                                }
                            }
                        }
                    }
                } else if length > 0.0 && !print.layers.is_empty() {
                    print.travel_mm += length;
                    if feedrate > 0.0 {
                        print.travel_seconds += length / (feedrate / 60.0);
                    }
                } else if length == 0.0 && fed < 0.0 {
                    print.retractions += 1;
                }
                position = target;
            }
            _ => {}
        }
    }
    print
}

/// The angle a circular move from `from` to `to` around `from + offset`
/// turns through, in radians.
fn arc_sweep(from: [f64; 2], to: [f64; 2], offset: [f64; 2], clockwise: bool) -> f64 {
    let centre = [from[0] + offset[0], from[1] + offset[1]];
    let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]);
    let mut sweep = angle(to) - angle(from);
    if clockwise {
        sweep = -sweep;
    }
    // A move that ends where it began is a full circle.
    if sweep <= 1e-12 {
        sweep += std::f64::consts::TAU;
    }
    sweep
}

/// Length of that move.
fn arc_length(from: [f64; 2], to: [f64; 2], offset: [f64; 2], clockwise: bool) -> f64 {
    offset[0].hypot(offset[1]) * arc_sweep(from, to, offset, clockwise)
}

/// Points along that move, no more than 5 degrees apart, the last of
/// them `to`.
fn arc_points(from: [f64; 2], to: [f64; 2], offset: [f64; 2], clockwise: bool) -> Vec<[f64; 2]> {
    let centre = [from[0] + offset[0], from[1] + offset[1]];
    let radius = offset[0].hypot(offset[1]);
    let sweep = arc_sweep(from, to, offset, clockwise);
    let start = (from[1] - centre[1]).atan2(from[0] - centre[0]);
    let steps = (sweep / 5f64.to_radians()).ceil().max(1.0) as usize;
    let mut points: Vec<[f64; 2]> = (1..steps)
        .map(|step| {
            let turned = sweep * step as f64 / steps as f64;
            let angle = if clockwise {
                start - turned
            } else {
                start + turned
            };
            [
                centre[0] + radius * angle.cos(),
                centre[1] + radius * angle.sin(),
            ]
        })
        .collect();
    points.push(to);
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_features_and_filament_are_tallied() {
        let text = "\
M83
G1 X0 Y0 F6000
G1 X20 Y0 E1.0 ; purge
;LAYER_CHANGE
;Z:0.2
;HEIGHT:0.2
G1 X0 Y0
;TYPE:Outer wall
G1 X10 Y0 E0.5
G1 X10 Y10 E0.5
G1 E-0.8 F1800
G1 X0 Y10
G1 E0.8
;TYPE:Sparse infill
G1 X0 Y0 E0.25
;LAYER_CHANGE
;Z:0.4
;TYPE:Outer wall
G1 X10 Y0 E0.5
";
        let print = parse(text, Some(0));
        assert_eq!(
            print.before_layers,
            Stat {
                length_mm: 20.0,
                filament_mm: 1.0,
                ..Stat::default()
            }
        );
        assert_eq!(print.layers.len(), 2);
        assert_eq!((print.layers[0].z, print.layers[1].z), (0.2, 0.4));
        assert_eq!(
            print.layers[0].features["Outer wall"],
            Stat {
                length_mm: 20.0,
                filament_mm: 1.0,
                ..Stat::default()
            }
        );
        assert_eq!(
            print.layers[0].features["Sparse infill"],
            Stat {
                length_mm: 10.0,
                filament_mm: 0.25,
                ..Stat::default()
            }
        );
        assert_eq!(print.layers[0].bounds, Some([0.0, 0.0, 10.0, 10.0]));
        assert_eq!((print.retractions, print.travel_mm), (1, 30.0));
        assert_eq!(
            print.features()["Outer wall"],
            Stat {
                length_mm: 30.0,
                filament_mm: 1.5,
                ..Stat::default()
            }
        );
        assert_eq!(
            print.total(),
            Stat {
                length_mm: 40.0,
                filament_mm: 1.75,
                ..Stat::default()
            }
        );
        // Only the layer asked for keeps its moves.
        assert_eq!(
            (
                print.layers[0].segments.len(),
                print.layers[1].segments.len()
            ),
            (3, 0)
        );
        assert_eq!(print.layers[0].segments[2].feature, "Sparse infill");
    }

    #[test]
    fn start_block_is_the_custom_lines_before_the_first_layer() {
        let text = "; header\nM106 S0\n;TYPE:Custom\nPRINT_START  \n\nM73 P0 R9\nG28 Z\n;LAYER_CHANGE\n;TYPE:Custom\nLATER\n";
        assert_eq!(parse(text, None).start_block, ["PRINT_START", "G28 Z"]);
    }

    #[test]
    fn absolute_extrusion_and_resets_are_followed() {
        let text = ";LAYER_CHANGE\n;TYPE:Inner wall\nG92 E0\nG1 X10 E1.0\nG1 X20 E1.5\nG92 E0\nG1 X30 E0.25\nG1 X40 E0.25\n";
        let print = parse(text, None);
        // The last move repeats E, so it is a travel.
        assert_eq!(
            print.layers[0].features["Inner wall"],
            Stat {
                length_mm: 30.0,
                filament_mm: 1.75,
                ..Stat::default()
            }
        );
        assert_eq!(print.travel_mm, 10.0);
    }

    #[test]
    fn arcs_count_their_curved_length() {
        // A quarter circle of radius 10, counter-clockwise, then a full circle.
        let text = ";LAYER_CHANGE\nM83\nG1 X10 Y0\nG3 X0 Y10 I-10 J0 E1\nG2 X0 Y10 I0 J-10 E1\n";
        let print = parse(text, None);
        let expected = std::f64::consts::PI * 5.0 + std::f64::consts::TAU * 10.0;
        let stat = print.layers[0].features["Unknown"];
        assert!((stat.length_mm - expected).abs() < 1e-9);
        assert_eq!(stat.arcs, 2);
        assert!((stat.arc_mm - expected).abs() < 1e-9);
    }

    #[test]
    fn an_arcs_bulge_is_in_the_layers_box_and_its_drawing() {
        // Half a circle of radius 10 over the top, from (10, 0) to
        // (-10, 0): its ends are level, its top 10 mm above them.
        let text = ";LAYER_CHANGE\nM83\nG1 X10 Y0\n;TYPE:Outer wall\nG3 X-10 Y0 I-10 J0 E1\nG1 X-10 Y-5 E0.5\n";
        let print = parse(text, Some(0));
        let bounds = print.layers[0].bounds.unwrap();
        assert!((bounds[0] + 10.0).abs() < 1e-9 && (bounds[2] - 10.0).abs() < 1e-9);
        assert!((bounds[1] + 5.0).abs() < 1e-9 && (bounds[3] - 10.0).abs() < 1e-9);
        // Drawn as chords that stay on the circle, then the straight move.
        let segments = &print.layers[0].segments;
        assert_eq!(segments.len(), 36 + 1);
        assert!(
            segments[..36]
                .iter()
                .all(|segment| (segment.to[0].hypot(segment.to[1]) - 10.0).abs() < 1e-9)
        );
        assert_eq!(segments[35].to, [-10.0, 0.0]);
        assert_eq!(
            (segments[36].from, segments[36].to),
            ([-10.0, 0.0], [-10.0, -5.0])
        );
        let wall = print.layers[0].features["Outer wall"];
        assert_eq!((wall.arcs, wall.filament_mm), (1, 1.5));
    }

    #[test]
    fn how_each_feature_is_printed_is_tallied() {
        let text = "\
M83
;LAYER_CHANGE
;Z:0.2
M106 S0
M204 S500
;TYPE:Outer wall
;WIDTH:0.5
G1 X10 Y0 E0.5 F1800
;LAYER_CHANGE
;Z:0.4
M104 S210
SET_VELOCITY_LIMIT ACCEL=5000 ACCEL_TO_DECEL=2500
M106 S127.5
;TYPE:Outer wall
;WIDTH:0.42
G1 X20 Y0 E0.4 F6000
M106 S255
M106 P2 S90
G1 X50 Y0 E1.2 F3000
G1 X50 Y30 F12000
M104 S0 T1
";
        let print = parse(text, None);
        let first = print.how_first["Outer wall"];
        assert_eq!(
            (
                first.speed(),
                first.width(),
                first.acceleration(),
                first.fan()
            ),
            (30.0, 0.5, 500.0, 0.0)
        );
        let rest = print.how["Outer wall"];
        // 10 mm at 100 mm/s and 30 mm at 50 mm/s: 40 mm in 0.7 s.
        assert!((rest.speed() - 40.0 / 0.7).abs() < 1e-9);
        assert_eq!((rest.slowest, rest.fastest), (50.0, 100.0));
        assert!((rest.width() - 0.42).abs() < 1e-12 && rest.acceleration() == 5000.0);
        // Half fan for a quarter of the way, full for the rest; the
        // second fan is another fan.
        assert!((rest.fan() - 87.5).abs() < 1e-9);
        assert!(
            (rest.mm3_per_mm() - 1.6 * std::f64::consts::PI * 0.875 * 0.875 / 40.0).abs() < 1e-12
        );
        assert!((print.travel_seconds - 30.0 / 200.0).abs() < 1e-12);
        assert_eq!(
            print.temperatures,
            [(1, "M104 S210".to_owned()), (1, "M104 S0 T1".to_owned())]
        );
    }
}
