//! Each object's own layers: its own `layer_height`, its height ranges'
//! layer heights, or its variable layer height profile
//! (`edge_slice::layering`, a port of Slicing.cpp), and the objects of a
//! plate put together in groups that are sliced at the same heights.
//!
//! The reference slices every object by itself at its own heights and
//! prints the layers of all of them in order of their tops, layers whose
//! tops are within `EPSILON` of one another together
//! (GCode.cpp `collect_layers_to_print`). Here the objects whose layers
//! are the same are sliced as one, as every object on a plate always was:
//! the objects that set nothing of their own heights are one group, cut
//! at the plate's heights as before, and each other set of objects whose
//! layers agree is a group of its own. `crate::pipeline` slices each
//! group and lays their layers together by their tops.
//!
//! What the reference refuses of objects of their own heights is refused
//! here with its words (Print.cpp `Print::validate`).

use edge_config::{Settings, Value};
use edge_slice::layering::{self, HeightRange, SlicingParams, Span};

use edge_geom::ExPolygon;
use edge_perimeter::Perimeters;

use crate::Failure;
use crate::pipeline::ObjectStage;
use crate::regions::{Scope, scope};

/// libslic3r.h `EPSILON`.
const EPSILON: f64 = 1e-4;

/// How the objects of a plate are sliced.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layering {
    /// For each object on the plate (by its place), the group it is in:
    /// 0 for the plate's own heights.
    pub(crate) group_of: Vec<usize>,
    /// Each group's layers from the second on (the first is the plate's
    /// own): bottom and top of each, measured from the bottom of the
    /// model.
    pub(crate) groups: Vec<Vec<Span>>,
    /// Each group's objects' layer height, from the second group on.
    pub(crate) layer_heights: Vec<f64>,
    /// Whether any object has layers of its own (a range or a profile,
    /// or its own `layer_height`).
    pub(crate) any_own: bool,
}

impl Layering {
    /// Whether every object is cut at the plate's heights.
    pub(crate) fn plain(&self) -> bool {
        self.groups.is_empty()
    }
}

/// What the plate gives an object's layers.
pub(crate) struct PlateLayers<'a> {
    pub(crate) settings: &'a Settings,
    /// The plate's layer height and first layer height, as the slice
    /// takes them (the command line's where it gives them).
    pub(crate) layer_height: f64,
    pub(crate) first_layer_height: f64,
    /// Each object's height, by its place.
    pub(crate) heights: &'a [f64],
}

/// The plate's settings with an object's own laid over them: what its
/// layers are settled by (the reference's `PrintObjectConfig`).
fn object_settings(plate: &Settings, own: &crate::OwnObject) -> Settings {
    let mut settings = plate.clone();
    for (key, value) in &own.settings {
        if scope(key) != Scope::Plate {
            settings.insert(key.clone(), Value::One(value.clone()));
        }
    }
    settings
}

/// One object's layering as the reference has it.
struct Object {
    params: SlicingParams,
    /// Its profile: the one its file has where it fits, else the one its
    /// height ranges give (PrintObject.cpp `update_layer_height_profile`).
    profile: Vec<f64>,
    precise_z: bool,
    /// Whether it has height ranges or a profile (Model.hpp
    /// `has_custom_layering`).
    custom: bool,
    raft_layers: i64,
    /// Supports that the organic tree generator lays.
    organic: bool,
}

fn read_object(
    plate: &PlateLayers<'_>,
    own: Option<&crate::OwnObject>,
    height: f64,
) -> Result<Object, Failure> {
    let settings = match own {
        Some(own) => object_settings(plate.settings, own),
        None => plate.settings.clone(),
    };
    let layer_height =
        match own.and_then(|own| own.settings.iter().find(|(k, _)| k == "layer_height")) {
            Some((_, value)) => value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("layer_height \"{value}\": not a number"))?,
            None => plate.layer_height,
        };
    let first = if plate.first_layer_height <= 0.0 {
        layer_height
    } else {
        plate.first_layer_height
    };
    let filaments: Vec<usize> = own
        .map(|own| {
            let mut used: Vec<usize> = (own.volumes.iter())
                .filter(|volume| volume.kind == crate::PartKind::Normal)
                .filter_map(|volume| volume.filament)
                .collect();
            used.sort_unstable();
            used.dedup();
            used
        })
        .unwrap_or_default();
    let (min, max) = crate::layer_heights::limits(&settings, layer_height, &filaments)?;
    let params = SlicingParams {
        layer_height,
        first_layer_height: first,
        min_layer_height: min,
        max_layer_height: max,
        height,
    };
    let mut ranges: Vec<HeightRange> = Vec::new();
    if let Some(own) = own {
        for range in &own.ranges {
            let range_height = match range.settings.iter().find(|(k, _)| k == "layer_height") {
                Some((_, value)) => value.trim().parse::<f64>().unwrap_or(layer_height),
                None => layer_height,
            };
            ranges.push(HeightRange {
                low: range.min_z - own.bottom,
                high: range.max_z - own.bottom,
                layer_height: range_height,
            });
        }
    }
    // A `std::map` keyed by the range's ends.
    ranges.sort_by(|a, b| {
        (a.low, a.high)
            .partial_cmp(&(b.low, b.high))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let file_profile = own.and_then(|own| own.layer_height_profile.as_ref());
    let profile = match file_profile {
        Some(profile) if layering::profile_fits(&params, profile) => profile.clone(),
        _ => layering::profile_from_ranges(&params, &ranges),
    };
    let custom =
        own.is_some_and(|own| !own.ranges.is_empty() || own.layer_height_profile.is_some());
    let supported = settings.flag("enable_support")? == Some(true)
        || settings.integer("raft_layers")?.unwrap_or(0) > 0
        || settings.integer("enforce_support_layers")?.unwrap_or(0) > 0;
    let tree = settings
        .text("support_type")
        .is_some_and(|kind| kind.starts_with("tree"));
    let organic = supported
        && tree
        && matches!(
            settings.text("support_style"),
            None | Some("default") | Some("tree_organic")
        );
    Ok(Object {
        params,
        profile,
        precise_z: settings.flag("precise_z_height")? == Some(true),
        custom,
        raft_layers: settings.integer("raft_layers")?.unwrap_or(0),
        organic,
    })
}

/// Whether two lists of layers agree as far as the shorter goes, each
/// height within half of `EPSILON` (the reference's own test of two
/// objects' layers, Print.cpp).
fn agree(a: &[Span], b: &[Span]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (x.0 - y.0).abs() <= 0.5 * EPSILON && (x.1 - y.1).abs() <= 0.5 * EPSILON)
}

/// Each object's layers and the groups they are sliced in, or what the
/// reference refuses of them.
pub(crate) fn settle(
    plate: &PlateLayers<'_>,
    own: &[crate::OwnObject],
) -> Result<Layering, Failure> {
    let count = plate.heights.len();
    let mut objects = Vec::with_capacity(count);
    for (place, height) in plate.heights.iter().enumerate() {
        let theirs = own.iter().find(|own| own.object == place);
        objects.push(read_object(plate, theirs, *height)?);
    }
    refuse(plate.settings, &objects)?;
    let mut layering = Layering {
        group_of: vec![0; count],
        ..Layering::default()
    };
    let plate_precise = plate.settings.flag("precise_z_height")? == Some(true);
    for (place, object) in objects.iter().enumerate() {
        let own = own.iter().any(|own| own.object == place);
        if !own {
            continue;
        }
        let mine = layering::object_layers(&object.params, &object.profile, false);
        let plain_params = SlicingParams {
            layer_height: plate.layer_height,
            ..object.params
        };
        let plain = layering::object_layers(
            &plain_params,
            &layering::profile_from_ranges(&plain_params, &[]),
            false,
        );
        let same = mine.len() == plain.len() && agree(&mine, &plain);
        if same && object.precise_z == plate_precise {
            continue;
        }
        layering.any_own = true;
        let layers = layering::object_layers(&object.params, &object.profile, object.precise_z);
        // With a group whose layers agree with its own as far as both go.
        let found = (layering.groups.iter_mut().enumerate())
            .find(|(at, group)| {
                agree(group, &layers) && layering.layer_heights[*at] == object.params.layer_height
            })
            .map(|(at, group)| {
                if layers.len() > group.len() {
                    *group = layers.clone();
                }
                at
            });
        let at = match found {
            Some(at) => at,
            None => {
                layering.groups.push(layers);
                layering.layer_heights.push(object.params.layer_height);
                layering.groups.len() - 1
            }
        };
        layering.group_of[place] = at + 1;
    }
    Ok(layering)
}

/// What the reference refuses before slicing of objects of their own
/// layers (Print.cpp `Print::validate`).
fn refuse(settings: &Settings, objects: &[Object]) -> Result<(), Failure> {
    let any_custom = objects.iter().any(|object| object.custom);
    // "Custom layering is not allowed for tree supports as of now."
    for object in objects {
        if object.organic
            && object.custom
            && !layering::layers_fixed(&object.params, &object.profile)
        {
            return Err("Variable layer height is not supported with Organic supports.".into());
        }
    }
    // Print.cpp `has_wipe_tower`.
    let tower = settings.flag("enable_prime_tower")? == Some(true)
        && (settings.text("timelapse_type") == Some("1")
            || (settings.flag("spiral_mode")? != Some(true)
                && settings
                    .texts("filament_diameter")
                    .map_or(0, <[String]>::len)
                    > 1));
    if !tower || objects.len() < 2 {
        return Ok(());
    }
    let first = &objects[0];
    let mut tallest = 0;
    for (index, object) in objects.iter().enumerate().skip(1) {
        if (object.params.first_layer_height - first.params.first_layer_height).abs() > EPSILON
            || (object.params.layer_height - first.params.layer_height).abs() > EPSILON
        {
            return Err(
                "The prime tower requires that all objects have the same layer heights.".into(),
            );
        }
        if object.raft_layers != first.raft_layers {
            return Err(
                "The prime tower requires that all objects are printed over the same number of raft layers."
                    .into(),
            );
        }
        // `equal_layering`: without a raft, the same layer heights.
        if object.params.layer_height != first.params.layer_height
            || object.params.first_layer_height != first.params.first_layer_height
        {
            return Err(
                "The prime tower requires that all objects are sliced with the same layer heights."
                    .into(),
            );
        }
        if any_custom {
            let end = |profile: &[f64]| profile[profile.len() - 2];
            if end(&object.profile) > end(&objects[tallest].profile) {
                tallest = index;
            }
        }
    }
    if any_custom {
        let eps = 0.5 * EPSILON;
        for (index, object) in objects.iter().enumerate() {
            if index == tallest {
                continue;
            }
            // Element by element, as far as the shorter goes.
            let differs = (object.profile.iter())
                .zip(&objects[tallest].profile)
                .any(|(a, b)| (a - b).abs() > eps);
            if differs {
                return Err(
                    "The prime tower is only supported if all objects have the same variable layer height."
                        .into(),
                );
            }
        }
    }
    Ok(())
}

/// The group a body is sliced in.
fn group_of_body(layering: &Layering, own: &[crate::OwnObject], body: &edge_3mf::Body) -> usize {
    body.part
        .and_then(|(at, _)| own.get(at))
        .and_then(|object| layering.group_of.get(object.object))
        .copied()
        .unwrap_or(0)
}

/// Slices each group of objects at its own layers (`stage`, given the
/// group, its bodies, its painted objects and its objects' meshes) and
/// lays the groups' layers together by their tops, layers whose tops are
/// within `EPSILON` of one another as one, at the middle of them
/// (GCode.cpp `collect_layers_to_print`).
pub(crate) fn slice_groups(
    layering: &Layering,
    bodies: &[edge_3mf::Body],
    own: &[crate::OwnObject],
    painted: &[edge_3mf::Painted],
    objects: &[edge_mesh::Mesh],
    mut stage: impl FnMut(
        usize,
        &[edge_3mf::Body],
        &[edge_3mf::Painted],
        &[edge_mesh::Mesh],
    ) -> Result<ObjectStage, Failure>,
) -> Result<ObjectStage, Failure> {
    // A painted object's group: the object whose bounds are its parts'.
    let bounds_of = |meshes: &mut dyn Iterator<Item = &edge_mesh::Mesh>| {
        meshes
            .filter_map(edge_mesh::Mesh::bounds)
            .reduce(|(lo, hi), (low, high)| {
                (
                    std::array::from_fn(|i| lo[i].min(low[i])),
                    std::array::from_fn(|i| hi[i].max(high[i])),
                )
            })
    };
    let mut painted_group = Vec::with_capacity(painted.len());
    for object in painted {
        let own = bounds_of(&mut object.parts.iter());
        let place = objects
            .iter()
            .position(|mesh| own.is_some() && mesh.bounds() == own)
            .ok_or("colour painted on an object that cannot be told among objects of their own layer heights: not done")?;
        painted_group.push(layering.group_of.get(place).copied().unwrap_or(0));
    }
    let mut stages: Vec<ObjectStage> = Vec::new();
    for group in 0..=layering.groups.len() {
        let mine: Vec<edge_3mf::Body> = (bodies.iter())
            .filter(|body| group_of_body(layering, own, body) == group)
            .cloned()
            .collect();
        if mine.is_empty() {
            continue;
        }
        let mine_painted: Vec<edge_3mf::Painted> = (painted.iter().zip(&painted_group))
            .filter(|(_, at)| **at == group)
            .map(|(object, _)| object.clone())
            .collect();
        let mine_objects: Vec<edge_mesh::Mesh> = (objects.iter().enumerate())
            .filter(|(place, _)| layering.group_of.get(*place).copied().unwrap_or(0) == group)
            .map(|(_, mesh)| mesh.clone())
            .collect();
        stages.push(stage(group, &mine, &mine_painted, &mine_objects)?);
    }
    Ok(lay_together(stages))
}

/// Unites the islands of several layers' walls, each group's after the
/// last.
fn add_walls(into: &mut Perimeters, from: &Perimeters) {
    let offset = into.loops.len();
    into.loops.extend(from.loops.iter().cloned());
    into.islands.extend(
        from.islands
            .iter()
            .map(|range| range.start + offset..range.end + offset),
    );
    into.infill.extend(from.infill.iter().cloned());
    into.infill_no_overlap
        .extend(from.infill_no_overlap.iter().cloned());
    into.gap_fill.extend(from.gap_fill.iter().cloned());
    into.gaps_skipped += from.gaps_skipped;
    into.over_air.extend(from.over_air.iter().cloned());
}

/// Each group's layers laid together by their tops.
fn lay_together(mut stages: Vec<ObjectStage>) -> ObjectStage {
    if stages.len() == 1 {
        return stages.pop().expect("one stage");
    }
    // Every layer of every group by its top, the lower first and, at one
    // top, in the order of the groups.
    let mut ordering: Vec<(f64, usize, usize)> = Vec::new();
    for (group, stage) in stages.iter().enumerate() {
        for index in 0..stage.layers.len() {
            ordering.push((stage.heights.top_of(index), group, index));
        }
    }
    ordering.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut plate: Vec<(f64, Vec<(usize, usize)>)> = Vec::new();
    let mut i = 0;
    while i < ordering.len() {
        let mut j = i + 1;
        let zmax = ordering[i].0 + EPSILON;
        while j < ordering.len() && ordering[j].0 <= zmax {
            j += 1;
        }
        let top = 0.5 * (ordering[i].0 + ordering[j - 1].0);
        let mut members: Vec<(usize, usize)> =
            ordering[i..j].iter().map(|&(_, g, x)| (g, x)).collect();
        members.sort_unstable();
        plate.push((top, members));
        i = j;
    }
    let count = plate.len();
    // The regions: every group's keys, each with its filament.
    let mut keys: Vec<usize> = stages
        .iter()
        .flat_map(|stage| stage.keys.iter().copied())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let slot_in = |stage: &ObjectStage, key: usize| stage.keys.iter().position(|own| *own == key);
    let used: Vec<usize> = keys
        .iter()
        .map(|key| {
            stages
                .iter()
                .find_map(|stage| slot_in(stage, *key).map(|slot| stage.used[slot]))
                .unwrap_or(1)
        })
        .collect();
    let settings_of: Vec<crate::regions::StageSettings> = keys
        .iter()
        .map(|key| {
            stages
                .iter()
                .find_map(|stage| slot_in(stage, *key).map(|slot| stage.stages[slot].clone()))
                .expect("a key of some group")
        })
        .collect();
    // A group's share of a layer in a region: its layer itself where it
    // has one region.
    let share = |stage: &ObjectStage, slot: usize, index: usize| -> Vec<ExPolygon> {
        match stage.shares.get(slot) {
            Some(own) => own[index].clone(),
            None => stage.layers[index].regions.clone(),
        }
    };
    let one_region = keys.len() == 1;
    let mut layers = Vec::with_capacity(count);
    let mut modelled = Vec::with_capacity(count);
    let mut below = Vec::with_capacity(count);
    let mut planes = Vec::with_capacity(count);
    let mut shelter = Vec::with_capacity(count);
    let mut supports = Vec::with_capacity(count);
    let mut shares: Vec<Vec<Vec<ExPolygon>>> = vec![Vec::with_capacity(count); keys.len()];
    let mut walls_by: Vec<Vec<Perimeters>> = vec![Vec::with_capacity(count); keys.len()];
    let mut surfaces_by: Vec<Vec<Vec<edge_infill::Surface>>> =
        vec![Vec::with_capacity(count); keys.len()];
    let mut fills_by: Vec<Vec<Vec<edge_extrusion::ExtrusionPath>>> =
        vec![Vec::with_capacity(count); keys.len()];
    let contoured = stages.iter().any(|stage| !stage.contours.is_empty());
    let mut contours: Vec<Vec<edge_contour::LayerProfiles>> = if contoured {
        vec![Vec::with_capacity(count); keys.len()]
    } else {
        Vec::new()
    };
    let outline = |stage: &ObjectStage, index: usize| -> Vec<ExPolygon> {
        stage
            .modelled
            .get(index)
            .unwrap_or(&stage.layers[index].regions)
            .clone()
    };
    for (_, members) in &plate {
        let mut layer = edge_slice::LayerRegions {
            z: stages[members[0].0].planes[members[0].1],
            regions: Vec::new(),
            by_filament: Vec::new(),
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        let (mut model, mut under, mut shelters, mut support) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for &(group, index) in members {
            let stage = &stages[group];
            let own = &stage.layers[index];
            layer.regions.extend(own.regions.iter().cloned());
            layer.open_chains += own.open_chains;
            let merge = |into: &mut Vec<(usize, Vec<ExPolygon>)>,
                         from: &[(usize, Vec<ExPolygon>)]| {
                let from: Vec<(usize, Vec<ExPolygon>)> =
                    if from.is_empty() && !own.regions.is_empty() {
                        vec![(stage.keys[0], own.regions.clone())]
                    } else {
                        from.to_vec()
                    };
                for (key, regions) in from {
                    match into.iter_mut().find(|(own, _)| *own == key) {
                        Some((_, into)) => into.extend(regions),
                        None => into.push((key, regions)),
                    }
                }
                into.sort_by_key(|(key, _)| *key);
            };
            merge(&mut layer.by_filament, &own.by_filament);
            merge(&mut layer.cut_by_filament, &own.cut_by_filament);
            model.extend(outline(stage, index));
            if index > 0 {
                under.extend(outline(stage, index - 1));
            }
            shelters.extend(stage.shelter[index].iter().cloned());
            support.extend(stage.supports[index].iter().cloned());
        }
        if one_region {
            layer.by_filament.clear();
            layer.cut_by_filament.clear();
        }
        for (slot, key) in keys.iter().enumerate() {
            let (mut walls, mut surfaces, mut fills, mut share_of) =
                (Perimeters::default(), Vec::new(), Vec::new(), Vec::new());
            let mut profiles = edge_contour::LayerProfiles::default();
            for &(group, index) in members {
                let stage = &stages[group];
                let Some(own) = slot_in(stage, *key) else {
                    continue;
                };
                share_of.extend(share(stage, own, index));
                let perimeters = &stage.walls_by[own][index];
                add_walls(&mut walls, perimeters);
                surfaces.extend(stage.surfaces_by[own][index].iter().cloned());
                let paths = &stage.fills_by[own][index];
                fills.extend(paths.iter().cloned());
                if contoured {
                    match stage.contours.get(own).and_then(|layers| layers.get(index)) {
                        Some(theirs) => {
                            profiles.loops.extend(theirs.loops.iter().cloned());
                            profiles.paths.extend(theirs.paths.iter().cloned());
                        }
                        None => {
                            profiles.loops.extend(perimeters.loops.iter().map(|_| None));
                            profiles.paths.extend(paths.iter().map(|_| None));
                        }
                    }
                }
            }
            shares[slot].push(share_of);
            walls_by[slot].push(walls);
            surfaces_by[slot].push(surfaces);
            fills_by[slot].push(fills);
            if contoured {
                contours[slot].push(profiles);
            }
        }
        planes.push(layer.z);
        layers.push(layer);
        modelled.push(model);
        below.push(under);
        shelter.push(shelters);
        supports.push(support);
    }
    if one_region && stages.iter().all(|stage| stage.shares.is_empty()) {
        shares.clear();
    }
    let tops: Vec<f64> = plate.iter().map(|(top, _)| *top).collect();
    let notes = |pick: fn(&ObjectStage) -> &Vec<String>| -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        for stage in &stages {
            for note in pick(stage) {
                if !all.contains(note) {
                    all.push(note.clone());
                }
            }
        }
        all
    };
    let quality_notes = notes(|stage| &stage.quality_notes);
    let size_notes = notes(|stage| &stage.size_notes);
    let ironing_notes = notes(|stage| &stage.ironing_notes);
    let paint_notes = notes(|stage| &stage.paint_notes);
    let code_notes = notes(|stage| &stage.code_notes);
    let mut object_filaments: Vec<usize> = (stages.iter())
        .flat_map(|stage| stage.object_filaments.iter().copied())
        .collect();
    object_filaments.sort_unstable();
    object_filaments.dedup();
    let mut times = [std::time::Duration::ZERO; 7];
    for stage in &stages {
        for (sum, own) in times.iter_mut().zip(stage.times) {
            *sum += own;
        }
    }
    let elephant_layers = stages
        .iter()
        .map(|stage| stage.elephant_layers)
        .max()
        .unwrap_or(0);
    let contour_speed = stages.iter().any(|stage| stage.contour_speed);
    let body_keys = stages
        .iter()
        .flat_map(|stage| stage.body_keys.iter().copied())
        .collect();
    let first = stages.swap_remove(0);
    ObjectStage {
        heights: crate::layer_heights::LayerHeights::tops(
            first.heights.height_of(0),
            first.heights.height_of(0),
            &tops,
        ),
        planes,
        printable: first.printable,
        layers,
        modelled,
        elephant_foot: first.elephant_foot,
        elephant_layers,
        keys,
        used,
        shares,
        stages: settings_of,
        plate_stages: first.plate_stages,
        walls_by,
        surfaces_by,
        fills_by,
        shelter,
        contours,
        contour_speed,
        supports,
        support: first.support,
        infill: first.infill,
        object_filaments,
        quality_notes,
        size_notes,
        ironing_notes,
        paint_notes,
        code_notes,
        times,
        body_keys,
        below: Some(below),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_mesh::Mesh;

    fn own(
        place: usize,
        settings: &[(&str, &str)],
        ranges: Vec<edge_3mf::LayerRange>,
    ) -> crate::OwnObject {
        crate::OwnObject {
            object: place,
            name: format!("object {place}"),
            settings: settings
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            volumes: Vec::new(),
            ranges,
            support_paint: Vec::new(),
            layer_height_profile: None,
            bottom: 0.0,
            solid: Mesh::default(),
        }
    }

    fn plate<'a>(settings: &'a Settings, heights: &'a [f64]) -> PlateLayers<'a> {
        PlateLayers {
            settings,
            layer_height: 0.2,
            first_layer_height: 0.2,
            heights,
        }
    }

    #[test]
    fn objects_that_set_nothing_of_their_heights_are_cut_at_the_plates() {
        let settings = Settings::from_json("{}").unwrap();
        let heights = [10.0, 5.0];
        let layering = settle(
            &plate(&settings, &heights),
            &[own(1, &[("wall_loops", "3")], Vec::new())],
        )
        .unwrap();
        assert!(layering.plain());
        // The plate's own layer height set again is no height of its own.
        let layering = settle(
            &plate(&settings, &heights),
            &[own(1, &[("layer_height", "0.2")], Vec::new())],
        )
        .unwrap();
        assert!(layering.plain());
    }

    #[test]
    fn an_object_of_its_own_height_is_a_group_of_its_own() {
        let settings = Settings::from_json("{}").unwrap();
        let heights = [10.0, 5.0, 4.0];
        let layering = settle(
            &plate(&settings, &heights),
            &[
                own(1, &[("layer_height", "0.12")], Vec::new()),
                own(2, &[("layer_height", "0.12")], Vec::new()),
            ],
        )
        .unwrap();
        assert_eq!(layering.group_of, vec![0, 1, 1]);
        assert_eq!(layering.groups.len(), 1);
        let layers = &layering.groups[0];
        // The first layer and then 0.12 mm layers to 5 mm.
        assert!((layers[0].1 - 0.2).abs() < 1e-9);
        assert!((layers[1].1 - 0.32).abs() < 1e-9);
        assert_eq!(layers.len(), 1 + ((5.0 - 0.2) / 0.12_f64).round() as usize);
    }

    #[test]
    fn a_range_of_another_height_makes_the_objects_layers_its_own() {
        let settings = Settings::from_json("{}").unwrap();
        let heights = [10.0];
        let range = edge_3mf::LayerRange {
            min_z: 4.0,
            max_z: 6.0,
            settings: vec![("layer_height".into(), "0.1".into())],
        };
        let layering = settle(
            &plate(&settings, &heights),
            &[own(0, &[], vec![range.clone()])],
        )
        .unwrap();
        assert_eq!(layering.group_of, vec![1]);
        assert_eq!(layering.groups[0].len(), 60);
        // A range at the plate's height is none of its own.
        let same = edge_3mf::LayerRange {
            settings: vec![("layer_height".into(), "0.2".into())],
            ..range
        };
        assert!(
            settle(&plate(&settings, &heights), &[own(0, &[], vec![same])])
                .unwrap()
                .plain()
        );
    }

    #[test]
    fn the_prime_tower_refuses_objects_of_different_heights_as_the_reference_does() {
        let settings = Settings::from_json(
            r#"{"enable_prime_tower": "1", "filament_diameter": ["1.75", "1.75"]}"#,
        )
        .unwrap();
        let heights = [10.0, 5.0];
        let error = settle(
            &plate(&settings, &heights),
            &[own(1, &[("layer_height", "0.12")], Vec::new())],
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "The prime tower requires that all objects have the same layer heights."
        );
        // A range on one object of two of different heights.
        let range = edge_3mf::LayerRange {
            min_z: 1.0,
            max_z: 2.0,
            settings: vec![("layer_height".into(), "0.1".into())],
        };
        let error = settle(&plate(&settings, &heights), &[own(1, &[], vec![range])]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "The prime tower is only supported if all objects have the same variable layer height."
        );
        // One filament: no tower, nothing refused.
        let one =
            Settings::from_json(r#"{"enable_prime_tower": "1", "filament_diameter": ["1.75"]}"#)
                .unwrap();
        assert!(
            settle(
                &plate(&one, &heights),
                &[own(1, &[("layer_height", "0.12")], Vec::new())]
            )
            .is_ok()
        );
    }

    #[test]
    fn organic_supports_refuse_a_variable_layer_height() {
        let settings = Settings::from_json(
            r#"{"enable_support": "1", "support_type": "tree(auto)", "support_style": "default"}"#,
        )
        .unwrap();
        let heights = [10.0];
        let range = edge_3mf::LayerRange {
            min_z: 4.0,
            max_z: 6.0,
            settings: vec![("layer_height".into(), "0.1".into())],
        };
        let error = settle(&plate(&settings, &heights), &[own(0, &[], vec![range])]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Variable layer height is not supported with Organic supports."
        );
    }
}
