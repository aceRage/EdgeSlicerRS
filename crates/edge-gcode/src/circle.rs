//! The walls round a circle that the auto circle compensation resized.
//!
//! The filament's compensation was measured with those walls printed at
//! one speed (`circle_compensation_speed`), so they are printed at it
//! (GCode.cpp `extrude_loop` and `_extrude`, from Bambu Studio): it is
//! their speed in place of the outer or inner wall's, with everything
//! that holds a wall's speed down still holding it (the first layer, the
//! hot end's flow, an overhang), and a loop small enough for
//! `small_perimeter_speed` keeps that where it is the lower. Nor are
//! they slowed to give a layer time to cool: the reference marks them
//! "; Slow Down Start" and "; Slow Down End", which its cooling stage
//! leaves at the speed written.
//!
//! Not done: the reference joins such a wall with a scarf seam where
//! `apply_scarf_seam_on_circles` is on; there are no scarf seams here
//! yet, and the settings say so. The two comments are not written:
//! nothing but the reference's own cooling stage reads them.

use edge_extrusion::{ExtrusionLoop, Role};

use crate::{Writer, overhang};

impl Writer<'_> {
    /// The speed the loop about to be laid goes at as the wall of a
    /// compensated circle, before anything holds it down; none for any
    /// other loop. To be asked once the loop's speed as a small one is
    /// settled.
    pub(crate) fn circle_wall_speed(&self, wall: &ExtrusionLoop) -> Option<f64> {
        let speed = self.settings.circle_compensation_speed;
        // "unless the loop overhangs": not where a stretch of it is laid
        // as an overhang wall.
        if !wall.circle_compensation || speed <= 0.0 || self.hangs_clear(wall) {
            return None;
        }
        // "a caller's small perimeter speed still caps it"
        Some(
            self.state
                .small_wall
                .map_or(speed, |small| small.min(speed)),
        )
    }

    /// Whether any of the loop is over air, as the walls are judged when
    /// they are laid: only where overhanging walls are told apart
    /// (`detect_overhang_wall`), and never on the first layer.
    fn hangs_clear(&self, wall: &ExtrusionLoop) -> bool {
        let s = self.settings;
        let (Some(below), false, true) = (
            self.below.as_ref(),
            self.state.first_layer,
            s.detect_overhang_wall,
        ) else {
            return false;
        };
        let mut round = wall.polygon.points.clone();
        round.extend(wall.polygon.points.first().copied());
        overhang::Judge {
            below,
            width: wall.flow.width,
            clear_beyond: Some(0.5 * s.nozzle_diameter),
            speed: &|_| 0.0,
            fan_at: None,
        }
        .pieces(&round)
        .iter()
        .any(|piece| piece.clear)
    }

    /// A wall's speed: the compensated circle's, the small loop's, or
    /// `usual`.
    pub(crate) fn wall_speed(&self, usual: f64) -> f64 {
        self.state
            .circle_wall
            .or(self.state.small_wall)
            .unwrap_or(usual)
    }

    /// Whether a line in this role is part of a compensated circle's
    /// wall, and so keeps its speed when the layer is slowed to cool.
    pub(crate) fn circle_wall_held(&self, role: Role) -> bool {
        self.state.circle_wall.is_some()
            && matches!(role, Role::ExternalPerimeter | Role::Perimeter)
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use edge_config::FloatOrPercent;
    use edge_extrusion::Flow;
    use edge_geom::{ExPolygon, Point2, Polygon};

    use super::*;
    use crate::{Layer, Lift, PrintSettings, Seam, Summary, write};

    /// A square wall, the outer one, marked or not as a compensated
    /// circle's.
    fn wall(x: f64, side: f64, circle: bool) -> ExtrusionLoop {
        ExtrusionLoop {
            widths: Vec::new(),
            clockwise: false,
            circle_compensation: circle,
            polygon: Polygon {
                points: [(x, 0.0), (x + side, 0.0), (x + side, side), (x, side)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            role: Role::ExternalPerimeter,
            is_hole: false,
            depth: 0,
            flow: Flow::new(0.45, 0.2, 0.4).unwrap(),
        }
    }

    /// Walls at 30 mm/s, compensated circles' at 70, and nothing else
    /// that would change a wall's speed.
    fn settings() -> PrintSettings {
        PrintSettings {
            seam: Seam::Nearest,
            seam_gap: 0.0,
            lift: Lift::Normal,
            slow_down_for_layer_cooling: false,
            outer_wall_speed: 30.0,
            inner_wall_speed: 40.0,
            initial_layer_speed: 20.0,
            filament_max_volumetric_speed: 0.0,
            enable_overhang_speed: false,
            detect_overhang_wall: false,
            overhang_fan: None,
            circle_compensation_speed: 70.0,
            ..PrintSettings::default()
        }
    }

    /// Two layers of these walls, each wall an island, over `below`.
    fn printed(
        loops: &[ExtrusionLoop],
        below: &[ExPolygon],
        settings: &PrintSettings,
    ) -> (String, Summary) {
        let islands: Vec<Range<usize>> = (0..loops.len()).map(|at| at..at + 1).collect();
        let layer = |z: f64, below| Layer {
            z,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            tower: &[],
            around: &[],
            support: &[],
            loops,
            islands: &islands,
            paths: &[],
            shelter: &[],
            below,
        };
        let mut out = Vec::new();
        let summary = write(&mut out, &[layer(0.2, &[]), layer(0.4, below)], settings).unwrap();
        (String::from_utf8(out).unwrap(), summary)
    }

    /// The speeds, in mm/s, that lines are laid at on the first layer
    /// and on the second, slowest first.
    fn speeds(text: &str) -> [Vec<f64>; 2] {
        let mut found = [Vec::new(), Vec::new()];
        let (mut layer, mut feed) = (0usize, 0.0);
        for line in text.lines() {
            if line.starts_with(";LAYER_CHANGE") {
                layer += 1;
            }
            if !line.starts_with("G1 ") {
                continue;
            }
            let word = |letter: char| {
                line.split(' ')
                    .find_map(|part| part.strip_prefix(letter)?.parse::<f64>().ok())
            };
            feed = word('F').unwrap_or(feed);
            let extrudes = word('E').is_some_and(|e| e > 0.0) && word('X').is_some();
            if extrudes && (1..=2).contains(&layer) && !found[layer - 1].contains(&(feed / 60.0)) {
                found[layer - 1].push(feed / 60.0);
            }
        }
        for layer in &mut found {
            layer.sort_by(f64::total_cmp);
        }
        found
    }

    #[test]
    fn a_compensated_circles_wall_goes_at_the_compensation_speed() {
        let loops = [wall(0.0, 20.0, false), wall(30.0, 20.0, true)];
        let (text, _) = printed(&loops, &[], &settings());
        let [first, second] = speeds(&text);
        // The first layer's own speed holds both walls there.
        assert_eq!(first, [20.0]);
        assert_eq!(second, [30.0, 70.0]);
        // Marked or not, a wall has the same plastic.
        let (_, marked) = printed(&loops, &[], &settings());
        let plain = [wall(0.0, 20.0, false), wall(30.0, 20.0, false)];
        let (plain_text, unmarked) = printed(&plain, &[], &settings());
        assert_eq!(marked.filament_mm, unmarked.filament_mm);
        assert_eq!(speeds(&plain_text)[1], [30.0]);

        // The hot end's flow still holds it: 4 mm3/s through a line of
        // 0.45 by 0.2 mm is about 49 mm/s.
        let choked = PrintSettings {
            filament_max_volumetric_speed: 4.0,
            ..settings()
        };
        let [_, second] = speeds(&printed(&loops, &[], &choked).0);
        assert_eq!(second.len(), 2);
        assert!(
            second[0] == 30.0 && second[1] > 45.0 && second[1] < 55.0,
            "{second:?}"
        );

        // At no speed of its own it is a wall like any other.
        let none = PrintSettings {
            circle_compensation_speed: 0.0,
            ..settings()
        };
        assert_eq!(speeds(&printed(&loops, &[], &none).0)[1], [30.0]);
    }

    #[test]
    fn the_speed_is_the_filaments_and_the_seam_not_made_is_said() {
        let read = |json: &str, filament: usize| {
            PrintSettings::for_filament(&edge_config::Settings::from_json(json).unwrap(), filament)
                .unwrap()
        };
        // The reference's default where a profile says nothing.
        assert_eq!(read("{}", 0).circle_compensation_speed, 200.0);
        let two = r#"{"circle_compensation_speed": ["120", "80"]}"#;
        assert_eq!(read(two, 0).circle_compensation_speed, 120.0);
        assert_eq!(read(two, 1).circle_compensation_speed, 80.0);
        // A scarf seam on compensated circles is asked for unless turned
        // off, and only means anything with the compensation on.
        let said = |json: &str| {
            read(json, 0)
                .unsupported
                .iter()
                .filter(|note| note.contains("circle"))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert!(said("{}").is_empty());
        assert!(said(r#"{"apply_scarf_seam_on_circles": "1"}"#).is_empty());
        assert_eq!(
            said(r#"{"enable_circle_compensation": "1"}"#),
            [
                "apply_scarf_seam_on_circles \"1\": the walls of compensated circles are joined with a plain butt seam"
            ]
        );
        assert!(
            said(r#"{"enable_circle_compensation": "1", "apply_scarf_seam_on_circles": "0"}"#)
                .is_empty()
        );
    }

    #[test]
    fn a_small_loops_speed_still_caps_it() {
        // Both loops are small: 16 mm round, under a circle of 3 mm.
        let loops = [wall(0.0, 4.0, false), wall(30.0, 4.0, true)];
        let small = |speed: f64| PrintSettings {
            small_perimeter_threshold: 3.0,
            small_perimeter_speed: FloatOrPercent::Value(speed),
            ..settings()
        };
        // Slower than the compensation speed: kept.
        assert_eq!(speeds(&printed(&loops, &[], &small(12.0)).0)[1], [12.0]);
        // Faster: the compensation speed takes its place.
        assert_eq!(
            speeds(&printed(&loops, &[], &small(90.0)).0)[1],
            [70.0, 90.0]
        );
    }

    #[test]
    fn it_is_not_slowed_to_give_the_layer_time_to_cool() {
        let cooling = PrintSettings {
            slow_down_for_layer_cooling: true,
            slow_down_layer_time: 30.0,
            slow_down_min_speed: 10.0,
            ..settings()
        };
        // Alone on its layer, a marked wall leaves nothing to slow.
        let alone = [wall(0.0, 20.0, true)];
        let (text, summary) = printed(&alone, &[], &cooling);
        // On the first layer it goes at that layer's speed, and is no
        // more slowed there: the reference marks it on every layer.
        assert_eq!(speeds(&text), [vec![20.0], vec![70.0]]);
        assert_eq!(summary.slowed_layers, 0);
        // Beside a wall that is slowed, it keeps its speed.
        let loops = [wall(0.0, 20.0, false), wall(30.0, 20.0, true)];
        let (text, summary) = printed(&loops, &[], &cooling);
        let [_, second] = speeds(&text);
        assert_eq!(second.len(), 2);
        assert!(second[0] < 30.0 && second[1] == 70.0, "{second:?}");
        assert_eq!(summary.slowed_layers, 2);
        // The same wall unmarked is slowed with the rest.
        let plain = [wall(0.0, 20.0, false), wall(30.0, 20.0, false)];
        let [_, second] = speeds(&printed(&plain, &[], &cooling).0);
        assert!(second.iter().all(|speed| *speed < 30.0), "{second:?}");
    }

    #[test]
    fn a_loop_that_hangs_over_air_is_a_wall_like_any_other() {
        let loops = [wall(0.0, 20.0, true)];
        let detecting = PrintSettings {
            detect_overhang_wall: true,
            bridge_speed: 25.0,
            ..settings()
        };
        let under = |x0: f64, x1: f64| {
            [ExPolygon {
                contour: Polygon {
                    points: [(x0, -5.0), (x1, -5.0), (x1, 25.0), (x0, 25.0)]
                        .map(|(x, y)| Point2::from_mm(x, y))
                        .to_vec(),
                },
                holes: Vec::new(),
            }]
        };
        // With all of it over the layer beneath, at the compensation
        // speed.
        let [_, second] = speeds(&printed(&loops, &under(-5.0, 25.0), &detecting).0);
        assert_eq!(second, [70.0]);
        // With half of it over air, that half is an overhang wall and
        // the rest goes at the wall's own speed: the reference gives a
        // loop any of which is an overhang wall no speed of its own.
        let (text, _) = printed(&loops, &under(-5.0, 10.0), &detecting);
        let [_, second] = speeds(&text);
        assert!(
            second.contains(&30.0) && second.contains(&25.0),
            "{second:?}"
        );
        assert!(!second.contains(&70.0), "{second:?}");
        assert!(text.contains(";TYPE:Overhang wall"));
        // Where overhanging walls are not told apart, nothing of it is
        // one, and it keeps the compensation speed.
        let (text, _) = printed(&loops, &under(-5.0, 10.0), &settings());
        assert_eq!(speeds(&text)[1], [70.0]);
    }
}
