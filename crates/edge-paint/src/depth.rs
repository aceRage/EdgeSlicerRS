//! How deep a painted colour reaches into a part (`paint_depth_mode`), a
//! port of the reference's paint depth (PaintDepth.cpp and the parts of
//! MultiMaterialSegmentation.cpp that use it).
//!
//! With the depth unlimited, the default, a colour takes everything its
//! painted surface is nearest, as it always did. Bounded, by a count of
//! walls or by millimetres, it takes a band of that width along the
//! outline (`cut_segmented_layers`, [`keep_core`]), the same thickness
//! down from a painted top and up from a painted bottom
//! ([`crate::shells`]), and the rest of the part keeps its own filament.

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2};

/// `paint_depth_mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// A colour takes all it is nearest (`pdmUnlimited`).
    #[default]
    Unlimited,
    /// About `paint_depth_walls` walls deep (`pdmWalls`).
    Walls,
    /// `paint_depth_mm` deep (`pdmMillimeters`).
    Millimeters,
}

impl Mode {
    /// The setting's value as a project stores it.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unlimited" => Some(Self::Unlimited),
            "walls" => Some(Self::Walls),
            "millimeters" => Some(Self::Millimeters),
            _ => None,
        }
    }
}

/// The width of the band a bounded colour keeps along the outline, in mm,
/// 0 for unlimited (`paint_depth_band_mm`). In walls the band is `walls`
/// (at least one) spacings of the inner wall, what Arachne takes off each
/// side before it lays walls (twice the outer wall's width less its
/// spacing) and a quarter spacing more, so that the walls asked for fit
/// on every layer; never less than nothing. In single precision, as the
/// reference works it out.
pub fn band_mm(
    mode: Mode,
    walls: i64,
    mm: f64,
    outer_width: f32,
    outer_spacing: f32,
    inner_spacing: f32,
) -> f32 {
    match mode {
        Mode::Unlimited => 0.0,
        Mode::Millimeters => mm as f32,
        Mode::Walls => {
            let walls = walls.max(1) as f32;
            let band =
                walls * inner_spacing + 2.0 * (outer_width - outer_spacing) + 0.25 * inner_spacing;
            band.max(0.0)
        }
    }
}

/// With classic walls a band is never narrower than an outer wall's width
/// and spacing (a "wall stack"): classic walls cannot fill a narrower
/// one honestly (`paint_depth_band_classic_floor_mm`). A band of nothing
/// stays nothing.
pub fn band_classic_floor_mm(band: f32, outer_width: f32, outer_spacing: f32) -> f32 {
    if band <= 0.0 {
        return band;
    }
    let stack = outer_width + outer_spacing;
    if stack > 0.0 { band.max(stack) } else { band }
}

/// The notch cut into the band on even layers so that the colours key
/// into each other (`mmu_segmented_region_interlocking_depth`): in walls
/// no deeper than a quarter of the inner wall's spacing, the margin the
/// band is built with (`paint_depth_interlocking_depth_mm`).
pub fn interlocking_mm(mode: Mode, configured: f64, inner_spacing: f32) -> f32 {
    if configured <= 0.0 || inner_spacing <= 0.0 || mode != Mode::Walls {
        return configured as f32;
    }
    (configured as f32).min(0.25 * inner_spacing)
}

/// With classic walls the notch is no deeper than the band is wider than a
/// wall stack, so that the band never falls under the floor on even
/// layers either (`paint_depth_classic_notch_cap_mm`).
pub fn classic_notch_cap_mm(interlocking: f32, band: f32, wall_stack: f32) -> f32 {
    if interlocking <= 0.0 || band <= 0.0 || wall_stack <= 0.0 {
        return interlocking;
    }
    interlocking.min((band - wall_stack).max(0.0))
}

/// What a bounded depth needs, worked out from the profile; all of it
/// nothing where the depth is unlimited.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Depth {
    /// The band each colour keeps along the outline, in mm; 0 for
    /// unlimited (`segmentation_max_width`).
    pub band: f32,
    /// The notch cut into the band on even layers, in mm
    /// (`segmentation_interlocking_depth`).
    pub interlocking: f32,
    /// The narrowest claim of paint worth printing, the outer wall's width,
    /// in mm (`segmentation_min_claim_width`).
    pub min_claim_width: f32,
    /// How thick a colour is carried down from a top and up from a bottom,
    /// in mm: the band, where the depth is bounded
    /// (`paint_depth_normal_mm`).
    pub normal: f32,
    /// An outer wall's width and spacing, in mm: how far from the outline
    /// colour carried in from a face keeps (`segmentation_wall_stack`).
    pub wall_stack: f32,
    /// `interlocking_beam`: colours are keyed by beams, and the band is not
    /// cut at all.
    pub beam: bool,
}

impl Depth {
    pub fn bounded(&self) -> bool {
        self.normal > 0.0
    }
}

/// `float(scale_(mm))`: a width in mm in scaled units, through single
/// precision as the reference passes it.
pub(crate) fn scaled_f32(mm: f32) -> f32 {
    (f64::from(mm) / 1e-6) as f32
}

/// A delta in scaled units as the clipping library takes it.
pub(crate) fn delta(scaled: f32) -> Coord {
    scaled.round() as Coord
}

/// What of a layer keeps its own filament where the colours are cut back
/// to a band of `band` (`paint_depth_clamp_keep_core`): the layer shrunk
/// by `band`, and where a part is too thin for that, shrunk by the widest
/// of `ladder / 4`, `ladder / 8`, ... (six steps, none under
/// `min_claim`) that leaves a part at least twice as wide, so that a thin
/// part keeps half of itself and paints a skin no thinner than a wall.
/// `ladder` is the band without the even layers' notch, so that which
/// parts count as thin does not change from layer to layer. All in scaled
/// units.
pub(crate) fn keep_core(
    layer: &[ExPolygon],
    band: f32,
    ladder: f32,
    min_claim: f32,
) -> Result<Vec<ExPolygon>, ClipError> {
    let join = Join::MITER;
    let core_full = edge_clip::offset(layer, -delta(ladder), join)?;
    let mut thin = if core_full.is_empty() {
        layer.to_vec()
    } else {
        edge_clip::difference(layer, &edge_clip::offset(&core_full, delta(ladder), join)?)?
    };
    let mut core = if band == ladder {
        core_full
    } else {
        edge_clip::offset(layer, -delta(band), join)?
    };
    let mut b = 0.25 * ladder;
    let mut step = 0;
    while step < 6 && !thin.is_empty() && b >= min_claim && b > 0.0 {
        let fits =
            edge_clip::intersection(&thin, &edge_clip::opening(layer, delta(2.0 * b), join)?)?;
        if !fits.is_empty() {
            core.extend(edge_clip::intersection(
                &edge_clip::offset(layer, -delta(b), join)?,
                &fits,
            )?);
            core = edge_clip::union_ex(&core)?;
            thin = edge_clip::difference(&thin, &fits)?;
        }
        step += 1;
        b *= 0.5;
    }
    Ok(core)
}

/// One layer's colours cut back to the band (`cut_segmented_layers`, for
/// one layer): `shares` holds every colour's share, 0 among them. Gives
/// back what keeps the part's own filament, or nothing where nothing was
/// cut. `index` is the layer's number in the object: even layers have the
/// band narrowed by the notch.
pub(crate) fn cut(
    index: usize,
    layer: &[ExPolygon],
    shares: &mut [Vec<ExPolygon>],
    depth: &Depth,
) -> Result<Vec<ExPolygon>, ClipError> {
    if (depth.band <= 0.0 && depth.interlocking <= 0.0) || depth.beam {
        return Ok(Vec::new());
    }
    let band = scaled_f32(depth.band);
    let interlocking = scaled_f32(depth.interlocking);
    let notched = if interlocking > 0.0 {
        (band - interlocking).max(0.0)
    } else {
        0.0
    };
    let width = if index.is_multiple_of(2) && notched > 0.0 {
        notched
    } else {
        band
    };
    if width <= 0.0 || shares.iter().all(Vec::is_empty) {
        return Ok(Vec::new());
    }
    let core = keep_core(layer, width, band, scaled_f32(depth.min_claim_width))?;
    for share in shares.iter_mut().filter(|share| !share.is_empty()) {
        *share = edge_clip::difference(share, &core)?;
    }
    Ok(core)
}

/// Whether `point` lies inside `region` (on a hole's edge counts as out).
pub(crate) fn contains(region: &ExPolygon, point: Point2) -> bool {
    let inside = |ring: &edge_geom::Polygon| {
        let points = &ring.points;
        let mut crossings = false;
        for at in 0..points.len() {
            let (a, b) = (points[at], points[(at + 1) % points.len()]);
            if (a.y > point.y) != (b.y > point.y) {
                let x =
                    a.x as f64 + (point.y - a.y) as f64 * (b.x - a.x) as f64 / (b.y - a.y) as f64;
                if (point.x as f64) < x {
                    crossings = !crossings;
                }
            }
        }
        crossings
    };
    inside(&region.contour) && !region.holes.iter().any(inside)
}

fn area(regions: &[ExPolygon]) -> f64 {
    regions.iter().map(ExPolygon::area_mm2).sum()
}

/// The painted colour an island of the part's own filament goes to
/// (`interclaim_absorb_winner`): the one whose claim shares the most with
/// the island grown by `eps`, the lowest number on a tie; 0 where none
/// touches it. `claims[0]` is not a candidate.
pub fn absorb_winner(
    island: &[ExPolygon],
    claims: &[Vec<ExPolygon>],
    eps: Coord,
) -> Result<usize, ClipError> {
    let grown = edge_clip::offset(island, eps, Join::MITER)?;
    let (mut best, mut best_area) = (0, 0.0);
    for (colour, claim) in claims.iter().enumerate().skip(1) {
        if claim.is_empty() {
            continue;
        }
        let shared = edge_clip::intersection(&grown, claim)?;
        if shared.is_empty() {
            continue;
        }
        let shared = area(&shared);
        if best == 0 || shared > best_area {
            (best, best_area) = (colour, shared);
        }
    }
    Ok(best)
}

/// How narrow an island must be to be given to a colour beside it, in mm
/// (`interclaim_absorb_effective_claim_width`): `min_claim`, or wider
/// where a colour that touches it (grown by `eps`) has gap fill off,
/// `widths[colour]` saying how wide for each (0 where gap fill is on).
pub fn absorb_claim_width(
    island: &[ExPolygon],
    claims: &[Vec<ExPolygon>],
    widths: &[f32],
    min_claim: f32,
    eps: Coord,
) -> Result<f32, ClipError> {
    let mut grown: Option<Vec<ExPolygon>> = None;
    let mut widened = 0.0f32;
    for (colour, claim) in claims.iter().enumerate().skip(1) {
        let Some(&candidate) = widths.get(colour) else {
            continue;
        };
        if candidate <= widened || claim.is_empty() {
            continue;
        }
        if grown.is_none() {
            grown = Some(edge_clip::offset(island, eps, Join::MITER)?);
        }
        let grown = grown.as_deref().unwrap_or_default();
        if !edge_clip::intersection(grown, claim)?.is_empty() {
            widened = candidate;
        }
    }
    Ok(min_claim.max(widened))
}

#[cfg(test)]
mod tests;
