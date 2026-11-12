//! What organic trees hold up (TreeSupport.cpp `TreeSupport::detect_overhangs`,
//! which the organic generator calls for its overhangs, with the layer rule
//! of SupportOverhangDetect.cpp): each layer's area past the one below by
//! more than the threshold angle allows, measured on what is wide enough
//! to print; parts that start in mid-air and stay narrow ("sharp tails"),
//! held up every half millimetre; and with `support_remove_small_overhang`
//! clusters of overhangs too small to need it left out, unless they are a
//! cantilever or a sharp tail.

use edge_clip::Join;
use edge_geom::{Coord, ExPolygon, Point2, scaled};
use rayon::prelude::*;

use super::geometry::{Polys, diff, extent, intersection, offset, union};
use crate::{Job, SupportError};

/// SupportOverhangDetect.cpp `tree_support_extrudable`: the part of a layer
/// at least a line wide.
fn extrudable(slices: &[ExPolygon], width: Coord) -> Result<Polys, SupportError> {
    let wide = edge_clip::offset2(slices, -width / 2, width, Join::MITER)?;
    intersection(slices, &wide)
}

fn overlaps(a: &[ExPolygon], b: &[ExPolygon]) -> Result<bool, SupportError> {
    Ok(!intersection(a, b)?.is_empty())
}

fn size(regions: &[ExPolygon]) -> (Coord, Coord) {
    extent(regions).map_or((0, 0), |[a, b, c, d]| (c - a, d - b))
}

/// Distance from a point to the nearest edge of some regions.
fn distance_to(regions: &[ExPolygon], point: Point2) -> f64 {
    let mut least = f64::INFINITY;
    for ring in regions.iter().flat_map(ExPolygon::rings) {
        let points = &ring.points;
        for index in 0..points.len() {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
            let (px, py) = ((point.x - a.x) as f64, (point.y - a.y) as f64);
            let length2 = dx * dx + dy * dy;
            let t = if length2 > 0.0 {
                ((px * dx + py * dy) / length2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            least = least.min((px - t * dx).hypot(py - t * dy));
        }
    }
    least
}

/// A group of overhangs that touch from layer to layer.
struct Cluster {
    /// Every overhang in it, with its layer. The reference keeps one a
    /// layer (a `std::map` keyed by layer) and so drops the second of two
    /// overhangs on one layer that join the same cluster: that one is
    /// never supported. Here all are kept.
    members: Vec<(usize, usize)>,
    merged: Polys,
    bounds: [Coord; 4],
    min_layer: usize,
    max_layer: usize,
    cantilever: bool,
}

/// The overhangs organic trees hold up, layer by layer, and which of them
/// are sharp tails (grown by 0.2 mm before their tips are placed, as the
/// reference does in `generate_support_areas`).
pub(crate) fn detect(job: &Job<'_>) -> Result<Vec<Polys>, SupportError> {
    let settings = job.settings;
    let count = job.slices.len();
    let width = settings.line_width.resolve(settings.nozzle_diameter);
    let width_scaled = scaled(width);
    let threshold = if settings.threshold_angle > 1e-4 {
        settings.threshold_angle + 1.0
    } else {
        30.0
    }
    .min(89.0)
    .to_radians();
    let length_well_supported = scaled(6.0);
    let sharp_tail_max_height = 16.0;

    let printable: Vec<Polys> = job
        .slices
        .par_iter()
        .map(|layer| extrudable(layer, width_scaled))
        .collect::<Result<_, _>>()?;

    struct Found {
        overhangs: Polys,
        cantilevers: Polys,
        sharp_tails: Vec<(ExPolygon, f64)>,
    }
    let found: Vec<Found> = (0..count)
        .into_par_iter()
        .map(|layer| -> Result<Found, SupportError> {
            let current = &printable[layer];
            if layer == 0 {
                // On the bed, anything small is the start of a sharp tail.
                let sharp_tails = current
                    .iter()
                    .filter(|slice| {
                        let (x, y) = size(std::slice::from_ref(slice));
                        !(x > length_well_supported && y > length_well_supported)
                    })
                    .map(|slice| (slice.clone(), job.height_of(0)))
                    .collect();
                return Ok(Found {
                    overhangs: Vec::new(),
                    cantilevers: Vec::new(),
                    sharp_tails,
                });
            }
            let lower = &printable[layer - 1];
            let reach = job.height_of(layer - 1) / threshold.tan();
            let lower_grown = offset(lower, scaled(reach), Join::Square)?;
            let overhangs = diff(current, &lower_grown)?;
            let mut sharp_tails = Vec::new();
            {
                for slice in current {
                    let one = std::slice::from_ref(slice);
                    let near = offset(one, scaled(0.1 * width), Join::MITER)?;
                    if !overlaps(&near, lower)?
                        && !offset(one, -scaled(0.1 * width), Join::MITER)?.is_empty()
                    {
                        sharp_tails.push((slice.clone(), 0.0));
                    }
                }
            }
            // A cantilever reaches more than 3 mm from what it hangs from.
            let boundary_area = offset(
                &lower_grown,
                scaled((width - reach).max(0.0) + 0.1),
                Join::MITER,
            )?;
            let mut cantilevers = Vec::new();
            for overhang in &overhangs {
                let boundary = intersection(std::slice::from_ref(overhang), &boundary_area)?;
                if boundary.is_empty() {
                    continue;
                }
                let furthest = overhang
                    .contour
                    .points
                    .iter()
                    .map(|p| distance_to(&boundary, *p))
                    .fold(0.0, f64::max);
                if furthest > scaled(3.0) as f64 {
                    cantilevers.push(overhang.clone());
                }
            }
            Ok(Found {
                overhangs,
                cantilevers,
                sharp_tails,
            })
        })
        .collect::<Result<_, _>>()?;

    // Sharp tails carry on up while they stay narrow, to 16 mm.
    let mut sharp_tails: Vec<Vec<(ExPolygon, f64)>> =
        found.iter().map(|f| f.sharp_tails.clone()).collect();
    for layer in 1..count {
        let below: Vec<ExPolygon> = sharp_tails[layer - 1].iter().map(|t| t.0.clone()).collect();
        if below.is_empty() {
            continue;
        }
        let below_size = size(&below);
        for slice in &printable[layer] {
            let one = std::slice::from_ref(slice);
            let supported = intersection(one, &below)?;
            if supported.is_empty() {
                continue;
            }
            let (x, y) = size(&supported);
            if x > length_well_supported && y > length_well_supported {
                continue;
            }
            let mut height = job.height_of(layer);
            for (tail, tail_height) in &sharp_tails[layer - 1] {
                if overlaps(std::slice::from_ref(tail), one)? {
                    height += tail_height;
                    break;
                }
            }
            if height > sharp_tail_max_height {
                continue;
            }
            let fresh = diff(one, &below)?;
            let (fx, fy) = size(&fresh);
            let grows = fx - below_size.0 > scaled(5.0) && fy - below_size.1 > scaled(5.0);
            if grows || !offset(&fresh, -scaled(5.0 * width), Join::MITER)?.is_empty() {
                continue;
            }
            sharp_tails[layer].push((slice.clone(), height));
        }
    }

    // Clusters of overhangs that touch from layer to layer.
    let mut clusters: Vec<Cluster> = Vec::new();
    for (layer, layer_found) in found.iter().enumerate() {
        for (index, overhang) in layer_found.overhangs.iter().enumerate() {
            let one = std::slice::from_ref(overhang);
            let grown = offset(one, width_scaled, Join::MITER)?;
            let bounds = extent(&grown).unwrap_or([0; 4]);
            let mut joined = None;
            for (at, cluster) in clusters.iter_mut().enumerate() {
                if layer == 0 || cluster.max_layer + 1 < layer {
                    continue;
                }
                let [a, b, c, d] = cluster.bounds;
                if bounds[0] > c || bounds[2] < a || bounds[1] > d || bounds[3] < b {
                    continue;
                }
                let previous: Vec<ExPolygon> = cluster
                    .members
                    .iter()
                    .filter(|(l, _)| *l + 1 == layer)
                    .map(|(l, i)| found[*l].overhangs[*i].clone())
                    .collect();
                if previous.is_empty() || !overlaps(&previous, &grown)? {
                    continue;
                }
                cluster.members.push((layer, index));
                cluster.merged = union(&[cluster.merged.clone(), grown.clone()].concat())?;
                cluster.max_layer = cluster.max_layer.max(layer);
                cluster.bounds = [
                    a.min(bounds[0]),
                    b.min(bounds[1]),
                    c.max(bounds[2]),
                    d.max(bounds[3]),
                ];
                joined = Some(at);
                break;
            }
            let at = match joined {
                Some(at) => at,
                None => {
                    // The first of a cluster is not grown (the reference's
                    // offset is still zero then).
                    clusters.push(Cluster {
                        members: vec![(layer, index)],
                        merged: one.to_vec(),
                        bounds: extent(one).unwrap_or([0; 4]),
                        min_layer: layer,
                        max_layer: layer,
                        cantilever: false,
                    });
                    clusters.len() - 1
                }
            };
            if overlaps(one, &layer_found.cantilevers)? {
                clusters[at].cantilever = true;
            }
        }
    }

    let mut kept: Vec<Polys> = vec![Vec::new(); count];
    for cluster in &clusters {
        let mut small = false;
        if settings.remove_small_overhang {
            let mut sharp = false;
            for layer_tails in &sharp_tails[cluster.min_layer..=cluster.max_layer] {
                let tails: Vec<ExPolygon> = layer_tails.iter().map(|t| t.0.clone()).collect();
                if overlaps(&tails, &cluster.merged)? {
                    sharp = true;
                    break;
                }
            }
            if !sharp && !cluster.cantilever {
                let eroded = offset(&cluster.merged, -width_scaled, Join::MITER)?;
                let (x, y) = size(&eroded);
                small = x < 2 * width_scaled || y < 2 * width_scaled;
            }
        }
        if !small {
            for &(layer, index) in &cluster.members {
                kept[layer].push(found[layer].overhangs[index].clone());
            }
        }
    }

    let mut overhangs = Vec::with_capacity(count);
    for layer in 0..count {
        let mut out = std::mem::take(&mut kept[layer]);
        if settings.critical_regions_only {
            // Only cantilevers, and sharp tails below.
            out = found[layer].cantilevers.clone();
        }
        // Sharp tails get a tip every half millimetre of height.
        let tails: Polys = if layer == 0 {
            sharp_tails[0].iter().map(|t| t.0.clone()).collect()
        } else {
            let lower = offset(&printable[layer - 1], scaled(0.0125), Join::MITER)?;
            let mut tails = Vec::new();
            for (tail, height) in &sharp_tails[layer] {
                let areas = diff(std::slice::from_ref(tail), &lower)?;
                if !areas.is_empty() && ((height * 10.0) as i64) % 5 == 0 {
                    tails.extend(areas);
                }
            }
            tails
        };
        if !tails.is_empty() {
            out.extend(offset(&tails, scaled(0.2), Join::MITER)?);
        }
        overhangs.push(out);
    }
    Ok(overhangs)
}
