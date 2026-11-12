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

/// Whether some point lies further than `limit` from every corner of
/// some regions. The reference measures a cantilever this way, to the
/// nearest corner (`MultiPoint::distance_to`), not to the nearest edge.
fn reaches_beyond(points: &[Point2], regions: &[ExPolygon], limit: f64) -> bool {
    let corners: Vec<Point2> = regions
        .iter()
        .flat_map(ExPolygon::rings)
        .flat_map(|ring| ring.points.iter().copied())
        .collect();
    let limit2 = limit * limit;
    points.iter().any(|p| {
        !corners.iter().any(|c| {
            let (dx, dy) = ((c.x - p.x) as f64, (c.y - p.y) as f64);
            dx * dx + dy * dy <= limit2
        })
    })
}

/// For each slice of a layer, the slices of the layer below it meets and
/// the box round where they meet.
type Meets = Vec<Vec<(usize, [Coord; 4])>>;

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
                // Within a tenth of a line of the layer below: the layer
                // below grown, once, rather than each slice.
                let lower_near = offset(lower, scaled(0.1 * width), Join::MITER)?;
                for slice in current {
                    let one = std::slice::from_ref(slice);
                    if super::geometry::intersection_near(one, &lower_near)?.is_empty()
                        && !offset(one, -scaled(0.1 * width), Join::MITER)?.is_empty()
                    {
                        sharp_tails.push((slice.clone(), 0.0));
                    }
                }
            }
            // A cantilever reaches more than 3 mm from what it hangs from.
            let boundary_area = if overhangs.is_empty() {
                Vec::new()
            } else {
                offset(
                    &lower_grown,
                    scaled((width - reach).max(0.0) + 0.1),
                    Join::MITER,
                )?
            };
            let mut cantilevers = Vec::new();
            for overhang in &overhangs {
                let boundary = super::geometry::intersection_near(
                    std::slice::from_ref(overhang),
                    &boundary_area,
                )?;
                if boundary.is_empty() {
                    continue;
                }
                if reaches_beyond(&overhang.contour.points, &boundary, scaled(3.0) as f64) {
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
    // Sharp tails carry on up while they stay narrow, to 16 mm. Which
    // slices meet which on the layer below is worked out for every layer
    // at once; going up the layers is then only bookkeeping, with a
    // difference now and then.
    let meets: Vec<Meets> = (0..count)
        .into_par_iter()
        .map(|layer| -> Result<Meets, SupportError> {
            if layer == 0 {
                return Ok(Vec::new());
            }
            let lower = &printable[layer - 1];
            let lower_boxes: Vec<Option<[Coord; 4]>> = lower
                .iter()
                .map(|region| extent(std::slice::from_ref(region)))
                .collect();
            printable[layer]
                .iter()
                .map(|slice| {
                    let one = std::slice::from_ref(slice);
                    let Some([x0, y0, x1, y1]) = extent(one) else {
                        return Ok(Vec::new());
                    };
                    let mut out = Vec::new();
                    for (j, region) in lower.iter().enumerate() {
                        let Some([a, b, c, d]) = lower_boxes[j] else {
                            continue;
                        };
                        if a > x1 || c < x0 || b > y1 || d < y0 {
                            continue;
                        }
                        let met = intersection(one, std::slice::from_ref(region))?;
                        if let Some(bounds) = extent(&met) {
                            out.push((j, bounds));
                        }
                    }
                    Ok(out)
                })
                .collect()
        })
        .collect::<Result<_, _>>()?;
    // Tails by layer, as the slices they are and their heights.
    let mut tails_at: Vec<Vec<(usize, f64)>> = vec![Vec::new(); count];
    for (layer, layer_found) in found.iter().enumerate() {
        for (tail, height) in &layer_found.sharp_tails {
            if let Some(index) = printable[layer].iter().position(|slice| slice == tail) {
                tails_at[layer].push((index, *height));
            }
        }
    }
    for layer in 1..count {
        if tails_at[layer - 1].is_empty() {
            continue;
        }
        let below: Vec<ExPolygon> = tails_at[layer - 1]
            .iter()
            .map(|(j, _)| printable[layer - 1][*j].clone())
            .collect();
        let below_size = size(&below);
        let mut fresh_tails = Vec::new();
        for (i, slice) in printable[layer].iter().enumerate() {
            let met: Vec<&(usize, [Coord; 4])> = meets[layer][i]
                .iter()
                .filter(|(j, _)| tails_at[layer - 1].iter().any(|(t, _)| t == j))
                .collect();
            if met.is_empty() {
                continue;
            }
            let [x0, y0, x1, y1] = met.iter().fold(
                [Coord::MAX, Coord::MAX, Coord::MIN, Coord::MIN],
                |[a, b, c, d], (_, [e, f, g, h])| [a.min(*e), b.min(*f), c.max(*g), d.max(*h)],
            );
            if x1 - x0 > length_well_supported && y1 - y0 > length_well_supported {
                continue;
            }
            let mut height = job.height_of(layer);
            if let Some((_, tail_height)) = tails_at[layer - 1]
                .iter()
                .find(|(t, _)| met.iter().any(|(j, _)| j == t))
            {
                height += tail_height;
            }
            if height > sharp_tail_max_height {
                continue;
            }
            let fresh = diff(std::slice::from_ref(slice), &below)?;
            let (fx, fy) = size(&fresh);
            let grows = fx - below_size.0 > scaled(5.0) && fy - below_size.1 > scaled(5.0);
            if grows || !offset(&fresh, -scaled(5.0 * width), Join::MITER)?.is_empty() {
                continue;
            }
            fresh_tails.push((i, height));
        }
        tails_at[layer].extend(fresh_tails);
    }
    let sharp_tails: Vec<Vec<(ExPolygon, f64)>> = tails_at
        .iter()
        .enumerate()
        .map(|(layer, tails)| {
            tails
                .iter()
                .map(|(i, height)| (printable[layer][*i].clone(), *height))
                .collect()
        })
        .collect();
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

    let overhangs: Vec<Polys> = kept
        .into_par_iter()
        .enumerate()
        .map(|(layer, mut out)| -> Result<Polys, SupportError> {
            if settings.critical_regions_only {
                // Only cantilevers, and sharp tails below.
                out = found[layer].cantilevers.clone();
            }
            // Sharp tails get a tip every half millimetre of height.
            let tails: Polys = if layer == 0 {
                sharp_tails[0].iter().map(|t| t.0.clone()).collect()
            } else {
                let due: Vec<&ExPolygon> = sharp_tails[layer]
                    .iter()
                    .filter(|(_, height)| ((height * 10.0) as i64) % 5 == 0)
                    .map(|(tail, _)| tail)
                    .collect();
                let mut tails = Vec::new();
                if !due.is_empty() {
                    let lower = offset(&printable[layer - 1], scaled(0.0125), Join::MITER)?;
                    for tail in due {
                        tails.extend(diff(std::slice::from_ref(tail), &lower)?);
                    }
                }
                tails
            };
            if !tails.is_empty() {
                out.extend(offset(&tails, scaled(0.2), Join::MITER)?);
            }
            Ok(out)
        })
        .collect::<Result<_, _>>()?;
    Ok(overhangs)
}
