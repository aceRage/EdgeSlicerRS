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
/// outer wall as laid at the layer height, and `heights` how thick each
/// layer of the print is.
pub(crate) fn settings(
    settings: &Settings,
    outer: &Flow,
    layer_height: f64,
    heights: &[f64],
) -> Result<(PaintSettings, Vec<String>), ConfigError> {
    let mut notes = Vec::new();
    // As many layers as the shell has, and no fewer than its thickness
    // takes; none where the count is zero (`compute_layer_color_stat`).
    //
    // The thickness is taken in layers of the layer height. The
    // reference measures it in the layers as they lie behind each face
    // (MultiMaterialSegmentation.cpp `effective_shell_layers_by_thickness`),
    // which comes to another number where the first layer or the last
    // five (`precise_z_height`) are of another height; a note says so
    // where that is the case.
    let mut layers = |count_key: &str, key: &str, default: i64| -> Result<usize, ConfigError> {
        let count = settings.integer(count_key)?.unwrap_or(default).max(0) as usize;
        let thickness = settings.float(key)?.unwrap_or(0.0);
        if count == 0 {
            return Ok(0);
        }
        let deep = count.max((thickness / layer_height - 1e-9).ceil().max(0.0) as usize);
        let top = key.starts_with("top");
        let (pair, of) = if top {
            (((count, 0), (thickness, 0.0)), "top")
        } else {
            (((0, count), (0.0, thickness)), "bottom")
        };
        let (tops, bottoms) = edge_infill::shell_depths(heights, pair.0, pair.1);
        // The first face whose shell is not as deep as paint is carried,
        // short of where the print ends.
        let other = (0..heights.len()).find_map(|face| {
            let (shell, room) = if top {
                (tops[face], face + 1)
            } else {
                (bottoms[face], heights.len() - face)
            };
            (shell != deep.min(room)).then_some((face, shell))
        });
        if let Some((face, shell)) = other {
            notes.push(format!(
                "{key} \"{thickness}\": paint on a {of} face goes {deep} layer(s) deep, which is that \
                 thickness in layers of {layer_height} mm; the shell behind a {of} face on layer {} \
                 is {shell} layer(s) deep",
                face + 1
            ));
        }
        Ok(deep)
    };
    let top_layers = layers("top_shell_layers", "top_shell_thickness", 4)?;
    let bottom_layers = layers("bottom_shell_layers", "bottom_shell_thickness", 3)?;
    let gaps_filled = settings
        .floats("gap_infill_speed")?
        .and_then(|speeds| speeds.first().copied().flatten())
        .is_none_or(|speed| speed > 0.0);
    let small = if gaps_filled {
        0.25 * outer.width
    } else {
        0.5 * (outer.width + 0.7 * outer.spacing)
    };

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
            top_layers,
            bottom_layers,
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
        let read = |json: &str| {
            settings(&Settings::from_json(json).unwrap(), &outer, 0.2, &[0.2; 40]).unwrap()
        };
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

    #[test]
    fn paint_that_goes_another_depth_than_the_shell_is_said() {
        let outer = Flow::new(0.42, 0.2, 0.4).unwrap();
        let asked = Settings::from_json(
            r#"{"top_shell_layers": "5", "top_shell_thickness": "1.0",
                "bottom_shell_layers": "3", "bottom_shell_thickness": "0.85"}"#,
        )
        .unwrap();
        // Layers all of 0.2 mm: five make the top shell and five the
        // bottom one, and paint goes as deep.
        let (paint, notes) = settings(&asked, &outer, 0.2, &[0.2; 40]).unwrap();
        assert_eq!((paint.top_layers, paint.bottom_layers), (5, 5));
        assert!(notes.is_empty(), "{notes:?}");

        // The last five made 0.19 mm: the shell under the top is six
        // layers, and paint still goes five.
        let mut heights = [0.2; 40];
        heights[35..].fill(0.19);
        let (paint, notes) = settings(&asked, &outer, 0.2, &heights).unwrap();
        assert_eq!(paint.top_layers, 5);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].starts_with("top_shell_thickness \"1\""),
            "{}",
            notes[0]
        );
        assert!(notes[0].contains("goes 5 layer(s) deep"), "{}", notes[0]);
        assert!(
            notes[0].contains("on layer 36 is 6 layer(s) deep"),
            "{}",
            notes[0]
        );

        // A first layer of 0.25 mm: four layers over the bed start under
        // 0.85 mm, where five of 0.2 mm would be needed.
        let mut heights = [0.2; 40];
        heights[0] = 0.25;
        let (paint, notes) = settings(&asked, &outer, 0.2, &heights).unwrap();
        assert_eq!(paint.bottom_layers, 5);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].starts_with("bottom_shell_thickness \"0.85\""),
            "{}",
            notes[0]
        );
        assert!(
            notes[0].contains("on layer 1 is 4 layer(s) deep"),
            "{}",
            notes[0]
        );

        // No thickness asked, or no shell: nothing to say.
        let counted = Settings::from_json(
            r#"{"top_shell_layers": "5", "top_shell_thickness": "0",
                "bottom_shell_layers": "0", "bottom_shell_thickness": "1"}"#,
        )
        .unwrap();
        let (_, notes) = settings(&counted, &outer, 0.2, &heights).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
    }
}
