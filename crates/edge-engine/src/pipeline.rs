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
    /// Each layer's own height and top: the two above, but for the last
    /// layers where `precise_z_height` tuned them.
    heights: crate::layer_heights::LayerHeights,
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

/// A layer's outlines moved out by `contour` and its holes made larger
/// by `hole`, in scaled units, either of which may be negative
/// (PrintObjectSlice.cpp `_shrink_contour_holes`). A hole is grown by
/// itself, as a solid piece, and taken out of the outline grown.
fn shrink_contour_holes(
    contour: i64,
    hole: i64,
    regions: &[ExPolygon],
) -> Result<Vec<ExPolygon>, edge_clip::ClipError> {
    use edge_clip::Join;
    let mut made = Vec::new();
    for region in regions {
        let mut holes = Vec::new();
        for ring in &region.holes {
            let mut solid = ring.clone();
            solid.points.reverse();
            let solid = ExPolygon {
                contour: solid,
                holes: Vec::new(),
            };
            if hole != 0 {
                holes.extend(edge_clip::offset(&[solid], hole, Join::MITER)?);
            } else {
                holes.push(solid);
            }
        }
        let outline = ExPolygon {
            contour: region.contour.clone(),
            holes: Vec::new(),
        };
        let contours = if contour != 0 {
            let grown = edge_clip::offset(&[outline], contour, Join::MITER)?;
            if grown.is_empty() {
                continue;
            }
            grown
        } else {
            vec![outline]
        };
        made.extend(edge_clip::difference(
            &edge_clip::union_ex(&contours)?,
            &edge_clip::union_ex(&holes)?,
        )?);
    }
    edge_clip::union_ex(&made)
}

/// Applies the XY size compensation to one layer: its regions, and each
/// filament's share of them.
fn compensate_size(
    layer: &mut edge_slice::LayerRegions,
    contour: i64,
    hole: i64,
) -> Result<(), edge_clip::ClipError> {
    use edge_clip::Join;
    let (grow, shrink) = (contour.max(hole), contour.min(hole));
    if layer.by_filament.len() <= 1 {
        if grow > 0 {
            layer.regions = shrink_contour_holes(contour.max(0), hole.max(0), &layer.regions)?;
        }
        if shrink < 0 {
            layer.regions = shrink_contour_holes(contour.min(0), hole.min(0), &layer.regions)?;
        }
        if let Some((_, own)) = layer.by_filament.first_mut() {
            own.clone_from(&layer.regions);
        }
        return Ok(());
    }
    if grow > 0 {
        layer.regions = shrink_contour_holes(contour.max(0), hole.max(0), &layer.regions)?;
        let mut taken: Vec<ExPolygon> = Vec::new();
        let last = layer.by_filament.len() - 1;
        for (at, (_, own)) in layer.by_filament.iter_mut().enumerate() {
            let mut reached = edge_clip::intersection(
                &edge_clip::offset(own, grow, Join::MITER)?,
                &layer.regions,
            )?;
            if at > 0 {
                reached = edge_clip::difference(&reached, &taken)?;
            }
            if at < last {
                taken.extend(reached.iter().cloned());
            }
            *own = reached;
        }
    }
    if shrink < 0 {
        layer.regions = shrink_contour_holes(contour.min(0), hole.min(0), &layer.regions)?;
        for (_, own) in &mut layer.by_filament {
            *own = edge_clip::intersection(own, &layer.regions)?;
        }
    }
    Ok(())
}

/// What each of `regions` has inside `within`, region by region, so that
/// they stay in their order.
fn kept_inside(
    regions: &[ExPolygon],
    within: &[ExPolygon],
) -> Result<Vec<ExPolygon>, edge_clip::ClipError> {
    let bounds = |region: &ExPolygon| {
        let points = &region.contour.points;
        let low = |axis: fn(&Point2) -> i64| points.iter().map(axis).min().unwrap_or(0);
        let high = |axis: fn(&Point2) -> i64| points.iter().map(axis).max().unwrap_or(0);
        (low(|p| p.x), low(|p| p.y), high(|p| p.x), high(|p| p.y))
    };
    let boxes: Vec<_> = within.iter().map(bounds).collect();
    let mut kept = Vec::with_capacity(regions.len());
    for region in regions {
        let (left, bottom, right, top) = bounds(region);
        // Only what could reach the region is handed to the clipper.
        let near: Vec<ExPolygon> = within
            .iter()
            .zip(&boxes)
            .filter(|(_, b)| b.0 <= right && b.2 >= left && b.1 <= top && b.3 >= bottom)
            .map(|(piece, _)| piece.clone())
            .collect();
        kept.extend(edge_clip::intersection(
            std::slice::from_ref(region),
            &near,
        )?);
    }
    Ok(kept)
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
    let mut planes = edge_slice::layer_planes(
        min[2].into(),
        max[2].into(),
        first_layer_height,
        layer_height,
    );
    // How thick each layer is. With `precise_z_height` the last five are
    // tuned, and cut where they then lie, so that the print ends at the
    // top of the part; `crate::layer_heights` has the rest.
    let filaments: Vec<usize> = bodies.iter().map(|body| body.filament).collect();
    let tops: Vec<f64> = objects
        .iter()
        .filter_map(Mesh::bounds)
        .map(|(_, high)| high[2].into())
        .collect();
    let (heights, mut quality_notes) = crate::layer_heights::settle(
        &settings,
        &mut planes,
        &crate::layer_heights::Stack {
            span: (min[2].into(), max[2].into()),
            first_layer_height,
            layer_height,
            filaments: &filaments,
            tops: &tops,
        },
    )?;
    let borrowed: Vec<edge_slice::Body<'_>> = bodies
        .iter()
        .map(|body| edge_slice::Body {
            solid: &body.solid,
            negative: body.negative.as_ref(),
            filament: body.filament,
        })
        .collect();
    // The hole options need a round hole cut round: see
    // `edge_slice::slice_regions_without_flat_crossings`. Without them
    // the layers are cut as they always were.
    let holes = crate::holes::HoleSettings::from_settings(&settings)?;
    let mut layers = if holes.any() {
        edge_slice::slice_regions_without_flat_crossings(
            &borrowed,
            &planes,
            scaled(closing_radius),
        )?
    } else {
        edge_slice::slice_regions(&borrowed, &planes, scaled(closing_radius))?
    };

    // Overhangs are made printable by growing the layers under them
    // (`make_overhang_printable`), before anything else is done to the
    // layers as cut; `crate::printable` has the rest.
    let mut printable = crate::printable::Printable::from_settings(&settings)?;
    quality_notes.extend(printable.apply(&mut layers, &heights, !painted.is_empty())?);

    // Colours painted onto a part move what they cover from the filament
    // the part is made of to the one painted. Each painted object is
    // shared out by itself, from its own outline.
    let mut paint_notes = Vec::new();
    if !painted.is_empty() {
        let outer = walls.params(layer_height, false)?.outer;
        let (paint, notes) =
            crate::paint::settings(&settings, &outer, layer_height, &heights.all())?;
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

    // Outlines and holes are made larger or smaller by what the profile
    // sets, to make up for a printer that prints them off size
    // (`xy_contour_compensation`, `xy_hole_compensation`;
    // PrintObjectSlice.cpp, the "make_slices in parallel" part of
    // `PrintObject::slice_volumes`). Growing is done first and shrinking
    // after. On a layer in several filaments the layer is grown as a
    // whole and each filament takes what its own share grown reaches of
    // it, the first to come keeping what two would both reach; shrinking
    // trims every share to the layer shrunk.
    //
    // As the reference has it, none of it is done where colour is
    // painted on: the two cannot be combined. Not as the reference: it
    // leaves out the painted object only, and here everything on the
    // plate is sliced together, so one painted object leaves the whole
    // plate as modelled. A note says so.
    let xy_contour = settings.float("xy_contour_compensation")?.unwrap_or(0.0);
    let xy_hole = settings.float("xy_hole_compensation")?.unwrap_or(0.0);
    let mut size_notes = Vec::new();
    if xy_contour != 0.0 || xy_hole != 0.0 {
        if painted.is_empty() {
            let (contour, hole) = (scaled(xy_contour), scaled(xy_hole));
            layers.par_iter_mut().try_for_each(|layer| {
                compensate_size(layer, contour, hole).map_err(|e| e.to_string())
            })?;
        } else {
            size_notes.push(
                "xy_contour_compensation, xy_hole_compensation: not used on a plate with colour painted on it"
                    .to_owned(),
            );
        }
    }

    // The first layers are printed slightly smaller than modelled, tapering
    // to nothing over `elefant_foot_compensation_layers`. The modelled
    // outlines are kept: neighbouring layers are still compared with them.
    //
    // As the reference does it (PrintObjectSlice.cpp, the "make_slices in
    // parallel" part of `PrintObject::slice_volumes`): the external wall of
    // the layer in hand sets how narrow a part may be left; a layer in one
    // material is shrunk as it is; a layer in several is closed
    // first by one and a half times the closing radius, so that the seams
    // between its materials are not taken for outline, that is shrunk, and
    // each material keeps what it has inside the result. What was shrunk,
    // closed or not, is what the layer counts as modelled.
    //
    // The reference decides between the two for each object by its count
    // of regions; here a layer on which more than one filament has a share
    // is treated as the second kind, whichever object the shares are in.
    // The reference also leaves a part on a raft as modelled; no raft is
    // printed here, so the first layer is on the bed whatever
    // `raft_layers` says and is shrunk.
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
        let height = heights.height_of(index);
        let outer = walls.params(height, index == 0)?.outer;
        let amount = elephant_foot * (1.0 - index as f64 / elephant_layers as f64);
        let compensate = |regions: &[ExPolygon]| {
            edge_slice::compensate_elephant_foot(
                regions,
                scaled(outer.width + outer.spacing),
                scaled(amount),
            )
        };
        // Regions stay in the order they were in: islands are printed in
        // an order worked out from it. The reference unites what was
        // shrunk again, which here would only shuffle regions that
        // cannot have come to overlap.
        let (whole, shrunk) = if layer.by_filament.len() > 1 {
            let whole = edge_clip::closing(
                &layer.regions,
                scaled(closing_radius * 1.5),
                edge_clip::Join::MITER,
            )?;
            let shrunk = kept_inside(&layer.regions, &compensate(&whole)?)?;
            (whole, shrunk)
        } else {
            let shrunk = compensate(&layer.regions)?;
            (std::mem::take(&mut layer.regions), shrunk)
        };
        layer.regions = shrunk;
        modelled.push(whole);
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
    let mut shares: Vec<Vec<Vec<ExPolygon>>> = if used.is_empty() {
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
    // Round outlines and holes are resized by what the filament prints
    // them off by, and round holes that run up through the part are made
    // polygons of few sides (`enable_circle_compensation`,
    // `hole_to_polyhole`): after the compensations above and in that
    // order, as in the reference. `crate::holes` has the rest.
    let circle_marks = crate::holes::apply(&holes, &mut layers, &mut shares, &used)?;
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
                    let height = heights.height_of(index);
                    let mut params = walls
                        .params_at(height, index, layers.len())
                        .map_err(|e| e.to_string())?;
                    if let Some(fuzzy) = &mut params.fuzzy {
                        // The middle of the layer, where it was sliced.
                        fuzzy.slice_z = heights.top_of(index) - height / 2.0;
                    }
                    // A top surface is told from the part as a whole, on
                    // the layer above and the one below.
                    let around = Some(edge_perimeter::Around {
                        upper: outlines.get(index + 1).copied().unwrap_or(&[]),
                        lower: index.checked_sub(1).map(|below| outlines[below]),
                    });
                    let regions = share(filament, index);
                    let mut made = edge_perimeter::classic_between(regions, &params, around)
                        .map_err(|e| e.to_string())?;
                    // The walls round a compensated circle are told apart.
                    circle_marks
                        .flag(filament, index, regions, &params, &mut made)
                        .map_err(|e| e.to_string())?;
                    Ok(made)
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
    let mut supports = edge_support::generate(
        &outlines,
        middle,
        first_layer_height,
        layer_height,
        &support,
    )?;
    heights.fit(&mut supports)?;
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
    // A shell is as thick as `top_shell_thickness` and
    // `bottom_shell_thickness` ask, measured in the layers as they lie.
    let thick = heights.all();
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
            &thick,
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
    // Which is worth bridging is `dont_filter_internal_bridges`' to say.
    let bridged_by = edge_infill::bridge_over_infill_by(
        &anchored_by,
        &beneath,
        &infill.over_infill(layer_height)?,
    )?;
    // And the sparse fill under a bridge's edge, where that edge runs
    // along the walls, is made solid to carry it.
    let edges = edge_infill::bridge_edges(&bridged_by, solid_spacing)?;
    let bridged_by = bridged_by
        .iter()
        .map(|surfaces| edge_infill::carry_bridge_edges(surfaces, &edges, solid_spacing))
        .collect::<Result<Vec<_>, _>>()?;
    // A second layer of bridge over those, where the profile asks
    // (`enable_extra_bridge_layer`); over a bridge over air it is made
    // when surfaces are told apart.
    let bridged_by = if infill.extra_bridge_layer.internal() {
        edge_infill::second_internal_bridges(&bridged_by, &infill.params(layer_height, false)?)?
    } else {
        bridged_by
    };
    classify_time += classifying.elapsed();
    for (surfaces, perimeters) in bridged_by.into_iter().zip(&walls_by) {
        let fills = surfaces
            .par_iter()
            .enumerate()
            .map(|(index, surfaces)| {
                let height = heights.height_of(index);
                let mut params = infill
                    .params(height, index == 0)
                    .map_err(|e| e.to_string())?;
                // A sparse pattern is laid through the middle of the
                // object it fills, as in the reference.
                params.origin = middle;
                params.z = heights.top_of(index);
                edge_infill::fill_within(
                    surfaces,
                    index,
                    &params,
                    &|surface| {
                        frame_of(&middles, surface.area.contour.points.first().copied()).unwrap_or(
                            edge_infill::Frame {
                                origin: middle,
                                bounds: None,
                            },
                        )
                    },
                    // What is inside this filament's walls on the layer,
                    // for the fill's own gap fill.
                    &perimeters[index].infill_no_overlap,
                )
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
    // Ironing: the surfaces `ironing_type` names are gone over again
    // once everything else in their island is down (Fill.cpp
    // `Layer::make_ironing`, which the reference runs after all the fill
    // is made). Each filament irons its own share of a layer, as the
    // reference irons a region with that region's solid fill filament,
    // "to not contaminate the print with another material leaking from
    // the nozzle".
    let ironing = edge_infill::IroningSettings::from_settings(&settings)?;
    let mut ironing_notes = Vec::new();
    if ironing.enabled() {
        ironing_notes.clone_from(&ironing.unsupported);
        // The last layer of each object: the last whose middle, where it
        // is sliced, is still inside the object. A bare model is one
        // object, and ends with the print.
        let lasts: Vec<(Point2, Point2, usize)> = objects
            .iter()
            .filter_map(Mesh::bounds)
            .map(|(lo, hi)| {
                (
                    Point2::from_mm(lo[0].into(), lo[1].into()),
                    Point2::from_mm(hi[0].into(), hi[1].into()),
                    planes
                        .iter()
                        .rposition(|plane| *plane <= f64::from(hi[2]))
                        .unwrap_or(0),
                )
            })
            .collect();
        let opening = infill.shells(layer_height)?.opening;
        for (filament, fills) in fills_by.iter_mut().enumerate() {
            let own = ironing.for_filament(used[filament], layer_height);
            let slices: Vec<&[ExPolygon]> = (0..layers.len())
                .map(|index| share(filament, index))
                .collect();
            let faces = edge_infill::exposed(&slices, &outlines, opening)?;
            let (surfaces, perimeters) = (&surfaces_by[filament], &walls_by[filament]);
            let ironed = (0..layers.len())
                .into_par_iter()
                .map(|index| {
                    // The object standing at a point is the one with the
                    // smallest bounds that hold it, as for sparse fill.
                    let ends_here = |point: Point2| {
                        lasts
                            .iter()
                            .filter(|(low, high, _)| {
                                point.x >= low.x
                                    && point.x <= high.x
                                    && point.y >= low.y
                                    && point.y <= high.y
                            })
                            .min_by_key(|(low, high, _)| {
                                i128::from(high.x - low.x) * i128::from(high.y - low.y)
                            })
                            .map_or(layers.len() - 1, |&(_, _, last)| last)
                            == index
                    };
                    let areas = edge_infill::ironing_areas(
                        &edge_infill::IronedLayer {
                            slices: slices[index],
                            faces: &faces[index],
                            on_bed: index == 0,
                            surfaces: &surfaces[index],
                            infill: &perimeters[index].infill,
                            // As modelled: the reference keeps a layer
                            // shrunk against elephant foot its outline
                            // from before (PrintObjectSlice.cpp,
                            // "Store the uncompensated value there"),
                            // and ironing is kept back from that.
                            outline: outlines[index],
                            ends_here: &ends_here,
                        },
                        &own,
                    )
                    .map_err(|e| e.to_string())?;
                    edge_infill::iron(
                        &areas,
                        index,
                        &own,
                        &|area| {
                            frame_of(&middles, area.contour.points.first().copied())
                                .map_or(middle, |frame| frame.origin)
                        },
                        // Numbered on from the layer's surfaces.
                        surfaces[index].len() as u32 + 1,
                    )
                    .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<Vec<ExtrusionPath>>, String>>()?;
            for (layer, lines) in fills.iter_mut().zip(ironed) {
                layer.extend(lines);
            }
        }
    }
    let infill_time = started.elapsed();

    step(watch, Stage::Around)?;
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
    order_notes.extend(size_notes);
    order_notes.extend(quality_notes);
    order_notes.extend(ironing_notes);
    order_notes.extend(crate::unfollowed::notes(&settings, bodies.len()));
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
    let tower = edge_tower::plan(&rams, &heights.all(), &tower_settings)?;

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
        let alone = own.len() == 1 && bodies.iter().all(|body| body.negative.is_none());
        for mesh in own {
            let Some((low, high)) = mesh.bounds() else {
                continue;
            };
            let body = [edge_slice::Body {
                solid: mesh,
                negative: None,
                filament: 1,
            }];
            // The object's own first layer, grown as the plate's was
            // where overhangs were made printable down to the bed.
            let first =
                printable.first_layer(&body, alone, &planes, scaled(closing_radius), &heights)?;
            footprints.push((first, f64::from(high[2] - low[2])));
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
            let height = heights.height_of(index);
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
        heights,
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
        self.heights.height_of(index)
    }

    /// The top of layer `index` above the bed, in mm.
    pub fn top_of(&self, index: usize) -> f64 {
        self.heights.top_of(index)
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
                        all.infill_no_overlap
                            .extend(own.infill_no_overlap.iter().cloned());
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

#[cfg(test)]
mod tests {
    use edge_geom::Polygon;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    /// A 20 mm square with a 10 mm square hole in it.
    fn frame() -> ExPolygon {
        let mut hole = rect(5.0, 5.0, 15.0, 15.0);
        hole.points.reverse();
        ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![hole],
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    #[test]
    fn outlines_and_holes_are_resized_each_by_its_own_amount() {
        let sized = |contour: f64, hole: f64| {
            area(&shrink_contour_holes(scaled(contour), scaled(hole), &[frame()]).unwrap())
        };
        assert!((sized(0.0, 0.0) - 300.0).abs() < 1e-6);
        // The outline out by 0.1 mm a side; the hole larger by 0.2 mm a side.
        assert!((sized(0.1, 0.0) - (20.2 * 20.2 - 100.0)).abs() < 1e-3);
        assert!((sized(0.0, 0.2) - (400.0 - 10.4 * 10.4)).abs() < 1e-3);
        // And smaller, both.
        assert!((sized(-0.1, -0.2) - (19.8 * 19.8 - 9.6 * 9.6)).abs() < 1e-3);
        // An outline shrunk to nothing takes its region with it.
        let speck = ExPolygon {
            contour: rect(0.0, 0.0, 0.1, 0.1),
            holes: Vec::new(),
        };
        assert!(
            shrink_contour_holes(scaled(-0.1), 0, &[speck])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_layer_in_two_filaments_is_resized_as_a_whole() {
        // Two halves of a 20 mm square, one in each filament.
        let half = |x0: f64, x1: f64| ExPolygon {
            contour: rect(x0, 0.0, x1, 20.0),
            holes: Vec::new(),
        };
        let layer = || edge_slice::LayerRegions {
            z: 0.1,
            regions: vec![half(0.0, 20.0)],
            by_filament: vec![(1, vec![half(0.0, 10.0)]), (2, vec![half(10.0, 20.0)])],
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        // Grown: the whole is out by 0.1 mm, the shares still tile it,
        // and the seam between them has not moved in the first one's
        // favour by more than it reached.
        let mut grown = layer();
        compensate_size(&mut grown, scaled(0.1), 0).unwrap();
        assert!((area(&grown.regions) - 20.2 * 20.2).abs() < 1e-3);
        let shares: f64 = grown.by_filament.iter().map(|(_, own)| area(own)).sum();
        assert!((shares - area(&grown.regions)).abs() < 1e-3);
        assert!(area(&grown.by_filament[0].1) > area(&grown.by_filament[1].1));
        // Shrunk: every share is trimmed to the layer shrunk.
        let mut shrunk = layer();
        compensate_size(&mut shrunk, scaled(-0.1), 0).unwrap();
        assert!((area(&shrunk.regions) - 19.8 * 19.8).abs() < 1e-3);
        for (_, own) in &shrunk.by_filament {
            assert!((area(own) - 9.9 * 19.8).abs() < 1e-3);
        }
    }
}
