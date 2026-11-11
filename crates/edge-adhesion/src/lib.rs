//! What is printed around a part to help it stick: the skirt, loops at a
//! distance that prime the nozzle, and the brim, loops against the first
//! layer's outline that hold it down.
//!
//! After the reference's `Print::_make_skirt` and Brim.cpp. Not done:
//! brims inside holes, brim ears, draft shields and a skirt per object. A
//! setting that asks for one of these is listed in
//! [`AdhesionSettings::unsupported`].

use edge_clip::{ClipError, Join};
use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Flow, FlowError, LineWidths, Role};
use edge_geom::{ExPolygon, Point2, Polygon, convex_hull, scaled, unscaled};

#[derive(Debug, thiserror::Error)]
pub enum AdhesionError {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error(transparent)]
    Flow(#[from] FlowError),
}

/// Which parts get a brim (`brim_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrimKind {
    None,
    /// Around the outside of every part, `brim_width` wide.
    Outer,
    /// Around the parts that need one, as wide as each needs: see [`auto_width`].
    Auto,
}

/// One object on the bed, as the brim sees it.
#[derive(Clone, Copy, Debug)]
pub struct Footing<'a> {
    /// The object's first layer.
    pub footprint: &'a [ExPolygon],
    /// How tall the object is, in mm.
    pub height: f64,
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
    pub brim: BrimKind,
    /// Width of an `Outer` brim in mm.
    pub brim_width: f64,
    /// The filament's material, such as "PLA", and the fastest the print
    /// goes in mm/s: both weigh on how wide an automatic brim is.
    pub filament_type: String,
    pub max_speed: f64,
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
            brim: BrimKind::Auto,
            brim_width: 0.0,
            filament_type: String::new(),
            max_speed: 100.0,
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
        let brim = match settings.text("brim_type") {
            None | Some("auto_brim") => BrimKind::Auto,
            Some("outer_only") => BrimKind::Outer,
            Some("no_brim") => BrimKind::None,
            Some("outer_and_inner") => {
                unsupported.push(
                    "brim_type \"outer_and_inner\": the brim goes round the outside only".into(),
                );
                BrimKind::Outer
            }
            Some(other) => {
                unsupported.push(format!("brim_type \"{other}\": no brim is added"));
                BrimKind::None
            }
        };
        // The fastest of the speeds the reference looks at (Model.cpp
        // `setPrintSpeedTable`), each at its default when not given.
        let mut max_speed: f64 = 0.0;
        for (key, default) in [
            ("inner_wall_speed", 60.0),
            ("outer_wall_speed", 60.0),
            ("sparse_infill_speed", 100.0),
            ("internal_solid_infill_speed", 100.0),
            ("top_surface_speed", 100.0),
            ("support_speed", 80.0),
        ] {
            let speed = settings
                .floats(key)?
                .and_then(|values| values.first().copied().flatten())
                .unwrap_or(default);
            max_speed = max_speed.max(speed);
        }
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
            brim,
            brim_width: settings.float("brim_width")?.unwrap_or(defaults.brim_width),
            filament_type: settings
                .texts("filament_type")
                .and_then(|types| types.first())
                .cloned()
                .unwrap_or_default(),
            max_speed,
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

/// Area in mm², centre, and second moments of area about that centre
/// (about the X axis, then the Y axis, in mm⁴) of one region.
fn moments(region: &ExPolygon) -> Option<(f64, [f64; 2], [f64; 2])> {
    // Sums over one ring taken anticlockwise: its area, its first moments
    // and its second moments about the origin.
    let ring = |polygon: &Polygon| {
        let points = &polygon.points;
        let (mut doubled, mut first, mut second) = (0.0, [0.0; 2], [0.0; 2]);
        for index in 0..points.len() {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            let (x1, y1) = (unscaled(a.x), unscaled(a.y));
            let (x2, y2) = (unscaled(b.x), unscaled(b.y));
            let cross = x1 * y2 - x2 * y1;
            doubled += cross;
            first[0] += (x1 + x2) * cross / 6.0;
            first[1] += (y1 + y2) * cross / 6.0;
            second[0] += (y1 * y1 + y1 * y2 + y2 * y2) * cross / 12.0;
            second[1] += (x1 * x1 + x1 * x2 + x2 * x2) * cross / 12.0;
        }
        // A ring given clockwise comes out negated throughout.
        let sign = if doubled < 0.0 { -1.0 } else { 1.0 };
        (
            sign * doubled / 2.0,
            first.map(|sum| sign * sum),
            second.map(|sum| sign * sum),
        )
    };
    if region.contour.points.len() < 3 {
        return None;
    }
    let (mut area, mut first, mut second) = ring(&region.contour);
    for hole in &region.holes {
        let (hole_area, hole_first, hole_second) = ring(hole);
        area -= hole_area;
        for axis in 0..2 {
            first[axis] -= hole_first[axis];
            second[axis] -= hole_second[axis];
        }
    }
    if area <= 0.0 {
        return None;
    }
    let centre = [first[0] / area, first[1] / area];
    // Moved from the origin to the region's own centre.
    Some((
        area,
        centre,
        [
            second[0] - centre[1] * centre[1] * area,
            second[1] - centre[0] * centre[0] * area,
        ],
    ))
}

/// Extent of some regions' contours in mm: width, then depth.
fn spread<'a>(regions: impl IntoIterator<Item = &'a ExPolygon>) -> [f64; 2] {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for point in regions
        .into_iter()
        .flat_map(|region| &region.contour.points)
    {
        (min_x, max_x) = (min_x.min(point.x), max_x.max(point.x));
        (min_y, max_y) = (min_y.min(point.y), max_y.max(point.y));
    }
    if min_x > max_x {
        return [0.0, 0.0];
    }
    [unscaled(max_x - min_x), unscaled(max_y - min_y)]
}

/// How wide a brim an object needs, in mm, by the reference's rule of
/// thumb (Brim.cpp `configBrimWidthByVolumeGroups`): wider the taller the
/// object stands on a footprint that resists tipping little, the faster it
/// is printed, and the longer it is across, since a long first layer pulls
/// harder at its ends as it cools. Under 5 mm it is not worth having and
/// is zero; 18 mm is the most.
///
/// One difference: the reference works this out for each group of an
/// object's parts that touch on the first layer; here an object is one group.
pub fn auto_width(object: &Footing<'_>, settings: &AdhesionSettings) -> f64 {
    let parts: Vec<_> = object.footprint.iter().filter_map(moments).collect();
    let total: f64 = parts.iter().map(|(area, ..)| area).sum();
    if parts.is_empty() || total <= 0.0 {
        return 0.0;
    }
    // Second moments of the whole footprint about its common centre.
    let centre: [f64; 2] =
        std::array::from_fn(|axis| parts.iter().map(|(a, c, _)| a * c[axis]).sum::<f64>() / total);
    let (mut about_x, mut about_y) = (0.0, 0.0);
    for (area, own, second) in &parts {
        about_x += second[0] + area * (own[1] - centre[1]).powi(2);
        about_y += second[1] + area * (own[0] - centre[0]).powi(2);
    }
    let [width, depth] = spread(object.footprint);
    let across = width.hypot(depth);
    // How far a first layer of this material reaches before shrinkage
    // matters, and how well it sticks (Model.cpp `getThermalLength`,
    // Brim.cpp `getadhesionCoeff`).
    let (reach, grip) = match settings.filament_type.as_str() {
        "ABS" | "PA-CF" | "PET-CF" => (100.0, 1.0),
        "PC" => (40.0, 1.0),
        "TPU" => (1000.0, 0.5),
        "PETG" | "PCTG" => (200.0, 2.0),
        _ => (200.0, 1.0),
    };
    let height = object.height;
    let tipping = (height / about_x * depth).max(height / about_y * width) * height / 1920.0;
    let shrinking = across * 8.0 / reach * height.min(30.0) / 30.0;
    let wanted = grip
        * (tipping * settings.max_speed)
            .max(shrinking)
            .min(18.0)
            .min(1.5 * across);
    if wanted < 5.0 && wanted < 1.5 * across {
        0.0
    } else {
        wanted.min(18.0)
    }
}

/// The brim's width for one island of an object whose brim is `width` mm,
/// as Brim.cpp `outer_inner_brim_area` settles it. Widths are whole pairs
/// of lines `spacing` apart.
fn island_width(
    island: &ExPolygon,
    width: f64,
    object_height: f64,
    automatic: bool,
    spacing: f64,
) -> f64 {
    let pairs = |width: f64| (width / spacing / 2.0).floor() * spacing * 2.0;
    let mut width = pairs(width);
    if automatic {
        // A tall object with no brim still gets one round an island too
        // slight to hold on by itself: under 1.1 mm of area per mm of outline.
        let slight = island.area_mm2() / island.contour.perimeter_mm() < 1.1;
        if width < 5.0 && object_height > 10.0 && slight && width < spacing {
            width += pairs(5.0);
        }
        // Never wider than the island is long.
        let [across, deep] = spread([island]);
        width = width.min(across.max(deep));
    }
    pairs(width)
}

/// The brim around the first layer of some objects, one piece of it
/// after another, each outermost loop first.
/// Each island's outline, holes filled in, is given a band of its width
/// beyond the gap; the bands are united, kept off every island, and filled
/// with loops a line apart working in from both edges (Brim.cpp
/// `tryExPolygonOffset`). No brim is laid on `taken`, ground something
/// else stands on, such as the prime tower. The reference keeps the brim
/// off the tower's wall only and lets it cross the tower's own brim; here
/// `taken` is given with that brim, so the two never overlap.
pub fn brim(
    objects: &[Footing<'_>],
    taken: &[ExPolygon],
    settings: &AdhesionSettings,
    height: f64,
) -> Result<Vec<ExtrusionLoop>, AdhesionError> {
    if settings.brim == BrimKind::None {
        return Ok(Vec::new());
    }
    let flow = settings.brim_flow(height)?;
    let automatic = settings.brim == BrimKind::Auto;
    let join = Join::Round {
        tolerance: scaled(0.0125),
    };
    let filled = |island: &ExPolygon| ExPolygon {
        contour: island.contour.clone(),
        holes: Vec::new(),
    };
    let gap = scaled(settings.brim_object_gap);
    let mut bands = Vec::new();
    for object in objects {
        let width = if automatic {
            auto_width(object, settings)
        } else {
            settings.brim_width
        };
        for island in object.footprint {
            let width = island_width(island, width, object.height, automatic, flow.spacing);
            if width <= 0.0 {
                continue;
            }
            let inner = edge_clip::offset(&[filled(island)], gap, join)?;
            let outer = edge_clip::offset(&inner, scaled(width), join)?;
            bands.extend(edge_clip::difference(&outer, &inner)?);
        }
    }
    if bands.is_empty() {
        return Ok(Vec::new());
    }
    // No brim runs over a part, or closer to one than the gap.
    let islands: Vec<ExPolygon> = objects
        .iter()
        .flat_map(|object| object.footprint)
        .map(filled)
        .collect();
    let mut keep_off = edge_clip::offset(&islands, gap, join)?;
    keep_off.extend(taken.iter().cloned());
    let area = edge_clip::difference(&edge_clip::union_ex(&bands)?, &keep_off)?;

    let detail = scaled(0.0125);
    let spacing = flow.spacing;
    // One piece of the band at a time, so the nozzle finishes the brim
    // of one part before it leaves for the next (the reference lays each
    // object's brim by itself).
    let mut loops = Vec::new();
    for piece in &area {
        let mut left = edge_clip::offset(
            &edge_clip::simplify(std::slice::from_ref(piece), detail)?,
            scaled(-0.5 * spacing),
            join,
        )?;
        // Each pass gives the next line in from every edge: outlines working
        // inward, and the edges nearest the parts working outward.
        let (mut from_outside, mut from_parts) = (Vec::new(), Vec::new());
        let mut pass = 0;
        while !left.is_empty() {
            left = edge_clip::simplify(&left, detail)?;
            for region in &left {
                from_outside.push((region.contour.clone(), false, pass));
                from_parts.extend(region.holes.iter().map(|hole| (hole.clone(), true, pass)));
            }
            let shrunk = edge_clip::offset(&left, scaled(-1.3 * spacing), join)?;
            left = edge_clip::offset(
                &edge_clip::simplify(&shrunk, detail)?,
                scaled(0.3 * spacing),
                join,
            )?;
            pass += 1;
        }
        // Printed from the outside in, ending against the parts.
        from_parts.reverse();
        loops.extend(from_outside.into_iter().chain(from_parts).map(
            |(polygon, is_hole, depth)| ExtrusionLoop {
                polygon,
                role: Role::Brim,
                is_hole,
                depth,
                flow,
            },
        ));
    }
    Ok(loops)
}

/// The skirt for one layer, outermost loop first: loops around the convex
/// hull of everything in `outlines` and `supports` (the layers the skirt
/// stands beside), `brim`, and `also`: the corners of anything else on the
/// bed, such as the prime tower (Print.cpp `_make_skirt`).
pub fn skirt(
    outlines: &[&[ExPolygon]],
    brim: &[ExtrusionLoop],
    supports: &[&[ExtrusionPath]],
    also: &[Point2],
    settings: &AdhesionSettings,
    height: f64,
) -> Result<Vec<ExtrusionLoop>, AdhesionError> {
    let flow = settings.skirt_flow(height)?;
    let points = outlines
        .iter()
        .flat_map(|layer| layer.iter())
        .flat_map(|island| &island.contour.points)
        .chain(brim.iter().flat_map(|line| &line.polygon.points))
        .chain(
            supports
                .iter()
                .flat_map(|layer| layer.iter())
                .flat_map(|line| &line.points),
        )
        .chain(also)
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
        let loops = skirt(&[&parts], &[], &[], &[], &two, 0.2).unwrap();
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
        assert!(
            skirt(&[&parts], &[], &[], &[], &none, 0.2)
                .unwrap()
                .is_empty()
        );
        assert!(skirt(&[&[]], &[], &[], &[], &two, 0.2).unwrap().is_empty());
        assert!(two.has_skirt(0) && !two.has_skirt(1) && !none.has_skirt(0));
    }

    fn rectangle(x: f64, y: f64, width: f64, depth: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [
                    (x, y),
                    (x + width, y),
                    (x + width, y + depth),
                    (x, y + depth),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn outer(width: f64) -> AdhesionSettings {
        AdhesionSettings {
            brim: BrimKind::Outer,
            brim_width: width,
            ..settings()
        }
    }

    #[test]
    fn the_brim_fills_its_width_with_whole_pairs_of_lines() {
        let part = [square(0.0, 0.0, 10.0)];
        let on_bed = [Footing {
            footprint: &part,
            height: 5.0,
        }];
        let wide = AdhesionSettings {
            brim_object_gap: 0.1,
            ..outer(3.0)
        };
        let flow = wide.brim_flow(0.2).unwrap();
        let loops = brim(&on_bed, &[], &wide, 0.2).unwrap();
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
        let furthest = reach(&loops[0]);
        let expected = 0.1 + (count as f64 - 0.5) * flow.spacing;
        assert!(
            (furthest - expected).abs() < 0.02,
            "{furthest} for {expected}"
        );

        // Too narrow for a pair of lines, or no brim asked for: nothing.
        assert!(brim(&on_bed, &[], &outer(0.6), 0.2).unwrap().is_empty());
        let none = AdhesionSettings {
            brim: BrimKind::None,
            ..outer(3.0)
        };
        assert!(brim(&on_bed, &[], &none, 0.2).unwrap().is_empty());

        // The skirt goes round the brim, not just the part.
        let around = skirt(&[&part], &loops, &[], &[], &wide, 0.2).unwrap();
        assert!(reach(&around[0]) > furthest + 2.0);
    }

    #[test]
    fn two_parts_close_together_share_a_brim_that_stays_off_both() {
        let parts = [square(0.0, 0.0, 10.0), square(11.0, 0.0, 10.0)];
        let on_bed = [Footing {
            footprint: &parts,
            height: 5.0,
        }];
        let loops = brim(&on_bed, &[], &outer(3.0), 0.2).unwrap();
        // The outermost loop goes round both.
        let [min_x, _, max_x, _] = extent(&loops[0].polygon);
        assert!(min_x < -2.0 && max_x > 23.0, "{min_x} to {max_x}");
        // No loop has a corner inside either part.
        let inside = |p: &Point2| {
            let (x, y) = (unscaled(p.x), unscaled(p.y));
            y > 0.1 && y < 9.9 && ((x > 0.1 && x < 9.9) || (x > 11.1 && x < 20.9))
        };
        assert!(
            loops
                .iter()
                .all(|line| !line.polygon.points.iter().any(inside))
        );
    }

    #[test]
    fn moments_are_those_of_the_shape() {
        // A 10 x 4 rectangle away from the origin: b h^3 / 12 each way.
        let (area, centre, second) = moments(&rectangle(20.0, 30.0, 10.0, 4.0)).unwrap();
        assert!((area - 40.0).abs() < 1e-9);
        assert!((centre[0] - 25.0).abs() < 1e-9 && (centre[1] - 32.0).abs() < 1e-9);
        assert!((second[0] - 10.0 * 64.0 / 12.0).abs() < 1e-6, "{second:?}");
        assert!((second[1] - 4.0 * 1000.0 / 12.0).abs() < 1e-6, "{second:?}");
        // A square frame is the outer square less the inner one.
        let mut frame = square(0.0, 0.0, 10.0);
        let mut hole = square(3.0, 3.0, 4.0).contour;
        hole.points.reverse();
        frame.holes.push(hole);
        let (area, _, second) = moments(&frame).unwrap();
        assert!((area - 84.0).abs() < 1e-9);
        assert!((second[0] - (10_000.0 - 256.0) / 12.0).abs() < 1e-6);
    }

    #[test]
    fn an_automatic_brim_is_as_wide_as_the_object_needs() {
        let pla = AdhesionSettings {
            filament_type: "PLA".into(),
            ..settings()
        };
        let width = |footprint: &[ExPolygon], height: f64, settings: &AdhesionSettings| {
            auto_width(&Footing { footprint, height }, settings)
        };
        // A low, wide plate holds itself down.
        assert_eq!(width(&[square(0.0, 0.0, 100.0)], 2.0, &pla), 0.0);
        // A tall thin tower tips easily: as wide as allowed, which for a
        // footprint 7 mm across is one and a half times that.
        let tower = [square(0.0, 0.0, 5.0)];
        let needed = width(&tower, 100.0, &pla);
        assert!((needed - 1.5 * 50.0_f64.sqrt()).abs() < 1e-9, "{needed}");
        // A long bar pulls at its ends as it cools: 8 mm for 200 mm of
        // PLA 30 mm tall, twice that for ABS, and twice for PETG's grip.
        let bar = [rectangle(0.0, 0.0, 200.0, 10.0)];
        let across = 200.0_f64.hypot(10.0);
        let of = |material: &str| {
            let settings = AdhesionSettings {
                filament_type: material.into(),
                ..settings()
            };
            width(&bar, 30.0, &settings)
        };
        assert!((of("PLA") - across * 8.0 / 200.0).abs() < 1e-9);
        assert!((of("ABS") - across * 8.0 / 100.0).abs() < 1e-9);
        assert!((of("PETG") - 2.0 * across * 8.0 / 200.0).abs() < 1e-9);
        // Never more than 18 mm: 250 mm of ABS would otherwise ask for 20.
        let abs = AdhesionSettings {
            filament_type: "ABS".into(),
            ..settings()
        };
        assert_eq!(
            width(&[rectangle(0.0, 0.0, 250.0, 20.0)], 200.0, &abs),
            18.0
        );
        // The same bar half as tall needs half the brim, which is too
        // little to bother with.
        assert_eq!(width(&bar, 15.0, &pla), 0.0);

        // Loops come out for the objects that need them and not the others.
        let plate = [square(50.0, 50.0, 60.0)];
        let on_bed = [
            Footing {
                footprint: &tower,
                height: 100.0,
            },
            Footing {
                footprint: &plate,
                height: 2.0,
            },
        ];
        let loops = brim(&on_bed, &[], &pla, 0.2).unwrap();
        assert!(!loops.is_empty());
        // Held to the tower's own 5 mm, in whole pairs of lines.
        let reach = loops
            .iter()
            .map(|line| extent(&line.polygon)[2])
            .fold(0.0, f64::max);
        assert!(reach > 5.0 + 4.0 && reach < 5.0 + 5.0, "reaches {reach}");
    }

    #[test]
    fn a_slight_island_of_a_tall_object_gets_a_brim_anyway() {
        let spacing = 0.5;
        let sliver = rectangle(0.0, 0.0, 30.0, 1.0);
        // No brim by the rule of thumb, but tall and barely holding on.
        assert_eq!(island_width(&sliver, 0.0, 20.0, true, spacing), 5.0);
        assert_eq!(island_width(&sliver, 0.0, 8.0, true, spacing), 0.0);
        assert_eq!(
            island_width(&square(0.0, 0.0, 30.0), 0.0, 20.0, true, spacing),
            0.0
        );
        // A set width is only rounded to pairs of lines.
        assert_eq!(island_width(&sliver, 3.4, 20.0, false, spacing), 3.0);
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
                "initial_layer_line_width": "0.5", "filament_type": ["PETG", "PLA"],
                "outer_wall_speed": "200", "sparse_infill_speed": ["300"]}"#,
        );
        assert_eq!((outer.skirt_loops, outer.skirt_distance), (2, 3.0));
        assert_eq!(
            (outer.brim, outer.brim_width, outer.brim_object_gap),
            (BrimKind::Outer, 5.0, 0.1)
        );
        assert_eq!(
            (outer.filament_type.as_str(), outer.max_speed),
            ("PETG", 300.0)
        );
        assert!(outer.has_skirt(1) && !outer.has_skirt(2));
        assert!(outer.unsupported.is_empty());

        let auto =
            read(r#"{"brim_type": "auto_brim", "brim_width": "5", "draft_shield": "enabled"}"#);
        assert_eq!((auto.brim, auto.unsupported.len()), (BrimKind::Auto, 1));
        assert_eq!(read(r#"{"brim_type": "no_brim"}"#).brim, BrimKind::None);
        let ears = read(r#"{"brim_type": "brim_ears", "brim_width": "5"}"#);
        assert_eq!((ears.brim, ears.unsupported.len()), (BrimKind::None, 1));
    }

    #[test]
    fn each_parts_brim_is_laid_whole_before_the_next() {
        // Two squares too far apart for their brims to meet.
        let (left, right) = ([square(0.0, 0.0, 20.0)], [square(60.0, 0.0, 20.0)]);
        let on_bed = [
            Footing {
                footprint: &left,
                height: 10.0,
            },
            Footing {
                footprint: &right,
                height: 10.0,
            },
        ];
        let settings = AdhesionSettings {
            brim: BrimKind::Outer,
            brim_width: 5.0,
            ..AdhesionSettings::default()
        };
        let loops = brim(&on_bed, &[], &settings, 0.2).unwrap();
        let sides: Vec<bool> = loops
            .iter()
            .map(|line| unscaled(line.polygon.points[0].x) > 40.0)
            .collect();
        assert!(sides.len() >= 4 && sides.contains(&true) && sides.contains(&false));
        // One change of side in all: every loop of one, then of the other.
        assert_eq!(
            sides.windows(2).filter(|pair| pair[0] != pair[1]).count(),
            1
        );
        // And each part's loops still work in from the outside.
        let own: Vec<f64> = loops
            .iter()
            .zip(&sides)
            .filter(|(line, side)| !**side && !line.is_hole)
            .map(|(line, _)| line.length_mm())
            .collect();
        assert!(own.windows(2).all(|pair| pair[0] > pair[1]));
    }

    #[test]
    fn brim_and_skirt_give_way_to_what_else_is_on_the_bed() {
        let part = [square(0.0, 0.0, 20.0)];
        let on_bed = [Footing {
            footprint: &part,
            height: 10.0,
        }];
        let settings = AdhesionSettings {
            brim: BrimKind::Outer,
            brim_width: 5.0,
            skirt_loops: 1,
            skirt_distance: 2.0,
            ..AdhesionSettings::default()
        };
        // A tower whose ground reaches into where the brim would be.
        let tower = [square(22.0, 5.0, 10.0)];
        let free = brim(&on_bed, &[], &settings, 0.2).unwrap();
        let clear = brim(&on_bed, &tower, &settings, 0.2).unwrap();
        // How many points along the lines, a quarter of a millimetre
        // apart, lie on the tower's ground.
        let reach = |loops: &[ExtrusionLoop]| {
            let mut inside = 0;
            for line in loops {
                let points = &line.polygon.points;
                for (index, from) in points.iter().enumerate() {
                    let to = points[(index + 1) % points.len()];
                    let (ax, ay) = (unscaled(from.x), unscaled(from.y));
                    let (bx, by) = (unscaled(to.x), unscaled(to.y));
                    let steps = ((bx - ax).hypot(by - ay) / 0.25).ceil().max(1.0) as usize;
                    for step in 0..steps {
                        let along = step as f64 / steps as f64;
                        let (x, y) = (ax + (bx - ax) * along, ay + (by - ay) * along);
                        if x > 22.01 && x < 31.99 && y > 5.01 && y < 14.99 {
                            inside += 1;
                        }
                    }
                }
            }
            inside
        };
        assert!(reach(&free) > 0, "the brim would cross the tower's ground");
        assert_eq!(reach(&clear), 0, "no brim point lies on the tower's ground");
        assert!(!clear.is_empty());

        // The skirt goes round the tower as well as the part.
        let corners: Vec<Point2> = tower[0].contour.points.clone();
        let around = skirt(&[&part], &clear, &[], &corners, &settings, 0.2).unwrap();
        let furthest = around[0]
            .polygon
            .points
            .iter()
            .map(|point| unscaled(point.x))
            .fold(f64::MIN, f64::max);
        assert!(
            furthest > 32.0 + 2.0,
            "skirt reaches {furthest}, beyond the tower"
        );
    }
}
