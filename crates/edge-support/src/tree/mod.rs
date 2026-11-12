//! Organic tree supports, a port of the reference's (TreeSupport3D.cpp,
//! TreeModelVolumes.cpp), which is what `tree(auto)` with the default
//! style builds (SupportParameters.hpp: "Orca: use organic as default").
//!
//! The steps are the reference's, each in its own module:
//!
//! - `overhangs`: what is held up (TreeSupport::detect_overhangs);
//! - `volumes`: where a branch of each radius may be on each layer, worked
//!   out up front (TreeModelVolumes);
//! - `tips`: roofs under the overhangs and tips under them
//!   (generate_initial_areas);
//! - `pathing`: every branch carried down as the whole area its centre
//!   could be in, areas that meet merged (create_layer_pathing);
//! - `nodes`: a centre picked in each area from the bed up
//!   (create_nodes_from_area);
//! - `branch`: the branches smoothed, built as tubes and sliced
//!   (organic_draw_branches);
//! - this module: dense layers at the top and foot
//!   (generate_interface_layers), the first layer spread out
//!   (generate_raft_base) and the lines (generate_support_toolpaths,
//!   with `paths` for a branch's walls).

mod branch;
mod config;
mod element;
mod geometry;
mod nodes;
mod overhangs;
mod pathing;
mod paths;
mod tips;
mod volumes;

use edge_clip::Join;
use edge_extrusion::{ExtrusionPath, Role};
use edge_geom::{ExPolygon, scaled};
use rayon::prelude::*;

use crate::{Job, SupportError, fill};
use config::{EPSILON, SCALED_EPSILON, TreeConfig};
use geometry::{Polys, diff, intersection, offset, simplify, union};

/// An island of branches larger than this, in mm², gets a second wall
/// (SupportParameters.hpp `tree_branch_diameter_double_wall_area_scaled`).
const DOUBLE_WALL_AREA: f64 = 0.25 * std::f64::consts::PI * 25.0;

/// What the trees put on each layer, before it is turned into lines.
struct Layers {
    base: Vec<Polys>,
    /// Dense layers: the contact right under what is held up, and the
    /// interfaces under it and at a foot on the part.
    contacts: Vec<Polys>,
    interfaces: Vec<Polys>,
    /// Interfaces that come from a foot, laid at the foot's spacing.
    bottom_interfaces: Vec<Polys>,
}

pub(crate) fn build(job: &Job<'_>) -> Result<Vec<Vec<ExtrusionPath>>, SupportError> {
    let count = job.slices.len();
    let none = || vec![Vec::new(); count];
    let mut clock = Clock::new();
    let config = TreeConfig::new(job)?;
    let overhangs = overhangs::detect(job)?;
    clock.lap("overhangs");
    // The highest layer a tip can be on (`precalculate`).
    let Some(highest) = overhangs
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, overhang)| !overhang.is_empty())
        .map(|(layer, _)| layer)
        .max()
    else {
        return Ok(none());
    };
    let max_layer = highest.saturating_sub(config.z_distance_top_layers);
    if max_layer == 0 {
        return Ok(none());
    }
    let mut volumes = volumes::Volumes::new(job.slices, &config)?;
    volumes.precalculate(&config, max_layer)?;
    clock.lap("volumes");
    let tips = tips::place(
        &volumes,
        &config,
        &overhangs,
        job.settings.angle.to_radians(),
    )?;
    clock.lap("tips");
    let mut elements = tips.layers;
    elements.resize(max_layer, Vec::new());
    pathing::create_layer_pathing(&volumes, &config, &mut elements)?;
    clock.lap("pathing");
    nodes::create_nodes_from_area(&volumes, &config, &mut elements)?;
    clock.lap("nodes");
    let drawn = branch::draw(job, &volumes, &config, &mut elements)?;
    clock.lap("draw");
    let layers = finish(job, &config, drawn, tips.roofs)?;
    clock.lap("interfaces");
    let mut out = toolpaths(job, &config, &layers)?;
    clock.lap("toolpaths");
    out.resize(count, Vec::new());
    Ok(out)
}

/// Times each step when `EDGE_TREE_TIMES` is set, to stderr.
struct Clock {
    on: bool,
    last: std::time::Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            on: std::env::var_os("EDGE_TREE_TIMES").is_some(),
            last: std::time::Instant::now(),
        }
    }

    fn lap(&mut self, step: &str) {
        if self.on {
            eprintln!(
                "tree supports: {step} {:.1} ms",
                self.last.elapsed().as_secs_f64() * 1e3
            );
            self.last = std::time::Instant::now();
        }
    }
}

/// `organic_draw_branches`' last step and `generate_interface_layers`:
/// each layer's body, its contacts taken out, and the dense layers.
fn finish(
    job: &Job<'_>,
    config: &TreeConfig,
    drawn: branch::Drawn,
    roofs: tips::Roofs,
) -> Result<Layers, SupportError> {
    let count = drawn
        .base
        .len()
        .max(roofs.contacts.len())
        .max(roofs.interfaces.len());
    let at = |layers: &[Polys], layer: usize| layers.get(layer).cloned().unwrap_or_default();
    let contacts: Vec<Polys> = (0..count)
        .into_par_iter()
        .map(|layer| union(&at(&roofs.contacts, layer)))
        .collect::<Result<_, _>>()?;
    let roof_interfaces: Vec<Polys> = (0..count)
        .into_par_iter()
        .map(|layer| union(&at(&roofs.interfaces, layer)))
        .collect::<Result<_, _>>()?;
    // The body, less the contacts, and the feet.
    let simplify_to = scaled(0.03).min(config.resolution);
    let pieces: Vec<(Polys, Polys)> = (0..count)
        .into_par_iter()
        .map(|layer| -> Result<(Polys, Polys), SupportError> {
            // Both come united from the drawing.
            let mut base = simplify(&at(&drawn.base, layer), simplify_to)?;
            let mut feet = at(&drawn.bottom_contacts, layer);
            if !contacts[layer].is_empty() && !base.is_empty() {
                base = diff(&base, &contacts[layer])?;
                feet = diff(&feet, &contacts[layer])?;
            }
            if !feet.is_empty() {
                base = diff(&base, &feet)?;
            }
            Ok((base, feet))
        })
        .collect::<Result<_, _>>()?;
    let (mut base, feet): (Vec<Polys>, Vec<Polys>) = pieces.into_iter().unzip();

    // Dense layers over the body (generate_interface_layers): the contacts
    // of the layer and the one above projected onto it, and the feet of the
    // layers below, each closed up and rounded, inside the body; and the
    // roofs the tips were placed under.
    let settings = job.settings;
    let flow = settings.flow(job.layer_height, false)?;
    let spacing = flow.scaled_spacing();
    let top_density = (flow.spacing / (flow.spacing + settings.interface_spacing)).min(1.0);
    let bottom_density =
        (flow.spacing / (flow.spacing + settings.bottom_interface_spacing)).min(1.0);
    let smoothing = (spacing as f64 * 1.5) as i64;
    let island_top = (spacing as f64 / top_density) as i64;
    let island_bottom = (spacing as f64 / bottom_density) as i64;
    let num_top = config.roof_layers.saturating_sub(1);
    let num_bottom = config.floor_layers;
    let present: Vec<usize> = (0..count)
        .filter(|layer| !base[*layer].is_empty())
        .collect();
    let regularize = |polys: Polys, radius: i64| -> Result<Polys, SupportError> {
        if polys.is_empty() {
            return Ok(polys);
        }
        let closed = offset(
            &offset(&polys, smoothing + radius, Join::Square)?,
            -smoothing,
            Join::Square,
        )?;
        smooth_outward(&closed, smoothing)
    };
    let found: Vec<(usize, Polys, bool)> = present
        .par_iter()
        .enumerate()
        .map(
            |(k, &layer)| -> Result<(usize, Polys, bool), SupportError> {
                let mut top = Vec::new();
                if num_top > 0 {
                    let top_layer = present[(k + num_top - 1).min(present.len() - 1)];
                    for contact in &contacts[layer..=(top_layer + 1).min(count - 1)] {
                        top.extend(contact.iter().cloned());
                    }
                }
                let mut bottom = Vec::new();
                if num_bottom > 0 && layer > 0 {
                    let lowest = present[k.saturating_sub(num_bottom - 1)].saturating_sub(1);
                    for foot in &feet[lowest..layer] {
                        bottom.extend(foot.iter().cloned());
                    }
                }
                let from_feet_only = top.is_empty();
                let mut dense = regularize(bottom, island_bottom)?;
                dense.extend(regularize(top, island_top)?);
                let mut dense = intersection(&dense, &base[layer])?;
                if !roof_interfaces[layer].is_empty() {
                    dense = geometry::union2(&roof_interfaces[layer], &dense)?;
                }
                Ok((
                    layer,
                    dense,
                    from_feet_only && roof_interfaces[layer].is_empty(),
                ))
            },
        )
        .collect::<Result<_, _>>()?;
    let mut interfaces: Vec<Polys> = roof_interfaces;
    let mut bottom_interfaces: Vec<Polys> = vec![Vec::new(); count];
    for (layer, dense, from_feet) in found {
        if dense.is_empty() {
            continue;
        }
        base[layer] = diff(&base[layer], &dense)?;
        if from_feet {
            bottom_interfaces[layer] = dense;
            interfaces[layer].clear();
        } else {
            interfaces[layer] = dense;
        }
    }
    Ok(Layers {
        base,
        contacts,
        interfaces,
        bottom_interfaces,
    })
}

/// Rounds off concave corners sharper than 135 degrees by up to `distance`
/// (MutablePolygon.cpp `smooth_outward`). Here a closing by half the
/// distance stands in for the reference's corner by corner clipping:
/// both fill the same notches; the reference's cuts them straight.
fn smooth_outward(polys: &[ExPolygon], distance: i64) -> Result<Polys, SupportError> {
    let half = distance / 2;
    offset(&offset(polys, half, Join::MITER)?, -half, Join::MITER)
}

/// The lines of every layer (`generate_raft_base` for the first layer and
/// `generate_support_toolpaths`).
fn toolpaths(
    job: &Job<'_>,
    config: &TreeConfig,
    layers: &Layers,
) -> Result<Vec<Vec<ExtrusionPath>>, SupportError> {
    let settings = job.settings;
    let count = layers.base.len();
    (0..count)
        .into_par_iter()
        .map(|layer| -> Result<Vec<ExtrusionPath>, SupportError> {
            let first = layer == 0;
            let flow = settings.flow(job.height_of(layer), first)?;
            let mut paths = Vec::new();
            let mut base = layers.base[layer].clone();
            let mut contacts = layers.contacts[layer].clone();
            let mut interfaces = layers.interfaces[layer].clone();
            let bottom_interfaces = layers.bottom_interfaces[layer].clone();
            if first {
                // On the bed the body spreads out in steps, so that it does
                // not leak through a wall, keeping its own small gap from
                // the part.
                let mut dense = offset(&contacts, SCALED_EPSILON, Join::Square)?;
                dense.extend(offset(&interfaces, SCALED_EPSILON, Join::Square)?);
                let inflate = scaled(settings.first_layer_expansion) - scaled(EPSILON);
                if !base.is_empty() {
                    let part = job.slices.first().copied().unwrap_or(&[]);
                    let trimming = offset(part, scaled(settings.first_layer_gap), Join::Square)?;
                    if inflate > SCALED_EPSILON {
                        let steps =
                            5.max((inflate as f64 / flow.scaled_width() as f64).ceil() as i64);
                        let step = inflate / steps;
                        for _ in 0..steps {
                            base = diff(&offset(&base, step, Join::MITER)?, &trimming)?;
                        }
                    } else {
                        base = diff(&base, &trimming)?;
                    }
                    if !dense.is_empty() {
                        base = diff(&base, &dense)?;
                    }
                    if let Some(bed) = &job.bed {
                        base = intersection(&base, std::slice::from_ref(bed))?;
                    }
                }
                // Outlined, and filled at the first layer's density.
                paths::walls(&base, &flow, false, false, &mut paths)?;
                let spacing = flow.scaled_spacing();
                let inner = offset(
                    &edge_clip::offset2(
                        &base,
                        SCALED_EPSILON,
                        -(SCALED_EPSILON + flow.scaled_width() / 2),
                        Join::MITER,
                    )?,
                    -(0.4 * spacing as f64) as i64,
                    Join::MITER,
                )?;
                fill(
                    &inner,
                    flow.spacing / settings.first_layer_density.clamp(0.1, 1.0),
                    settings.angle + 90.0,
                    Role::Support,
                    &flow,
                    false,
                    &mut paths,
                )?;
            } else if settings.tree_wall_count >= 2 {
                paths::walls(&base, &flow, true, true, &mut paths)?;
            } else {
                let (wide, slight): (Vec<ExPolygon>, Vec<ExPolygon>) =
                    base.iter().cloned().partition(|island| {
                        settings.tree_wall_count == 0 && island.area_mm2() > DOUBLE_WALL_AREA
                    });
                paths::walls(&wide, &flow, true, true, &mut paths)?;
                paths::walls(&slight, &flow, false, true, &mut paths)?;
            }
            if let Some(bed) = &job.bed {
                contacts = intersection(&contacts, std::slice::from_ref(bed))?;
                interfaces = intersection(&interfaces, std::slice::from_ref(bed))?;
            }
            let _ = config;
            fill(
                &bottom_interfaces,
                flow.spacing + settings.bottom_interface_spacing,
                settings.angle + 90.0,
                Role::SupportInterface,
                &flow,
                true,
                &mut paths,
            )?;
            let dense = geometry::union2(&contacts, &interfaces)?;
            fill(
                &dense,
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
