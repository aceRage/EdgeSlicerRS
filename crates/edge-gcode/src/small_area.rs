//! Small area flow compensation (`small_area_infill_flow_compensation`,
//! with its curve `small_area_infill_flow_compensation_model`): a short
//! line of solid fill is laid with less plastic than its length says,
//! by a share that falls with how short it is. A small area fills with
//! lines too short for the nozzle to reach its speed on, and comes out
//! overfilled without.
//!
//! A port of the reference's GCode/SmallAreaInfillFlowCompensator.cpp,
//! of the curve it draws through the model's points
//! (GCode/PchipInterpolatorHelper.cpp), and of where its writer calls
//! for one (GCode.cpp `GCode::_needSAFC`, and `_extrude`, which scales
//! every move by its own length).
//!
//! Not as the reference, both where it goes against what it is plainly
//! for:
//!
//! - `_needSAFC` asks whether the pattern *set* for a line's kind is one
//!   of the straight ones, not what the line was laid as, so the loops
//!   a narrow solid area is filled with
//!   (`detect_narrow_internal_solid_infill`) pass for straight lines
//!   there and every short stretch of a loop is thinned: a loop round a
//!   curve, cut into chords of half a millimetre, is laid at two thirds
//!   of its plastic from end to end. Here a loop keeps its flow.
//! - It takes a bottom surface for whatever is on the first layer
//!   (`on_first_layer() && bottom_surface_pattern == ...`): there every
//!   kind of solid fill goes by the bottom surface's pattern, and a
//!   bottom surface on any other layer (one resting on supports) is
//!   never compensated, though `modify_flow` names its kind. Here each
//!   kind goes by its own pattern on every layer.

use edge_config::{ConfigError, Settings};
use edge_extrusion::{ExtrusionPath, Role};

const FLAG: &str = "small_area_infill_flow_compensation";
const MODEL: &str = "small_area_infill_flow_compensation_model";

/// The curve short lines of solid fill are thinned by, and which kinds
/// of solid fill it is for.
#[derive(Clone, Debug, PartialEq)]
pub struct SmallArea {
    /// The model's points: a length in mm and the share of its plastic a
    /// line that long is laid with, the lengths rising from zero.
    lengths: Vec<f64>,
    shares: Vec<f64>,
    /// The curve's slope at each point.
    slopes: Vec<f64>,
    /// Whether top surfaces, bottom surfaces and solid fill inside the
    /// part are set to a pattern of straight lines, the only ones a
    /// line's length says anything about.
    top: bool,
    bottom: bool,
    solid: bool,
}

/// The patterns of straight lines (`supported_patterns` in `_needSAFC`).
fn straight(settings: &Settings, key: &str, default: bool) -> bool {
    settings
        .texts(key)
        .and_then(|pattern| pattern.first())
        .map_or(default, |pattern| {
            matches!(
                pattern.as_str(),
                "rectilinear" | "alignedrectilinear" | "monotonic" | "monotonicline"
            )
        })
}

impl SmallArea {
    /// The compensation the settings ask for; none when it is off or the
    /// model is empty. A model the reference refuses to slice with is an
    /// error here too.
    pub fn read(settings: &Settings) -> Result<Option<Self>, ConfigError> {
        if settings.flag(FLAG)? != Some(true) {
            return Ok(None);
        }
        let Some(lines) = settings.texts(MODEL).filter(|lines| !lines.is_empty()) else {
            return Ok(None);
        };
        let bad = |found: &str, expected: &'static str| ConfigError::BadValue {
            key: MODEL.into(),
            found: found.to_owned(),
            expected,
        };
        let (mut lengths, mut shares) = (Vec::new(), Vec::new());
        for line in lines {
            // "length,share", with whatever space and line breaks the
            // settings page left round them; a preset, and the table of
            // the reference's defaults, has a line break written out as
            // a backslash and an n. A line with nothing before its
            // comma is passed over, and so is one with no comma, as in
            // the reference.
            let plain = line.replace("\\n", "\n").replace("\\r", "\r");
            let mut parts = plain.split(',');
            let length = parts.next().unwrap_or("").trim();
            if length.is_empty() {
                continue;
            }
            let number = |text: &str| {
                text.trim()
                    .trim_end_matches(';')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| bad(line, "a length in mm and a share, as in \"0.2,0.4444\""))
            };
            let length = number(length)?;
            if let Some(share) = parts.next().filter(|share| !share.is_empty()) {
                lengths.push(length);
                shares.push(number(share)?);
            }
        }
        let all = lines.join(";");
        // The lengths start at nothing and rise; so do the shares, to
        // end at the whole of a line's plastic.
        for (index, &length) in lengths.iter().enumerate() {
            if index == 0 {
                if length != 0.0 {
                    return Err(bad(&all, "its first length to be 0"));
                }
            } else if length <= lengths[index - 1] {
                return Err(bad(&all, "lengths that rise from one point to the next"));
            }
        }
        if shares.windows(2).any(|pair| pair[1] <= pair[0]) {
            return Err(bad(&all, "shares that rise with the length"));
        }
        if shares.last().is_some_and(|&last| last != 1.0) {
            return Err(bad(&all, "its last share to be 1"));
        }
        if lengths.len() < 2 {
            return Err(bad(&all, "at least two points"));
        }
        let slopes = slopes(&lengths, &shares);
        Ok(Some(Self {
            lengths,
            shares,
            slopes,
            top: straight(settings, "top_surface_pattern", true),
            bottom: straight(settings, "bottom_surface_pattern", true),
            solid: straight(settings, "internal_solid_infill_pattern", true),
        }))
    }

    /// Whether the moves of a fill line are thinned by their length: a
    /// line of solid fill of a kind set to straight lines, and not one
    /// of the loops a narrow area is filled with.
    pub fn applies(&self, path: &ExtrusionPath) -> bool {
        !path.looped()
            && match path.role {
                Role::TopSolidInfill => self.top,
                Role::BottomSurface => self.bottom,
                Role::SolidInfill => self.solid,
                _ => false,
            }
    }

    /// The share of its plastic a move `length` mm long is laid with
    /// (`flow_comp_model`): all of it past the model's last length.
    pub fn share(&self, length: f64) -> f64 {
        let last = self.lengths.len() - 1;
        if length == 0.0 || length > self.lengths[last] {
            return 1.0;
        }
        if length <= self.lengths[0] {
            return self.shares[0];
        }
        if length >= self.lengths[last] {
            return self.shares[last];
        }
        // Between the two points it lies between, a cubic that meets
        // each at the curve's slope there (`interpolate`).
        let index = self.lengths.partition_point(|&at| at < length) - 1;
        let step = self.lengths[index + 1] - self.lengths[index];
        let t = (length - self.lengths[index]) / step;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.shares[index]
            + h10 * step * self.slopes[index]
            + h01 * self.shares[index + 1]
            + h11 * step * self.slopes[index + 1]
    }
}

/// The slope of the curve at each of its points, so chosen that it
/// never overshoots between two (`computePCHIP`): at an end, the slope
/// of the stretch beside it; between two stretches that rise or fall
/// together, their harmonic mean weighted by their lengths; flat where
/// they do not.
fn slopes(x: &[f64], y: &[f64]) -> Vec<f64> {
    let count = x.len() - 1;
    let step: Vec<f64> = (0..count).map(|i| x[i + 1] - x[i]).collect();
    let rise: Vec<f64> = (0..count).map(|i| (y[i + 1] - y[i]) / step[i]).collect();
    let mut slopes = vec![0.0; count + 1];
    slopes[0] = rise[0];
    slopes[count] = rise[count - 1];
    for i in 1..count {
        if rise[i - 1] * rise[i] > 0.0 {
            let w1 = 2.0 * step[i] + step[i - 1];
            let w2 = step[i] + 2.0 * step[i - 1];
            slopes[i] = (w1 + w2) / (w1 / rise[i - 1] + w2 / rise[i]);
        }
    }
    slopes
}

#[cfg(test)]
mod tests {
    use edge_extrusion::Flow;
    use edge_geom::Point2;

    use super::*;

    fn read(json: &str) -> Result<Option<SmallArea>, ConfigError> {
        SmallArea::read(&Settings::from_json(json).unwrap())
    }

    /// The reference's own model.
    const STOCK: &str = r#""small_area_infill_flow_compensation_model":
        ["0,0", "\n0.2,0.4444", "\n0.4,0.6145", "\n0.6,0.7059", "\n0.8,0.7619",
         "\n1.5,0.8571", "\n2,0.8889", "\n3,0.9231", "\n5,0.9520", "\n10,1"]"#;

    fn stock() -> SmallArea {
        read(&format!(
            r#"{{"small_area_infill_flow_compensation": "1", {STOCK}}}"#
        ))
        .unwrap()
        .unwrap()
    }

    #[test]
    fn it_is_off_unless_asked_for_and_given_a_model() {
        assert_eq!(read("{}").unwrap(), None);
        assert_eq!(
            read(&format!(
                r#"{{"small_area_infill_flow_compensation": "0", {STOCK}}}"#
            ))
            .unwrap(),
            None
        );
        // On with no model is off, as in the reference.
        assert_eq!(
            read(
                r#"{"small_area_infill_flow_compensation": "1",
                    "small_area_infill_flow_compensation_model": []}"#
            )
            .unwrap(),
            None
        );
        assert!(
            read(&format!(
                r#"{{"small_area_infill_flow_compensation": "1", {STOCK}}}"#
            ))
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn the_reference_s_own_defaults_make_a_model() {
        // The model a profile leaves out is the one in the table of the
        // reference's defaults, in the text form it is kept in there.
        let mut settings = edge_config::reference_defaults().clone();
        assert_eq!(SmallArea::read(&settings).unwrap(), None);
        settings.insert(FLAG, edge_config::Value::One("1".into()));
        let read = SmallArea::read(&settings).unwrap().unwrap();
        assert_eq!(read.lengths, stock().lengths);
        assert_eq!(read.shares, stock().shares);
    }

    #[test]
    fn a_short_move_is_thinned_by_the_curve_through_the_model_s_points() {
        let model = stock();
        // At a point, the point's own share.
        for (length, share) in [(0.2, 0.4444), (0.8, 0.7619), (3.0, 0.9231), (10.0, 1.0)] {
            assert!((model.share(length) - share).abs() < 1e-12, "{length}");
        }
        // Past the last length, and for no length at all, everything.
        assert_eq!(model.share(10.001), 1.0);
        assert_eq!(model.share(250.0), 1.0);
        assert_eq!(model.share(0.0), 1.0);
        // Between points the curve rises and never passes the next one.
        let mut before = 0.0;
        for step in 1..=1000 {
            let length = f64::from(step) * 0.01;
            let share = model.share(length);
            assert!(share >= before && share <= 1.0, "{length}: {share}");
            before = share;
        }
        // The cubic the reference draws, worked apart from this code by
        // the formulas of `computePCHIP` and `interpolate`: its slopes
        // are 1.230145 at 0.2 mm and 0.594537 at 0.4 mm.
        for (length, share) in [
            (0.1, 0.246_996_362_9),
            (0.3, 0.545_340_214_5),
            (7.0, 0.972_840_278_9),
        ] {
            assert!((model.share(length) - share).abs() < 1e-9, "{length}");
        }
        // The join between two solid lines 0.42 mm wide laid at 45
        // degrees is 0.533 mm long: 68% of its plastic, which is what
        // the reference's file has (E 0.01115 where it was 0.01639).
        assert!((model.share(0.533) - 0.01115 / 0.01639).abs() < 1e-3);
    }

    #[test]
    fn a_model_the_reference_refuses_is_refused() {
        let with = |model: &str| {
            read(&format!(
                r#"{{"small_area_infill_flow_compensation": "1",
                     "small_area_infill_flow_compensation_model": {model}}}"#
            ))
        };
        assert!(with(r#"["0,0", "1,0.5", "10,1"]"#).unwrap().is_some());
        // Spaces, line breaks and the semicolons of the settings page.
        assert!(
            with(r#"["0, 0;", "\n 1 , 0.5;", "\n10,1;"]"#)
                .unwrap()
                .is_some()
        );
        // A line with nothing before its comma, or no comma, is passed over.
        assert_eq!(
            with(r#"["0,0", "", " ,3", "7", "1,0.5", "10,1"]"#).unwrap(),
            with(r#"["0,0", "1,0.5", "10,1"]"#).unwrap()
        );
        for refused in [
            // Does not start at nothing.
            r#"["0.1,0", "1,0.5", "10,1"]"#,
            // Lengths that do not rise.
            r#"["0,0", "1,0.5", "1,0.7", "10,1"]"#,
            r#"["0,0", "2,0.5", "1,0.7", "10,1"]"#,
            // Shares that do not rise.
            r#"["0,0", "1,0.5", "2,0.5", "10,1"]"#,
            // Does not end at the whole.
            r#"["0,0", "1,0.5", "10,0.9"]"#,
            // Not numbers.
            r#"["0,0", "1,half", "10,1"]"#,
            r#"["0,0", "one,0.5", "10,1"]"#,
            // One point is no curve.
            r#"["0,1"]"#,
        ] {
            assert!(with(refused).is_err(), "{refused}");
        }
        // Off, a bad model is nobody's business.
        assert_eq!(
            read(
                r#"{"small_area_infill_flow_compensation": "0",
                    "small_area_infill_flow_compensation_model": ["nonsense"]}"#
            )
            .unwrap(),
            None
        );
    }

    fn path(role: Role, part: u32) -> ExtrusionPath {
        ExtrusionPath {
            points: vec![Point2::from_mm(0.0, 0.0), Point2::from_mm(1.0, 0.0)],
            role,
            flow: Flow::new(0.42, 0.2, 0.4).unwrap(),
            in_turn: false,
            part,
        }
    }

    #[test]
    fn only_straight_lines_of_solid_fill_are_compensated() {
        let model = stock();
        for role in [Role::SolidInfill, Role::TopSolidInfill, Role::BottomSurface] {
            assert!(model.applies(&path(role, 1)), "{role:?}");
        }
        for role in [
            Role::SparseInfill,
            Role::BridgeInfill,
            Role::InternalBridge,
            Role::GapFill,
            Role::Ironing,
            Role::Perimeter,
            Role::ExternalPerimeter,
            Role::Support,
        ] {
            assert!(!model.applies(&path(role, 1)), "{role:?}");
        }
        // The loops a narrow area is filled with keep their flow.
        assert!(!model.applies(&path(Role::SolidInfill, ExtrusionPath::LOOPED | 1)));

        // A kind set to a pattern that is not straight lines is left
        // alone, as the reference leaves it, each kind by its own.
        let curled = read(&format!(
            r#"{{"small_area_infill_flow_compensation": "1", {STOCK},
                 "top_surface_pattern": "concentric",
                 "internal_solid_infill_pattern": "monotonicline",
                 "bottom_surface_pattern": "hilbertcurve"}}"#
        ))
        .unwrap()
        .unwrap();
        assert!(!curled.applies(&path(Role::TopSolidInfill, 1)));
        assert!(curled.applies(&path(Role::SolidInfill, 1)));
        assert!(!curled.applies(&path(Role::BottomSurface, 1)));
    }
    /// What the writer feeds for each move of one layer of fill: the
    /// feature it is laid as, the move's length in mm and its E.
    fn fed(paths: &[ExtrusionPath], model: Option<SmallArea>) -> Vec<(String, f64, f64)> {
        let settings = crate::PrintSettings {
            small_area: model,
            slow_down_for_layer_cooling: false,
            ..crate::PrintSettings::default()
        };
        let layers = [crate::Layer {
            region: None,
            z: 0.2,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            wipe: None,
            tower: &[],
            tower_z: None,
            tower_first: false,
            around: &[],
            support: &[],
            loops: &[],
            islands: &[],
            paths,
            shelter: &[],
            below: &[],
        }];
        let mut out = Vec::new();
        crate::write(&mut out, &layers, &settings).unwrap();
        let text = String::from_utf8(out).unwrap();
        let word = |line: &str, word: char| -> Option<f64> {
            line.split(' ')
                .find_map(|part| part.strip_prefix(word)?.parse().ok())
        };
        let (mut feature, mut at) = (String::new(), None::<(f64, f64)>);
        let mut moves = Vec::new();
        for line in text.lines() {
            if let Some(name) = line.strip_prefix(";TYPE:") {
                feature = name.to_owned();
            }
            if !line.starts_with("G1 ") {
                continue;
            }
            let to = match (word(line, 'X'), word(line, 'Y')) {
                (Some(x), Some(y)) => (x, y),
                _ => continue,
            };
            if let (Some(from), Some(e)) = (at, word(line, 'E')) {
                moves.push((feature.clone(), (to.0 - from.0).hypot(to.1 - from.1), e));
            }
            at = Some(to);
        }
        moves
    }

    /// Two lines 20 mm long joined by half a millimetre, the way solid
    /// lines are joined along a wall.
    fn zigzag(role: Role, part: u32) -> ExtrusionPath {
        ExtrusionPath {
            points: [(10.0, 10.0), (30.0, 10.0), (30.0, 10.5), (10.0, 10.5)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            ..path(role, part)
        }
    }

    #[test]
    fn the_writer_thins_each_short_move_of_a_solid_line_and_no_other() {
        let model = stock();
        let join = model.share(0.5);
        assert!((0.6..0.7).contains(&join));
        for role in [Role::SolidInfill, Role::TopSolidInfill, Role::BottomSurface] {
            let plain = fed(&[zigzag(role, 1)], None);
            let thinned = fed(&[zigzag(role, 1)], Some(model.clone()));
            assert_eq!(plain.len(), 3, "{role:?}");
            assert_eq!(thinned.len(), 3, "{role:?}");
            // The long lines are laid as they were, the join between
            // them at the curve's share for its length.
            assert_eq!(thinned[0], plain[0], "{role:?}");
            assert_eq!(thinned[2], plain[2], "{role:?}");
            assert!((plain[1].1 - 0.5).abs() < 1e-9);
            assert!(
                (thinned[1].2 / plain[1].2 - join).abs() < 2e-3,
                "{role:?}: {} of {}",
                thinned[1].2,
                plain[1].2
            );
        }
        // Sparse fill, and the loops of a narrow area, keep their flow
        // however short their moves.
        for other in [
            zigzag(Role::SparseInfill, 1),
            zigzag(Role::SolidInfill, ExtrusionPath::LOOPED | 1),
        ] {
            assert_eq!(
                fed(std::slice::from_ref(&other), Some(model.clone())),
                fed(std::slice::from_ref(&other), None)
            );
        }
        // A solid line laid after one that is not is thinned all the
        // same, and one that is not after one that is, is not.
        let mixed = [zigzag(Role::SparseInfill, 1), {
            let mut solid = zigzag(Role::SolidInfill, 2);
            for point in &mut solid.points {
                point.y += 5_000_000;
            }
            solid
        }];
        let (plain, thinned) = (fed(&mixed, None), fed(&mixed, Some(model.clone())));
        let short: Vec<(f64, f64)> = plain
            .iter()
            .zip(&thinned)
            .filter(|(plain, _)| plain.1 < 1.0)
            .map(|(plain, thinned)| (plain.2, thinned.2))
            .collect();
        assert_eq!(short.len(), 2);
        let unchanged = short
            .iter()
            .filter(|(plain, thinned)| plain == thinned)
            .count();
        assert_eq!(unchanged, 1, "{short:?}");
        // Less plastic is counted as used, by what the join lost.
        let used = |model: Option<SmallArea>| -> f64 {
            fed(&[zigzag(Role::SolidInfill, 1)], model)
                .iter()
                .map(|fed| fed.2)
                .sum()
        };
        assert!(used(Some(model)) < used(None));
    }
}
