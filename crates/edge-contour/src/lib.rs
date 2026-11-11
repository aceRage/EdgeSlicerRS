//! Z contouring ("ZAA", Z anti-aliasing): top surfaces, and the walls
//! round them, laid not flat at their layer's top but at the height of
//! the model's own surface under each point, within the layer, so that a
//! shallow slope or a curved top prints without its stair steps.
//!
//! A port of the reference's ContourZ.cpp (EdgeSlicer's port of
//! OrcaSlicer PR #12736, with its follow-ups), as PrintObject.cpp
//! `contour_z` runs it: every layer but the first, each top surface
//! line, ironing line and wall is cut into points 0.1 mm apart; a ray
//! is cast straight up from each, from the plane the layer was sliced
//! at, to the model's surface; how far that is above or below the
//! layer's top, kept to the band the line may move in, is the point's
//! height ([`sample::delta`]); the heights along the line are smoothed;
//! a line that moves by less than 10 µm all along is left flat; and of
//! the points left, those in line with the two before them in three
//! dimensions are dropped. What is left is the line's new points, and
//! its heights, by place, a [`Profile`] the writer lays it by.
//!
//! With contouring on, every layer but the first is sliced
//! `zaa_min_z` above its bottom rather than through its middle
//! (PrintObjectSlice.cpp `compute_slice_z`), so that a line may go down
//! to there and up by as much above the layer's top and still describe
//! the same solid: [`ZaaSettings::slice_at`].

mod profile;
pub mod sample;
pub mod speed;
mod surface;

use std::sync::Arc;

use edge_config::{ConfigError, Settings};
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Role};
use edge_geom::{Coord, Point2, scaled, unscaled};

pub use profile::Profile;
pub use sample::Kind;
pub use surface::Surface;

/// How far apart the points a line is contoured at are, at most, in mm
/// (`ZAA_SAMPLE_RESOLUTION_MM`).
pub const RESOLUTION: f64 = 0.1;
/// How far a point may lie off the line through the two before it and
/// still be dropped, in mm (`ZAA_COLLAPSE_TOLERANCE_MM`).
pub const COLLAPSE_TOLERANCE: f64 = 0.001;
/// A line that moves by less than this all along is left flat, in mm
/// (`ZAA_MIN_PATH_DELTA_MM`).
pub const LEAST_MOVE: f64 = 0.010;

/// Z contouring as the settings ask for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZaaSettings {
    /// `zaa_min_z`: how far above its bottom a layer is sliced, and so
    /// how far above the layer beneath a line may go down to and above
    /// its own layer's top a top surface may go up to, in mm.
    pub min_z: f64,
    /// `zaa_minimize_perimeter_height`: walls on a surface sloped more
    /// than this many degrees are let down the slope by half their
    /// width; 0 for never.
    pub minimize_perimeter_height: f64,
    /// `zaa_speed_scaling`: a move made thinner is made slower in step.
    pub speed_scaling: bool,
}

/// What the settings ask of Z contouring: whether it is done, and what
/// of it is not, each note starting with the setting it is about.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asked {
    pub zaa: Option<ZaaSettings>,
    pub notes: Vec<String>,
}

fn on(settings: &Settings, key: &str) -> Result<bool, ConfigError> {
    Ok(settings.flag(key)?.unwrap_or(false))
}

/// Whether any filament has `key` on: a filament's own setting is a list.
fn any_on(settings: &Settings, key: &str) -> bool {
    settings
        .texts(key)
        .is_some_and(|values| values.iter().any(|value| value == "1" || value == "true"))
}

impl ZaaSettings {
    /// Z contouring as `settings` ask for it (PrintObject.cpp
    /// `need_z_contouring` for when it is not done).
    pub fn read(settings: &Settings) -> Result<Asked, ConfigError> {
        let mut asked = Asked::default();
        if !on(settings, "zaa_enabled")? {
            return Ok(asked);
        }
        let ranged = |key: &str, default: f64, max: f64| -> Result<f64, ConfigError> {
            let value = settings.float(key)?.unwrap_or(default);
            if (0.0..=max).contains(&value) {
                Ok(value)
            } else {
                Err(ConfigError::BadValue {
                    key: key.to_owned(),
                    found: value.to_string(),
                    expected: if max == 90.0 {
                        "an angle from 0 to 90 degrees"
                    } else {
                        "a height from 0 to 100 mm"
                    },
                })
            }
        };
        let zaa = Self {
            min_z: ranged("zaa_min_z", 0.05, 100.0)?,
            minimize_perimeter_height: ranged("zaa_minimize_perimeter_height", 35.0, 90.0)?,
            speed_scaling: settings.flag("zaa_speed_scaling")?.unwrap_or(true),
        };
        // Vase mode owns the height of its whole path, and the
        // reference leaves contouring out with it ("spiral vase mode is
        // on"); this engine does not print vases, and says so apart.
        if on(settings, "spiral_mode")? {
            asked.notes.push(
                "zaa_enabled \"1\": not with spiral_mode, as in the reference: top surfaces are laid flat"
                    .to_owned(),
            );
            return Ok(asked);
        }
        // "local-Z dithering owns the layer Z budget".
        if on(settings, "dithering_local_z_mode")? {
            asked.notes.push(
                "dithering_local_z_mode \"1\": local Z dithering is not done; as in the reference, \
                 top surfaces are then not contoured either (zaa_enabled)"
                    .to_owned(),
            );
            return Ok(asked);
        }
        if let Some(kind) = settings.text("seam_slope_type")
            && kind != "none"
        {
            asked.notes.push(format!(
                "seam_slope_type \"{kind}\": a wall contoured in height (zaa_enabled) is closed with \
                 the seam gap and no scarf; the reference lays the scarf's stretch flat along the \
                 contour at full flow, twice over"
            ));
        }
        if zaa.speed_scaling && any_on(settings, "slow_down_for_layer_cooling") {
            asked.notes.push(
                "zaa_speed_scaling \"1\": where a layer is slowed down to cool, a contoured move \
                 slowed for its height is held to the layer's speed cap like any other line; the \
                 reference slows such moves in proportion, keeping their speeds in step with their \
                 heights"
                    .to_owned(),
            );
        }
        asked.zaa = Some(zaa);
        Ok(asked)
    }

    /// Where a layer is sliced, from its bottom and its height, in mm:
    /// `zaa_min_z` above its bottom, or for the first layer through its
    /// middle (PrintObjectSlice.cpp `compute_slice_z`; "ZAA does not
    /// contour the first layer anyway, and moving its plane produced
    /// first-layer artifacts", upstream #13766). An error where that is
    /// above the layer's top.
    pub fn slice_at(&self, index: usize, bottom: f64, height: f64) -> Result<f64, String> {
        if index == 0 {
            return Ok(bottom + height / 2.0);
        }
        // `is_approx` at libslic3r's EPSILON.
        if self.min_z > height + 1e-4 {
            return Err(format!(
                "zaa_min_z is {} mm, more than the {height:.3} mm of layer {}: a layer cannot be \
                 sliced above its own top (the reference stops with \"Bad min Z value\")",
                self.min_z,
                index + 1
            ));
        }
        Ok(bottom + self.min_z)
    }
}

/// One layer as contouring sees it, in the mesh's own heights: its top,
/// where it was sliced and how tall it is, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct At {
    pub print_z: f64,
    pub slice_z: f64,
    pub height: f64,
}

/// A line contoured: its new points, its heights by place, and where in
/// the line given each point came from (the stretch it lies on, from 0,
/// and how far along it), for what else is carried point by point.
#[derive(Clone, Debug, PartialEq)]
pub struct Contoured {
    pub points: Vec<Point2>,
    pub profile: Profile,
    pub along: Vec<f64>,
}

/// How far `p` is off the line through `a` and `b`, squared
/// (Line.hpp `distance_to_infinite_squared`).
fn off_line_squared(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> f64 {
    let v = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let va = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let l2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if l2 == 0.0 {
        return va[0] * va[0] + va[1] * va[1] + va[2] * va[2];
    }
    let t = (va[0] * v[0] + va[1] * v[1] + va[2] * v[2]) / l2;
    let d = [t * v[0] - va[0], t * v[1] - va[1], t * v[2] - va[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// Contours a line (ContourZ.cpp `contour_extrusion_path`): `points`,
/// and back to the first again where it is `closed`. None where it is
/// left flat. A closed line comes back closed, without its first point
/// again at the end.
pub fn contour_line(
    points: &[Point2],
    closed: bool,
    kind: Kind,
    half_width: f64,
    at: &At,
    zaa: &ZaaSettings,
    surface: &Surface,
) -> Option<Contoured> {
    let mut chain = points.to_vec();
    if closed {
        chain.extend(points.first().copied());
    }
    // "Safety check (upstream #13508)."
    if chain.len() < 2 {
        return None;
    }
    // The points 0.1 mm apart at most, each stretch cut evenly, with
    // where it came from and whether it is one of the line's own.
    // How far up a surface can be and still move a point: past the
    // band and its fade, and for a wall half its width more, which the
    // slope may take it back down by ([`sample::delta`]).
    let band = match kind {
        Kind::Ironing => at.height,
        Kind::Wall | Kind::Top => zaa.min_z,
    };
    let reach = (at.print_z - at.slice_z) + band + sample::TOP_TOLERANCE + half_width + 0.01;
    let mut xy: Vec<[f64; 2]> = Vec::new();
    let mut heights: Vec<f64> = Vec::new();
    let mut own: Vec<bool> = Vec::new();
    let mut along: Vec<f64> = Vec::new();
    for (stretch, pair) in chain.windows(2).enumerate() {
        let a = [unscaled(pair[0].x), unscaled(pair[0].y)];
        let b = [unscaled(pair[1].x), unscaled(pair[1].y)];
        let delta = [b[0] - a[0], b[1] - a[1]];
        let steps = (delta[0].hypot(delta[1]) / RESOLUTION).ceil() as usize;
        if steps == 0 {
            continue;
        }
        for step in 0..=steps {
            let p = [
                a[0] + delta[0] * step as f64 / steps as f64,
                a[1] + delta[1] * step as f64 / steps as f64,
            ];
            if let Some(last) = xy.last()
                && (p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2) < 1e-18
            {
                // One stretch's end is the next one's start.
                if step == 0
                    && let Some(last) = own.last_mut()
                {
                    *last = true;
                }
                continue;
            }
            let sample = sample::Sample {
                kind,
                hit: surface.up(p[0], p[1], at.slice_z, reach),
                print_z: at.print_z,
                slice_z: at.slice_z,
                height: at.height,
                min_z: zaa.min_z,
                minimize_perimeter_height: zaa.minimize_perimeter_height,
                half_width,
            };
            xy.push(p);
            heights.push(sample::delta(&sample));
            own.push(step == 0);
            along.push(stretch as f64 + step as f64 / steps as f64);
        }
    }
    if xy.len() < 2 {
        return None;
    }
    sample::smooth(&mut heights, sample::SMOOTH_RADIUS);
    if heights.iter().all(|height| height.abs() < LEAST_MOVE) {
        return None;
    }
    // A point in line with the two before it, in three dimensions,
    // takes the place of the one before it, unless it is one of the
    // line's own (which keeps a line of three points from becoming one
    // of two, upstream #13508).
    let mut kept: Vec<([f64; 3], f64)> = Vec::with_capacity(xy.len());
    for (index, (p, height)) in xy.iter().zip(&heights).enumerate() {
        let point = [p[0], p[1], *height];
        if kept.len() >= 2 && !own[index] {
            let (a, b) = (kept[kept.len() - 2].0, kept[kept.len() - 1].0);
            if off_line_squared(a, b, point) < COLLAPSE_TOLERANCE * COLLAPSE_TOLERANCE {
                *kept.last_mut().expect("two kept") = (point, along[index]);
                continue;
            }
        }
        kept.push((point, along[index]));
    }
    if kept.len() < 2 {
        return None;
    }
    let mut points: Vec<Point2> = kept
        .iter()
        .map(|(p, _)| Point2 {
            x: scaled(p[0]),
            y: scaled(p[1]),
        })
        .collect();
    let profile = Profile::new(
        points.clone(),
        kept.iter().map(|(p, _)| p[2] as f32).collect(),
    );
    let mut along: Vec<f64> = kept.iter().map(|(_, along)| *along).collect();
    if closed && points.len() > 1 && points.first() == points.last() {
        points.pop();
        along.pop();
    }
    if points.len() < if closed { 3 } else { 2 } {
        return None;
    }
    Some(Contoured {
        points,
        profile,
        along,
    })
}

/// What a layer's contoured lines are laid by, beside the layer's own
/// loops and paths: for each, its profile where it was contoured. Both
/// are empty where nothing on the layer was.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerProfiles {
    pub loops: Vec<Option<Arc<Profile>>>,
    pub paths: Vec<Option<Arc<Profile>>>,
}

impl LayerProfiles {
    pub fn is_empty(&self) -> bool {
        self.loops.iter().chain(&self.paths).all(Option::is_none)
    }
}

/// What a line of this role is, for contouring; None for a line that is
/// not contoured (ContourZ.cpp `contour_extrusion_path`: top surfaces,
/// ironing and walls, not a wall over air).
fn kind_of(role: Role) -> Option<Kind> {
    match role {
        Role::TopSolidInfill => Some(Kind::Top),
        Role::Ironing => Some(Kind::Ironing),
        Role::ExternalPerimeter | Role::Perimeter => Some(Kind::Wall),
        _ => None,
    }
}

/// A loop's widths at the points it was contoured at, each taken along
/// the stretch it lies on.
fn widths_along(widths: &[Coord], along: &[f64]) -> Vec<Coord> {
    let count = widths.len();
    along
        .iter()
        .map(|&at| {
            let stretch = (at.floor() as usize).min(count.saturating_sub(1));
            let t = at - stretch as f64;
            let (a, b) = (widths[stretch] as f64, widths[(stretch + 1) % count] as f64);
            (a + (b - a) * t).round() as Coord
        })
        .collect()
}

/// Contours a layer's walls and lines in place (ContourZ.cpp
/// `Layer::make_contour_z`), and gives what they are laid by.
pub fn contour_layer(
    loops: &mut [ExtrusionLoop],
    paths: &mut [ExtrusionPath],
    at: &At,
    zaa: &ZaaSettings,
    surface: &Surface,
) -> LayerProfiles {
    let mut made = LayerProfiles {
        loops: vec![None; loops.len()],
        paths: vec![None; paths.len()],
    };
    for (wall, kept) in loops.iter_mut().zip(&mut made.loops) {
        let Some(kind) = kind_of(wall.role) else {
            continue;
        };
        let Some(line) = contour_line(
            &wall.polygon.points,
            true,
            kind,
            wall.flow.width / 2.0,
            at,
            zaa,
            surface,
        ) else {
            continue;
        };
        if !wall.widths.is_empty() && wall.widths.len() == wall.polygon.points.len() {
            wall.widths = widths_along(&wall.widths, &line.along);
        }
        wall.polygon.points = line.points;
        *kept = Some(Arc::new(line.profile));
    }
    for (path, kept) in paths.iter_mut().zip(&mut made.paths) {
        let Some(kind) = kind_of(path.role) else {
            continue;
        };
        let Some(line) = contour_line(
            &path.points,
            false,
            kind,
            path.flow.width / 2.0,
            at,
            zaa,
            surface,
        ) else {
            continue;
        };
        path.points = line.points;
        *kept = Some(Arc::new(line.profile));
    }
    if made.is_empty() {
        return LayerProfiles::default();
    }
    made
}

#[cfg(test)]
mod tests;
