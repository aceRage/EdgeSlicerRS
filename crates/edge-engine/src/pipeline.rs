use std::{
    borrow::Cow,
    sync::Arc,
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
    /// Every stage before the walls: cutting, paint and the compensations.
    pub slice: Duration,
    /// Of `slice`, cutting the meshes into layers and making overhangs
    /// printable; and sharing the layers out by painted colour.
    pub cut: Duration,
    pub paint: Duration,
    pub perimeters: Duration,
    pub supports: Duration,
    pub surfaces: Duration,
    pub fill: Duration,
    /// Tool order, prime tower, skirt and brim.
    pub around: Duration,
    /// Waiting at the end for the seams' measure, made beside the rest.
    pub seams_wait: Duration,
}

/// The objects on the plate as seams are placed by them: each one's
/// bounds across the bed and their middle; its surface, with where in it
/// the triangles of what is cut out of it start (a bare model's surface
/// is its one body); its printed parts by themselves, for where they
/// meet; and the height the print stands on.
struct Seams {
    frames: Vec<(Point2, Point2, Point2)>,
    meshes: Vec<(Mesh, usize)>,
    parts: Vec<edge_3mf::InstanceParts>,
    bare: Option<Mesh>,
    floor: f64,
}

impl Seams {
    /// Hands `then` the scene the seams are placed in, with each
    /// physical layer's outline and what was measured of the objects.
    fn scene<R>(
        &self,
        outlines: &[&[ExPolygon]],
        part_joints: bool,
        measured: Option<&edge_gcode::SeamMeasures>,
        then: impl FnOnce(&edge_gcode::SeamScene<'_>) -> R,
    ) -> R {
        let surface = |object: usize| -> Option<(&Mesh, usize)> {
            match self.meshes.get(object) {
                Some((mesh, cut_from)) => Some((mesh, *cut_from)),
                None => self.bare.as_ref().map(|mesh| (mesh, mesh.triangles.len())),
            }
        };
        // Each object's parts, for where they meet.
        let parts: Vec<Vec<edge_gcode::SeamPart<'_>>> = (0..self.frames.len())
            .map(|object| {
                (self.parts.get(object).into_iter())
                    .flat_map(|object| &object.parts)
                    .map(|mesh| edge_gcode::SeamPart {
                        vertices: &mesh.vertices,
                        triangles: &mesh.triangles,
                    })
                    .collect()
            })
            .collect();
        let objects: Vec<edge_gcode::SeamObject<'_>> = (self.frames.iter().enumerate())
            .map(|(object, &(min, max, centre))| edge_gcode::SeamObject {
                min,
                max,
                centre,
                mesh: surface(object).map(|(mesh, negative_from)| edge_gcode::SeamMesh {
                    vertices: &mesh.vertices,
                    triangles: &mesh.triangles,
                    negative_from,
                }),
                parts: &parts[object],
                alone: self.parts.get(object).is_some_and(|object| object.alone),
                enforcers: (self.parts.get(object)).map_or(&[], |object| &object.enforcers),
                blockers: (self.parts.get(object)).map_or(&[], |object| &object.blockers),
            })
            .collect();
        then(&edge_gcode::SeamScene {
            outlines,
            objects: &objects,
            floor: self.floor,
            part_joints,
            measured,
        })
    }
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
    /// The objects on the plate, as seams are placed by them: each one's
    /// bounds across the bed and their middle; its surface, with where
    /// in it the triangles of what is cut out of it start (none for a
    /// bare model, whose surface is its one body's); and the height the
    /// print stands on.
    seams: Arc<Seams>,
    /// How visible each object's surface is and where its parts meet.
    measured: Option<edge_gcode::SeamMeasures>,
    /// The filaments in use, by number from 1.
    pub used: Vec<usize>,
    /// The filament each region prints in, by number from 1: what walls,
    /// surfaces and fill are kept by (`crate::regions`). Without regions
    /// of their own settings, `used` itself.
    pub(crate) slots: Vec<usize>,
    /// Each of those regions' key, and the regions with settings of
    /// their own.
    pub(crate) keys: Vec<usize>,
    pub(crate) regions: crate::regions::Regions,
    /// Each filament's share of every layer; empty with one filament,
    /// whose share is the layer itself.
    shares: Vec<Vec<Vec<ExPolygon>>>,
    /// Walls, surfaces and fill, by filament in `used`, then by layer.
    pub walls_by: Vec<Vec<Perimeters>>,
    pub surfaces_by: Vec<Vec<Vec<Surface>>>,
    pub fills_by: Vec<Vec<Vec<ExtrusionPath>>>,
    /// Where on each layer a move to a fill line needs no retraction.
    pub shelter: Vec<Vec<ExPolygon>>,
    /// What the lines contoured in height are laid by, by filament in
    /// `used` and layer as the walls and fill are; empty without Z
    /// contouring. And whether a contoured move is slowed for its height.
    contours: Vec<Vec<edge_contour::LayerProfiles>>,
    contour_speed: bool,
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
    /// The G-code the person set at layers, by the layer's top, and what
    /// of what was set there is not done (`crate::layer_codes`).
    pub(crate) codes: Vec<edge_gcode::AtLayer>,
    pub(crate) code_notes: Vec<String>,
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
    /// How long each part of writing it took.
    pub timings: edge_gcode::WriteTimings,
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
        placed,
        settings,
        layer_height: (layer_height, _),
        first_layer_height: (first_layer_height, _),
        closing_radius: (closing_radius, _),
        wall_loops,
        bed_type: _,
        remarks,
    } = job;
    let (mut bodies, objects, plate) = (loaded.bodies, loaded.objects, loaded.plate);
    let negatives = loaded.negatives;
    let seam_parts = loaded.parts;
    let painted = loaded.painted;
    let first_layer_sequence = loaded.first_layer_sequence;
    let custom_gcode = loaded.custom_gcode;
    let own = loaded.own;
    let (min, max) = loaded.bounds;

    let print = PrintSettings::from_settings(&settings)?;
    // A bare model has no place on the bed of its own: it goes in the
    // middle, as the reference's arranger leaves a single part. One that
    // was put somewhere by hand stays there.
    let mut centred = None;
    if let (false, Some([min_x, min_y, max_x, max_y])) = (is_project || placed, print.bed) {
        let x = (min_x + max_x) / 2.0 - f64::from(min[0] + max[0]) / 2.0;
        let y = (min_y + max_y) / 2.0 - f64::from(min[1] + max[1]) / 2.0;
        shift(&mut bodies, x, y);
        centred = Some(((min_x + max_x) / 2.0, (min_y + max_y) / 2.0));
    }
    // Laid out from the middle of what is being printed, where it stands
    // now (a bare model has been moved to the middle of the bed).
    let extent =
        bodies
            .iter()
            .filter_map(|body| body.solid.bounds())
            .reduce(|(lo, hi), (min, max)| {
                (
                    std::array::from_fn(|i| lo[i].min(min[i])),
                    std::array::from_fn(|i| hi[i].max(max[i])),
                )
            });
    let middle = extent.map_or(Point2::from_mm(0.0, 0.0), |(lo, hi)| {
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
    // The same of each object for its seams, which the reference
    // reckons from the middle of each object's bounds; a bare model is
    // one object.
    let seam_frames: Vec<(Point2, Point2, Point2)> = if middles.is_empty() {
        let (lo, hi) = extent.unwrap_or((min, max));
        vec![(
            Point2::from_mm(lo[0].into(), lo[1].into()),
            Point2::from_mm(hi[0].into(), hi[1].into()),
            middle,
        )]
    } else {
        middles.clone()
    };
    // Each object's whole surface, what is cut out of it last: what its
    // seams are hidden by; a bare model is its one body.
    let seam_meshes: Vec<(Mesh, usize)> = (objects.iter().enumerate())
        .map(|(object, solid)| {
            let mut solid = solid.clone();
            let cut_from = solid.triangles.len();
            if let Some(negative) = negatives.get(object) {
                solid.append(negative, &edge_mesh::Transform::IDENTITY);
            }
            (solid, cut_from)
        })
        .collect();
    let bare = (seam_meshes.is_empty() && bodies.len() == 1).then(|| bodies[0].solid.clone());
    let seams = Arc::new(Seams {
        frames: seam_frames,
        meshes: seam_meshes,
        parts: seam_parts,
        bare,
        floor: min[2].into(),
    });
    // How visible each object's surface is and where its parts meet go
    // by the meshes alone: they are measured beside the slicing and
    // waited for at the end. On one thread there is nothing beside it,
    // and they are measured with the seams, as the G-code is written.
    let (setup, part_joints) = (
        print.seam,
        settings.flag("seam_prefer_part_joints")?.unwrap_or(true),
    );
    let measuring = (rayon::current_num_threads() > 1).then(|| {
        let seams = Arc::clone(&seams);
        // On threads of its own, half as many as slicing has, so that
        // neither waits on the other's work queued ahead of its own.
        let threads = rayon::current_num_threads() / 2;
        std::thread::spawn(move || {
            let measure = || {
                seams.scene(&[], part_joints, None, |scene| {
                    edge_gcode::measure_seams(scene, setup)
                })
            };
            match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
                Ok(pool) => pool.install(measure),
                Err(_) => measure(),
            }
        })
    });
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
    // With Z contouring every layer but the first is sliced `zaa_min_z`
    // above its bottom (`crate::contour`).
    let zaa = edge_contour::ZaaSettings::read(&settings)?;
    if let Some(contouring) = &zaa.zaa {
        crate::contour::plane(contouring, &mut planes, &heights, min[2].into())?;
    }
    quality_notes.extend(zaa.notes);
    // Each body's region: its filament's, or one of its own where its
    // object or the part sets something (`crate::regions`).
    let mut regions = crate::regions::Regions::default();
    let mut own_set: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    let body_keys: Vec<usize> = bodies
        .iter()
        .map(|body| match body.part {
            Some((object, volume)) => {
                let (object, volume) = (&own[object], &own[object].volumes[volume]);
                let overrides =
                    crate::regions::part_overrides(&settings, object, volume, &mut own_set);
                regions.key(&settings, body.filament, &overrides)
            }
            None => body.filament,
        })
        .collect();
    let borrowed: Vec<edge_slice::Body<'_>> = bodies
        .iter()
        .zip(&body_keys)
        .map(|(body, key)| edge_slice::Body {
            solid: &body.solid,
            negative: body.negative.as_ref(),
            filament: *key,
        })
        .collect();
    let holes = crate::holes::HoleSettings::from_settings(&settings)?;
    let mut layers = edge_slice::slice_regions(&borrowed, &planes, scaled(closing_radius))?;
    // Height ranges and modifiers lay their settings over what of an
    // object's parts they take (`crate::regions`).
    crate::regions::lay_over_parts(
        &mut layers,
        &planes,
        scaled(closing_radius),
        &own,
        &bodies,
        &body_keys,
        &mut regions,
        &settings,
        &mut own_set,
    )?;

    // Overhangs are made printable by growing the layers under them
    // (`make_overhang_printable`), before anything else is done to the
    // layers as cut; `crate::printable` has the rest.
    let mut printable = crate::printable::Printable::from_settings(&settings)?;
    quality_notes.extend(printable.apply(&mut layers, &heights, !painted.is_empty())?);
    let cut_time = started.elapsed();
    let painting = Instant::now();

    // Colours painted onto a part move what they cover from the filament
    // the part is made of to the one painted. Each painted object is
    // shared out by itself, from its own outline.
    let mut paint_notes = Vec::new();
    if !painted.is_empty() {
        let outer = walls.params(layer_height, false)?.outer;
        let (paint, notes) =
            crate::paint::settings(&settings, &outer, layer_height, &heights.all())?;
        paint_notes = notes;
        let base = body_keys.first().copied().unwrap_or(1);
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

    let paint_time = painting.elapsed();

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

    // What the plate prints in, and the filament changes set at its
    // layers: from a change up every layer is printed in the filament it
    // names, as the reference overrides the extruder of every region of a
    // layer (ToolOrdering.cpp `collect_extruders`, `extruder_override`).
    let object_filaments: Vec<usize> = {
        let mut own: Vec<usize> = layers
            .iter()
            .flat_map(|layer| {
                layer
                    .by_filament
                    .iter()
                    .map(|(key, _)| regions.filament(*key))
            })
            .collect();
        if own.is_empty() {
            own = body_keys
                .iter()
                .map(|key| regions.filament(*key))
                .take(1)
                .collect();
        }
        own.sort_unstable();
        own.dedup();
        own
    };
    let (switches, mut code_notes) =
        crate::layer_codes::switches(custom_gcode.as_ref(), &settings, &object_filaments);
    if let ([own], false) = (object_filaments.as_slice(), switches.is_empty()) {
        let tops: Vec<f64> = (0..layers.len())
            .map(|index| heights.top_of(index))
            .collect();
        let by_layer = crate::layer_codes::filament_by_layer(&switches, *own, &tops);
        if by_layer.iter().any(|filament| filament != own) && !regions.any() {
            for (layer, filament) in layers.iter_mut().zip(by_layer) {
                layer.by_filament = vec![(filament, layer.regions.clone())];
                layer.cut_by_filament = vec![(filament, layer.regions.clone())];
            }
        } else if by_layer.iter().any(|filament| filament != own) {
            // Every region of the layer goes over to the filament named,
            // each keeping its own settings.
            let single = body_keys.first().copied().unwrap_or(*own);
            for (layer, filament) in layers.iter_mut().zip(by_layer) {
                for shares in [&mut layer.by_filament, &mut layer.cut_by_filament] {
                    if shares.is_empty() {
                        *shares = vec![(single, layer.regions.clone())];
                    }
                    for (key, _) in shares.iter_mut() {
                        *key = regions.in_filament(*key, &settings, filament);
                    }
                    shares.sort_by_key(|(key, _)| *key);
                }
            }
        }
    }

    // The filaments in use, and each one's share of every layer. With one
    // filament its share is the layer itself.
    //
    // Each of these is a region (`crate::regions`): `keys` holds each
    // one's key and `used` the filament it prints in, so a filament may
    // come more than once in `used` where it prints regions with settings
    // of their own. Without any, a region is a filament and its key the
    // filament's number.
    let mut keys: Vec<usize> = layers
        .iter()
        .flat_map(|layer| layer.by_filament.iter().map(|(key, _)| *key))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let mut shares: Vec<Vec<Vec<ExPolygon>>> = if keys.is_empty() {
        keys = vec![body_keys.first().copied().unwrap_or(1)];
        Vec::new()
    } else {
        keys.iter()
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
    let used: Vec<usize> = keys.iter().map(|key| regions.filament(*key)).collect();
    // The tools: each filament in use once, in order, and the tool each
    // region is printed with.
    let mut tools = used.clone();
    tools.sort_unstable();
    tools.dedup();
    let tool_of: Vec<usize> = (used.iter())
        .map(|filament| tools.binary_search(filament).unwrap_or(0))
        .collect();
    // The settings of the stages worked out region by region: the
    // plate's for a filament's own region.
    let walls_override = |mut own: crate::regions::StageSettings| {
        if let Some(count) = wall_loops {
            own.walls.wall_loops = count;
        }
        own
    };
    let plate_stages = crate::regions::StageSettings {
        walls,
        infill: InfillSettings::from_settings(&settings)?,
        ironing: edge_infill::IroningSettings::from_settings(&settings)?,
    };
    let stages: Vec<crate::regions::StageSettings> =
        keys.iter()
            .zip(&used)
            .map(|(key, filament)| match regions.own(*key) {
                Some(region) => crate::regions::StageSettings::read(&region.settings, *filament)
                    .map(walls_override),
                None => Ok(plate_stages.clone()),
            })
            .collect::<Result<_, _>>()?;
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
    // With each layer's walls, what of it is left to be bridged without
    // any: the roof of a hole that narrows from the layer below
    // (`counterbore_hole_bridging`), which `edge_perimeter`'s
    // `counterbore` has the rest of. Every filament's layers side by
    // side: a plate of few layers and many filaments has too few layers
    // alone to keep every thread busy.
    //
    // Regions whose walls are alike have them laid round all of them at
    // once (`crate::regions::wall_groups`): the first of them in each
    // group lays them, by the settings of the one with the densest
    // sparse fill on the layer, and each keeps what of the area inside
    // them is its own.
    let groups = crate::regions::wall_groups(&keys, &used, &regions, &settings);
    let members = |leader: usize| -> Vec<usize> {
        (0..used.len())
            .filter(|slot| groups[*slot] == leader)
            .collect()
    };
    let count = layers.len();
    let walled: Vec<(Perimeters, Vec<ExPolygon>)> = (0..used.len() * count)
        .into_par_iter()
        .map(|at| {
            let (filament, index) = (at / count, at % count);
            if groups[filament] != filament {
                return Ok((Perimeters::default(), Vec::new()));
            }
            let group = members(filament);
            let present: Vec<usize> = (group.iter().copied())
                .filter(|member| !share(*member, index).is_empty())
                .collect();
            let united;
            let (lays, regions) = match present.as_slice() {
                [] => (filament, share(filament, index)),
                [one] => (*one, share(*one, index)),
                several => {
                    let all: Vec<ExPolygon> = (several.iter())
                        .flat_map(|member| share(*member, index).iter().cloned())
                        .collect();
                    united = edge_clip::union_ex(&all).map_err(|e| e.to_string())?;
                    // Layer.cpp `make_perimeters`: "Use the region with
                    // highest infill rate".
                    let densest = several.iter().copied().fold(several[0], |best, member| {
                        if stages[member].infill.sparse_infill_density_percent
                            > stages[best].infill.sparse_infill_density_percent
                        {
                            member
                        } else {
                            best
                        }
                    });
                    (densest, united.as_slice())
                }
            };
            let height = heights.height_of(index);
            let walls = &stages[lays].walls;
            let mut params = walls
                .params_at(height, index, layers.len())
                .map_err(|e| e.to_string())?;
            if let Some(fuzzy) = &mut params.fuzzy {
                // The middle of the layer, where it was sliced;
                // with Z contouring, where that was instead.
                fuzzy.slice_z = match zaa.zaa {
                    Some(_) => planes[index] - f64::from(min[2]),
                    None => heights.top_of(index) - height / 2.0,
                };
            }
            // A top surface is told from the part as a whole, on
            // the layer above and the one below.
            let around = Some(edge_perimeter::Around {
                upper: outlines.get(index + 1).copied().unwrap_or(&[]),
                lower: index.checked_sub(1).map(|below| outlines[below]),
            });
            let bores = match (&walls.counterbore, index.checked_sub(1)) {
                (Some(bore), Some(below)) => Some(
                    edge_perimeter::split_counterbores(regions, outlines[below], &params, bore)
                        .map_err(|e| e.to_string())?,
                ),
                _ => None,
            };
            let (walled, bridged) = match bores {
                Some(split) => (Cow::Owned(split.walled), split.bridged),
                None => (Cow::Borrowed(regions), Vec::new()),
            };
            let mut made = edge_perimeter::classic_between(&walled, &params, around)
                .map_err(|e| e.to_string())?;
            // The walls round a compensated circle are told apart,
            // by the regions as they were cut.
            for member in &group {
                circle_marks
                    .flag(*member, index, share(*member, index), &params, &mut made)
                    .map_err(|e| e.to_string())?;
            }
            Ok((made, bridged))
        })
        .collect::<Result<_, String>>()?;
    let (mut walls_by, mut bored_by): (Vec<Vec<Perimeters>>, Vec<Vec<Vec<ExPolygon>>>) = (
        Vec::with_capacity(used.len()),
        Vec::with_capacity(used.len()),
    );
    let mut walled = walled.into_iter();
    for _ in 0..used.len() {
        let (walls, bored): (Vec<Perimeters>, Vec<Vec<ExPolygon>>) =
            walled.by_ref().take(count).unzip();
        walls_by.push(walls);
        bored_by.push(bored);
    }
    // Each region of a group keeps what of the area inside the walls is
    // its own (Layer.cpp `split_fill_surfaces_by_region`).
    for leader in (0..used.len()).filter(|slot| groups[*slot] == *slot) {
        let group = members(leader);
        if group.len() < 2 {
            continue;
        }
        let split: Vec<Vec<(Vec<ExPolygon>, Vec<ExPolygon>)>> = (0..count)
            .into_par_iter()
            .map(|index| {
                let laid = &walls_by[leader][index];
                (group.iter())
                    .map(|member| {
                        let own = share(*member, index);
                        Ok((
                            edge_clip::intersection(&laid.infill, own)?,
                            edge_clip::intersection(&laid.infill_no_overlap, own)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, edge_clip::ClipError>>()
            })
            .collect::<Result<_, _>>()?;
        for (index, parts) in split.into_iter().enumerate() {
            for (member, (infill, no_overlap)) in group.iter().zip(parts) {
                let own = &mut walls_by[*member][index];
                own.infill = infill;
                own.infill_no_overlap = no_overlap;
            }
        }
    }
    let perimeter_time = started.elapsed();

    step(watch, Stage::Supports)?;
    let started = Instant::now();
    let support = SupportSettings::from_settings(&settings)?;
    // Support enforcers and blockers, and supports painted on.
    let annotations = if support.enabled
        && own.iter().any(|object| {
            !object.support_paint.is_empty()
                || (object.volumes.iter()).any(|volume| {
                    matches!(
                        volume.kind,
                        edge_3mf::PartKind::SupportEnforcer | edge_3mf::PartKind::SupportBlocker
                    )
                })
        }) {
        crate::regions::support_annotations(&own, &bodies, &planes, scaled(closing_radius))?
    } else {
        edge_support::Annotations::default()
    };
    let mut supports = edge_support::generate_with(
        &outlines,
        middle,
        first_layer_height,
        layer_height,
        &support,
        &annotations,
    )?;
    heights.fit(&mut supports)?;
    let support_time = started.elapsed();

    step(watch, Stage::Fill)?;
    let started = Instant::now();
    let infill = plate_stages.infill.clone();
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
    // With regions of their own settings, as deep as the deepest walls of
    // those on the layer, and spaced as the closest.
    let inside_all = if regions.any() && used.len() > 1 {
        let own_around: Vec<Option<edge_infill::WallsAround>> = (stages.iter())
            .map(|own| own.infill.walls_around(layer_height))
            .collect::<Result<_, _>>()?;
        let deepest = |index: usize| {
            (0..used.len())
                .filter(|slot| !share(*slot, index).is_empty())
                .filter_map(|slot| own_around[slot])
                .reduce(|a, b| edge_infill::WallsAround {
                    depth: a.depth.max(b.depth),
                    spacing: a.spacing.min(b.spacing),
                })
        };
        edge_infill::inside_walls_by_layer(&inside_by, &printed, &deepest)?
    } else {
        edge_infill::inside_walls(&inside_by, &printed, around)?
    };
    let mut anchored_by = Vec::with_capacity(used.len());
    let mut shelter: Vec<Vec<ExPolygon>> = vec![Vec::new(); layers.len()];
    // A shell is as thick as `top_shell_thickness` and
    // `bottom_shell_thickness` ask, measured in the layers as they lie.
    let thick = heights.all();
    let shells_of: Vec<edge_infill::Shells> = (stages.iter())
        .map(|own| own.infill.shells(layer_height))
        .collect::<Result<_, _>>()?;
    let anchoring_of: Vec<edge_infill::Anchoring> = (stages.iter())
        .map(|own| own.infill.anchoring(layer_height))
        .collect::<Result<_, _>>()?;
    // Filament by filament side by side, each over its layers.
    type Classified = (Vec<Vec<ExPolygon>>, Vec<Vec<Surface>>);
    let classified: Vec<Classified> = inside_by
        .par_iter()
        .enumerate()
        .map(|(filament, inside_walls)| -> Result<Classified, String> {
            let (shells, anchoring) = (&shells_of[filament], &anchoring_of[filament]);
            let slices: Vec<&[ExPolygon]> = (0..layers.len())
                .map(|index| share(filament, index))
                .collect();
            let sheltered =
                edge_infill::internal(&slices, &outlines, shells).map_err(|e| e.to_string())?;
            // Whatever the filament, a surface is a top or a bottom by
            // what the whole part has above and below it.
            let surfaces = edge_infill::classify(
                &slices,
                &outlines,
                inside_walls,
                &inside_all,
                &thick,
                shells,
            )
            .map_err(|e| e.to_string())?;
            // Bridges, bottoms and tops reach into the fill beside them,
            // and each bridge is turned to run from support to support.
            let mut anchored =
                edge_infill::anchor(&surfaces, anchoring).map_err(|e| e.to_string())?;
            // What the walls left to be bridged over a hole that narrows
            // is a bridge as it stands, walls between it and the rest.
            if let Some(bore) = &stages[filament].walls.counterbore {
                for (index, bridged) in bored_by[filament].iter().enumerate().skip(1) {
                    if !bridged.is_empty() {
                        edge_infill::bridge_counterbores(
                            &mut anchored[index],
                            bridged,
                            outlines[index - 1],
                            shells.opening,
                            bore.fixed_angle,
                            bore.added_angle,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok((sheltered, anchored))
        })
        .collect::<Result<_, String>>()?;
    for (sheltered, anchored) in classified {
        // Each filament's pieces are kept apart, as the reference keeps
        // its regions': a move from one into another is not sheltered.
        for (all, own) in shelter.iter_mut().zip(sheltered) {
            all.extend(own);
        }
        anchored_by.push(anchored);
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
    // What laying a layer's fill needs. A sparse pattern is laid
    // through the middle of the object it fills, as in the reference.
    let params_of = |filament: usize, index: usize| -> Result<edge_infill::FillParams, Failure> {
        let mut params = stages[filament]
            .infill
            .params(heights.height_of(index), index == 0)?;
        params.origin = middle;
        params.z = heights.top_of(index);
        Ok(params)
    };
    let frame = |surface: &edge_infill::Surface| {
        frame_of(&middles, surface.area.contour.points.first().copied()).unwrap_or(
            edge_infill::Frame {
                origin: middle,
                bounds: None,
            },
        )
    };
    // Solid over sparse fill is laid as a bridge across it, run out to
    // the sparse lines beneath that carry it: those lines are laid for
    // this as they will be laid, from each layer's sparse surfaces as
    // they stand. Which is worth bridging is
    // `dont_filter_internal_bridges`' to say; and the sparse fill under
    // a bridge's edge, where that edge runs along the walls, is made
    // solid to carry it.
    let tops: Vec<f64> = (0..layers.len())
        .map(|index| heights.top_of(index))
        .collect();
    let sparse_lines = |index: usize| -> Result<Vec<Vec<Point2>>, Failure> {
        let mut lines = Vec::new();
        for (filament, surfaces) in anchored_by.iter().enumerate() {
            let sparse: Vec<edge_infill::Surface> = surfaces[index]
                .iter()
                .filter(|s| s.kind == edge_infill::SurfaceKind::Sparse)
                .cloned()
                .collect();
            if sparse.is_empty() {
                continue;
            }
            let params = params_of(filament, index)?;
            lines.extend(
                edge_infill::fill_within(&sparse, index, &params, &frame, &[])?
                    .into_iter()
                    .map(|path| path.points),
            );
        }
        Ok(lines)
    };
    let bridged_by = edge_infill::bridge_over_infill_by(
        &anchored_by,
        &beneath,
        &tops,
        &infill.over_infill(layer_height)?,
        &sparse_lines,
    )?;
    // A second layer of bridge over those, where the profile asks
    // (`enable_extra_bridge_layer`); over a bridge over air it is made
    // when surfaces are told apart.
    let bridged_by = if infill.extra_bridge_layer.internal() {
        edge_infill::second_internal_bridges(&bridged_by, &infill.params(layer_height, false)?)?
    } else {
        bridged_by
    };
    classify_time += classifying.elapsed();
    // Every filament's layers side by side, as the walls are.
    //
    // Regions in one filament whose fill is laid alike on a layer are
    // filled as one, as the reference fills together the surfaces of a
    // layer that share their fill's parameters (Fill.cpp `group_fills`):
    // the first of them lays the lines over the surfaces of all, each
    // kind of surface united across them.
    let count = layers.len();
    let filled: Vec<Vec<ExtrusionPath>> = (0..bridged_by.len() * count)
        .into_par_iter()
        .map(|at| {
            let (filament, index) = (at / count, at % count);
            let params = params_of(filament, index).map_err(|e| e.to_string())?;
            let mut alike = vec![filament];
            if regions.any() {
                let same = |other: usize| -> Result<bool, String> {
                    Ok(used[other] == used[filament]
                        && params_of(other, index).map_err(|e| e.to_string())? == params)
                };
                for (other, surfaces) in bridged_by.iter().enumerate() {
                    if other < filament && same(other)? && !surfaces[index].is_empty() {
                        // An earlier region lays this one's lines.
                        return Ok(Vec::new());
                    }
                    if other > filament && same(other)? {
                        alike.push(other);
                    }
                }
            }
            if alike.len() == 1 {
                return edge_infill::fill_within(
                    &bridged_by[filament][index],
                    index,
                    &params,
                    &frame,
                    // What is inside this filament's walls on the layer,
                    // for the fill's own gap fill.
                    &walls_by[filament][index].infill_no_overlap,
                )
                .map_err(|e| e.to_string());
            }
            let (merged, no_overlap) = crate::regions::merge_surfaces(
                alike.iter().map(|slot| bridged_by[*slot][index].as_slice()),
                alike
                    .iter()
                    .map(|slot| walls_by[*slot][index].infill_no_overlap.as_slice()),
            )
            .map_err(|e| e.to_string())?;
            edge_infill::fill_within(&merged, index, &params, &frame, &no_overlap)
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, String>>()?;
    let mut filled = filled.into_iter();
    for (surfaces, perimeters) in bridged_by.into_iter().zip(&walls_by) {
        let mut fills: Vec<Vec<ExtrusionPath>> = filled.by_ref().take(count).collect();
        // The lines that fill gaps between walls go with the fill, and
        // before it: the reference lays them with the walls.
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
    // A region irons by its own settings.
    let mut ironing_notes: Vec<String> = Vec::new();
    for own in stages.iter().filter(|own| own.ironing.enabled()) {
        for note in &own.ironing.unsupported {
            if !ironing_notes.contains(note) {
                ironing_notes.push(note.clone());
            }
        }
    }
    if stages.iter().any(|own| own.ironing.enabled()) {
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
        for (filament, fills) in fills_by.iter_mut().enumerate() {
            let ironing = &stages[filament].ironing;
            if !ironing.enabled() {
                continue;
            }
            let opening = shells_of[filament].opening;
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
    // Z contouring: every layer's walls, top surfaces and ironing laid
    // at the height of the model's surface under them.
    let contours = match &zaa.zaa {
        Some(contouring) => crate::contour::apply(
            contouring,
            &bodies,
            &planes,
            &heights,
            min[2].into(),
            &mut walls_by,
            &mut fills_by,
        ),
        None => Vec::new(),
    };
    let infill_time = started.elapsed();

    step(watch, Stage::Around)?;
    let started = Instant::now();
    // Which filaments print on each layer, and in what order
    // (ToolOrdering.cpp). The first layer takes them by the size of
    // the smallest piece each prints, smaller first; every later layer
    // starts with the tool already in hand and takes the rest from the
    // highest down.
    //
    // The order is worked out for tools, each filament in use once; each
    // tool then prints its regions one after another, in the order of
    // their keys.
    let on_layer = |filament: usize, index: usize| {
        !walls_by[filament][index].loops.is_empty() || !fills_by[filament][index].is_empty()
    };
    let tools_on = |index: usize| -> Vec<usize> {
        (0..tools.len())
            .filter(|tool| {
                (0..used.len())
                    .any(|filament| tool_of[filament] == *tool && on_layer(filament, index))
            })
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
                .filter(|(key, _)| regions.filament(*key) == tools[*tool])
                .flat_map(|(_, regions)| regions);
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
            crate::order::first_layer(&pieces, &first_layer_sequence, |tool| tools[tool] as u32);
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
    let plate_pattern = (plate_stages.infill.unsupported_pattern.as_ref())
        .map(|pattern| format!("sparse_infill_pattern \"{pattern}\": grid is used in its place"));
    order_notes.extend(size_notes);
    order_notes.extend(quality_notes);
    order_notes.extend(ironing_notes);
    order_notes.extend(crate::unfollowed::notes(&settings));
    order_notes.extend(crate::regions::unfollowed(&own_set));
    // What a region's own settings ask of its walls and fill that is not
    // done, said once.
    for (key, own) in keys.iter().zip(&stages) {
        let Some(region) = regions.own(*key) else {
            continue;
        };
        let pattern = (own.infill.unsupported_pattern.as_ref()).map(|pattern| {
            format!("sparse_infill_pattern \"{pattern}\": grid is used in its place")
        });
        let said = (own.walls.notes(&region.settings).into_iter())
            .chain(own.infill.unsupported.iter().cloned())
            .chain(pattern);
        for note in said {
            let plate_says = order_notes.contains(&note)
                || plate_stages.infill.unsupported.contains(&note)
                || Some(&note) == plate_pattern.as_ref();
            if !plate_says {
                order_notes.push(note);
            }
        }
    }
    if regions.any() && !painted.is_empty() {
        order_notes.push(
            "paint: colour painted on an object with settings of its own prints with the plate's settings"
                .to_owned(),
        );
    }
    // Everything on the plate is sliced as one, so travel that goes
    // round the walls takes a move from one object to another as it
    // takes one between two pieces of an object: along the inside of
    // each. The reference keeps objects apart and takes such a move
    // round their holes only (AvoidCrossingPerimeters.cpp
    // `get_boundary_external`).
    if objects.len() > 1 && settings.flag("reduce_crossing_wall")?.unwrap_or(false) {
        order_notes.push(
            "reduce_crossing_wall: a move from one object to another follows the inside of each \
             as far as it can, where the reference takes it round their holes only"
                .to_owned(),
        );
    }
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
                rams[index].push(held.and_then(|old| {
                    tower_settings
                        .ramming
                        .get(tools[old] - 1)
                        .copied()
                        .flatten()
                }));
                held = Some(*filament);
            }
        }
    }
    let tower = edge_tower::plan(&rams, &heights.all(), &tower_settings)?;
    // Each tool's turn on a layer, as its regions there, in order.
    let tool_orders = orders;
    let orders: Vec<Vec<usize>> = (tool_orders.iter().enumerate())
        .map(|(index, order)| {
            order
                .iter()
                .flat_map(|tool| {
                    let mine: Vec<usize> = (0..used.len())
                        .filter(|filament| tool_of[*filament] == *tool)
                        .collect();
                    let printing: Vec<usize> = (mine.iter().copied())
                        .filter(|filament| index < layers.len() && on_layer(*filament, index))
                        .collect();
                    if printing.is_empty() {
                        mine.into_iter().take(1).collect()
                    } else {
                        printing
                    }
                })
                .collect()
        })
        .collect();

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
    let footprints: Vec<(Vec<ExPolygon>, f64)> = if adhesion.brim == BrimKind::None {
        Vec::new()
    } else {
        let alone = own.len() == 1 && bodies.iter().all(|body| body.negative.is_none());
        // Object by object side by side, kept in their order.
        own.par_iter()
            .filter_map(|mesh| {
                let (low, high) = mesh.bounds()?;
                let body = [edge_slice::Body {
                    solid: mesh,
                    negative: None,
                    filament: 1,
                }];
                // The object's own first layer, grown as the plate's was
                // where overhangs were made printable down to the bed.
                Some(
                    printable
                        .first_layer(&body, alone, &planes, scaled(closing_radius), &heights)
                        .map(|first| (first, f64::from(high[2] - low[2]))),
                )
            })
            .collect::<Result<_, _>>()?
    };
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
    let mut held = (orders.first())
        .and_then(|order| order.first())
        .map(|filament| tool_of[*filament]);
    for (index, order) in orders.iter().enumerate() {
        let mut blocks = tower.layers[index].changes.iter();
        let mut finish = Some(&tower.layers[index].finish);
        let any_change = changes[index] > 0;
        for filament in order {
            let mut lines = Vec::new();
            let (mut ram, mut enter) = (None, Vec::new());
            let switched = Some(tool_of[*filament]) != held;
            if switched {
                if let Some(block) = blocks.next() {
                    lines.extend(block.lines.iter().cloned());
                    ram = block.ram.clone();
                    enter = block.enter.clone();
                }
                held = Some(tool_of[*filament]);
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

    // What is written at each printed layer, from the items set there.
    let mut printed: Vec<(f64, Vec<usize>)> = Vec::new();
    for entry in &entries {
        let top = heights.top_of(entry.layer);
        if printed.last().is_none_or(|(z, _)| *z != top) {
            printed.push((top, Vec::new()));
        }
        if let Some((_, filaments)) = printed.last_mut() {
            filaments.push(used[entry.filament] - 1);
        }
    }
    let (codes, assign_notes) = crate::layer_codes::assign(
        custom_gcode.as_ref(),
        &settings,
        &printed,
        &object_filaments,
    )?;
    code_notes.extend(assign_notes);

    let around_time = started.elapsed();
    let waiting = Instant::now();
    let measured = measuring.map(|measuring| {
        measuring
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    });
    let seams_wait = waiting.elapsed();
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
        seams,
        measured,
        used: tools,
        slots: used,
        keys,
        regions,
        shares,
        walls_by,
        surfaces_by,
        fills_by,
        shelter,
        contours,
        contour_speed: zaa.zaa.is_some_and(|contouring| contouring.speed_scaling),
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
            cut: cut_time,
            paint: paint_time,
            perimeters: perimeter_time,
            supports: support_time,
            surfaces: classify_time,
            fill: infill_time - classify_time,
            around: around_time,
            seams_wait,
        },
        codes,
        code_notes,
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
        if self.slots.len() <= 1 {
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
                    for filament in 0..self.slots.len() {
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
            .chain(&self.code_notes)
            // What the tower does not do matters only where there is one.
            .chain(
                self.tower_settings
                    .unsupported
                    .iter()
                    .filter(|_| self.tower.footprint.is_some()),
            )
    }

    /// Every note of what is not done, with the fill pattern that had to
    /// be replaced first: all there is to say of the settings this print
    /// was sliced with that it does not follow.
    pub fn gaps(&self) -> Vec<String> {
        let pattern = self.infill.unsupported_pattern.as_ref().map(|pattern| {
            format!("sparse_infill_pattern \"{pattern}\": grid is used in its place")
        });
        pattern.into_iter().chain(self.notes().cloned()).collect()
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
        // A region with settings of its own lays its walls and fill by
        // them; each such region's place among these, by region.
        let mut region_prints: Vec<PrintSettings> = Vec::new();
        let mut region_of: Vec<Option<usize>> = Vec::with_capacity(self.keys.len());
        for (key, filament) in self.keys.iter().zip(&self.slots) {
            region_of.push(match self.regions.own(*key) {
                Some(region) => {
                    let mut print = PrintSettings::for_filament(&region.settings, filament - 1)?;
                    print.produced_at = Some(produced_at.to_owned());
                    region_prints.push(print);
                    Some(region_prints.len() - 1)
                }
                None => None,
            });
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
                    filament: self.slots[*filament] - 1,
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
                    region: region_of[*filament],
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
        // What travel is routed by where it is to go round the inside of
        // the walls (`reduce_crossing_wall`): each layer's outline, all
        // filaments together, its top surfaces, and how its walls are
        // spaced (AvoidCrossingPerimeters.cpp `get_boundary`,
        // `get_perimeter_spacing`, `get_external_perimeter_width`).
        let routed = self.print.reduce_crossing_wall;
        let tops: Vec<Vec<ExPolygon>> = (0..self.layers.len())
            .filter(|_| routed)
            .map(|index| {
                (self.surfaces_by.iter())
                    .flat_map(|surfaces| &surfaces[index])
                    .filter(|surface| surface.kind == edge_infill::SurfaceKind::Top)
                    .map(|surface| surface.area.clone())
                    .collect()
            })
            .collect();
        let mut slices = Vec::new();
        for turn in self.entries.iter().filter(|_| routed) {
            let index = turn.layer;
            let walls = self.walls.params(self.height_of(index), index == 0)?;
            slices.push(edge_gcode::Slices {
                outline: &self.layers[index].regions,
                tops: &tops[index],
                spacing: walls.inner.spacing,
                outer_width: walls.outer.width,
            });
        }
        // Whole or not at all: a file cut short by an error must never be
        // left where it could be sent to a printer.
        let mut text = Vec::new();
        // What seams are placed by beside the walls: each layer's
        // outline, as it is under the layer above, and the objects.
        let mut printed: Vec<&[ExPolygon]> = Vec::new();
        for (entry, turn) in self.entries.iter().enumerate() {
            if entry == 0 || self.entries[entry - 1].layer != turn.layer {
                printed.push(outlines[turn.layer]);
            }
        }
        // Each entry's contoured lines, where there are any.
        let none = edge_contour::LayerProfiles::default();
        let profiles: Vec<&edge_contour::LayerProfiles> = (self.entries.iter())
            .filter(|_| !self.contours.is_empty())
            .map(|turn| {
                (self.contours.get(turn.filament))
                    .and_then(|layers| layers.get(turn.layer))
                    .unwrap_or(&none)
            })
            .collect();
        let contours = edge_gcode::Contours {
            layers: &profiles,
            speed_scaling: self.contour_speed,
        };
        let part_joints = (self.settings.flag("seam_prefer_part_joints")?).unwrap_or(true);
        let (summary, timings) =
            self.seams
                .scene(&printed, part_joints, self.measured.as_ref(), |scene| {
                    edge_gcode::write_timed(
                        &mut text,
                        &plan,
                        &slices,
                        scene,
                        &contours,
                        &self.codes,
                        &prints,
                        &region_prints,
                    )
                })?;
        Ok(Gcode {
            text,
            summary,
            timings,
        })
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
