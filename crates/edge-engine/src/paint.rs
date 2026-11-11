//! Colours painted onto parts, applied to what each filament prints.

use std::collections::BTreeMap;

use edge_clip::ClipError;
use edge_config::{ConfigError, Settings};
use edge_extrusion::Flow;
use edge_geom::ExPolygon;
use edge_paint::{PaintSettings, Share};
use edge_slice::LayerRegions;

/// What the profile says of how deep paint goes, as [`PaintSettings`],
/// with a note for each thing it asks that is not done. `outer` is the
/// outer wall as laid at the layer height.
pub(crate) fn settings(
    settings: &Settings,
    outer: &Flow,
    layer_height: f64,
) -> Result<(PaintSettings, Vec<String>), ConfigError> {
    // As many layers as the shell has, and no fewer than its thickness
    // takes; none where the count is zero (`compute_layer_color_stat`).
    let layers = |count: &str, thickness: &str, default: i64| -> Result<usize, ConfigError> {
        let count = settings.integer(count)?.unwrap_or(default).max(0) as usize;
        let thickness = settings.float(thickness)?.unwrap_or(0.0);
        Ok(if count == 0 {
            0
        } else {
            count.max((thickness / layer_height - 1e-9).ceil().max(0.0) as usize)
        })
    };
    let gaps_filled = settings
        .floats("gap_infill_speed")?
        .and_then(|speeds| speeds.first().copied().flatten())
        .is_none_or(|speed| speed > 0.0);
    let small = if gaps_filled {
        0.25 * outer.width
    } else {
        0.5 * (outer.width + 0.7 * outer.spacing)
    };

    let mut notes = Vec::new();
    if let Some(mode) = settings.text("paint_depth_mode")
        && mode != "unlimited"
    {
        notes.push(format!(
            "paint_depth_mode \"{mode}\": painted colours go as deep as the part, as with \"unlimited\""
        ));
    }
    for key in [
        "mmu_segmented_region_max_width",
        "mmu_segmented_region_interlocking_depth",
    ] {
        if settings.float(key)?.is_some_and(|value| value > 0.0) {
            notes.push(format!(
                "{key}: painted colours go as deep as the part and do not interlock"
            ));
        }
    }
    if settings.flag("interlocking_beam")? == Some(true) {
        notes.push(
            "interlocking_beam \"1\": painted colours are not keyed into each other".to_owned(),
        );
    }
    for key in [
        "top_color_penetration_layers",
        "bottom_color_penetration_layers",
    ] {
        if settings.integer(key)?.is_some_and(|layers| layers > 0) {
            notes.push(format!(
                "{key}: paint on a face goes through the shell layers behind it"
            ));
        }
    }
    Ok((
        PaintSettings {
            top_layers: layers("top_shell_layers", "top_shell_thickness", 4)?,
            bottom_layers: layers("bottom_shell_layers", "bottom_shell_thickness", 3)?,
            wall_width: outer.scaled_width(),
            wall_spacing: outer.scaled_spacing(),
            small: edge_geom::scaled(small),
        },
        notes,
    ))
}

/// Gives the painted areas of a layer to the filaments they are painted
/// for, and takes them from whichever had them: `painted` holds each
/// painted object's areas in turn. A layer that was all one filament's,
/// `base`, is first set down as that filament's.
///
/// What a filament is given is united with what it had, cracks and all:
/// two parts of an object seldom meet exactly, and slicing here closes
/// the cracks between them. The reference leaves them, so where paint
/// makes both sides of a crack one filament it lays walls round the
/// crack, loops as wide as two half walls in the middle of a solid
/// area. That is not copied.
pub(crate) fn apply(
    layer: &mut LayerRegions,
    painted: &[&[Share]],
    base: usize,
) -> Result<(), ClipError> {
    let regions = layer.regions.clone();
    for shares in [&mut layer.by_filament, &mut layer.cut_by_filament] {
        if shares.is_empty() {
            *shares = vec![(base, regions.clone())];
        }
        let mut by_filament: BTreeMap<usize, Vec<ExPolygon>> = shares.drain(..).collect();
        for (colour, area) in painted.iter().flat_map(|object| object.iter()) {
            let colour = usize::from(*colour);
            let area = edge_clip::intersection(area, &regions)?;
            if area.is_empty() {
                continue;
            }
            for (_, own) in by_filament.iter_mut().filter(|(is, _)| **is != colour) {
                *own = edge_clip::difference(own, &area)?;
            }
            let own = by_filament.entry(colour).or_default();
            own.extend(area);
            *own = edge_clip::union_ex(own)?;
        }
        *shares = by_filament
            .into_iter()
            .filter(|(_, own)| !own.is_empty())
            .collect();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use edge_geom::{Point2, Polygon};

    use super::*;

    fn block(x0: f64, x1: f64) -> Vec<ExPolygon> {
        vec![ExPolygon {
            contour: Polygon {
                points: [(x0, 0.0), (x1, 0.0), (x1, 10.0), (x0, 10.0)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }]
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    #[test]
    fn paint_moves_material_from_one_filament_to_another() {
        // A layer all in filament 1, a quarter of it painted for 3.
        let mut layer = LayerRegions {
            z: 0.1,
            regions: block(0.0, 20.0),
            by_filament: Vec::new(),
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        apply(&mut layer, &[&[(3, block(15.0, 30.0))]], 1).unwrap();
        let shares: Vec<(usize, f64)> = layer
            .by_filament
            .iter()
            .map(|(filament, own)| (*filament, area(own).round()))
            .collect();
        assert_eq!(shares, [(1, 150.0), (3, 50.0)]);
        assert_eq!(layer.cut_by_filament.len(), 2);

        // Painted for the filament it already is, nothing moves; painted
        // over another's, that one gives way.
        apply(
            &mut layer,
            &[&[(1, block(0.0, 5.0))], &[(1, block(15.0, 18.0))]],
            1,
        )
        .unwrap();
        let shares: Vec<(usize, f64)> = layer
            .by_filament
            .iter()
            .map(|(filament, own)| (*filament, area(own).round()))
            .collect();
        assert_eq!(shares, [(1, 180.0), (3, 20.0)]);
        // What a filament is given joins what it had.
        assert_eq!(layer.by_filament[0].1.len(), 1);

        // With nothing painted a layer is still set down as its filament's.
        let mut plain = LayerRegions {
            z: 0.3,
            regions: block(0.0, 20.0),
            by_filament: Vec::new(),
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        apply(&mut plain, &[], 2).unwrap();
        assert_eq!(plain.by_filament.len(), 1);
        assert_eq!(plain.by_filament[0].0, 2);
    }

    #[test]
    fn shell_layers_and_small_areas_come_from_the_profile() {
        let outer = Flow::new(0.42, 0.2, 0.4).unwrap();
        let read = |json: &str| settings(&Settings::from_json(json).unwrap(), &outer, 0.2).unwrap();
        let (paint, notes) = read(
            r#"{"top_shell_layers": "5", "top_shell_thickness": "1.4",
                "bottom_shell_layers": "0", "bottom_shell_thickness": "1"}"#,
        );
        // Seven layers make 1.4 mm; no bottom shell at all means none.
        assert_eq!((paint.top_layers, paint.bottom_layers), (7, 0));
        assert_eq!(paint.small, edge_geom::scaled(0.105));
        assert!(notes.is_empty());

        let (paint, notes) = read(
            r#"{"gap_infill_speed": ["0"], "paint_depth_mode": "walls",
                "mmu_segmented_region_max_width": "1.5", "interlocking_beam": "1"}"#,
        );
        assert!(paint.small > edge_geom::scaled(0.3));
        assert_eq!(notes.len(), 3);
    }
}
