//! Supports: scaffolding under whatever would otherwise be printed over air.
//!
//! Two styles, both first working versions of the reference's rather than
//! ports of it: normal supports, here, and tree supports, in `tree.rs`.
//! They share what decides whether a support works and comes off again:
//!
//! - what counts as an overhang (`support_threshold_angle`, or by overlap);
//! - the gap above a support, a whole number of layers, and the dense
//!   interface layers under it;
//! - the gaps beside and below a support wherever it passes the part.
//!
//! Normal supports are columns that stop where they meet the part on the
//! way down. Their areas are squared off to a grid one line spacing wide,
//! as the reference's default "grid" style does, which makes them a little
//! larger than what they hold up and gives every column whole lines to
//! stand on. The grid here is always square to the bed, whatever
//! `support_angle`.
//!
//! Not done: supports only where painted, the "snug" and the older tree
//! styles, bridges left unsupported, rafts, an outline round each normal
//! support, and support in a filament of its own.

mod settings;
mod tree;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, FlowError, Role};
use edge_geom::{ExPolygon, Point2, Polygon, scaled, unscaled};
use rayon::prelude::*;

pub use settings::{Style, SupportSettings};

#[derive(Debug, thiserror::Error)]
pub enum SupportError {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error(transparent)]
    Flow(#[from] FlowError),
}

/// Gap kept beside a support from the part's layers just above and below
/// it, which it does not share a height with (SupportMaterial.cpp
/// `no_overlap_xy_gap`).
const NEIGHBOUR_GAP: f64 = 0.2;
/// How far past its grid a support area reaches beyond half a line, in
/// scaled units, so the lines on the grid's edge fall inside it
/// (SupportMaterial.cpp `expansion_to_slice`).
const GRID_MARGIN: i64 = 5;
/// An overhang this long in any direction is kept however thin it is: it
/// sticks out with nothing under its far end.
const CANTILEVER: f64 = 3.0;

/// Extent of a region's contour in scaled units: min x, min y, max x, max y.
fn bounds(region: &ExPolygon) -> [i64; 4] {
    region.contour.points.iter().fold(
        [i64::MAX, i64::MAX, i64::MIN, i64::MIN],
        |[min_x, min_y, max_x, max_y], p| {
            [
                min_x.min(p.x),
                min_y.min(p.y),
                max_x.max(p.x),
                max_y.max(p.y),
            ]
        },
    )
}

/// Squares regions off to a grid of `cell`-sized squares fixed to the
/// plane: every square a region reaches into is taken whole. After the
/// reference's SupportGridPattern, which samples the same way.
fn snap(regions: &[ExPolygon], cell: i64) -> Result<Vec<ExPolygon>, SupportError> {
    if regions.is_empty() || cell <= 0 {
        return Ok(regions.to_vec());
    }
    // Rows of samples close enough together that nothing a line could be
    // printed on slips between them.
    let pitch = (cell / 8).max(1);
    let mut cells = std::collections::BTreeSet::new();
    for region in regions {
        let [_, min_y, _, max_y] = bounds(region);
        let mut crossings: Vec<i64> = Vec::new();
        let mut y = min_y.div_euclid(pitch) * pitch + pitch / 2;
        while y <= max_y {
            crossings.clear();
            for ring in region.rings() {
                let points = &ring.points;
                for index in 0..points.len() {
                    let (a, b) = (points[index], points[(index + 1) % points.len()]);
                    if (a.y <= y) != (b.y <= y) {
                        let along = (y - a.y) as f64 / (b.y - a.y) as f64;
                        crossings.push(a.x + ((b.x - a.x) as f64 * along).round() as i64);
                    }
                }
            }
            crossings.sort_unstable();
            for pair in crossings
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|pair| pair[1] > pair[0])
            {
                // An edge lying on a grid line reaches into no further
                // square, so squaring off twice changes nothing.
                for column in pair[0].div_euclid(cell)..=(pair[1] - 1).div_euclid(cell) {
                    cells.insert((y.div_euclid(cell), column));
                }
            }
            y += pitch;
        }
    }
    // One rectangle for each run of squares side by side.
    let mut blocks = Vec::new();
    let mut cells = cells.into_iter().peekable();
    while let Some((row, first)) = cells.next() {
        let mut last = first;
        while cells.peek() == Some(&(row, last + 1)) {
            cells.next();
            last += 1;
        }
        let (x0, x1, y0, y1) = (
            first * cell,
            (last + 1) * cell,
            row * cell,
            (row + 1) * cell,
        );
        blocks.push(ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2 { x, y })
                    .to_vec(),
            },
            holes: Vec::new(),
        });
    }
    Ok(edge_clip::union_ex(&blocks)?)
}

/// Of `regions`, those that share some area with `kept`.
fn touching(regions: Vec<ExPolygon>, kept: &[ExPolygon]) -> Result<Vec<ExPolygon>, SupportError> {
    let mut out = Vec::with_capacity(regions.len());
    for region in regions {
        if !edge_clip::intersection(std::slice::from_ref(&region), kept)?.is_empty() {
            out.push(region);
        }
    }
    Ok(out)
}

/// What one layer of the part needs holding up.
#[derive(Clone, Debug, Default)]
pub(crate) struct Overhang {
    /// Everything over air; a support column stands under all of it.
    pub whole: Vec<ExPolygon>,
    /// The part of it far enough out to need the dense interface.
    pub dense: Vec<ExPolygon>,
}

/// The overhang of a layer over the one below, after
/// `normal_support_region_overhangs` and `fill_contact_layer`.
fn overhang(
    current: &[ExPolygon],
    lower: &[ExPolygon],
    lower_height: f64,
    covered: Option<&[ExPolygon]>,
    settings: &SupportSettings,
    wall_width: f64,
) -> Result<Overhang, SupportError> {
    if current.is_empty() {
        return Ok(Overhang::default());
    }
    // How far a layer may reach past the one below unsupported: by the
    // threshold angle (the reference adds a degree), or, with no angle
    // set, by a share of the wall's width.
    let reach = if settings.threshold_angle > 0.0 {
        let angle = (settings.threshold_angle + 1.0).min(89.0);
        lower_height / angle.to_radians().tan()
    } else {
        wall_width
            - settings
                .threshold_overlap
                .resolve(wall_width)
                .min(wall_width)
    };
    let reach = scaled(reach.max(0.0));
    let mut beyond =
        edge_clip::difference(current, &edge_clip::offset(lower, reach, Join::Square)?)?;
    if let Some(covered) = covered {
        beyond = edge_clip::difference(&beyond, covered)?;
    }
    if beyond.is_empty() {
        return Ok(Overhang::default());
    }
    // Grown back to the edge of the layer below, so the support reaches
    // under the whole of what hangs, not just the part past the threshold.
    let grown =
        edge_clip::intersection(&edge_clip::offset(&beyond, reach, Join::Square)?, current)?;
    let mut whole = edge_clip::difference(&grown, lower)?;
    // Slivers thinner than a fifth of a wall hold nothing up.
    if edge_clip::offset(&whole, scaled(-0.1 * wall_width), Join::MITER)?.is_empty() {
        return Ok(Overhang::default());
    }
    if settings.expansion > 0.0 {
        whole = edge_clip::offset(&whole, scaled(settings.expansion), Join::Square)?;
    }
    // The dense top goes only where the overhang is more than about 0.6 of
    // a wall past the layer below; nearer in, the wall carries itself.
    let near = edge_clip::offset2(
        lower,
        scaled(-0.5 * wall_width),
        scaled(1.1 * wall_width),
        Join::MITER,
    )?;
    let far = edge_clip::difference(&whole, &near)?;
    let dense = edge_clip::intersection(
        &edge_clip::offset(&far, scaled(0.1 * wall_width), Join::MITER)?,
        &whole,
    )?;
    Ok(Overhang { whole, dense })
}

/// Drops overhangs too small to need support (`support_remove_small_overhang`):
/// groups of overhangs that touch from layer to layer and that, taken
/// together, are under two walls wide and not a cantilever.
///
/// The reference also keeps "sharp tails", islands that start in mid-air;
/// here only the size is looked at, and anything `CANTILEVER` long is kept.
fn drop_small(overhangs: &mut [Overhang], wall_width: f64) -> Result<(), SupportError> {
    // Every island, with its layer, extent and a copy grown by a wall.
    let mut islands = Vec::new();
    for (layer, overhang) in overhangs.iter().enumerate() {
        for (index, region) in overhang.whole.iter().enumerate() {
            let grown = edge_clip::offset(
                std::slice::from_ref(region),
                scaled(wall_width),
                Join::MITER,
            )?;
            islands.push((layer, index, bounds(region), grown));
        }
    }
    let mut group: Vec<usize> = (0..islands.len()).collect();
    fn root(group: &mut [usize], mut at: usize) -> usize {
        while group[at] != at {
            group[at] = group[group[at]];
            at = group[at];
        }
        at
    }
    let margin = scaled(wall_width);
    let mut from = 0;
    while from < islands.len() {
        let layer = islands[from].0;
        let until = islands[from..]
            .iter()
            .position(|island| island.0 != layer)
            .map_or(islands.len(), |offset| from + offset);
        let next_until = islands[until..]
            .iter()
            .position(|island| island.0 != layer + 1)
            .map_or(islands.len(), |offset| until + offset);
        let has_next = until < islands.len() && islands[until].0 == layer + 1;
        if has_next {
            for a in from..until {
                for b in until..next_until {
                    let ([a0, a1, a2, a3], [b0, b1, b2, b3]) = (islands[a].2, islands[b].2);
                    let apart = a0 - margin > b2 + margin
                        || b0 - margin > a2 + margin
                        || a1 - margin > b3 + margin
                        || b1 - margin > a3 + margin;
                    if apart {
                        continue;
                    }
                    if !edge_clip::intersection(&islands[a].3, &islands[b].3)?.is_empty() {
                        let (ra, rb) = (root(&mut group, a), root(&mut group, b));
                        group[ra.max(rb)] = ra.min(rb);
                    }
                }
            }
        }
        from = until;
    }
    // Each group's extent decides for all its islands.
    let mut extent: Vec<[i64; 4]> = vec![[i64::MAX, i64::MAX, i64::MIN, i64::MIN]; islands.len()];
    for (index, island) in islands.iter().enumerate() {
        let at = root(&mut group, index);
        let [a, b, c, d] = island.2;
        let [e, f, g, h] = extent[at];
        extent[at] = [a.min(e), b.min(f), c.max(g), d.max(h)];
    }
    let mut small: Vec<Vec<usize>> = vec![Vec::new(); overhangs.len()];
    for (index, island) in islands.iter().enumerate() {
        let at = root(&mut group, index);
        let [min_x, min_y, max_x, max_y] = extent[at];
        let (across, deep) = (unscaled(max_x - min_x), unscaled(max_y - min_y));
        if across.min(deep) < 2.0 * wall_width && across.max(deep) <= CANTILEVER {
            small[island.0].push(island.1);
        }
    }
    for (overhang, dropped) in overhangs.iter_mut().zip(small) {
        if dropped.is_empty() {
            continue;
        }
        let mut index = 0;
        overhang.whole.retain(|_| {
            index += 1;
            !dropped.contains(&(index - 1))
        });
        overhang.dense = edge_clip::intersection(&overhang.dense, &overhang.whole)?;
    }
    Ok(())
}

/// The support for every layer of a part. `slices` are the part's layers
/// from the bed up, the first `first_layer_height` mm thick and the rest
/// `layer_height`; the result has a layer's support lines at the same index.
///
/// `centre` is the middle of the part as seen from above. The grid that
/// supports are squared off to, and their lines, are laid out from there,
/// as the reference lays them out from the middle of each object, so a
/// part's supports do not change with where it sits on the bed.
pub fn generate(
    slices: &[&[ExPolygon]],
    centre: Point2,
    first_layer_height: f64,
    layer_height: f64,
    settings: &SupportSettings,
) -> Result<Vec<Vec<ExtrusionPath>>, SupportError> {
    if !settings.enabled || slices.len() < 2 {
        return Ok(vec![Vec::new(); slices.len()]);
    }
    let moved = |polygon: &Polygon| Polygon {
        points: polygon
            .points
            .iter()
            .map(|p| Point2 {
                x: p.x - centre.x,
                y: p.y - centre.y,
            })
            .collect(),
    };
    let local: Vec<Vec<ExPolygon>> = slices
        .iter()
        .map(|layer| {
            layer
                .iter()
                .map(|region| ExPolygon {
                    contour: moved(&region.contour),
                    holes: region.holes.iter().map(moved).collect(),
                })
                .collect()
        })
        .collect();
    let local: Vec<&[ExPolygon]> = local.iter().map(Vec::as_slice).collect();
    let bed = (settings.bed.len() >= 3).then(|| ExPolygon {
        contour: moved(&Polygon {
            points: settings
                .bed
                .iter()
                .map(|(x, y)| Point2::from_mm(*x, *y))
                .collect(),
        }),
        holes: Vec::new(),
    });
    let mut supports = about_origin(&local, first_layer_height, layer_height, settings, bed)?;
    for point in supports
        .iter_mut()
        .flatten()
        .flat_map(|path| &mut path.points)
    {
        point.x += centre.x;
        point.y += centre.y;
    }
    Ok(supports)
}

/// A part to be supported, its middle at the origin, and how.
pub(crate) struct Job<'a> {
    pub slices: &'a [&'a [ExPolygon]],
    pub first_layer_height: f64,
    pub layer_height: f64,
    pub settings: &'a SupportSettings,
    /// Layers of air left above a support and below one that stands on
    /// the part: `top_z_distance` and `bottom_z_distance` in whole layers.
    pub gap_above: usize,
    pub gap_below: usize,
    /// The bed's outline, the part's middle at the origin, when known.
    pub bed: Option<ExPolygon>,
}

impl Job<'_> {
    pub fn height_of(&self, index: usize) -> f64 {
        if index == 0 {
            self.first_layer_height
        } else {
            self.layer_height
        }
    }

    /// Where no support may be on a layer: within the set distance of the
    /// part on that layer, and within a little of the part on the layers
    /// inside the gaps above and below. The second is also what leaves the
    /// gap under a support that rests on the part.
    pub fn keep_off(&self, index: usize) -> Result<Vec<ExPolygon>, SupportError> {
        let mut near_part = edge_clip::offset(
            self.slices[index],
            scaled(self.settings.xy_distance),
            Join::MITER,
        )?;
        let last = (index + self.gap_above).min(self.slices.len() - 1);
        for near in (index.saturating_sub(self.gap_below)..=last).filter(|near| *near != index) {
            near_part.extend(edge_clip::offset(
                self.slices[near],
                scaled(NEIGHBOUR_GAP),
                Join::MITER,
            )?);
        }
        Ok(edge_clip::union_ex(&near_part)?)
    }
}

/// [`generate`] for a part whose middle is at the origin.
fn about_origin(
    slices: &[&[ExPolygon]],
    first_layer_height: f64,
    layer_height: f64,
    settings: &SupportSettings,
    bed: Option<ExPolygon>,
) -> Result<Vec<Vec<ExtrusionPath>>, SupportError> {
    // A support's top sits a whole number of layers under what it holds
    // up, leaving `top_z_distance` of air: an overhang on layer L is met by
    // a contact on layer L - 1 - gap.
    let layers_in = |distance: f64| (distance / layer_height + 1e-4).round().max(0.0) as usize;
    let job = Job {
        slices,
        first_layer_height,
        layer_height,
        settings,
        gap_above: layers_in(settings.top_z_distance),
        gap_below: layers_in(settings.bottom_z_distance),
        bed,
    };
    match settings.style {
        Style::Normal => normal(&job, overhangs(&job)?),
        Style::Tree => tree::build(&job),
    }
}

/// What every layer of the part needs holding up.
fn overhangs(job: &Job<'_>) -> Result<Vec<Overhang>, SupportError> {
    let (slices, settings) = (job.slices, job.settings);
    let count = slices.len();
    let wall_width = settings.wall_width(job.layer_height)?;

    let detect = |index: usize, covered: Option<&[ExPolygon]>| {
        if index == 0 {
            return Ok(Overhang::default());
        }
        overhang(
            slices[index],
            slices[index - 1],
            job.height_of(index - 1),
            covered,
            settings,
            wall_width,
        )
    };
    let mut overhangs: Vec<Overhang> = (0..count)
        .into_par_iter()
        .map(|index| detect(index, None))
        .collect::<Result<_, SupportError>>()?;
    // With supports on the build plate only, nothing is held up that has
    // any of the part beneath it. What lies beneath a layer is everything
    // under it put together, which is only worth working out for the
    // layers that have an overhang at all.
    let highest = overhangs
        .iter()
        .rposition(|overhang| !overhang.whole.is_empty());
    if let (true, Some(highest)) = (settings.on_build_plate_only, highest) {
        let mut beneath: Vec<ExPolygon> = Vec::new();
        for index in 1..=highest {
            beneath.extend(edge_clip::offset(
                slices[index - 1],
                scaled(0.01),
                Join::MITER,
            )?);
            if !overhangs[index].whole.is_empty() {
                beneath = edge_clip::union_ex(&beneath)?;
                overhangs[index] = detect(index, Some(&beneath))?;
            }
        }
    }
    if settings.remove_small_overhang {
        drop_small(&mut overhangs, wall_width)?;
    }
    Ok(overhangs)
}

/// Normal supports: columns squared off to a grid and filled with lines.
fn normal(
    job: &Job<'_>,
    overhangs: Vec<Overhang>,
) -> Result<Vec<Vec<ExtrusionPath>>, SupportError> {
    let (slices, settings, layer_height) = (job.slices, job.settings, job.layer_height);
    let (gap_above, gap_below) = (job.gap_above, job.gap_below);
    let count = slices.len();
    let height_of = |index: usize| job.height_of(index);
    let flow = settings.flow(layer_height, false)?;
    // The grid supports are squared off to: one sparse line spacing wide.
    let cell = scaled(flow.spacing + settings.base_spacing);
    let mut contacts: Vec<Vec<ExPolygon>> = vec![Vec::new(); count];
    let mut dense: Vec<Vec<ExPolygon>> = vec![Vec::new(); count];
    for (layer, overhang) in overhangs.into_iter().enumerate() {
        // An overhang too near the bed for a support to fit under is left.
        let Some(at) = layer.checked_sub(1 + gap_above) else {
            continue;
        };
        contacts[at].extend(snap(&overhang.whole, cell)?);
        dense[at].extend(snap(&overhang.dense, cell)?);
    }

    // Each contact stands on a column that goes down until it meets the
    // part, and no further: what is cut off stays cut off below.
    let stick = scaled(0.5 * flow.width);
    let mut columns: Vec<Vec<ExPolygon>> = vec![Vec::new(); count];
    let mut above: Vec<ExPolygon> = Vec::new();
    for index in (0..count).rev() {
        above.extend(contacts[index].iter().cloned());
        if above.is_empty() {
            continue;
        }
        let part = edge_clip::offset(slices[index], 1, Join::MITER)?;
        let clear = edge_clip::difference(&edge_clip::union_ex(&above)?, &part)?;
        // Slivers left by the cut carry nothing.
        let clear = edge_clip::opening(&clear, stick, Join::MITER)?;
        // Squared off again, but never into a place the column did not
        // already reach: a square the part cuts in two keeps only the side
        // the column is on.
        let squared = edge_clip::difference(&snap(&clear, cell)?, &part)?;
        above = touching(squared, &clear)?;
        columns[index] = above.clone();
    }

    // Where a column comes to rest on the part, a line's width wider:
    // the part there is one layer and the gap below the column's foot.
    let mut feet: Vec<Vec<ExPolygon>> = vec![Vec::new(); count];
    if settings.interface_bottom_layers > 0 {
        for index in (1 + gap_below)..count {
            if columns[index].is_empty() {
                continue;
            }
            let landing = edge_clip::intersection(&columns[index], slices[index - 1 - gap_below])?;
            feet[index] = edge_clip::offset(&landing, flow.scaled_width(), Join::Square)?;
        }
    }

    (0..count)
        .into_par_iter()
        .map(|index| {
            if columns[index].is_empty() {
                return Ok(Vec::new());
            }
            let keep_off = job.keep_off(index)?;
            let first = index == 0;
            let flow = settings.flow(height_of(index), first)?;
            // Half a line beyond the grid, so the outermost lines lie on it.
            let reach = flow.scaled_spacing() / 2 + GRID_MARGIN;
            let mut area = edge_clip::difference(
                &edge_clip::offset(&columns[index], reach, Join::MITER)?,
                &keep_off,
            )?;
            let mut paths = Vec::new();
            if first {
                // On the bed a support spreads out for a firmer footing,
                // keeping its own small distance from the part.
                let grown = edge_clip::offset(
                    &area,
                    scaled(settings.first_layer_expansion),
                    Join::Round {
                        tolerance: scaled(0.05),
                    },
                )?;
                area = edge_clip::difference(
                    &grown,
                    &edge_clip::offset(slices[0], scaled(settings.first_layer_gap), Join::MITER)?,
                )?;
                let spacing = flow.spacing / settings.first_layer_density.clamp(0.1, 1.0);
                fill(
                    &area,
                    spacing,
                    settings.angle + 90.0,
                    Role::Support,
                    &flow,
                    false,
                    &mut paths,
                )?;
                return Ok(paths);
            }

            // The contact and the layers just under it are dense, so the
            // part has something even to rest on; the rest is sparse.
            let top = (index + settings.interface_top_layers).min(count);
            let near_top: Vec<ExPolygon> = dense[index..top].iter().flatten().cloned().collect();
            let interface = if near_top.is_empty() {
                Vec::new()
            } else {
                edge_clip::intersection(
                    &area,
                    &edge_clip::offset(&edge_clip::union_ex(&near_top)?, reach, Join::MITER)?,
                )?
            };
            // The first layers of a support standing on the part are dense
            // too, so it comes away from the surface under it cleanly.
            let low = index.saturating_sub(settings.interface_bottom_layers.saturating_sub(1));
            let near_foot: Vec<ExPolygon> = feet[low..=index].iter().flatten().cloned().collect();
            let foot = if near_foot.is_empty() {
                Vec::new()
            } else {
                edge_clip::difference(
                    &edge_clip::intersection(&area, &edge_clip::union_ex(&near_foot)?)?,
                    &interface,
                )?
            };
            let base = edge_clip::difference(&edge_clip::difference(&area, &interface)?, &foot)?;
            fill(
                &foot,
                flow.spacing + settings.bottom_interface_spacing,
                settings.angle + 90.0,
                Role::SupportInterface,
                &flow,
                true,
                &mut paths,
            )?;
            fill(
                &base,
                flow.spacing + settings.base_spacing,
                settings.angle,
                Role::Support,
                &flow,
                false,
                &mut paths,
            )?;
            fill(
                &interface,
                flow.spacing + settings.interface_spacing,
                settings.angle + 90.0,
                Role::SupportInterface,
                &flow,
                true,
                &mut paths,
            )?;
            Ok(paths)
        })
        .collect()
}

/// Lines `spacing` mm apart at `angle` degrees across each region, joined
/// into zigzags. A region too narrow for a line to cross still gets its
/// outline, so nothing above it is left standing on air.
///
/// Lines lie on one grid for the whole plane unless `anchored`, when each
/// region's lines start at its own edge, so the edge of a dense layer is
/// held up wherever it happens to fall.
pub(crate) fn fill(
    regions: &[ExPolygon],
    spacing: f64,
    angle: f64,
    role: Role,
    flow: &Flow,
    anchored: bool,
    paths: &mut Vec<ExtrusionPath>,
) -> Result<(), SupportError> {
    // Lines stop half a line spacing short of the edge, as the reference's
    // do; areas reach that much past the grid, plus a hair, so a line on
    // the grid's own edge is kept.
    let half = flow.scaled_spacing() / 2;
    let (sin, cos) = angle.to_radians().sin_cos();
    for mut region in edge_clip::offset(regions, -half, Join::MITER)? {
        // Rows lie where this is a whole number of spacings; the region is
        // moved so that its edge is just short of one, and the lines are
        // moved back.
        let across = |p: &Point2| -unscaled(p.x) * sin + unscaled(p.y) * cos;
        let (dx, dy) = if anchored {
            let low = region
                .contour
                .points
                .iter()
                .map(across)
                .fold(f64::INFINITY, f64::min);
            let slide = (low / spacing).ceil() * spacing - low - 0.002;
            (scaled(-sin * slide), scaled(cos * slide))
        } else {
            (0, 0)
        };
        if (dx, dy) != (0, 0) {
            for ring in std::iter::once(&mut region.contour).chain(&mut region.holes) {
                for point in &mut ring.points {
                    *point = Point2 {
                        x: point.x + dx,
                        y: point.y + dy,
                    };
                }
            }
        }
        let mut lines = edge_infill::lines::paths(
            &region,
            scaled(spacing),
            &[angle.to_radians()],
            half,
            scaled(3.0 * spacing),
        );
        for point in lines.iter_mut().flatten().chain(
            std::iter::once(&mut region.contour)
                .chain(&mut region.holes)
                .flat_map(|ring| &mut ring.points),
        ) {
            *point = Point2 {
                x: point.x - dx,
                y: point.y - dy,
            };
        }
        if lines.is_empty() {
            let mut points = region.contour.points;
            if points.len() >= 3 {
                points.push(points[0]);
                paths.push(ExtrusionPath {
                    points,
                    role,
                    flow: *flow,
                    in_turn: false,
                    part: 0,
                });
            }
            continue;
        }
        paths.extend(lines.into_iter().map(|points| ExtrusionPath {
            points,
            role,
            flow: *flow,
            in_turn: false,
            part: 0,
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
