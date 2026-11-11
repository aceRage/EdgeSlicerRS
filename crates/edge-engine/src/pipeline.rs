use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

use edge_3mf::Body;
use edge_adhesion::{AdhesionSettings, BrimKind, Footing};
use edge_config::{FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionLoop, ExtrusionPath};
use edge_gcode::{PrintSettings, Summary};
use edge_geom::{ExPolygon, Point2, Polygon, scaled, unscaled};
use edge_infill::{InfillSettings, Surface};
use edge_mesh::Mesh;
use edge_perimeter::{Perimeters, WallSettings};
use edge_slice::LayerRegions;
use edge_support::SupportSettings;
use edge_tower::{Tower, TowerSettings};
use rayon::prelude::*;

use crate::{Failure, Job, Stage, Watcher, load::shift};

/// How long each stage took.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub slice: Duration,
    pub perimeters: Duration,
    pub supports: Duration,
    pub surfaces: Duration,
    pub fill: Duration,
}

/// One tool's turn on a layer.
pub(crate) struct Entry {
    pub(crate) layer: usize,
    /// Which of the filaments in use, counting from 0.
    pub(crate) filament: usize,
    /// What it lays on the prime tower.
    pub(crate) lines: Vec<ExtrusionPath>,
    /// What the tool before it lays there first, when its filament is
    /// rammed before it is put away.
    pub(crate) ram: Option<edge_tower::Rammed>,
    /// Its way in through the tower's wall.
    pub(crate) enter: Vec<Point2>,
}

/// A model sliced: every layer's walls, fill, supports and what goes
/// round them, ready to be written as G-code or drawn.
pub struct Sliced {
    pub bodies: Vec<Body>,
    /// The least and greatest corners of the model as loaded, in mm.
    pub bounds: ([f32; 3], [f32; 3]),
    /// The settings sliced with, complete.
    pub settings: Settings,
    pub layer_height: f64,
    pub first_layer_height: f64,
    pub print: PrintSettings,
    pub walls: WallSettings,
    pub infill: InfillSettings,
    pub support: SupportSettings,
    pub adhesion: AdhesionSettings,
    pub tower_settings: TowerSettings,
    /// Where on the bed a bare model was put, when it was.
    pub centred: Option<(f64, f64)>,
    /// Elephant foot compensation in mm, and the layers it is spread over.
    pub elephant_foot: f64,
    pub elephant_layers: usize,
    pub layers: Vec<LayerRegions>,
    /// The first layers' outlines as modelled, before they were shrunk
    /// against elephant foot.
    modelled: Vec<Vec<ExPolygon>>,
    /// The filaments in use, by number from 1.
    pub used: Vec<usize>,
    /// Each filament's share of every layer; empty with one filament,
    /// whose share is the layer itself.
    shares: Vec<Vec<Vec<ExPolygon>>>,
    /// Walls, surfaces and fill, by filament in `used`, then by layer.
    pub walls_by: Vec<Vec<Perimeters>>,
    pub surfaces_by: Vec<Vec<Vec<Surface>>>,
    pub fills_by: Vec<Vec<Vec<ExtrusionPath>>>,
    /// Where on each layer a move to a fill line needs no retraction.
    pub shelter: Vec<Vec<ExPolygon>>,
    pub supports: Vec<Vec<ExtrusionPath>>,
    /// Skirt and brim, by layer; the skirt's loops come first.
    pub around: Vec<Vec<ExtrusionLoop>>,
    pub skirt_loops: usize,
    /// The filaments printing on each layer, in order, as indices into `used`.
    pub orders: Vec<Vec<usize>>,
    /// Tool changes on each layer.
    pub changes: Vec<usize>,
    pub tower: Tower,
    /// What the profile asks of the order of tools that is not followed.
    pub(crate) order_notes: Vec<String>,
    /// What the profile asks of painted colours that is not followed.
    pub(crate) paint_notes: Vec<String>,
    /// (layer, index into `used`, what that tool prints on the tower when
    /// its turn comes), in the order printed.
    pub(crate) entries: Vec<Entry>,
    /// Things settled before slicing that the person should know of.
    pub remarks: Vec<String>,
    pub timings: Timings,
}

/// The object a point belongs to, for laying its sparse pattern: of the
/// objects whose bounds hold the point, the one with the smallest bounds,
/// so an object stood inside another's outline is told from it.
fn frame_of(
    objects: &[(Point2, Point2, Point2)],
    point: Option<Point2>,
) -> Option<edge_infill::Frame> {
    let point = point?;
    objects
        .iter()
        .filter(|(low, high, _)| {
            point.x >= low.x && point.x <= high.x && point.y >= low.y && point.y <= high.y
        })
        .min_by_key(|(low, high, _)| i128::from(high.x - low.x) * i128::from(high.y - low.y))
        .map(|&(low, high, middle)| edge_infill::Frame {
            origin: middle,
            bounds: Some((low, high)),
        })
}

/// The G-code for a sliced model.
pub struct Gcode {
    pub text: Vec<u8>,
    pub summary: Summary,
}

fn step(watch: &dyn Watcher, stage: Stage) -> Result<(), Failure> {
    if watch.cancelled() {
        return Err(Box::new(crate::Cancelled));
    }
    watch.stage(stage);
    Ok(())
}

/// Slices a model: layers, walls, supports, fill, tool order, prime tower,
/// skirt and brim. `watch` is told as each stage starts and may stop the
/// work between stages.
pub fn slice(job: Job, watch: &dyn Watcher) -> Result<Sliced, Failure> {
    let Job {
        loaded,
        is_project,
        settings,
        layer_height: (layer_height, _),
        first_layer_height: (first_layer_height, _),
        closing_radius: (closing_radius, _),
        wall_loops,
        bed_type: _,
        remarks,
    } = job;
    let (mut bodies, objects, plate) = (loaded.bodies, loaded.objects, loaded.plate);
    let painted = loaded.painted;
    let first_layer_sequence = loaded.first_layer_sequence;
    let (min, max) = loaded.bounds;

    let print = PrintSettings::from_settings(&settings)?;
    // A bare model has no place on the bed of its own: it goes in the
    // middle, as the reference's arranger leaves a single part.
    let mut centred = None;
    if let (false, Some([min_x, min_y, max_x, max_y])) = (is_project, print.bed) {
        let x = (min_x + max_x) / 2.0 - f64::from(min[0] + max[0]) / 2.0;
        let y = (min_y + max_y) / 2.0 - f64::from(min[1] + max[1]) / 2.0;
        shift(&mut bodies, x, y);
        centred = Some(((min_x + max_x) / 2.0, (min_y + max_y) / 2.0));
    }
    let mut walls = WallSettings::from_settings(&settings)?;
    if let Some(count) = wall_loops {
        walls.wall_loops = count;
    }

    step(watch, Stage::Layers)?;
    let started = Instant::now();
    let planes = edge_slice::layer_planes(
        min[2].into(),
        max[2].into(),
        first_layer_height,
        layer_height,
    );
    let borrowed: Vec<edge_slice::Body<'_>> = bodies
        .iter()
        .map(|body| edge_slice::Body {
            solid: &body.solid,
            negative: body.negative.as_ref(),
            filament: body.filament,
        })
        .collect();
    let mut layers = edge_slice::slice_regions(&borrowed, &planes, scaled(closing_radius))?;

    // Colours painted onto a part move what they cover from the filament
    // the part is made of to the one painted. Each painted object is
    // shared out by itself, from its own outline.
    let mut paint_notes = Vec::new();
    if !painted.is_empty() {
        let outer = walls.params(layer_height, false)?.outer;
        let (paint, notes) = crate::paint::settings(&settings, &outer, layer_height)?;
        paint_notes = notes;
        let base = bodies.first().map_or(1, |body| body.filament);
        // Worked out side by side, then applied in the order given.
        let shares = painted
            .par_iter()
            .map(|object| {
                let own = edge_slice::slice_regions(
                    &[edge_slice::Body {
                        solid: &object.solid,
                        negative: object.negative.as_ref(),
                        filament: base,
                    }],
                    &planes,
                    scaled(closing_radius),
                )
                .map_err(|e| e.to_string())?;
                let outlines: Vec<&[ExPolygon]> =
                    own.iter().map(|layer| layer.regions.as_slice()).collect();
                let facets: Vec<edge_paint::Facet> = object
                    .facets
                    .iter()
                    .map(|facet| edge_paint::Facet {
                        corners: facet.corners,
                        filament: facet.filament,
                    })
                    .collect();
                edge_paint::segment(&outlines, &planes, &facets, &paint).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        layers
            .par_iter_mut()
            .enumerate()
            .try_for_each(|(index, layer)| {
                let painted: Vec<&[edge_paint::Share]> = shares
                    .iter()
                    .map(|object| object[index].as_slice())
                    .collect();
                crate::paint::apply(layer, &painted, base)
            })?;
    }

    // The first layers are printed slightly smaller than modelled, tapering
    // to nothing over `elefant_foot_compensation_layers`. The modelled
    // outlines are kept: neighbouring layers are still compared with them.
    let elephant_foot = settings.float("elefant_foot_compensation")?.unwrap_or(0.0);
    let elephant_layers = if elephant_foot > 0.0 {
        usize::try_from(
            settings
                .integer("elefant_foot_compensation_layers")?
                .unwrap_or(1),
        )
        .unwrap_or(0)
    } else {
        0
    }
    .min(layers.len());
    let mut modelled: Vec<Vec<ExPolygon>> = Vec::with_capacity(elephant_layers);
    for (index, layer) in layers.iter_mut().take(elephant_layers).enumerate() {
        let height = if index == 0 {
            first_layer_height
        } else {
            layer_height
        };
        let outer = walls.params(height, index == 0)?.outer;
        let amount = elephant_foot * (1.0 - index as f64 / elephant_layers as f64);
        let shrunk = edge_slice::compensate_elephant_foot(
            &layer.regions,
            scaled(outer.width + outer.spacing),
            scaled(amount),
        )?;
        modelled.push(std::mem::replace(&mut layer.regions, shrunk));
    }
    let slice_time = started.elapsed();

    // The filaments in use, and each one's share of every layer. With one
    // filament its share is the layer itself.
    let mut used: Vec<usize> = layers
        .iter()
        .flat_map(|layer| layer.by_filament.iter().map(|(filament, _)| *filament))
        .collect();
    used.sort_unstable();
    used.dedup();
    let shares: Vec<Vec<Vec<ExPolygon>>> = if used.is_empty() {
        used = vec![bodies.first().map_or(1, |body| body.filament)];
        Vec::new()
    } else {
        used.iter()
            .map(|filament| {
                layers
                    .par_iter()
                    .enumerate()
                    .map(|(index, layer)| {
                        let own = layer
                            .by_filament
                            .iter()
                            .find(|(of, _)| of == filament)
                            .map_or(&[][..], |(_, regions)| regions.as_slice());
                        // The first layers are shrunk as a whole; each
                        // share keeps what is left of it.
                        if index < elephant_layers {
                            edge_clip::intersection(own, &layer.regions).map_err(|e| e.to_string())
                        } else {
                            Ok(own.to_vec())
                        }
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .collect::<Result<_, String>>()?
    };
    let share = |filament: usize, layer: usize| -> &[ExPolygon] {
        match shares.get(filament) {
            Some(own) => &own[layer],
            None => &layers[layer].regions,
        }
    };

    // The part's outlines as modelled: what supports, brim and skirt stand
    // off, and what neighbouring layers are compared by, so a first layer
    // shrunk against elephant foot does not count as an overhang above it.
    let outlines: Vec<&[ExPolygon]> = (0..layers.len())
        .map(|index| {
            modelled
                .get(index)
                .map_or(layers[index].regions.as_slice(), Vec::as_slice)
        })
        .collect();

    step(watch, Stage::Walls)?;
    let started = Instant::now();
    let walls_by: Vec<Vec<Perimeters>> = (0..used.len())
        .map(|filament| {
            (0..layers.len())
                .into_par_iter()
                .map(|index| {
                    let height = if index == 0 {
                        first_layer_height
                    } else {
                        layer_height
                    };
                    let mut params = walls
                        .params_at(height, index, layers.len())
                        .map_err(|e| e.to_string())?;
                    if let Some(fuzzy) = &mut params.fuzzy {
                        // The middle of the layer, where it was sliced.
                        fuzzy.slice_z =
                            first_layer_height + index as f64 * layer_height - height / 2.0;
                    }
                    // A top surface is told from the part as a whole, on
                    // the layer above and the one below.
                    let around = Some(edge_perimeter::Around {
                        upper: outlines.get(index + 1).copied().unwrap_or(&[]),
                        lower: index.checked_sub(1).map(|below| outlines[below]),
                    });
                    edge_perimeter::classic_between(share(filament, index), &params, around)
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<Perimeters>, String>>()
        })
        .collect::<Result<_, String>>()?;
    let perimeter_time = started.elapsed();

    step(watch, Stage::Supports)?;
    let started = Instant::now();
    let support = SupportSettings::from_settings(&settings)?;
    // Laid out from the middle of what is being printed.
    let middle = bodies
        .iter()
        .filter_map(|body| body.solid.bounds())
        .reduce(|(lo, hi), (min, max)| {
            (
                std::array::from_fn(|i| lo[i].min(min[i])),
                std::array::from_fn(|i| hi[i].max(max[i])),
            )
        })
        .map_or(Point2::from_mm(0.0, 0.0), |(lo, hi)| {
            Point2::from_mm(
                f64::from(lo[0] + hi[0]) / 2.0,
                f64::from(lo[1] + hi[1]) / 2.0,
            )
        });
    // Each object's bounds across the bed and its middle, for laying its
    // sparse pattern. A bare model is one object, with the one middle.
    let middles: Vec<(Point2, Point2, Point2)> = objects
        .iter()
        .filter_map(Mesh::bounds)
        .map(|(lo, hi)| {
            (
                Point2::from_mm(lo[0].into(), lo[1].into()),
                Point2::from_mm(hi[0].into(), hi[1].into()),
                Point2::from_mm(
                    f64::from(lo[0] + hi[0]) / 2.0,
                    f64::from(lo[1] + hi[1]) / 2.0,
                ),
            )
        })
        .collect();
    let supports = edge_support::generate(
        &outlines,
        middle,
        first_layer_height,
        layer_height,
        &support,
    )?;
    let support_time = started.elapsed();

    step(watch, Stage::Fill)?;
    let started = Instant::now();
    let infill = InfillSettings::from_settings(&settings)?;
    let mut classify_time = Duration::ZERO;
    let mut surfaces_by = Vec::with_capacity(used.len());
    let mut fills_by: Vec<Vec<Vec<ExtrusionPath>>> = Vec::with_capacity(used.len());
    let classifying = Instant::now();
    // Each layer's area inside the walls, over every filament. Where
    // filaments meet inside a part, the walls between them are not the
    // part's shell.
    let inside_by: Vec<Vec<&[ExPolygon]>> = walls_by
        .iter()
        .map(|perimeters| {
            perimeters
                .iter()
                .map(|layer| layer.infill.as_slice())
                .collect()
        })
        .collect();
    let printed: Vec<&[ExPolygon]> = layers
        .iter()
        .map(|layer| layer.regions.as_slice())
        .collect();
    let around = if used.len() > 1 {
        infill.walls_around(layer_height)?
    } else {
        None
    };
    let inside_all = edge_infill::inside_walls(&inside_by, &printed, around)?;
    let mut anchored_by = Vec::with_capacity(used.len());
    let mut shelter: Vec<Vec<ExPolygon>> = vec![Vec::new(); layers.len()];
    for (filament, inside_walls) in inside_by.iter().enumerate() {
        let slices: Vec<&[ExPolygon]> = (0..layers.len())
            .map(|index| share(filament, index))
            .collect();
        // Each filament's pieces are kept apart, as the reference keeps
        // its regions': a move from one into another is not sheltered.
        for (all, own) in shelter.iter_mut().zip(edge_infill::internal(
            &slices,
            &outlines,
            &infill.shells(layer_height)?,
        )?) {
            all.extend(own);
        }
        // Whatever the filament, a surface is a top or a bottom by what
        // the whole part has above and below it.
        let surfaces = edge_infill::classify(
            &slices,
            &outlines,
            inside_walls,
            &inside_all,
            &infill.shells(layer_height)?,
        )?;
        // Bridges, bottoms and tops reach into the fill beside them, and
        // each bridge is turned to run from support to support.
        anchored_by.push(edge_infill::anchor(
            &surfaces,
            &infill.anchoring(layer_height)?,
        )?);
    }
    // What each layer gives the next to rest on, whatever the filament.
    let beneath: Vec<edge_infill::Beneath> = (0..layers.len())
        .into_par_iter()
        .map(|index| {
            edge_infill::Beneath::gather(anchored_by.iter().zip(&walls_by).map(
                |(surfaces, perimeters)| {
                    (
                        surfaces[index].as_slice(),
                        perimeters[index].infill.as_slice(),
                    )
                },
            ))
        })
        .collect();
    classify_time += classifying.elapsed();
    let classifying = Instant::now();
    let solid_spacing = infill.params(layer_height, false)?.solid.scaled_spacing();
    // Solid over sparse fill is laid as a bridge across it.
    let bridged_by = anchored_by
        .iter()
        .map(|surfaces| edge_infill::bridge_over_infill(surfaces, &beneath, solid_spacing))
        .collect::<Result<Vec<_>, _>>()?;
    // And the sparse fill under a bridge's edge, where that edge runs
    // along the walls, is made solid to carry it.
    let edges = edge_infill::bridge_edges(&bridged_by, solid_spacing)?;
    let bridged_by = bridged_by
        .iter()
        .map(|surfaces| edge_infill::carry_bridge_edges(surfaces, &edges, solid_spacing))
        .collect::<Result<Vec<_>, _>>()?;
    classify_time += classifying.elapsed();
    for (surfaces, perimeters) in bridged_by.into_iter().zip(&walls_by) {
        let fills = surfaces
            .par_iter()
            .enumerate()
            .map(|(index, surfaces)| {
                let height = if index == 0 {
                    first_layer_height
                } else {
                    layer_height
                };
                let mut params = infill
                    .params(height, index == 0)
                    .map_err(|e| e.to_string())?;
                // A sparse pattern is laid through the middle of the
                // object it fills, as in the reference.
                params.origin = middle;
                params.z = first_layer_height + index as f64 * layer_height;
                edge_infill::fill_from(surfaces, index, &params, &|surface| {
                    frame_of(&middles, surface.area.contour.points.first().copied()).unwrap_or(
                        edge_infill::Frame {
                            origin: middle,
                            bounds: None,
                        },
                    )
                })
                .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<Vec<ExtrusionPath>>, String>>()?;
        // The lines that fill gaps between walls go with the fill, and
        // before it: the reference lays them with the walls.
        let mut fills = fills;
        for (layer, walls) in fills.iter_mut().zip(perimeters) {
            layer.splice(0..0, walls.gap_fill.iter().cloned());
            // So do walls over fill that hangs over air; the writer
            // takes those ahead of the loops.
            layer.splice(0..0, walls.over_air.iter().cloned());
        }
        surfaces_by.push(surfaces);
        fills_by.push(fills);
    }
    let infill_time = started.elapsed();

    step(watch, Stage::Around)?;
    let height_of = |index: usize| {
        if index == 0 {
            first_layer_height
        } else {
            layer_height
        }
    };
    // Which filaments print on each layer, and in what order
    // (ToolOrdering.cpp). The first layer takes them by the size of
    // the smallest piece each prints, smaller first; every later layer
    // starts with the tool already in hand and takes the rest from the
    // highest down.
    let on_layer = |filament: usize, index: usize| {
        !walls_by[filament][index].loops.is_empty() || !fills_by[filament][index].is_empty()
    };
    let tools_on = |index: usize| -> Vec<usize> {
        (0..used.len())
            .filter(|filament| on_layer(*filament, index))
            .collect()
    };
    let tower_settings = TowerSettings::from_settings(&settings, plate)?;
    // On the bed, each tool is judged by its smallest piece as cut,
    // pieces too slight to print a line of not counted.
    let mut on_bed = if layers.is_empty() {
        Vec::new()
    } else {
        tools_on(0)
    };
    if on_bed.len() > 1 {
        let nozzle = settings
            .floats("nozzle_diameter")?
            .and_then(|nozzles| nozzles.first().copied().flatten())
            .unwrap_or(0.4);
        let line = match settings.float_or_percent("initial_layer_line_width")? {
            Some(FloatOrPercent::Value(width)) => width,
            Some(FloatOrPercent::Percent(percent)) => percent / 100.0 * nozzle,
            None => 0.0,
        };
        let mut pieces = Vec::with_capacity(on_bed.len());
        for tool in &on_bed {
            let cut = layers[0]
                .cut_by_filament
                .iter()
                .find(|(filament, _)| *filament == used[*tool])
                .map_or(&[][..], |(_, regions)| regions.as_slice());
            let mut smallest = f64::INFINITY;
            for region in cut {
                let shrunk = edge_clip::offset(
                    std::slice::from_ref(region),
                    -scaled(0.2 * line),
                    edge_clip::Join::MITER,
                )?;
                if !shrunk.is_empty() {
                    smallest = smallest.min(region.contour.signed_area_mm2().abs());
                }
            }
            pieces.push((*tool, smallest));
        }
        on_bed =
            crate::order::first_layer(&pieces, &first_layer_sequence, |tool| used[tool] as u32);
    }
    let later: Vec<Vec<usize>> = (1..layers.len()).map(tools_on).collect();
    // A nozzle for each filament: every change costs the tower the same.
    let price = tower_settings.prime_volume as f32;
    let mut orders = crate::order::layers(on_bed, &later, |_, _| price);
    // A layer with nothing of the part on it still has its place, for
    // the tool in hand.
    let mut in_hand: Option<usize> = None;
    for order in &mut orders {
        if order.is_empty() {
            order.push(in_hand.unwrap_or(0));
        }
        in_hand = order.last().copied();
    }
    // What the profile asks of the order and is not followed.
    // What the walls were asked for and do not do goes with these.
    let mut order_notes = walls.notes(&settings);
    if used.len() > 1 {
        if let Some(kind) = settings.text("toolchange_ordering")
            && kind != "default"
        {
            order_notes.push(format!(
                "toolchange_ordering \"{kind}\": tools are taken in the order that wastes least"
            ));
        }
        if settings
            .floats("other_layers_print_sequence")?
            .is_some_and(|sequence| sequence.iter().flatten().any(|number| *number != 0.0))
        {
            order_notes.push(
                "other_layers_print_sequence: above the first layer tools are taken in the order that wastes least"
                    .to_owned(),
            );
        }
    }
    // Tool changes on each layer: every switch, counting one at the
    // start of a layer that begins with a tool not in hand.
    let mut changes = vec![0usize; layers.len()];
    // And for each, how the filament put away is rammed on the tower.
    let mut rams: Vec<Vec<Option<edge_tower::Ramming>>> = vec![Vec::new(); layers.len()];
    let mut held = orders.first().and_then(|order| order.first().copied());
    for (index, order) in orders.iter().enumerate() {
        for filament in order {
            if Some(*filament) != held {
                changes[index] += 1;
                rams[index].push(
                    held.and_then(|old| {
                        tower_settings.ramming.get(used[old] - 1).copied().flatten()
                    }),
                );
                held = Some(*filament);
            }
        }
    }
    let heights: Vec<f64> = (0..layers.len()).map(height_of).collect();
    let tower = edge_tower::plan(&rams, &heights, &tower_settings)?;

    // Skirt and brim, per layer, in the order they are printed.
    let adhesion = AdhesionSettings::from_settings(&settings)?;
    let beside = &outlines[..layers.len().min(adhesion.skirt_height.max(1))];
    let supports_beside: Vec<&[ExtrusionPath]> = supports
        .iter()
        .take(beside.len())
        .map(Vec::as_slice)
        .collect();
    // The brim is worked out object by object: each one's first layer and
    // how tall it stands. A bare model is the one object.
    let own: Vec<&Mesh> = if objects.is_empty() {
        bodies.iter().map(|body| &body.solid).collect()
    } else {
        objects.iter().collect()
    };
    let mut footprints: Vec<(Vec<ExPolygon>, f64)> = Vec::new();
    if adhesion.brim != BrimKind::None {
        for mesh in own {
            let Some((low, high)) = mesh.bounds() else {
                continue;
            };
            let body = [edge_slice::Body {
                solid: mesh,
                negative: None,
                filament: 1,
            }];
            let first = edge_slice::slice_regions(&body, &planes[..1], scaled(closing_radius))?;
            if let Some(layer) = first.into_iter().next() {
                footprints.push((layer.regions, f64::from(high[2] - low[2])));
            }
        }
    }
    let footings: Vec<Footing<'_>> = footprints
        .iter()
        .map(|(footprint, height)| Footing {
            footprint,
            height: *height,
        })
        .collect();
    // The prime tower's ground, brim included: no brim is laid on it,
    // and the skirt goes round it.
    let tower_ground: Vec<ExPolygon> = tower
        .footprint
        .map(|[min_x, min_y, max_x, max_y]| ExPolygon {
            contour: Polygon {
                points: [
                    (min_x, min_y),
                    (max_x, min_y),
                    (max_x, max_y),
                    (min_x, max_y),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        })
        .into_iter()
        .collect();
    let tower_corners: Vec<Point2> = tower_ground
        .iter()
        .flat_map(|ground| ground.contour.points.iter().copied())
        .collect();
    let brim = edge_adhesion::brim(&footings, &tower_ground, &adhesion, first_layer_height)?;
    let mut around: Vec<Vec<ExtrusionLoop>> = vec![Vec::new(); layers.len()];
    for (index, lines) in around.iter_mut().enumerate() {
        if adhesion.has_skirt(index) {
            let height = if index == 0 {
                first_layer_height
            } else {
                layer_height
            };
            *lines = edge_adhesion::skirt(
                beside,
                &brim,
                &supports_beside,
                &tower_corners,
                &adhesion,
                height,
            )?;
        }
    }
    let skirt_loops = around.first().map_or(0, Vec::len);
    if let Some(first) = around.first_mut() {
        first.extend(brim);
    }

    // What each tool prints on the tower when its turn comes: its own
    // block after a change, and once a layer the tower's wall and fill,
    // by the first tool to arrive, or by the tool in hand when none does.
    let mut entries: Vec<Entry> = Vec::new();
    let mut held = orders.first().and_then(|order| order.first().copied());
    for (index, order) in orders.iter().enumerate() {
        let mut blocks = tower.layers[index].changes.iter();
        let mut finish = Some(&tower.layers[index].finish);
        let any_change = changes[index] > 0;
        for filament in order {
            let mut lines = Vec::new();
            let (mut ram, mut enter) = (None, Vec::new());
            let switched = Some(*filament) != held;
            if switched {
                if let Some(block) = blocks.next() {
                    lines.extend(block.lines.iter().cloned());
                    ram = block.ram.clone();
                    enter = block.enter.clone();
                }
                held = Some(*filament);
            }
            if (switched || !any_change)
                && let Some(finish) = finish.take()
            {
                lines.extend(finish.iter().cloned());
            }
            entries.push(Entry {
                layer: index,
                filament: *filament,
                lines,
                ram,
                enter,
            });
        }
    }

    Ok(Sliced {
        bodies,
        bounds: (min, max),
        settings,
        layer_height,
        first_layer_height,
        print,
        walls,
        infill,
        support,
        adhesion,
        tower_settings,
        centred,
        elephant_foot,
        elephant_layers,
        layers,
        modelled,
        used,
        shares,
        walls_by,
        surfaces_by,
        fills_by,
        shelter,
        supports,
        around,
        skirt_loops,
        orders,
        changes,
        tower,
        order_notes,
        paint_notes,
        entries,
        remarks,
        timings: Timings {
            slice: slice_time,
            perimeters: perimeter_time,
            supports: support_time,
            surfaces: classify_time,
            fill: infill_time - classify_time,
        },
    })
}

impl Sliced {
    /// How thick layer `index` is, in mm.
    pub fn height_of(&self, index: usize) -> f64 {
        if index == 0 {
            self.first_layer_height
        } else {
            self.layer_height
        }
    }

    /// The top of layer `index` above the bed, in mm.
    pub fn top_of(&self, index: usize) -> f64 {
        self.first_layer_height + index as f64 * self.layer_height
    }

    /// The part's outline on each layer as modelled.
    pub fn outlines(&self) -> Vec<&[ExPolygon]> {
        (0..self.layers.len())
            .map(|index| {
                self.modelled
                    .get(index)
                    .map_or(self.layers[index].regions.as_slice(), Vec::as_slice)
            })
            .collect()
    }

    /// The part of layer `layer` printed in filament `filament`, an index
    /// into `used`.
    pub fn share(&self, filament: usize, layer: usize) -> &[ExPolygon] {
        match self.shares.get(filament) {
            Some(own) => &own[layer],
            None => &self.layers[layer].regions,
        }
    }

    /// Every filament's walls and fill on a layer taken together, by
    /// layer. One filament's are given as they are.
    pub fn combined(&self) -> (Cow<'_, [Perimeters]>, Cow<'_, [Vec<ExtrusionPath>]>) {
        if self.used.len() <= 1 {
            return (
                Cow::Borrowed(&self.walls_by[0]),
                Cow::Borrowed(&self.fills_by[0]),
            );
        }
        let (perimeters, fills): (Vec<Perimeters>, Vec<Vec<ExtrusionPath>>) =
            (0..self.layers.len())
                .map(|index| {
                    let mut all = Perimeters::default();
                    let mut fill = Vec::new();
                    for filament in 0..self.used.len() {
                        let own = &self.walls_by[filament][index];
                        let from = all.loops.len();
                        all.islands.extend(
                            own.islands
                                .iter()
                                .map(|island| island.start + from..island.end + from),
                        );
                        all.loops.extend(own.loops.iter().cloned());
                        all.infill.extend(own.infill.iter().cloned());
                        fill.extend(self.fills_by[filament][index].iter().cloned());
                    }
                    (all, fill)
                })
                .unzip();
        (Cow::Owned(perimeters), Cow::Owned(fills))
    }

    /// What the settings asked for that is not done, each with what is
    /// done instead.
    pub fn notes(&self) -> impl Iterator<Item = &String> {
        self.print
            .unsupported
            .iter()
            .chain(&self.adhesion.unsupported)
            .chain(&self.support.unsupported)
            .chain(&self.infill.unsupported)
            .chain(&self.order_notes)
            .chain(&self.paint_notes)
            // What the tower does not do matters only where there is one.
            .chain(
                self.tower_settings
                    .unsupported
                    .iter()
                    .filter(|_| self.tower.footprint.is_some()),
            )
    }

    /// The filaments in use that the printer has no tool for.
    pub fn without_a_tool(&self) -> impl Iterator<Item = usize> + '_ {
        self.used
            .iter()
            .copied()
            .filter(|filament| *filament > self.print.tools)
    }

    /// The print as G-code, with `produced_at` as the time in its first
    /// line. Nothing is returned for a print that would leave the bed.
    pub fn gcode(&self, produced_at: &str) -> Result<Gcode, Failure> {
        // One set of settings for each filament up to the highest in use.
        let highest = self.used.iter().copied().max().unwrap_or(1);
        // A filament the settings say nothing about cannot be printed:
        // there is no temperature to bring its nozzle to.
        if let Some(described) = self
            .settings
            .texts("nozzle_temperature")
            .map(<[String]>::len)
            && described < highest
        {
            return Err(format!(
                "the model prints in filament {highest}, but the settings describe only {described} \
                 filament(s); no G-code was written. Slice with a filament preset, which is then used \
                 for every filament, or give the project the filaments it is missing"
            )
            .into());
        }
        let mut prints: Vec<PrintSettings> = (0..highest)
            .map(|filament| PrintSettings::for_filament(&self.settings, filament))
            .collect::<Result<_, _>>()?;
        for print in &mut prints {
            print.produced_at = Some(produced_at.to_owned());
        }
        let outlines = self.outlines();
        // Layer tops, measured from the bottom of the model as the bed.
        // Skirt, brim and supports go to whichever tool starts the layer.
        let plan: Vec<edge_gcode::Layer<'_>> = self
            .entries
            .iter()
            .enumerate()
            .map(|(entry, turn)| {
                let (index, filament) = (turn.layer, &turn.filament);
                let starts = entry == 0 || self.entries[entry - 1].layer != index;
                let own = &self.walls_by[*filament][index];
                edge_gcode::Layer {
                    z: self.top_of(index),
                    height: self.height_of(index),
                    filament: self.used[*filament] - 1,
                    ram: turn.ram.as_ref().map(|ram| edge_gcode::Ram {
                        lines: &ram.lines,
                        speed: ram.speed,
                        park: ram.park,
                    }),
                    enter: &turn.enter,
                    tower: &turn.lines,
                    around: if starts { &self.around[index] } else { &[] },
                    support: if starts { &self.supports[index] } else { &[] },
                    loops: &own.loops,
                    islands: &own.islands,
                    paths: &self.fills_by[*filament][index],
                    shelter: &self.shelter[index],
                    below: if index > 0 { outlines[index - 1] } else { &[] },
                }
            })
            .collect();
        // Nothing may be sent to a printer that would run it off its bed.
        if let Some([min_x, min_y, max_x, max_y]) = self.print.bed {
            let points = plan.iter().flat_map(|layer| {
                let loops = layer.around.iter().chain(layer.loops);
                let ram = layer.ram.iter().flat_map(|ram| {
                    ram.lines
                        .iter()
                        .flat_map(|line| &line.points)
                        .chain([&ram.park])
                });
                loops
                    .flat_map(|line| &line.polygon.points)
                    .chain(
                        layer
                            .tower
                            .iter()
                            .chain(layer.support)
                            .chain(layer.paths)
                            .flat_map(|line| &line.points),
                    )
                    .chain(ram)
                    .chain(layer.enter)
            });
            let mut reach: Option<[f64; 4]> = None;
            for point in points {
                let (x, y) = (unscaled(point.x), unscaled(point.y));
                let [a, b, c, d] = reach.unwrap_or([x, y, x, y]);
                reach = Some([a.min(x), b.min(y), c.max(x), d.max(y)]);
            }
            if let Some([a, b, c, d]) = reach
                && (a < min_x || b < min_y || c > max_x || d > max_y)
            {
                return Err(format!(
                    "the print reaches from {a:.1} x {b:.1} to {c:.1} x {d:.1} mm, off the bed, which runs \
                     from {min_x} x {min_y} to {max_x} x {max_y} mm; no G-code was written"
                )
                .into());
            }
        }
        // Whole or not at all: a file cut short by an error must never be
        // left where it could be sent to a printer.
        let mut text = Vec::new();
        let summary = edge_gcode::write_tools(&mut text, &plan, &prints)?;
        Ok(Gcode { text, summary })
    }
}
