//! Bridges over infill (PrintObject.cpp `PrintObject::bridge_over_infill`):
//! solid fill that lies on sparse fill is laid as a bridge across it, run
//! out the way it goes to the sparse lines that carry it, and turned to
//! cross them; `dont_filter_internal_bridges` says how small a patch is
//! still worth it, and `enable_extra_bridge_layer` lays a second layer of
//! bridge across the first.

use std::f64::consts::{FRAC_PI_2, PI};

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon};
use rayon::prelude::*;

use crate::FillParams;
use crate::anchored::{anchored, bridging_angle, closed_lines, segments_of_lines};
use crate::surfaces::{Beneath, Surface, SurfaceKind};

/// How small bridges over infill are sorted out
/// (`dont_filter_internal_bridges`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalBridgeFilter {
    /// `disabled`, the default: a patch of solid fill partly held up by
    /// solid fill beneath is bridged only where what is not held up is
    /// more than three line spacings square; otherwise it is left as
    /// solid fill laid straight on the sparse fill. What is within four
    /// spacings of solid fill beneath, and three of the walls, counts as
    /// held up.
    Filter,
    /// `limited`: as the default, with solid fill taken to hold up what
    /// is within two spacings of it, and the walls one.
    Limited,
    /// `nofilter`: as `limited`, and every patch found is bridged however
    /// small, with four spacings round it.
    NoFilter,
}

/// Which way a bridge over infill runs, by the sparse pattern beneath it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BridgeDirection {
    /// Square to the lines round it, by vote (`determine_bridging_angle`).
    FromLines,
    /// For the patterns that turn every way (Hilbert curve, octagram
    /// spiral): square to the pattern's own direction, in radians, where
    /// the same filament's sparse fill lies beneath and there are lines
    /// to carry it; by vote elsewhere. Their outline is also followed
    /// four times as finely.
    AcrossPattern(f64),
    /// At this angle where there are lines to carry it (3D honeycomb and
    /// cross hatch: 45° on from `infill_direction`); by vote elsewhere.
    Fixed(f64),
}

/// What laying bridges over infill needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverInfill {
    /// The distance between solid lines.
    pub spacing: Coord,
    pub filter: InternalBridgeFilter,
    /// The round thread of a thick bridge (`bridging_flow(frSolidInfill,
    /// true)`), which the reference measures a bridge over infill and
    /// its surroundings by whether or not it is laid thick: the distance
    /// between threads, a thread's width, and its height in mm.
    pub thread_spacing: Coord,
    pub thread_width: Coord,
    pub thread_height: f64,
    pub direction: BridgeDirection,
    /// `internal_bridge_angle` where it is set, in radians, and whether it
    /// turns the direction found (`relative_bridge_angle`) or replaces it.
    pub set_angle: Option<(f64, bool)>,
}

/// Edges meant to coincide are made to (the reference's `SCALED_EPSILON`).
const SNAP: Coord = 100;
/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;
/// How deep, as a share of its height, a thread sinks into what is
/// beneath it (`target_flow_height_factor`).
const SINKS: f64 = 0.9;

/// A patch of solid fill worth bridging: the filament whose solid fill it
/// was picked out of, and the area picked out.
#[derive(Clone, Debug)]
struct Candidate {
    filament: usize,
    area: Vec<ExPolygon>,
}

/// A candidate once run out to its anchors, and its direction.
#[derive(Clone, Debug)]
struct Bridged {
    filament: usize,
    area: Vec<ExPolygon>,
    angle: f64,
}

fn area_of(regions: &[ExPolygon]) -> f64 {
    regions.iter().map(ExPolygon::area_mm2).sum()
}

/// The least and greatest corners of the regions, if they have any.
fn extent(regions: &[ExPolygon]) -> Option<(Point2, Point2)> {
    regions
        .iter()
        .flat_map(|region| region.contour.points.iter())
        .fold(None, |found, p| {
            Some(match found {
                None => (*p, *p),
                Some((low, high)) => (
                    Point2 {
                        x: low.x.min(p.x),
                        y: low.y.min(p.y),
                    },
                    Point2 {
                        x: high.x.max(p.x),
                        y: high.y.max(p.y),
                    },
                ),
            })
        })
}

/// Whether a ring's bounding box comes within `margin` of `bounds`.
fn meets(ring: &Polygon, bounds: (Point2, Point2), margin: Coord) -> bool {
    let Some(first) = ring.points.first() else {
        return false;
    };
    let (mut low, mut high) = (*first, *first);
    for p in &ring.points {
        low = Point2 {
            x: low.x.min(p.x),
            y: low.y.min(p.y),
        };
        high = Point2 {
            x: high.x.max(p.x),
            y: high.y.max(p.y),
        };
    }
    low.x <= bounds.1.x + margin
        && high.x >= bounds.0.x - margin
        && low.y <= bounds.1.y + margin
        && high.y >= bounds.0.y - margin
}

/// The regions whose bounding boxes touch that of `area`: all a boolean
/// of `area` with `regions` that keeps no more than `area` covers can be
/// changed by.
fn touching(regions: &[ExPolygon], area: &[ExPolygon]) -> Vec<ExPolygon> {
    match extent(area) {
        Some(bounds) => regions
            .iter()
            .filter(|region| meets(&region.contour, bounds, 0))
            .cloned()
            .collect(),
        None => Vec::new(),
    }
}

/// The regions within `reach` of `area`, and of those, and so on: the
/// rest are too far from it to make any difference to how it is run out.
/// The reference grows and cuts the whole layer's fill for each patch;
/// a run-out never leaves the part of it its patch is in, grown, so what
/// does not come within reach of that part changes nothing but the time
/// it takes. Bounding boxes are compared, which takes in too much, never
/// too little.
fn nearby<'a>(regions: &'a [ExPolygon], area: &[ExPolygon], reach: Coord) -> Vec<&'a ExPolygon> {
    let Some(start) = extent(area) else {
        return Vec::new();
    };
    let boxes: Vec<Option<(Point2, Point2)>> = regions
        .iter()
        .map(|region| extent(std::slice::from_ref(region)))
        .collect();
    let mut taken = vec![false; regions.len()];
    let mut reached = vec![start];
    while let Some(bounds) = reached.pop() {
        for (index, found) in boxes.iter().enumerate() {
            if let Some(other) = found
                && !taken[index]
                && other.0.x <= bounds.1.x + reach
                && other.1.x >= bounds.0.x - reach
                && other.0.y <= bounds.1.y + reach
                && other.1.y >= bounds.0.y - reach
            {
                taken[index] = true;
                reached.push(*other);
            }
        }
    }
    regions
        .iter()
        .zip(taken)
        .filter(|(_, taken)| *taken)
        .map(|(region, _)| region)
        .collect()
}

/// Turns solid fill that lies over sparse fill into bridges over infill,
/// for every filament (`surfaces[f][i]` is filament `f`'s layer `i`):
/// a port of PrintObject.cpp `PrintObject::bridge_over_infill` up to its
/// second layer of bridge, which is [`second_internal_bridges`].
///
/// `beneath[i]` is layer `i` over every filament, `tops[i]` the height of
/// its top in mm, and `sparse_lines(i)` the sparse lines that will be laid
/// on layer `i` as its surfaces stand, over every filament: the reference
/// lays them again for this (`generate_sparse_infill_polylines_for_anchoring`),
/// and so is this asked for them only where the layer above has solid fill
/// worth bridging.
///
/// Step by step, as the reference:
/// 1. Each layer's solid fill is sorted for what is worth bridging, layer
///    by layer at once: what has neither the walls nor solid fill close
///    beneath it (`over.filter` says how close), grown by four spacings.
/// 2. The sparse lines of each layer under such a patch are laid.
/// 3. Each patch is cut to where the sparse fill beneath is deep enough
///    for a thick bridge's thread and has not been bridged within that
///    depth, and pieces that come nowhere near the middle of such fill
///    are dropped; it is turned to cross the sparse lines round it, and
///    each of its lines is run on, the way it goes, past the first sparse
///    line at either end. A patch whose run-out area comes within three
///    spacings of one already made on the layer takes that one's
///    direction. What it covers is kept to the layer's sparse and solid
///    fill, and is not reached into by the next patch's run-out. Layers
///    whose bridges could lie on each other are taken in order; the rest
///    at once.
/// 4. On each layer the bridges are cut from its sparse and solid fill,
///    and the sparse fill along the walls under a bridge's edge on the
///    layer above is made solid, so the ends of its lines have something
///    under them.
///
/// Different from the reference, where it does not do what it means to:
/// - Two patches on a layer may be run out over the same ground. The
///   reference then lays both, one over the other, unless they run the
///   same way; here the first to be made keeps it.
pub fn bridge_over_infill_by<E>(
    surfaces: &[Vec<Vec<Surface>>],
    beneath: &[Beneath],
    tops: &[f64],
    over: &OverInfill,
    sparse_lines: &(dyn Fn(usize) -> Result<Vec<Vec<Point2>>, E> + Sync),
) -> Result<Vec<Vec<Vec<Surface>>>, E>
where
    E: From<ClipError> + Send,
{
    let count = beneath.len();
    assert_eq!(tops.len(), count, "one height per layer");
    for own in surfaces {
        assert_eq!(own.len(), count, "one layer beneath per layer");
    }
    // 1. What is worth bridging, layer by layer.
    let candidates: Vec<Vec<Candidate>> = (0..count)
        .into_par_iter()
        .map(|layer| candidates_on(surfaces, beneath, layer, over))
        .collect::<Result<_, ClipError>>()?;
    let had: Vec<bool> = candidates.iter().map(|found| !found.is_empty()).collect();
    if !had.contains(&true) {
        return Ok(surfaces.to_vec());
    }
    // 2. The sparse lines under each.
    let wanted: Vec<usize> = (1..count).filter(|layer| had[*layer]).collect();
    let laid: Vec<Vec<Vec<Point2>>> = wanted
        .par_iter()
        .map(|layer| sparse_lines(layer - 1))
        .collect::<Result<_, E>>()?;
    let mut lines: Vec<Vec<Vec<Point2>>> = vec![Vec::new(); count];
    for (layer, found) in wanted.iter().zip(laid) {
        lines[layer - 1] = found;
    }
    // 3. Run out and turned. Layers close enough that a bridge on one
    // could lie on a bridge on the other, and whose patches come within
    // 7 mm of each other, go in order, each knowing what those under it
    // made; the reference's way of keeping its threads apart.
    let sunk = SINKS * over.thread_height;
    let covered: Vec<Vec<ExPolygon>> = wanted
        .par_iter()
        .map(|layer| {
            let reach = edge_geom::scaled(7.0);
            let boxes: Vec<Polygon> = candidates[*layer]
                .iter()
                .filter_map(|c| extent(&c.area))
                .map(|(low, high)| {
                    let (x0, y0, x1, y1) =
                        (low.x - reach, low.y - reach, high.x + reach, high.y + reach);
                    Polygon {
                        points: vec![
                            Point2 { x: x0, y: y0 },
                            Point2 { x: x1, y: y0 },
                            Point2 { x: x1, y: y1 },
                            Point2 { x: x0, y: y1 },
                        ],
                    }
                })
                .collect();
            edge_clip::union(&boxes)
        })
        .collect::<Result<_, ClipError>>()?;
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (job, &layer) in wanted.iter().enumerate() {
        let joined = match clusters.last() {
            Some(cluster) => {
                let last = *cluster.last().expect("clusters are never empty");
                tops[wanted[last]] >= tops[layer] - sunk - EPSILON
                    && !edge_clip::intersection(&covered[last], &covered[job])?.is_empty()
            }
            None => false,
        };
        match clusters.last_mut() {
            Some(cluster) if joined => cluster.push(job),
            _ => clusters.push(vec![job]),
        }
    }
    let made: Vec<Vec<(usize, Vec<Bridged>)>> = clusters
        .par_iter()
        .map(|cluster| {
            let mut done: Vec<(usize, Vec<Bridged>)> = Vec::with_capacity(cluster.len());
            for &job in cluster {
                let layer = wanted[job];
                // What was bridged on the layers of the cluster beneath
                // that a thread reaches down to.
                let mut lower = Vec::new();
                for (under, bridged) in done.iter().rev() {
                    if tops[*under] < tops[layer] - sunk - EPSILON {
                        break;
                    }
                    lower.extend(bridged.iter().flat_map(|b| b.area.iter().cloned()));
                }
                let bridged = bridges_on(
                    layer,
                    &candidates[layer],
                    surfaces,
                    beneath,
                    tops,
                    over,
                    &lines[layer - 1],
                    &lower,
                )?;
                done.push((layer, bridged));
            }
            Ok(done)
        })
        .collect::<Result<_, ClipError>>()?;
    let mut bridged: Vec<Vec<Bridged>> = vec![Vec::new(); count];
    for (layer, found) in made.into_iter().flatten() {
        bridged[layer] = found;
    }
    // 4. Cut in.
    let layers: Vec<Vec<Vec<Surface>>> = (0..count)
        .into_par_iter()
        .map(|layer| {
            if !had[layer] && !had.get(layer + 1).copied().unwrap_or(false) {
                return Ok(surfaces.iter().map(|own| own[layer].clone()).collect());
            }
            cut_in(surfaces, layer, &bridged, over.spacing)
        })
        .collect::<Result<_, ClipError>>()?;
    // From layer by filament back to filament by layer.
    let mut out: Vec<Vec<Vec<Surface>>> = surfaces
        .iter()
        .map(|own| Vec::with_capacity(own.len()))
        .collect();
    for layer in layers {
        for (own, changed) in layer.into_iter().zip(&mut out) {
            changed.push(own);
        }
    }
    Ok(out)
}

/// The patches of a layer's solid fill worth bridging, in the order of
/// the filaments and their surfaces (the first part of
/// `PrintObject::bridge_over_infill`, "gather and filter surfaces").
fn candidates_on(
    surfaces: &[Vec<Vec<Surface>>],
    beneath: &[Beneath],
    layer: usize,
    over: &OverInfill,
) -> Result<Vec<Candidate>, ClipError> {
    let solid = |s: &Surface| s.kind == SurfaceKind::InternalSolid;
    if layer == 0 || !surfaces.iter().any(|own| own[layer].iter().any(solid)) {
        return Ok(Vec::new());
    }
    let join = Join::MITER;
    let spacing = over.spacing;
    let unit = edge_geom::unscaled(spacing);
    // What the layer beneath gives no solid footing: its area inside the
    // walls, less what is solid there, both first shrunk so that slivers
    // neither count as footing nor are bridged for their own sake.
    let reach = if over.filter == InternalBridgeFilter::Filter {
        3
    } else {
        1
    };
    let below = &beneath[layer - 1];
    let footing = edge_clip::offset(
        &edge_clip::offset(&below.solid, -spacing, join)?,
        (1 + reach) * spacing,
        join,
    )?;
    let open = edge_clip::offset(
        &edge_clip::closing(&below.infill, SNAP, join)?,
        -reach * spacing,
        join,
    )?;
    let unsupported = edge_clip::difference(&open, &footing)?;
    let mut found = Vec::new();
    for (filament, own) in surfaces.iter().enumerate() {
        for surface in &own[layer] {
            if !solid(surface) {
                continue;
            }
            let whole = std::slice::from_ref(&surface.area);
            let over_area = if unsupported.is_empty() {
                Vec::new()
            } else {
                edge_clip::intersection(whole, &unsupported)?
            };
            if over.filter == InternalBridgeFilter::NoFilter {
                // Every patch, however small, with four spacings round
                // it wherever that falls.
                let area = edge_clip::offset(&over_area, 4 * spacing, join)?;
                if !area.is_empty() {
                    found.push(Candidate { filament, area });
                }
                continue;
            }
            if over_area.is_empty() {
                continue;
            }
            // Part held up and the rest small: not worth it. (The
            // reference compares the areas to its `EPSILON` in scaled
            // units squared.)
            let partly = area_of(&over_area) < surface.area.area_mm2() - 1e-12;
            if partly && area_of(&over_area) <= 9.0 * unit * unit {
                continue;
            }
            let mut worth =
                edge_clip::intersection(whole, &edge_clip::offset(&over_area, 4 * spacing, join)?)?;
            // Scraps of solid the bridge would leave beside it join it,
            // by their outlines.
            for scrap in edge_clip::difference(whole, &edge_clip::offset(&worth, spacing, join)?)? {
                let area = scrap.contour.signed_area_mm2().abs();
                if area < unit * 12.0 && area > unit * unit {
                    worth.push(ExPolygon {
                        contour: scrap.contour,
                        holes: Vec::new(),
                    });
                }
            }
            let worth = edge_clip::intersection(&edge_clip::closing(&worth, SNAP, join)?, whole)?;
            if !worth.is_empty() {
                found.push(Candidate {
                    filament,
                    area: worth,
                });
            }
        }
    }
    Ok(found)
}

/// One layer's candidates run out to their anchors and turned (the
/// third part of `PrintObject::bridge_over_infill`). `lines` are the
/// sparse lines of the layer beneath and `lower` what was bridged on the
/// layers beneath within a thread's reach.
#[allow(clippy::too_many_arguments)]
fn bridges_on(
    layer: usize,
    candidates: &[Candidate],
    surfaces: &[Vec<Vec<Surface>>],
    beneath: &[Beneath],
    tops: &[f64],
    over: &OverInfill,
    lines: &[Vec<Point2>],
    lower: &[ExPolygon],
) -> Result<Vec<Bridged>, ClipError> {
    let join = Join::MITER;
    // Left to right, then outward from the first: neighbours that would
    // compete for the same anchors come to choose the same direction.
    let mut order: Vec<(&Candidate, (Point2, Point2))> = candidates
        .iter()
        .map(|c| (c, extent(&c.area).unwrap_or_default()))
        .collect();
    order.sort_by_key(|(_, (low, _))| (low.x, low.y));
    if order.len() > 2 {
        let origin = order[0].1.1;
        let distance = |p: Point2| {
            let (dx, dy) = ((origin.x - p.x) as f64, (origin.y - p.y) as f64);
            dx * dx + dy * dy
        };
        order[1..].sort_by(|l, r| distance(l.1.0).total_cmp(&distance(r.1.0)));
    }
    let spacing = over.thread_spacing;
    let at = |share: f64| (share * spacing as f64) as Coord;
    // Sparse fill deep enough for a thread to sink into: sparse on every
    // layer it reaches, nothing else on any of them. The reference takes
    // the share a thread sinks twice over here (the height it hands
    // `gather_areas_w_depth` has it already); so it is here, the layer
    // beneath always counted.
    let sunk = SINKS * over.thread_height;
    let bottom = tops[layer] - SINKS * sunk - EPSILON;
    let (mut sparse, mut dense) = (Vec::new(), Vec::new());
    for under in (0..layer).rev() {
        if tops[under] < bottom && under + 1 < layer {
            break;
        }
        for own in surfaces {
            for surface in &own[under] {
                if surface.kind == SurfaceKind::Sparse {
                    sparse.push(surface.area.clone());
                } else {
                    dense.push(surface.area.clone());
                }
            }
        }
    }
    let mut deep = edge_clip::difference(
        &edge_clip::closing(&sparse, SNAP, join)?,
        &edge_clip::closing(&dense, SNAP, join)?,
    )?;
    if !lower.is_empty() {
        deep = edge_clip::difference(&deep, lower)?;
    }
    let deep = edge_clip::offset(&deep, at(1.5), join)?;
    // The layer's own sparse and solid fill, which a bridge may reach
    // into, its top surfaces, which it may not, and all of it.
    let (mut expansion, mut top) = (Vec::new(), Vec::new());
    for own in surfaces {
        for surface in &own[layer] {
            match surface.kind {
                SurfaceKind::Sparse | SurfaceKind::InternalSolid => {
                    expansion.push(surface.area.clone());
                }
                SurfaceKind::Top => top.push(surface.area.clone()),
                _ => {}
            }
        }
    }
    let total = edge_clip::closing(&beneath[layer].infill, SNAP, join)?;
    let mut expansion =
        edge_clip::intersection(&edge_clip::closing(&expansion, SNAP, join)?, &deep)?;
    // The sparse lines beneath that a bridge can rest on.
    let anchors = edge_clip::clip_lines(lines, &edge_clip::offset(&expansion, -spacing, join)?)?;
    let middle = edge_clip::offset(&deep, -at(4.5), join)?;
    let scan = match over.direction {
        BridgeDirection::AcrossPattern(_) => (spacing / 4).max(1),
        _ => spacing.max(1),
    };
    // How near the layer's fill has to come to a patch to be taken with
    // it: further than its outline is grown and its lines are run past
    // their anchors, with room to spare.
    let reach = 4 * spacing + over.thread_width;
    // The edges of all the layer's fill, a little outside it.
    let outline = closed_lines(&edge_clip::offset(&total, at(1.3), join)?);
    let mut made: Vec<Bridged> = Vec::with_capacity(order.len());
    for (candidate, _) in order {
        let mut pieces = Vec::new();
        for piece in
            edge_clip::intersection(&edge_clip::offset(&candidate.area, spacing, join)?, &deep)?
        {
            if !edge_clip::intersection(std::slice::from_ref(&piece), &middle)?.is_empty() {
                pieces.push(piece);
            }
        }
        if pieces.is_empty() {
            continue;
        }
        // What it may reach into: itself and the layer's fill, of which
        // only what lies near it, or near what lies near it, can matter
        // (see `nearby`).
        let mut limiting = pieces.clone();
        limiting.extend(nearby(&expansion, &pieces, reach).into_iter().cloned());
        let limiting = edge_clip::union_ex(&limiting)?;
        let mut boundary = outline.clone();
        boundary.extend(closed_lines(&edge_clip::offset(&limiting, at(0.3), join)?));
        let mut angle = None;
        if !anchors.is_empty() {
            match over.direction {
                BridgeDirection::AcrossPattern(direction) => {
                    let own_sparse: Vec<ExPolygon> = surfaces[candidate.filament][layer - 1]
                        .iter()
                        .filter(|s| s.kind == SurfaceKind::Sparse)
                        .map(|s| s.area.clone())
                        .collect();
                    if !edge_clip::intersection(&pieces, &own_sparse)?.is_empty() {
                        angle = Some((direction + FRAC_PI_2).rem_euclid(PI));
                    }
                }
                BridgeDirection::Fixed(direction) => angle = Some(direction),
                BridgeDirection::FromLines => {}
            }
        }
        let mut angle = angle.unwrap_or_else(|| {
            let by = if anchors.is_empty() {
                &boundary
            } else {
                &anchors
            };
            bridging_angle(&pieces, &segments_of_lines(by))
        });
        if let Some((set, relative)) = over.set_angle {
            angle = if relative { angle + set } else { set };
        }
        // Run out to the sparse lines, or to the edges of what it may
        // reach into where there are none.
        boundary.extend(anchors.iter().cloned());
        let held = segments_of_lines(&boundary);
        let run_out = |angle| anchored(&pieces, &held, spacing, over.thread_width, angle, scan);
        let mut area = run_out(angle)?;
        let near = edge_clip::offset(&area, 3 * spacing, join)?;
        for other in &made {
            if !edge_clip::intersection(&other.area, &near)?.is_empty() {
                angle = other.angle;
                area = run_out(angle)?;
                break;
            }
        }
        let area = edge_clip::opening(&area, at(0.75), join)?;
        let area = edge_clip::closing(&area, spacing, join)?;
        let area = edge_clip::intersection(&area, &limiting)?;
        let area = edge_clip::intersection(&area, &touching(&total, &area))?;
        let covered = touching(&top, &area);
        let area = if covered.is_empty() {
            area
        } else {
            edge_clip::difference(&area, &covered)?
        };
        if let Some(bounds) = extent(&area) {
            let (hit, missed): (Vec<ExPolygon>, Vec<ExPolygon>) = expansion
                .into_iter()
                .partition(|region| meets(&region.contour, bounds, 0));
            expansion = missed;
            expansion.extend(edge_clip::difference(&hit, &area)?);
        }
        made.push(Bridged {
            filament: candidate.filament,
            area,
            angle,
        });
    }
    Ok(made)
}

/// A layer once its bridges are cut into it, filament by filament (the
/// fourth part of `PrintObject::bridge_over_infill`): each filament's
/// surfaces that are neither sparse nor solid fill as they were, then its
/// sparse fill, its bridges and its solid fill.
fn cut_in(
    surfaces: &[Vec<Vec<Surface>>],
    layer: usize,
    bridged: &[Vec<Bridged>],
    spacing: Coord,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    let join = Join::MITER;
    let here = &bridged[layer];
    let cut: Vec<ExPolygon> = here.iter().flat_map(|b| b.area.iter().cloned()).collect();
    // The band round the inside of each bridge on the layer above.
    let mut edges = Vec::new();
    for bridge in bridged.get(layer + 1).into_iter().flatten() {
        edges.extend(edge_clip::difference(
            &bridge.area,
            &edge_clip::offset(&bridge.area, -spacing, join)?,
        )?);
    }
    // Bridges that run the same way on a filament are one, as the
    // reference's fill takes them (Fill.cpp `group_fills`); and ground two
    // would cover is the first's. They go in the order of the last of
    // each, so the last bridge on the layer is still last: a second layer
    // of bridge is laid square to it.
    let mut kept: Vec<(usize, f64, Vec<ExPolygon>, usize)> = Vec::new();
    let mut taken: Vec<ExPolygon> = Vec::new();
    for (index, bridge) in here.iter().enumerate() {
        if bridge.area.is_empty() {
            continue;
        }
        let area = if taken.is_empty() {
            edge_clip::union_ex(&bridge.area)?
        } else {
            edge_clip::difference(&bridge.area, &taken)?
        };
        taken.extend(bridge.area.iter().cloned());
        match kept
            .iter_mut()
            .find(|(filament, angle, _, _)| *filament == bridge.filament && *angle == bridge.angle)
        {
            Some((_, _, same, last)) => {
                same.extend(area);
                *last = index;
            }
            None => kept.push((bridge.filament, bridge.angle, area, index)),
        }
    }
    kept.sort_by_key(|(_, _, _, last)| *last);
    let mut out = Vec::with_capacity(surfaces.len());
    for (filament, own) in surfaces.iter().enumerate() {
        let own = &own[layer];
        let mut changed: Vec<Surface> = own
            .iter()
            .filter(|s| !matches!(s.kind, SurfaceKind::Sparse | SurfaceKind::InternalSolid))
            .cloned()
            .collect();
        let of = |kind| -> Vec<ExPolygon> {
            own.iter()
                .filter(|s| s.kind == kind)
                .map(|s| s.area.clone())
                .collect()
        };
        let (sparse, mut solid) = (of(SurfaceKind::Sparse), of(SurfaceKind::InternalSolid));
        // Under a bridge's edge, where it runs along the walls, the
        // sparse fill is made solid; widened by a line, so that where
        // walls lean and the two bands only just meet there is still room
        // to print.
        let mut carrying = Vec::new();
        if !edges.is_empty() && !sparse.is_empty() {
            let all: Vec<ExPolygon> = own.iter().map(|s| s.area.clone()).collect();
            let all = edge_clip::union_ex(&all)?;
            let beside_walls =
                edge_clip::difference(&all, &edge_clip::offset(&all, -spacing, join)?)?;
            carrying = edge_clip::intersection(
                &sparse,
                &edge_clip::offset(
                    &edge_clip::intersection(&edges, &beside_walls)?,
                    spacing,
                    join,
                )?,
            )?;
        }
        let mut sparse = sparse;
        if !cut.is_empty() {
            sparse = edge_clip::difference(&sparse, &cut)?;
        }
        if !carrying.is_empty() {
            sparse = edge_clip::difference(&sparse, &carrying)?;
        }
        changed.extend(
            sparse
                .into_iter()
                .map(|area| Surface::new(SurfaceKind::Sparse, area)),
        );
        for (_, angle, area, _) in kept.iter().filter(|(owner, _, _, _)| *owner == filament) {
            changed.extend(edge_clip::union_ex(area)?.into_iter().map(|area| Surface {
                kind: SurfaceKind::InternalBridge,
                area,
                angle: Some(*angle),
            }));
        }
        solid.extend(carrying);
        let solid = if cut.is_empty() {
            edge_clip::union_ex(&solid)?
        } else {
            edge_clip::difference(&solid, &cut)?
        };
        changed.extend(
            solid
                .into_iter()
                .map(|area| Surface::new(SurfaceKind::InternalSolid, area)),
        );
        out.push(changed);
    }
    Ok(out)
}

/// Lays a second layer of bridge over every bridge over infill: what of
/// the next layer's solid and sparse fill lies on a layer's bridges
/// becomes a bridge too, laid square to the first (the last part of
/// PrintObject.cpp `PrintObject::bridge_over_infill`, "Create a second
/// internal bridge layer above the first bridge layer").
/// `surfaces[f][i]` is filament `f`'s layer `i`, and a layer's bridges
/// are taken over every filament. `params` are a layer's above the
/// first: bridges, and patches of a second layer, narrower than twice
/// the solid fill's line width are passed over.
///
/// As in the reference, only a first bridge gets a second layer, and
/// where a layer's bridges are not all laid the same way the second
/// layer is laid square to the last of them.
///
/// Not as the reference has it: there, of a surface that gets a patch
/// of second bridge, whatever is not that patch is put through the same
/// sieve, and what is narrower than two lines is dropped from the layer
/// altogether and nothing is printed in its place. That is an error in
/// the reference (it means to drop bridges too small to matter, not
/// fill); here everything that is not the patch stays what it was.
pub fn second_internal_bridges(
    surfaces: &[Vec<Vec<Surface>>],
    params: &FillParams,
) -> Result<Vec<Vec<Vec<Surface>>>, ClipError> {
    let join = Join::MITER;
    let narrow = params.solid.scaled_width();
    let count = surfaces.iter().map(Vec::len).max().unwrap_or(0);
    // Layer by layer, each from the layer beneath as it was given: a
    // second layer is never read as a first.
    let layers = (0..count)
        .into_par_iter()
        .map(|layer| {
            let unchanged = || -> Vec<Vec<Surface>> {
                surfaces
                    .iter()
                    .map(|own| own.get(layer).cloned().unwrap_or_default())
                    .collect()
            };
            if layer == 0 {
                return Ok(unchanged());
            }
            let first: Vec<&Surface> = surfaces
                .iter()
                .flat_map(|own| own.get(layer - 1).into_iter().flatten())
                .filter(|s| s.kind == SurfaceKind::InternalBridge)
                .collect();
            let Some(last) = first.last() else {
                return Ok(unchanged());
            };
            let angle = last
                .angle
                .unwrap_or_else(|| params.internal_bridge_angle(layer - 1))
                + FRAC_PI_2;
            let bridged: Vec<ExPolygon> = first.iter().map(|s| s.area.clone()).collect();
            let bridged = edge_clip::opening(&bridged, narrow, join)?;
            if bridged.is_empty() {
                return Ok(unchanged());
            }
            surfaces
                .iter()
                .map(|own| {
                    let own = own.get(layer).map_or(&[][..], Vec::as_slice);
                    // What is left alone comes first, then what was cut.
                    let mut kept = Vec::with_capacity(own.len());
                    let (mut second, mut rests) = (Vec::new(), Vec::new());
                    for surface in own {
                        if !matches!(
                            surface.kind,
                            SurfaceKind::Sparse | SurfaceKind::InternalSolid
                        ) {
                            kept.push(surface.clone());
                        }
                    }
                    for surface in own {
                        if !matches!(
                            surface.kind,
                            SurfaceKind::Sparse | SurfaceKind::InternalSolid
                        ) {
                            continue;
                        }
                        let whole = std::slice::from_ref(&surface.area);
                        let over = edge_clip::opening(
                            &edge_clip::intersection(whole, &bridged)?,
                            narrow,
                            join,
                        )?;
                        if over.is_empty() {
                            kept.push(surface.clone());
                            continue;
                        }
                        let rest = edge_clip::difference(whole, &over)?;
                        second.extend(over);
                        rests.extend(
                            rest.into_iter()
                                .map(|area| Surface::new(surface.kind, area)),
                        );
                    }
                    // The pieces of second layer are one bridge, laid
                    // across them all as the reference's fill lays
                    // surfaces of a kind and direction (Fill.cpp
                    // `group_fills`), not piece by piece.
                    if !second.is_empty() {
                        kept.extend(edge_clip::union_ex(&second)?.into_iter().map(|area| {
                            Surface {
                                kind: SurfaceKind::InternalBridge,
                                area,
                                angle: Some(angle),
                            }
                        }));
                    }
                    kept.extend(rests);
                    Ok(kept)
                })
                .collect::<Result<Vec<Vec<Surface>>, ClipError>>()
        })
        .collect::<Result<Vec<Vec<Vec<Surface>>>, ClipError>>()?;
    // From layer by filament back to filament by layer.
    let mut out: Vec<Vec<Vec<Surface>>> = surfaces
        .iter()
        .map(|own| Vec::with_capacity(own.len()))
        .collect();
    for layer in layers {
        for ((own, given), changed) in layer.into_iter().zip(surfaces).zip(&mut out) {
            if changed.len() < given.len() {
                changed.push(own);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use edge_geom::{Point2, Polygon, scaled};

    use super::*;

    use InternalBridgeFilter::{Filter, Limited, NoFilter};
    use SurfaceKind::{InternalBridge, InternalSolid, Sparse};

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    /// Solid lines 0.4 mm apart, and the round thread of a thick bridge
    /// 0.4 mm across.
    fn over(filter: InternalBridgeFilter) -> OverInfill {
        OverInfill {
            spacing: scaled(0.4),
            filter,
            thread_spacing: scaled(0.4),
            thread_width: scaled(0.4),
            thread_height: 0.4,
            direction: BridgeDirection::FromLines,
            set_angle: None,
        }
    }

    fn area(layer: &[Surface], kind: SurfaceKind) -> f64 {
        layer
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.area.area_mm2())
            .sum()
    }

    fn total(layer: &[Surface]) -> f64 {
        layer.iter().map(|s| s.area.area_mm2()).sum()
    }

    /// Sparse lines `pitch` apart at `angle` degrees through the origin,
    /// cut to the sparse fill of a layer.
    fn lines_in(layer: &[Surface], angle: f64, pitch: f64) -> Vec<Vec<Point2>> {
        let (sin, cos) = angle.to_radians().sin_cos();
        let lines: Vec<Vec<Point2>> = (-100..100)
            .map(|i| {
                let offset = i as f64 * pitch;
                let (x, y) = (-sin * offset, cos * offset);
                vec![
                    Point2::from_mm(x - 200.0 * cos, y - 200.0 * sin),
                    Point2::from_mm(x + 200.0 * cos, y + 200.0 * sin),
                ]
            })
            .collect();
        let sparse: Vec<ExPolygon> = layer
            .iter()
            .filter(|s| s.kind == Sparse)
            .map(|s| s.area.clone())
            .collect();
        edge_clip::clip_lines(&lines, &sparse).unwrap()
    }

    /// Bridges over infill on a stack of layers 0.2 mm apart, every
    /// filament's area inside the walls on every layer `whole`, and the
    /// sparse lines a grid at 45° and 135° 2 mm apart.
    fn bridged(
        stacks: &[Vec<Vec<Surface>>],
        whole: &ExPolygon,
        over: &OverInfill,
    ) -> Vec<Vec<Vec<Surface>>> {
        bridged_with(stacks, whole, over, &|layer: &[Surface]| {
            let mut lines = lines_in(layer, 45.0, 2.0);
            lines.extend(lines_in(layer, 135.0, 2.0));
            lines
        })
    }

    fn bridged_with(
        stacks: &[Vec<Vec<Surface>>],
        whole: &ExPolygon,
        over: &OverInfill,
        lines: &(dyn Fn(&[Surface]) -> Vec<Vec<Point2>> + Sync),
    ) -> Vec<Vec<Vec<Surface>>> {
        let count = stacks[0].len();
        let beneath: Vec<Beneath> = (0..count)
            .map(|layer| {
                Beneath::gather(
                    stacks
                        .iter()
                        .map(|stack| (stack[layer].as_slice(), std::slice::from_ref(whole))),
                )
            })
            .collect();
        let tops: Vec<f64> = (0..count).map(|layer| 0.2 * (layer + 1) as f64).collect();
        let sparse_lines = |layer: usize| -> Result<Vec<Vec<Point2>>, Infallible> {
            let all: Vec<Surface> = stacks.iter().flat_map(|s| s[layer].clone()).collect();
            Ok(lines(&all))
        };
        match bridge_over_infill_by::<ClipError>(stacks, &beneath, &tops, over, &|layer| {
            Ok(sparse_lines(layer).unwrap())
        }) {
            Ok(found) => found,
            Err(error) => panic!("{error}"),
        }
    }

    /// A 30 mm square of sparse fill, and over it the same square with a
    /// strip of solid fill `width` wide from `from` in from its left
    /// edge, the rest sparse. Returns the upper layer as each filter
    /// leaves it.
    fn strip(from: f64, width: f64, filter: InternalBridgeFilter) -> Vec<Surface> {
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let solid = block(from, 0.0, from + width, 30.0);
        let mut upper = vec![Surface::new(InternalSolid, solid.clone())];
        upper.extend(
            edge_clip::difference(std::slice::from_ref(&whole), std::slice::from_ref(&solid))
                .unwrap()
                .into_iter()
                .map(|area| Surface::new(Sparse, area)),
        );
        let lower = vec![Surface::new(Sparse, whole.clone())];
        let found = bridged(&[vec![lower, upper]], &whole, &over(filter));
        let layer = found[0][1].clone();
        assert!((total(&layer) - 900.0).abs() < 1e-3, "{}", total(&layer));
        layer
    }

    #[test]
    fn a_whole_layer_of_solid_over_sparse_fill_is_one_bridge_across_the_lines() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let of = |kind| vec![Surface::new(kind, whole.clone())];
        let stack = vec![of(Sparse), of(InternalSolid), of(InternalSolid)];
        for filter in [Filter, Limited, NoFilter] {
            let found = bridged(std::slice::from_ref(&stack), &whole, &over(filter)).remove(0);
            let [bridge] = found[1].as_slice() else {
                panic!("{filter:?}: {:?}", found[1])
            };
            assert_eq!(bridge.kind, InternalBridge);
            assert!((bridge.area.area_mm2() - 400.0).abs() < 1e-3);
            // Across one set of the grid's lines, along the other.
            let angle = bridge.angle.unwrap().to_degrees();
            assert!(((angle - 45.0).rem_euclid(90.0)).abs() < 1e-6, "{angle}");
            // The layer over it rests on solid fill, and the one under it
            // is sparse but for the band along its walls under the
            // bridge's edge, which is made solid, a line wider than the
            // edge.
            assert_eq!(found[2], stack[2]);
            let band = 400.0 - 18.4 * 18.4;
            assert!((area(&found[0], InternalSolid) - band).abs() < 0.05);
            assert!((total(&found[0]) - 400.0).abs() < 1e-3);
        }
    }

    #[test]
    fn a_patch_is_run_out_to_the_sparse_lines_either_side() {
        // A 30 mm square of sparse fill, its lines along x every 5 mm,
        // and over it a 4 mm patch of solid fill in the middle.
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let patch = block(13.0, 13.0, 17.0, 17.0);
        let lower = vec![Surface::new(Sparse, whole.clone())];
        let mut upper = vec![Surface::new(InternalSolid, patch.clone())];
        upper.extend(
            edge_clip::difference(std::slice::from_ref(&whole), std::slice::from_ref(&patch))
                .unwrap()
                .into_iter()
                .map(|area| Surface::new(Sparse, area)),
        );
        let found = bridged_with(
            &[vec![lower, upper]],
            &whole,
            &over(Filter),
            &|layer: &[Surface]| lines_in(layer, 0.0, 5.0),
        )
        .remove(0);
        let layer = &found[1];
        let bridges: Vec<&Surface> = layer.iter().filter(|s| s.kind == InternalBridge).collect();
        assert_eq!(bridges.len(), 1);
        // Square to the lines beneath.
        assert!((bridges[0].angle.unwrap() - FRAC_PI_2).abs() < 1e-9);
        // Grown a line's spacing round, and run out along y to the lines
        // at 10 and 20 mm, a thread's width past each.
        let (low, high) = extent(std::slice::from_ref(&bridges[0].area)).unwrap();
        let (low_y, high_y) = (edge_geom::unscaled(low.y), edge_geom::unscaled(high.y));
        assert!((low_y - 9.6).abs() < 0.05, "{low_y}");
        assert!((high_y - 20.4).abs() < 0.05, "{high_y}");
        let (low_x, high_x) = (edge_geom::unscaled(low.x), edge_geom::unscaled(high.x));
        assert!(low_x > 12.0 && low_x < 13.0 && high_x > 17.0 && high_x < 18.0);
        // What it took of the sparse fill is no longer sparse, and the
        // layer is covered once.
        assert!(area(layer, InternalSolid) < 1e-6);
        assert!((total(layer) - 900.0).abs() < 1e-3);
    }

    #[test]
    fn a_strip_along_the_wall_is_bridged_only_with_less_filtering() {
        // Solid fill from 0.2 to 1.0 mm in from the wall: the default
        // counts sparse fill from 1.2 mm in and finds nothing under it.
        // Counting from 0.4 mm in there is 0.6 mm under it: both other
        // filters bridge it, run out into the sparse fill beside it.
        let narrow = |filter| strip(0.2, 0.8, filter);
        assert_eq!(area(&narrow(Filter), InternalBridge), 0.0);
        assert_eq!(narrow(Filter), strip(0.2, 0.8, Filter));
        for filter in [Limited, NoFilter] {
            let found = narrow(filter);
            assert!(area(&found, InternalBridge) > 24.0, "{filter:?}");
            assert_eq!(area(&found, InternalSolid), 0.0);
        }
        // A sliver 0.04 mm over sparse fill counted from 0.4 mm, 1.2 mm²
        // and under three spacings square, is bridged with no filter
        // only.
        let sliver = |filter| area(&strip(0.2, 0.24, filter), InternalBridge);
        assert_eq!(sliver(Filter), 0.0);
        assert_eq!(sliver(Limited), 0.0);
        assert!(sliver(NoFilter) > 0.0);
    }

    #[test]
    fn nothing_is_bridged_where_the_sparse_fill_beneath_is_too_narrow() {
        // A strip of solid over a band of sparse fill 2 mm wide between
        // two solid areas: no thread could sink far enough into it.
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let band = block(14.0, 0.0, 16.0, 30.0);
        let lower: Vec<Surface> = std::iter::once(Surface::new(Sparse, band.clone()))
            .chain(
                edge_clip::difference(std::slice::from_ref(&whole), std::slice::from_ref(&band))
                    .unwrap()
                    .into_iter()
                    .map(|area| Surface::new(InternalSolid, area)),
            )
            .collect();
        let upper = vec![Surface::new(InternalSolid, whole.clone())];
        for filter in [Filter, Limited, NoFilter] {
            let stack = vec![lower.clone(), upper.clone()];
            let found = bridged(std::slice::from_ref(&stack), &whole, &over(filter));
            assert_eq!(area(&found[0][1], InternalBridge), 0.0, "{filter:?}");
        }
    }

    #[test]
    fn a_bridge_is_not_laid_over_one_just_beneath_it() {
        // Sparse fill, then a patch of solid in the middle on each of the
        // next two layers. A thick bridge's thread is two layers deep:
        // the second patch would lie on the first's bridge, and is cut to
        // what is not over it.
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let layer = |patch: ExPolygon| -> Vec<Surface> {
            std::iter::once(Surface::new(InternalSolid, patch.clone()))
                .chain(
                    edge_clip::difference(
                        std::slice::from_ref(&whole),
                        std::slice::from_ref(&patch),
                    )
                    .unwrap()
                    .into_iter()
                    .map(|area| Surface::new(Sparse, area)),
                )
                .collect()
        };
        let stack = vec![
            vec![Surface::new(Sparse, whole.clone())],
            layer(block(10.0, 10.0, 16.0, 20.0)),
            layer(block(14.0, 10.0, 20.0, 20.0)),
        ];
        let deep = bridged(std::slice::from_ref(&stack), &whole, &over(NoFilter)).remove(0);
        let shallow = OverInfill {
            thread_height: 0.1,
            ..over(NoFilter)
        };
        let alone = bridged(std::slice::from_ref(&stack), &whole, &shallow).remove(0);
        assert!(area(&deep[1], InternalBridge) > 60.0);
        assert_eq!(deep[1], alone[1]);
        let (cut, whole_patch) = (
            area(&deep[2], InternalBridge),
            area(&alone[2], InternalBridge),
        );
        assert!(cut + 10.0 < whole_patch, "{cut} {whole_patch}");
    }

    #[test]
    fn every_filament_s_bridges_are_sorted_out_by_the_whole_layer_beneath() {
        // Two filaments side by side; the second's solid strip lies over
        // the first's sparse fill on the layer below.
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let right = block(15.0, 0.0, 30.0, 30.0);
        let first = vec![vec![Surface::new(Sparse, whole.clone())], Vec::new()];
        let strip = block(5.0, 0.0, 9.0, 30.0);
        let second = vec![
            Vec::new(),
            vec![
                Surface::new(InternalSolid, strip.clone()),
                Surface::new(Sparse, right.clone()),
            ],
        ];
        let found = bridged(&[first.clone(), second], &whole, &over(Limited));
        // The first filament's layer is left as it was, but for the band
        // under the bridge's edge along the walls.
        assert!((total(&found[0][0]) - 900.0).abs() < 1e-3);
        assert!(found[0][1].is_empty());
        // The second's strip is a bridge, and its own.
        assert!(area(&found[1][1], InternalBridge) >= 120.0 - 1e-3);
        assert!(area(&found[1][1], InternalSolid) < 1e-6);
    }

    #[test]
    fn a_set_angle_replaces_the_direction_found_or_turns_it() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let of = |kind| vec![Surface::new(kind, whole.clone())];
        let stack = vec![of(Sparse), of(InternalSolid)];
        let angle_with = |over: OverInfill| {
            bridged(std::slice::from_ref(&stack), &whole, &over).remove(0)[1][0]
                .angle
                .unwrap()
        };
        let found = angle_with(over(Filter));
        let set = |set_angle| OverInfill {
            set_angle,
            ..over(Filter)
        };
        assert_eq!(angle_with(set(Some((0.3, false)))), 0.3);
        assert!((angle_with(set(Some((0.3, true)))) - (found + 0.3)).abs() < 1e-12);
        // The patterns that turn every way are crossed square to their
        // own direction, and 3D honeycomb and cross hatch at theirs.
        let across = OverInfill {
            direction: BridgeDirection::AcrossPattern(0.2),
            ..over(Filter)
        };
        assert!((angle_with(across) - (0.2 + FRAC_PI_2)).abs() < 1e-12);
        let fixed = OverInfill {
            direction: BridgeDirection::Fixed(1.0),
            ..over(Filter)
        };
        assert_eq!(angle_with(fixed), 1.0);
    }

    /// What laying needs, with solid lines 0.45 mm wide and a grid of
    /// sparse lines at 45°.
    fn params() -> FillParams {
        crate::InfillSettings {
            pattern: crate::Pattern::Grid,
            ..crate::InfillSettings::default()
        }
        .params(0.2, false)
        .unwrap()
    }

    #[test]
    fn a_bridge_over_infill_gets_a_second_layer_laid_across_it() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let of = |kind| vec![Surface::new(kind, whole.clone())];
        // Sparse, a bridge over it, two layers of solid, the top.
        let stack = vec![
            of(Sparse),
            of(InternalBridge),
            of(InternalSolid),
            of(InternalSolid),
            of(SurfaceKind::Top),
        ];
        let found = second_internal_bridges(std::slice::from_ref(&stack), &params())
            .unwrap()
            .remove(0);
        // The layer over the bridge is bridge too, square to the first.
        let [second] = found[2].as_slice() else {
            panic!("expected one surface, got {}", found[2].len())
        };
        assert_eq!(second.kind, InternalBridge);
        assert!((second.area.area_mm2() - 400.0).abs() < 1e-6);
        let first = params().internal_bridge_angle(1);
        assert!((second.angle.unwrap() - (first + FRAC_PI_2)).abs() < 1e-12);
        // Nothing else changes: a second layer does not get a third.
        for layer in [0, 1, 3, 4] {
            assert_eq!(found[layer], stack[layer], "layer {layer}");
        }
    }

    #[test]
    fn only_what_lies_on_the_bridge_is_laid_again_and_the_rest_stays() {
        // A bridge 10 mm wide under solid fill 10.6 mm wide, sparse fill
        // beside both, and a top surface elsewhere on the upper layer.
        let bridge = block(0.0, 0.0, 10.0, 20.0);
        let lower = vec![
            Surface::new(InternalBridge, bridge.clone()),
            Surface::new(Sparse, block(10.0, 0.0, 30.0, 20.0)),
        ];
        let top = Surface::new(SurfaceKind::Top, block(30.0, 0.0, 40.0, 20.0));
        let upper = vec![
            Surface::new(InternalSolid, block(0.0, 0.0, 10.6, 20.0)),
            top.clone(),
            Surface::new(Sparse, block(10.6, 0.0, 30.0, 20.0)),
        ];
        let found = second_internal_bridges(&[vec![lower.clone(), upper]], &params())
            .unwrap()
            .remove(0);
        assert_eq!(found[0], lower);
        let layer = &found[1];
        assert!((area(layer, InternalBridge) - 200.0).abs() < 1e-6);
        // The 0.6 mm of solid beside the bridge, narrower than two lines,
        // is still solid: the reference drops it and prints nothing there.
        assert!((area(layer, InternalSolid) - 12.0).abs() < 1e-6);
        assert!((area(layer, Sparse) - 388.0).abs() < 1e-6);
        // What was left alone comes first, in its own order, then what
        // was cut.
        assert_eq!(layer[0], top);
        assert_eq!(layer[1].kind, Sparse);
        assert_eq!(
            layer[2..].iter().map(|s| s.kind).collect::<Vec<_>>(),
            [InternalBridge, InternalSolid]
        );
    }

    #[test]
    fn bridges_and_patches_narrower_than_two_lines_get_no_second_layer() {
        // A bridge 0.8 mm wide: shrunk by a line's 0.45 mm it is gone.
        let narrow = vec![
            vec![Surface::new(InternalBridge, block(0.0, 0.0, 0.8, 20.0))],
            vec![Surface::new(InternalSolid, block(0.0, 0.0, 20.0, 20.0))],
        ];
        let found = second_internal_bridges(std::slice::from_ref(&narrow), &params()).unwrap();
        assert_eq!(found[0], narrow);
        // A wide bridge under solid fill that covers 0.8 mm of it: the
        // patch is too narrow, and the solid fill is left whole.
        let grazed = vec![
            vec![Surface::new(InternalBridge, block(0.0, 0.0, 10.0, 20.0))],
            vec![Surface::new(InternalSolid, block(9.2, 0.0, 20.0, 20.0))],
        ];
        let found = second_internal_bridges(std::slice::from_ref(&grazed), &params()).unwrap();
        assert_eq!(found[0], grazed);
    }

    #[test]
    fn a_second_layer_lies_over_every_filament_s_bridges() {
        // The first filament's bridge under the second filament's solid
        // fill; the second filament prints nothing on the lower layer.
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let first = vec![
            vec![Surface::new(InternalBridge, whole.clone())],
            Vec::new(),
        ];
        let second = vec![Vec::new(), vec![Surface::new(InternalSolid, whole.clone())]];
        let found = second_internal_bridges(&[first.clone(), second], &params()).unwrap();
        assert_eq!(found[0], first);
        assert_eq!(found[1][0], Vec::new());
        assert_eq!(found[1][1].len(), 1);
        assert_eq!(found[1][1][0].kind, InternalBridge);
        // Lines one way on even layers and the other on odd: square to
        // the bridge on the odd layer beneath.
        let lines = FillParams {
            pattern: crate::Pattern::Lines,
            ..params()
        };
        let stack = vec![
            Vec::new(),
            vec![Surface::new(InternalBridge, whole.clone())],
            vec![Surface::new(InternalSolid, whole.clone())],
        ];
        let found = second_internal_bridges(std::slice::from_ref(&stack), &lines).unwrap();
        let angle = found[0][2][0].angle.unwrap();
        assert!((angle - (lines.sparse_angle + FRAC_PI_2 + FRAC_PI_2)).abs() < 1e-12);
    }
}
