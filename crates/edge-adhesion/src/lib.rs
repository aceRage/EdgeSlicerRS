//! What is printed around a part to help it stick: the skirt, loops at a
//! distance that prime the nozzle, and the brim, loops against the first
//! layer's outline that hold it down.
//!
//! After the reference's `Print::_make_skirt` and Brim.cpp. Not done: the
//! automatic brim, whose width the reference works out from a part's
//! height, footprint and material; brims inside holes; brim ears; draft
//! shields; a skirt per object. A setting that asks for one of these is
//! listed in [`AdhesionSettings::unsupported`].

use edge_clip::{ClipError, Join};
use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionLoop, Flow, FlowError, LineWidths, Role};
use edge_geom::{ExPolygon, Point2, Polygon, convex_hull, scaled, unscaled};

#[derive(Debug, thiserror::Error)]
pub enum AdhesionError {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error(transparent)]
    Flow(#[from] FlowError),
}

/// The settings skirt and brim depend on. Defaults are the reference's
/// (PrintConfig.cpp).
#[derive(Clone, Debug, PartialEq)]
pub struct AdhesionSettings {
    pub skirt_loops: usize,
    /// Gap between the part (or its brim) and the innermost skirt loop, in mm.
    pub skirt_distance: f64,
    /// How many layers the skirt is printed on.
    pub skirt_height: usize,
    /// Where the skirt starts, in degrees anticlockwise from +X as seen
    /// from its middle.
    pub skirt_start_angle: f64,
    /// Width of the brim around the outside of the part in mm; zero for none.
    pub brim_width: f64,
    /// Gap between the part and its brim, in mm, so it breaks off cleanly.
    pub brim_object_gap: f64,
    pub line_width: FloatOrPercent,
    pub initial_layer_line_width: FloatOrPercent,
    pub inner_wall_line_width: FloatOrPercent,
    pub nozzle_diameter: f64,
    /// Settings present in the input that are not honoured, each with what
    /// is done instead.
    pub unsupported: Vec<String>,
}

impl Default for AdhesionSettings {
    fn default() -> Self {
        Self {
            skirt_loops: 1,
            skirt_distance: 2.0,
            skirt_height: 1,
            skirt_start_angle: -135.0,
            brim_width: 0.0,
            brim_object_gap: 0.0,
            line_width: FloatOrPercent::Value(0.4),
            initial_layer_line_width: FloatOrPercent::Value(0.4),
            inner_wall_line_width: FloatOrPercent::Value(0.4),
            nozzle_diameter: 0.4,
            unsupported: Vec::new(),
        }
    }
}

impl AdhesionSettings {
    /// Reads what is present; anything absent keeps its default.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let mut unsupported = Vec::new();
        let count = |key: &str, default: usize| -> Result<usize, ConfigError> {
            settings.integer(key)?.map_or(Ok(default), |found| {
                usize::try_from(found).map_err(|_| ConfigError::BadValue {
                    key: key.into(),
                    found: found.to_string(),
                    expected: "zero or more",
                })
            })
        };
        let width = |key: &str, default| {
            settings
                .float_or_percent(key)
                .map(|found| found.unwrap_or(default))
        };
        let wanted = settings.float("brim_width")?.unwrap_or(defaults.brim_width);
        let brim_width = match settings.text("brim_type") {
            // Unset, the reference's brim is automatic too; only worth a
            // note when a width was given.
            None if wanted > 0.0 && settings.get("brim_width").is_some() => {
                unsupported.push(
                    "brim_width without brim_type: the default is an automatic brim, which is not \
                     worked out, so none is added"
                        .into(),
                );
                0.0
            }
            None => 0.0,
            Some("outer_only") => wanted,
            Some("no_brim") => 0.0,
            Some("outer_and_inner") => {
                unsupported.push(
                    "brim_type \"outer_and_inner\": the brim goes round the outside only".into(),
                );
                wanted
            }
            Some("auto_brim") => {
                unsupported.push(
                    "brim_type \"auto_brim\": automatic brims are not worked out, so none is added"
                        .into(),
                );
                0.0
            }
            Some(other) => {
                unsupported.push(format!("brim_type \"{other}\": no brim is added"));
                0.0
            }
        };
        if settings
            .text("draft_shield")
            .is_some_and(|v| v != "disabled")
        {
            unsupported.push("draft_shield: the skirt keeps to skirt_height".into());
        }
        if settings.text("skirt_type").is_some_and(|v| v != "combined") {
            unsupported.push("skirt_type: one skirt goes round everything".into());
        }
        if settings.float("min_skirt_length")?.is_some_and(|v| v > 0.0) {
            unsupported.push("min_skirt_length: the skirt has skirt_loops loops".into());
        }
        Ok(Self {
            skirt_loops: count("skirt_loops", defaults.skirt_loops)?,
            skirt_distance: settings
                .float("skirt_distance")?
                .unwrap_or(defaults.skirt_distance),
            skirt_height: count("skirt_height", defaults.skirt_height)?,
            skirt_start_angle: settings
                .float("skirt_start_angle")?
                .unwrap_or(defaults.skirt_start_angle),
            brim_width,
            brim_object_gap: settings
                .float("brim_object_gap")?
                .unwrap_or(defaults.brim_object_gap),
            line_width: width("line_width", defaults.line_width)?,
            initial_layer_line_width: width(
                "initial_layer_line_width",
                defaults.initial_layer_line_width,
            )?,
            inner_wall_line_width: width("inner_wall_line_width", defaults.inner_wall_line_width)?,
            nozzle_diameter: settings
                .floats("nozzle_diameter")?
                .and_then(|nozzles| nozzles.first().copied().flatten())
                .unwrap_or(defaults.nozzle_diameter),
            unsupported,
        })
    }

    fn widths(&self) -> LineWidths {
        LineWidths {
            line_width: self.line_width,
            initial_layer_line_width: self.initial_layer_line_width,
            nozzle_diameter: self.nozzle_diameter,
        }
    }

    /// The skirt's line on a layer `height` mm thick: the first layer's
    /// width, or else the general one (`Print::skirt_flow`).
    pub fn skirt_flow(&self, height: f64) -> Result<Flow, FlowError> {
        Flow::from_widths(
            &self.widths(),
            FloatOrPercent::Value(0.0),
            Flow::auto_width(self.nozzle_diameter),
            height,
            true,
        )
    }

    /// The brim's line: the first layer's width, or else the inner wall's
    /// (`Print::brim_flow`).
    pub fn brim_flow(&self, height: f64) -> Result<Flow, FlowError> {
        Flow::from_widths(
            &self.widths(),
            self.inner_wall_line_width,
            Flow::auto_width(self.nozzle_diameter),
            height,
            true,
        )
    }

    /// Whether the layer with this index has a skirt.
    pub fn has_skirt(&self, layer: usize) -> bool {
        self.skirt_loops > 0 && layer < self.skirt_height
    }
}

/// The brim around a first layer, outermost loop first. Each island's
/// outline, holes filled in, is pushed out by the gap and then a line at a
/// time; islands close together share the loops between them. A
/// difference from the reference, which fills the ring from both its edges:
/// the loop count for an island by itself is the same.
pub fn brim(
    first_layer: &[ExPolygon],
    settings: &AdhesionSettings,
    height: f64,
) -> Result<Vec<ExtrusionLoop>, AdhesionError> {
    let flow = settings.brim_flow(height)?;
    // A whole number of line pairs, as the reference rounds it.
    let pairs = (settings.brim_width / flow.spacing / 2.0).floor() as usize;
    if pairs == 0 || first_layer.is_empty() {
        return Ok(Vec::new());
    }
    let filled: Vec<ExPolygon> = first_layer
        .iter()
        .map(|island| ExPolygon {
            contour: island.contour.clone(),
            holes: Vec::new(),
        })
        .collect();
    let join = Join::Round {
        tolerance: scaled(0.0125),
    };
    let mut loops = Vec::new();
    for index in (0..2 * pairs).rev() {
        let distance = settings.brim_object_gap + (index as f64 + 0.5) * flow.spacing;
        for region in edge_clip::offset(&filled, scaled(distance), join)? {
            let rings = std::iter::once((region.contour, false))
                .chain(region.holes.into_iter().map(|hole| (hole, true)));
            for (polygon, is_hole) in rings {
                loops.push(ExtrusionLoop {
                    polygon,
                    role: Role::Brim,
                    is_hole,
                    depth: index,
                    flow,
                });
            }
        }
    }
    Ok(loops)
}

/// The skirt for one layer, outermost loop first: loops around the convex
/// hull of everything in `outlines` (the layers the skirt stands beside)
/// and `brim`.
pub fn skirt(
    outlines: &[&[ExPolygon]],
    brim: &[ExtrusionLoop],
    settings: &AdhesionSettings,
    height: f64,
) -> Result<Vec<ExtrusionLoop>, AdhesionError> {
    let flow = settings.skirt_flow(height)?;
    let points = outlines
        .iter()
        .flat_map(|layer| layer.iter())
        .flat_map(|island| &island.contour.points)
        .chain(brim.iter().flat_map(|line| &line.polygon.points))
        .copied();
    let hull = convex_hull(points);
    if settings.skirt_loops == 0 || hull.points.len() < 3 {
        return Ok(Vec::new());
    }
    let hull = [ExPolygon {
        contour: hull,
        holes: Vec::new(),
    }];
    let join = Join::Round {
        tolerance: scaled(0.1),
    };
    let mut loops = Vec::new();
    for index in (0..settings.skirt_loops).rev() {
        // The innermost loop's centre is half a line beyond the gap.
        let distance = settings.skirt_distance + (index as f64 + 0.5) * flow.spacing;
        let grown = edge_clip::offset(&hull, scaled(distance), join)?;
        let grown = edge_clip::simplify(&grown, scaled(0.05))?;
        let Some(region) = grown.into_iter().next() else {
            continue;
        };
        let mut polygon = region.contour;
        if loops.is_empty() {
            start_at_angle(&mut polygon, settings.skirt_start_angle);
        }
        loops.push(ExtrusionLoop {
            polygon,
            role: Role::Skirt,
            is_hole: false,
            depth: index,
            flow,
        });
    }
    Ok(loops)
}

/// Turns a loop so it starts at the corner nearest the point `degrees`
/// round from +X on the circle through its bounding box's corners
/// (`Skirt::find_start_point`).
fn start_at_angle(polygon: &mut Polygon, degrees: f64) {
    let points = &polygon.points;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for point in points {
        (min_x, max_x) = (min_x.min(point.x), max_x.max(point.x));
        (min_y, max_y) = (min_y.min(point.y), max_y.max(point.y));
    }
    let (centre_x, centre_y) = (unscaled(min_x + max_x) / 2.0, unscaled(min_y + max_y) / 2.0);
    let radius = (unscaled(max_x - min_x) / 2.0).hypot(unscaled(max_y - min_y) / 2.0);
    let angle = degrees.to_radians();
    let target = Point2::from_mm(
        centre_x + radius * angle.cos(),
        centre_y + radius * angle.sin(),
    );
    let far = |point: &Point2| {
        let (dx, dy) = ((point.x - target.x) as f64, (point.y - target.y) as f64);
        dx * dx + dy * dy
    };
    let first = (0..points.len())
        .min_by(|&a, &b| far(&points[a]).total_cmp(&far(&points[b])))
        .unwrap_or(0);
    polygon.points.rotate_left(first);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn settings() -> AdhesionSettings {
        AdhesionSettings {
            line_width: FloatOrPercent::Value(0.45),
            initial_layer_line_width: FloatOrPercent::Value(0.5),
            ..AdhesionSettings::default()
        }
    }

    /// Extent of a loop in mm: min x, min y, max x, max y.
    fn extent(polygon: &Polygon) -> [f64; 4] {
        polygon.points.iter().fold(
            [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
            |[min_x, min_y, max_x, max_y], p| {
                let (x, y) = (unscaled(p.x), unscaled(p.y));
                [min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)]
            },
        )
    }

    #[test]
    fn the_skirt_stands_off_the_hull_of_everything() {
        let parts = [square(0.0, 0.0, 10.0), square(30.0, 0.0, 10.0)];
        let two = AdhesionSettings {
            skirt_loops: 2,
            ..settings()
        };
        let loops = skirt(&[&parts], &[], &two, 0.2).unwrap();
        assert_eq!(loops.len(), 2);
        let flow = two.skirt_flow(0.2).unwrap();
        assert_eq!((flow.width, loops[0].role), (0.5, Role::Skirt));
        // Outermost first; the inner loop's centre is the gap and half a
        // line from the parts, the outer one a line further.
        let inner = 2.0 + flow.spacing / 2.0;
        for (line, distance) in [(&loops[1], inner), (&loops[0], inner + flow.spacing)] {
            let [min_x, min_y, max_x, max_y] = extent(&line.polygon);
            for (found, expected) in [
                (min_x, -distance),
                (min_y, -distance),
                (max_x, 40.0 + distance),
                (max_y, 10.0 + distance),
            ] {
                assert!((found - expected).abs() < 0.11, "{found} for {expected}");
            }
            assert!(line.polygon.is_ccw());
        }
        // It starts at the corner the start angle points at: front left.
        let start = loops[0].polygon.points[0];
        assert!(
            unscaled(start.x) < 0.5 && unscaled(start.y) < 0.5,
            "{start:?}"
        );

        let none = AdhesionSettings {
            skirt_loops: 0,
            ..settings()
        };
        assert!(skirt(&[&parts], &[], &none, 0.2).unwrap().is_empty());
        assert!(skirt(&[&[]], &[], &two, 0.2).unwrap().is_empty());
        assert!(two.has_skirt(0) && !two.has_skirt(1) && !none.has_skirt(0));
    }

    #[test]
    fn the_brim_fills_its_width_with_whole_pairs_of_lines() {
        let part = [square(0.0, 0.0, 10.0)];
        let wide = AdhesionSettings {
            brim_width: 3.0,
            brim_object_gap: 0.1,
            ..settings()
        };
        let flow = wide.brim_flow(0.2).unwrap();
        let loops = brim(&part, &wide, 0.2).unwrap();
        let count = 2 * (3.0 / flow.spacing / 2.0).floor() as usize;
        assert_eq!(loops.len(), count);
        assert!(loops.iter().all(|line| line.role == Role::Brim));
        // Outermost first, the innermost half a line beyond the gap.
        let reach = |line: &ExtrusionLoop| -extent(&line.polygon)[0];
        assert!(
            loops
                .windows(2)
                .all(|pair| reach(&pair[0]) > reach(&pair[1]))
        );
        let nearest = reach(loops.last().unwrap());
        assert!(
            (nearest - (0.1 + flow.spacing / 2.0)).abs() < 0.02,
            "{nearest}"
        );

        // Too narrow for a pair of lines, or no brim asked for: nothing.
        let narrow = AdhesionSettings {
            brim_width: 0.6,
            ..settings()
        };
        assert!(brim(&part, &narrow, 0.2).unwrap().is_empty());
        assert!(brim(&part, &settings(), 0.2).unwrap().is_empty());

        // The skirt goes round the brim, not just the part.
        let around = skirt(&[&part], &loops, &wide, 0.2).unwrap();
        let outer = reach(&loops[0]);
        assert!(reach(&around[0]) > outer + 2.0);
    }

    #[test]
    fn two_parts_close_together_share_a_brim() {
        let parts = [square(0.0, 0.0, 10.0), square(11.0, 0.0, 10.0)];
        let wide = AdhesionSettings {
            brim_width: 3.0,
            ..settings()
        };
        let loops = brim(&parts, &wide, 0.2).unwrap();
        // The outermost loop goes round both.
        let [min_x, _, max_x, _] = extent(&loops[0].polygon);
        assert!(min_x < -2.0 && max_x > 23.0, "{min_x} to {max_x}");
        // No loop runs over either part.
        let covered = edge_clip::intersection(
            &parts,
            &loops
                .iter()
                .filter(|line| !line.is_hole)
                .map(|line| ExPolygon {
                    contour: line.polygon.clone(),
                    holes: Vec::new(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let area: f64 = covered.iter().map(ExPolygon::area_mm2).sum();
        assert!((area - 200.0).abs() < 1e-6, "{area}");
    }

    #[test]
    fn settings_say_what_is_not_done() {
        let read = |json: &str| {
            AdhesionSettings::from_settings(&Settings::from_json(json).unwrap()).unwrap()
        };
        assert_eq!(read("{}"), AdhesionSettings::default());
        let outer = read(
            r#"{"skirt_loops": "2", "skirt_distance": "3", "skirt_height": "2", "brim_type": "outer_only",
                "brim_width": "5", "brim_object_gap": "0.1", "nozzle_diameter": ["0.4", "0.4"],
                "initial_layer_line_width": "0.5"}"#,
        );
        assert_eq!((outer.skirt_loops, outer.skirt_distance), (2, 3.0));
        assert_eq!((outer.brim_width, outer.brim_object_gap), (5.0, 0.1));
        assert!(outer.has_skirt(1) && !outer.has_skirt(2));
        assert!(outer.unsupported.is_empty());

        let auto =
            read(r#"{"brim_type": "auto_brim", "brim_width": "5", "draft_shield": "enabled"}"#);
        assert_eq!(auto.brim_width, 0.0);
        assert_eq!(auto.unsupported.len(), 2);
        assert_eq!(
            read(r#"{"brim_type": "no_brim", "brim_width": "5"}"#).brim_width,
            0.0
        );
        assert_eq!(
            read(r#"{"brim_type": "brim_ears", "brim_width": "5"}"#)
                .unsupported
                .len(),
            1
        );
    }
}
