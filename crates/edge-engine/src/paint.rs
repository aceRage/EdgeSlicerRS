//! Colours painted onto parts, applied to what each filament prints.

use std::collections::BTreeMap;

use edge_clip::ClipError;
use edge_config::{ConfigError, Settings};
use edge_extrusion::Flow;
use edge_geom::ExPolygon;
use edge_paint::{PaintSettings, Share};
use edge_slice::LayerRegions;
use rayon::prelude::*;

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
            // Set for each object by [`run_scale`].
            run_scale: 1.0,
        },
        notes,
    ))
}

/// How the lengths runs of colour are filtered by are scaled for an
/// object printed with `filaments` (counting from 1): by the finest of
/// their nozzles (MultiMaterialSegmentation.cpp
/// `mm_segmentation_finest_nozzle`, `mm_segmentation_run_filter_scale`).
/// The reference takes the object's extruders, every filament any of
/// its regions prints with; here they are the filament the object is
/// made of and those painted on it.
pub(crate) fn run_scale(
    settings: &Settings,
    filaments: impl IntoIterator<Item = usize>,
) -> Result<f32, ConfigError> {
    let nozzles: Vec<f64> = settings
        .floats("nozzle_diameter")?
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect();
    let finest = filaments
        .into_iter()
        .filter_map(|filament| {
            nozzles
                .get(filament.saturating_sub(1))
                .or(nozzles.first())
                .copied()
        })
        .filter(|nozzle| *nozzle > 0.0)
        .fold(0.0, |finest: f64, nozzle| {
            if finest == 0.0 {
                nozzle
            } else {
                finest.min(nozzle)
            }
        });
    Ok(edge_paint::run_filter_scale(finest))
}

/// Each layer of a painted object as colour is shared out over it: its
/// printed parts cut one by one, each with its cracks closed by itself,
/// then put together. As the reference has it
/// (MultiMaterialSegmentation.cpp, the start of `segmentation_by_painting`,
/// from the layer's regions as `PrintObjectSlice.cpp` cuts them part by
/// part): each piece is grown by ten times `SCALED_EPSILON`, a
/// micrometre, the pieces are united, islands and holes under 0.01 mm²
/// are dropped, and what is left is shrunk back. So two parts that
/// overlap or touch become one, and two that stand a few micrometres
/// apart keep the crack between them; the paint on the faces either side
/// of it colours the layer from there, as paint on the outside does, and
/// a part inside another keeps the colour painted on it.
///
/// Cutting the whole object as one mesh instead would fill such cracks
/// (they are far narrower than the closing radius) and leave only the
/// outside to share the layer out from.
pub(crate) fn outlines(
    object: &edge_3mf::Painted,
    planes: &[f64],
    closing_radius: edge_geom::Coord,
    base: usize,
) -> Result<Vec<Vec<ExPolygon>>, ClipError> {
    /// Ten times the reference's `SCALED_EPSILON`, in scaled units.
    const MERGE: edge_geom::Coord = 1_000;
    /// The reference's `remove_small_and_small_holes` limit, in mm².
    const SMALL: f64 = 0.01;
    /// Five times `SCALED_EPSILON`, the reference's `expolygons_simplify`
    /// tolerance there.
    const SIMPLIFY: edge_geom::Coord = 500;
    let parts = object
        .parts
        .par_iter()
        .map(|part| {
            edge_slice::slice_regions(
                &[edge_slice::Body {
                    solid: part,
                    negative: object.negative.as_ref(),
                    filament: base,
                }],
                planes,
                closing_radius,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    (0..planes.len())
        .into_par_iter()
        .map(|layer| {
            let pieces: Vec<ExPolygon> = parts
                .iter()
                .flat_map(|part| part[layer].regions.iter().cloned())
                .collect();
            if pieces.is_empty() {
                return Ok(Vec::new());
            }
            let mut merged = edge_clip::offset(&pieces, MERGE, edge_clip::Join::MITER)?;
            merged.retain(|piece| piece.area_mm2() >= SMALL);
            for piece in &mut merged {
                piece
                    .holes
                    .retain(|hole| hole.signed_area_mm2().abs() >= SMALL);
            }
            let shrunk = edge_clip::offset(&merged, -MERGE, edge_clip::Join::MITER)?;
            let mut simplified = edge_clip::simplify(&shrunk, SIMPLIFY)?;
            for piece in &mut simplified {
                drop_near_duplicates(&mut piece.contour);
                for hole in &mut piece.holes {
                    drop_near_duplicates(hole);
                }
                piece.holes.retain(|hole| !hole.points.is_empty());
            }
            simplified.retain(|piece| !piece.contour.points.is_empty());
            Ok(simplified)
        })
        .collect()
}

/// Takes out a corner closer than 0.01 mm to the one before it where the
/// outline barely turns there, by under 30 degrees seen from the corner
/// before: the reference's `remove_duplicates(MutablePolygon&, scaled_eps,
/// max_angle)` (MutablePolygon.cpp) as `segmentation_by_painting` calls
/// it. A ring left with under three corners is emptied.
fn drop_near_duplicates(ring: &mut edge_geom::Polygon) {
    const NEAR: i64 = 10_000;
    let cos_max_2 = (std::f64::consts::PI / 6.0).cos().powi(2);
    let points = &mut ring.points;
    if points.len() >= 3 {
        // From the second corner once round, the first staying, as the
        // reference's walk does: each corner is weighed against the last
        // one kept and the next one along.
        let count = points.len();
        let mut kept = Vec::with_capacity(count);
        kept.push(points[0]);
        for at in 1..count {
            let (prev, p, next) = (kept[kept.len() - 1], points[at], points[(at + 1) % count]);
            let v1 = ((p.x - prev.x) as f64, (p.y - prev.y) as f64);
            let v1_norm = v1.0 * v1.0 + v1.1 * v1.1;
            if v1_norm < (NEAR * NEAR) as f64 {
                let v2 = ((next.x - prev.x) as f64, (next.y - prev.y) as f64);
                let dot = v1.0 * v2.0 + v1.1 * v2.1;
                if dot * dot > cos_max_2 * v1_norm * (v2.0 * v2.0 + v2.1 * v2.1) {
                    continue;
                }
            }
            kept.push(p);
        }
        *points = kept;
    }
    if points.len() < 3 {
        points.clear();
    }
}

/// Gives the painted areas of a layer to the filaments they are painted
/// for, and takes them from whichever had them: `painted` holds each
/// painted object's areas in turn. A layer that was all one filament's,
/// `base`, is first set down as that filament's.
///
/// What a filament is given is united with what it had, cracks and all:
/// two parts of an object seldom meet exactly, and while the colours are
/// shared out over an outline that keeps the cracks between them
/// ([`outlines`]), the layer they are given out of has them closed. The
/// reference leaves them in the layer too, so where paint makes both
/// sides of a crack one filament it lays walls round the crack, loops as
/// wide as two half walls in the middle of a solid area. That is not
/// copied.
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

    /// A painted object of two boxes side by side, `gap` mm apart.
    fn two_boxes(gap: f32) -> edge_3mf::Painted {
        let cube = |min: [f32; 3], max: [f32; 3]| {
            edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(min, max))
        };
        edge_3mf::Painted {
            negative: None,
            facets: Vec::new(),
            parts: vec![
                cube([0.0, 0.0, 0.0], [10.0, 10.0, 1.0]),
                cube([10.0 + gap, 0.0, 0.0], [20.0, 10.0, 1.0]),
            ],
        }
    }

    #[test]
    fn parts_a_micrometre_apart_are_one_outline_and_wider_cracks_stay() {
        let touching = outlines(&two_boxes(0.001), &[0.5], 0, 1).unwrap();
        assert_eq!(touching[0].len(), 1);
        assert!((area(&touching[0]) - 200.0).abs() < 0.01);
        // Three micrometres is more than growing each side by one closes,
        // and the crack between the boxes is kept.
        let apart = outlines(&two_boxes(0.003), &[0.5], 0, 1).unwrap();
        assert_eq!(apart[0].len(), 2);
    }

    #[test]
    fn near_duplicate_corners_go_only_where_the_outline_runs_on() {
        let ring = |points: &[(f64, f64)]| Polygon {
            points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
        };
        // A corner 5 µm along a straight edge goes; one 5 µm off at a
        // right angle to where the outline goes on stays, as does
        // everything 0.01 mm or more apart.
        let mut straight = ring(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (5.005, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
        ]);
        drop_near_duplicates(&mut straight);
        assert_eq!(straight.points.len(), 4);
        let mut turning = ring(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (5.0, 0.005),
            (10.0, 0.005),
            (10.0, 10.0),
        ]);
        drop_near_duplicates(&mut turning);
        assert_eq!(turning.points.len(), 5);
        let mut tiny = ring(&[(0.0, 0.0), (0.001, 0.0), (0.002, 0.0)]);
        drop_near_duplicates(&mut tiny);
        assert!(tiny.points.is_empty());
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
