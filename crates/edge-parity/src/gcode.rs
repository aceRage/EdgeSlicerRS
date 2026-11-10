//! Reads G-code back into what was printed: per layer and per feature, how
//! far the nozzle moved while extruding and how much filament it fed. It
//! understands what both this engine and the reference slicer write.

use std::collections::BTreeMap;

/// Extrusion of one feature on one layer, or summed over many.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stat {
    /// Distance moved while extruding, in mm.
    pub length_mm: f64,
    /// Filament fed over that distance, in mm.
    pub filament_mm: f64,
}

impl Stat {
    fn add(&mut self, other: Stat) {
        self.length_mm += other.length_mm;
        self.filament_mm += other.filament_mm;
    }
}

/// A straight extruding move, for drawing.
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
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Print {
    pub layers: Vec<Layer>,
    /// Extruded before the first layer: purge lines and the like.
    pub before_layers: Stat,
    pub retractions: usize,
    /// Distance moved without extruding, in mm, once layers have begun.
    pub travel_mm: f64,
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

    for line in text.lines() {
        let line = line.trim();
        if let Some(comment) = line.strip_prefix(';') {
            if comment.starts_with("LAYER_CHANGE") {
                print.layers.push(Layer::default());
            } else if let Some(name) = comment.strip_prefix("TYPE:") {
                feature = name.trim().to_owned();
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
        match command {
            "M82" => relative_e = false,
            "M83" => relative_e = true,
            "G92" => {
                if let Some(e) = word('E') {
                    last_e = e;
                }
            }
            "G0" | "G1" | "G2" | "G3" => {
                let target = [
                    word('X').unwrap_or(position[0]),
                    word('Y').unwrap_or(position[1]),
                ];
                let fed = match word('E') {
                    Some(e) if relative_e => e,
                    Some(e) => {
                        let fed = e - last_e;
                        last_e = e;
                        fed
                    }
                    None => 0.0,
                };
                let length = match command {
                    "G2" | "G3" => arc_length(
                        position,
                        target,
                        [word('I').unwrap_or(0.0), word('J').unwrap_or(0.0)],
                        command == "G2",
                    ),
                    _ => (target[0] - position[0]).hypot(target[1] - position[1]),
                };
                if length > 0.0 && fed > 0.0 {
                    let stat = Stat {
                        length_mm: length,
                        filament_mm: fed,
                    };
                    let index = print.layers.len();
                    match print.layers.last_mut() {
                        None => print.before_layers.add(stat),
                        Some(layer) => {
                            layer.features.entry(feature.clone()).or_default().add(stat);
                            let b = layer.bounds.get_or_insert([
                                f64::MAX,
                                f64::MAX,
                                f64::MIN,
                                f64::MIN,
                            ]);
                            for point in [position, target] {
                                *b = [
                                    b[0].min(point[0]),
                                    b[1].min(point[1]),
                                    b[2].max(point[0]),
                                    b[3].max(point[1]),
                                ];
                            }
                            if keep_segments == Some(index - 1) {
                                layer.segments.push(Segment {
                                    from: position,
                                    to: target,
                                    feature: feature.clone(),
                                });
                            }
                        }
                    }
                } else if length > 0.0 && !print.layers.is_empty() {
                    print.travel_mm += length;
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

/// Length of a circular move from `from` to `to` around `from + offset`.
fn arc_length(from: [f64; 2], to: [f64; 2], offset: [f64; 2], clockwise: bool) -> f64 {
    let centre = [from[0] + offset[0], from[1] + offset[1]];
    let radius = offset[0].hypot(offset[1]);
    let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]);
    let mut sweep = angle(to) - angle(from);
    if clockwise {
        sweep = -sweep;
    }
    // A move that ends where it began is a full circle.
    if sweep <= 1e-12 {
        sweep += std::f64::consts::TAU;
    }
    radius * sweep
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
                filament_mm: 1.0
            }
        );
        assert_eq!(print.layers.len(), 2);
        assert_eq!((print.layers[0].z, print.layers[1].z), (0.2, 0.4));
        assert_eq!(
            print.layers[0].features["Outer wall"],
            Stat {
                length_mm: 20.0,
                filament_mm: 1.0
            }
        );
        assert_eq!(
            print.layers[0].features["Sparse infill"],
            Stat {
                length_mm: 10.0,
                filament_mm: 0.25
            }
        );
        assert_eq!(print.layers[0].bounds, Some([0.0, 0.0, 10.0, 10.0]));
        assert_eq!((print.retractions, print.travel_mm), (1, 30.0));
        assert_eq!(
            print.features()["Outer wall"],
            Stat {
                length_mm: 30.0,
                filament_mm: 1.5
            }
        );
        assert_eq!(
            print.total(),
            Stat {
                length_mm: 40.0,
                filament_mm: 1.75
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
    fn absolute_extrusion_and_resets_are_followed() {
        let text = ";LAYER_CHANGE\n;TYPE:Inner wall\nG92 E0\nG1 X10 E1.0\nG1 X20 E1.5\nG92 E0\nG1 X30 E0.25\nG1 X40 E0.25\n";
        let print = parse(text, None);
        // The last move repeats E, so it is a travel.
        assert_eq!(
            print.layers[0].features["Inner wall"],
            Stat {
                length_mm: 30.0,
                filament_mm: 1.75
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
        assert!((print.layers[0].features["Unknown"].length_mm - expected).abs() < 1e-9);
    }
}
