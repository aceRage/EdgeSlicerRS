//! A centre for every branch on every layer, picked inside its influence
//! area from the bed up (TreeSupport3D.cpp `create_nodes_from_area`), and
//! where a branch stands on the part, the highest place it can.

use super::config::TreeConfig;
use super::element::Element;
use super::geometry::{intersection_near, move_inside_if_outside};
use super::volumes::Volumes;
use crate::SupportError;

/// Places the parents of `layers[layer][index]` on the layer above, as
/// near the element's own centre as their areas allow
/// (`set_points_on_areas`), and marks them carried.
fn set_points_on_areas(layers: &mut [Vec<Element>], layer: usize, index: usize) {
    let Some(at) = layers[layer][index].state.result else {
        return;
    };
    if layer + 1 >= layers.len() {
        return;
    }
    let parents = layers[layer][index].parents.clone();
    for parent in parents {
        let next = &mut layers[layer + 1][parent as usize];
        if next.state.result.is_none() {
            next.state.result = Some(move_inside_if_outside(&next.area, at));
        }
        next.state.marked = true;
    }
}

/// A branch that can stand on the part, placed on the highest layer above
/// it that it can stand on, and what is below that dropped
/// (`set_to_model_contact_to_model_gracious`).
fn set_to_model_gracious(
    volumes: &Volumes,
    config: &TreeConfig,
    layers: &mut [Vec<Element>],
    layer: usize,
    index: usize,
) -> Result<(), SupportError> {
    let mut last_success: Option<(usize, usize)> = None;
    let (mut l, mut i) = (layer, index);
    loop {
        let element = &layers[l][i];
        let radius = element.state.collision_radius(config);
        if intersection_near(&element.area, volumes.placeable(radius, l))?.is_empty() {
            break;
        }
        last_success = Some((l, i));
        if element.parents.len() != 1 || l + 1 >= layers.len() {
            break;
        }
        i = element.parents[0] as usize;
        l += 1;
    }
    match last_success {
        None => {
            // Nowhere to stand after all: it goes down anyway.
            let element = &mut layers[layer][index];
            element.state.to_model_gracious = false;
            element.state.result = Some(move_inside_if_outside(
                &element.area,
                element.state.next_position,
            ));
        }
        Some((top_layer, top_index)) => {
            let (mut l, mut i) = (layer, index);
            while (l, i) != (top_layer, top_index) {
                let element = &mut layers[l][i];
                element.state.deleted = true;
                i = element.parents[0] as usize;
                l += 1;
            }
            let element = &mut layers[top_layer][top_index];
            element.state.result = Some(move_inside_if_outside(
                &element.area,
                element.state.next_position,
            ));
        }
    }
    Ok(())
}

/// Gives every branch a centre, from the bed up, and drops what does not
/// stand on anything.
pub(crate) fn create_nodes_from_area(
    volumes: &Volumes,
    config: &TreeConfig,
    layers: &mut [Vec<Element>],
) -> Result<(), SupportError> {
    if layers.is_empty() {
        return Ok(());
    }
    if layers.len() > 1 {
        for element in &mut layers[1] {
            element.state.marked = false;
        }
    }
    for index in 0..layers[0].len() {
        let element = &mut layers[0][index];
        element.state.result = Some(move_inside_if_outside(
            &element.area,
            element.state.next_position,
        ));
        set_points_on_areas(layers, 0, index);
    }
    for layer in 1..layers.len() {
        if layer + 1 < layers.len() {
            for element in &mut layers[layer + 1] {
                element.state.marked = false;
            }
        }
        for index in 0..layers[layer].len() {
            if layers[layer][index].state.result.is_none() {
                let state = &layers[layer][index].state;
                if state.to_buildplate
                    || (state.distance_to_top < config.min_dtt_to_model && !state.supports_roof)
                {
                    // Lost on the way to the bed, or too short a stub to
                    // stand on the part.
                    layers[layer][index].state.deleted = true;
                } else if state.to_model_gracious {
                    set_to_model_gracious(volumes, config, layers, layer, index)?;
                } else {
                    let element = &mut layers[layer][index];
                    element.state.result = Some(move_inside_if_outside(
                        &element.area,
                        element.state.next_position,
                    ));
                }
            }
            let state = &layers[layer][index].state;
            if !state.deleted && !state.marked && state.target_height == layer {
                // A tip with nothing under it.
                layers[layer][index].state.deleted = true;
            }
            if layers[layer][index].state.deleted {
                if layer + 1 < layers.len() {
                    for parent in layers[layer][index].parents.clone() {
                        layers[layer + 1][parent as usize].state.result = None;
                    }
                }
            } else {
                set_points_on_areas(layers, layer, index);
            }
        }
    }
    remove_deleted(layers);
    Ok(())
}

/// Takes out what was dropped and renumbers the links to the layer above
/// (`remove_deleted_elements`).
fn remove_deleted(layers: &mut [Vec<Element>]) {
    let mut map_above: Vec<Option<u32>> = Vec::new();
    for layer in (0..layers.len()).rev() {
        let mut map: Vec<Option<u32>> = Vec::with_capacity(layers[layer].len());
        let mut kept = 0u32;
        for element in &layers[layer] {
            if element.state.deleted {
                map.push(None);
            } else {
                map.push(Some(kept));
                kept += 1;
            }
        }
        let elements = std::mem::take(&mut layers[layer]);
        layers[layer] = elements
            .into_iter()
            .filter(|element| !element.state.deleted)
            .map(|mut element| {
                if !map_above.is_empty() {
                    element.parents = element
                        .parents
                        .iter()
                        .filter_map(|parent| map_above.get(*parent as usize).copied().flatten())
                        .collect();
                }
                element
            })
            .collect();
        map_above = map;
    }
}
